//! 在线元数据补全编排:封面/歌词多级兜底匹配(网易云 → LRCLIB/iTunes → 酷我)与在线曲目下载入库。

use crate::import::{import_paths_blocking, media_ext_from_url, ImportReport};
use crate::model;
use crate::online::{
    artist_matches, build_lrc_from_kuwo, html_unescape, kuwo_keywords, name_matches, normalize_tag,
    strip_bracketed, title_is_live,
};
use crate::pack::sanitize_name;
use crate::{lyrics, online, panic_message, AppState};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;

// ===== 在线元数据补全(网易云公开接口,仅补封面/歌词,不涉及流媒体) =====

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NeteaseReport {
    cover: bool,
    lyrics: u32,
    /// 其中由 LRCLIB 兜底补上的数量(含在 lyrics 内)
    lyrics_lrclib: u32,
    /// 其中由酷我兜底补上的数量(含在 lyrics 内)
    lyrics_kuwo: u32,
    skipped: u32,
}

/// 在线封面专用客户端:禁止跟随重定向。直链已通过域名白名单,
/// 但 30x 仍可能把请求带到白名单外/内网地址,图片抓取不接受跳转。
/// 不携带任何平台的 Cookie/Referer——跨平台拉图时不能把网易云凭据发给别家
fn online_cover_client() -> reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            online::build_http_client(online::HttpClientSpec {
                timeout: std::time::Duration::from_secs(15),
                ua: online::BROWSER_UA,
                referer: None,
                cookie: None,
                follow_redirects: false,
            })
        })
        .clone()
}

/// 在线曲目封面落盘:取专辑图直链,存到封面缓存 covers/online/,
/// 返回本地路径(前端经 asset 协议渲染,与本地封面同一套链路)。
/// 缓存命中直接回;None = 无图;网络失败按 Err 透传
pub(crate) fn ensure_online_cover(
    source: &str,
    id: &str,
    cover_url: Option<&str>,
    app: &tauri::AppHandle,
) -> Result<Option<String>, String> {
    // id 会直接拼进缓存文件名,先挡住路径穿越;各来源的 id 形状在 online.rs 统一校验
    online::validate_id(source, id)?;
    let dir = app
        .state::<AppState>()
        .cache_dir
        .join("covers")
        .join("online");
    // 网易云沿用旧文件名,避免已有缓存失效;其他来源加前缀隔离,防 hash/id 撞名
    let file = if source == "netease" {
        format!("{id}.jpg")
    } else {
        format!("{source}_{id}.jpg")
    };
    let dest = dir.join(file);
    if dest.is_file() {
        return Ok(Some(dest.to_string_lossy().into_owned()));
    }
    let url = match source {
        "netease" => online::cover_url_netease(id)?,
        // QQ/酷狗封面直链来自搜索结果;主机白名单在 Rust 端再校验一次
        "qq" | "kugou" => {
            let Some(raw) = cover_url else {
                return Ok(None);
            };
            match online::validate_cover_url(source, raw) {
                Ok(u) => Some(u),
                Err(_) => return Ok(None),
            }
        }
        // 酷我搜索接口不提供封面字段,首版保持占位
        _ => None,
    };
    let Some(url) = url else {
        return Ok(None);
    };
    // fetch_capped_allow_http 自带 20MiB 上限与 http→https 回退;图即 jpg,原样落盘。
    // 专用客户端禁止重定向,30x 一律按无图处理
    let Some(bytes) = fetch_capped_allow_http(&online_cover_client(), &url) else {
        return Ok(None);
    };
    if bytes.is_empty() {
        return Ok(None);
    }
    fs::create_dir_all(&dir).map_err(|e| format!("创建封面目录失败: {e}"))?;
    // 先写临时名再改名:半截文件不能被当作完整封面缓存
    let tmp = dest.with_extension("tmp");
    fs::write(&tmp, &bytes).map_err(|e| format!("封面写盘失败: {e}"))?;
    fs::rename(&tmp, &dest).map_err(|e| format!("封面落位失败: {e}"))?;
    Ok(Some(dest.to_string_lossy().into_owned()))
}

/// 在线下载专用客户端:整首歌的传输不能套取链接口的 15s 短超时,
/// 320k 曲目约 10MB,弱网下要跑几十秒。不带 Cookie/Referer——直链无鉴权,
/// 把某平台的凭据发给别家 CDN 只是多余暴露。允许重定向是因为存下来的字节
/// 必须与播放器直接流式听到的那份一致(试听 CDN 本身就靠 30x 选节点)
fn download_client() -> reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            online::build_http_client(online::HttpClientSpec {
                timeout: std::time::Duration::from_secs(300),
                ua: online::BROWSER_UA,
                referer: None,
                cookie: None,
                follow_redirects: true,
            })
        })
        .clone()
}

/// 给下载的音频写基础标签(标题/艺人/专辑/封面)。文件名解析只是兜底,
/// 标签才是曲库元数据的正源;失败不致命,下载流程继续(仅元数据降级)
fn write_audio_tags(
    path: &Path,
    name: &str,
    artist: &str,
    album: &str,
    cover: Option<&Path>,
) -> Result<(), String> {
    use lofty::config::WriteOptions;
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::tag::{ItemKey, Tag};
    let mut tagged = lofty::probe::Probe::open(path)
        .and_then(|p| p.read())
        .map_err(|e| format!("音频解析失败: {e}"))?;
    {
        let tag = match tagged.primary_tag_mut() {
            Some(t) => t,
            // CDN 裸流常无任何标签:按文件同型补一个空标签再填
            None => {
                let ty = tagged.primary_tag_type();
                tagged.insert_tag(Tag::new(ty));
                tagged
                    .primary_tag_mut()
                    .ok_or_else(|| "无法建立音频标签".to_string())?
            }
        };
        tag.insert_text(ItemKey::TrackTitle, name.to_string());
        tag.insert_text(ItemKey::TrackArtist, artist.to_string());
        if !album.trim().is_empty() {
            tag.insert_text(ItemKey::AlbumTitle, album.to_string());
        }
        // 内嵌封面:在线专辑图恒为 jpg;有旧封面先摘,防重复叠加
        if let Some(cover) = cover {
            let data = fs::read(cover).map_err(|e| format!("封面读取失败: {e}"))?;
            if !data.is_empty() {
                tag.remove_picture_type(PictureType::CoverFront);
                tag.push_picture(Picture::new_unchecked(
                    PictureType::CoverFront,
                    Some(MimeType::Jpeg),
                    None,
                    data,
                ));
            }
        }
    }
    // 变更写回原文件:TaggedFile 经 AudioFile::save_to_path 就地保存
    tagged
        .save_to_path(path, WriteOptions::default())
        .map_err(|e| format!("标签写入失败: {e}"))
}

/// 下载入库附带的曲目元数据:取自在线搜索结果,是内嵌标签与文件名的正源
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OnlineDownloadMeta {
    name: String,
    artist: String,
    album: String,
    /// QQ/酷狗的封面直链来自搜索结果(网易云走详情接口,这里为空)
    #[serde(default)]
    cover_url: Option<String>,
}

pub(crate) fn online_download_blocking(
    source: &str,
    id: &str,
    meta: &OnlineDownloadMeta,
    app: &tauri::AppHandle,
) -> Result<ImportReport, String> {
    let OnlineDownloadMeta {
        name,
        artist,
        album,
        cover_url,
    } = meta;
    // 1. 取链:与播放同一来源,存进库的就是刚听到的那份文件
    let url = online::play_url(source, id)?;
    // 2. 下载到内存(免费档整首约 3-12MB,一次读入可接受)
    let resp = download_client()
        .get(&url)
        .send()
        .map_err(|e| online::net_err("下载请求失败", &e))?;
    if !resp.status().is_success() {
        return Err(format!("下载失败(HTTP {})", resp.status().as_u16()));
    }
    let bytes = resp
        .bytes()
        .map_err(|_| "下载中断,请检查网络后重试".to_string())?;
    if bytes.is_empty() {
        return Err("下载内容为空,请稍后重试".into());
    }
    // 3. 落到一次性临时目录:导入管线按 file_name() 复制,所以唯一性靠目录、
    //    文件名保持干净的「艺人 - 标题」——标签写失败时扫描器还能从文件名兜底解析
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let stem = format!("{} - {}", sanitize_name(artist), sanitize_name(name));
    let ext = media_ext_from_url(&url);
    let staging = std::env::temp_dir().join(format!("taurimusic-dl-{nanos}"));
    fs::create_dir_all(&staging).map_err(|e| format!("创建临时目录失败: {e}"))?;
    // 从这里起任何一步失败都要清掉临时目录:导入管线对暂存件只读(复制走),
    // 成功与失败 alike 都不再需要它。闭包统一收尾,防 `?` 早退泄漏
    let report = (|| -> Result<ImportReport, String> {
        let tmp = staging.join(format!("{stem}.{ext}"));
        fs::write(&tmp, &bytes).map_err(|e| format!("写入临时文件失败: {e}"))?;
        // 顺带取在线封面嵌进标签、歌词写成同名 .lrc:入库件即刻带图文,
        // 两者失败都不致命(后续可用在线匹配补齐)
        let cover = ensure_online_cover(source, id, cover_url.as_deref(), app)
            .ok()
            .flatten()
            .map(PathBuf::from);
        let _ = write_audio_tags(&tmp, name, artist, album, cover.as_deref());
        let mut pending = vec![tmp.to_string_lossy().into_owned()];
        let lrc_tmp = tmp.with_extension("lrc");
        if let Ok(Some(payload)) = online::lyric_payload(source, id) {
            if let Some(text) = lyrics::render(&payload) {
                if fs::write(&lrc_tmp, text).is_ok() {
                    pending.push(lrc_tmp.to_string_lossy().into_owned());
                }
            }
        }
        // 复用散装导入管线:复制进导入目录 + 全量扫描 + 指纹记录 + 事件广播
        import_paths_blocking(&pending, app)
    })();
    // 临时目录整体清掉:音频与旁路歌词成对生成,也成对消失
    let _ = fs::remove_dir_all(&staging);
    report
}

pub(crate) fn netease_enrich_blocking(
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

    let client = online::netease_client();

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
            // 封面下载与在线曲目封面(ensure_online_cover)同一威胁模型:
            // URL 来自接口 JSON,30x 可能把请求带去白名单外/内网,统一禁止重定向。
            // 三级兜底给出的直链本身已是 https 或带 http→https 首选回退,不依赖 30x
            Some(pic_url) => match fetch_capped_allow_http(&online_cover_client(), &pic_url) {
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

    // 歌词:批量并行匹配无 .lrc 的曲目(网易云 → LRCLIB → 酷我 三级兜底);
    // 已有外文歌词但缺译文的曲目顺带补译(只并译文进现有文件,原文不替换)
    let targets: Vec<&model::Track> = tracks
        .iter()
        .filter(|t| match &t.lrc_path {
            None => true,
            Some(p) => fs::read_to_string(p)
                .map(|c| lyrics::needs_translation(&c))
                .unwrap_or(false),
        })
        .collect();
    let results: Vec<Option<(String, &'static str)>> = std::thread::scope(|scope| {
        let handles: Vec<_> = targets
            .chunks(4)
            .map(|chunk| {
                let client = &client;
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|t| {
                            if t.lrc_path.is_some() {
                                // 补译路径:网易 tlyric 与现有文件对齐合并,失败静默跳过
                                return netease_backfill_translation(client, t)
                                    .map(|text| (text, "netease"));
                            }
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
            .flat_map(|h| match h.join() {
                Ok(v) => v,
                Err(payload) => {
                    // 单个分块 worker panic 只丢这一块的歌词匹配,不该炸整个命令;
                    // 但必须留痕,静默吞掉(unwrap_or_default)会让"补词无结果"无从排查
                    eprintln!(
                        "[enrich] 歌词匹配 worker panic: {}",
                        panic_message(&*payload)
                    );
                    Vec::new()
                }
            })
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

/// 网易云搜歌 + 同一次请求带回歌词原文与译文(tv=1 时接口返回 tlyric 官方译配)
fn netease_lyric_pair(
    client: &reqwest::blocking::Client,
    title: &str,
    artist: &str,
    duration: Option<f64>,
) -> Option<(String, Option<String>)> {
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
        let url = format!("https://music.163.com/api/song/lyric?id={id}&lv=1&kv=1&tv=1");
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
                let trans = v
                    .pointer("/tlyric/lyric")
                    .and_then(|x| x.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                return Some((text.to_string(), trans.map(str::to_string)));
            }
        }
    }
    None
}

pub fn netease_lyric(
    client: &reqwest::blocking::Client,
    title: &str,
    artist: &str,
    duration: Option<f64>,
) -> Option<String> {
    let (text, trans) = netease_lyric_pair(client, title, artist, duration)?;
    Some(match trans {
        // 原文与译文出自同一次请求(同一版本),配不上的译文行自然丢弃
        Some(t) => lyrics::merge_translation(&text, &t).0,
        None => text,
    })
}

/// 已有歌词的曲目补译文:再取一次网易云 tlyric,按时间戳并进现有文件。
/// 原文一个字都不换;配对率不足八成(网易版本与本地文件错位)时宁可不写
fn netease_backfill_translation(
    client: &reqwest::blocking::Client,
    t: &model::Track,
) -> Option<String> {
    let path = t.lrc_path.as_deref()?;
    let orig = fs::read_to_string(path).ok()?;
    if !lyrics::needs_translation(&orig) {
        return None;
    }
    let (_, trans) = netease_lyric_pair(client, &t.title, &t.artist, Some(t.duration))?;
    let (merged, paired, total) = lyrics::merge_translation(&orig, &trans?);
    if total > 0 && paired * 5 >= total * 4 {
        Some(merged)
    } else {
        None
    }
}

// ===== LRCLIB 歌词兜底 + iTunes 封面兜底(公开接口,无需密钥) =====

/// 通用 HTTP 客户端(LRCLIB / iTunes 等公开接口):仅带标识 UA,不带网易云的站头。
/// 只用于 JSON 接口调用;抓图必须走禁止重定向的 online_cover_client
pub fn generic_client() -> reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            online::build_http_client(online::HttpClientSpec {
                timeout: std::time::Duration::from_secs(15),
                ua: online::APP_UA,
                referer: None,
                cookie: None,
                follow_redirects: true,
            })
        })
        .clone()
}

/// 从 LRCLIB 搜索结果里挑最佳一条:艺人能对上才要;
/// 现场版标记优先于同步格式优先于时长吻合(时间轴错了同步歌词毫无意义)。
/// 返回 (歌词文本, 译文):translatedLyrics 多为无时间戳的纯文本,
/// 由调用方交给 merge_translation——对不齐的译文会原样丢弃
fn pick_lrclib_result(
    items: &[serde_json::Value],
    artist: &str,
    duration: Option<f64>,
    want_live: bool,
) -> Option<(String, Option<String>)> {
    let mut best: Option<(u8, String, Option<String>)> = None;
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
        let trans = it["translatedLyrics"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
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
        if best.as_ref().map(|(s, _, _)| score > *s).unwrap_or(true) {
            best = Some((score, text.to_string(), trans));
        }
    }
    best.map(|(_, text, trans)| (text, trans))
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
                            let trans = v["translatedLyrics"]
                                .as_str()
                                .map(str::trim)
                                .filter(|s| !s.is_empty());
                            return Some(match trans {
                                Some(t) => lyrics::merge_translation(text, t).0,
                                None => text.to_string(),
                            });
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
        if let Some((text, trans)) = pick_lrclib_result(items, artist, duration, want_live) {
            return Some(match trans {
                Some(t) => lyrics::merge_translation(&text, &t).0,
                None => text,
            });
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

/// 酷我歌曲搜索:返回 abslist 列表。
/// 走 online::kuwo_rs_search 统一实现;参数保持本地匹配链的旧档
/// (client=mp + vipver + rn=10,与在线搜索的 kt/30 分叉是历史现状,不改线上行为)
fn kuwo_search(
    client: &reqwest::blocking::Client,
    keyword: &str,
) -> Option<Vec<serde_json::Value>> {
    online::kuwo_rs_search(client, keyword, 0, 10, "mp", Some("MUSIC_8.0.3.2_QQZS")).ok()
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

#[cfg(test)]
mod tests {
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
            pick_lrclib_result(&items, "beyond", Some(300.0), false),
            Some(("[00:01.00] line".into(), None))
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
            pick_lrclib_result(&t2, "陈奕迅", None, false),
            Some(("[00:02.00] y".into(), None))
        );
        // 带译文的条目:译文随歌词一起带回
        let t3 = vec![
            serde_json::json!({"artistName": "beyond", "duration": 300.0,
            "syncedLyrics": "[00:01.00] line", "translatedLyrics": "[00:01.00] 行"}),
        ];
        assert_eq!(
            pick_lrclib_result(&t3, "beyond", Some(300.0), false),
            Some(("[00:01.00] line".into(), Some("[00:01.00] 行".into())))
        );
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
            ("b".into(), None)
        );
        // 非现场版请求:同步格式优先
        assert_eq!(
            pick_lrclib_result(&items, "李晓杰", Some(280.0), false).unwrap(),
            ("[00:01]a".into(), None)
        );
    }
}
