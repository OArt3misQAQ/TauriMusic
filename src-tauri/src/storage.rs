//! 数据根目录:引导文件解析、路径重映射与整库迁移。
//!
//! 应用数据(曲库/音乐/封面/回收站)默认落在系统数据目录(Windows 是 %APPDATA%,
//! 即 C 盘),用户可在设置里把整库迁到任意本地目录(如 D 盘)。引导文件
//! storage.json 固定留在默认目录:启动时先读它决定真正的数据根;自定义根
//! 建不出来(外置盘拔出)时回退默认位置,配置保留,重新插上重启即恢复。
//!
//! 迁移语义:复制-校验-清理。先整库复制到新位置(写好重映射后的
//! library.json 与引导文件),全部成功后才清旧位置文件;中途任何失败
//! 都不会影响旧库,新位置的半成品会被清掉(恢复默认流程除外)。

use crate::model::Library;
use crate::{lock_scan_gate, AppState, LIBRARY_FILE};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{Emitter, Manager};

/// 引导文件名:固定在默认数据目录,记录自定义数据根(缺省 = 用默认位置)
const BOOTSTRAP_FILE: &str = "storage.json";
/// 迁移目标统一落在所选目录下的这个子目录,避免用户选盘符根目录时撒一地文件
const ROOT_DIR_NAME: &str = "TauriMusic";

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Bootstrap {
    /// 自定义数据根;None/缺字段 = 使用默认位置
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root: Option<String>,
}

/// 启动时决定数据根:引导文件有效且自定义根可创建则用之,否则回退默认。
/// 回退时不动引导文件——移动硬盘重新插上后重启即可回到自定义库
pub(crate) fn resolve_data_root(default_dir: &Path) -> (PathBuf, bool) {
    let Some(root) = read_bootstrap(default_dir) else {
        return (default_dir.to_path_buf(), false);
    };
    let custom = PathBuf::from(&root);
    match fs::create_dir_all(&custom) {
        Ok(()) => (custom, true),
        Err(e) => {
            eprintln!(
                "[storage] 自定义数据根不可用({e}),本次启动回退默认位置: {}",
                default_dir.display()
            );
            (default_dir.to_path_buf(), false)
        }
    }
}

fn read_bootstrap(default_dir: &Path) -> Option<String> {
    let bytes = fs::read(default_dir.join(BOOTSTRAP_FILE)).ok()?;
    match serde_json::from_slice::<Bootstrap>(&bytes) {
        Ok(b) => b.root.filter(|r| !r.trim().is_empty()),
        Err(e) => {
            // 引导文件损坏视为未配置(默认位置必有完整数据),留痕便于排查
            eprintln!("[storage] 引导文件解析失败,按默认位置处理: {e}");
            None
        }
    }
}

fn write_bootstrap(default_dir: &Path, root: Option<&Path>) -> Result<(), String> {
    let payload = Bootstrap {
        root: root.map(|p| p.to_string_lossy().into_owned()),
    };
    let bytes = serde_json::to_vec_pretty(&payload).map_err(|e| e.to_string())?;
    // 引导文件只有几百字节,直接 tmp→rename 原子写,防断电半截
    let dest = default_dir.join(BOOTSTRAP_FILE);
    let tmp = default_dir.join(format!("{BOOTSTRAP_FILE}.tmp"));
    fs::write(&tmp, bytes).map_err(|e| format!("写引导文件失败: {e}"))?;
    fs::rename(&tmp, &dest).map_err(|e| format!("落引导文件失败: {e}"))
}

/// 大小写不敏感地拼接规范化路径键:统一分隔符并转小写,仅用于比较
fn norm_key(p: &Path) -> String {
    p.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// child 是否位于 parent 之下(组件边界严格区分,`D:\Lib` 不吃 `D:\LibX`)
fn is_under(child: &str, parent: &str) -> bool {
    child != parent && child.starts_with(&format!("{parent}\\"))
}

/// 目标合法性校验,返回真正的数据根(所选目录下的 TauriMusic 子目录)。
/// 拒绝:与当前位置相同、互为祖先/后代(嵌套会让"清旧位置"误删新库)、
/// 目标已有曲库数据(恢复默认流程除外——默认目录里的旧 library.json 是
/// 上次迁走时留的回退备份,本来就要被当前库覆写)
fn validate_target(current: &Path, default_dir: &Path, picked: &Path) -> Result<PathBuf, String> {
    if !picked.is_absolute() {
        return Err("请选择一个绝对路径".into());
    }
    if !picked.is_dir() {
        return Err("所选目录不存在".into());
    }
    let root = if picked
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case(ROOT_DIR_NAME))
    {
        picked.to_path_buf()
    } else {
        picked.join(ROOT_DIR_NAME)
    };
    let (cur_key, root_key) = (norm_key(current), norm_key(&root));
    if root_key == cur_key {
        return Err("新位置与当前位置相同".into());
    }
    if is_under(&root_key, &cur_key) || is_under(&cur_key, &root_key) {
        return Err("新位置不能嵌在当前存储目录里(或反过来),请另选目录".into());
    }
    // 已有曲库数据 → 拒:所选目录本身有(可能直接选中了上一个库的根),
    // 或算出的 TauriMusic 子目录里有,都视为占用
    if root != default_dir
        && (root.join(LIBRARY_FILE).is_file() || picked.join(LIBRARY_FILE).is_file())
    {
        return Err("所选位置已有曲库数据(library.json),请换一个目录".into());
    }
    Ok(root)
}

/// 路径前缀重映射:识别 old 前缀(大小写/分隔符不敏感)后替换为 new。
/// 组件级比较天然防住 `D:\Lib` 误吃 `D:\LibX`;不属于 old 之下返回 None
fn remap_path(path: &Path, old: &Path, new: &Path) -> Option<PathBuf> {
    let pc: Vec<_> = path.components().collect();
    let oc: Vec<_> = old.components().collect();
    if pc.len() < oc.len() {
        return None;
    }
    for (a, b) in pc.iter().zip(oc.iter()) {
        let (a, b) = (
            a.as_os_str().to_string_lossy(),
            b.as_os_str().to_string_lossy(),
        );
        if !a.eq_ignore_ascii_case(&b) {
            return None;
        }
    }
    let mut out = new.to_path_buf();
    for c in &pc[oc.len()..] {
        out.push(c.as_os_str());
    }
    Some(out)
}

fn remap_track(t: &mut crate::model::Track, old_data: &Path, new_data: &Path) {
    if let Some(p) = remap_path(Path::new(&t.path), old_data, new_data) {
        t.path = p.to_string_lossy().into_owned();
    }
    for p in [&mut t.cover, &mut t.cover_large, &mut t.lrc_path] {
        if let Some(s) = p.as_deref() {
            if let Some(n) = remap_path(Path::new(s), old_data, new_data) {
                *p = Some(n.to_string_lossy().into_owned());
            }
        }
    }
}

/// 曲库内全部绝对路径字段重映射:登记文件夹、曲目(音频/歌词/封面)与
/// 回收站条目(回收站元数据内嵌在 library.json,文件实体随 Trash/ 目录搬)
fn remap_library(lib: &mut Library, old_data: &Path, new_data: &Path) {
    for f in &mut lib.folders {
        if let Some(n) = remap_path(Path::new(f), old_data, new_data) {
            *f = n.to_string_lossy().into_owned();
        }
    }
    for t in &mut lib.tracks {
        remap_track(t, old_data, new_data);
    }
    for e in &mut lib.trash {
        remap_track(&mut e.track, old_data, new_data);
        if let Some(n) = remap_path(Path::new(&e.trashed_audio), old_data, new_data) {
            e.trashed_audio = n.to_string_lossy().into_owned();
        }
        if let Some(l) = &mut e.trashed_lrc {
            if let Some(n) = remap_path(Path::new(l), old_data, new_data) {
                *l = n.to_string_lossy().into_owned();
            }
        }
    }
}

/// 递归清点目录内文件数(跨卷复制前算进度分母);skip 中的目录名整支跳过
fn count_tree(dir: &Path, skip: &[&str]) -> u64 {
    let mut n = 0u64;
    let Ok(rd) = fs::read_dir(dir) else {
        return 0;
    };
    for e in rd.filter_map(|e| e.ok()) {
        let p = e.path();
        if !p.is_dir() {
            n += 1;
            continue;
        }
        let skipped = p
            .file_name()
            .is_some_and(|n| skip.iter().any(|s| n.eq_ignore_ascii_case(s)));
        if !skipped {
            n += count_tree(&p, skip);
        }
    }
    n
}

/// 递归复制目录(跨卷安全:一律 fs::copy);返回复制的文件数。
/// 每个文件回调一次进度,由调用方节流发事件
fn copy_tree(
    src: &Path,
    dest: &Path,
    skip: &[&str],
    on_file: &mut dyn FnMut(),
) -> Result<u64, String> {
    fs::create_dir_all(dest).map_err(|e| format!("创建 {} 失败: {e}", dest.display()))?;
    let rd = fs::read_dir(src).map_err(|e| format!("读取 {} 失败: {e}", src.display()))?;
    let mut n = 0u64;
    for e in rd.filter_map(|e| e.ok()) {
        let sp = e.path();
        let dp = dest.join(e.file_name());
        if sp.is_dir() {
            let skipped = sp
                .file_name()
                .is_some_and(|n| skip.iter().any(|s| n.eq_ignore_ascii_case(s)));
            if !skipped {
                n += copy_tree(&sp, &dp, skip, on_file)?;
            }
        } else {
            fs::copy(&sp, &dp).map_err(|e| format!("复制 {} 失败: {e}", sp.display()))?;
            on_file();
            n += 1;
        }
    }
    Ok(n)
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MigrationProgress {
    done: u64,
    total: u64,
    /// copy = 复制中,finalize = 收尾(引导文件/清理),done = 全部完成
    phase: &'static str,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MigrationReport {
    files: u64,
    /// 非致命告警(如旧位置个别文件被占用没删掉),前端展示给用户
    warnings: Vec<String>,
}

/// 旧版封面缓存搬家:0.9.x 封面缓存在系统缓存目录(LOCALAPPDATA),
/// 改为随数据根走(应用数据自包含,迁移时整库带走)。只在数据根封面为空
/// 时搬(首次升级);同步执行——本地盘几秒级的一次性成本,换路径不变式。
/// 返回是否真的搬了(调用方据此同步改写曲库里的封面路径)
pub(crate) fn adopt_legacy_covers(cache_dir: &Path, covers_dir: &Path) -> bool {
    let legacy = cache_dir.join("covers");
    if !legacy.is_dir() {
        return false;
    }
    let empty = fs::read_dir(covers_dir)
        .map(|mut rd| rd.next().is_none())
        .unwrap_or(false);
    if !empty {
        return false;
    }
    let mut noop = || {};
    if copy_tree(&legacy, covers_dir, &["preview"], &mut noop).is_ok() {
        // 旧目录是可再生的缓存,搬成功即删,别在 C 盘留整份副本
        let _ = fs::remove_dir_all(&legacy);
        eprintln!("[storage] 旧封面缓存已并入数据根");
        return true;
    }
    false
}

/// 旧封面缓存并入数据根后,曲库里的封面路径同步改指新位置——
/// 否则首次扫描发现封面文件全部缺失,会把整库当「已变化」全量重解析
pub(crate) fn remap_cover_paths(lib: &mut Library, old_covers: &Path, new_covers: &Path) {
    let fix = |p: &mut Option<String>| {
        if let Some(s) = p.as_deref() {
            if let Some(n) = remap_path(Path::new(s), old_covers, new_covers) {
                *p = Some(n.to_string_lossy().into_owned());
            }
        }
    };
    for t in &mut lib.tracks {
        fix(&mut t.cover);
        fix(&mut t.cover_large);
    }
    // 回收站快照同样带封面路径(还原后沿用)
    for e in &mut lib.trash {
        fix(&mut e.track.cover);
        fix(&mut e.track.cover_large);
    }
}

/// 迁移整库到新数据根(picked = None 表示恢复默认位置)。
/// 复制-校验-清理:引导文件在全部复制成功后才落盘,中途失败旧库毫发无损
pub(crate) fn change_storage_root_blocking(
    app: &tauri::AppHandle,
    picked: Option<&str>,
) -> Result<MigrationReport, String> {
    let state = app.state::<AppState>();
    let default_dir = state.default_data_dir.clone();
    let old_data = state.data_dir.clone();
    let new_data = match picked {
        Some(p) => validate_target(&old_data, &default_dir, Path::new(p))?,
        None => default_dir.clone(),
    };
    // 恢复默认的防呆:本就在默认位置时按钮不可见,这里兜住命令级直调
    if new_data == old_data {
        return Err("当前已在使用该位置".into());
    }

    // 扫描闸罩住全程:扫描/删除/还原中途改库,会让新旧两份 library.json 永久分叉
    let _gate = lock_scan_gate(&state);

    // 1) 曲库重映射后写进新根(锁内只序列化,写盘在锁外)
    fs::create_dir_all(&new_data).map_err(|e| format!("创建目标目录失败: {e}"))?;
    let bytes = {
        let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        remap_library(&mut lib, &old_data, &new_data);
        lib.to_bytes().map_err(|e| e.to_string())?
    };
    Library::write_bytes(&new_data.join(LIBRARY_FILE), &bytes)
        .map_err(|e| format!("写出曲库失败: {e}"))?;

    // 2) 统计原样复制(内容只有日期/曲目 id 键值,不含路径)
    let old_stats = old_data.join(crate::stats::STATS_FILE);
    if old_stats.is_file() {
        fs::copy(&old_stats, new_data.join(crate::stats::STATS_FILE))
            .map_err(|e| format!("复制统计数据失败: {e}"))?;
    }

    // 3) 大目录逐支复制:covers/preview 是导出暂存,不迁
    let skip = ["preview"];
    let total = count_tree(&old_data.join("Music"), &skip)
        + count_tree(&old_data.join("Trash"), &skip)
        + count_tree(&old_data.join("covers"), &skip);
    let mut done = 0u64;
    let mut on_file = || {
        done += 1;
        // 节流:每 32 个文件报一次,万首库也只发几百条事件
        if done.is_multiple_of(32) {
            let _ = app.emit(
                "storage-migration",
                MigrationProgress {
                    done,
                    total,
                    phase: "copy",
                },
            );
        }
    };
    let copy_result = (|| -> Result<(), String> {
        copy_tree(
            &old_data.join("Music"),
            &new_data.join("Music"),
            &skip,
            &mut on_file,
        )?;
        copy_tree(
            &old_data.join("Trash"),
            &new_data.join("Trash"),
            &skip,
            &mut on_file,
        )?;
        copy_tree(
            &old_data.join("covers"),
            &new_data.join("covers"),
            &skip,
            &mut on_file,
        )?;
        Ok(())
    })();
    if let Err(e) = copy_result {
        // 中途失败:旧库未动。非默认目标整体清掉半成品;恢复默认流程的目标
        // 目录里还有回退备份等既有文件,只报错不删
        if new_data != default_dir {
            let _ = fs::remove_dir_all(&new_data);
        }
        return Err(e);
    }
    let _ = app.emit(
        "storage-migration",
        MigrationProgress {
            done: total,
            total,
            phase: "finalize",
        },
    );

    // 4) 引导文件落盘(固定在默认目录)。到这里迁移已实际成功,
    //    即使后续清理告警也不回滚——重启后即用新根
    if new_data == default_dir {
        // 恢复默认:删掉引导文件回到"未配置"状态
        let _ = fs::remove_file(default_dir.join(BOOTSTRAP_FILE));
    } else {
        write_bootstrap(&default_dir, Some(&new_data))?;
    }

    // 5) 尽力清理旧位置:大目录全删;曲库/统计文件只在旧根非默认时删
    //    (默认目录里的那份是数据根不可用时的回退数据,必须留)
    let mut warnings = Vec::new();
    for dir in ["Music", "Trash", "covers"] {
        if let Err(e) = fs::remove_dir_all(old_data.join(dir)) {
            warnings.push(format!(
                "旧位置 {dir}/ 清理失败(文件被占用?),可稍后手动删除: {e}"
            ));
        }
    }
    if old_data != default_dir {
        for f in [LIBRARY_FILE, crate::stats::STATS_FILE, BOOTSTRAP_FILE] {
            let _ = fs::remove_file(old_data.join(f));
        }
    }

    let _ = app.emit(
        "storage-migration",
        MigrationProgress {
            done: total,
            total,
            phase: "done",
        },
    );
    Ok(MigrationReport {
        files: total,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        copy_tree, count_tree, is_under, norm_key, read_bootstrap, remap_cover_paths,
        remap_library, remap_path, resolve_data_root, validate_target, write_bootstrap,
        BOOTSTRAP_FILE,
    };
    use crate::model::{Library, Track, TrashEntry};
    use crate::LIBRARY_FILE;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// 平台原生路径辅助:测试断言按分隔符分叉,保证 Linux CI 与 Windows 都能跑
    fn p(old_root: &str, rel: &str) -> String {
        let sep = if cfg!(windows) { "\\" } else { "/" };
        format!("{old_root}{sep}{}", rel.replace('/', sep))
    }

    fn old_root() -> &'static str {
        if cfg!(windows) {
            "D:\\Old"
        } else {
            "/tmp/old"
        }
    }

    fn new_root() -> &'static str {
        if cfg!(windows) {
            "E:\\New"
        } else {
            "/tmp/new"
        }
    }

    fn track(path: &str) -> Track {
        Track {
            path: path.into(),
            ..Default::default()
        }
    }

    #[test]
    fn remap_only_rewrites_paths_under_old_root() {
        let (old, new) = (Path::new(old_root()), Path::new(new_root()));
        // 基本重映射:old 之下按组件替换前缀,余段原样保留(中文名不受伤)
        assert_eq!(
            remap_path(Path::new(&p(old_root(), "Music/艺人/b.flac")), old, new),
            Some(PathBuf::from(p(new_root(), "Music/艺人/b.flac")))
        );
        // 组件边界:`…\Old` 不能吃掉 `…\OldX`
        let sibling = format!("{}X", old_root());
        assert_eq!(remap_path(Path::new(&p(&sibling, "a.mp3")), old, new), None);
        // 相等路径整体替换;不相关路径不动
        assert_eq!(remap_path(old, old, new), Some(new.to_path_buf()));
        assert_eq!(remap_path(Path::new("/etc/passwd"), old, new), None);
    }

    #[test]
    fn remap_library_covers_every_path_field() {
        // 曲库内所有绝对路径都要跟着搬:曲目(音频/歌词/封面)、登记文件夹、回收站
        let (old, new) = (PathBuf::from(old_root()), PathBuf::from(new_root()));
        let music = p(old_root(), "Music/a/b.flac");
        let mut lib = Library {
            version: 1,
            // 外部文件夹(old 根之外)必须原样保留
            folders: vec![p(old_root(), "Music"), "Z:\\外部".into()],
            tracks: vec![Track {
                cover: Some(p(old_root(), "covers/1.jpg")),
                cover_large: Some(p(old_root(), "covers/1@512.jpg")),
                lrc_path: Some(p(old_root(), "Music/a/b.lrc")),
                ..track(&music)
            }],
            playlists: vec![],
            trash: vec![TrashEntry {
                track: track(&music),
                trashed_audio: p(old_root(), "Trash/1_b.flac"),
                trashed_lrc: None,
                deleted_at: 0.0,
            }],
        };
        remap_library(&mut lib, &old, &new);
        assert_eq!(lib.tracks[0].path, p(new_root(), "Music/a/b.flac"));
        assert_eq!(
            lib.tracks[0].cover.as_deref(),
            Some(p(new_root(), "covers/1.jpg").as_str())
        );
        assert_eq!(
            lib.tracks[0].lrc_path.as_deref(),
            Some(p(new_root(), "Music/a/b.lrc").as_str())
        );
        assert_eq!(lib.folders[0], p(new_root(), "Music"));
        assert_eq!(lib.folders[1], "Z:\\外部");
        assert_eq!(lib.trash[0].trashed_audio, p(new_root(), "Trash/1_b.flac"));
    }

    #[test]
    fn validate_target_rejects_same_nested_and_occupied() {
        let base = std::env::temp_dir().join(format!("tm-storage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let current = base.join("cur");
        let default_dir = base.join("default");
        fs::create_dir_all(current.join("Music")).unwrap();
        fs::create_dir_all(&default_dir).unwrap();

        // 与当前位置相同
        assert!(validate_target(&current, &default_dir, &current).is_err());
        // 目标嵌在当前位置之下:选当前根的子目录,算出的 TauriMusic 根在当前根里
        let sub = current.join("sub2");
        fs::create_dir_all(&sub).unwrap();
        assert!(validate_target(&current, &default_dir, &sub).is_err());
        // 当前位置嵌在目标之下:当前根名为 TauriMusic 时,选其父目录会算出
        // 把当前根整个包进去的目标(清理旧位置会误删新库)
        let nested_base = base.join("TauriMusic");
        let nested_cur = nested_base.join("lib");
        fs::create_dir_all(&nested_cur).unwrap();
        assert!(validate_target(&nested_cur, &default_dir, &nested_base).is_err());

        // 目标已有曲库数据(无论在所选目录还是其 TauriMusic 子目录)→ 拒。
        // 「恢复默认」不走本函数(命令里 None 直接用 default_dir),无需放行特例
        let occupied = base.join("occ");
        fs::create_dir_all(occupied.join("TauriMusic")).unwrap();
        fs::write(occupied.join(LIBRARY_FILE), b"{}").unwrap();
        assert!(validate_target(&current, &default_dir, &occupied).is_err());
        let occupied2 = base.join("occ2");
        fs::create_dir_all(occupied2.join("TauriMusic")).unwrap();
        fs::write(occupied2.join("TauriMusic").join(LIBRARY_FILE), b"{}").unwrap();
        assert!(validate_target(&current, &default_dir, &occupied2).is_err());

        // 正常目标:目录名已叫 TauriMusic 时不再套一层子目录
        let named = base.join("TauriMusic");
        fs::create_dir_all(&named).unwrap();
        assert_eq!(
            validate_target(&current, &default_dir, &named).unwrap(),
            named
        );
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn bootstrap_roundtrip_and_fallback() {
        let base = std::env::temp_dir().join(format!("tm-boot-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        // 无引导文件 → 默认位置
        assert_eq!(resolve_data_root(&base).0, base);

        // 指向存在的目录 → 生效;损坏的 JSON 视为未配置
        let custom = base.join("d2");
        write_bootstrap(&base, Some(&custom)).unwrap();
        assert_eq!(resolve_data_root(&base).0, custom);
        fs::write(base.join(BOOTSTRAP_FILE), b"{broken").unwrap();
        assert_eq!(resolve_data_root(&base).0, base);

        // 根目录不存在 → 启动时自动创建
        let fresh = base.join("d3");
        write_bootstrap(&base, Some(&fresh)).unwrap();
        let (root, custom_flag) = resolve_data_root(&base);
        assert!(custom_flag && root == fresh && fresh.is_dir());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn nested_check_respects_component_boundary() {
        // `D:\Lib` 不是 `D:\LibX` 的前缀(裸字符串 starts_with 会误判,规范化后按边界比)
        assert!(!is_under(
            &norm_key(Path::new("D:\\LibX\\a")),
            &norm_key(Path::new("D:\\Lib"))
        ));
        assert!(is_under(
            &norm_key(Path::new("D:\\Lib\\a\\b")),
            &norm_key(Path::new("D:\\Lib"))
        ));
        assert!(!is_under("x", "x"));
    }

    #[test]
    fn copy_tree_counts_and_skips() {
        let base = std::env::temp_dir().join(format!("tm-tree-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("src/sub/preview")).unwrap();
        fs::write(base.join("src/a.bin"), [1u8]).unwrap();
        fs::write(base.join("src/sub/b.bin"), [2u8]).unwrap();
        fs::write(base.join("src/sub/preview/c.bin"), [3u8]).unwrap();
        assert_eq!(count_tree(&base.join("src"), &["preview"]), 2);
        let mut n = 0u32;
        copy_tree(
            &base.join("src"),
            &base.join("dst"),
            &["preview"],
            &mut || n += 1,
        )
        .unwrap();
        assert_eq!(n, 2);
        assert!(base.join("dst/sub/b.bin").is_file());
        assert!(!base.join("dst/sub/preview").exists());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn remap_cover_paths_only_touches_covers() {
        // 旧封面缓存搬家后,曲目/回收站快照里的封面路径要跟过去,
        // 音频与歌词路径(在 Music 下,不在 covers 下)必须原样保留
        let (old, new) = (
            PathBuf::from(p(old_root(), "covers")),
            PathBuf::from(p(new_root(), "covers")),
        );
        let music = p(old_root(), "Music/a/b.flac");
        let mut lib = Library {
            version: 1,
            folders: vec![],
            tracks: vec![Track {
                cover: Some(p(old_root(), "covers/1.jpg")),
                cover_large: Some(p(old_root(), "covers/1@512.jpg")),
                lrc_path: Some(p(old_root(), "Music/a/b.lrc")),
                ..track(&music)
            }],
            playlists: vec![],
            trash: vec![TrashEntry {
                track: Track {
                    cover: Some(p(old_root(), "covers/2.jpg")),
                    ..track(&music)
                },
                trashed_audio: p(old_root(), "Trash/2_b.flac"),
                trashed_lrc: None,
                deleted_at: 0.0,
            }],
        };
        remap_cover_paths(&mut lib, &old, &new);
        assert_eq!(
            lib.tracks[0].cover.as_deref(),
            Some(p(new_root(), "covers/1.jpg").as_str())
        );
        assert_eq!(
            lib.tracks[0].cover_large.as_deref(),
            Some(p(new_root(), "covers/1@512.jpg").as_str())
        );
        assert_eq!(
            lib.tracks[0].lrc_path.as_deref(),
            Some(p(old_root(), "Music/a/b.lrc").as_str())
        );
        assert_eq!(
            lib.trash[0].track.cover.as_deref(),
            Some(p(new_root(), "covers/2.jpg").as_str())
        );
    }

    #[test]
    fn read_bootstrap_tolerates_missing_file() {
        // 文件缺失返回 None 而不是 panic:这是启动路径,必须静默兜底
        let missing = std::env::temp_dir().join(format!("tm-none-{}", std::process::id()));
        let _ = fs::remove_dir_all(&missing);
        assert_eq!(read_bootstrap(&missing), None);
    }
}
