mod lyrics;
mod media;
mod model;
mod scanner;
mod single_instance;
mod stats;

use model::Library;
use sha2::Digest as _;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{ipc::InvokeResponseBody, Emitter, Listener, Manager, State, WindowEvent};

pub(crate) fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub(crate) fn toggle_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
        } else {
            show_main(app);
        }
    }
}

pub struct AppState {
    pub lib: Mutex<Library>,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    /// 扫描重入闸:同一时刻只允许一次全量扫描
    pub scan_gate: Mutex<()>,
    /// 听歌统计(仅本机 stats.json):内存常驻,冲刷时原子落盘
    pub stats: Mutex<stats::StatsData>,
    /// 待预览导入的音乐包路径:文件关联打开时前端可能尚未就绪,先入队由前端初始化后拉取
    pub pending_imports: Mutex<Vec<String>>,
}

pub(crate) const LIBRARY_FILE: &str = "library.json";
/// 应用回收站目录(data_dir/Trash):删除的音乐移到这里,保留 TRASH_RETENTION_DAYS 天
pub(crate) const TRASH_DIR_NAME: &str = "Trash";
/// 回收站保留天数:超期在启动时自动清理
pub(crate) const TRASH_RETENTION_DAYS: f64 = 30.0;
/// 应用自带的音乐导入目录(data_dir/Music),随应用首次启动创建并登记;
/// 扫描器会把顶层散装音频按「艺人\专辑」归位到该目录下
pub(crate) const IMPORT_DIR_NAME: &str = "Music";

/// 收紧 asset 协议面:只放行封面缓存与已登记曲库文件夹下的文件。
/// tauri.conf.json 的静态 `**` 范围已移除,未登记路径无法经 asset: 访问,
/// 被投毒的前端无法再借封面/音频通道探测任意本地文件。
fn refresh_asset_scope(app: &tauri::AppHandle) {
    let scope = app.asset_protocol_scope();
    let _ = scope.allow_directory(app.state::<AppState>().cache_dir.join("covers"), true);
    if let Ok(lib) = app.state::<AppState>().lib.lock() {
        for folder in &lib.folders {
            let _ = scope.allow_directory(folder, true);
        }
    }
}

/// 确保导入目录存在,并把它登记进曲库扫描来源。
/// 先落盘成功再改内存,落盘失败时内存态不发生变化。
fn ensure_import_dir(app: &tauri::AppHandle, state: &AppState) -> Result<PathBuf, String> {
    let dir = state.data_dir.join(IMPORT_DIR_NAME);
    fs::create_dir_all(&dir).map_err(|e| format!("创建导入目录失败: {e}"))?;
    let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
    let path = dir.to_string_lossy().to_string();
    if !lib.folders.iter().any(|f| f == &path) {
        lib.folders.push(path);
        if let Err(e) = lib.save(&state.data_dir.join(LIBRARY_FILE)) {
            lib.folders.pop();
            return Err(e.to_string());
        }
        refresh_asset_scope(app);
    }
    Ok(dir)
}

#[tauri::command]
fn get_import_dir(app: tauri::AppHandle, state: State<AppState>) -> Result<String, String> {
    let dir = ensure_import_dir(&app, &state)?;
    Ok(dir.to_string_lossy().into_owned())
}

#[tauri::command]
fn open_import_dir(app: tauri::AppHandle, state: State<AppState>) -> Result<String, String> {
    let dir = ensure_import_dir(&app, &state)?;
    std::process::Command::new("explorer")
        .arg(&dir)
        .spawn()
        .map_err(|e| format!("打开资源管理器失败: {e}"))?;
    Ok(dir.to_string_lossy().into_owned())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
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

/// 拖拽导入:整个文件夹登记为扫描来源(不复制),散装音频/歌词文件复制进导入目录
#[tauri::command]
async fn import_paths(paths: Vec<String>, app: tauri::AppHandle) -> Result<ImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || import_paths_blocking(&paths, &app))
        .await
        .map_err(|e| e.to_string())?
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

    // 短锁:登记文件夹 + 快照去重基准(路径/大小 与 SHA 指纹)
    let (mut known, known_shas) = {
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
        if dirty {
            lib.save(&state.data_dir.join(LIBRARY_FILE))
                .map_err(|e| e.to_string())?;
        }
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
        (known, shas)
    };

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

fn import_paths_blocking(paths: &[String], app: &tauri::AppHandle) -> Result<ImportReport, String> {
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

/// 用路径组件判断 target 是否位于 dir 之下(与字符串前缀匹配不同,
/// 不会把 F:\music2 误当成 F:\music 的子目录)。
/// 手动按 /\ 双分隔符切分组件,不依赖 std 的平台语义,
/// Windows 风格路径字符串在非 Windows 平台上同样可判定。
fn is_under(child: &str, dir: &str) -> bool {
    fn components(p: &str) -> impl Iterator<Item = &str> {
        p.split(['\\', '/']).filter(|s| !s.is_empty())
    }
    // Windows 路径大小写不敏感(盘符/目录),其他平台精确比较
    fn part_eq(a: &str, b: &str) -> bool {
        if cfg!(windows) {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    }
    let mut child_parts = components(child);
    let mut matched = 0usize;
    for d in components(dir) {
        matched += 1;
        match child_parts.next() {
            Some(c) if part_eq(c, d) => continue,
            _ => return false,
        }
    }
    matched > 0
}

/// 把打包完成的临时压缩包落位到用户指定路径。
/// 打包期间目标路径不存在文件,用户不会拷到半成品;
/// 同盘 rename 原子直达,跨盘(系统 Temp 与目标不同卷)退化为拷贝,源是完整封口的包。
fn place_archive(tmp: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if fs::rename(tmp, dest).is_ok() {
        return Ok(());
    }
    // 跨盘拷贝中途失败(磁盘满/拔盘):必须清掉目标处的半成品,
    // 否则下次落位同名文件会被 dedup_dest 改名成 "xxx (2)"
    if let Err(e) = fs::copy(tmp, dest) {
        let _ = fs::remove_file(dest);
        return Err(format!("落位到 {} 失败: {e}", dest.display()));
    }
    let _ = fs::remove_file(tmp);
    Ok(())
}

/// 目标已存在时追加序号 name (2).ext / name (3).ext,保留扩展名
fn dedup_dest(dest: PathBuf) -> PathBuf {
    if !dest.exists() {
        return dest;
    }
    let parent = dest.parent().unwrap_or(Path::new(".")).to_path_buf();
    let stem = dest
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let ext = dest
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let mut i = 2;
    loop {
        let candidate = parent.join(format!("{stem} ({i}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
        i += 1;
    }
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

// ===== TMC 音乐包(.tmc = 标准 7z:音频 + 同名 .lrc 歌词 + cover 封面 + meta.json) =====

pub(crate) fn sanitize_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => out.push('_'),
            c if (c as u32) < 32 => {}
            c => out.push(c),
        }
    }
    // Windows 文件名:首尾空白与结尾的点会被静默吞掉或报错,保留名不可用
    let mut t = out.trim().trim_end_matches(['.', ' ']).to_string();
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    // Windows 保留设备名对「主名」生效:CON.mp3 与 CON 一样不可用
    let base = t.split('.').next().unwrap_or_default();
    if RESERVED.contains(&base.to_ascii_uppercase().as_str()) {
        t.insert(0, '_');
    }
    if t.is_empty() {
        "track".into()
    } else {
        t
    }
}

/// 解包 .tmc 到导入目录下的同名子目录,由扫描器拾取(meta.json 会补齐缺失标签)。
/// 内容级去重:音频 SHA-256 与曲库已有曲目相同则整目录清理并跳过(与包导入同标准)
#[tauri::command]
async fn import_tmc(path: String, app: tauri::AppHandle) -> Result<PackImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || import_tmc_file(Path::new(&path), &app))
        .await
        .map_err(|e| e.to_string())?
}

fn import_tmc_file(src: &Path, app: &tauri::AppHandle) -> Result<PackImportReport, String> {
    if !src.is_file() {
        return Err("文件不存在".into());
    }
    let state = app.state::<AppState>();
    let import_dir = ensure_import_dir(app, &state)?;
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "tmc".into());
    let dest = import_dir.join(&stem);
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| format!("清理旧目录失败: {e}"))?;
    }
    fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    sevenz_rust2::decompress_file(src, &dest).map_err(|e| format!("解包失败: {e}"))?;

    // 新包带音频 SHA-256:解包结果与记录不符说明文件损坏,拒绝入库并清理
    let meta_path = dest.join("meta.json");
    let mut title = stem.clone();
    if meta_path.is_file() {
        if let Ok(v) =
            serde_json::from_slice::<serde_json::Value>(&fs::read(&meta_path).unwrap_or_default())
        {
            if let Some(expect) = v.get("sha256").and_then(|x| x.as_str()) {
                let audio = fs::read_dir(&dest).ok().and_then(|rd| {
                    rd.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
                        p.extension()
                            .and_then(|x| x.to_str())
                            .map(|x| scanner::EXTENSIONS.contains(&x.to_lowercase().as_str()))
                            .unwrap_or(false)
                    })
                });
                if let Some(audio) = audio {
                    let ok = sha256_hex(&audio)
                        .map(|h| h.eq_ignore_ascii_case(expect))
                        .unwrap_or(false);
                    if !ok {
                        let _ = fs::remove_dir_all(&dest);
                        return Err("音乐包校验失败:文件已损坏".into());
                    }
                }
            }
            if let Some(t) = v.get("title").and_then(|x| x.as_str()) {
                if !t.trim().is_empty() {
                    title = t.trim().to_string();
                }
            }
        }
    }
    let audio = fs::read_dir(&dest)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.extension()
                .and_then(|x| x.to_str())
                .map(|x| scanner::EXTENSIONS.contains(&x.to_lowercase().as_str()))
                .unwrap_or(false)
        })
        .ok_or("包内没有音频文件")?;
    // 内容级去重:音频 SHA-256 与曲库已有曲目相同 → 清理并跳过(单曲包此前没有去重)
    let sha = sha256_hex(&audio).map(|h| h.to_ascii_lowercase());
    if let Some(sha) = &sha {
        let dup = {
            let lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
            lib.tracks.iter().any(|t| {
                t.sha256
                    .as_deref()
                    .map(|s| s.eq_ignore_ascii_case(sha))
                    .unwrap_or(false)
            })
        };
        if dup {
            let _ = fs::remove_dir_all(&dest);
            return Ok(PackImportReport {
                name: title,
                imported: 0,
                skipped: 1,
            });
        }
    }
    scanner::run_scan(app)?;
    if let Some(sha) = &sha {
        record_track_shas(&state, &[(audio.clone(), sha.clone())])?;
    }
    let _ = app.emit("library-changed", ());
    Ok(PackImportReport {
        name: title,
        imported: 1,
        skipped: 0,
    })
}

// ===== 音乐包导入预览:解包到暂存目录解析元数据,不写库;确认后才走正式导入 =====

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewTrack {
    title: String,
    artist: String,
    album: String,
    duration: u32,
    /// 相对暂存根的封面路径(资产协议已放行 covers 缓存),None 渲染占位
    cover: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PackagePreview {
    /// tmc(单曲包)/ tmca(专辑包)/ tmcl(歌单包)
    kind: String,
    name: String,
    /// 暂存目录绝对根:封面 = root + 相对路径,经资产协议展示
    root: String,
    /// 暂存目录名(covers/preview/<id>),前端确认/取消后调 discard_preview 清理
    staging_id: String,
    tracks: Vec<PreviewTrack>,
}

/// 读音频内嵌标签与时长(失败全部容错返回 None,预览不能因为坏文件而中断)
fn read_preview_tags(path: &Path) -> (Option<String>, Option<String>, Option<String>, Option<u32>) {
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::tag::Accessor;
    let Ok(tagged) = lofty::probe::Probe::open(path).and_then(|p| p.read()) else {
        return (None, None, None, None);
    };
    let duration = tagged.properties().duration().as_secs() as u32;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let field = |f: fn(&lofty::tag::Tag) -> Option<std::borrow::Cow<'_, str>>| -> Option<String> {
        tag.and_then(f)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let title = field(lofty::tag::Tag::title);
    let artist = field(lofty::tag::Tag::artist);
    let album = field(lofty::tag::Tag::album);
    (title, artist, album, Some(duration))
}

/// 在目录里找封面:cover.* 命名优先,否则取最大的图片文件;返回相对暂存根的路径
fn find_cover_in(dir: &Path, staging: &Path) -> Option<String> {
    let mut best: Option<(std::path::PathBuf, u64)> = None;
    for e in fs::read_dir(dir).ok()?.filter_map(|e| e.ok()) {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        let ext = p
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| x.to_lowercase())
            .unwrap_or_default();
        if !matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp") {
            continue;
        }
        let len = e.metadata().map(|m| m.len()).unwrap_or(0);
        let is_named = p
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("cover"))
            .unwrap_or(false);
        // cover.* 直接采用;否则与当前最大图比大小
        if is_named || best.as_ref().map(|(_, l)| len > *l).unwrap_or(true) {
            best = Some((p, len));
        }
    }
    let p = best?.0;
    p.strip_prefix(staging)
        .ok()
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
}

fn inspect_tmc(staging: &Path) -> Result<PackagePreview, String> {
    let meta = fs::read(staging.join("meta.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    let json_str = |k: &str| -> Option<String> {
        meta.as_ref()
            .and_then(|m| m.get(k))
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let audio = fs::read_dir(staging)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.extension()
                .and_then(|x| x.to_str())
                .map(|x| scanner::EXTENSIONS.contains(&x.to_lowercase().as_str()))
                .unwrap_or(false)
        })
        .ok_or("包内没有音频文件")?;
    let (tag_title, tag_artist, tag_album, duration) = read_preview_tags(&audio);
    let cover = find_cover_in(staging, staging);
    Ok(PackagePreview {
        kind: "tmc".into(),
        name: json_str("title")
            .or(tag_title.clone())
            .unwrap_or_else(|| "未命名单曲".into()),
        root: staging.to_string_lossy().to_string(),
        staging_id: String::new(),
        tracks: vec![PreviewTrack {
            title: json_str("title")
                .or(tag_title)
                .unwrap_or_else(|| "未知标题".into()),
            artist: json_str("artist")
                .or(tag_artist)
                .unwrap_or_else(|| "未知艺人".into()),
            album: json_str("album").or(tag_album).unwrap_or_default(),
            duration: duration.unwrap_or(0),
            cover,
        }],
    })
}

fn inspect_manifest(staging: &Path, manifest: &str, kind: &str) -> Result<PackagePreview, String> {
    let v: serde_json::Value = serde_json::from_slice(
        &fs::read(staging.join(manifest)).map_err(|e| format!("{manifest} 读取失败: {e}"))?,
    )
    .map_err(|e| format!("{manifest} 解析失败: {e}"))?;
    let name = v
        .get("name")
        .and_then(|x| x.as_str())
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .unwrap_or_else(|| "未命名".into());
    let root_cover = find_cover_in(staging, staging);
    let mut tracks: Vec<PreviewTrack> = Vec::new();
    for item in v
        .get("tracks")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
    {
        let jstr = |k: &str| -> Option<String> {
            item.get(k)
                .and_then(|x| x.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        // dir 缺省/为空 = 包根;提供时必须过 zip-slip 校验——预览是恶意包
        // 最先触达的入口,`../` 可借 read_preview_tags 读到暂存目录外的文件
        let dir_raw = item.get("dir").and_then(|x| x.as_str()).unwrap_or("");
        let dir = if dir_raw.is_empty() {
            String::new()
        } else {
            match safe_manifest_part(dir_raw) {
                Some(d) => d,
                None => continue,
            }
        };
        let audio_raw = item.get("audio").and_then(|x| x.as_str()).unwrap_or("");
        let Some(audio) = safe_manifest_part(audio_raw) else {
            continue;
        };
        let abs = staging.join(&dir).join(&audio);
        let (tag_title, tag_artist, tag_album, duration) = if abs.is_file() {
            read_preview_tags(&abs)
        } else {
            (None, None, None, None)
        };
        let track_dir = staging.join(&dir);
        let cover = find_cover_in(&track_dir, staging).or_else(|| root_cover.clone());
        tracks.push(PreviewTrack {
            title: jstr("title")
                .or(tag_title)
                .unwrap_or_else(|| "未知标题".into()),
            artist: jstr("artist")
                .or(tag_artist)
                .unwrap_or_else(|| "未知艺人".into()),
            album: jstr("album").or(tag_album).unwrap_or_default(),
            duration: duration.unwrap_or(0),
            cover,
        });
    }
    if tracks.is_empty() {
        return Err("包内没有曲目清单".into());
    }
    Ok(PackagePreview {
        kind: kind.into(),
        name,
        root: staging.to_string_lossy().to_string(),
        staging_id: String::new(),
        tracks,
    })
}

/// 解析音乐包内容供前端预览:解包到 covers/preview 暂存(资产协议已放行),
/// 只读元数据不写库;确认导入走原有命令重新解包到导入目录,取消则整目录清理
#[tauri::command]
async fn inspect_package(path: String, app: tauri::AppHandle) -> Result<PackagePreview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let src = Path::new(&path);
        if !src.is_file() {
            return Err("文件不存在".into());
        }
        let ext = src
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| x.to_lowercase())
            .unwrap_or_default();
        let kind = match ext.as_str() {
            "tmc" | "tmca" | "tmcl" => ext,
            _ => return Err("仅支持 .tmc/.tmca/.tmcl".into()),
        };
        let state = app.state::<AppState>();
        let staging_root = state.cache_dir.join("covers").join("preview");
        fs::create_dir_all(&staging_root).map_err(|e| e.to_string())?;
        let id = format!(
            "{}-{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
            std::process::id()
        );
        let staging = staging_root.join(&id);
        fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
        if let Err(e) = sevenz_rust2::decompress_file(src, &staging) {
            let _ = fs::remove_dir_all(&staging);
            return Err(format!("解包失败: {e}"));
        }
        let parsed = match kind.as_str() {
            "tmc" => inspect_tmc(&staging),
            "tmca" => inspect_manifest(&staging, "album.json", "tmca"),
            _ => inspect_manifest(&staging, "playlist.json", "tmcl"),
        };
        match parsed {
            Ok(mut preview) => {
                preview.staging_id = id;
                Ok(preview)
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&staging);
                Err(e)
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 清理一个预览暂存目录:前端在确认导入完成或取消时调用
#[tauri::command]
fn discard_preview(id: String, state: tauri::State<AppState>) -> Result<(), String> {
    // 只认纯目录名:阻断任何路径穿越企图
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err("非法的暂存 id".into());
    }
    let dir = state.cache_dir.join("covers").join("preview").join(&id);
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 拉走并清空待预览的音乐包路径:启动期前端未就绪时 handle_open_paths 会先入队
#[tauri::command]
fn take_pending_imports(state: tauri::State<AppState>) -> Vec<String> {
    state
        .pending_imports
        .lock()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default()
}

/// 显示桌面歌词小窗(常驻窗口,启动即创建;显示/隐藏状态由前端记忆)
#[tauri::command]
fn show_lyrics_window(app: tauri::AppHandle) -> Result<(), String> {
    let w = app.get_webview_window("lyrics").ok_or("歌词窗口不存在")?;
    let _ = w.show();
    Ok(())
}

/// 隐藏桌面歌词小窗
#[tauri::command]
fn hide_lyrics_window(app: tauri::AppHandle) -> Result<(), String> {
    let w = app.get_webview_window("lyrics").ok_or("歌词窗口不存在")?;
    let _ = w.hide();
    Ok(())
}

/// 处理"打开方式"/命令行传入的文件:.tmc 解包导入,音频复制进导入目录,完成后重扫
fn handle_open_paths(app: &tauri::AppHandle, args: &[String]) {
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

/// 为一次导出创建 Temp 暂存目录:包裹先打包在这里,用户点「导出」后才落位到所选位置
#[tauri::command]
fn make_stage_dir() -> Result<String, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("taurimusic-stage-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.to_string_lossy().to_string())
}

/// 清理暂存目录(落位完成或用户取消后);只认应用自己的暂存目录名,防误删
#[tauri::command]
fn cleanup_stage(dir: String) {
    let p = PathBuf::from(&dir);
    // 纵深防御:除了目录名前缀,还要求父目录确实是系统 Temp
    let in_temp = p.starts_with(std::env::temp_dir());
    let ours = p
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with("taurimusic-stage-"))
        .unwrap_or(false);
    if ours && in_temp {
        let _ = fs::remove_dir_all(p);
    }
}

/// 把曲库中的一首歌打包进导出暂存区(不落位),返回暂存文件路径
#[tauri::command]
async fn stage_tmc(id: String, staging: String, app: tauri::AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || stage_tmc_blocking(&id, Path::new(&staging), &app))
        .await
        .map_err(|e| e.to_string())?
}

fn stage_tmc_blocking(id: &str, staging: &Path, app: &tauri::AppHandle) -> Result<String, String> {
    // 用户已取消(暂存目录被清理):拒绝,避免工作目录创建时把暂存目录重建出来
    if !staging.is_dir() {
        return Err("导出已取消".into());
    }
    let state = app.state::<AppState>();
    let track = {
        let lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        lib.tracks.iter().find(|t| t.id == id).cloned()
    };
    let Some(t) = track else {
        return Err("曲目不存在".into());
    };
    let audio = Path::new(&t.path);
    if !audio.is_file() {
        return Err(format!("音频文件不存在: {}", t.path));
    }
    let ext = audio
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase().to_string())
        .unwrap_or_else(|| "mp3".into());
    let stem = sanitize_name(&t.title);

    // meta.json 与 7z 都写在暂存区的工作子目录里,打包封口后移到暂存区顶层;
    // meta 的音频 SHA 来自打包时的单遍哈希,回调里才落盘,不再预读第二遍
    let workspace = staging.join(".tmp");
    fs::create_dir_all(&workspace).map_err(|e| e.to_string())?;
    let meta_path = workspace.join("meta.json");

    // 条目先收集成表再打包;打开失败必须报错,不给 SevenZWriter 静默写空条目的机会
    let mut assets: ExportAssets = Vec::new();
    let audio_key = format!("{stem}.{ext}");
    fs::File::open(audio).map_err(|e| format!("打开音频失败 {}: {e}", audio.display()))?;
    assets.push((audio_key.clone(), audio.to_path_buf()));
    if let Some(lrc) = &t.lrc_path {
        let lrc_path = Path::new(lrc);
        if lrc_path.is_file() {
            assets.push((format!("{stem}.lrc"), lrc_path.to_path_buf()));
        }
    }
    if let Some(cover) = &t.cover {
        let cover_path = Path::new(cover);
        if cover_path.is_file() {
            let cext = cover_path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase().to_string())
                .unwrap_or_else(|| "jpg".into());
            let cname = if cext == "png" {
                "cover.png"
            } else {
                "cover.jpg"
            };
            assets.push((cname.to_string(), cover_path.to_path_buf()));
        }
    }

    // 7z 先写入工作目录,打包封口后落进暂存区,用户目录不会出现半成品
    let tmp_archive = workspace.join(format!("{stem}.tmc"));
    let _ = fs::remove_file(&tmp_archive);
    pack_with_hashes(app, &tmp_archive, &assets, "TMC", &mut |hashes| {
        let Some(audio_sha) = hashes.get(&audio_key) else {
            return Err("音频哈希缺失".into());
        };
        let meta = serde_json::json!({
            "title": t.title,
            "artist": t.artist,
            "album": t.album,
            "albumArtist": t.album_artist,
            "year": t.year,
            "trackNo": t.track_no,
            "genre": t.genre,
            "sha256": audio_sha,
        });
        fs::write(
            &meta_path,
            serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        Ok(("meta.json".to_string(), meta_path.clone()))
    })?;
    // 同一批次里同标题的歌在暂存区内自动加序号,落位时再按目标目录去重
    let staged = dedup_dest(staging.join(format!("{stem}.tmc")));
    place_archive(&tmp_archive, &staged)?;
    let _ = fs::remove_dir_all(&workspace);
    Ok(staged.to_string_lossy().to_string())
}

#[tauri::command]
async fn pick_tmc_file() -> Option<String> {
    rfd::AsyncFileDialog::new()
        .set_title("选择音乐包(TMC / TMCL)")
        .add_filter("音乐包 (*.tmc, *.tmcl, *.tmca)", &["tmc", "tmcl", "tmca"])
        .pick_file()
        .await
        .map(|f| f.path().to_string_lossy().to_string())
}

#[tauri::command]
async fn pick_audio_files() -> Option<Vec<String>> {
    rfd::AsyncFileDialog::new()
        .set_title("选择音乐文件")
        .add_filter(
            "音频文件",
            &["mp3", "m4a", "flac", "ogg", "oga", "opus", "wav"],
        )
        .pick_files()
        .await
        .map(|files| {
            files
                .iter()
                .map(|f| f.path().to_string_lossy().to_string())
                .collect()
        })
}

/// 批量导出的目标文件夹
#[tauri::command]
async fn pick_export_dir() -> Option<String> {
    rfd::AsyncFileDialog::new()
        .set_title("选择导出位置")
        .pick_folder()
        .await
        .map(|f| f.path().to_string_lossy().to_string())
}

/// 把暂存好的完整包裹落位到用户目录:同名自动加序号,
/// 单个失败不中断整批;返回实际落位路径(数量少于入参即有失败)。
#[tauri::command]
async fn place_staged(files: Vec<String>, dest_dir: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = Path::new(&dest_dir);
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let mut placed = Vec::new();
        for f in &files {
            let src = Path::new(f);
            let Some(name) = src.file_name() else {
                continue;
            };
            let dest = dedup_dest(dir.join(name));
            if place_archive(src, &dest).is_ok() {
                placed.push(dest.to_string_lossy().to_string());
            }
        }
        Ok(placed)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 把资源表写进暂存区压缩包(进度按条目上报)。
/// 音频本身已是压缩格式,采用 7z 的 Copy(存储)方法:打包接近纯拷贝速度,
/// meta/歌词体积可忽略;标准 7z 方法,任何解压工具可读
/// 字节序列转小写十六进制
fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Read 包装:边被读取边计算 SHA-256(导出打包单遍完成"哈希收集+写入")
struct HashingReader<R: std::io::Read> {
    inner: R,
    hasher: sha2::Sha256,
}
impl<R: std::io::Read> std::io::Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.hasher.update(&buf[0..n]);
        }
        Ok(n)
    }
}

/// manifest 构建回调:输入 归档名→SHA 的收集结果,返回 (manifest 归档名, manifest 文件路径)
type ManifestBuilder<'a> =
    &'a mut dyn FnMut(&HashMap<String, String>) -> Result<(String, PathBuf), String>;

/// 打包内容资源并顺带收集各文件 SHA-256:每条资源用 HashingReader 边喂边算,
/// 内容全部打包完成后回调生成 manifest(调用方写文件),作为最后一条条目推入再封口。
/// 7z 条目顺序无语义要求;这样每个文件只读一遍盘(旧流程哈希、打包各读一遍)
fn pack_with_hashes(
    app: &tauri::AppHandle,
    tmp_archive: &Path,
    assets: &ExportAssets,
    what: &str,
    build_manifest: ManifestBuilder<'_>,
) -> Result<(), String> {
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod};
    let mut writer =
        ArchiveWriter::create(tmp_archive).map_err(|e| format!("创建 {what} 失败: {e}"))?;
    writer.set_content_methods(vec![EncoderConfiguration::new(EncoderMethod::COPY)]);
    let mut hashes: HashMap<String, String> = HashMap::new();
    let total = assets.len() + 1;
    for (i, (name, path)) in assets.iter().enumerate() {
        let _ = app.emit(
            "export-progress",
            serde_json::json!({ "done": i, "total": total }),
        );
        let file =
            fs::File::open(path).map_err(|e| format!("打开 {} 失败: {e}", path.display()))?;
        let mut hr = HashingReader {
            inner: file,
            hasher: sha2::Sha256::new(),
        };
        writer
            .push_archive_entry(ArchiveEntry::from_path(path, name.clone()), Some(&mut hr))
            .map_err(|e| e.to_string())?;
        let digest: String = to_hex(hr.hasher.finalize().as_slice());
        hashes.insert(name.clone(), digest);
    }
    let (manifest_name, manifest_path) = build_manifest(&hashes)?;
    let _ = app.emit(
        "export-progress",
        serde_json::json!({ "done": assets.len(), "total": total }),
    );
    let file = fs::File::open(&manifest_path)
        .map_err(|e| format!("打开 {} 失败: {e}", manifest_path.display()))?;
    writer
        .push_archive_entry(
            ArchiveEntry::from_path(&manifest_path, manifest_name),
            Some(file),
        )
        .map_err(|e| e.to_string())?;
    writer
        .finish()
        .map_err(|e| format!("写入 {what} 失败: {e}"))?;
    Ok(())
}

/// 把打包时收集到的哈希按归档名注入 manifest 条目:
/// audio→sha256、lrc→lrcSha256、cover→coverSha256
fn inject_manifest_shas(items: &mut [serde_json::Value], hashes: &HashMap<String, String>) {
    for item in items {
        let Some(obj) = item.as_object_mut() else {
            continue;
        };
        for (sha_key, name_key) in [
            ("sha256", "audio"),
            ("lrcSha256", "lrc"),
            ("coverSha256", "cover"),
        ] {
            if let Some(h) = obj
                .get(name_key)
                .and_then(|v| v.as_str())
                .and_then(|n| hashes.get(n))
            {
                obj.insert(sha_key.to_string(), serde_json::json!(h));
            }
        }
    }
}

/// 导出资源表:包内条目名 → 源文件路径
type ExportAssets = Vec<(String, PathBuf)>;
/// 规划结果:(资源表, manifest 的 tracks 节点)
type PlannedAssets = (ExportAssets, Vec<serde_json::Value>);

/// 把曲目按「music/NN. 名字」规划进导出资源表:每首一个子目录,内含音频、
/// 同名 .lrc、专属封面与 meta.json(供扫描器补齐无内嵌标签的文件)。
/// TMCL 与 TMCA 的打包共用此规划;SHA 由打包阶段单遍收集后注入,这里不做哈希。
/// 音频文件缺失的条目静默跳过
fn plan_track_assets(tracks: &[model::Track], workspace: &Path) -> Result<PlannedAssets, String> {
    let mut assets: Vec<(String, PathBuf)> = Vec::new();
    let mut items: Vec<serde_json::Value> = Vec::new();
    for (i, t) in tracks.iter().enumerate() {
        let audio = Path::new(&t.path);
        if !audio.is_file() {
            continue; // 文件缺失的条目静默跳过(名称已在曲库核对过)
        }
        let ext = audio
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase().to_string())
            .unwrap_or_else(|| "mp3".into());
        let stem = format!("{} - {}", sanitize_name(&t.title), sanitize_name(&t.artist));
        let folder = format!("music/{:02}. {stem}", i + 1);
        let audio_name = format!("{stem}.{ext}");

        assets.push((format!("{folder}/{audio_name}"), audio.to_path_buf()));
        let mut item = serde_json::json!({
            "dir": folder,
            "audio": audio_name,
            "title": t.title,
            "artist": t.artist,
            "album": t.album,
            "albumArtist": t.album_artist,
            "duration": t.duration,
        });
        if let Some(lrc) = &t.lrc_path {
            let lp = Path::new(lrc);
            if lp.is_file() {
                let lrc_name = format!("{stem}.lrc");
                assets.push((format!("{folder}/{lrc_name}"), lp.to_path_buf()));
                item["lrc"] = serde_json::json!(lrc_name);
            }
        }
        if let Some(cover) = &t.cover {
            let cp = Path::new(cover);
            if cp.is_file() {
                let cext = cp
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase().to_string())
                    .unwrap_or_else(|| "jpg".into());
                let cover_name = format!("{stem}.cover.{cext}");
                assets.push((format!("{folder}/{cover_name}"), cp.to_path_buf()));
                item["cover"] = serde_json::json!(cover_name);
            }
        }
        // meta.json 供扫描器补齐无内嵌标签的文件;写在临时目录再随包
        let meta_dir = workspace.join(format!("{i:02}"));
        fs::create_dir_all(&meta_dir).map_err(|e| e.to_string())?;
        let meta = serde_json::json!({
            "title": t.title,
            "artist": t.artist,
            "album": t.album,
            "albumArtist": t.album_artist,
            "year": t.year,
            "trackNo": t.track_no,
            "genre": t.genre,
        });
        let meta_path = meta_dir.join("meta.json");
        fs::write(
            &meta_path,
            serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        assets.push((format!("{folder}/meta.json"), meta_path));

        items.push(item);
    }
    Ok((assets, items))
}

// ===== TMCA 专辑包(.tmcl 的专辑版:单文件整张专辑,导入只入库不建歌单) =====

/// 把整张专辑打包进导出暂存区(不落位),返回暂存文件路径
#[tauri::command]
async fn stage_tmca(
    album_key: String,
    staging: String,
    app: tauri::AppHandle,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        stage_tmca_blocking(&album_key, Path::new(&staging), &app)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn stage_tmca_blocking(
    album_key: &str,
    staging: &Path,
    app: &tauri::AppHandle,
) -> Result<String, String> {
    // 用户已取消(暂存目录被清理):拒绝,避免工作目录创建时把暂存目录重建出来
    if !staging.is_dir() {
        return Err("导出已取消".into());
    }
    let state = app.state::<AppState>();
    let mut tracks: Vec<model::Track> = {
        let lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        lib.tracks
            .iter()
            .filter(|t| format!("{}\u{1}{}", t.album_artist, t.album) == album_key)
            .cloned()
            .collect()
    };
    // 与专辑详情页一致的排序:碟号 → 曲目号 → 标题
    tracks.sort_by(|a, b| {
        (
            a.disc_no.unwrap_or(1),
            a.track_no.unwrap_or(0),
            a.title.as_str(),
        )
            .cmp(&(
                b.disc_no.unwrap_or(1),
                b.track_no.unwrap_or(0),
                b.title.as_str(),
            ))
    });
    // 音频缺失的曲目不能进专辑包,先滤掉(全部缺失时报错)
    tracks.retain(|t| Path::new(&t.path).is_file());
    if tracks.is_empty() {
        return Err("专辑中没有可导出的文件".into());
    }
    let album_name = tracks[0].album.clone();

    let workspace = staging.join(".tmp");
    let _ = fs::remove_dir_all(&workspace);
    fs::create_dir_all(&workspace).map_err(|e| e.to_string())?;

    let (mut assets, mut items) = plan_track_assets(&tracks, &workspace)?;

    // 专辑封面是专辑级属性,包内只保留一份:去重后放包根 cover.{ext},
    // 导入时再分发到各曲目目录(存在多种不同封面的特殊专辑退回逐曲目携带)
    let root_cover = dedupe_album_cover_assets(&mut assets);
    let mut items = match &root_cover {
        Some(_) => {
            for item in &mut items {
                if let Some(obj) = item.as_object_mut() {
                    obj.remove("cover");
                }
            }
            items
        }
        None => items,
    };

    // 7z 写进暂存区工作目录,写入进度按资源数上报;封口后落进暂存区顶层。
    // manifest 最后打包:曲目/封面 SHA 来自打包时的单遍哈希,每个文件只读一遍盘
    let manifest_path = workspace.join("album.json");
    let tmp_archive = workspace.join("album.tmca");
    pack_with_hashes(app, &tmp_archive, &assets, "TMCA", &mut |hashes| {
        inject_manifest_shas(&mut items, hashes);
        let cover_sha = root_cover.as_ref().and_then(|n| hashes.get(n));
        let manifest = serde_json::json!({
            "format": "tmca",
            "version": 1,
            "name": album_name,
            "albumArtist": tracks[0].album_artist,
            "year": tracks[0].year,
            "genre": tracks[0].genre,
            "cover": root_cover,
            "coverSha256": cover_sha,
            "createdAt": now_secs(),
            "tracks": items,
        });
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        Ok(("album.json".to_string(), manifest_path.clone()))
    })?;
    // 包名取清洗后的专辑名;与暂存区内已有文件重名时自动加序号
    let staged = dedup_dest(staging.join(format!("{}.tmca", sanitize_name(&album_name))));
    place_archive(&tmp_archive, &staged)?;
    let _ = fs::remove_dir_all(&workspace);
    Ok(staged.to_string_lossy().to_string())
}

/// TMCA 专辑包封面去重:封面条目(名字形如 music/NN. xxx/{stem}.cover.{ext})按源文件去重,
/// 只有唯一来源时移除全部逐曲目副本、在资源表第 2 位插入包根 cover.{ext};
/// 存在多种不同封面(特殊版本)返回 None,调用方保持原样(正确性优先)
fn dedupe_album_cover_assets(assets: &mut ExportAssets) -> Option<String> {
    let cover_idxs: Vec<usize> = assets
        .iter()
        .enumerate()
        .filter(|(_, (name, _))| {
            // 封面条目必须以 ".cover." + 图片扩展结尾:标题本身含 ".cover." 的
            // 音频条目(如 "Track.cover Remix.mp3")不能被误当成封面移出资源表
            let n = Path::new(name)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            n.rsplit_once(".cover.")
                .map(|(_, ext)| matches!(ext, "jpg" | "jpeg" | "png" | "webp"))
                .unwrap_or(false)
        })
        .map(|(i, _)| i)
        .collect();
    if cover_idxs.is_empty() {
        return None;
    }
    let first_src = assets[cover_idxs[0]].1.clone();
    if cover_idxs.iter().any(|&i| assets[i].1 != first_src) {
        return None;
    }
    let ext = Path::new(&first_src)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("jpg")
        .to_lowercase();
    let root_name = format!("cover.{ext}");
    for &i in cover_idxs.iter().rev() {
        assets.remove(i);
    }
    assets.insert(1, (root_name.clone(), first_src));
    Some(root_name)
}

/// 把 TMCA 包根的单份封面复制进各曲目目录(无副本时才复制,避免重复覆盖)。
/// 扫描器按目录级 cover.{ext} 约定取用;包内无根封面(特殊多封面专辑)则不动
fn distribute_pack_cover(dest: &Path, tracks_json: &[serde_json::Value]) {
    let root = ["cover.jpg", "cover.png", "cover.webp", "cover.jpeg"]
        .iter()
        .map(|n| dest.join(n))
        .find(|p| p.is_file());
    let Some(root) = root else { return };
    let Some(name) = root.file_name() else { return };
    for item in tracks_json {
        let Some(dir) = item
            .get("dir")
            .and_then(|x| x.as_str())
            .and_then(safe_manifest_part)
        else {
            continue;
        };
        let dir_path = dest.join(dir);
        if !dir_path.is_dir() {
            continue;
        }
        let target = dir_path.join(name);
        if !target.exists() {
            let _ = fs::copy(&root, &target);
        }
    }
}

/// 依据 manifest 的 tracks 字段为每个子目录写 meta.json(扫描器据此补齐无内嵌标签的文件)。
/// 包内自带的 meta.json 信息更全(含 year/trackNo/genre),已存在时不覆盖
fn write_manifest_meta_files(dest: &Path, tracks_json: &[serde_json::Value]) {
    for item in tracks_json {
        let Some(dir) = item
            .get("dir")
            .and_then(|x| x.as_str())
            .and_then(safe_manifest_part)
        else {
            continue;
        };
        let dir_path = dest.join(dir);
        let _ = fs::create_dir_all(&dir_path);
        let meta_path = dir_path.join("meta.json");
        if meta_path.is_file() {
            continue;
        }
        let meta = serde_json::json!({
            "title": item.get("title"),
            "artist": item.get("artist"),
            "album": item.get("album"),
            "albumArtist": item.get("albumArtist"),
        });
        let _ = fs::write(
            meta_path,
            serde_json::to_string_pretty(&meta).unwrap_or_default(),
        );
    }
}

/// 导入 .tmca 专辑包:解包到导入目录 → 校验/去重 → 扫描入库;专辑归类由元数据成立
#[tauri::command]
async fn import_tmca(path: String, app: tauri::AppHandle) -> Result<PackImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || import_tmca_blocking(&path, &app))
        .await
        .map_err(|e| e.to_string())?
}

fn import_tmca_blocking(path: &str, app: &tauri::AppHandle) -> Result<PackImportReport, String> {
    let src = Path::new(path);
    if !src.is_file() {
        return Err("文件不存在".into());
    }
    let state = app.state::<AppState>();
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "album".into());
    let import_dir = state.data_dir.join(IMPORT_DIR_NAME);
    fs::create_dir_all(&import_dir).map_err(|e| e.to_string())?;
    let dest = import_dir.join(sanitize_name(&stem));
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| format!("清理旧目录失败: {e}"))?;
    }
    fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    sevenz_rust2::decompress_file(src, &dest).map_err(|e| format!("解包失败: {e}"))?;

    let manifest_path = dest.join("album.json");
    if !manifest_path.is_file() {
        return Err("不是有效的 TMCA:缺少 album.json".into());
    }
    let v: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("album.json 解析失败: {e}"))?;
    if v.get("format").and_then(|x| x.as_str()) != Some("tmca") {
        return Err("不是有效的 TMCA:format 不符".into());
    }
    let album_name = v
        .get("name")
        .and_then(|x| x.as_str())
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .unwrap_or_else(|| stem.clone());
    let tracks_json = v
        .get("tracks")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();

    // 逐文件 SHA-256 校验 + 与曲库内容去重
    let (tracks_json, skipped) = verify_and_dedupe_pack(&dest, tracks_json, &state)?;

    // 为无内嵌标签的文件补 meta.json(扫描器据此补齐元数据)
    write_manifest_meta_files(&dest, &tracks_json);

    // 包级封面分发到各曲目目录(目录级封面约定);曲目带内嵌封面时扫描器自动优先内嵌
    distribute_pack_cover(&dest, &tracks_json);

    // 扫描入库(与拖拽导入同一路径);专辑归类由 meta.json/内嵌标签成立
    scanner::run_scan(app)?;

    // 来源包记录的 SHA 写进曲库,供后续包导入做内容去重
    record_pack_shas(&state, &dest, &tracks_json)?;

    let _ = app.emit("library-changed", ());
    // 全部曲目都与曲库重复:不是错误,作为正常跳过返回(导入预览里明确提示)
    if tracks_json.is_empty() && skipped > 0 {
        return Ok(PackImportReport {
            name: album_name,
            imported: 0,
            skipped,
        });
    }
    Ok(PackImportReport {
        name: album_name,
        imported: tracks_json.len() as u32,
        skipped,
    })
}

// ===== 在线元数据补全(网易云公开接口,仅补封面/歌词,不涉及流媒体) =====

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseReport {
    cover: bool,
    lyrics: u32,
    /// 其中由 LRCLIB 兜底补上的数量(含在 lyrics 内)
    lyrics_lrclib: u32,
    /// 其中由酷我兜底补上的数量(含在 lyrics 内)
    lyrics_kuwo: u32,
    skipped: u32,
}

/// 为整张专辑在线匹配封面与歌词:封面存为曲目目录 cover.jpg,歌词存为同名 .lrc
#[tauri::command]
async fn netease_enrich_album(
    album_key: String,
    app: tauri::AppHandle,
) -> Result<NeteaseReport, String> {
    tauri::async_runtime::spawn_blocking(move || netease_enrich_blocking(&album_key, &app))
        .await
        .map_err(|e| e.to_string())?
}

fn netease_enrich_blocking(
    album_key: &str,
    app: &tauri::AppHandle,
) -> Result<NeteaseReport, String> {
    let state = app.state::<AppState>();

    // 触碰音频文件 mtime,让增量扫描感知旁路元数据(cover.jpg/.lrc)已更新
    fn touch_audio(path: &Path) {
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .and_then(|f| f.set_modified(std::time::SystemTime::now()));
    }

    let tracks: Vec<model::Track> = {
        let lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        lib.tracks
            .iter()
            .filter(|t| format!("{}\u{1}{}", t.album_artist, t.album) == album_key)
            .cloned()
            .collect()
    };
    if tracks.is_empty() {
        return Err("未找到该专辑的曲目".into());
    }

    let client = netease_client();

    let mut report = NeteaseReport {
        cover: false,
        lyrics: 0,
        lyrics_lrclib: 0,
        lyrics_kuwo: 0,
        skipped: 0,
    };

    // 封面:存在无封面曲目时才匹配(网易云 → iTunes → 酷我 三级兜底)
    if tracks.iter().any(|t| t.cover.is_none()) {
        let first = &tracks[0];
        let pic_url = netease_album_cover(&client, &first.album, &first.album_artist, &first.title)
            .or_else(|| itunes_album_cover(&first.album, &first.album_artist))
            .or_else(|| {
                kuwo_album_cover(
                    &generic_client(),
                    &first.title,
                    &first.album_artist,
                    Some(first.duration),
                )
            });
        match pic_url {
            Some(pic_url) => match fetch_capped_allow_http(&generic_client(), &pic_url) {
                Some(bytes) => {
                    let mut dirs: Vec<PathBuf> = tracks
                        .iter()
                        .filter(|t| t.cover.is_none())
                        .filter_map(|t| Path::new(&t.path).parent().map(|p| p.to_path_buf()))
                        .collect();
                    dirs.sort();
                    dirs.dedup();
                    // 混居目录(同一目录里还有其他专辑的曲目)不能写共享 cover.jpg,
                    // 否则目录里所有专辑都会套上同一张封面
                    let multi_dirs: std::collections::HashSet<PathBuf> = {
                        let lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
                        let mut dir_albums: std::collections::HashMap<
                            PathBuf,
                            std::collections::HashSet<String>,
                        > = std::collections::HashMap::new();
                        for t in &lib.tracks {
                            if let Some(p) = Path::new(&t.path).parent() {
                                dir_albums
                                    .entry(p.to_path_buf())
                                    .or_default()
                                    .insert(format!("{}\u{1}{}", t.album_artist, t.album));
                            }
                        }
                        dirs.iter()
                            .filter(|d| dir_albums.get(*d).map(|s| s.len() > 1).unwrap_or(false))
                            .cloned()
                            .collect()
                    };
                    for d in &dirs {
                        if multi_dirs.contains(d) {
                            // 写曲目专属旁路封面:{歌名}.cover.jpg
                            for t in tracks.iter().filter(|t| t.cover.is_none()) {
                                let tp = Path::new(&t.path);
                                if tp.parent().map(|p| p == d.as_path()).unwrap_or(false) {
                                    let stem = tp
                                        .file_stem()
                                        .map(|s| s.to_string_lossy().to_string())
                                        .unwrap_or_default();
                                    let _ = fs::write(d.join(format!("{stem}.cover.jpg")), &bytes);
                                }
                            }
                        } else {
                            let _ = fs::write(d.join("cover.jpg"), &bytes);
                        }
                    }
                    // 让该专辑所有无封面曲目在增量扫描中被重新解析
                    for t in tracks.iter().filter(|t| t.cover.is_none()) {
                        touch_audio(Path::new(&t.path));
                    }
                    report.cover = true;
                }
                None => report.skipped += 1,
            },
            None => report.skipped += 1,
        }
    }

    // 歌词:批量并行匹配无 .lrc 的曲目(网易云 → LRCLIB → 酷我 三级兜底)
    let targets: Vec<&model::Track> = tracks.iter().filter(|t| t.lrc_path.is_none()).collect();
    let results: Vec<Option<(String, &'static str)>> = std::thread::scope(|scope| {
        let handles: Vec<_> = targets
            .chunks(4)
            .map(|chunk| {
                let client = &client;
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|t| {
                            netease_lyric(client, &t.title, &t.artist, Some(t.duration))
                                .map(|text| (text, "netease"))
                                .or_else(|| {
                                    lrclib_lyric(
                                        &generic_client(),
                                        &t.title,
                                        &t.artist,
                                        Some(t.duration),
                                    )
                                    .map(|text| (text, "lrclib"))
                                })
                                .or_else(|| {
                                    kuwo_lyric(
                                        &generic_client(),
                                        &t.title,
                                        &t.artist,
                                        Some(t.duration),
                                    )
                                    .map(|text| (text, "kuwo"))
                                })
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    for (t, result) in targets.iter().zip(results) {
        match result {
            Some((text, source)) => {
                let parent = Path::new(&t.path)
                    .parent()
                    .unwrap_or_else(|| Path::new("."));
                let stem = Path::new(&t.path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "track".into());
                let dest = parent.join(format!("{stem}.lrc"));
                if fs::write(&dest, text).is_ok() {
                    touch_audio(Path::new(&t.path));
                    report.lyrics += 1;
                    match source {
                        "lrclib" => report.lyrics_lrclib += 1,
                        "kuwo" => report.lyrics_kuwo += 1,
                        _ => {}
                    }
                } else {
                    report.skipped += 1;
                }
            }
            None => report.skipped += 1,
        }
    }
    Ok(report)
}

/// 封面图片大小上限:URL 来自外部接口的 JSON,不能无限下载落盘
const COVER_MAX_BYTES: u64 = 20 * 1024 * 1024;

/// 下载封面:不超过大小上限;http 来源的图床先试 https(部分网络对酷我图床的
/// https 不可达),连接失败再回退同路径 http(图片字节有 20MiB 上限,风险可接受)
fn fetch_capped_allow_http(client: &reqwest::blocking::Client, url: &str) -> Option<Vec<u8>> {
    if let Some(rest) = url.strip_prefix("http://") {
        if let Some(bytes) = fetch_capped_inner(client, &format!("https://{rest}"), true) {
            return Some(bytes);
        }
    }
    fetch_capped_inner(client, url, false)
}

fn fetch_capped_inner(
    client: &reqwest::blocking::Client,
    url: &str,
    https_only: bool,
) -> Option<Vec<u8>> {
    use std::io::Read;
    if https_only && !url.starts_with("https://") {
        return None;
    }
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return None;
    }
    let resp = client.get(url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    if resp
        .content_length()
        .is_some_and(|len| len > COVER_MAX_BYTES)
    {
        return None;
    }
    let mut buf = Vec::new();
    resp.take(COVER_MAX_BYTES + 1).read_to_end(&mut buf).ok()?;
    (buf.len() as u64 <= COVER_MAX_BYTES).then_some(buf)
}

pub fn netease_client() -> reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::blocking::Client::builder()
                .user_agent(
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
                )
                .default_headers(
                    reqwest::header::HeaderMap::from_iter([
                        (
                            reqwest::header::REFERER,
                            "https://music.163.com".parse().unwrap(),
                        ),
                        (
                            reqwest::header::COOKIE,
                            "os=pc; appver=2.9.7".parse().unwrap(),
                        ),
                    ]),
                )
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new())
        })
        .clone()
}

/// 繁体转简体(进程级单例转换器):normalize_tag 与现场版识别共用
fn to_simplified(s: &str) -> String {
    use character_converter::CharacterConverter;
    static CONVERTER: std::sync::OnceLock<CharacterConverter> = std::sync::OnceLock::new();
    CONVERTER
        .get_or_init(CharacterConverter::new)
        .traditional_to_simplified(s)
}

/// 归一化标签用于跨源比对:繁体转简体 + 小写 + 去空白(本地标签常见繁体/大小写差异)
fn normalize_tag(s: &str) -> String {
    to_simplified(s)
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// 标题是否带现场/演唱会标记:live 需独立成词(避免 Alive/Livin' 误判),
/// 中文无词边界直接子串判。现场版与录音室版的时间轴完全不同,选错版歌词必错位
fn title_is_live(s: &str) -> bool {
    let norm = to_simplified(s).to_lowercase();
    if norm.contains("现场") || norm.contains("演唱会") || norm.contains("实况") {
        return true;
    }
    norm.split(|c: char| !c.is_ascii_alphabetic())
        .any(|w| w == "live")
}

/// 归一化后的双向包含比对
fn name_matches(a: &str, b: &str) -> bool {
    let (a, b) = (normalize_tag(a), normalize_tag(b));
    !a.is_empty() && !b.is_empty() && (a.contains(&b) || b.contains(&a))
}

/// 艺人组任一分段与候选名比对成功即通过
fn artist_matches(artist: &str, candidate: &str) -> bool {
    artist
        .split(['/', '&', ',', '、'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .any(|seg| name_matches(seg, candidate))
}

/// 去掉标题里的括号段(全角/半角):酷我搜索对括号敏感,
/// 「八千里路明月照（挥笔生花问桀骜）」整串搜索命中不了原曲,去掉后能精确命中
fn strip_bracketed(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0u32;
    for c in s.chars() {
        match c {
            '(' | '（' | '【' | '「' | '『' | '[' => depth += 1,
            ')' | '）' | '】' | '」' | '』' | ']' => {
                depth = depth.saturating_sub(1);
                out.push(' ');
            }
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<&str>>().join(" ")
}

/// 酷我搜索关键词:完整标题优先,有括号段时再补一条去括号的
fn kuwo_keywords(title: &str, artist: &str) -> Vec<String> {
    let mut keys = vec![format!("{title} {artist}")];
    let stripped = strip_bracketed(title);
    if !stripped.is_empty() && stripped != title {
        keys.push(format!("{stripped} {artist}"));
    }
    keys
}

fn netease_search(
    client: &reqwest::blocking::Client,
    keyword: &str,
    search_type: u32,
    limit: u32,
) -> Option<serde_json::Value> {
    let resp = client
        .post("https://music.163.com/api/search/get/web")
        .form(&[
            ("s", keyword),
            ("type", &search_type.to_string()),
            ("limit", &limit.to_string()),
        ])
        .send()
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<serde_json::Value>().ok()
}

/// 专辑封面:按专辑名搜专辑并校验艺人,无命中再按歌名搜歌曲取其所属专辑图。
/// 不校验艺人会拿到同名但毫无关系的专辑封面(如「Control」会命中 Janet Jackson)。
pub fn netease_album_cover(
    client: &reqwest::blocking::Client,
    album: &str,
    album_artist: &str,
    fallback_title: &str,
) -> Option<String> {
    if let Some(v) = netease_search(client, album, 10, 5) {
        if let Some(albums) = v.pointer("/result/albums").and_then(|x| x.as_array()) {
            // 优先取艺人对得上的专辑,全对不上时不用专辑搜索结果(大概率是同名的别家)
            if let Some(pic) = albums
                .iter()
                .find(|a| {
                    a["artist"]["name"]
                        .as_str()
                        .map(|n| artist_matches(album_artist, n))
                        .unwrap_or(false)
                })
                .and_then(|a| a["picUrl"].as_str())
            {
                return Some(pic.replace("http://", "https://"));
            }
        }
    }
    // 回退:按歌名搜歌曲,优先艺人对得上的,否则取首个
    let v = netease_search(client, fallback_title, 1, 5)?;
    let songs = v.pointer("/result/songs")?.as_array()?;
    let pic = songs
        .iter()
        .find(|s| {
            s["artists"]
                .as_array()
                .map(|arr| {
                    arr.iter().any(|a| {
                        a["name"]
                            .as_str()
                            .map(|n| artist_matches(album_artist, n))
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        })
        .or_else(|| songs.first())?["album"]["picUrl"]
        .as_str()
        .map(|s| s.replace("http://", "https://"))?;
    Some(pic)
}

/// 歌词:按歌名搜索,优先选艺人名能对上的结果(繁简/大小写归一化比对)
/// 从网易云搜索结果挑最佳歌曲:艺人吻合为底,现场版标题与时长吻合加权。
/// 纯函数便于单测:现场版歌曲若按首条盲取,常拿到录音室版(时间轴完全不同)
fn pick_netease_song<'a>(
    songs: &'a [serde_json::Value],
    artist: &str,
    duration: Option<f64>,
    want_live: bool,
) -> Option<&'a serde_json::Value> {
    let mut best: Option<(u32, &serde_json::Value)> = None;
    for s in songs {
        let mut score = 0u32;
        let artist_ok = s["artists"]
            .as_array()
            .map(|arr| {
                arr.iter().any(|a| {
                    a["name"]
                        .as_str()
                        .map(|n| artist_matches(artist, n))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        if artist_ok {
            score += 4;
        }
        if want_live && s["name"].as_str().map(title_is_live).unwrap_or(false) {
            score += 3;
        }
        // 搜索接口的 duration 单位是毫秒
        if let (Some(d), Some(t)) = (duration, s["duration"].as_f64()) {
            if (d - t / 1000.0).abs() <= 3.0 {
                score += 2;
            }
        }
        if score > 0 && best.as_ref().map(|(b, _)| score > *b).unwrap_or(true) {
            best = Some((score, s));
        }
    }
    best.map(|(_, s)| s)
}

pub fn netease_lyric(
    client: &reqwest::blocking::Client,
    title: &str,
    artist: &str,
    duration: Option<f64>,
) -> Option<String> {
    let want_live = title_is_live(title);
    // 现场版全名搜索常落空(各家括号后缀命名不一),补一条去括号搜索把两个版本都捞进候选
    let mut keywords = vec![title.to_string()];
    let stripped = strip_bracketed(title);
    if want_live && !stripped.is_empty() && stripped != title {
        keywords.push(stripped);
    }
    for kw in keywords {
        let Some(v) = netease_search(client, &kw, 1, 10) else {
            continue;
        };
        let Some(songs) = v.pointer("/result/songs").and_then(|x| x.as_array()) else {
            continue;
        };
        let Some(pick) =
            pick_netease_song(songs, artist, duration, want_live).or_else(|| songs.first())
        else {
            continue;
        };
        let Some(id) = pick["id"].as_i64() else {
            continue;
        };
        let url = format!("https://music.163.com/api/song/lyric?id={id}&lv=1&kv=1&tv=-1");
        let Ok(resp) = client.get(&url).send() else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        if let Ok(v) = resp.json::<serde_json::Value>() {
            if let Some(text) = v
                .pointer("/lrc/lyric")
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return Some(text.to_string());
            }
        }
    }
    None
}

// ===== LRCLIB 歌词兜底 + iTunes 封面兜底(公开接口,无需密钥) =====

/// 通用 HTTP 客户端(LRCLIB / iTunes 等公开接口):仅带标识 UA,不带网易云的站头
pub fn generic_client() -> reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::blocking::Client::builder()
                .user_agent(concat!(
                    "TauriMusic/",
                    env!("CARGO_PKG_VERSION"),
                    " (https://github.com/OArt3misQAQ/TauriMusic)"
                ))
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new())
        })
        .clone()
}

/// 从 LRCLIB 搜索结果里挑最佳一条:艺人能对上才要;
/// 现场版标记优先于同步格式优先于时长吻合(时间轴错了同步歌词毫无意义)
fn pick_lrclib_result(
    items: &[serde_json::Value],
    artist: &str,
    duration: Option<f64>,
    want_live: bool,
) -> Option<String> {
    let mut best: Option<(u8, String)> = None;
    for it in items {
        let Some(name) = it["artistName"].as_str() else {
            continue;
        };
        if !artist_matches(artist, name) {
            continue;
        }
        let synced = it["syncedLyrics"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let plain = it["plainLyrics"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(text) = synced.or(plain) else {
            continue; // 纯音乐条目两字段皆空,跳过
        };
        let mut score = 0u8;
        if want_live && it["trackName"].as_str().map(title_is_live).unwrap_or(false) {
            score += 4; // 现场版标记是版本正确性的最强信号
        }
        if synced.is_some() {
            score += 2; // 逐行同步歌词比纯文本更有价值
        }
        if let (Some(d), Some(t)) = (duration, it["duration"].as_f64()) {
            if (d - t).abs() <= 3.0 {
                score += 1; // 时长吻合基本可断定是同一版本
            }
        }
        if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
            best = Some((score, text.to_string()));
        }
    }
    best.map(|(_, text)| text)
}

/// LRCLIB 歌词兜底:网易云没有时,从这里补逐行同步歌词(或纯文本)
pub fn lrclib_lyric(
    client: &reqwest::blocking::Client,
    title: &str,
    artist: &str,
    duration: Option<f64>,
) -> Option<String> {
    let want_live = title_is_live(title);
    // 1) /api/get 精确匹配:LRCLIB 直接返回最贴合的一条(带时长时按秒对齐)
    let mut get = client
        .get("https://lrclib.net/api/get")
        .query(&[("artist_name", artist), ("track_name", title)]);
    if let Some(d) = duration {
        get = get.query(&[("duration", (d as u64).to_string())]);
    }
    if let Ok(resp) = get.send() {
        if resp.status().is_success() {
            if let Ok(v) = resp.json::<serde_json::Value>() {
                // 与网易云同规则:艺人对不上就不用,避免同名歌错配
                let ok = v["artistName"]
                    .as_str()
                    .map(|n| artist_matches(artist, n))
                    .unwrap_or(false);
                if ok {
                    for key in ["syncedLyrics", "plainLyrics"] {
                        if let Some(text) = v[key].as_str().map(str::trim).filter(|s| !s.is_empty())
                        {
                            return Some(text.to_string());
                        }
                    }
                }
            }
        }
    }
    // 2) /api/search:先「歌名+艺人」,现场版再补去括号变体(把两个版本都捞进候选),最后仅歌名
    let mut queries: Vec<Vec<(&str, String)>> = vec![
        vec![
            ("track_name", title.to_string()),
            ("artist_name", artist.to_string()),
        ],
        vec![("q", title.to_string())],
    ];
    let stripped = if want_live {
        strip_bracketed(title)
    } else {
        String::new()
    };
    if !stripped.is_empty() && stripped != title {
        queries.insert(
            1,
            vec![
                ("track_name", stripped),
                ("artist_name", artist.to_string()),
            ],
        );
    }
    for query in &queries {
        // 单个搜索变体请求失败只影响该变体,不能放弃其余变体
        let Ok(resp) = client
            .get("https://lrclib.net/api/search")
            .query(query)
            .send()
        else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(v) = resp.json::<serde_json::Value>() else {
            continue;
        };
        let Some(items) = v.as_array() else {
            continue;
        };
        if let Some(text) = pick_lrclib_result(items, artist, duration, want_live) {
            return Some(text);
        }
    }
    None
}

/// iTunes 封面兜底:网易云搜不到的专辑(尤其外语/冷门)从苹果曲库补,
/// artworkUrl100 换成 1200x1200 高清图;先查中国区再查美区
pub fn itunes_album_cover(album: &str, album_artist: &str) -> Option<String> {
    let client = generic_client();
    let norm_album = normalize_tag(album);
    for country in ["cn", "us"] {
        // 单区失败(网络抖动/无搜索结果)只能跳过该区,不能从函数提前返回——
        // 否则"先中国区再美区"的兜底永远跑不到美区,而"中国区搜不到"恰恰
        // 是最该去美区再试的场景(冷门外语专辑)
        let Ok(resp) = client
            .get("https://itunes.apple.com/search")
            .query(&[
                ("term", format!("{album_artist} {album}")),
                ("media", "music".to_string()),
                ("entity", "album".to_string()),
                ("limit", "5".to_string()),
                ("country", country.to_string()),
            ])
            .send()
        else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(v) = resp.json::<serde_json::Value>() else {
            continue;
        };
        let Some(results) = v["results"].as_array() else {
            continue;
        };
        let pic = results
            .iter()
            .find(|r| {
                r["artistName"]
                    .as_str()
                    .map(|n| artist_matches(album_artist, n))
                    .unwrap_or(false)
                    && r["collectionName"]
                        .as_str()
                        .map(|n| {
                            name_matches(n, album)
                                || !norm_album.is_empty() && normalize_tag(n).contains(&norm_album)
                        })
                        .unwrap_or(false)
            })
            .or_else(|| {
                // 艺人对上但专辑名对不上时也接受(苹果的专辑命名常带后缀,如 "- Single")
                results.iter().find(|r| {
                    r["artistName"]
                        .as_str()
                        .map(|n| artist_matches(album_artist, n))
                        .unwrap_or(false)
                })
            })
            .and_then(|r| r["artworkUrl100"].as_str())
            .map(|s| s.replace("100x100bb", "1200x1200bb"));
        if let Some(pic) = pic {
            return Some(pic);
        }
    }
    None
}

// ===== 酷我歌词兜底(公开接口,免登录) =====

/// 酷我 r.s 返回单引号伪 JSON,转成严格 JSON:
/// 仅当引号紧邻结构性字符(前一字符为 {[,: 或后一字符为 }],:)时视为定界符,
/// 值内部的撇号(Don't)原样保留;找不到定界符时解析会失败并走优雅降级
fn kuwo_fix_json(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut prev = '\0';
    for (i, &c) in chars.iter().enumerate() {
        if c == '\'' {
            let prev_struct = matches!(prev, '\0' | '{' | '[' | ':' | ',');
            let mut next = ' ';
            for &n in &chars[i + 1..] {
                if !n.is_whitespace() {
                    next = n;
                    break;
                }
            }
            let next_struct = matches!(next, '\0' | '}' | ']' | ':' | ',');
            out.push(if prev_struct || next_struct {
                '"'
            } else {
                '\''
            });
        } else {
            out.push(c);
        }
        prev = c;
    }
    out
}

/// 酷我字段里的 HTML 实体解码(&apos; &nbsp; &amp; 等,数字实体一并处理)
fn html_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        let end = rest.find(';').map(|e| e + 1).unwrap_or(0);
        if end == 0 || end > 10 {
            out.push('&');
            rest = &rest[1..];
            continue;
        }
        let entity = &rest[..end];
        let decoded = match entity {
            "&apos;" | "&#39;" => Some('\''),
            "&quot;" => Some('"'),
            "&nbsp;" => Some('\u{a0}'),
            "&amp;" => Some('&'),
            "&lt;" => Some('<'),
            "&gt;" => Some('>'),
            _ => entity
                .strip_prefix("&#")
                .and_then(|n| n.strip_suffix(';'))
                .and_then(|n| {
                    u32::from_str_radix(
                        n.trim_start_matches('x'),
                        if n.starts_with('x') { 16 } else { 10 },
                    )
                    .ok()
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// 把酷我 lrclist(time 为秒字符串)拼成标准 LRC 文本
fn build_lrc_from_kuwo(lines: &[serde_json::Value]) -> Option<String> {
    let mut out = String::new();
    for l in lines {
        let Some(text) = l["lineLyric"].as_str() else {
            continue;
        };
        let t = l["time"]
            .as_str()
            .and_then(|s| s.trim().parse::<f64>().ok())
            .or_else(|| l["time"].as_f64());
        let Some(t) = t else { continue };
        let mut m = (t / 60.0).floor() as i64;
        let mut s = t - m as f64 * 60.0;
        if s >= 59.995 {
            m += 1;
            s = 0.0;
        }
        out.push_str(&format!("[{m:02}:{s:05.2}]{text}\n"));
    }
    (!out.is_empty()).then(|| out.trim_end().to_string())
}

/// 酷我歌曲搜索:返回 abslist 列表(单引号伪 JSON 已修复为严格 JSON)
fn kuwo_search(
    client: &reqwest::blocking::Client,
    keyword: &str,
) -> Option<Vec<serde_json::Value>> {
    let resp = client
        .get("https://search.kuwo.cn/r.s")
        .query(&[
            ("all", keyword.to_string()),
            ("ft", "music".to_string()),
            ("itemset", "web_2013".to_string()),
            ("client", "mp".to_string()),
            ("pn", "0".to_string()),
            ("rn", "10".to_string()),
            ("rformat", "json".to_string()),
            ("encoding", "utf8".to_string()),
            ("vipver", "MUSIC_8.0.3.2_QQZS".to_string()),
        ])
        .send()
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let raw = resp.text().ok()?;
    let v: serde_json::Value = serde_json::from_str(&kuwo_fix_json(&raw)).ok()?;
    v["abslist"].as_array().cloned()
}

/// 酷我歌曲详情(必须带 m 站 Referer,否则报「音乐查询失败」):返回 data 节点。
/// m 站接口不稳定:同一 rid 会随机返回缺 songinfo 的精简响应(实测约半数),
/// 校验关键字段并重试;重试耗尽时若有歌词行也返回(歌词匹配不依赖 songinfo)
fn kuwo_song_detail(client: &reqwest::blocking::Client, rid: &str) -> Option<serde_json::Value> {
    let mut last: Option<serde_json::Value> = None;
    for _ in 0..3 {
        // m 站三类抖动(请求失败/偶发 502/响应不是 JSON)都按"本次作废,继续重试"处理,
        // 直接 return None 会让整个酷我链路被一次网络抖动降级掉
        let Ok(resp) = client
            .get("https://m.kuwo.cn/newh5/singles/songinfoandlrc")
            .query(&[("musicId", rid)])
            .header(
                reqwest::header::REFERER,
                "https://m.kuwo.cn/newh5/singles/songinfoandlrc",
            )
            .send()
        else {
            continue;
        };
        if !resp.status().is_success() {
            std::thread::sleep(std::time::Duration::from_millis(150));
            continue;
        }
        let Ok(v) = resp.json::<serde_json::Value>() else {
            continue;
        };
        let data = v.get("data").cloned().unwrap_or_default();
        if data.get("songinfo").is_some() {
            return Some(data);
        }
        if data.get("lrclist").is_some() {
            last = Some(data);
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    last
}

/// 酷我歌词兜底:LRCLIB 也没有时再试酷我
pub fn kuwo_lyric(
    client: &reqwest::blocking::Client,
    title: &str,
    artist: &str,
    duration: Option<f64>,
) -> Option<String> {
    for kw in kuwo_keywords(title, artist) {
        let Some(list) = kuwo_search(client, &kw) else {
            continue;
        };
        let matched: Vec<&serde_json::Value> = list
            .iter()
            .filter(|s| {
                s["ARTIST"]
                    .as_str()
                    .map(|n| artist_matches(artist, &html_unescape(n)))
                    .unwrap_or(false)
            })
            .collect();
        let pool: Vec<&serde_json::Value> = if matched.is_empty() {
            list.iter().collect()
        } else {
            matched
        };
        // 评分挑选:现场版标记优先于时长吻合;全 0 分退回首个(与旧行为一致)
        let want_live = title_is_live(title);
        let mut pick = pool.first().copied();
        let mut best = 0u32;
        for s in pool.iter().copied() {
            let mut score = 0u32;
            if want_live
                && s["SONGNAME"]
                    .as_str()
                    .map(|n| title_is_live(&html_unescape(n)))
                    .unwrap_or(false)
            {
                score += 3;
            }
            if let (Some(d), Some(k)) = (
                duration,
                s["DURATION"]
                    .as_str()
                    .and_then(|x| x.trim().parse::<f64>().ok()),
            ) {
                if (d - k).abs() <= 5.0 {
                    score += 2;
                }
            }
            if score > best {
                best = score;
                pick = Some(s);
            }
        }
        let Some(pick) = pick else { continue };
        let Some(rid) = pick["MUSICRID"].as_str() else {
            continue;
        };
        let Some(data) = kuwo_song_detail(client, rid.trim_start_matches("MUSIC_")) else {
            continue;
        };
        if let Some(lines) = data["lrclist"].as_array() {
            if let Some(text) = build_lrc_from_kuwo(lines) {
                return Some(text);
            }
        }
    }
    None
}

/// 酷我封面兜底:网易云与 iTunes 都没有时(抖音神曲/新歌常见),从酷我取官方专辑封面。
/// 搜索选曲后从 songinfo.pic 取图并升级为 900 高清;
/// 本地标签艺人常有错字(如「泛国同学」vs「泽国同学」),艺人全对不上时
/// 只有时长 ±2s 吻合才敢要,防止拿到同名的别家封面
pub fn kuwo_album_cover(
    client: &reqwest::blocking::Client,
    title: &str,
    artist: &str,
    duration: Option<f64>,
) -> Option<String> {
    for kw in kuwo_keywords(title, artist) {
        let Some(list) = kuwo_search(client, &kw) else {
            continue;
        };
        let parse_dur = |s: &serde_json::Value| {
            s["DURATION"]
                .as_str()
                .and_then(|d| d.trim().parse::<f64>().ok())
        };
        let matched: Vec<&serde_json::Value> = list
            .iter()
            .filter(|s| {
                s["ARTIST"]
                    .as_str()
                    .map(|n| artist_matches(artist, &html_unescape(n)))
                    .unwrap_or(false)
            })
            .collect();
        let pick = if !matched.is_empty() {
            matched
                .iter()
                .copied()
                .find(|s| match (duration, parse_dur(s)) {
                    (Some(d), Some(k)) => (d - k).abs() <= 5.0,
                    _ => false,
                })
                .unwrap_or(matched[0])
        } else {
            // 艺人全对不上:只有时长 ±2s 吻合才敢要。本组关键词无吻合时
            // continue 试下一组,直接 ? 返回会让剩余关键词变体全部失效
            match duration.and_then(|d| {
                list.iter()
                    .find(|s| parse_dur(s).map(|k| (d - k).abs() <= 2.0).unwrap_or(false))
            }) {
                Some(p) => p,
                None => continue,
            }
        };
        let Some(rid) = pick["MUSICRID"].as_str() else {
            continue;
        };
        let Some(data) = kuwo_song_detail(client, rid.trim_start_matches("MUSIC_")) else {
            continue;
        };
        if let Some(pic) = data["songinfo"]["pic"].as_str() {
            // 图床升级 900 高清;协议保持原样,https 不可达时由下载侧回退 http
            return Some(pic.replace("/albumcover/240/", "/albumcover/900/"));
        }
    }
    None
}

// ===== 格式关联(HKCU 注册到"打开方式",无需管理员) =====

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteReport {
    pub deleted: u32,
    pub failed: u32,
}

/// 把一个文件移入回收站目录:同卷 rename 原子直达,跨卷退化为拷贝+删源。
/// 拷贝后删源失败视为整体失败(清掉拷贝,原文件留在原处)
fn move_into_trash(src: &Path, trash_dir: &Path, name: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(trash_dir).map_err(|e| format!("创建回收站目录失败: {e}"))?;
    let dest = dedup_dest(trash_dir.join(name));
    if fs::rename(src, &dest).is_ok() {
        return Ok(dest);
    }
    fs::copy(src, &dest).map_err(|e| format!("复制进回收站失败: {e}"))?;
    if let Err(e) = fs::remove_file(src) {
        let _ = fs::remove_file(&dest);
        return Err(format!("移除原文件失败: {e}"));
    }
    Ok(dest)
}

/// 删除曲目:音频与同名歌词移入应用回收站(可还原,超期自动清理),
/// 曲库记录转入回收站条目;导入目录内的空子目录一并清理
#[tauri::command]
async fn delete_tracks(ids: Vec<String>, app: tauri::AppHandle) -> Result<DeleteReport, String> {
    tauri::async_runtime::spawn_blocking(move || delete_tracks_blocking(&ids, &app))
        .await
        .map_err(|e| e.to_string())?
}

fn delete_tracks_blocking(ids: &[String], app: &tauri::AppHandle) -> Result<DeleteReport, String> {
    use model::TrashEntry;

    let state = app.state::<AppState>();
    let import_dir = state.data_dir.join("Music");
    let trash_dir = state.data_dir.join(TRASH_DIR_NAME);

    // 1) 锁内:取出匹配曲目并先从曲库移除(避免长时间持锁做文件搬运)
    let to_delete: Vec<model::Track> = {
        let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        let id_set: std::collections::HashSet<&String> = ids.iter().collect();
        let (del, kept): (Vec<model::Track>, Vec<model::Track>) =
            lib.tracks.drain(..).partition(|t| id_set.contains(&t.id));
        lib.tracks = kept;
        if let Err(e) = lib.save(&state.data_dir.join(LIBRARY_FILE)) {
            // 落盘失败必须回滚内存:否则界面少了歌而磁盘没少,重启后又"复活"
            lib.tracks.extend(del);
            return Err(e.to_string());
        }
        del
    };

    // 2) 锁外:移入应用回收站(音频 + 同名歌词)
    let now = now_secs();
    let mut entries: Vec<TrashEntry> = Vec::new();
    let mut failed_tracks: Vec<model::Track> = Vec::new();
    let mut parents: Vec<PathBuf> = Vec::new();
    for t in &to_delete {
        let audio = Path::new(&t.path);
        let ext = audio
            .extension()
            .map(|x| x.to_string_lossy().to_string())
            .unwrap_or_else(|| "mp3".into());
        let base = format!("{}_{}", t.id, sanitize_name(&t.title));
        match move_into_trash(audio, &trash_dir, &format!("{base}.{ext}")).map(|ta| {
            // 歌词搬运失败降级为"无歌词的回收站条目":音频已经进了回收站,
            // 若整体判失败回填曲库,音频会变成无条目的孤儿——永远不被清理也还原不了
            let tl = match &t.lrc_path {
                Some(l) if Path::new(l).is_file() => {
                    move_into_trash(Path::new(l), &trash_dir, &format!("{base}.lrc")).ok()
                }
                _ => None,
            };
            (ta, tl)
        }) {
            Ok((trashed_audio, trashed_lrc)) => {
                if let Some(parent) = audio.parent() {
                    let parent = parent.to_path_buf();
                    if !parents.contains(&parent) {
                        parents.push(parent);
                    }
                }
                entries.push(TrashEntry {
                    track: t.clone(),
                    trashed_audio: trashed_audio.to_string_lossy().to_string(),
                    trashed_lrc: trashed_lrc.map(|p| p.to_string_lossy().to_string()),
                    deleted_at: now,
                });
            }
            Err(_) => failed_tracks.push(t.clone()),
        }
    }

    // 3) 锁内:回收站条目落库;失败的回填曲库,避免幽灵状态
    {
        let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        lib.trash.extend(entries);
        lib.tracks.extend(failed_tracks.iter().cloned());
        lib.save(&state.data_dir.join(LIBRARY_FILE))
            .map_err(|e| e.to_string())?;
    }
    if !failed_tracks.is_empty() {
        eprintln!(
            "[delete] {} 首移入回收站失败(文件被占用?),已保留曲库记录",
            failed_tracks.len()
        );
    }

    // 4) 导入目录的子目录(非根)如果没有音频文件了,连同残留的 meta/封面一起清掉
    for d in &parents {
        if d.starts_with(&import_dir) && *d != import_dir {
            let has_audio = std::fs::read_dir(d)
                .map(|rd| {
                    rd.filter_map(|e| e.ok()).any(|e| {
                        e.path()
                            .extension()
                            .map(|x| {
                                let x = x.to_string_lossy().to_lowercase();
                                scanner::EXTENSIONS.contains(&x.as_str())
                            })
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false);
            if !has_audio {
                let _ = fs::remove_dir_all(d);
            }
        }
    }

    let failed = failed_tracks.len() as u32;
    Ok(DeleteReport {
        deleted: to_delete.len() as u32 - failed,
        failed,
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub restored: u32,
    pub failed: u32,
}

/// 从回收站还原:文件移回原路径(原位被占则自动改名),曲库记录恢复。
/// 返回 (成功条目及其最终路径, 失败条目)
fn restore_entries(
    entries: Vec<model::TrashEntry>,
) -> (Vec<(model::Track, PathBuf)>, Vec<model::TrashEntry>) {
    let mut restored: Vec<(model::Track, PathBuf)> = Vec::new();
    let mut failed: Vec<model::TrashEntry> = Vec::new();
    for e in entries {
        let audio = Path::new(&e.trashed_audio);
        let dest = Path::new(&e.track.path);
        let restored_audio = move_back(audio, dest);
        let restored_audio = match restored_audio {
            Some(p) => p,
            None => {
                failed.push(e);
                continue;
            }
        };
        let lrc = match (&e.trashed_lrc, &e.track.lrc_path) {
            (Some(trashed), Some(orig)) => move_back(Path::new(trashed), Path::new(orig)),
            _ => None,
        };
        let mut track = e.track.clone();
        track.path = restored_audio.to_string_lossy().to_string();
        if let Some(l) = lrc {
            track.lrc_path = Some(l.to_string_lossy().to_string());
        }
        restored.push((track, restored_audio));
    }
    (restored, failed)
}

/// 把回收站文件移回目标路径;目标存在或同卷 rename 失败时自动换名,
/// 返回最终落位路径;原目录不存在则重建
fn move_back(src: &Path, dest: &Path) -> Option<PathBuf> {
    if !src.is_file() {
        return None;
    }
    if let Some(parent) = dest.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let final_dest = dedup_dest(dest.to_path_buf());
    if fs::rename(src, &final_dest).is_ok() {
        return Some(final_dest);
    }
    if fs::copy(src, &final_dest).is_ok() {
        let _ = fs::remove_file(src);
        return Some(final_dest);
    }
    None
}

/// 还原回收站条目:文件移回原位,曲库记录恢复(触发扫描后重新可见)
#[tauri::command]
async fn restore_tracks(ids: Vec<String>, app: tauri::AppHandle) -> Result<RestoreReport, String> {
    tauri::async_runtime::spawn_blocking(move || restore_tracks_blocking(&ids, &app))
        .await
        .map_err(|e| e.to_string())?
}

fn restore_tracks_blocking(
    ids: &[String],
    app: &tauri::AppHandle,
) -> Result<RestoreReport, String> {
    let state = app.state::<AppState>();
    let all = {
        let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        if ids.is_empty() {
            std::mem::take(&mut lib.trash)
        } else {
            let id_set: std::collections::HashSet<&String> = ids.iter().collect();
            let (sel, rest): (Vec<_>, Vec<_>) = lib
                .trash
                .drain(..)
                .partition(|e| id_set.contains(&e.track.id));
            lib.trash = rest;
            sel
        }
    };
    if all.is_empty() {
        return Ok(RestoreReport {
            restored: 0,
            failed: 0,
        });
    }

    let (restored, mut failed) = restore_entries(all);
    let failed_count = failed.len() as u32;

    {
        let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        for (track, _) in &restored {
            lib.tracks.push(track.clone());
        }
        // 只清掉确实还原成功的 id;失败的条目留在回收站
        let ok_ids: std::collections::HashSet<&str> =
            restored.iter().map(|(t, _)| t.id.as_str()).collect();
        failed.retain(|e| !ok_ids.contains(e.track.id.as_str()));
        lib.trash.extend(failed);
        lib.save(&state.data_dir.join(LIBRARY_FILE))
            .map_err(|e| e.to_string())?;
    }
    let _ = app.emit("library-changed", ());
    Ok(RestoreReport {
        restored: restored.len() as u32,
        failed: failed_count,
    })
}

/// 彻底删除回收站条目:ids 为空表示清空全部。文件直接删除(不可恢复)
#[tauri::command]
async fn purge_trash(ids: Vec<String>, app: tauri::AppHandle) -> Result<u32, String> {
    tauri::async_runtime::spawn_blocking(move || purge_trash_blocking(&ids, &app))
        .await
        .map_err(|e| e.to_string())?
}

fn purge_trash_blocking(ids: &[String], app: &tauri::AppHandle) -> Result<u32, String> {
    let state = app.state::<AppState>();
    let purged: Vec<model::TrashEntry> = {
        let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        if ids.is_empty() {
            std::mem::take(&mut lib.trash)
        } else {
            let id_set: std::collections::HashSet<&String> = ids.iter().collect();
            let (sel, rest): (Vec<_>, Vec<_>) = lib
                .trash
                .drain(..)
                .partition(|e| id_set.contains(&e.track.id));
            lib.trash = rest;
            sel
        }
    };
    for e in &purged {
        let _ = fs::remove_file(&e.trashed_audio);
        if let Some(l) = &e.trashed_lrc {
            let _ = fs::remove_file(l);
        }
    }
    if !purged.is_empty() {
        let lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        lib.save(&state.data_dir.join(LIBRARY_FILE))
            .map_err(|e| e.to_string())?;
    }
    Ok(purged.len() as u32)
}

/// 清理超过保留期的回收站条目(启动时调用)
fn purge_expired_trash(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let cutoff = now_secs() - TRASH_RETENTION_DAYS * 86400.0;
    let expired: Vec<model::TrashEntry> = {
        let mut lib = state.lib.lock().unwrap_or_else(|e| e.into_inner());
        let (exp, keep): (Vec<model::TrashEntry>, Vec<model::TrashEntry>) =
            lib.trash.drain(..).partition(|e| e.deleted_at < cutoff);
        // 未过期的条目必须放回去:曾在这里被整表丢弃,导致每次启动都把
        // 回收站清空——未过期条目既不能还原也永远不会被清理(孤儿文件)
        lib.trash = keep;
        exp
    };
    if expired.is_empty() {
        return;
    }
    for e in &expired {
        let _ = fs::remove_file(&e.trashed_audio);
        if let Some(l) = &e.trashed_lrc {
            let _ = fs::remove_file(l);
        }
    }
    if let Ok(lib) = state.lib.lock() {
        let _ = lib.save(&state.data_dir.join(LIBRARY_FILE));
    }
    eprintln!("[trash] 清理超期回收站条目 {} 条", expired.len());
}

// ===== 播放列表(用户歌单:创建/改名/增删曲目/排序/导出导入) =====

/// io::Write 包装:把写入的字节流喂给 SHA-256
struct Sha256Writer(sha2::Sha256);
impl std::io::Write for Sha256Writer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// 流式计算文件 SHA-256(十六进制小写);音频可达数百 MB,io::copy 分块搬运不驻留大内存
fn sha256_hex(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut w = Sha256Writer(sha2::Sha256::new());
    std::io::copy(&mut file, &mut w).ok()?;
    // 逐字节十六进制编码(sha2 0.11 的摘要类型不再实现 LowerHex)
    Some(to_hex(w.0.finalize().as_slice()))
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn new_playlist_id(name: &str) -> String {
    format!(
        "{:016x}",
        scanner::hash_str(&format!("{name}:{}", now_secs()))
    )
}

/// 同名时自动追加序号:"歌单" → "歌单 (2)"
fn unique_playlist_name(existing: &[model::Playlist], base: &str) -> String {
    if !existing.iter().any(|p| p.name == base) {
        return base.to_string();
    }
    for i in 2..1000 {
        let cand = format!("{base} ({i})");
        if !existing.iter().any(|p| p.name == cand) {
            return cand;
        }
    }
    base.to_string()
}

/// 把曲目解析为播放列表条目(元数据快照,id 供后续回查)
fn entry_of(t: &model::Track) -> model::PlaylistEntry {
    model::PlaylistEntry {
        id: t.id.clone(),
        path: t.path.clone(),
        title: t.title.clone(),
        artist: t.artist.clone(),
        album: t.album.clone(),
        album_artist: t.album_artist.clone(),
        duration: t.duration,
    }
}

#[tauri::command]
fn create_playlist(name: String, state: State<AppState>) -> Result<Vec<model::Playlist>, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("播放列表名称不能为空".into());
    }
    let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
    if lib.playlists.iter().any(|p| p.name == name) {
        return Err("已存在同名播放列表".into());
    }
    lib.playlists.push(model::Playlist {
        id: new_playlist_id(&name),
        name,
        created_at: now_secs(),
        entries: Vec::new(),
    });
    lib.save(&state.data_dir.join(LIBRARY_FILE))
        .map_err(|e| e.to_string())?;
    Ok(lib.playlists.clone())
}

#[tauri::command]
fn rename_playlist(
    id: String,
    name: String,
    state: State<AppState>,
) -> Result<Vec<model::Playlist>, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("播放列表名称不能为空".into());
    }
    let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
    if lib.playlists.iter().any(|p| p.id != id && p.name == name) {
        return Err("已存在同名播放列表".into());
    }
    let pl = lib
        .playlists
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or("播放列表不存在")?;
    pl.name = name;
    lib.save(&state.data_dir.join(LIBRARY_FILE))
        .map_err(|e| e.to_string())?;
    Ok(lib.playlists.clone())
}

#[tauri::command]
fn delete_playlist(id: String, state: State<AppState>) -> Result<Vec<model::Playlist>, String> {
    let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
    lib.playlists.retain(|p| p.id != id);
    lib.save(&state.data_dir.join(LIBRARY_FILE))
        .map_err(|e| e.to_string())?;
    Ok(lib.playlists.clone())
}

/// 把曲目加入播放列表(按路径去重,返回新增数量与最新列表)
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddToPlaylistReport {
    added: u32,
    skipped: u32,
    playlists: Vec<model::Playlist>,
}

#[tauri::command]
fn add_tracks_to_playlist(
    playlist_id: String,
    track_ids: Vec<String>,
    state: State<AppState>,
) -> Result<AddToPlaylistReport, String> {
    let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
    let mut resolved: Vec<model::PlaylistEntry> = Vec::new();
    let by_id: std::collections::HashMap<&str, &model::Track> =
        lib.tracks.iter().map(|t| (t.id.as_str(), t)).collect();
    for id in &track_ids {
        if let Some(t) = by_id.get(id.as_str()) {
            resolved.push(entry_of(t));
        }
    }
    if resolved.is_empty() {
        return Err("未找到可加入的曲目".into());
    }
    let mut added = 0u32;
    let mut skipped = 0u32;
    {
        let pl = lib
            .playlists
            .iter_mut()
            .find(|p| p.id == playlist_id)
            .ok_or("播放列表不存在")?;
        for e in resolved {
            if pl.entries.iter().any(|x| x.path == e.path) {
                skipped += 1;
            } else {
                pl.entries.push(e);
                added += 1;
            }
        }
    }
    lib.save(&state.data_dir.join(LIBRARY_FILE))
        .map_err(|e| e.to_string())?;
    Ok(AddToPlaylistReport {
        added,
        skipped,
        playlists: lib.playlists.clone(),
    })
}

#[tauri::command]
fn remove_playlist_entry(
    playlist_id: String,
    index: usize,
    state: State<AppState>,
) -> Result<Vec<model::Playlist>, String> {
    let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
    let pl = lib
        .playlists
        .iter_mut()
        .find(|p| p.id == playlist_id)
        .ok_or("播放列表不存在")?;
    if index < pl.entries.len() {
        pl.entries.remove(index);
    }
    lib.save(&state.data_dir.join(LIBRARY_FILE))
        .map_err(|e| e.to_string())?;
    Ok(lib.playlists.clone())
}

/// 调整曲目顺序:把 from 位置的条目移动到 to 位置
#[tauri::command]
fn move_playlist_entry(
    playlist_id: String,
    from: usize,
    to: usize,
    state: State<AppState>,
) -> Result<Vec<model::Playlist>, String> {
    let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
    let pl = lib
        .playlists
        .iter_mut()
        .find(|p| p.id == playlist_id)
        .ok_or("播放列表不存在")?;
    let n = pl.entries.len();
    if from >= n || to >= n || from == to {
        return Ok(lib.playlists.clone());
    }
    let e = pl.entries.remove(from);
    pl.entries.insert(to, e);
    lib.save(&state.data_dir.join(LIBRARY_FILE))
        .map_err(|e| e.to_string())?;
    Ok(lib.playlists.clone())
}

// ===== TMCL 播放列表包(.tmcl = 标准 7z:音乐源文件 + 歌词 + 封面 + playlist.json) =====

/// 把播放列表打包进导出暂存区(不落位),写入过程中按资源数上报 export-progress 事件
#[tauri::command]
async fn stage_tmcl(
    playlist_id: String,
    staging: String,
    app: tauri::AppHandle,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        stage_tmcl_blocking(&playlist_id, Path::new(&staging), &app)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn stage_tmcl_blocking(
    playlist_id: &str,
    staging: &Path,
    app: &tauri::AppHandle,
) -> Result<String, String> {
    // 用户已取消(暂存目录被清理):拒绝,避免工作目录创建时把暂存目录重建出来
    if !staging.is_dir() {
        return Err("导出已取消".into());
    }
    let state = app.state::<AppState>();
    let (pl, tracks) = {
        let lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        let pl = lib
            .playlists
            .iter()
            .find(|p| p.id == playlist_id)
            .cloned()
            .ok_or("播放列表不存在")?;
        // 条目回查曲库:id 优先,路径兜底;两者都找不到则该曲目已不可用
        let mut by_id: std::collections::HashMap<&str, &model::Track> = HashMap::new();
        let mut by_path: std::collections::HashMap<String, &model::Track> = HashMap::new();
        for t in &lib.tracks {
            by_id.insert(t.id.as_str(), t);
            by_path.insert(norm_path(&t.path), t);
        }
        let tracks: Vec<model::Track> = pl
            .entries
            .iter()
            .filter_map(|e| {
                let t = by_id
                    .get(e.id.as_str())
                    .or_else(|| by_path.get(&norm_path(&e.path)));
                t.map(|t| (*t).clone())
            })
            .collect();
        (pl, tracks)
    };
    if tracks.is_empty() {
        return Err("播放列表为空或曲目文件均不可用".into());
    }

    let workspace = staging.join(".tmp");
    let _ = fs::remove_dir_all(&workspace);
    fs::create_dir_all(&workspace).map_err(|e| e.to_string())?;

    let (assets, mut items) = plan_track_assets(&tracks, &workspace)?;
    if items.is_empty() {
        let _ = fs::remove_dir_all(&workspace);
        return Err("播放列表中没有可导出的文件".into());
    }

    // 7z 写进暂存区工作目录,写入进度按资源数上报;封口后落进暂存区顶层。
    // manifest 最后打包:曲目 SHA 来自打包时的单遍哈希,每个文件只读一遍盘
    let manifest_path = workspace.join("playlist.json");
    let tmp_archive = workspace.join("playlist.tmcl");
    pack_with_hashes(app, &tmp_archive, &assets, "TMCL", &mut |hashes| {
        inject_manifest_shas(&mut items, hashes);
        let manifest = serde_json::json!({
            "format": "tmcl",
            "version": 1,
            "name": pl.name,
            "createdAt": pl.created_at,
            "tracks": items,
        });
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        Ok(("playlist.json".to_string(), manifest_path.clone()))
    })?;
    // 包名取清洗后的列表名;与暂存区内已有文件重名时自动加序号
    let staged = dedup_dest(staging.join(format!("{}.tmcl", sanitize_name(&pl.name))));
    place_archive(&tmp_archive, &staged)?;
    let _ = fs::remove_dir_all(&workspace);
    Ok(staged.to_string_lossy().to_string())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackImportReport {
    pub name: String,
    pub imported: u32,
    pub skipped: u32,
}

/// 导入 .tmcl:解包到导入目录 → 校验/去重 → 补 meta.json → 扫描 → 建立播放列表
#[tauri::command]
async fn import_playlist_tmcl(
    path: String,
    app: tauri::AppHandle,
) -> Result<PackImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || import_playlist_tmcl_blocking(&path, &app))
        .await
        .map_err(|e| e.to_string())?
}

/// 路径规范化:统一分隔符为反斜杠并小写,用于跨来源路径比较
/// (manifest 里的相对路径用 '/',walkdir 产生 '';Windows 也不区分大小写)
fn norm_path(p: &str) -> String {
    p.replace('/', "\\").to_lowercase()
}

/// 校验解包后的文件与 manifest 记录的 SHA-256 是否一致,返回损坏文件相对路径清单。
/// 旧包没有哈希字段则跳过(向后兼容);名字字段先过 safe_manifest_part 防穿越
fn verify_manifest_hashes(dest: &Path, items: &[serde_json::Value]) -> Vec<String> {
    let mut bad: Vec<String> = Vec::new();
    for item in items {
        for (hash_key, name_key) in [
            ("sha256", "audio"),
            ("lrcSha256", "lrc"),
            ("coverSha256", "cover"),
        ] {
            let Some(expect) = item.get(hash_key).and_then(|x| x.as_str()) else {
                continue;
            };
            let Some(name) = item
                .get(name_key)
                .and_then(|x| x.as_str())
                .and_then(safe_manifest_part)
            else {
                continue;
            };
            let dir = item
                .get("dir")
                .and_then(|x| x.as_str())
                .and_then(safe_manifest_part)
                .unwrap_or_default();
            let file = dest.join(&dir).join(&name);
            let ok = sha256_hex(&file)
                .map(|h| h.eq_ignore_ascii_case(expect))
                .unwrap_or(false);
            if !ok {
                bad.push(format!("{dir}/{name}"));
            }
        }
    }
    bad
}

/// 包导入的哈希三合一:损坏校验(不一致→Err 拒绝入库)、内容去重(与曲库已记录
/// SHA 相同→删除解包产物并跳过)、返回保留条目与跳过数。旧包无哈希字段原样通过
fn verify_and_dedupe_pack(
    dest: &Path,
    items: Vec<serde_json::Value>,
    state: &AppState,
) -> Result<(Vec<serde_json::Value>, u32), String> {
    let bad = verify_manifest_hashes(dest, &items);
    if !bad.is_empty() {
        return Err(format!("包校验失败,文件已损坏: {}", bad.join("; ")));
    }
    let known: std::collections::HashSet<String> = {
        let lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        lib.tracks
            .iter()
            .filter_map(|t| t.sha256.as_deref().map(|s| s.to_ascii_lowercase()))
            .collect()
    };
    let mut kept: Vec<serde_json::Value> = Vec::with_capacity(items.len());
    let mut skipped = 0u32;
    for item in items {
        let sha = item
            .get("sha256")
            .and_then(|x| x.as_str())
            .map(|s| s.to_ascii_lowercase());
        let dir = item
            .get("dir")
            .and_then(|x| x.as_str())
            .and_then(safe_manifest_part);
        let _audio = item
            .get("audio")
            .and_then(|x| x.as_str())
            .and_then(safe_manifest_part);
        if let (Some(sha), Some(dir)) = (&sha, &dir) {
            if known.contains(sha) {
                // 内容与曲库已有曲目相同:每首歌一个独立目录,可安全整目录移除
                let _ = fs::remove_dir_all(dest.join(dir));
                skipped += 1;
                continue;
            }
        }
        kept.push(item);
    }
    Ok((kept, skipped))
}

/// 把 (文件路径, SHA) 补写进曲库记录(按路径匹配;路径可能因去重改名,做规范化比对,
/// 归位到「艺人\专辑」子目录的散装文件按文件名兜底)。
/// 之后扫描遇到未变化的文件会原样保留该哈希,跨包去重持续有效。
/// 包导入(record_pack_shas)与散装导入共用
fn record_track_shas(state: &AppState, pairs: &[(PathBuf, String)]) -> Result<(), String> {
    if pairs.is_empty() {
        return Ok(());
    }
    let import_dir = state.data_dir.join(IMPORT_DIR_NAME);
    let import_str = import_dir.to_string_lossy().to_string();
    let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
    for (path, sha) in pairs {
        let abs_str = path.to_string_lossy().replace('/', "\\");
        let abs_norm = norm_path(&abs_str);
        let sha = sha.to_ascii_lowercase();
        // 先按记录时的原始路径精确/规范化匹配;散装导入的文件随后会被
        // organize_top_level 归位到「艺人\专辑」子目录,指纹路径已失配——
        // 退一步按文件名在导入目录下重新定位(取尚无指纹的那个)
        let idx = lib
            .tracks
            .iter()
            .position(|t| t.path == abs_str || norm_path(&t.path) == abs_norm)
            .or_else(|| {
                let name = path.file_name()?.to_string_lossy().to_string();
                lib.tracks.iter().position(|t| {
                    t.sha256.is_none()
                        && is_under(&t.path, &import_str)
                        && Path::new(&t.path)
                            .file_name()
                            .is_some_and(|n| n.to_string_lossy() == name)
                })
            });
        if let Some(i) = idx {
            lib.tracks[i].sha256 = Some(sha);
        }
    }
    lib.save(&state.data_dir.join(LIBRARY_FILE))
        .map_err(|e| e.to_string())
}

fn record_pack_shas(
    state: &AppState,
    dest: &Path,
    items: &[serde_json::Value],
) -> Result<(), String> {
    let mut pairs: Vec<(PathBuf, String)> = Vec::new();
    for item in items {
        let (sha, dir, audio) = (
            item.get("sha256").and_then(|x| x.as_str()),
            item.get("dir")
                .and_then(|x| x.as_str())
                .and_then(safe_manifest_part),
            item.get("audio")
                .and_then(|x| x.as_str())
                .and_then(safe_manifest_part),
        );
        let (Some(sha), Some(dir), Some(audio)) = (sha, dir, audio) else {
            continue;
        };
        pairs.push((dest.join(dir).join(audio), sha.to_string()));
    }
    record_track_shas(state, &pairs)
}

/// 校验 TMCL manifest 里的相对目录/文件名:拒绝绝对路径、盘符前缀与 `..` 组件。
/// 这是 zip-slip 的第二道闸——解包库(sevenz-rust2 safe_join)管住压缩包条目,
/// 这里管住 playlist.json 的字段,防止恶意包把 meta.json 写到导入目录之外
fn safe_manifest_part(part: &str) -> Option<String> {
    use std::path::Component;
    let normalized = part.replace('\\', "/");
    // Windows 盘符前缀(C:/...)在非 Windows 平台不是 Prefix 组件而是普通名字,
    // 必须手动识别,否则 Linux 上会让 "C:/evil" 混过校验(冒号在 Windows
    // 文件名里本就非法,不存在误伤的合法目录名)
    let b = normalized.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return None;
    }
    let mut out: Vec<&str> = Vec::new();
    for c in Path::new(&normalized).components() {
        match c {
            Component::Normal(p) => out.push(p.to_str()?),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!out.is_empty()).then(|| out.join("/"))
}

fn import_playlist_tmcl_blocking(
    path: &str,
    app: &tauri::AppHandle,
) -> Result<PackImportReport, String> {
    let src = Path::new(path);
    if !src.is_file() {
        return Err("文件不存在".into());
    }
    let state = app.state::<AppState>();
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "playlist".into());
    let import_dir = state.data_dir.join(IMPORT_DIR_NAME);
    fs::create_dir_all(&import_dir).map_err(|e| e.to_string())?;
    let dest = import_dir.join(sanitize_name(&stem));
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| format!("清理旧目录失败: {e}"))?;
    }
    fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    sevenz_rust2::decompress_file(src, &dest).map_err(|e| format!("解包失败: {e}"))?;

    let manifest_path = dest.join("playlist.json");
    if !manifest_path.is_file() {
        return Err("不是有效的 TMCL:缺少 playlist.json".into());
    }
    let v: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("playlist.json 解析失败: {e}"))?;
    let list_name = v
        .get("name")
        .and_then(|x| x.as_str())
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .unwrap_or_else(|| stem.clone());
    let tracks_json = v
        .get("tracks")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();

    // 逐文件 SHA-256 校验 + 与曲库内容去重
    let (tracks_json, skipped) = verify_and_dedupe_pack(&dest, tracks_json, &state)?;

    // 为无内嵌标签的文件写 meta.json(扫描器据此补齐元数据)
    write_manifest_meta_files(&dest, &tracks_json);

    // 扫描入库(与拖拽导入同一路径)
    scanner::run_scan(app)?;

    // 来源包记录的 SHA 写进曲库,供后续包导入做内容去重
    record_pack_shas(&state, &dest, &tracks_json)?;

    // 全部曲目都与曲库重复:不建空歌单,作为正常跳过返回(导入预览里明确提示)
    if tracks_json.is_empty() && skipped > 0 {
        return Ok(PackImportReport {
            name: list_name,
            imported: 0,
            skipped,
        });
    }
    // 建立播放列表:优先用扫描后的曲库数据,否则退回 manifest 快照
    let mut entries: Vec<model::PlaylistEntry> = Vec::new();
    for item in &tracks_json {
        let Some(dir) = item
            .get("dir")
            .and_then(|x| x.as_str())
            .and_then(safe_manifest_part)
        else {
            continue;
        };
        let Some(audio) = item
            .get("audio")
            .and_then(|x| x.as_str())
            .and_then(safe_manifest_part)
        else {
            continue;
        };
        let abs = dest.join(dir).join(audio);
        if !abs.is_file() {
            continue;
        }
        // 统一分隔符:manifest 用 '/',walkdir 的 track.path 用 '\'
        let abs_str = abs.to_string_lossy().replace('/', "\\");
        let abs_norm = norm_path(&abs_str);
        let from_lib = {
            let lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
            lib.tracks
                .iter()
                .find(|t| t.path == abs_str || norm_path(&t.path) == abs_norm)
                .cloned()
        };
        entries.push(match from_lib {
            Some(t) => entry_of(&t),
            None => model::PlaylistEntry {
                id: String::new(),
                path: abs_str,
                title: item
                    .get("title")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                artist: item
                    .get("artist")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                album: item
                    .get("album")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                album_artist: item
                    .get("albumArtist")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                duration: item.get("duration").and_then(|x| x.as_f64()).unwrap_or(0.0),
            },
        });
    }
    if entries.is_empty() {
        if skipped > 0 {
            return Err("歌单内全部曲目都已存在,已跳过导入".into());
        }
        return Err("TMCL 中没有可导入的曲目".into());
    }

    let imported = entries.len() as u32;
    let final_name = {
        let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        let name = unique_playlist_name(&lib.playlists, &list_name);
        lib.playlists.push(model::Playlist {
            id: new_playlist_id(&name),
            name: name.clone(),
            created_at: now_secs(),
            entries,
        });
        lib.save(&state.data_dir.join(LIBRARY_FILE))
            .map_err(|e| e.to_string())?;
        name
    };
    let _ = app.emit("library-changed", ());
    Ok(PackImportReport {
        name: final_name,
        imported,
        skipped,
    })
}
pub const ASSOC_EXTS: &[(&str, &str)] = &[
    ("tmc", "TMC 音乐包"),
    ("tmcl", "TMCL 播放列表"),
    ("tmca", "TMCA 专辑包"),
    ("mp3", "MP3 音频"),
    ("flac", "FLAC 音频"),
    ("m4a", "M4A 音频"),
    ("wav", "WAV 音频"),
    ("ogg", "OGG 音频"),
    ("opus", "OPUS 音频"),
];

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssocState {
    ext: String,
    label: String,
    registered: bool,
}

fn assoc_desc(ext: &str) -> &'static str {
    ASSOC_EXTS
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, d)| *d)
        .unwrap_or("音频")
}

#[tauri::command]
fn get_associations() -> Vec<AssocState> {
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        ASSOC_EXTS
            .iter()
            .map(|(ext, label)| {
                let registered = hkcu
                    .open_subkey(format!(r"Software\Classes\.{ext}\OpenWithProgids"))
                    .ok()
                    .map(|k| {
                        k.get_value::<String, _>(format!("TauriMusic.{ext}"))
                            .is_ok()
                    })
                    .unwrap_or(false);
                AssocState {
                    ext: ext.to_string(),
                    label: label.to_string(),
                    registered,
                }
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        ASSOC_EXTS
            .iter()
            .map(|(ext, label)| AssocState {
                ext: ext.to_string(),
                label: label.to_string(),
                registered: false,
            })
            .collect()
    }
}

#[tauri::command]
fn set_association(ext: String, enable: bool) -> Result<(), String> {
    // ext 来自前端 invoke:必须限定在支持名单内。get_associations 只读白名单,
    // 这里若不设防,被攻破的前端可借本命令改写任意 HKCU 类注册表
    // (如 .exe 的用户级默认 ProgID,或拼出带 \ 的子键路径)
    if !ASSOC_EXTS.iter().any(|(e, _)| *e == ext) {
        return Err(format!("不支持的扩展名: .{ext}"));
    }
    let desc = assoc_desc(&ext);
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;

    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let progid = format!("TauriMusic.{ext}");
        if enable {
            let cmd_key = hkcu
                .create_subkey(format!(r"Software\Classes\{progid}\shell\open\command"))
                .map_err(|e| e.to_string())?
                .0;
            cmd_key
                .set_value("", &format!("\"{}\" \"%1\"", exe.display()))
                .map_err(|e| e.to_string())?;
            let icon_key = hkcu
                .create_subkey(format!(r"Software\Classes\{progid}\DefaultIcon"))
                .map_err(|e| e.to_string())?
                .0;
            icon_key
                .set_value("", &format!("{},0", exe.display()))
                .map_err(|e| e.to_string())?;
            let id_key = hkcu
                .create_subkey(format!(r"Software\Classes\{progid}"))
                .map_err(|e| e.to_string())?
                .0;
            id_key
                .set_value("", &format!("TauriMusic {desc}"))
                .map_err(|e| e.to_string())?;
            let ext_key = hkcu
                .create_subkey(format!(r"Software\Classes\.{ext}\OpenWithProgids"))
                .map_err(|e| e.to_string())?
                .0;
            ext_key
                .set_value(&progid, &String::new())
                .map_err(|e| e.to_string())?;
            // 用户级默认 ProgID:无 UserChoice 时双击即用 TauriMusic(优先于 HKLM)
            let def_key = hkcu
                .create_subkey(format!(r"Software\Classes\.{ext}"))
                .map_err(|e| e.to_string())?
                .0;
            def_key.set_value("", &progid).map_err(|e| e.to_string())?;
        } else {
            if let Ok(k) = hkcu.open_subkey_with_flags(
                format!(r"Software\Classes\.{ext}\OpenWithProgids"),
                KEY_WRITE,
            ) {
                let _ = k.delete_value(&progid);
            }
            // 仅当用户级默认仍指向我们时才摘除,不破坏其他应用的关联
            if let Ok(k) =
                hkcu.open_subkey_with_flags(format!(r"Software\Classes\.{ext}"), KEY_WRITE)
            {
                let current: Result<String, _> = k.get_value("");
                if matches!(current, Ok(v) if v == progid) {
                    let _ = k.delete_value("");
                }
            }
            let _ = hkcu.delete_subkey_all(format!(r"Software\Classes\{progid}"));
        }
        // 通知 Explorer 立即刷新关联与图标缓存
        #[link(name = "shell32")]
        extern "system" {
            fn SHChangeNotify(
                wEventId: u32,
                uFlags: u32,
                dwItem1: *const std::ffi::c_void,
                dwItem2: *const std::ffi::c_void,
            );
        }
        unsafe {
            SHChangeNotify(0x0800_0000, 0x0000, std::ptr::null(), std::ptr::null());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (desc, exe, enable);
        Err("仅支持 Windows".into())
    }
}

// ===== 自启动(HKCU Run 键,用户级,无需管理员;与文件关联同一套注册表写法) =====

// 仅 Windows 代码路径引用:不加 cfg 的话 Linux 编译时是死代码,clippy -D warnings 会判死
#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_VALUE: &str = "TauriMusic";

/// 查询是否已开启开机自启动(HKCU Run 键下有无 TauriMusic 条目)
#[tauri::command]
fn get_autostart() -> bool {
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
        hkcu.open_subkey(RUN_KEY)
            .and_then(|k| k.get_value::<String, _>(RUN_VALUE))
            .is_ok()
    }
    #[cfg(not(windows))]
    false
}

/// 开启/关闭开机自启动:写或删 HKCU Run 键的 TauriMusic 条目(值为当前 exe 路径)
#[tauri::command]
fn set_autostart(enable: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE};
        let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
        let run = hkcu
            .open_subkey_with_flags(RUN_KEY, KEY_QUERY_VALUE | KEY_SET_VALUE)
            .map_err(|e| e.to_string())?;
        if enable {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            run.set_value(RUN_VALUE, &format!("\"{}\"", exe.display()))
                .map_err(|e| e.to_string())?;
        } else {
            run.delete_value(RUN_VALUE).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = enable;
        Err("仅支持 Windows".into())
    }
}

// ===== 听歌统计(仅本机,零联网):采集在前端,这里只做合并与原子落盘 =====

/// 冲刷一批增量:每曲收听秒数 + 播放次数 + 当日总时长。
/// 合并在内存锁内完成(微秒级),落盘放 blocking 线程
#[tauri::command]
async fn stats_add(
    state: State<'_, AppState>,
    samples: HashMap<String, f64>,
    plays: Vec<String>,
    day: String,
    day_sec: f64,
) -> Result<(), String> {
    let path = state.data_dir.join(stats::STATS_FILE);
    let snapshot = {
        let mut s = state.stats.lock().map_err(|_| "统计锁中毒")?;
        s.merge(&samples, &plays, &day, day_sec, stats::now_secs());
        s.clone()
    };
    tauri::async_runtime::spawn_blocking(move || snapshot.save(&path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// 全量读取(统计页打开时拉一次)
#[tauri::command]
fn stats_get(state: State<AppState>) -> Result<stats::StatsData, String> {
    state
        .stats
        .lock()
        .map(|g| g.clone())
        .map_err(|_| "统计锁中毒".into())
}

/// 清空统计:内存归零并立即落盘(设置页「清除统计数据」)
#[tauri::command]
async fn stats_clear(state: State<'_, AppState>) -> Result<(), String> {
    let path = state.data_dir.join(stats::STATS_FILE);
    let snapshot = {
        let mut s = state.stats.lock().map_err(|_| "统计锁中毒")?;
        *s = stats::StatsData::default();
        s.clone()
    };
    tauri::async_runtime::spawn_blocking(move || snapshot.save(&path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

// ===== 系统媒体控制(SMTC)+ 托盘菜单的状态推送(见 media.rs) =====

#[tauri::command]
fn smtc_set_track(
    app: tauri::AppHandle,
    title: String,
    artist: String,
    cover: Option<String>,
    duration: f64,
) -> Result<(), String> {
    media::on_track(&app, title, artist, cover, duration);
    Ok(())
}

#[tauri::command]
fn smtc_set_status(app: tauri::AppHandle, playing: bool) -> Result<(), String> {
    media::on_status(&app, playing);
    Ok(())
}

#[tauri::command]
fn smtc_set_position(app: tauri::AppHandle, position: f64, duration: f64) -> Result<(), String> {
    media::on_position(&app, position, duration);
    Ok(())
}

#[tauri::command]
fn get_library(state: State<AppState>) -> Result<InvokeResponseBody, String> {
    // 锁内直接序列化成 JSON 字符串原样透传(InvokeResponseBody::Json),
    // 不再先建整棵 serde_json::Value 树:大曲库时启动/重扫的瞬时内存峰值显著更低;
    // Json 变体以 application/json 返回,前端 invoke<Library> 拿到的仍是解析好的对象
    let lib = state.lib.lock().map_err(|_| "曲库状态不可用".to_string())?;
    serde_json::to_string(&*lib)
        .map(InvokeResponseBody::Json)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn add_folder(path: String, app: tauri::AppHandle, state: State<AppState>) -> Result<(), String> {
    if !Path::new(&path).is_dir() {
        return Err("该路径不是一个文件夹".into());
    }
    let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
    if lib.folders.iter().any(|f| f == &path) {
        return Ok(());
    }
    // 嵌套或大小写变体的重复登记会让同一文件被扫成两条曲目:
    // 组件级双向包含判定(F:\music 与 F:\music\flac、F:\Music 与 f:\music 都算重叠)
    if let Some(f) = lib
        .folders
        .iter()
        .find(|f| is_under(&path, f) || is_under(f, &path))
    {
        return Err(format!("该文件夹与已登记的「{f}」重叠,无需重复添加"));
    }
    lib.folders.push(path);
    if let Err(e) = lib.save(&state.data_dir.join(LIBRARY_FILE)) {
        lib.folders.pop();
        return Err(e.to_string());
    }
    // 新登记的文件夹要同步进 asset 协议放行名单,否则封面/音频加载不出来
    refresh_asset_scope(&app);
    Ok(())
}

#[tauri::command]
fn remove_folder(path: String, state: State<AppState>) -> Result<(), String> {
    let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
    lib.folders.retain(|f| f != &path);
    // 组件级前缀匹配:字符串 starts_with 会把 F:\music2 误当成 F:\music 的子目录
    lib.tracks.retain(|t| !is_under(&t.path, &path));
    lib.save(&state.data_dir.join(LIBRARY_FILE))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn pick_music_folder() -> Option<String> {
    rfd::AsyncFileDialog::new()
        .set_title("选择音乐文件夹")
        .pick_folder()
        .await
        .map(|f| f.path().to_string_lossy().to_string())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanWithLibrary {
    pub report: model::ScanReport,
    /// 扫描后的完整曲库(JSON 字符串,前端解析)——省掉启动链路上的第二次全量传输
    pub library: String,
}

#[tauri::command]
async fn scan_library(app: tauri::AppHandle) -> Result<ScanWithLibrary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let report = scanner::run_scan(&app)?;
        let state = app.state::<AppState>();
        let library = {
            let lib = state.lib.lock().map_err(|_| "曲库状态不可用".to_string())?;
            serde_json::to_string(&*lib).map_err(|e| e.to_string())?
        };
        Ok(ScanWithLibrary { report, library })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_lyrics(id: String, app: tauri::AppHandle) -> Option<lyrics::LyricsPayload> {
    // 锁内只查表取 Track;歌词解析(lofty 音频探测)移到阻塞线程
    let track = {
        let state = app.state::<AppState>();
        let lib = state.lib.lock().ok()?;
        lib.tracks.iter().find(|t| t.id == id).cloned()
    }?;
    tauri::async_runtime::spawn_blocking(move || lyrics::load_for_track(&track))
        .await
        .ok()?
}

// ===== 资源占用(关于页展示) =====

static RES_SYS: std::sync::OnceLock<Mutex<sysinfo::System>> = std::sync::OnceLock::new();

/// 主进程资源占用:内存 MB + CPU 百分比(CPU 为两次调用间的均值,首次为 0)
#[derive(serde::Serialize)]
struct ResourceUsage {
    memory_mb: f64,
    cpu: f32,
}

#[tauri::command]
fn get_resource_usage() -> ResourceUsage {
    let sys_mutex = RES_SYS.get_or_init(|| Mutex::new(sysinfo::System::new()));
    let mut sys = sys_mutex.lock().unwrap_or_else(|e| e.into_inner());
    let (memory_mb, cpu) = match sysinfo::get_current_pid() {
        Ok(pid) => {
            sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
            sys.process(pid)
                .map(|p| (p.memory() as f64 / 1048576.0, p.cpu_usage()))
                .unwrap_or((0.0, 0.0))
        }
        Err(_) => (0.0, 0.0),
    };
    ResourceUsage { memory_mb, cpu }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 启动守卫：已有实例运行时询问是否重启（旧实例卡死时的兜底）；
    // 放行与否最终仍由下面的单实例插件裁定，这里只负责询问与清场
    if !single_instance::guard_startup() {
        return;
    }
    tauri::Builder::default()
        // 二次启动时唤起已有窗口,并把传入的文件交给运行中的实例导入
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            show_main(app);
            handle_open_paths(app, &args);
        }))
        // 点 × 隐藏到托盘常驻,托盘菜单"退出"才真正退出;
        // 托盘迷你面板失焦即隐藏(它 show 时会接管焦点)
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            // 刚弹出时焦点稳定过程会有一次假失焦,150ms 内忽略,
            // 否则右键面板闪现即消失
            WindowEvent::Focused(false)
                if window.label() == "tray" && !media::tray_recently_shown() =>
            {
                let _ = window.hide();
            }
            _ => {}
        })
        .setup(|app| {
            let handle = app.handle();
            let data_dir = handle.path().app_data_dir().expect("无法确定应用数据目录");
            let cache_dir = handle.path().app_cache_dir().expect("无法确定应用缓存目录");
            std::fs::create_dir_all(&data_dir).ok();
            std::fs::create_dir_all(cache_dir.join("covers")).ok();
            // 清理上次预览遗留的暂存目录:确认/取消时都会清理,这里兜底异常退出残留
            let _ = fs::remove_dir_all(cache_dir.join("covers").join("preview"));
            // 兜底清理异常退出残留的导出暂存目录:正常流程由前端 cleanup_stage 处理,
            // 崩溃/断电时整包音频会永久留在系统 Temp(可能几百 MB × 多个)
            for entry in fs::read_dir(std::env::temp_dir())
                .into_iter()
                .flatten()
                .flatten()
            {
                let p = entry.path();
                let ours = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("taurimusic-stage-"));
                if ours {
                    let _ = fs::remove_dir_all(&p);
                }
            }
            let lib = Library::load(&data_dir.join(LIBRARY_FILE));
            let stats = stats::StatsData::load(&data_dir.join(stats::STATS_FILE));
            app.manage(AppState {
                lib: Mutex::new(lib),
                data_dir,
                cache_dir,
                scan_gate: Mutex::new(()),
                stats: Mutex::new(stats),
                pending_imports: Mutex::new(Vec::new()),
            });
            // 首次启动即建好导入目录并登记为扫描来源
            ensure_import_dir(handle, &app.state::<AppState>()).ok();
            // asset 协议初始放行:封面缓存 + 曲库文件夹(静态 ** 范围已移除)
            refresh_asset_scope(handle);
            // 清理超过保留期的回收站条目
            purge_expired_trash(handle);
            // 窗口在配置里全部 create:false,由这里手动创建:Tauri 内部顺序是
            // 先建窗口(WebView 立即加载前端)后跑本钩子,冷启动时前端会抢在
            // .manage 之前调命令,报「state not managed」→「曲库读取失败」。
            // 状态注册完毕后才开始加载页面,竞态从根上消除
            for window_config in app.config().app.windows.iter() {
                tauri::WebviewWindowBuilder::from_config(handle, window_config)?.build()?;
            }
            // 命令行/双击"打开方式"传入的文件
            let cli_args: Vec<String> = std::env::args().collect();
            handle_open_paths(handle, &cli_args);

            // 托盘 + 系统媒体控制(SMTC):迷你面板/封面/进度/播放控制都在 media.rs
            media::init(handle)?;

            // 托盘迷你面板的系统级动作(退出/唤起主窗口)由前端 emit 过来
            let sys_handle = handle.clone();
            handle.listen("media://sys", move |event| {
                match event.payload().trim_matches('"') {
                    "quit" => sys_handle.exit(0),
                    "show" => crate::show_main(&sys_handle),
                    _ => {}
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_library,
            add_folder,
            remove_folder,
            pick_music_folder,
            scan_library,
            get_lyrics,
            get_import_dir,
            open_import_dir,
            import_paths,
            import_tmc,
            inspect_package,
            discard_preview,
            take_pending_imports,
            show_lyrics_window,
            hide_lyrics_window,
            pick_tmc_file,
            pick_audio_files,
            pick_export_dir,
            make_stage_dir,
            stage_tmc,
            stage_tmcl,
            stage_tmca,
            place_staged,
            cleanup_stage,
            netease_enrich_album,
            get_associations,
            set_association,
            get_autostart,
            set_autostart,
            stats_add,
            stats_get,
            stats_clear,
            smtc_set_track,
            smtc_set_status,
            smtc_set_position,
            delete_tracks,
            restore_tracks,
            purge_trash,
            create_playlist,
            rename_playlist,
            delete_playlist,
            add_tracks_to_playlist,
            remove_playlist_entry,
            move_playlist_entry,
            import_playlist_tmcl,
            import_tmca,
            get_resource_usage
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{
        artist_matches, copy_into_import_dir, is_under, name_matches, place_archive,
        same_file_content, sanitize_name, sha256_hex, CopyResult,
    };
    use crate::model;
    use std::fs;

    #[test]
    fn is_under_uses_path_components() {
        // 字符串前缀匹配会把 F:\music2 误当成 F:\music 的子目录,组件匹配不会
        assert!(is_under(r"F:\music\a.mp3", r"F:\music"));
        assert!(is_under(r"F:\music\sub\b.mp3", r"F:\music"));
        assert!(is_under(r"F:\music", r"F:\music"));
        assert!(!is_under(r"F:\music2\a.mp3", r"F:\music"));
        assert!(!is_under(r"F:\musica\c.mp3", r"F:\music"));
        // 正斜杠形式(跨平台)
        assert!(is_under("F:/music/a.mp3", r"F:\music"));
        assert!(!is_under("/music/other/x.mp3", "/music/sub"));
        // 空目录参数不构成包含关系
        assert!(!is_under(r"F:\music\a.mp3", ""));
        // 盘符大小写不敏感仅限 Windows
        #[cfg(windows)]
        assert!(is_under(r"f:\MUSIC\a.mp3", r"F:\music"));
        #[cfg(not(windows))]
        assert!(!is_under(r"f:\MUSIC\a.mp3", r"F:\music"));
    }

    #[test]
    fn place_archive_moves_tmp_into_place() {
        let dir = std::env::temp_dir().join(format!("tm-place-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let stage = dir.join("stage");
        let target = dir.join("target");
        fs::create_dir_all(&stage).unwrap();
        fs::create_dir_all(&target).unwrap();

        // 常规:临时包落位(同盘 rename),临时文件消失
        let tmp1 = stage.join("a.tmc");
        fs::write(&tmp1, b"archive-1").unwrap();
        place_archive(&tmp1, &target.join("a.tmc")).unwrap();
        assert!(target.join("a.tmc").is_file());
        assert!(!tmp1.exists());

        // 目标已存在:rename 失败自动退化为拷贝覆盖(单文件 TMCL 用户确认覆盖的场景)
        let tmp2 = stage.join("a.tmc");
        fs::write(&tmp2, b"archive-2").unwrap();
        place_archive(&tmp2, &target.join("a.tmc")).unwrap();
        assert_eq!(fs::read(target.join("a.tmc")).unwrap(), b"archive-2");
        assert!(!tmp2.exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dedup_dest_appends_sequence() {
        use super::dedup_dest;
        let dir = std::env::temp_dir().join(format!("tm-dedupdest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // 不存在:原样返回
        assert_eq!(dedup_dest(dir.join("song.tmc")), dir.join("song.tmc"));
        // 已存在:追加序号,扩展名保留;新序号也存在时继续递增
        fs::write(dir.join("song.tmc"), b"a").unwrap();
        assert_eq!(dedup_dest(dir.join("song.tmc")), dir.join("song (2).tmc"));
        fs::write(dir.join("song (2).tmc"), b"b").unwrap();
        assert_eq!(dedup_dest(dir.join("song.tmc")), dir.join("song (3).tmc"));
        // 不同扩展名互不干扰
        fs::write(dir.join("song.tmcl"), b"c").unwrap();
        assert_eq!(dedup_dest(dir.join("song.tmcl")), dir.join("song (2).tmcl"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pick_lrclib_result_prefers_synced_and_matching_artist() {
        use super::pick_lrclib_result;
        let items = vec![
            // 艺人对不上:即使有同步歌词也不要
            serde_json::json!({"artistName": "Other Band", "duration": 300.0,
                "syncedLyrics": "[00:09.00] wrong", "plainLyrics": "wrong"}),
            // 艺人对上(大小写归一)+ 纯文本 + 时长吻合
            serde_json::json!({"artistName": "beyond", "duration": 301.0,
                "syncedLyrics": null, "plainLyrics": "plain text"}),
            // 艺人对上(繁体归一)+ 逐行同步歌词:优先于纯文本
            serde_json::json!({"artistName": "BEYOND", "duration": 300.0,
                "syncedLyrics": "[00:01.00] line", "plainLyrics": "dup"}),
        ];
        assert_eq!(
            pick_lrclib_result(&items, "beyond", Some(300.0), false).as_deref(),
            Some("[00:01.00] line")
        );
        // 查询艺人与所有候选都对不上 → 不挑任何一条
        assert_eq!(
            pick_lrclib_result(&items, "陈奕迅", Some(300.0), false),
            None
        );
        // 繁简归一化后能对上传统写法的艺人名
        let t2 = vec![
            serde_json::json!({"artistName": "陳奕迅", "duration": 200.0,
            "syncedLyrics": "[00:02.00] y", "plainLyrics": ""}),
        ];
        assert_eq!(
            pick_lrclib_result(&t2, "陈奕迅", None, false).as_deref(),
            Some("[00:02.00] y")
        );
    }

    #[test]
    fn kuwo_helpers_parse_pseudo_json_and_lrc() {
        use super::{build_lrc_from_kuwo, html_unescape, kuwo_fix_json};

        // 伪 JSON → 严格 JSON:定界引号转换,值内撇号保留
        let raw = "{'ARTIST':'Don&apos;T STOP','DURATION':'324','abslist':[{'A':'x'},{'B':'y'}]}";
        let fixed = kuwo_fix_json(raw);
        let v: serde_json::Value = serde_json::from_str(&fixed).unwrap();
        assert_eq!(v["abslist"][1]["B"], "y");
        // HTML 实体解码
        assert_eq!(html_unescape("Don&apos;T&nbsp;STOP"), "Don'T\u{a0}STOP");
        assert_eq!(html_unescape("&#39;ok&#x26;"), "'ok&");
        // lrclist → LRC 文本
        let lines = vec![
            serde_json::json!({"lineLyric": "今天我", "time": "19.34"}),
            serde_json::json!({"lineLyric": "寒夜里看雪飘过", "time": "75"}),
            serde_json::json!({"lineLyric": "", "time": "80"}),
        ];
        assert_eq!(
            build_lrc_from_kuwo(&lines).as_deref(),
            Some("[00:19.34]今天我\n[01:15.00]寒夜里看雪飘过\n[01:20.00]")
        );
        // 空列表 → None
        assert_eq!(build_lrc_from_kuwo(&[]), None);
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
    fn strip_bracketed_removes_parenthetical_for_search() {
        use super::{kuwo_keywords, strip_bracketed};
        assert_eq!(
            strip_bracketed("八千里路明月照（挥笔生花问桀骜）"),
            "八千里路明月照"
        );
        assert_eq!(strip_bracketed("Song (Live)"), "Song");
        assert_eq!(strip_bracketed("A【B】C"), "A C");
        assert_eq!(strip_bracketed("普通标题"), "普通标题");
        // 关键词组:完整标题在前,去括号的在后(与原标题相同则只有一条)
        assert_eq!(
            kuwo_keywords("八千里路明月照（挥笔生花问桀骜）", "泛国同学"),
            vec![
                "八千里路明月照（挥笔生花问桀骜） 泛国同学".to_string(),
                "八千里路明月照 泛国同学".to_string(),
            ]
        );
        assert_eq!(
            kuwo_keywords("普通标题", "艺人"),
            vec!["普通标题 艺人".to_string()]
        );
    }

    #[test]
    fn title_is_live_distinguishes_live_markers_from_lookalikes() {
        use super::title_is_live;
        // 常见现场版标记:英文括号后缀 / 中文标记 / 繁体
        assert!(title_is_live("朋友们的酒 (Live in China)"));
        assert!(title_is_live("Song (Live)"));
        assert!(title_is_live("海阔天空（现场版）"));
        assert!(title_is_live("海阔天空（現場）"));
        assert!(title_is_live("演唱会 2023"));
        assert!(title_is_live("Live版"));
        // 误判防线:live 必须独立成词
        assert!(!title_is_live("Alive"));
        assert!(!title_is_live("Livin' on a Prayer"));
        assert!(!title_is_live("普通标题"));
    }

    #[test]
    fn pick_netease_song_prefers_live_and_duration_match() {
        use super::pick_netease_song;
        use serde_json::json;
        let studio = json!({"id": 1, "name": "朋友们的酒", "duration": 240000, "artists": [{"name": "李晓杰"}]});
        let live = json!({"id": 2, "name": "朋友们的酒 (Live in China)", "duration": 280000, "artists": [{"name": "李晓杰"}]});
        let songs = vec![studio, live];
        // 现场版请求:live 标记加权压过先出现的录音室版
        assert_eq!(
            pick_netease_song(&songs, "李晓杰", None, true).unwrap()["id"].as_i64(),
            Some(2)
        );
        // 非现场版请求:同分取先出现的(与旧的「首个艺人吻合」行为一致)
        assert_eq!(
            pick_netease_song(&songs, "李晓杰", None, false).unwrap()["id"].as_i64(),
            Some(1)
        );
        // 本地时长 280s 与 live 吻合:即使不识别 live 也能靠时长纠出版本
        assert_eq!(
            pick_netease_song(&songs, "李晓杰", Some(280.0), false).unwrap()["id"].as_i64(),
            Some(2)
        );
        // 艺人全对不上且无其他得分项:宁缺毋滥
        let other = json!({"id": 3, "name": "朋友们的酒", "artists": [{"name": "别人"}]});
        assert!(pick_netease_song(std::slice::from_ref(&other), "李晓杰", None, false).is_none());
    }

    #[test]
    fn pick_lrclib_result_live_beats_synced_studio() {
        use super::pick_lrclib_result;
        use serde_json::json;
        let studio = json!({"artistName": "李晓杰", "trackName": "朋友们的酒", "syncedLyrics": "[00:01]a", "plainLyrics": "a", "duration": 240.0});
        let live = json!({"artistName": "李晓杰", "trackName": "朋友们的酒 (Live)", "syncedLyrics": "", "plainLyrics": "b", "duration": 280.0});
        let items = vec![studio, live];
        // 现场版请求:live 纯文本胜过录音室同步歌词——版本错了时间轴同步毫无意义
        assert_eq!(
            pick_lrclib_result(&items, "李晓杰", Some(280.0), true).unwrap(),
            "b"
        );
        // 非现场版请求:同步格式优先
        assert_eq!(
            pick_lrclib_result(&items, "李晓杰", Some(280.0), false).unwrap(),
            "[00:01]a"
        );
    }

    #[test]
    fn tmc_store_archive_roundtrips_via_own_decoder() {
        // 导出统一走 Copy(存储)模式,打包结果必须能被应用自身的解包路径读回
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod};

        let dir = std::env::temp_dir().join(format!("tm-7z-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("in.bin");
        fs::write(&src, vec![7u8; 4096]).unwrap();

        let archive = dir.join("out.tmc");
        let mut writer = ArchiveWriter::create(&archive).unwrap();
        writer.set_content_methods(vec![EncoderConfiguration::new(EncoderMethod::COPY)]);
        let file = fs::File::open(&src).unwrap();
        writer
            .push_archive_entry(
                ArchiveEntry::from_path(&src, "a.bin".to_string()),
                Some(file),
            )
            .unwrap();
        writer.finish().unwrap();

        let out = dir.join("roundtrip");
        sevenz_rust2::decompress_file(&archive, &out).unwrap();
        assert_eq!(fs::read(out.join("a.bin")).unwrap(), vec![7u8; 4096]);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tmc_import_rejects_zip_slip_entry_names() {
        // RUSTSEC-2026-0245 的回归防线:恶意包的穿越条目名必须被拒收,文件绝不落盘
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter};

        let dir = std::env::temp_dir().join(format!("tm-slip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("payload.bin");
        fs::write(&src, b"evil").unwrap();

        let archive = dir.join("evil.tmc");
        let mut writer = ArchiveWriter::create(&archive).unwrap();
        for name in [
            "..\\escaped.txt",
            "../../../escaped.txt",
            "C:\\Temp\\escaped.txt",
        ] {
            let file = fs::File::open(&src).unwrap();
            writer
                .push_archive_entry(ArchiveEntry::from_path(&src, name.to_string()), Some(file))
                .unwrap();
        }
        writer.finish().unwrap();

        let dest = dir.join("out");
        assert!(sevenz_rust2::decompress_file(&archive, &dest).is_err());
        // 无论逃逸到解包目录的哪里,都不允许出现落盘文件
        assert!(!dir.join("escaped.txt").exists());
        assert!(!dest.join("escaped.txt").exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn manifest_part_rejects_traversal_and_accepts_normal() {
        use super::safe_manifest_part;
        // 正常的 manifest 字段(与导出侧生成格式一致)
        assert_eq!(
            safe_manifest_part("music/01. 海阔天空 - Beyond"),
            Some("music/01. 海阔天空 - Beyond".to_string())
        );
        assert_eq!(safe_manifest_part("a\\b\\c"), Some("a/b/c".to_string()));
        // 穿越与绝对路径一律拒绝
        assert_eq!(safe_manifest_part("../evil"), None);
        assert_eq!(safe_manifest_part("a/../../evil"), None);
        assert_eq!(safe_manifest_part(r"..\..\evil"), None);
        assert_eq!(safe_manifest_part(r"C:\Temp\evil"), None);
        assert_eq!(safe_manifest_part("/etc/evil"), None);
        assert_eq!(safe_manifest_part(""), None);
        assert_eq!(safe_manifest_part("."), None);
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
        let dir = std::env::temp_dir().join(format!("tm-import-{}", std::process::id()));
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

    #[test]
    fn name_matching_is_normalized() {
        // 大小写
        assert!(name_matches("Beyond", "BEYOND"));
        // 繁简
        assert!(name_matches("湯幻月", "汤幻月"));
        // 空串不匹配
        assert!(!name_matches("", "Beyond"));
        // 无关艺人
        assert!(!name_matches("Janet Jackson", "Beyond"));
    }

    #[test]
    fn artist_group_matches_any_segment() {
        // 多艺人组里任一命中即可,且繁体「湯」与简体「汤」互通
        assert!(artist_matches("湯幻月/小义学长", "小义学长"));
        assert!(artist_matches("湯幻月/小义学长", "汤幻月"));
        assert!(!artist_matches("湯幻月/小义学长", "张治环"));
    }

    #[test]
    fn sanitize_replaces_invalid_filename_chars() {
        assert_eq!(
            sanitize_name("a/b\\c:d*e?f\"g<h>i|j"),
            "a_b_c_d_e_f_g_h_i_j"
        );
        assert_eq!(sanitize_name("海阔天空"), "海阔天空");
        assert_eq!(sanitize_name("  Beyond  "), "Beyond");
        // 结尾的点与空格被 Windows 静默吞掉,一并去掉
        assert_eq!(sanitize_name("海阔天空. "), "海阔天空");
        // Windows 保留名加前缀(主名比对,带扩展名的 CON.mp3 同样保留)
        assert_eq!(sanitize_name("CON"), "_CON");
        assert_eq!(sanitize_name("com1"), "_com1");
        assert_eq!(sanitize_name("CON.mp3"), "_CON.mp3");
        assert_eq!(sanitize_name("my.CON"), "my.CON");
        // 控制字符剔除,空结果回退默认名
        assert_eq!(sanitize_name("\u{0}\u{1}"), "track");
        assert_eq!(sanitize_name(""), "track");
    }

    #[test]
    fn sha256_hex_matches_known_vectors() {
        use super::sha256_hex;
        let dir = std::env::temp_dir().join(format!("tm-sha-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // 标准测试向量:空文件与 "abc"
        let empty = dir.join("empty.bin");
        fs::write(&empty, b"").unwrap();
        assert_eq!(
            sha256_hex(&empty).as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        let abc = dir.join("abc.bin");
        fs::write(&abc, b"abc").unwrap();
        assert_eq!(
            sha256_hex(&abc).as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn verify_manifest_hashes_detects_tampering() {
        use super::{sha256_hex, verify_manifest_hashes};
        let dir = std::env::temp_dir().join(format!("tm-verify-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("music/01")).unwrap();
        let audio = dir.join("music/01").join("a.mp3");
        fs::write(&audio, b"hello").unwrap();
        let sha = sha256_hex(&audio).unwrap();
        let item = serde_json::json!({ "dir": "music/01", "audio": "a.mp3", "sha256": sha });
        assert!(verify_manifest_hashes(&dir, std::slice::from_ref(&item)).is_empty());
        // 篡改后必须被发现并报出相对路径
        fs::write(&audio, b"hellO").unwrap();
        assert_eq!(
            verify_manifest_hashes(&dir, &[item]),
            vec!["music/01/a.mp3"]
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn plan_track_assets_shapes_manifest_items() {
        use super::plan_track_assets;
        let dir = std::env::temp_dir().join(format!("tm-plan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("a.wav");
        fs::write(&audio, b"riff-wave-bytes").unwrap();
        let track = model::Track {
            path: audio.to_string_lossy().to_string(),
            title: "测试".into(),
            artist: "艺人".into(),
            ..Default::default()
        };
        let (_, items) = plan_track_assets(&[track], &dir).unwrap();
        assert_eq!(items.len(), 1);
        // SHA 字段不在规划期产生:由打包阶段单遍收集后经 inject_manifest_shas 注入
        assert_eq!(items[0]["audio"], "测试 - 艺人.wav");
        assert!(items[0]["sha256"].is_null());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn inject_manifest_shas_matches_by_archive_name() {
        use super::inject_manifest_shas;
        use std::collections::HashMap;
        let mut items = vec![serde_json::json!({
            "dir": "music/01. a - b",
            "audio": "a - b.mp3",
            "lrc": "a - b.lrc",
            "cover": "a - b.cover.jpg",
        })];
        let mut hashes = HashMap::new();
        hashes.insert("a - b.mp3".to_string(), "hash-audio".to_string());
        hashes.insert("a - b.lrc".to_string(), "hash-lrc".to_string());
        hashes.insert("a - b.cover.jpg".to_string(), "hash-cover".to_string());
        inject_manifest_shas(&mut items, &hashes);
        assert_eq!(items[0]["sha256"], "hash-audio");
        assert_eq!(items[0]["lrcSha256"], "hash-lrc");
        assert_eq!(items[0]["coverSha256"], "hash-cover");
        // 没有对应哈希的字段不注入
        let mut empty = vec![serde_json::json!({"audio": "missing.mp3"})];
        inject_manifest_shas(&mut empty, &hashes);
        assert!(empty[0]["sha256"].is_null());
    }
}
