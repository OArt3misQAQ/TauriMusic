//! 应用回收站:删除移入、还原、彻底清除与超期自动清理。

use crate::model::{self, Library, TrashEntry};
use crate::pack::{dedup_dest, sanitize_name};
use crate::playlists::now_secs_f64;
use crate::scanner;
use crate::{lock_scan_gate, AppState, LIBRARY_FILE};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{Emitter, Manager};

/// 应用回收站目录(data_dir/Trash):删除的音乐移到这里,保留 TRASH_RETENTION_DAYS 天
const TRASH_DIR_NAME: &str = "Trash";
/// 回收站保留天数:超期在启动时自动清理
const TRASH_RETENTION_DAYS: f64 = 30.0;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteReport {
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

pub(crate) fn delete_tracks_blocking(
    ids: &[String],
    app: &tauri::AppHandle,
) -> Result<DeleteReport, String> {
    let state = app.state::<AppState>();
    let import_dir = state.data_dir.join("Music");
    let trash_dir = state.data_dir.join(TRASH_DIR_NAME);

    // 1) 锁内:取出匹配曲目并先从曲库移除(避免长时间持锁做文件搬运)。
    //    先拿扫描闸:扫描期间曲目被借走,并发删除会把空曲库落盘、
    //    或在扫描整体替换 tracks 时被"复活"
    let to_delete: Vec<model::Track> = {
        let _gate = lock_scan_gate(&state);
        let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        let id_set: std::collections::HashSet<&String> = ids.iter().collect();
        let (del, kept): (Vec<model::Track>, Vec<model::Track>) =
            lib.tracks.drain(..).partition(|t| id_set.contains(&t.id));
        lib.tracks = kept;
        // 锁内只序列化,落盘在锁外;失败重拿锁回滚,否则界面少了歌而磁盘没少,
        // 重启后又"复活"
        let bytes = lib.to_bytes().map_err(|e| e.to_string())?;
        drop(lib);
        if let Err(e) = Library::write_bytes(&state.data_dir.join(LIBRARY_FILE), &bytes) {
            if let Ok(mut lib) = state.lib.lock() {
                lib.tracks.extend(del);
            }
            return Err(e.to_string());
        }
        del
    };

    // 2) 锁外:移入应用回收站(音频 + 同名歌词)
    let now = now_secs_f64();
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

    // 3) 锁内:回收站条目落库;失败的回填曲库,避免幽灵状态。
    //    整段本就在同一个闸的保护下(步骤 1 已拿),这里只需短锁序列化
    {
        let _gate = lock_scan_gate(&state);
        let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        lib.trash.extend(entries);
        lib.tracks.extend(failed_tracks.iter().cloned());
        let bytes = lib.to_bytes().map_err(|e| e.to_string())?;
        drop(lib);
        Library::write_bytes(&state.data_dir.join(LIBRARY_FILE), &bytes)
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
pub(crate) struct RestoreReport {
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

pub(crate) fn restore_tracks_blocking(
    ids: &[String],
    app: &tauri::AppHandle,
) -> Result<RestoreReport, String> {
    let state = app.state::<AppState>();
    // 先拿扫描闸:扫描期间曲目被借走,并发还原会把还原件推进空曲库再落盘,
    // 或在扫描整体替换 tracks 时被"复活"
    let _gate = lock_scan_gate(&state);
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
        // 锁内只序列化,磁盘 IO 在锁外(失败时内存保持已变更,与原实现一致)
        let bytes = lib.to_bytes().map_err(|e| e.to_string())?;
        drop(lib);
        Library::write_bytes(&state.data_dir.join(LIBRARY_FILE), &bytes)
            .map_err(|e| e.to_string())?;
    }
    let _ = app.emit("library-changed", ());
    Ok(RestoreReport {
        restored: restored.len() as u32,
        failed: failed_count,
    })
}

pub(crate) fn purge_trash_blocking(ids: &[String], app: &tauri::AppHandle) -> Result<u32, String> {
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
        // 落盘同样要互斥扫描:此刻 lib.tracks 可能被扫描借走,直接序列化会把
        // 空曲库写进磁盘
        let bytes = {
            let _gate = lock_scan_gate(&state);
            let lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
            lib.to_bytes().map_err(|e| e.to_string())?
        };
        Library::write_bytes(&state.data_dir.join(LIBRARY_FILE), &bytes)
            .map_err(|e| e.to_string())?;
    }
    Ok(purged.len() as u32)
}

/// 清理超过保留期的回收站条目(启动时调用)
pub(crate) fn purge_expired_trash(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let cutoff = now_secs_f64() - TRASH_RETENTION_DAYS * 86400.0;
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
        // 启动清理是非关键路径,失败仅忽略(与原实现一致);
        // 序列化在锁内、落盘在锁外
        if let Ok(bytes) = lib.to_bytes() {
            drop(lib);
            let _ = Library::write_bytes(&state.data_dir.join(LIBRARY_FILE), &bytes);
        }
    }
    eprintln!("[trash] 清理超期回收站条目 {} 条", expired.len());
}
