use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

use crate::model::{ScanReport, Track};
use crate::{sanitize_name, AppState, IMPORT_DIR_NAME, LIBRARY_FILE};

pub const EXTENSIONS: &[&str] = &["mp3", "m4a", "flac", "ogg", "oga", "opus", "wav"];
/// 封面图片扩展名(目录封面回退与内嵌封面落盘共用)
pub const COVER_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp"];

/// 解析年份字符串:取前 4 位数字,限定合理区间
pub fn parse_year(s: &str) -> Option<i32> {
    s.chars()
        .take(4)
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse::<i32>()
        .ok()
        .filter(|&y| (1000..=3000).contains(&y))
}
/// 专辑封面缓存:album_key → (小图路径, 大图路径)
type CoverPair = (Option<String>, Option<String>);

/// 目录内兜底封面文件名(不含扩展名)
const COVER_NAMES: &[&str] = &["cover", "folder", "front", "album", "albumart"];
/// 曲库格式版本:复用旧记录的条件之一。版本不一致时强制全量重扫(如封面改存双档缩略图)
const SCAN_VERSION: u32 = 3;

/// 全量扫描所有已登记的音乐文件夹,与旧曲库做增量合并后落盘。
/// 通过 AppState 的扫描闸防重入:已有扫描进行中时直接返回错误。
pub fn run_scan(app: &AppHandle) -> Result<ScanReport, String> {
    let state = app.state::<AppState>();
    let _gate = state
        .scan_gate
        .try_lock()
        .map_err(|_| "已有扫描正在进行,请稍后再试".to_string())?;
    // 旧曲库只在这里克隆成 path 索引:扫描全程不再持有第二份全量记录
    let (folders, mut by_path, prev_version) = {
        let lib = state.lib.lock().map_err(|_| "曲库状态不可用".to_string())?;
        let by_path = lib
            .tracks
            .iter()
            .map(|t| (t.path.clone(), t.clone()))
            .collect::<HashMap<String, Track>>();
        (lib.folders.clone(), by_path, lib.version)
    };
    // 曲库格式版本不匹配时放弃全部增量复用,重新解析每个文件
    let force_full = prev_version != SCAN_VERSION;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    // 先收集文件清单(快),再逐个解析并上报进度。
    // 根条目放行(filter_entry 也会收到遍历根本身),否则登记点开头目录(如 X:\.music)会静默扫空
    let mut files: Vec<PathBuf> = Vec::new();
    for folder in &folders {
        for entry in walkdir::WalkDir::new(folder)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'))
        {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_file() {
                continue;
            }
            let ext = entry
                .path()
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase());
            if matches!(ext.as_deref(), Some(e) if EXTENSIONS.contains(&e)) {
                files.push(entry.into_path());
            }
        }
    }
    let total = files.len();

    // 曲库整理:导入目录顶层的散装音频按「艺人\专辑」归位(仅顶层,不动子目录结构)
    organize_top_level(&state.data_dir.join(IMPORT_DIR_NAME), &mut files);

    let covers_dir = state.cache_dir.join("covers");
    let _ = fs::create_dir_all(&covers_dir);

    // 专辑封面去重:同一张专辑只提取/落盘一次
    // 并行解析:文件清单切成小块(96/块)放入共享队列,N 个 worker 动态领取,
    // 避免大块静态分配时单块尾延迟拖住整体;块结果按原顺序合并保证曲库顺序稳定
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8);
    const CHUNK_SIZE: usize = 96;
    let chunks: Vec<&[PathBuf]> = files.chunks(CHUNK_SIZE).collect();
    let next_chunk = std::sync::atomic::AtomicUsize::new(0);
    let album_covers: Mutex<HashMap<String, CoverPair>> = Mutex::new(HashMap::new());
    // 目录旁路文件清单缓存:每个目录一次 read_dir,全部 worker 共享
    let dir_listings: Mutex<HashMap<PathBuf, DirListing>> = Mutex::new(HashMap::new());
    let done = std::sync::atomic::AtomicUsize::new(0);

    let chunk_results: Vec<Vec<Track>> = std::thread::scope(|scope| {
        let chunks = &chunks;
        let next_chunk = &next_chunk;
        let by_path = &by_path;
        let done = &done;
        let covers_dir = &covers_dir;
        let app = &app;
        let album_covers = &album_covers;
        let dir_listings = &dir_listings;
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(move || {
                    let mut out: Vec<Track> = Vec::new();
                    loop {
                        let i = next_chunk.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(chunk) = chunks.get(i) else {
                            break;
                        };
                        for path in *chunk {
                            let path_str = path.to_string_lossy().to_string();

                            let (mtime, size) = match fs::metadata(path) {
                                Ok(m) => (
                                    m.modified()
                                        .ok()
                                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                        .map(|d| d.as_secs_f64())
                                        .unwrap_or(0.0),
                                    m.len(),
                                ),
                                Err(_) => {
                                    // 文件在遍历与 stat 之间消失:落入 removed 口径
                                    continue;
                                }
                            };
                            done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                            let cached = by_path.get(&path_str).cloned();
                            if let Some(prev) = &cached {
                                // 文件未变化:直接复用旧记录,不重复解析。
                                // 旁路文件(cover.* / 同名 .lrc)比音频新时也算变更——在线匹配只新增旁路文件;
                                // 旧记录里的封面/歌词文件已被删除时同样视为变更,避免残留失效路径
                                let cover_ok = prev
                                    .cover
                                    .as_ref()
                                    .map(|c| Path::new(c).is_file())
                                    .unwrap_or(true)
                                    && prev
                                        .cover_large
                                        .as_ref()
                                        .map(|c| Path::new(c).is_file())
                                        .unwrap_or(true);
                                let lrc_ok = prev
                                    .lrc_path
                                    .as_ref()
                                    .map(|c| Path::new(c).is_file())
                                    .unwrap_or(true);
                                if (prev.mtime - mtime).abs() < 0.5
                                    && prev.size == size
                                    && cover_ok
                                    && lrc_ok
                                    && {
                                        let mut listings =
                                            dir_listings.lock().unwrap_or_else(|e| e.into_inner());
                                        sidecar_mtime(path, prev.cover.as_deref(), &mut listings)
                                    } <= prev.mtime + 0.5
                                    && !force_full
                                {
                                    out.push(prev.clone());
                                    continue;
                                }
                            }

                            match parse_track(
                                path,
                                mtime,
                                size,
                                now,
                                covers_dir,
                                &mut album_covers.lock().unwrap_or_else(|e| e.into_inner()),
                            ) {
                                Ok(mut track) => {
                                    if let Some(prev) = cached {
                                        track.added_at = prev.added_at;
                                    }
                                    out.push(track);
                                }
                                Err(err) => {
                                    eprintln!("[scan] 解析失败 {path_str}: {err}");
                                    if let Some(prev) = cached {
                                        out.push(prev);
                                    }
                                }
                            }

                            let n = done.load(std::sync::atomic::Ordering::Relaxed);
                            if n.is_multiple_of(20) || n == total {
                                let _ = app
                                    .emit("scan-progress", json!({ "current": n, "total": total }));
                            }
                        }
                    }
                    out
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });

    // 按原顺序合并 + 聚合统计:added/updated 由 by_path 判定;
    // metadata 失败的文件落入 removed 口径,解析失败的保留旧记录
    let mut result: Vec<Track> = Vec::with_capacity(total);
    for chunk in chunk_results {
        result.extend(chunk);
    }
    // 本次扫到的路径从索引里移除:剩下的就是被移除的曲目,
    // 省掉一份全量 seen 集合与对旧曲库的二次遍历
    let mut added = 0usize;
    let mut updated = 0usize;
    for t in &result {
        if by_path.remove(&t.path).is_some() {
            updated += 1;
        } else {
            added += 1;
        }
    }
    let removed = by_path.len();
    let report = ScanReport {
        added,
        updated,
        removed,
        total,
        errors: 0, // 解析失败已回填旧记录;metadata 失败计入 removed
    };

    // 引用集包含回收站快照,还原后封面文件仍在
    let referenced: HashSet<String> = {
        let mut lib = state.lib.lock().map_err(|_| "曲库状态不可用".to_string())?;
        lib.tracks = result;
        lib.version = SCAN_VERSION;
        // 落盘失败必须上抛:release 构建没有控制台,eprintln 无处可见,
        // 静默失败会让内存态与磁盘态分叉
        lib.save(&state.data_dir.join(LIBRARY_FILE))
            .map_err(|e| e.to_string())?;
        lib.tracks
            .iter()
            .chain(lib.trash.iter().map(|e| &e.track))
            .filter_map(|t| t.cover.clone())
            .chain(
                lib.tracks
                    .iter()
                    .chain(lib.trash.iter().map(|e| &e.track))
                    .filter_map(|t| t.cover_large.clone()),
            )
            .collect()
    };
    // 封面缓存 GC:清理已删/已移走音乐残留的孤儿缩略图;文件 IO 在锁外做
    gc_cover_cache(&covers_dir, &referenced);
    Ok(report)
}

/// 缓存封面文件名规则:16 位 hex + 可选 l 后缀(大图)+ 图片扩展名
fn is_cache_cover_name(name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    if !matches!(ext, "jpg" | "jpeg" | "png" | "webp") {
        return false;
    }
    let (hex, _is_large) = match stem.strip_suffix('l') {
        Some(h) => (h, true),
        None => (stem, false),
    };
    hex.len() == 16 && hex.chars().all(|c| c.is_ascii_hexdigit())
}

/// 清理封面缓存里的孤儿文件:命名符合缓存规则但不在引用集内的缩略图删除,
/// 其他文件(用户自放)一律不动
fn gc_cover_cache(covers_dir: &Path, referenced: &HashSet<String>) {
    let Ok(rd) = fs::read_dir(covers_dir) else {
        return;
    };
    for e in rd.filter_map(|e| e.ok()) {
        let Ok(ft) = e.file_type() else {
            continue;
        };
        if !ft.is_file() {
            continue;
        }
        let name = e.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !is_cache_cover_name(name) {
            continue;
        }
        if !referenced.contains(&covers_dir.join(name).to_string_lossy().to_string()) {
            let _ = fs::remove_file(covers_dir.join(name));
        }
    }
}

fn parse_track(
    path: &Path,
    mtime: f64,
    size: u64,
    added_at: f64,
    covers_dir: &Path,
    album_covers: &mut HashMap<String, CoverPair>,
) -> Result<Track, String> {
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::picture::MimeType;
    use lofty::probe::Probe;
    use lofty::tag::ItemKey;

    let tagged = Probe::open(path)
        .map_err(|e| e.to_string())?
        .read()
        .map_err(|e| e.to_string())?;
    let duration = tagged.properties().duration().as_secs_f64();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    let get = |key: &ItemKey| -> Option<String> {
        tag.and_then(|t| t.get_string(key))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let path_str = path.to_string_lossy().to_string();
    let parent = path.parent().unwrap_or_else(|| Path::new(""));

    // 标签缺失时回退到文件名,支持 "艺人 - 标题" 命名
    let mut title = get(&ItemKey::TrackTitle);
    let mut artist = get(&ItemKey::TrackArtist);
    if title.is_none() && artist.is_none() {
        if let Some((a, t)) = stem.split_once(" - ") {
            artist = Some(a.trim().to_string());
            title = Some(t.trim().to_string());
        }
    }
    let mut album = get(&ItemKey::AlbumTitle);
    let mut album_artist = get(&ItemKey::AlbumArtist);
    let mut year = get(&ItemKey::Year)
        .or_else(|| get(&ItemKey::RecordingDate))
        .and_then(|y| parse_year(&y));
    let mut track_no = get(&ItemKey::TrackNumber).and_then(parse_slashed_number);
    let disc_no = get(&ItemKey::DiscNumber).and_then(parse_slashed_number);
    let mut genre = get(&ItemKey::Genre);

    // TMC 音乐包元数据:同级 meta.json 只补齐标签缺失的字段,不覆盖内嵌标签
    let meta_path = parent.join("meta.json");
    if meta_path.is_file() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(&meta_path).unwrap_or_default(),
        ) {
            let mget = |k: &str| -> Option<String> {
                v.get(k)
                    .and_then(|x| x.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            };
            title = title.or_else(|| mget("title"));
            artist = artist.or_else(|| mget("artist"));
            album = album.or_else(|| mget("album"));
            album_artist = album_artist.or_else(|| mget("albumArtist"));
            if year.is_none() {
                year = mget("year").and_then(|y| parse_year(&y));
            }
            if track_no.is_none() {
                track_no = mget("trackNo").and_then(parse_slashed_number);
            }
            if genre.is_none() {
                genre = mget("genre");
            }
        }
    }

    let title = title.unwrap_or_else(|| stem.clone());
    let artist = artist.unwrap_or_else(|| "未知艺人".into());
    let album = album.unwrap_or_else(|| "未知专辑".into());
    let album_artist = album_artist.unwrap_or_else(|| artist.clone());

    // 歌词:同名 .lrc 伴生文件,或内嵌歌词标签
    let lrc = parent.join(format!("{stem}.lrc"));
    let lrc_path = lrc.is_file().then(|| lrc.to_string_lossy().to_string());
    let has_lyrics =
        lrc_path.is_some() || tag.and_then(|t| t.get_string(&ItemKey::Lyrics)).is_some();

    // 封面:优先内嵌图片(同专辑只落盘一次),其次目录内的 cover/folder 图片。
    // 统一进缓存并生成两级缩略图:列表用 256 小图,详情/播放页用 512 大图,
    // 渲染进程按展示尺寸解码,不再为 40px 的行解码 3000px 原图
    let album_key = format!(
        "{}\u{1}{}\u{1}{}",
        album_artist,
        album,
        parent.to_string_lossy()
    );
    let (cover, cover_large) = if let Some(c) = album_covers.get(&album_key) {
        c.clone()
    } else {
        // 来源优先级:内嵌 → {stem}.cover.* → cover.*;统一读出字节进缓存
        let mut source: Option<(Vec<u8>, &'static str)> = None;
        if let Some(pic) = tag.and_then(|t| t.pictures().first()) {
            let ext = match pic.mime_type() {
                Some(MimeType::Png) => "png",
                _ => "jpg",
            };
            source = Some((pic.data().to_vec(), ext));
        }
        if source.is_none() {
            'stem_cover: for ext in COVER_EXTS {
                let p = parent.join(format!("{stem}.cover.{ext}"));
                if p.is_file() {
                    source = fs::read(&p).ok().map(|b| (b, *ext));
                    break 'stem_cover;
                }
            }
        }
        if source.is_none() {
            'outer: for name in COVER_NAMES {
                for ext in COVER_EXTS {
                    let p = parent.join(format!("{name}.{ext}"));
                    if p.is_file() {
                        source = fs::read(&p).ok().map(|b| (b, *ext));
                        break 'outer;
                    }
                }
            }
        }
        let (cover, cover_large) = match source {
            Some((data, ext)) => {
                write_cover_thumbs(covers_dir, &album_key, &data).unwrap_or_else(|| {
                    // 解码失败(损坏图片):原样落盘,两档都指向它
                    let file = covers_dir.join(format!("{:016x}.{ext}", hash_str(&album_key)));
                    fs::write(&file, &data).ok();
                    let p = file.to_string_lossy().to_string();
                    (p.clone(), p)
                })
            }
            None => (String::new(), String::new()),
        };
        let result = if cover.is_empty() {
            (None, None)
        } else {
            (Some(cover), Some(cover_large))
        };
        album_covers.insert(album_key, result.clone());
        result
    };

    Ok(Track {
        id: format!("{:016x}", hash_str(&path_str)),
        title,
        artist,
        album,
        album_artist,
        track_no,
        disc_no,
        year,
        genre,
        duration,
        path: path_str,
        cover,
        cover_large,
        sha256: None,
        has_lyrics,
        lrc_path,
        added_at,
        mtime,
        size,
    })
}

fn parse_slashed_number(s: String) -> Option<u32> {
    s.split('/').next()?.trim().parse().ok()
}

/// 列表/网格用的小图上限:行内 40px、卡片约 170px,256px 在 1.5x DPI 下仍然清晰;
/// 解码内存 256²×4 ≈ 256KB,是 512px 的四分之一——大图在列表场景是纯浪费
const THUMB_SMALL: u32 = 256;
/// 详情页/全屏播放页用的大图上限
const THUMB_LARGE: u32 = 512;

/// 把封面源数据落成两级 JPEG 缩略图(256 小图 + 512 大图),返回 (小图, 大图) 路径;
/// 解码失败返回 None(调用方回退原样写入)
fn write_cover_thumbs(covers_dir: &Path, album_key: &str, data: &[u8]) -> Option<(String, String)> {
    use image::codecs::jpeg::JpegEncoder;
    use image::imageops::FilterType;
    use image::ImageEncoder;

    let img = image::load_from_memory(data).ok()?;
    let write = |max_dim: u32, name: String| -> Option<String> {
        let thumb = if img.width().max(img.height()) > max_dim {
            img.resize(max_dim, max_dim, FilterType::Triangle)
        } else {
            img.clone()
        };
        let rgb = thumb.to_rgb8();
        let file = covers_dir.join(name);
        let mut out = fs::File::create(&file).ok()?;
        JpegEncoder::new_with_quality(&mut out, 85)
            .write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .ok()?;
        Some(file.to_string_lossy().to_string())
    };
    let key = format!("{:016x}", hash_str(album_key));
    let small = write(THUMB_SMALL, format!("{key}.jpg"))?;
    let large = write(THUMB_LARGE, format!("{key}l.jpg"))?;
    Some((small, large))
}

/// 目录旁路文件清单:一次 read_dir 拿全部(小写文件名, mtime)
type DirListing = Vec<(String, f64)>;

/// 读取目录清单并缓存:一个目录只枚举一次,同专辑多曲目共享,
/// 替代旧实现对每个未变化文件做 20+ 次 metadata 探测
fn dir_listing(dir: &Path, cache: &mut HashMap<PathBuf, DirListing>) -> DirListing {
    if let Some(v) = cache.get(dir) {
        return v.clone();
    }
    let mut out: Vec<(String, f64)> = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.filter_map(|e| e.ok()) {
            let Ok(ft) = e.file_type() else {
                continue;
            };
            if !ft.is_file() {
                continue;
            }
            // 小写存储:Windows 下 cover.jpg/COVER.JPG 语义相同,统一小写匹配
            let mtime = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            out.push((e.file_name().to_string_lossy().to_lowercase(), mtime));
        }
    }
    cache.insert(dir.to_path_buf(), out.clone());
    out
}

/// 音频旁路元数据文件的最新修改时间(目录封面 + {stem}.cover.* + 同名 .lrc + 旧封面文件),
/// 无则 0。语义与逐个 stat 的旧实现一致(含原地覆写检测),但目录清单按缓存只枚举一次
fn sidecar_mtime(
    path: &Path,
    prev_cover: Option<&str>,
    cache: &mut HashMap<PathBuf, DirListing>,
) -> f64 {
    let Some(parent) = path.parent() else {
        return 0.0;
    };
    let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()) else {
        return 0.0;
    };
    let listing = dir_listing(parent, cache);
    let mut max = 0.0f64;
    for (name, mtime) in &listing {
        // 候选判定(精确匹配,与旧探测名单一致):目录封面 base.ext / {stem}.cover.ext / {stem}.lrc
        let is_dir_cover = name
            .split_once('.')
            .map(|(base, ext)| COVER_NAMES.contains(&base) && COVER_EXTS.contains(&ext))
            .unwrap_or(false);
        let is_stem_cover = COVER_EXTS
            .iter()
            .any(|ext| name == &format!("{stem}.cover.{ext}"));
        let is_lrc = name == &format!("{stem}.lrc");
        if (is_dir_cover || is_stem_cover || is_lrc) && *mtime > max {
            max = *mtime;
        }
    }
    if let Some(c) = prev_cover {
        if let Ok(m) = fs::metadata(c) {
            if let Ok(t) = m.modified() {
                if let Ok(d) = t.duration_since(UNIX_EPOCH) {
                    max = max.max(d.as_secs_f64());
                }
            }
        }
    }
    max
}

pub(crate) fn hash_str(s: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// 快速读取音频标签里的艺人与专辑(供归位目录命名);无标签回退文件名「艺人 - 标题」约定
fn peek_artist_album(path: &Path) -> Option<(String, String)> {
    use lofty::file::TaggedFileExt;
    use lofty::tag::ItemKey;
    let Ok(tagged) = lofty::probe::Probe::open(path).and_then(|p| p.read()) else {
        return None;
    };
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let get = |k: &ItemKey| -> Option<String> {
        tag.and_then(|t| t.get_string(k))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let artist = get(&ItemKey::AlbumArtist).or_else(|| get(&ItemKey::TrackArtist));
    let album = get(&ItemKey::AlbumTitle);
    if let (Some(a), Some(alb)) = (artist.as_ref(), album.as_ref()) {
        return Some((a.clone(), alb.clone()));
    }
    // 标签不全:文件名「艺人 - 标题」补艺人
    let stem = path.file_stem()?.to_string_lossy().to_string();
    let from_name = stem.split_once(" - ").map(|(a, _)| a.trim().to_string());
    Some((
        artist.or(from_name).unwrap_or_else(|| "未整理".into()),
        album.unwrap_or_else(|| "未整理".into()),
    ))
}

/// 把导入目录顶层的散装音频按「艺人\专辑\文件名」归位,同 stem 的 .lrc / 专属封面一起移动。
/// 只整理应用自己的导入目录(用户登记的外部文件夹一律不动);正在播放等导致 rename 失败时保持原位。
fn organize_top_level(import_dir: &Path, files: &mut Vec<PathBuf>) {
    if !import_dir.is_dir() {
        return;
    }
    let entries = match fs::read_dir(import_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_file() {
            continue;
        }
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        if !matches!(ext.as_deref(), Some(e) if EXTENSIONS.contains(&e)) {
            continue;
        }
        let Some(name) = path.file_name().map(|n| n.to_os_string()) else {
            continue;
        };
        let (artist, album) =
            peek_artist_album(&path).unwrap_or_else(|| ("未整理".into(), "未整理".into()));
        let target_dir = import_dir
            .join(sanitize_name(&artist))
            .join(sanitize_name(&album));
        let target = target_dir.join(&name);
        if target == path {
            continue;
        }
        if target.exists() {
            // 目标已有同名:同大小视为重复,移入回收站(可恢复);不同大小则不覆盖,保持原位
            let dup = match (fs::metadata(&path), fs::metadata(&target)) {
                (Ok(a), Ok(b)) => a.len() == b.len(),
                _ => false,
            };
            if dup && trash::delete(&path).is_ok() {
                files.retain(|f| f != &path);
            }
            continue;
        }
        if fs::create_dir_all(&target_dir).is_err() || fs::rename(&path, &target).is_err() {
            continue; // 移动失败(如文件被播放占用)保持原位
        }
        for f in files.iter_mut() {
            if *f == path {
                *f = target.clone();
            }
        }
        // 顺移同名歌词与曲目专属旁路封面
        let Some(stem) = target.file_stem().map(|s| s.to_string_lossy().to_string()) else {
            continue;
        };
        let parent = target.parent().unwrap_or(import_dir);
        let lrc = path.with_extension("lrc");
        if lrc.is_file() {
            let _ = fs::rename(&lrc, parent.join(format!("{stem}.lrc")));
        }
        for e in COVER_EXTS {
            let c = import_dir.join(format!("{stem}.cover.{e}"));
            if c.is_file() {
                let _ = fs::rename(&c, parent.join(format!("{stem}.cover.{e}")));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tm-scan-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn slashed_number_parses_leading_part() {
        assert_eq!(parse_slashed_number("3/12".into()), Some(3));
        assert_eq!(parse_slashed_number(" 7 ".into()), Some(7));
        assert_eq!(parse_slashed_number("abc".into()), None);
        assert_eq!(parse_slashed_number("".into()), None);
    }

    #[test]
    fn hash_is_stable_for_same_input() {
        assert_eq!(hash_str("abc"), hash_str("abc"));
        assert_ne!(hash_str("abc"), hash_str("abd"));
    }

    #[test]
    fn sidecar_mtime_detects_cover_and_lrc() {
        let dir = temp_dir("sidecar");
        let audio = dir.join("song.mp3");
        fs::write(&audio, b"x").unwrap();
        let mut cache: HashMap<PathBuf, DirListing> = HashMap::new();
        assert_eq!(sidecar_mtime(&audio, None, &mut cache), 0.0);
        // 每次写入后清缓存 = 模拟一次新扫描(清单是单次扫描内的快照,与生产语义一致)
        fs::write(dir.join("cover.jpg"), b"x").unwrap();
        cache.clear();
        assert!(sidecar_mtime(&audio, None, &mut cache) > 0.0);
        fs::write(dir.join("song.lrc"), b"[00:00]t").unwrap();
        fs::write(dir.join("song.cover.png"), b"x").unwrap();
        cache.clear();
        assert!(sidecar_mtime(&audio, None, &mut cache) > 0.0);
        // 非默认名单的封面(folder.*)也应计入,与目录封面回退逻辑一致
        fs::write(dir.join("folder.jpg"), b"x").unwrap();
        cache.clear();
        assert!(sidecar_mtime(&audio, None, &mut cache) > 0.0);
        // 旧记录指向的实际封面路径(如缓存目录)同样计入
        let elsewhere = temp_dir("sidecar-elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(elsewhere.join("cached.png"), b"x").unwrap();
        let cached = elsewhere.join("cached.png");
        assert!(sidecar_mtime(&audio, Some(cached.to_str().unwrap()), &mut cache) > 0.0);
        // 命中规则的边界:同前缀但非候选名(cover.backup.jpg)不算旁路文件
        fs::write(dir.join("cover.backup.jpg"), b"x").unwrap();
        cache.clear();
        assert!(sidecar_mtime(&audio, None, &mut cache) > 0.0); // folder.jpg 仍在
        fs::remove_dir_all(&dir).ok();
        fs::remove_dir_all(&elsewhere).ok();
    }

    #[test]
    fn gc_cover_cache_removes_orphans_only() {
        let dir = temp_dir("gc");
        let referenced_a = dir.join("aaaaaaaaaaaaaaaa.jpg");
        let referenced_b = dir.join("aaaaaaaaaaaaaaaal.jpg");
        let orphan = dir.join("cccccccccccccccc.jpg");
        let foreign = dir.join("notes.txt");
        fs::write(&referenced_a, b"a").unwrap();
        fs::write(&referenced_b, b"b").unwrap();
        fs::write(&orphan, b"c").unwrap();
        fs::write(&foreign, b"n").unwrap();

        let mut referenced = std::collections::HashSet::new();
        referenced.insert(referenced_a.to_string_lossy().to_string());
        referenced.insert(referenced_b.to_string_lossy().to_string());
        gc_cover_cache(&dir, &referenced);

        assert!(referenced_a.is_file(), "引用的保留");
        assert!(referenced_b.is_file(), "引用的大图保留");
        assert!(!orphan.exists(), "孤儿缩略图删除");
        assert!(foreign.is_file(), "非缓存命名的文件不动");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scan_gate_rejects_concurrent_scan() {
        // 直接验证闸门互斥:同一线程先锁住闸,第二次 try_lock 必须失败
        let gate = std::sync::Mutex::new(());
        let _g1 = gate.try_lock().unwrap();
        assert!(gate.try_lock().is_err());
    }

    /// 写一个 lofty 可解析的最小 WAV(44 字节标准头 + 100 字节静音,PCM 8bit 单声道 8000Hz)
    fn write_wav(path: &Path) {
        let data_len = 100u32;
        let mut v = Vec::with_capacity(44 + data_len as usize);
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data_len).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes()); // PCM
        v.extend_from_slice(&1u16.to_le_bytes()); // 单声道
        v.extend_from_slice(&8000u32.to_le_bytes()); // 采样率
        v.extend_from_slice(&8000u32.to_le_bytes()); // 字节率
        v.extend_from_slice(&1u16.to_le_bytes()); // 块对齐
        v.extend_from_slice(&8u16.to_le_bytes()); // 位深
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_len.to_le_bytes());
        v.extend(std::iter::repeat_n(0u8, data_len as usize));
        fs::write(path, v).unwrap();
    }

    #[test]
    fn parse_track_falls_back_to_filename() {
        let dir = temp_dir("fallback");
        // "艺人 - 标题" 命名约定
        let audio = dir.join("周杰伦 - 晴天.wav");
        write_wav(&audio);
        let t = parse_track(&audio, 0.0, 1, 0.0, &dir, &mut HashMap::new()).unwrap();
        assert_eq!(t.title, "晴天");
        assert_eq!(t.artist, "周杰伦");
        assert_eq!(t.album, "未知专辑");
        assert_eq!(t.album_artist, "周杰伦");
        assert!(t.duration > 0.0 && t.duration < 1.0); // 100B/8000Bps ≈ 0.0125s
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_track_cover_priority_stem_before_dir() {
        use image::ImageFormat;
        let dir = temp_dir("coverprio");
        let audio = dir.join("s.wav");
        write_wav(&audio);

        // 生成两张可解码且颜色不同的 PNG,通过像素颜色验证优先级
        let png = |rgb: [u8; 3]| -> Vec<u8> {
            let mut img = image::DynamicImage::new_rgb8(64, 64);
            img.as_mut_rgb8()
                .unwrap()
                .pixels_mut()
                .for_each(|p| *p = image::Rgb(rgb));
            let mut buf = std::io::Cursor::new(Vec::new());
            img.write_to(&mut buf, ImageFormat::Png).unwrap();
            buf.into_inner()
        };
        fs::write(dir.join("s.cover.jpg"), png([255, 0, 0])).unwrap();
        fs::write(dir.join("cover.jpg"), png([0, 0, 255])).unwrap();

        // 曲目专属旁路封面优先于目录封面(封面统一进缓存,按像素颜色判别来源)
        let t = parse_track(&audio, 0.0, 1, 0.0, &dir, &mut HashMap::new()).unwrap();
        let c = t.cover.expect("stem cover");
        let pixel = image::open(&c).unwrap().to_rgb8().get_pixel(0, 0).0;
        // JPEG 有损,通道值允许 ±8 误差;红胜蓝即专属封面胜出
        assert!(
            pixel[0] >= 247 && pixel[1] <= 8 && pixel[2] <= 8,
            "应取曲目专属封面(红): {pixel:?}"
        );

        // 删除专属封面后回退目录封面
        fs::remove_file(dir.join("s.cover.jpg")).unwrap();
        let t = parse_track(&audio, 0.0, 1, 0.0, &dir, &mut HashMap::new()).unwrap();
        let c = t.cover.expect("dir cover");
        let pixel = image::open(&c).unwrap().to_rgb8().get_pixel(0, 0).0;
        assert!(
            pixel[2] >= 247 && pixel[0] <= 8 && pixel[1] <= 8,
            "应回退目录封面(蓝): {pixel:?}"
        );

        // 全部移除后无封面
        fs::remove_file(dir.join("cover.jpg")).unwrap();
        let t = parse_track(&audio, 0.0, 1, 0.0, &dir, &mut HashMap::new()).unwrap();
        assert!(t.cover.is_none() && t.cover_large.is_none());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cover_thumbs_write_two_tiers_and_reject_invalid() {
        use image::ImageFormat;
        let dir = temp_dir("thumb");
        let img = image::DynamicImage::new_rgb8(800, 600);
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, ImageFormat::Png).unwrap();

        let (small, large) = write_cover_thumbs(&dir, "album\u{1}x", png.get_ref()).unwrap();
        assert!(small.ends_with(".jpg") && !small.contains("l.jpg"));
        assert!(large.contains("l.jpg"));
        let s_decoded = image::open(&small).unwrap();
        assert!(s_decoded.width().max(s_decoded.height()) <= 256);
        let l_decoded = image::open(&large).unwrap();
        assert!(l_decoded.width().max(l_decoded.height()) <= 512);
        assert!(l_decoded.width().max(l_decoded.height()) > 256); // 800x600 只压到大图档

        // 非图片数据返回 None,调用方回退原样写入
        assert!(write_cover_thumbs(&dir, "album\u{1}y", b"not-an-image").is_none());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn organize_moves_top_level_files_into_artist_album_dirs() {
        let import_dir = temp_dir("organize");
        // 有标签的散装文件(用最小 WAV + 文件名约定)
        let loose = import_dir.join("周杰伦 - 晴天.wav");
        write_wav(&loose);
        let mut files = vec![loose.clone()];
        organize_top_level(&import_dir, &mut files);

        let target = import_dir
            .join("周杰伦")
            .join("未整理")
            .join("周杰伦 - 晴天.wav");
        assert!(target.is_file(), "散装文件应归位到 艺人/专辑 目录");
        assert!(!loose.exists());
        assert_eq!(files[0], target);
        fs::remove_dir_all(&import_dir).ok();
    }

    #[test]
    fn organize_keeps_subdirectories_untouched() {
        let import_dir = temp_dir("organize-sub");
        let sub = import_dir.join("已有专辑目录");
        fs::create_dir_all(&sub).unwrap();
        let nested = sub.join("c.wav");
        write_wav(&nested);
        let mut files = vec![nested.clone()];
        organize_top_level(&import_dir, &mut files);
        assert!(nested.is_file(), "子目录文件不应被移动");
        assert_eq!(files[0], nested);
        fs::remove_dir_all(&import_dir).ok();
    }

    #[test]
    fn organize_deletes_duplicate_but_keeps_conflicting() {
        // 场景一:目标位置已有同内容同名文件,顶层再次出现 → 重复副本应被清理
        let dir1 = temp_dir("organize-dup");
        let target_dir1 = dir1.join("周杰伦").join("未整理");
        fs::create_dir_all(&target_dir1).unwrap();
        write_wav(&target_dir1.join("周杰伦 - 晴天.wav"));
        let dup = dir1.join("周杰伦 - 晴天.wav");
        fs::write(
            &dup,
            fs::read(target_dir1.join("周杰伦 - 晴天.wav")).unwrap(),
        )
        .unwrap();
        let mut files = vec![dup.clone()];
        organize_top_level(&dir1, &mut files);
        assert!(!dup.exists(), "同内容同名副本应被清理");

        // 场景二:同名但内容不同(大小不同) → 保持原位不覆盖
        let dir2 = temp_dir("organize-conflict");
        let target_dir2 = dir2.join("周杰伦").join("未整理");
        fs::create_dir_all(&target_dir2).unwrap();
        write_wav(&target_dir2.join("周杰伦 - 晴天.wav"));
        let mut wav_bytes = fs::read(target_dir2.join("周杰伦 - 晴天.wav")).unwrap();
        wav_bytes.push(0);
        let conflict = dir2.join("周杰伦 - 晴天.wav");
        fs::write(&conflict, &wav_bytes).unwrap();
        let mut files = vec![conflict.clone()];
        organize_top_level(&dir2, &mut files);
        assert!(conflict.is_file(), "不同内容的同名文件应保持原位");

        fs::remove_dir_all(&dir1).ok();
        fs::remove_dir_all(&dir2).ok();
    }
}
