//! 把目录里的音频文件批量打包成 .tmc 音乐包,元数据从网易云自动补齐。
//!
//! ```bash
//! cd src-tauri && cargo run --example make_beyond_tmc -- [输入目录] [输出目录] [文件名过滤词]
//! ```
//!
//! 默认:输入 = 桌面,输出 = ../beyond,过滤词 = beyond(文件名包含即命中,传空串处理全部)。
//!
//! 音频命名约定:`{标题}-{艺人}-{网易云歌曲ID}-{音质}.<ext>`;解析不出 ID 时退化为按
//! 「标题 + 艺人」搜索匹配。每个 .tmc(标准 7z)内含:
//! - `{标题}.{ext}` 音频(按标题重命名;打包副本会做标签修正,源文件不动)
//! - `{标题}.lrc` 网易云歌词(找不到则省略,应用可在线补)
//! - `cover.jpg` 网易云专辑封面(找不到则省略)
//! - `meta.json` 元数据(仅补齐音频标签缺失的字段)
//!
//! 标签修正规则(只改打包副本):标题去掉首尾杂字符;内嵌专辑名搜不到「同艺人专辑」时
//! 视为下载器写入的脏数据,改用网易云匹配歌曲的所属专辑;年份缺失时以专辑发行年补写。

use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::{ItemKey, ItemValue, TagItem};
use serde_json::{json, Value};
use sevenz_rust2::{ArchiveEntry, ArchiveWriter};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const EXTENSIONS: &[&str] = &["mp3", "m4a", "flac", "ogg", "oga", "opus", "wav"];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let input_dir = args.get(1).map(PathBuf::from).unwrap_or_else(desktop_dir);
    let out_dir = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../beyond"));
    let filter = args
        .get(3)
        .cloned()
        .unwrap_or_else(|| "beyond".into())
        .to_lowercase();
    fs::create_dir_all(&out_dir).expect("创建输出目录失败");

    let files: Vec<PathBuf> = fs::read_dir(&input_dir)
        .expect("读取输入目录失败")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .map(|x| EXTENSIONS.contains(&x.to_string_lossy().to_lowercase().as_str()))
                    .unwrap_or(false)
                && p.file_name()
                    .map(|n| n.to_string_lossy().to_lowercase().contains(&filter))
                    .unwrap_or(false)
        })
        .collect();
    let mut files = files;
    files.sort();
    if files.is_empty() {
        eprintln!("输入目录 {input_dir:?} 内没有文件名含 '{filter}' 的音频文件");
        return;
    }

    let client = client();
    let mut packed = 0usize;
    for path in &files {
        match build_tmc(path, &out_dir, &client) {
            Ok(dest) => {
                packed += 1;
                println!("  {}", dest.display());
            }
            Err(e) => eprintln!("  跳过 {path:?}: {e}"),
        }
    }
    println!(
        "完成:成功打包 {packed}/{} 个 TMC -> {}",
        files.len(),
        out_dir.display()
    );
}

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
        )
        .timeout(Duration::from_secs(20))
        .build()
        .expect("网络客户端初始化失败")
}

#[cfg(windows)]
fn desktop_dir() -> PathBuf {
    dirs_default(std::env::var("USERPROFILE").ok().as_deref())
}

#[cfg(not(windows))]
fn desktop_dir() -> PathBuf {
    dirs_default(std::env::var("HOME").ok().as_deref())
}

fn dirs_default(home: Option<&str>) -> PathBuf {
    home.map(|h| Path::new(h).join("Desktop"))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 把一个音频文件打包成 .tmc
fn build_tmc(
    path: &Path,
    out_dir: &Path,
    client: &reqwest::blocking::Client,
) -> Result<PathBuf, String> {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .ok_or("无文件名")?;
    let ext = path
        .extension()
        .map(|x| x.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let (name_title, name_artist, song_id) = parse_name(&stem);

    // 内嵌标签优先,文件名兜底
    let tagged = Probe::open(path)
        .map_err(|e| e.to_string())?
        .read()
        .map_err(|e| e.to_string())?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let tget = |k: &ItemKey| {
        tag.and_then(|t| t.get_string(k))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let title = tget(&ItemKey::TrackTitle)
        .unwrap_or(name_title)
        .trim_matches(|c: char| matches!(c, '\'' | '"' | '\u{2018}' | '\u{2019}'))
        .to_string();
    let artist = tget(&ItemKey::TrackArtist)
        .or_else(|| tget(&ItemKey::AlbumArtist))
        .or_else(|| (!name_artist.is_empty()).then_some(name_artist))
        .filter(|s| !s.is_empty())
        .ok_or("无法确定艺人")?;
    let tag_album = tget(&ItemKey::AlbumTitle);

    println!("{title} - {artist}:");

    // 1) 歌曲搜索:歌词兜底与专辑信息共用同一次命中
    let song = song_search(client, &title, &artist);
    let lrc = song_id.and_then(|id| lyric_by_id(client, id)).or_else(|| {
        song.as_ref()
            .and_then(|s| s["id"].as_i64())
            .and_then(|id| lyric_by_id(client, id as u64))
    });
    match (&song, &lrc) {
        (Some(s), Some(l)) => println!(
            "  歌词: {} 字 (网易云: {} id {})",
            l.chars().count(),
            s["name"].as_str().unwrap_or("?"),
            s["id"]
        ),
        (_, Some(l)) => println!("  歌词: {} 字", l.chars().count()),
        (_, None) => println!("  歌词: 未找到(包内将省略)"),
    }

    // 2) 专辑 + 封面 + 年份
    let song_album = song.as_ref().and_then(album_of);
    let song_pic = song.as_ref().and_then(song_cover);
    let (album, cover_url, year) = match &tag_album {
        Some(a) => {
            let (pic, y, trusted) = album_search(client, a, &artist);
            if trusted {
                println!("  专辑: {a}(标签可信,年份 {y:?})");
                (Some(a.clone()), pic, y)
            } else {
                println!("  专辑: {a}(标签不可信) -> 改用网易云匹配歌曲的专辑");
                let (a2, pic2, y2) = via_song_album(client, &song_album, &artist);
                (a2.or_else(|| Some(a.clone())), pic2.or(song_pic), y2)
            }
        }
        None => {
            println!("  专辑: 标签缺失 -> 用网易云匹配歌曲的专辑");
            let (a2, pic2, y2) = via_song_album(client, &song_album, &artist);
            (a2, pic2.or(song_pic), y2)
        }
    };
    match &cover_url {
        Some(_) => println!("  封面: 已取得,年份 {year:?}"),
        None => println!("  封面: 未找到(包内将省略)"),
    }

    // 3) 组包工作目录
    let work = out_dir.join(format!("_work_{title}"));
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let cleanup = || {
        let _ = fs::remove_dir_all(&work);
    };

    let audio_name = format!("{title}.{ext}");
    let work_audio = work.join(&audio_name);
    if let Err(e) = fs::copy(path, &work_audio) {
        cleanup();
        return Err(format!("复制音频失败: {e}"));
    }
    if let Err(e) = fix_tags(&work_audio, &title, album.as_deref(), year, &artist) {
        cleanup();
        return Err(format!("修正标签失败: {e}"));
    }
    if let Some(l) = &lrc {
        if let Err(e) = fs::write(work.join(format!("{title}.lrc")), l) {
            cleanup();
            return Err(format!("写歌词失败: {e}"));
        }
    }
    if let Some(url) = &cover_url {
        let cover_name = if url.split('?').next().unwrap_or("").ends_with(".png") {
            "cover.png"
        } else {
            "cover.jpg"
        };
        match download(client, url, &work.join(cover_name)) {
            Ok(()) => println!("  封面: 已下载 {cover_name}"),
            Err(e) => println!("  封面: 下载失败({e}),包内省略"),
        }
    }

    // meta.json 只补音频标签缺失的字段(扫描器以内嵌标签优先)
    let mut meta = serde_json::Map::new();
    meta.insert("title".into(), json!(title.clone()));
    meta.insert("artist".into(), json!(artist.clone()));
    if let Some(a) = &album {
        meta.insert("album".into(), json!(a));
    }
    meta.insert("albumArtist".into(), json!(artist.clone()));
    if let Some(y) = year {
        meta.insert("year".into(), json!(y.to_string()));
    }
    if let Err(e) = fs::write(
        work.join("meta.json"),
        serde_json::to_vec_pretty(&Value::Object(meta)).map_err(|e| e.to_string())?,
    ) {
        cleanup();
        return Err(format!("写 meta 失败: {e}"));
    }

    // 4) 7z 压缩成 .tmc
    let dest = out_dir.join(format!("{title} - {artist}.tmc"));
    if let Err(e) = pack(&dest, &work) {
        cleanup();
        return Err(e);
    }
    cleanup();
    Ok(dest)
}

/// 标签专辑不可信/缺失时,用搜索命中歌曲的所属专辑名再搜一次(要求艺人匹配)
fn via_song_album(
    client: &reqwest::blocking::Client,
    song_album: &Option<String>,
    artist: &str,
) -> (Option<String>, Option<String>, Option<i32>) {
    match song_album {
        Some(name) => {
            let (pic, year, trusted) = album_search(client, name, artist);
            if trusted {
                (Some(name.clone()), pic, year)
            } else {
                (None, None, None)
            }
        }
        None => (None, None, None),
    }
}

/// 解析 `{标题}-{艺人}-{歌曲ID}-{音质}` 命名,返回 (标题, 艺人, 歌曲ID)
fn parse_name(stem: &str) -> (String, String, Option<u64>) {
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() >= 4 {
        let quality = parts[parts.len() - 1];
        let id = parts[parts.len() - 2];
        let is_num = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
        if is_num(id) && is_num(quality) && quality.len() <= 4 {
            return (
                parts[..parts.len() - 3].join("-"),
                parts[parts.len() - 3].trim().to_string(),
                id.parse().ok(),
            );
        }
    }
    (stem.to_string(), String::new(), None)
}

fn netease_search(
    client: &reqwest::blocking::Client,
    keyword: &str,
    search_type: u32,
    limit: u32,
) -> Option<Value> {
    let resp = client
        .post("https://music.163.com/api/search/get/web")
        .header("Referer", "https://music.163.com")
        .header("Cookie", "os=pc; appver=2.9.7")
        .form(&[
            ("s", keyword.to_string()),
            ("type", search_type.to_string()),
            ("limit", limit.to_string()),
        ])
        .send()
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<Value>().ok()
}

/// 歌曲搜索:关键词含艺人词,结果里艺人名大小写不敏感匹配,返回命中的歌曲
fn song_search(client: &reqwest::blocking::Client, title: &str, artist: &str) -> Option<Value> {
    // 「真的爱妳」这类异体字在网易云检索不到,先做常见替换
    let titles: Vec<String> = if title.contains('妳') {
        vec![title.replace('妳', "你"), title.to_string()]
    } else {
        vec![title.to_string()]
    };
    for t in &titles {
        for kw in [format!("{t} {artist}"), t.clone()] {
            let Some(v) = netease_search(client, &kw, 1, 10) else {
                continue;
            };
            let Some(songs) = v.pointer("/result/songs").and_then(|x| x.as_array()) else {
                continue;
            };
            if let Some(song) = songs
                .iter()
                .find(|s| artist_matches(s, artist))
                .or_else(|| songs.first())
            {
                return Some(song.clone());
            }
        }
    }
    None
}

fn artist_matches(song: &Value, artist: &str) -> bool {
    let segs: Vec<String> = artist
        .split(['/', '&', ',', '、'])
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if segs.is_empty() {
        return false;
    }
    song["artists"]
        .as_array()
        .map(|arr| {
            arr.iter().any(|a| {
                a["name"]
                    .as_str()
                    .map(|n| {
                        let n = n.to_lowercase();
                        segs.iter()
                            .any(|seg| n.contains(seg.as_str()) || seg.contains(&n))
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn lyric_by_id(client: &reqwest::blocking::Client, id: u64) -> Option<String> {
    let url = format!("https://music.163.com/api/song/lyric?id={id}&lv=1&kv=1&tv=-1");
    let resp = client
        .get(&url)
        .header("Referer", "https://music.163.com")
        .header("Cookie", "os=pc; appver=2.9.7")
        .send()
        .ok()?;
    let v = resp.json::<Value>().ok()?;
    let text = v.pointer("/lrc/lyric")?.as_str()?.trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// 专辑搜索(type=10):返回 (封面链接, 发行年份, 是否为同艺人专辑)。
/// 只有艺人名能对上才算可信,避免杂锦集/他人同名专辑混入。
fn album_search(
    client: &reqwest::blocking::Client,
    album: &str,
    artist: &str,
) -> (Option<String>, Option<i32>, bool) {
    let Some(v) = netease_search(client, album, 10, 10) else {
        return (None, None, false);
    };
    let pick = v
        .pointer("/result/albums")
        .and_then(|x| x.as_array())
        .and_then(|arr| arr.iter().find(|a| artist_matches(a, artist)));
    match pick {
        Some(a) => {
            let pic = a["picUrl"].as_str().map(sized_cover);
            let year = a["publishTime"].as_i64().and_then(year_from_ms);
            (pic, year, true)
        }
        None => (None, None, false),
    }
}

fn song_cover(song: &Value) -> Option<String> {
    song.pointer("/album/picUrl")
        .and_then(|x| x.as_str())
        .map(sized_cover)
        .filter(|s| !s.is_empty())
}

fn album_of(song: &Value) -> Option<String> {
    song.pointer("/album/name")
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 封面链接限制到 1000px,避免动辄数 MB 的原图
fn sized_cover(url: &str) -> String {
    let url = url.replace("http://", "https://");
    if url.contains("param=") {
        return url;
    }
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}param=1000y1000")
}

fn download(client: &reqwest::blocking::Client, url: &str, dest: &Path) -> Result<(), String> {
    let bytes = client
        .get(url)
        .header("Referer", "https://music.163.com")
        .send()
        .map_err(|e| e.to_string())?
        .bytes()
        .map_err(|e| e.to_string())?;
    if bytes.len() < 1000 {
        return Err(format!("响应过小({} 字节)", bytes.len()));
    }
    fs::write(dest, &bytes).map_err(|e| e.to_string())
}

/// 修正打包副本的标签:标题去杂字符、写入可信专辑/年份/专辑艺人(源文件不动)
fn fix_tags(
    path: &Path,
    title: &str,
    album: Option<&str>,
    year: Option<i32>,
    album_artist: &str,
) -> Result<(), String> {
    let mut tagged = Probe::open(path)
        .map_err(|e| e.to_string())?
        .read()
        .map_err(|e| e.to_string())?;
    if tagged.primary_tag().is_none() && tagged.first_tag().is_none() {
        return Ok(());
    }
    let mut tag_opt = tagged.primary_tag_mut();
    if tag_opt.is_none() {
        tag_opt = tagged.first_tag_mut();
    }
    let Some(tag) = tag_opt else {
        return Ok(());
    };
    tag.insert(TagItem::new(
        ItemKey::TrackTitle,
        ItemValue::Text(title.to_string()),
    ));
    if let Some(a) = album {
        tag.insert(TagItem::new(
            ItemKey::AlbumTitle,
            ItemValue::Text(a.to_string()),
        ));
    }
    if let Some(y) = year {
        tag.insert(TagItem::new(ItemKey::Year, ItemValue::Text(y.to_string())));
    }
    if tag
        .get_string(&ItemKey::AlbumArtist)
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
    {
        tag.insert(TagItem::new(
            ItemKey::AlbumArtist,
            ItemValue::Text(album_artist.to_string()),
        ));
    }
    tagged
        .save_to_path(path, WriteOptions::default())
        .map_err(|e| e.to_string())
}

/// 毫秒时间戳 -> 年份(Howard Hinnant civil_from_days)
fn year_from_ms(ms: i64) -> Option<i32> {
    let z = ms.div_euclid(86_400_000) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    i32::try_from(year)
        .ok()
        .filter(|&y| (1000..3000).contains(&y))
}

fn pack(dest: &Path, work: &Path) -> Result<PathBuf, String> {
    let mut writer = ArchiveWriter::create(dest).map_err(|e| format!("创建 TMC 失败: {e}"))?;
    let mut entries: Vec<_> = fs::read_dir(work)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        writer
            .push_archive_entry(ArchiveEntry::from_path(&p, name), fs::File::open(&p).ok())
            .map_err(|e| format!("打包条目失败: {e}"))?;
    }
    writer.finish().map_err(|e| format!("写 TMC 失败: {e}"))?;
    Ok(dest.to_path_buf())
}
