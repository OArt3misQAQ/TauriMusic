//! 散装导入:拖拽与"打开方式"的音频/歌词复制进导入目录、ffmpeg 转码与内容指纹去重。

use crate::model::Library;
use crate::pack::{record_track_shas, sha256_hex};
use crate::scanner;
use crate::{lock_scan_gate, refresh_asset_scope, AppState, LIBRARY_FILE};
// 唤起主窗口是桌面专属(单实例/文件关联场景);Android 的主窗口由系统管理
#[cfg(desktop)]
use crate::show_main;
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

/// 散装导入报告
struct LooseImport {
    folders_added: u32,
    files_copied: u32,
    transcoded: u32,
    skipped: u32,
    duplicates: u32,
    /// 本次新收集的指纹,扫描后写进曲库供跨批次去重
    session_shas: Vec<(PathBuf, String)>,
}

impl LooseImport {
    /// 是否真的动了曲库数据(登记文件夹/落新文件/转码出件):纯重复或全跳过的
    /// 批次不值得触发全库 walkdir 重扫与 library.json 整库重写
    fn changed(&self) -> bool {
        self.folders_added > 0 || self.files_copied > 0 || self.transcoded > 0
    }
}

/// 单个散装文件的落盘决策:第一遍(并行)只读源与目标元数据、算内容指纹,不写盘
enum LoosePlan {
    /// 不收(非音频/歌词、无 ffmpeg 的非常规格式、目标已存在且源未更新)
    Skip,
    /// 与曲库既有内容重复
    Duplicate,
    /// 直接复制(带内容指纹供会话内去重;指纹获取失败则不带,照常复制)
    Copy { dest: PathBuf, sha: Option<String> },
    /// 交给 ffmpeg 转码为 FLAC
    Transcode { dest: PathBuf },
}

/// 把一批路径导入:文件夹登记进曲库,散装文件复制进导入目录并收集 SHA 指纹。
/// lib 锁只用于登记与快照,文件复制与哈希在锁外进行(大文件哈希不该卡住其他命令)。
/// import_paths 命令与 handle_open_paths 共用。
/// 三段式:规划(并行算指纹)→ 会话定案(串行批内去重)→ 执行(并行落盘),
/// 批量拖入大量非常规格式时 ffmpeg 转码不再逐个排队
fn import_loose(
    paths: &[PathBuf],
    import_dir: &Path,
    state: &AppState,
    ffmpeg: Option<&Path>,
) -> Result<LooseImport, String> {
    let mut folders_added = 0u32;
    let mut skipped = 0u32;
    let mut duplicates = 0u32;

    // 短闸 + 短锁:登记文件夹与落盘只在这里互斥扫描;文件复制(可能分钟级)
    // 不占闸,别把用户手动触发的扫描挡在门外
    let (known, known_shas, pending_bytes) = {
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

    // 锁外第一遍(并行):逐文件出计划。SHA 全量读盘是 IO 大头,
    // 无写副作用,可以安全吃满多核(与扫描器的 worker 模式同款)
    let plans: Vec<LoosePlan> = {
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .clamp(1, 4);
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            let paths = &paths;
            let next = &next;
            // 借用再入闭包:move 闭包各拿一份引用,而不是把 Vec/HashSet 挪进第一个 worker
            let known = &known;
            let known_shas = &known_shas;
            let handles: Vec<_> = (0..workers)
                .map(|_| {
                    scope.spawn(move || {
                        let mut out: Vec<(usize, LoosePlan)> = Vec::new();
                        loop {
                            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if i >= paths.len() {
                                break;
                            }
                            let plan =
                                plan_loose_file(&paths[i], import_dir, known, known_shas, ffmpeg);
                            out.push((i, plan));
                        }
                        out
                    })
                })
                .collect();
            let mut slots: Vec<Option<LoosePlan>> = (0..paths.len()).map(|_| None).collect();
            for out in handles.into_iter().map(|h| {
                // 规划 worker panic 只丢它领走的文件的计划,不让整批炸掉
                h.join().unwrap_or_else(|p| {
                    eprintln!("[import] 规划 worker panic: {}", crate::panic_message(&*p));
                    Vec::new()
                })
            }) {
                for (i, plan) in out {
                    slots[i] = Some(plan);
                }
            }
            // panic worker 领走但没交回的文件按跳过处理,保证每个槽位都有计划
            slots
                .into_iter()
                .map(|p| p.unwrap_or(LoosePlan::Skip))
                .collect()
        })
    };

    // 第二遍(串行):会话内去重与终判。同批两份相同内容,先到的照常复制,
    // 后到的凭前者登记的指纹判重复——语义与旧串行实现一致
    let mut final_plans = plans;
    let mut seen_shas: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut session_shas: Vec<(PathBuf, String)> = Vec::new();
    for plan in final_plans.iter_mut() {
        match plan {
            LoosePlan::Skip => skipped += 1,
            LoosePlan::Duplicate => duplicates += 1,
            LoosePlan::Copy {
                dest,
                sha: Some(sha),
            } => {
                if seen_shas.insert(sha.clone()) {
                    session_shas.push((dest.clone(), sha.clone()));
                } else {
                    *plan = LoosePlan::Duplicate;
                    duplicates += 1;
                }
            }
            // 指纹缺失(读盘失败)照常复制,不参与会话去重
            LoosePlan::Copy { .. } | LoosePlan::Transcode { .. } => {}
        }
    }

    // 第三遍(并行):按计划落盘。worker 动态领取,大文件复制与 ffmpeg 子进程
    // 不再逐个排队;上限 4 是因为 ffmpeg 内部已吃多核,再高只会挤爆磁盘
    #[derive(Clone, Copy)]
    enum ExecResult {
        Copied,
        Transcoded,
    }
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 4);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: Vec<(usize, Result<ExecResult, String>)> = std::thread::scope(|scope| {
        let final_plans = &final_plans;
        let paths = &paths;
        let next = &next;
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(move || {
                    let mut out: Vec<(usize, Result<ExecResult, String>)> = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if i >= final_plans.len() {
                            break;
                        }
                        match &final_plans[i] {
                            LoosePlan::Copy { dest, .. } => {
                                let r = fs::copy(&paths[i], dest)
                                    .map(|_| ExecResult::Copied)
                                    .map_err(|e| format!("复制 {} 失败: {e}", paths[i].display()));
                                out.push((i, r));
                            }
                            LoosePlan::Transcode { dest } => {
                                // 计划期已保证 ffmpeg 存在,否则该文件被规划为 Skip
                                let ff = ffmpeg.expect("转码计划存在则 ffmpeg 必已定位");
                                let r = transcode_to_flac(ff, &paths[i], dest)
                                    .map(|_| ExecResult::Transcoded)
                                    .map_err(|e| format!("转码 {} 失败: {e}", paths[i].display()));
                                out.push((i, r));
                            }
                            LoosePlan::Skip | LoosePlan::Duplicate => {}
                        }
                    }
                    out
                })
            })
            .collect();
        let mut all: Vec<_> = handles
            .into_iter()
            .flat_map(|h| {
                // 单 worker panic 只丢它领走的那批文件的落盘,不该炸整个导入
                h.join().unwrap_or_else(|p| {
                    eprintln!("[import] 复制 worker panic: {}", crate::panic_message(&*p));
                    Vec::new()
                })
            })
            .collect();
        // 按文件顺序归位:错误取首个(与旧串行实现一致的报错口径)
        all.sort_by_key(|(i, _)| *i);
        all
    });

    let mut files_copied = 0u32;
    let mut transcoded = 0u32;
    for (_, r) in &results {
        match r {
            Ok(ExecResult::Copied) => files_copied += 1,
            Ok(ExecResult::Transcoded) => transcoded += 1,
            Err(e) => return Err(e.clone()),
        }
    }
    Ok(LooseImport {
        folders_added,
        files_copied,
        transcoded,
        skipped,
        duplicates,
        session_shas,
    })
}

pub(crate) fn import_paths_blocking(
    paths: &[String],
    app: &tauri::AppHandle,
) -> Result<ImportReport, String> {
    let state = app.state::<AppState>();
    let import_dir = ensure_import_dir(app, &state)?;
    let path_bufs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    // 惰性探测 ffmpeg:PATH 探测要起一个子进程,只有批次里真有非常规格式
    // 才值得付这个成本(在线下载永远是 mp3/m4a/flac 直放格式,永远不含)
    let ffmpeg = if batch_needs_ffmpeg(&path_bufs) {
        find_ffmpeg(app)
    } else {
        None
    };
    let loose = import_loose(&path_bufs, &import_dir, &state, ffmpeg.as_deref())?;
    let report = ImportReport {
        folders_added: loose.folders_added,
        files_copied: loose.files_copied,
        transcoded: loose.transcoded,
        skipped: loose.skipped,
        duplicates: loose.duplicates,
    };

    // 只有真正动了曲库才继续:重复导入/在线单曲判重命中时,
    // 全库 walkdir 重扫 + 整库重写纯属浪费
    if loose.changed() {
        // 扫描入库并把新导入文件的 SHA 指纹写进曲库——散装导入与音乐包导入
        // 从此共用同一套内容指纹,互相去重
        scanner::run_scan(app)?;
        record_track_shas(&state, &loose.session_shas)?;
        if report.folders_added > 0 {
            refresh_asset_scope(app);
        }
        // 曲库已被本命令改写:与其他导入路径对齐,广播事件让监听方刷新
        // (handle_open_paths/import_tmc 等都发,唯独这里漏了)
        let _ = app.emit("library-changed", ());
    }
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

/// 批次里是否含有需要转码的非常规格式:决定要不要为它探测一次 ffmpeg。
/// PATH 探测要起一个 `ffmpeg -version` 子进程(几十毫秒),在线下载与
/// 纯歌词/直放格式批次永远不含这些扩展名,不必白付
fn batch_needs_ffmpeg(paths: &[PathBuf]) -> bool {
    paths.iter().any(|p| {
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| scanner::TRANSCODE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
            .unwrap_or(false)
    })
}

/// 出单个散装文件的落盘计划;只收音频与 .lrc 歌词。
/// 非常规格式(ape/wma 等,见 TRANSCODE_EXTS)在有 ffmpeg 时计划转成 FLAC 入库,原文件不动。
/// 去重规则:同名同大小视为已存在;与曲库已知文件大小相同且内容一致视为重复;
/// 同名但大小不同视为新版本,覆盖旧文件。转码件以「目标 flac 已存在且源未更新」判重。
/// 只读不写,使 SHA 全量读盘可以安全并行;写盘统一由执行阶段负责
fn plan_loose_file(
    src: &Path,
    import_dir: &Path,
    known: &[(String, u64)],
    known_shas: &std::collections::HashSet<String>,
    ffmpeg: Option<&Path>,
) -> LoosePlan {
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    let Some(ext) = ext else {
        return LoosePlan::Skip;
    };
    let is_lyrics = ext == "lrc";
    let is_direct = scanner::EXTENSIONS.contains(&ext.as_str());
    let is_transcode = scanner::TRANSCODE_EXTS.contains(&ext.as_str());
    if !is_lyrics && !is_direct && !is_transcode {
        return LoosePlan::Skip;
    }
    // 非常规格式:有 ffmpeg 才转 FLAC(封面走 attached_pic,容器不支持时自动退回纯音频);
    // 没有 ffmpeg 就静默跳过——报告的 skipped 由此变大,由前端提示安装 ffmpeg
    if is_transcode {
        let Some(stem) = src.file_stem().filter(|_| ffmpeg.is_some()) else {
            return LoosePlan::Skip;
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
                return LoosePlan::Skip;
            }
        }
        return LoosePlan::Transcode { dest };
    }
    let Some(name) = src.file_name() else {
        return LoosePlan::Skip;
    };
    let dest = import_dir.join(name);
    if dest.exists() {
        let same = match (fs::metadata(src), fs::metadata(&dest)) {
            (Ok(a), Ok(b)) => a.len() == b.len(),
            _ => false,
        };
        if same {
            return LoosePlan::Skip;
        }
        // 同名不同大小:新版本,覆盖旧文件(与旧实现一致,不再做内容去重)
        return LoosePlan::Copy { dest, sha: None };
    }
    if let Ok(m) = fs::metadata(src) {
        // 内容级去重:先查全库 SHA 指纹(O(1)),未命中再退回同大小逐字节比对
        // (候选最多比 5 个,避免大文件 IO 过久)
        if let Some(sha) = sha256_hex(src) {
            let sha_lc = sha.to_ascii_lowercase();
            if known_shas.contains(&sha_lc) {
                return LoosePlan::Duplicate;
            }
            let size = m.len();
            let mut checked = 0u32;
            for (p, s) in known {
                if *s == size && checked < 5 {
                    checked += 1;
                    if same_file_content(src, Path::new(p)) {
                        return LoosePlan::Duplicate;
                    }
                }
            }
            return LoosePlan::Copy {
                dest,
                sha: Some(sha_lc),
            };
        }
    }
    LoosePlan::Copy { dest, sha: None }
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
            let mut loose_changed = false;
            if !loose.is_empty() {
                // 「打开方式」通道的散装清单只收直放格式与歌词(下方过滤保证),
                // 按批次内容惰性探测 ffmpeg,尽量不起 -version 子进程
                let ffmpeg = if batch_needs_ffmpeg(&loose) {
                    find_ffmpeg(&app)
                } else {
                    None
                };
                match import_loose(&loose, &import_dir, &state, ffmpeg.as_deref()) {
                    Ok(r) => {
                        loose_changed = r.changed();
                        all_session_shas.extend(r.session_shas);
                    }
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
                #[cfg(desktop)]
                show_main(&app);
            }
            if loose_changed {
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
            // 曲库真的变了才广播:纯重复/不支持的批次不值得触发整库刷新
            if loose_changed {
                let _ = app.emit("library-changed", ());
            }
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
        batch_needs_ffmpeg, media_ext_from_url, plan_loose_file, same_file_content, sha256_hex,
        LoosePlan,
    };
    use std::fs;
    use std::path::Path;

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
    fn plan_skips_transcodables_without_ffmpeg() {
        // 无 ffmpeg:非常规格式在规划期即放弃,不产生任何文件;歌词照常计划复制
        let dir = std::env::temp_dir().join(format!("tm-import-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let imp = dir.join("imp");
        fs::create_dir_all(&imp).unwrap();
        let ape = dir.join("凉宫春日.ape");
        fs::write(&ape, b"ape-bytes").unwrap();
        assert!(matches!(
            plan_loose_file(&ape, &imp, &[], &Default::default(), None),
            LoosePlan::Skip
        ));
        let lrc = dir.join("凉宫春日.lrc");
        fs::write(&lrc, "[00:01]test").unwrap();
        assert!(matches!(
            plan_loose_file(&lrc, &imp, &[], &Default::default(), None),
            LoosePlan::Copy { .. }
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_transcodes_with_ffmpeg_and_respects_fresh_dest() {
        // 有 ffmpeg:非常规格式计划转码;目标 flac 已存在且源未更新则跳过
        let dir = std::env::temp_dir().join(format!("tm-plan-ff-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let imp = dir.join("imp");
        fs::create_dir_all(&imp).unwrap();
        let ape = dir.join("凉宫春日.ape");
        fs::write(&ape, b"ape-bytes").unwrap();
        let ffmpeg = Path::new("ffmpeg-stub");
        let plan = plan_loose_file(&ape, &imp, &[], &Default::default(), Some(ffmpeg));
        match plan {
            LoosePlan::Transcode { dest } => assert_eq!(dest, imp.join("凉宫春日.flac")),
            _ => panic!("有 ffmpeg 时应计划转码"),
        }
        // 目标已存在且比源新:计划跳过
        fs::write(imp.join("凉宫春日.flac"), b"fresh flac").unwrap();
        assert!(matches!(
            plan_loose_file(&ape, &imp, &[], &Default::default(), Some(ffmpeg)),
            LoosePlan::Skip
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn batch_ffmpeg_probe_only_for_transcodables() {
        // 惰性探测判据:直放格式与歌词批次不起 ffmpeg 子进程,含非常规格式才起;
        // 大小写不敏感(WMA/wma 同判)
        let mp3 = std::path::PathBuf::from("a.mp3");
        let lrc = std::path::PathBuf::from("b.lrc");
        assert!(!batch_needs_ffmpeg(&[mp3.clone(), lrc]));
        assert!(batch_needs_ffmpeg(&[
            mp3,
            std::path::PathBuf::from("c.WMA")
        ]));
        assert!(!batch_needs_ffmpeg(&[]));
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

        // 曲库里已有的原始文件
        let origin = dir.join("song.mp3");
        fs::write(&origin, b"same-audio-content").unwrap();
        let known = vec![(origin.to_string_lossy().to_string(), 18)];

        // 同内容不同名 → 重复拒收(字节比对兜底)
        let dup = dir.join("song (1).mp3");
        fs::write(&dup, b"same-audio-content").unwrap();
        assert!(matches!(
            plan_loose_file(&dup, &imp, &known, &no_shas, None),
            LoosePlan::Duplicate
        ));

        // 不同内容 → 计划复制,并带出指纹供批内会话去重
        let fresh = dir.join("other.mp3");
        fs::write(&fresh, b"different-content").unwrap();
        match plan_loose_file(&fresh, &imp, &known, &no_shas, None) {
            LoosePlan::Copy { dest, sha } => {
                assert_eq!(dest, imp.join("other.mp3"));
                assert!(sha.is_some(), "复制计划应带内容指纹");
            }
            _ => panic!("不同内容应计划复制"),
        }

        // 同名同大小 → 已存在跳过:先把文件真正放进导入目录再规划同名源
        let exists = dir.join("same-name.mp3");
        fs::write(&exists, b"existing-bytes").unwrap();
        fs::copy(&exists, imp.join("same-name.mp3")).unwrap();
        assert!(matches!(
            plan_loose_file(&exists, &imp, &known, &no_shas, None),
            LoosePlan::Skip
        ));

        // 同名但大小不同 → 新版本,计划覆盖旧文件
        fs::write(&exists, b"existing-bytes-v2-longer").unwrap();
        assert!(matches!(
            plan_loose_file(&exists, &imp, &known, &no_shas, None),
            LoosePlan::Copy { .. }
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

        assert!(matches!(
            plan_loose_file(&incoming, &imp, &[], &known_shas, None),
            LoosePlan::Duplicate
        ));
        assert!(!imp.join("new_name.mp3").exists(), "重复内容不落盘");
        fs::remove_dir_all(&dir).ok();
    }
}
