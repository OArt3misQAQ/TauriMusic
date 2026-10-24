//! 散装导入:拖拽与"打开方式"的音频/歌词复制进导入目录、ffmpeg 转码与内容指纹去重。

use crate::model::Library;
use crate::pack::{record_track_shas, sha256_hex};
use crate::scanner;
use crate::{lock_scan_gate, refresh_asset_scope, show_main, AppState, LIBRARY_FILE};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{Emitter, Manager};

/// 应用自带的音乐导入目录(data_dir/Music),随应用首次启动创建并登记;
/// 扫描器会把顶层散装音频按「艺人\专辑」归位到该目录下
pub(crate) const IMPORT_DIR_NAME: &str = "Music";

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportReport {
    folders_added: u32,
    files_copied: u32,
    /// 其中经 ffmpeg 转码为 FLAC 的非常规格式(ape/wma 等)数量
    transcoded: u32,
    skipped: u32,
    duplicates: u32,
}

/// 单文件导入结果:直接复制成功 / 转码成功 / 内容重复跳过 / 不支持或已存在跳过
enum CopyResult {
    Copied,
    Transcoded,
    Duplicate,
    Skipped,
}

/// 确保导入目录存在,并把它登记进曲库扫描来源。
/// 先落盘成功再改内存语义:落盘失败时回滚内存,内存态不发生变化。
/// 已登记的快路径不碰扫描闸(get_import_dir 是同步命令,不能在扫描期间卡主线程);
/// 真要写库时才闸→锁依次拿,与全仓库加锁顺序一致
pub(crate) fn ensure_import_dir(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<PathBuf, String> {
    let dir = state.data_dir.join(IMPORT_DIR_NAME);
    fs::create_dir_all(&dir).map_err(|e| format!("创建导入目录失败: {e}"))?;
    let path = dir.to_string_lossy().to_string();
    {
        let lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        if lib.folders.iter().any(|f| f == &path) {
            return Ok(dir);
        }
    }
    let _gate = lock_scan_gate(state);
    let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
    // 双检:等闸期间可能已被其他导入登记
    if !lib.folders.iter().any(|f| f == &path) {
        lib.folders.push(path.clone());
        // 锁内只序列化,磁盘 IO 在锁外
        let bytes = lib.to_bytes().map_err(|e| e.to_string())?;
        drop(lib);
        if let Err(e) = Library::write_bytes(&state.data_dir.join(LIBRARY_FILE), &bytes) {
            // 重新拿锁回滚:按值移除而非 pop,避免等锁期间的并发登记让序号错位
            if let Ok(mut lib) = state.lib.lock() {
                lib.folders.retain(|f| *f != path);
            }
            return Err(e.to_string());
        }
        refresh_asset_scope(app);
    }
    Ok(dir)
}

/// 散装导入报告:(登记文件夹数, 直接复制数, 转码数, 跳过数, 重复数, 本次收集的指纹)
type LooseImportResult = (u32, u32, u32, u32, u32, Vec<(PathBuf, String)>);

/// 把一批路径导入:文件夹登记进曲库,散装文件复制进导入目录并收集 SHA 指纹。
/// lib 锁只用于登记与快照,文件复制与哈希在锁外进行(大文件哈希不该卡住其他命令);
/// 返回报告与本次新收集的指纹。import_paths 命令与 handle_open_paths 共用
#[allow(clippy::too_many_arguments)]
fn import_loose(
    paths: &[PathBuf],
    import_dir: &Path,
    state: &AppState,
    ffmpeg: Option<&Path>,
) -> Result<LooseImportResult, String> {
    let mut folders_added = 0u32;
    let mut files_copied = 0u32;
    let mut transcoded = 0u32;
    let mut skipped = 0u32;
    let mut duplicates = 0u32;
    let mut session_shas: Vec<(PathBuf, String)> = Vec::new();

    // 短闸 + 短锁:登记文件夹与落盘只在这里互斥扫描;文件复制(可能分钟级)
    // 不占闸,别把用户手动触发的扫描挡在门外
    let (mut known, known_shas, pending_bytes) = {
        let _gate = lock_scan_gate(state);
        let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        let mut dirty = false;
        for raw in paths {
            let path = Path::new(raw);
            if path.is_dir() {
                let dir_str = path.to_string_lossy().to_string();
                if !lib.folders.iter().any(|f| f == &dir_str) {
                    lib.folders.push(dir_str);
                    folders_added += 1;
                    dirty = true;
                }
            }
        }
        // 锁内只序列化;落盘在锁外(与原实现一致:失败上抛,已 push 的文件夹留在内存)
        let bytes = if dirty {
            Some(lib.to_bytes().map_err(|e| e.to_string())?)
        } else {
            None
        };
        let known = lib
            .tracks
            .iter()
            .map(|t| (t.path.clone(), t.size))
            .collect::<Vec<_>>();
        let shas = lib
            .tracks
            .iter()
            .filter_map(|t| t.sha256.as_deref().map(|s| s.to_ascii_lowercase()))
            .collect::<std::collections::HashSet<_>>();
        (known, shas, bytes)
    };
    if let Some(bytes) = pending_bytes {
        Library::write_bytes(&state.data_dir.join(LIBRARY_FILE), &bytes)
            .map_err(|e| e.to_string())?;
    }

    // 锁外:复制文件(内容哈希在这里发生)
    for raw in paths {
        let path = Path::new(raw);
        if !path.is_file() {
            continue;
        }
        match copy_into_import_dir(
            path,
            import_dir,
            &known,
            &known_shas,
            &mut session_shas,
            ffmpeg,
        ) {
            Ok(CopyResult::Copied) => {
                files_copied += 1;
                if let (Ok(m), Some(name)) = (fs::metadata(path), path.file_name()) {
                    known.push((import_dir.join(name).to_string_lossy().to_string(), m.len()));
                }
            }
            Ok(CopyResult::Transcoded) => transcoded += 1,
            Ok(CopyResult::Duplicate) => duplicates += 1,
            _ => skipped += 1,
        }
    }
    Ok((
        folders_added,
        files_copied,
        transcoded,
        skipped,
        duplicates,
        session_shas,
    ))
}

pub(crate) fn import_paths_blocking(
    paths: &[String],
    app: &tauri::AppHandle,
) -> Result<ImportReport, String> {
    let state = app.state::<AppState>();
    let import_dir = ensure_import_dir(app, &state)?;
    let ffmpeg = find_ffmpeg(app);
    let path_bufs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let (folders_added, files_copied, transcoded, skipped, duplicates, session_shas) =
        import_loose(&path_bufs, &import_dir, &state, ffmpeg.as_deref())?;
    let report = ImportReport {
        folders_added,
        files_copied,
        transcoded,
        skipped,
        duplicates,
    };

    // 扫描入库并把新导入文件的 SHA 指纹写进曲库——散装导入与音乐包导入
    // 从此共用同一套内容指纹,互相去重
    scanner::run_scan(app)?;
    record_track_shas(&state, &session_shas)?;
    if report.folders_added > 0 {
        refresh_asset_scope(app);
    }
    // 曲库已被本命令改写:与其他导入路径对齐,广播事件让监听方刷新
    // (handle_open_paths/import_tmc 等都发,唯独这里漏了)
    let _ = app.emit("library-changed", ());
    Ok(report)
}

/// 两个文件内容是否完全一致(仅在大小相同时调用才有意义)
fn same_file_content(a: &Path, b: &Path) -> bool {
    use std::io::Read;
    let (Ok(mut fa), Ok(mut fb)) = (fs::File::open(a), fs::File::open(b)) else {
        return false;
    };
    let mut ba = [0u8; 64 * 1024];
    let mut bb = [0u8; 64 * 1024];
    loop {
        let (Ok(n1), Ok(n2)) = (fa.read(&mut ba), fb.read(&mut bb)) else {
            return false;
        };
        if n1 != n2 {
            return false;
        }
        if n1 == 0 {
            return true;
        }
        if ba[..n1] != bb[..n2] {
            return false;
        }
    }
}

/// 把单个文件复制进导入目录;只收音频与 .lrc 歌词。
/// 非常规格式(ape/wma 等,见 TRANSCODE_EXTS)在有 ffmpeg 时转成 FLAC 入库,原文件不动。
/// 去重规则:同名同大小视为已存在;与曲库或本次会话已知文件大小相同且内容一致视为重复;
/// 同名但大小不同视为新版本,覆盖旧文件。转码件以「目标 flac 已存在且源未更新」判重。
fn copy_into_import_dir(
    src: &Path,
    import_dir: &Path,
    known: &[(String, u64)],
    known_shas: &std::collections::HashSet<String>,
    session_shas: &mut Vec<(PathBuf, String)>,
    ffmpeg: Option<&Path>,
) -> Result<CopyResult, String> {
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    let Some(ext) = ext else {
        return Ok(CopyResult::Skipped);
    };
    let is_lyrics = ext == "lrc";
    let is_direct = scanner::EXTENSIONS.contains(&ext.as_str());
    let is_transcode = scanner::TRANSCODE_EXTS.contains(&ext.as_str());
    if !is_lyrics && !is_direct && !is_transcode {
        return Ok(CopyResult::Skipped);
    }
    // 非常规格式:有 ffmpeg 才转 FLAC(封面走 attached_pic,容器不支持时自动退回纯音频);
    // 没有 ffmpeg 就静默跳过——报告的 skipped 由此变大,由前端提示安装 ffmpeg
    if is_transcode {
        let Some(ff) = ffmpeg else {
            return Ok(CopyResult::Skipped);
        };
        let Some(stem) = src.file_stem() else {
            return Ok(CopyResult::Skipped);
        };
        let dest = import_dir.join(format!("{}.flac", stem.to_string_lossy()));
        if dest.exists() {
            // 已转过:源文件比转码件新(改过标签/换过版本)才重转
            let stale = match (fs::metadata(src), fs::metadata(&dest)) {
                (Ok(a), Ok(b)) => a
                    .modified()
                    .ok()
                    .zip(b.modified().ok())
                    .map(|(s, d)| s > d)
                    .unwrap_or(false),
                _ => false,
            };
            if !stale {
                return Ok(CopyResult::Skipped);
            }
        }
        transcode_to_flac(ff, src, &dest)
            .map_err(|e| format!("转码 {} 失败: {e}", src.display()))?;
        return Ok(CopyResult::Transcoded);
    }
    let Some(name) = src.file_name() else {
        return Ok(CopyResult::Skipped);
    };
    let dest = import_dir.join(name);
    if dest.exists() {
        let same = match (fs::metadata(src), fs::metadata(&dest)) {
            (Ok(a), Ok(b)) => a.len() == b.len(),
            _ => false,
        };
        if same {
            return Ok(CopyResult::Skipped);
        }
    } else if let Ok(m) = fs::metadata(src) {
        // 内容级去重:先查全库 SHA 指纹(O(1)),未命中再退回同大小逐字节比对
        // (候选最多比 5 个,避免大文件 IO 过久)
        if let Some(sha) = sha256_hex(src) {
            let sha_lc = sha.to_ascii_lowercase();
            if known_shas.contains(&sha_lc) {
                return Ok(CopyResult::Duplicate);
            }
            let size = m.len();
            let mut checked = 0u32;
            for (p, s) in known {
                if *s == size && checked < 5 {
                    checked += 1;
                    if same_file_content(src, Path::new(p)) {
                        return Ok(CopyResult::Duplicate);
                    }
                }
            }
            session_shas.push((dest.clone(), sha_lc));
        }
    }
    fs::copy(src, &dest)
        .map(|_| CopyResult::Copied)
        .map_err(|e| format!("复制 {} 失败: {e}", src.display()))
}

#[cfg(windows)]
const FFMPEG_BIN: &str = "ffmpeg.exe";
#[cfg(not(windows))]
const FFMPEG_BIN: &str = "ffmpeg";

/// 定位 ffmpeg:①打包资源(release 安装)②系统 PATH(开发机常见)。都没有则不转码。
/// 资源文件要求 >10MB:开发机构建需要一个占位文件让 tauri-build 的资源校验通过,
/// 真正的 ffmpeg.exe 远大于占位文本,体积即真伪判据
fn find_ffmpeg(app: &tauri::AppHandle) -> Option<PathBuf> {
    if let Ok(dir) = app.path().resource_dir() {
        let p = dir.join(FFMPEG_BIN);
        let big_enough = p.metadata().map(|m| m.len() > 10_000_000).unwrap_or(false);
        if p.is_file() && big_enough {
            return Some(p);
        }
    }
    if let Ok(out) = std::process::Command::new(FFMPEG_BIN)
        .arg("-version")
        .output()
    {
        if out.status.success() {
            return Some(PathBuf::from(FFMPEG_BIN));
        }
    }
    None
}

/// 转码为无损 FLAC:映射全部元数据与内嵌封面(attached_pic);
/// 部分容器/封装的图流写不进 FLAC 时,退回纯音频重试。
/// stderr 只取最后一行作错误上下文(ffmpeg 的正常输出全在 stderr 上)
fn transcode_to_flac(ffmpeg: &Path, src: &Path, dest: &Path) -> Result<(), String> {
    let attempt = |with_cover: bool| -> Result<(), String> {
        let mut cmd = std::process::Command::new(ffmpeg);
        cmd.arg("-y")
            .arg("-nostdin")
            .args(["-i"])
            .arg(src)
            .args(["-map", "0:a"]);
        if with_cover {
            cmd.args(["-map", "0:v?"])
                .args(["-c:v", "copy"])
                .args(["-disposition:v", "attached_pic"]);
        }
        cmd.args(["-c:a", "flac"])
            .arg(dest)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        let out = cmd.output().map_err(|e| format!("启动 ffmpeg 失败: {e}"))?;
        if out.status.success() {
            return Ok(());
        }
        Err(String::from_utf8_lossy(&out.stderr)
            .lines()
            .last()
            .unwrap_or("ffmpeg 转码失败")
            .to_string())
    };
    attempt(true).or_else(|_| attempt(false))
}

/// 处理"打开方式"/命令行传入的文件:.tmc 解包导入,音频复制进导入目录,完成后重扫
pub(crate) fn handle_open_paths(app: &tauri::AppHandle, args: &[String]) {
    let paths: Vec<PathBuf> = args
        .iter()
        .skip(1)
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .collect();
    if paths.is_empty() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // 7z 解压 + 全量扫描都是重活,放阻塞线程池,避免卡 tokio worker
        let worker = tauri::async_runtime::spawn_blocking(move || {
            let state = app.state::<AppState>();
            let import_dir = match ensure_import_dir(&app, &state) {
                Ok(d) => d,
                Err(e) => {
                    let _ = app.emit("import-error", format!("导入目录不可用: {e}"));
                    return;
                }
            };
            let mut all_session_shas: Vec<(PathBuf, String)> = Vec::new();
            let mut failures: Vec<String> = Vec::new();
            let mut loose: Vec<PathBuf> = Vec::new();
            let mut packages: Vec<String> = Vec::new();
            for p in &paths {
                let ext = p
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase().to_string())
                    .unwrap_or_default();
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.to_string_lossy().to_string());
                if scanner::EXTENSIONS.contains(&ext.as_str()) || ext == "lrc" {
                    // 散装音频与歌词:走共享导入通道(文件夹/文件统一处理)
                    loose.push(p.clone());
                    continue;
                }
                if ext == "tmc" || ext == "tmca" || ext == "tmcl" {
                    // 音乐包改走预览弹窗:前端确认后才正式导入(入队防启动期前端未就绪)
                    packages.push(p.to_string_lossy().to_string());
                    continue;
                }
                failures.push(format!("{name}: 不支持的格式"));
            }
            if !loose.is_empty() {
                let ffmpeg = find_ffmpeg(&app);
                match import_loose(&loose, &import_dir, &state, ffmpeg.as_deref()) {
                    Ok((_, _, _, _, _, shas)) => all_session_shas.extend(shas),
                    Err(e) => failures.push(format!("导入失败: {e}")),
                }
            }
            // 音乐包路径交给前端预览:已经运行的实例立即收事件;
            // 启动期的实例前端未就绪会错过,由 take_pending_imports 在初始化时拉取
            if !packages.is_empty() {
                if let Ok(mut g) = state.pending_imports.lock() {
                    g.extend(packages.iter().cloned());
                }
                let _ = app.emit("import-request", packages.len() as u32);
                show_main(&app);
            }
            if !loose.is_empty() {
                if let Err(e) = scanner::run_scan(&app) {
                    failures.push(format!("扫描失败: {e}"));
                }
            }
            if record_track_shas(&state, &all_session_shas).is_err() {
                eprintln!("[import] 散装导入指纹写入失败");
            }
            if !failures.is_empty() {
                // 汇总失败清单推给前端(状态条最多展示有限字数,取前 3 条)
                let summary = failures
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ");
                let more = if failures.len() > 3 {
                    format!(" 等 {} 项", failures.len())
                } else {
                    String::new()
                };
                let _ = app.emit("import-error", format!("{summary}{more}"));
            }
            let _ = app.emit("library-changed", ());
        });
        let _ = worker.await;
    });
}

/// 从直链推容器扩展名:各来源、各档位下发的格式不同(QQ 匿名档是 m4a,
/// 酷狗免费档可能给 flac)。扫描器按扩展名认格式,认不出才退回 mp3
pub(crate) fn media_ext_from_url(url: &str) -> &'static str {
    const CANDIDATES: &[&str] = &["mp3", "m4a", "flac", "ogg", "oga", "opus", "wav"];
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let tail = path.rsplit('/').next().unwrap_or(path);
    tail.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .and_then(|ext| CANDIDATES.iter().find(|c| **c == ext).copied())
        .unwrap_or("mp3")
}

#[cfg(test)]
mod tests {
    use super::{
        copy_into_import_dir, media_ext_from_url, same_file_content, sha256_hex, CopyResult,
    };
    use std::fs;

    #[test]
    fn media_ext_from_url_follows_the_delivered_container() {
        // 各来源给的容器不同:存错扩展名会让扫描器认不出这个文件
        assert_eq!(
            media_ext_from_url("https://m801.music.126.net/2026/x/y.mp3?session=1"),
            "mp3"
        );
        assert_eq!(
            media_ext_from_url("https://isure.stream.qqmusic.qq.com/C400ABC.m4a?vkey=1"),
            "m4a"
        );
        assert_eq!(
            media_ext_from_url("http://fsandroid.tx.kugou.com/2026/a/b.flac"),
            "flac"
        );
        // 认不出的扩展名退回 mp3(免费档几乎都是 mp3)
        assert_eq!(media_ext_from_url("https://kw-bj.kuwo.cn/a/b.xyz"), "mp3");
        assert_eq!(media_ext_from_url("https://a.com/path"), "mp3");
    }

    #[test]
    fn import_skips_transcodables_without_ffmpeg() {
        // 无 ffmpeg:非常规格式静默跳过,不产生任何文件;歌词与直放格式不受影响
        use super::{copy_into_import_dir, CopyResult};
        let dir = std::env::temp_dir().join(format!("tm-import-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let imp = dir.join("imp");
        fs::create_dir_all(&imp).unwrap();
        let ape = dir.join("凉宫春日.ape");
        fs::write(&ape, b"ape-bytes").unwrap();
        let got = copy_into_import_dir(&ape, &imp, &[], &Default::default(), &mut Vec::new(), None);
        assert!(matches!(got, Ok(CopyResult::Skipped)));
        assert!(!imp.join("凉宫春日.flac").exists());
        let lrc = dir.join("凉宫春日.lrc");
        fs::write(&lrc, "[00:01]test").unwrap();
        let got = copy_into_import_dir(&lrc, &imp, &[], &Default::default(), &mut Vec::new(), None);
        assert!(matches!(got, Ok(CopyResult::Copied)));
        assert!(imp.join("凉宫春日.lrc").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn content_compare_matches_identical_files() {
        let dir = std::env::temp_dir().join(format!("tm-dedup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.bin");
        let b = dir.join("b.bin");
        let c = dir.join("c.bin");
        fs::write(&a, [1u8, 2, 3, 4]).unwrap();
        fs::write(&b, [1u8, 2, 3, 4]).unwrap();
        fs::write(&c, [1u8, 2, 3, 5]).unwrap();
        assert!(same_file_content(&a, &b));
        assert!(!same_file_content(&a, &c));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn import_rejects_duplicate_content() {
        // 前缀必须与其他测试唯一:测试并行跑在同一进程,process::id 相同,
        // 共用目录会被另一测试的 remove_dir_all 连根删掉,fs::write 直接 NotFound
        let dir = std::env::temp_dir().join(format!("tm-import-dup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let imp = dir.join("imp");
        fs::create_dir_all(&imp).unwrap();
        let no_shas = std::collections::HashSet::new();
        let mut session: Vec<(std::path::PathBuf, String)> = Vec::new();

        // 曲库里已有的原始文件
        let origin = dir.join("song.mp3");
        fs::write(&origin, b"same-audio-content").unwrap();
        let known = vec![(origin.to_string_lossy().to_string(), 18)];

        // 同内容不同名 → 重复拒收
        let dup = dir.join("song (1).mp3");
        fs::write(&dup, b"same-audio-content").unwrap();
        assert!(matches!(
            copy_into_import_dir(&dup, &imp, &known, &no_shas, &mut session, None),
            Ok(CopyResult::Duplicate)
        ));

        // 不同内容 → 正常复制
        let fresh = dir.join("other.mp3");
        fs::write(&fresh, b"different-content").unwrap();
        assert!(matches!(
            copy_into_import_dir(&fresh, &imp, &known, &no_shas, &mut session, None),
            Ok(CopyResult::Copied)
        ));
        // 复制成功的同时记录了指纹,供会话内后续去重
        assert_eq!(session.len(), 1);

        // 同名同大小 → 已存在跳过
        let origin_copy = dir.join("origin_copy");
        fs::copy(&origin, &origin_copy).unwrap();
        assert!(matches!(
            copy_into_import_dir(&origin_copy, &imp, &known, &no_shas, &mut session, None),
            Ok(CopyResult::Skipped)
        ));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn import_rejects_by_sha_index() {
        // 字节比对兜底不可用(无候选)时,SHA 指纹命中同样拒收
        let dir = std::env::temp_dir().join(format!("tm-sha-dedup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let imp = dir.join("imp");
        fs::create_dir_all(&imp).unwrap();

        let incoming = dir.join("new_name.mp3");
        fs::write(&incoming, b"identical-bytes").unwrap();
        let mut known_shas = std::collections::HashSet::new();
        known_shas.insert(sha256_hex(&incoming).unwrap());
        let mut session: Vec<(std::path::PathBuf, String)> = Vec::new();

        assert!(matches!(
            copy_into_import_dir(&incoming, &imp, &[], &known_shas, &mut session, None),
            Ok(CopyResult::Duplicate)
        ));
        assert!(!imp.join("new_name.mp3").exists(), "重复内容不落盘");
        fs::remove_dir_all(&dir).ok();
    }
}
