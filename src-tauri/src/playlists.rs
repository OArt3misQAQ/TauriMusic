//! 用户歌单:CRUD 实现与条目快照/命名等共用助手。

use crate::model::{self, Library};
use crate::scanner;
use crate::{lock_scan_gate, AppState, LIBRARY_FILE};
use tauri::Manager;

// ===== 播放列表(用户歌单:创建/改名/增删曲目/排序/导出导入) =====

/// 当前 unix 秒(带小数):与 stats::now_secs()(整数,统计 last 用)语义不同,
/// 刻意按返回类型命名消歧——回收站时间戳/歌单创建时间是 f64 秒
pub(crate) fn now_secs_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub(crate) fn new_playlist_id(name: &str) -> String {
    format!(
        "{:016x}",
        scanner::hash_str(&format!("{name}:{}", now_secs_f64()))
    )
}

/// 同名时自动追加序号:"歌单" → "歌单 (2)"
pub(crate) fn unique_playlist_name(existing: &[model::Playlist], base: &str) -> String {
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
pub(crate) fn entry_of(t: &model::Track) -> model::PlaylistEntry {
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

/// 把曲目加入播放列表(按路径去重,返回新增数量与最新列表)
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddToPlaylistReport {
    added: u32,
    skipped: u32,
    playlists: Vec<model::Playlist>,
}

pub(crate) fn create_playlist_blocking(
    app: &tauri::AppHandle,
    name: String,
) -> Result<Vec<model::Playlist>, String> {
    let state = app.state::<AppState>();
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("播放列表名称不能为空".into());
    }
    // 写库一律互斥扫描:扫描期间曲目被借走,照常落盘会持久化空曲库
    let _gate = lock_scan_gate(&state);
    let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
    if lib.playlists.iter().any(|p| p.name == name) {
        return Err("已存在同名播放列表".into());
    }
    lib.playlists.push(model::Playlist {
        id: new_playlist_id(&name),
        name,
        created_at: now_secs_f64(),
        entries: Vec::new(),
    });
    let playlists = lib.playlists.clone();
    // 锁内只序列化,磁盘 IO 在锁外
    let bytes = lib.to_bytes().map_err(|e| e.to_string())?;
    drop(lib);
    Library::write_bytes(&state.data_dir.join(LIBRARY_FILE), &bytes).map_err(|e| e.to_string())?;
    Ok(playlists)
}

pub(crate) fn rename_playlist_blocking(
    app: &tauri::AppHandle,
    id: String,
    name: String,
) -> Result<Vec<model::Playlist>, String> {
    let state = app.state::<AppState>();
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("播放列表名称不能为空".into());
    }
    let _gate = lock_scan_gate(&state);
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
    let playlists = lib.playlists.clone();
    let bytes = lib.to_bytes().map_err(|e| e.to_string())?;
    drop(lib);
    Library::write_bytes(&state.data_dir.join(LIBRARY_FILE), &bytes).map_err(|e| e.to_string())?;
    Ok(playlists)
}

pub(crate) fn delete_playlist_blocking(
    app: &tauri::AppHandle,
    id: String,
) -> Result<Vec<model::Playlist>, String> {
    let state = app.state::<AppState>();
    let _gate = lock_scan_gate(&state);
    let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
    lib.playlists.retain(|p| p.id != id);
    let playlists = lib.playlists.clone();
    let bytes = lib.to_bytes().map_err(|e| e.to_string())?;
    drop(lib);
    Library::write_bytes(&state.data_dir.join(LIBRARY_FILE), &bytes).map_err(|e| e.to_string())?;
    Ok(playlists)
}

pub(crate) fn add_tracks_to_playlist_blocking(
    app: &tauri::AppHandle,
    playlist_id: String,
    track_ids: Vec<String>,
) -> Result<AddToPlaylistReport, String> {
    let state = app.state::<AppState>();
    let _gate = lock_scan_gate(&state);
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
    let report = AddToPlaylistReport {
        added,
        skipped,
        playlists: lib.playlists.clone(),
    };
    let bytes = lib.to_bytes().map_err(|e| e.to_string())?;
    drop(lib);
    Library::write_bytes(&state.data_dir.join(LIBRARY_FILE), &bytes).map_err(|e| e.to_string())?;
    Ok(report)
}

pub(crate) fn remove_playlist_entry_blocking(
    app: &tauri::AppHandle,
    playlist_id: String,
    index: usize,
) -> Result<Vec<model::Playlist>, String> {
    let state = app.state::<AppState>();
    let _gate = lock_scan_gate(&state);
    let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
    let pl = lib
        .playlists
        .iter_mut()
        .find(|p| p.id == playlist_id)
        .ok_or("播放列表不存在")?;
    if index < pl.entries.len() {
        pl.entries.remove(index);
    }
    let playlists = lib.playlists.clone();
    let bytes = lib.to_bytes().map_err(|e| e.to_string())?;
    drop(lib);
    Library::write_bytes(&state.data_dir.join(LIBRARY_FILE), &bytes).map_err(|e| e.to_string())?;
    Ok(playlists)
}

pub(crate) fn move_playlist_entry_blocking(
    app: &tauri::AppHandle,
    playlist_id: String,
    from: usize,
    to: usize,
) -> Result<Vec<model::Playlist>, String> {
    let state = app.state::<AppState>();
    let _gate = lock_scan_gate(&state);
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
    let playlists = lib.playlists.clone();
    let bytes = lib.to_bytes().map_err(|e| e.to_string())?;
    drop(lib);
    Library::write_bytes(&state.data_dir.join(LIBRARY_FILE), &bytes).map_err(|e| e.to_string())?;
    Ok(playlists)
}
