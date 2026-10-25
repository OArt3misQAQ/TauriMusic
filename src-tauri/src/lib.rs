mod assoc;
mod enrich;
mod import;
mod lyrics;
// 托盘 + SMTC 与启动守卫都是桌面专属设施,Android 无对应系统组件
#[cfg(desktop)]
mod media;
mod model;
mod online;
mod pack;
mod playlists;
mod scanner;
#[cfg(desktop)]
mod single_instance;
mod stats;
mod trash;

use model::Library;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use tauri::{ipc::InvokeResponseBody, Manager, State};
// media://sys 托盘动作监听只在桌面接
#[cfg(desktop)]
use tauri::Listener;
// 窗口事件(点 × 隐藏、托盘面板焦点抖动)只在桌面接
#[cfg(desktop)]
use tauri::WindowEvent;
// 跨源标签归一化与酷我字段解析助手已下沉到 online.rs(纯函数,测试随迁):
// enrich.rs 直接 use crate::online:: 取用;online.rs 自身仍经 crate::html_unescape
// 解析,这条重导出就是它的锚点
pub(crate) use online::html_unescape;
// examples/make_tmc 的在线匹配调试入口从 crate 根取网易云客户端与匹配链函数,保持原路径可用
pub use enrich::{
    generic_client, itunes_album_cover, kuwo_album_cover, kuwo_lyric, netease_album_cover,
    netease_lyric,
};
pub use online::netease_client;
// 各职责模块的实现函数与报告类型;#[tauri::command] 薄包装仍留在本文件
use assoc::AssocState;
use enrich::{
    ensure_online_cover, netease_enrich_blocking, online_download_blocking, NeteaseReport,
    OnlineDownloadMeta,
};
use import::{ensure_import_dir, handle_open_paths, import_paths_blocking, ImportReport};
use pack::{
    dedup_dest, import_playlist_tmcl_blocking, import_tmc_file, import_tmca_blocking,
    inspect_package_blocking, is_under, place_archive, stage_tmc_blocking, stage_tmca_blocking,
    stage_tmcl_blocking, PackImportReport, PackagePreview,
};
use playlists::{
    add_tracks_to_playlist_blocking, create_playlist_blocking, delete_playlist_blocking,
    move_playlist_entry_blocking, remove_playlist_entry_blocking, rename_playlist_blocking,
    AddToPlaylistReport,
};
use trash::{
    delete_tracks_blocking, purge_expired_trash, purge_trash_blocking, restore_tracks_blocking,
    DeleteReport, RestoreReport,
};

/// 统计落盘节流:上次持久化的 unix 秒。内存态每次冲刷都合并,
/// 磁盘全量重写(stats.json 随历史增长)压到最低 2 分钟一次,强冲不受限
static STATS_LAST_SAVE: AtomicU64 = AtomicU64::new(0);

// 主窗口唤起/切换只发生在桌面(单实例唤起、托盘动作);Android 的主窗口由系统管理
#[cfg(desktop)]
pub(crate) fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[cfg(desktop)]
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

/// 写库命令与扫描互斥的公共闸:必须先于曲库锁获取(全仓库统一「闸→lib」加锁顺序,
/// 反序会在"扫描持闸等 lib"与"写库持 lib 等闸"之间死锁)。
/// 用阻塞锁而非 try_lock 放行:扫描期间曲目被扫描器借走(lib.tracks 为空),
/// 放行写库命令会把 tracks 为空的中间态持久化进 library.json——崩溃后主文件是
/// 合法 JSON,连 .bak 回退都不会触发,等于丢库;排队等扫描收尾同时也消除了
/// "删除/还原与扫描整体替换 tracks"的复活竞态
pub(crate) fn lock_scan_gate(state: &AppState) -> MutexGuard<'_, ()> {
    // 上次扫描线程 panic 后锁中毒:恢复互斥数据继续,
    // 把"毒化"误报成"占用"会让之后的写库命令永久失败
    state.scan_gate.lock().unwrap_or_else(|e| e.into_inner())
}

/// 收紧 asset 协议面:只放行封面缓存与已登记曲库文件夹下的文件。
/// tauri.conf.json 的静态 `**` 范围已移除,未登记路径无法经 asset: 访问,
/// 被投毒的前端无法再借封面/音频通道探测任意本地文件。
pub(crate) fn refresh_asset_scope(app: &tauri::AppHandle) {
    let scope = app.asset_protocol_scope();
    let _ = scope.allow_directory(app.state::<AppState>().cache_dir.join("covers"), true);
    if let Ok(lib) = app.state::<AppState>().lib.lock() {
        for folder in &lib.folders {
            let _ = scope.allow_directory(folder, true);
        }
    }
}

#[tauri::command]
fn get_import_dir(app: tauri::AppHandle, state: State<AppState>) -> Result<String, String> {
    let dir = ensure_import_dir(&app, &state)?;
    Ok(dir.to_string_lossy().into_owned())
}

/// 打开导入目录(桌面调资源管理器;移动端无独立文件管理器概念)
#[tauri::command]
fn open_import_dir(app: tauri::AppHandle, state: State<AppState>) -> Result<String, String> {
    #[cfg(desktop)]
    {
        let dir = ensure_import_dir(&app, &state)?;
        std::process::Command::new("explorer")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("打开资源管理器失败: {e}"))?;
        Ok(dir.to_string_lossy().into_owned())
    }
    #[cfg(mobile)]
    {
        let _ = (&app, &state);
        Err("移动端不支持打开文件管理器".into())
    }
}

/// 拖拽导入:整个文件夹登记为扫描来源(不复制),散装音频/歌词文件复制进导入目录
#[tauri::command]
async fn import_paths(paths: Vec<String>, app: tauri::AppHandle) -> Result<ImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || import_paths_blocking(&paths, &app))
        .await
        .map_err(|e| e.to_string())?
}

/// 解包 .tmc 到导入目录下的同名子目录,由扫描器拾取(meta.json 会补齐缺失标签)。
/// 内容级去重:音频 SHA-256 与曲库已有曲目相同则整目录清理并跳过(与包导入同标准)
#[tauri::command]
async fn import_tmc(path: String, app: tauri::AppHandle) -> Result<PackImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || import_tmc_file(Path::new(&path), &app))
        .await
        .map_err(|e| e.to_string())?
}

/// 解析音乐包内容供前端预览:解包到 covers/preview 暂存(资产协议已放行),
/// 只读元数据不写库;确认导入走原有命令重新解包到导入目录,取消则整目录清理
#[tauri::command]
async fn inspect_package(path: String, app: tauri::AppHandle) -> Result<PackagePreview, String> {
    tauri::async_runtime::spawn_blocking(move || inspect_package_blocking(&path, &app))
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

/// Android 系统分享收件箱里的一条音频(由 MainActivity 复制内容 URI 后落盘)
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedAudio {
    path: String,
    name: String,
}

/// 分享收件箱:cache/shared-inbox。Kotlin 侧每复制完一首就写一份同名 .json 说明,
/// 用"一文件一说明"而不是共享队列文件,读写两端无需加锁也不会互相覆盖
fn shared_inbox_dir(cache_dir: &Path) -> PathBuf {
    cache_dir.join("shared-inbox")
}

/// 与扫描器直放格式保持一致:移动端没有 ffmpeg,转码格式分享进来也扫不到,
/// 提前丢弃避免把用户文件搬进曲库目录后变成"看不见的垃圾"
fn is_shared_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            scanner::EXTENSIONS
                .iter()
                .any(|x| x.eq_ignore_ascii_case(e))
        })
        .unwrap_or(false)
}

/// 读走收件箱说明文件并返回可用条目。条目指向收件箱外的路径一律丢弃:
/// 说明文件虽然是本应用写的,但缓存目录可被其他调试工具改写,不能当可信输入
fn take_shared_audio_entries(cache_dir: &Path) -> Vec<SharedAudio> {
    let inbox = shared_inbox_dir(cache_dir);
    let Ok(entries) = fs::read_dir(&inbox) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let sidecar = entry.path();
        if sidecar.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let parsed = fs::read_to_string(&sidecar)
            .ok()
            .and_then(|text| serde_json::from_str::<SharedAudio>(&text).ok());
        // 说明文件一次性消费:解析失败也删,避免每次轮询都卡在坏文件上
        let _ = fs::remove_file(&sidecar);
        let Some(item) = parsed else { continue };
        let path = PathBuf::from(&item.path);
        if !path.starts_with(&inbox) || !path.is_file() || !is_shared_audio(&path) {
            continue;
        }
        out.push(item);
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// 缓存目录在进程内恒定:前端每 3s 轮询一次,不必每次都走 path 插件
fn shared_cache_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    static CACHE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    if let Some(dir) = CACHE.get() {
        return Some(dir.clone());
    }
    let dir = app.path().app_cache_dir().ok()?;
    let _ = CACHE.set(dir.clone());
    Some(dir)
}

/// 拉取系统分享来的音频:返回本地缓存路径,前端走既有导入管线入库并播放
#[tauri::command]
fn take_shared_audio(app: tauri::AppHandle) -> Vec<SharedAudio> {
    match shared_cache_dir(&app) {
        Some(cache) => take_shared_audio_entries(&cache),
        None => Vec::new(),
    }
}

/// 导入完成后删除分享缓存副本:曲库已有一份,缓存不必再占空间
#[tauri::command]
fn discard_shared_audio(paths: Vec<String>, app: tauri::AppHandle) {
    let Some(cache) = shared_cache_dir(&app) else {
        return;
    };
    let inbox = shared_inbox_dir(&cache);
    for raw in paths {
        let path = PathBuf::from(raw);
        if path.starts_with(&inbox) {
            let _ = fs::remove_file(path);
        }
    }
}

/// 显示桌面歌词小窗(常驻窗口,启动即创建;显示/隐藏状态由前端记忆)。
/// 移动端无多窗口,前端不应调到这里;保底返回成功避免 UI 报错
#[tauri::command]
fn show_lyrics_window(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(desktop)]
    {
        let w = app.get_webview_window("lyrics").ok_or("歌词窗口不存在")?;
        let _ = w.show();
        Ok(())
    }
    #[cfg(mobile)]
    {
        let _ = &app;
        Ok(())
    }
}

/// 隐藏桌面歌词小窗(移动端同为无操作)
#[tauri::command]
fn hide_lyrics_window(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(desktop)]
    {
        let w = app.get_webview_window("lyrics").ok_or("歌词窗口不存在")?;
        let _ = w.hide();
        Ok(())
    }
    #[cfg(mobile)]
    {
        let _ = &app;
        Ok(())
    }
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

#[tauri::command]
async fn pick_tmc_file() -> Option<String> {
    #[cfg(desktop)]
    {
        rfd::AsyncFileDialog::new()
            .set_title("选择音乐包(TMC / TMCL)")
            .add_filter("音乐包 (*.tmc, *.tmcl, *.tmca)", &["tmc", "tmcl", "tmca"])
            .pick_file()
            .await
            .map(|f| f.path().to_string_lossy().to_string())
    }
    // 移动端没有原生文件选择对话框:返回 None 走"用户取消"分支,
    // 前端按平台隐藏入口后正常不会调到这里
    #[cfg(mobile)]
    None
}

#[tauri::command]
async fn pick_audio_files() -> Option<Vec<String>> {
    #[cfg(desktop)]
    {
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
    #[cfg(mobile)]
    None
}

/// 批量导出的目标文件夹
#[tauri::command]
async fn pick_export_dir() -> Option<String> {
    #[cfg(desktop)]
    {
        rfd::AsyncFileDialog::new()
            .set_title("选择导出位置")
            .pick_folder()
            .await
            .map(|f| f.path().to_string_lossy().to_string())
    }
    #[cfg(mobile)]
    None
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

/// 导入 .tmca 专辑包:解包到导入目录 → 校验/去重 → 扫描入库;专辑归类由元数据成立
#[tauri::command]
async fn import_tmca(path: String, app: tauri::AppHandle) -> Result<PackImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || import_tmca_blocking(&path, &app))
        .await
        .map_err(|e| e.to_string())?
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

/// 在线搜索(按来源分发;仅免费曲目可播,详见 online.rs 的合规边界)
#[tauri::command]
async fn online_search(
    source: String,
    keyword: String,
    page: u32,
) -> Result<Vec<online::OnlineTrack>, String> {
    tauri::async_runtime::spawn_blocking(move || online::search(&source, &keyword, page))
        .await
        .map_err(|e| e.to_string())?
}

/// 取在线曲目播放直链(VIP/版权受限曲目报错)。码率固定取各平台匿名免费上限,
/// 能给到多少由 online.rs 决定,不做任何付费试探
#[tauri::command]
async fn online_play_url(source: String, id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || online::play_url(&source, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// 在线曲目封面:落盘封面缓存后返回本地路径。
/// None = 该曲无图;前端把路径当本地封面渲染,播放页/氛围背景/托盘/SMTC 全部复用
#[tauri::command]
async fn online_cover(
    source: String,
    id: String,
    cover_url: Option<String>,
    app: tauri::AppHandle,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        ensure_online_cover(&source, &id, cover_url.as_deref(), &app)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 在线曲目保存到本地库:取免费直链 → 下载 → 写标签/封面/旁路歌词 → 临时文件走
/// 既有散装导入管线(复制进导入目录 + 扫描归位 + SHA 指纹去重)。
/// 付费/受限曲目在取链一步就报错,与在线播放共用同一道闸,不做任何绕过
#[tauri::command]
async fn online_download(
    source: String,
    id: String,
    meta: OnlineDownloadMeta,
    app: tauri::AppHandle,
) -> Result<ImportReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        online_download_blocking(&source, &id, &meta, &app)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 删除曲目:音频与同名歌词移入应用回收站(可还原,超期自动清理),
/// 曲库记录转入回收站条目;导入目录内的空子目录一并清理
#[tauri::command]
async fn delete_tracks(ids: Vec<String>, app: tauri::AppHandle) -> Result<DeleteReport, String> {
    tauri::async_runtime::spawn_blocking(move || delete_tracks_blocking(&ids, &app))
        .await
        .map_err(|e| e.to_string())?
}

/// 还原回收站条目:文件移回原位,曲库记录恢复(触发扫描后重新可见)
#[tauri::command]
async fn restore_tracks(ids: Vec<String>, app: tauri::AppHandle) -> Result<RestoreReport, String> {
    tauri::async_runtime::spawn_blocking(move || restore_tracks_blocking(&ids, &app))
        .await
        .map_err(|e| e.to_string())?
}

/// 彻底删除回收站条目:ids 为空表示清空全部。文件直接删除(不可恢复)
#[tauri::command]
async fn purge_trash(ids: Vec<String>, app: tauri::AppHandle) -> Result<u32, String> {
    tauri::async_runtime::spawn_blocking(move || purge_trash_blocking(&ids, &app))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn create_playlist(
    name: String,
    app: tauri::AppHandle,
) -> Result<Vec<model::Playlist>, String> {
    // 整库 JSON 落盘是磁盘活:同步命令跑主线程,大曲库/慢盘下会卡 UI 帧
    tauri::async_runtime::spawn_blocking(move || create_playlist_blocking(&app, name))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn rename_playlist(
    id: String,
    name: String,
    app: tauri::AppHandle,
) -> Result<Vec<model::Playlist>, String> {
    tauri::async_runtime::spawn_blocking(move || rename_playlist_blocking(&app, id, name))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn delete_playlist(
    id: String,
    app: tauri::AppHandle,
) -> Result<Vec<model::Playlist>, String> {
    tauri::async_runtime::spawn_blocking(move || delete_playlist_blocking(&app, id))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn add_tracks_to_playlist(
    playlist_id: String,
    track_ids: Vec<String>,
    app: tauri::AppHandle,
) -> Result<AddToPlaylistReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        add_tracks_to_playlist_blocking(&app, playlist_id, track_ids)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn remove_playlist_entry(
    playlist_id: String,
    index: usize,
    app: tauri::AppHandle,
) -> Result<Vec<model::Playlist>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        remove_playlist_entry_blocking(&app, playlist_id, index)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 调整曲目顺序:把 from 位置的条目移动到 to 位置
#[tauri::command]
async fn move_playlist_entry(
    playlist_id: String,
    from: usize,
    to: usize,
    app: tauri::AppHandle,
) -> Result<Vec<model::Playlist>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        move_playlist_entry_blocking(&app, playlist_id, from, to)
    })
    .await
    .map_err(|e| e.to_string())?
}

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

#[tauri::command]
fn get_associations() -> Vec<AssocState> {
    assoc::get_associations()
}

#[tauri::command]
fn set_association(ext: String, enable: bool) -> Result<(), String> {
    assoc::set_association(ext, enable)
}

#[tauri::command]
fn get_autostart() -> bool {
    assoc::get_autostart()
}

#[tauri::command]
fn set_autostart(enable: bool) -> Result<(), String> {
    assoc::set_autostart(enable)
}

// ===== 听歌统计(仅本机,零联网):采集在前端,这里只做合并与原子落盘 =====

/// 冲刷一批增量:每曲收听秒数 + 播放次数 + 当日总时长。
/// 合并在内存锁内完成(微秒级),落盘放 blocking 线程并做节流:
/// 播放中每 20s 冲一次内存,若每次都全量重写 stats.json,历史越大写放大越重,
/// 磁盘写入压到最低 2 分钟一次;force(隐藏/退出/关开关)与进程退出路径不受限
#[tauri::command]
async fn stats_add(
    state: State<'_, AppState>,
    samples: HashMap<String, f64>,
    plays: Vec<String>,
    day: String,
    day_sec: f64,
    online_meta: HashMap<String, stats::OnlineTrackMeta>,
    force: Option<bool>,
) -> Result<(), String> {
    let path = state.data_dir.join(stats::STATS_FILE);
    let snapshot = {
        let mut s = state.stats.lock().map_err(|_| "统计锁中毒")?;
        s.merge(
            &samples,
            &plays,
            &day,
            day_sec,
            &online_meta,
            stats::now_secs(),
        );
        let now = stats::now_secs();
        let due = force.unwrap_or(false)
            || now.saturating_sub(STATS_LAST_SAVE.load(Ordering::Relaxed)) >= 120;
        // 先判 due 再决定是否克隆:节流未到期时整份历史统计的深克隆是纯浪费
        due.then(|| s.clone())
    };
    let Some(snapshot) = snapshot else {
        return Ok(());
    };
    tauri::async_runtime::spawn_blocking(move || snapshot.save(&path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    STATS_LAST_SAVE.store(stats::now_secs(), Ordering::Relaxed);
    Ok(())
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
        *s = stats::StatsData::empty();
        s.clone()
    };
    tauri::async_runtime::spawn_blocking(move || snapshot.save(&path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

// ===== 系统媒体控制(SMTC)+ 托盘菜单的状态推送(见 media.rs) =====
// Android 系统的媒体通知走 MediaSession,由 WebView 的媒体元素自行对接,
// 这里无需推送;命令保留并在移动端空转,前端调用面保持两端一致

#[tauri::command]
fn smtc_set_track(
    app: tauri::AppHandle,
    title: String,
    artist: String,
    cover: Option<String>,
    duration: f64,
) -> Result<(), String> {
    #[cfg(desktop)]
    {
        media::on_track(&app, title, artist, cover, duration);
        Ok(())
    }
    #[cfg(mobile)]
    {
        let _ = (app, title, artist, cover, duration);
        Ok(())
    }
}

#[tauri::command]
fn smtc_set_status(app: tauri::AppHandle, playing: bool) -> Result<(), String> {
    #[cfg(desktop)]
    {
        media::on_status(&app, playing);
        Ok(())
    }
    #[cfg(mobile)]
    {
        let _ = (app, playing);
        Ok(())
    }
}

#[tauri::command]
fn smtc_set_position(app: tauri::AppHandle, position: f64, duration: f64) -> Result<(), String> {
    #[cfg(desktop)]
    {
        media::on_position(&app, position, duration);
        Ok(())
    }
    #[cfg(mobile)]
    {
        let _ = (app, position, duration);
        Ok(())
    }
}

#[tauri::command]
async fn get_library(app: tauri::AppHandle) -> Result<InvokeResponseBody, String> {
    // 锁内直接序列化成 JSON 字符串原样透传(InvokeResponseBody::Json),
    // 不再先建整棵 serde_json::Value 树:大曲库时启动/重扫的瞬时内存峰值显著更低;
    // Json 变体以 application/json 返回,前端 invoke<Library> 拿到的仍是解析好的对象。
    // 整库序列化是毫秒级纯 CPU 活,但同步命令跑主线程照样卡 UI 帧 ——
    // 挪进阻塞线程执行(与 scan_library 的收尾同构);命令名与入参不变,前端透明。
    // 持 State 引用进不了 spawn_blocking('static),改经 AppHandle 在闭包内取状态
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let lib = state.lib.lock().map_err(|_| "曲库状态不可用".to_string())?;
        serde_json::to_string(&*lib)
            .map(InvokeResponseBody::Json)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn add_folder(path: String, app: tauri::AppHandle) -> Result<(), String> {
    // 落盘与 asset 范围刷新都不该占主线程(同步命令跑主线程会卡 UI 帧)
    tauri::async_runtime::spawn_blocking(move || {
        if !Path::new(&path).is_dir() {
            return Err("该路径不是一个文件夹".into());
        }
        let state = app.state::<AppState>();
        // 写库一律互斥扫描:扫描期间曲目被借走,照常落盘会持久化空曲库
        let _gate = lock_scan_gate(&state);
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
        lib.folders.push(path.clone());
        // 锁内只序列化,磁盘 IO 在锁外;失败重拿锁回滚
        let bytes = lib.to_bytes().map_err(|e| e.to_string())?;
        drop(lib);
        if let Err(e) = Library::write_bytes(&state.data_dir.join(LIBRARY_FILE), &bytes) {
            if let Ok(mut lib) = state.lib.lock() {
                lib.folders.retain(|f| *f != path);
            }
            return Err(e.to_string());
        }
        // 新登记的文件夹要同步进 asset 协议放行名单,否则封面/音频加载不出来
        refresh_asset_scope(&app);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn remove_folder(path: String, app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _gate = lock_scan_gate(&state);
        let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        lib.folders.retain(|f| f != &path);
        // 组件级前缀匹配:字符串 starts_with 会把 F:\music2 误当成 F:\music 的子目录
        lib.tracks.retain(|t| !is_under(&t.path, &path));
        let bytes = lib.to_bytes().map_err(|e| e.to_string())?;
        drop(lib);
        Library::write_bytes(&state.data_dir.join(LIBRARY_FILE), &bytes)
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn pick_music_folder() -> Option<String> {
    #[cfg(desktop)]
    {
        rfd::AsyncFileDialog::new()
            .set_title("选择音乐文件夹")
            .pick_folder()
            .await
            .map(|f| f.path().to_string_lossy().to_string())
    }
    #[cfg(mobile)]
    None
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

/// 在线歌词进程内缓存:切歌往返不再重复请求平台;
/// 键为完整在线 id(含来源前缀),None 表示该歌确实无词(同样缓存,避免反复打接口)。
/// 容量超限整体清空——歌词载荷很小,重建缓存只是几次轻量请求
static ONLINE_LYRIC_CACHE: std::sync::OnceLock<
    Mutex<HashMap<String, Option<lyrics::LyricsPayload>>>,
> = std::sync::OnceLock::new();
const ONLINE_LYRIC_CACHE_CAP: usize = 128;

#[tauri::command]
async fn get_lyrics(id: String, app: tauri::AppHandle) -> Option<lyrics::LyricsPayload> {
    // 在线曲目:按来源前缀分发到各家公开歌词接口,进程内缓存命中即回
    if let Some((source, platform_id)) = online::split_online_id(&id) {
        // get_or_init 保证静态真的被初始化(纯 get() 在首拍永远是 None)
        let cache = ONLINE_LYRIC_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Ok(g) = cache.lock() {
            if let Some(hit) = g.get(&id) {
                return hit.clone();
            }
        }
        let src = source.to_string();
        let pid = platform_id.to_string();
        // 外层 Option=请求是否成功(失败不缓存,下次切回可重试);
        // 内层 None=该歌确实无词,同样落缓存,避免反复打接口
        let fetched: Option<Option<lyrics::LyricsPayload>> =
            match tauri::async_runtime::spawn_blocking(move || online::lyric_payload(&src, &pid))
                .await
            {
                Ok(Ok(payload)) => Some(payload),
                _ => None,
            };
        if let Some(payload) = &fetched {
            if let Ok(mut g) = cache.lock() {
                if g.len() >= ONLINE_LYRIC_CACHE_CAP {
                    g.clear();
                }
                g.insert(id.clone(), payload.clone());
            }
        }
        return fetched.flatten();
    }
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

/// 兜底清理异常退出残留在系统 Temp 的应用临时目录:
/// taurimusic-stage-*(导出暂存)与 taurimusic-dl-*(在线下载暂存)。
/// 正常流程各自会清理(前端 cleanup_stage / 下载命令收尾),这里只兜崩溃/断电
fn cleanup_stale_temp_dirs() {
    for entry in fs::read_dir(std::env::temp_dir())
        .into_iter()
        .flatten()
        .flatten()
    {
        let p = entry.path();
        let ours = p
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("taurimusic-stage-") || n.starts_with("taurimusic-dl-"));
        if ours {
            let _ = fs::remove_dir_all(&p);
        }
    }
}

/// 从 panic payload 里尽量还原消息:Box<dyn Any> 不实现 Display,
/// 直接 format 只会得到类型名,排障时没有信息量
pub(crate) fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&'static str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(|s| s.as_str()))
        .unwrap_or("未知 panic")
        .to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 启动守卫：已有实例运行时询问是否重启（旧实例卡死时的兜底）；
    // 放行与否最终仍由下面的单实例插件裁定，这里只负责询问与清场。
    // Android 单任务应用无多实例概念,守卫只在桌面编译
    #[cfg(desktop)]
    if !single_instance::guard_startup() {
        return;
    }
    let builder = tauri::Builder::default();
    // 桌面专属接线:二次启动唤起已有窗口并把传入文件交给运行中实例;
    // 窗口事件——点 × 隐藏到托盘常驻、托盘迷你面板失焦即隐藏。
    // Android 无多窗口/托盘,也不存在"二次启动进运行实例"的路径
    #[cfg(desktop)]
    let builder = builder
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
        });
    builder
        .setup(|app| {
            let handle = app.handle();
            let data_dir = handle.path().app_data_dir().expect("无法确定应用数据目录");
            let cache_dir = handle.path().app_cache_dir().expect("无法确定应用缓存目录");
            std::fs::create_dir_all(&data_dir).ok();
            std::fs::create_dir_all(cache_dir.join("covers")).ok();
            // 清理上次预览遗留的暂存目录:确认/取消时都会清理,这里兜底异常退出残留
            let _ = fs::remove_dir_all(cache_dir.join("covers").join("preview"));
            // 兜底清理异常退出残留的导出/下载暂存目录:崩溃/断电时整包音频
            // 会永久留在系统 Temp(可能几百 MB × 多个)
            cleanup_stale_temp_dirs();
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
            // 命令行/双击"打开方式"传入的文件(Android 下无命令行参数,空转)
            let cli_args: Vec<String> = std::env::args().collect();
            handle_open_paths(handle, &cli_args);

            #[cfg(desktop)]
            {
                // 窗口在配置里全部 create:false,由这里手动创建:Tauri 内部顺序是
                // 先建窗口(WebView 立即加载前端)后跑本钩子,冷启动时前端会抢在
                // .manage 之前调命令,报「state not managed」→「曲库读取失败」。
                // 状态注册完毕后才开始加载页面,竞态从根上消除。
                // Android 的主 WebView 由 Activity 自己创建,不走这份配置
                for window_config in app.config().app.windows.iter() {
                    tauri::WebviewWindowBuilder::from_config(handle, window_config)?.build()?;
                }

                // 托盘 + 系统媒体控制(SMTC):迷你面板/封面/进度/播放控制都在 media.rs
                media::init(handle)?;

                // 托盘迷你面板的系统级动作(退出/唤起主窗口)由前端 emit 过来
                let sys_handle = handle.clone();
                handle.listen("media://sys", move |event| {
                    match event.payload().trim_matches('"') {
                        "quit" => {
                            // 真正退出前把内存里的统计增量落盘:落盘节流期间
                            // 磁盘副本可能落后内存最多 2 分钟,kill 掉就永久丢了
                            let state = sys_handle.state::<AppState>();
                            if let Ok(s) = state.stats.lock() {
                                if let Err(e) = s.save(&state.data_dir.join(stats::STATS_FILE)) {
                                    eprintln!("[stats] 退出前落盘失败: {e}");
                                }
                            }
                            sys_handle.exit(0)
                        }
                        "show" => crate::show_main(&sys_handle),
                        _ => {}
                    }
                });
            }
            // Android 只有一个主窗口形态:配置里全部 create:false(桌面按上面手动建),
            // 核心 setup 又只自动建 create:true 的窗口,这里必须显式建主窗口,
            // 否则 Activity 常驻空白主题背景,WebView 根本不会被创建
            #[cfg(mobile)]
            for window_config in app
                .config()
                .app
                .windows
                .iter()
                .filter(|w| w.label == "main")
            {
                tauri::WebviewWindowBuilder::from_config(handle, window_config)?.build()?;
            }
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
            take_shared_audio,
            discard_shared_audio,
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
            get_resource_usage,
            online_search,
            online_play_url,
            online_download,
            online_cover
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use crate::model;
    use std::fs;

    /// 构造一个不依赖 Tauri AppHandle 的最小 AppState(指向临时目录)
    fn test_app_state(tag: &str) -> super::AppState {
        let dir = std::env::temp_dir().join(format!("tm-appstate-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        super::AppState {
            lib: std::sync::Mutex::new(model::Library::default()),
            data_dir: dir.clone(),
            cache_dir: dir.join("cache"),
            scan_gate: std::sync::Mutex::new(()),
            stats: std::sync::Mutex::new(crate::stats::StatsData::empty()),
            pending_imports: std::sync::Mutex::new(Vec::new()),
        }
    }

    #[test]
    fn panic_message_extracts_string_payloads() {
        use super::panic_message;
        use std::any::Any;
        // panic!("字面量") 的 payload 是 &'static str,panic!("{}", x) 是 String,
        // 其余类型拿不到文本——统一回退到"未知 panic"
        let s: Box<dyn Any + Send> = Box::new("静态文本");
        let t: Box<dyn Any + Send> = Box::new(String::from("动态文本"));
        let n: Box<dyn Any + Send> = Box::new(42u32);
        assert_eq!(panic_message(&*s), "静态文本");
        assert_eq!(panic_message(&*t), "动态文本");
        assert_eq!(panic_message(&*n), "未知 panic");
    }

    #[test]
    fn shared_inbox_only_takes_audio_inside_inbox() {
        use super::{shared_inbox_dir, take_shared_audio_entries};
        // 说明文件与音频都放在收件箱内才收;指向箱外、扩展名不支持、JSON 损坏的
        // 一律丢弃且说明文件要删掉(否则轮询永远卡在同一批坏文件上)
        let cache = std::env::temp_dir().join(format!("tm-share-{}", std::process::id()));
        let _ = fs::remove_dir_all(&cache);
        let inbox = shared_inbox_dir(&cache);
        fs::create_dir_all(&inbox).unwrap();

        let good = inbox.join("1-song.mp3");
        fs::write(&good, b"fake-audio").unwrap();
        let good_json = inbox.join("1-song.mp3.json");
        fs::write(
            &good_json,
            serde_json::json!({ "path": good.to_string_lossy(), "name": "song.mp3" }).to_string(),
        )
        .unwrap();

        let outside = cache.join("outside.mp3");
        fs::write(&outside, b"fake-audio").unwrap();
        let outside_json = inbox.join("outside.json");
        fs::write(
            &outside_json,
            serde_json::json!({ "path": outside.to_string_lossy(), "name": "outside.mp3" })
                .to_string(),
        )
        .unwrap();

        let image = inbox.join("cover.png");
        fs::write(&image, b"not-audio").unwrap();
        let image_json = inbox.join("cover.png.json");
        fs::write(
            &image_json,
            serde_json::json!({ "path": image.to_string_lossy(), "name": "cover.png" }).to_string(),
        )
        .unwrap();

        let broken_json = inbox.join("broken.json");
        fs::write(&broken_json, b"{not json").unwrap();

        let got = take_shared_audio_entries(&cache);
        assert_eq!(got.len(), 1, "只应接受收件箱内的音频");
        assert_eq!(got[0].path, good.to_string_lossy());
        assert_eq!(got[0].name, "song.mp3");
        assert!(!good_json.exists(), "说明文件应一次性消费");
        assert!(!outside_json.exists(), "箱外条目的说明文件同样要清掉");
        assert!(!image_json.exists(), "非音频条目的说明文件要清掉");
        assert!(!broken_json.exists(), "损坏的说明文件要清掉");
        assert!(outside.exists(), "箱外文件不能被误删");
        let _ = fs::remove_dir_all(&cache);
    }

    #[test]
    fn scan_gate_lock_recovers_from_poisoning() {
        use super::lock_scan_gate;
        // 上次持闸方 panic 会毒化闸:阻塞入口必须 into_inner 硬恢复,
        // 否则之后所有写库命令永久失败
        let state = test_app_state("poison");
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _g = lock_scan_gate(&state);
            panic!("模拟持闸方崩溃");
        }));
        let _g = lock_scan_gate(&state); // 不应死锁/恐慌
    }

    #[test]
    fn cleanup_stale_temp_dirs_removes_only_ours() {
        use super::cleanup_stale_temp_dirs;
        // 进程号保证唯一,避免误删其他进程的临时目录
        let tag = format!("{}", std::process::id());
        let stage = std::env::temp_dir().join(format!("taurimusic-stage-{tag}-test"));
        let dl = std::env::temp_dir().join(format!("taurimusic-dl-{tag}-test"));
        let foreign = std::env::temp_dir().join(format!("tm-foreign-{tag}"));
        for d in [&stage, &dl, &foreign] {
            let _ = fs::create_dir_all(d);
        }
        cleanup_stale_temp_dirs();
        assert!(!stage.exists(), "stage- 前缀残留应被清理");
        assert!(!dl.exists(), "dl- 前缀残留(在线下载暂存)同样应被清理");
        assert!(foreign.exists(), "非应用前缀的目录不能动");
        let _ = fs::remove_dir_all(&foreign);
    }
}
