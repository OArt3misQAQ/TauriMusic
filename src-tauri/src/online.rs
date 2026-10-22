//! 在线音源(网易云 / QQ 音乐 / 酷狗 / 酷我公开接口)。
//! 合规边界:仅调用各平台网页端公开的搜索与试听接口,免费曲目返回官方
//! CDN 直链可直接播放;VIP/付费/版权受限曲目不做登录态、签名绕过或付费
//! 内容解锁,原样向上报错。
//! 各家付费闸门不同:网易云/QQ/酷狗由接口自身对 VIP 返回空链;酷我 anti.s
//! 对 VIP 也会返回直链,因此酷我只允许本次会话搜索结果里 PAY=0 的曲目取链。
//! 接口无官方文档、字段随时可能变化,解析按宽松取值处理。

use base64::Engine as _;
use serde::Serialize;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// 搜索单条结果(前端展示 + 构造播放队列)
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OnlineTrack {
    /// 平台曲目 id(前端拼来源前缀作队列 id)
    pub id: String,
    pub name: String,
    pub artist: String,
    pub album: String,
    /// 时长(秒;接口给毫秒时已换算)
    pub duration: u64,
    /// 0=免费 1=VIP/付费(网易云另含 8=低音质免费)
    pub fee: u8,
    /// netease / qq / kugou / kuwo
    pub source: String,
    /// 搜索接口直接给出的封面直链(网易云走详情接口,这里为 None)
    pub cover: Option<String>,
}

/// 在线曲目 id 的统一前缀:net: / qq: / kugou: / kuwo:
pub fn is_online_id(id: &str) -> bool {
    split_online_id(id).is_some()
}

/// 拆出 (来源, 平台 id);本地路径 hash 不含冒号,不会误判
pub fn split_online_id(id: &str) -> Option<(&'static str, &str)> {
    if let Some(rest) = id.strip_prefix("net:") {
        Some(("netease", rest))
    } else if let Some(rest) = id.strip_prefix("qq:") {
        Some(("qq", rest))
    } else if let Some(rest) = id.strip_prefix("kugou:") {
        Some(("kugou", rest))
    } else if let Some(rest) = id.strip_prefix("kuwo:") {
        Some(("kuwo", rest))
    } else {
        None
    }
}

fn numeric_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
}

fn qq_id_ok(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn kugou_id_ok(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// 平台曲目 id 校验:它同时用于拼 API 查询与封面缓存文件名,
/// 放行任意字符串会让 `../` 之类的输入有机会穿透缓存目录
pub fn validate_id(source: &str, id: &str) -> Result<(), String> {
    let ok = match source {
        "netease" | "kuwo" => numeric_id(id),
        "qq" => qq_id_ok(id),
        "kugou" => kugou_id_ok(id),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err("在线曲目 id 非法".into())
    }
}

fn media_host_allowed(source: &str, host: &str) -> bool {
    match source {
        "netease" => {
            host == "music.163.com" || host == "music.126.net" || host.ends_with(".music.126.net")
        }
        // QQ 直链统一从 isure.stream.qqmusic.qq.com 取,只放行该 CDN 家族
        "qq" => host.ends_with(".stream.qqmusic.qq.com"),
        "kugou" => host == "kugou.com" || host.ends_with(".kugou.com"),
        "kuwo" => host == "kuwo.cn" || host.ends_with(".kuwo.cn"),
        _ => false,
    }
}

fn cover_host_allowed(source: &str, host: &str) -> bool {
    match source {
        "netease" => media_host_allowed("netease", host),
        "qq" => host == "y.gtimg.cn" || host.ends_with(".gtimg.cn"),
        "kugou" => media_host_allowed("kugou", host),
        _ => false,
    }
}

/// 校验并归一化在线 URL:http 一律升级为 https(WebView CSP 只放行
/// 白名单内的 https 源),拒绝非白名单主机、非 https、带 userinfo 或非常规端口
fn restrict_url(raw: &str, allowed: impl Fn(&str) -> bool) -> Result<String, String> {
    let upgraded = raw.replacen("http://", "https://", 1);
    let url = reqwest::Url::parse(&upgraded).map_err(|_| "在线媒体地址非法".to_string())?;
    let host = url.host_str().ok_or("在线媒体地址缺少主机")?;
    let port_ok = url.port().is_none() || url.port() == Some(443);
    if url.scheme() != "https"
        || !allowed(host)
        || !port_ok
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() == "/"
    {
        return Err("在线媒体地址不在允许范围".into());
    }
    Ok(url.to_string())
}

fn restrict_media_url(source: &str, raw: &str) -> Result<String, String> {
    restrict_url(raw, |host| media_host_allowed(source, host))
}

fn restrict_cover_url(source: &str, raw: &str) -> Result<String, String> {
    restrict_url(raw, |host| cover_host_allowed(source, host))
}

fn qq_client() -> reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::blocking::Client::builder()
                .user_agent(
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
                )
                .default_headers(reqwest::header::HeaderMap::from_iter([(
                    reqwest::header::REFERER,
                    "https://y.qq.com/".parse().unwrap(),
                )]))
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new())
        })
        .clone()
}

fn kugou_client() -> reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::blocking::Client::builder()
                .user_agent(
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
                )
                .default_headers(reqwest::header::HeaderMap::from_iter([(
                    reqwest::header::REFERER,
                    "https://www.kugou.com/".parse().unwrap(),
                )]))
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new())
        })
        .clone()
}

fn kuwo_client() -> reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::blocking::Client::builder()
                .user_agent(
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
                )
                .default_headers(reqwest::header::HeaderMap::from_iter([(
                    reqwest::header::REFERER,
                    "https://www.kuwo.cn/".parse().unwrap(),
                )]))
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new())
        })
        .clone()
}

fn build_payload(lrc: String, trans: Option<String>) -> Option<crate::lyrics::LyricsPayload> {
    let text = match trans {
        Some(t) if crate::lyrics::needs_translation(&lrc) => {
            let (merged, paired, total) = crate::lyrics::merge_translation(&lrc, &t);
            if total > 0 && paired * 5 >= total * 4 {
                merged
            } else {
                lrc
            }
        }
        _ => lrc,
    };
    let payload = crate::lyrics::parse(&text);
    (!payload.is_empty()).then_some(payload)
}

// ===== 网易云 =====

/// 把 result.songs 里的一条解析成 OnlineTrack:字段按宽松取值,
/// 任何字段缺失都不至于丢弃整条结果
fn parse_netease_song(v: &serde_json::Value) -> Option<OnlineTrack> {
    let id = v.get("id")?.as_u64()?.to_string();
    let name = v
        .get("name")
        .and_then(|x| x.as_str())
        .unwrap_or("未知曲目")
        .to_string();
    let artist = v
        .get("artists")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                .collect::<Vec<_>>()
                .join("/")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "未知艺人".into());
    let album = v
        .get("album")
        .and_then(|a| a.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();
    let duration_ms = v.get("duration").and_then(|d| d.as_u64()).unwrap_or(0);
    let fee = v.get("fee").and_then(|f| f.as_u64()).unwrap_or(1) as u8;
    Some(OnlineTrack {
        id,
        name,
        artist,
        album,
        duration: duration_ms / 1000,
        fee,
        source: "netease".into(),
        cover: None,
    })
}

/// 搜索网易云曲目(公开网页接口,type=1 单曲)。
/// 单条结果可能带同名义务字段缺失,宽松解析,失败条目跳过。
pub fn search_netease(keyword: &str, page: u32) -> Result<Vec<OnlineTrack>, String> {
    let keyword = keyword.trim();
    if keyword.is_empty() {
        return Ok(vec![]);
    }
    let client = crate::netease_client();
    let offset = page.saturating_mul(30).to_string();
    let resp = client
        .post("https://music.163.com/api/search/get/web")
        .form(&[
            ("s", keyword),
            ("type", "1"),
            ("limit", "30"),
            ("offset", &offset),
        ])
        .send()
        .map_err(|e| format!("搜索请求失败: {e}"))?;
    let v: serde_json::Value = resp.json().map_err(|e| format!("搜索响应解析失败: {e}"))?;
    // 网易云偶发业务错误码(如 -462 反爬验证),有 code 且非 200 时透传
    if let Some(code) = v.get("code").and_then(|c| c.as_i64()) {
        if code != 200 {
            return Err(format!("搜索失败(接口码 {code}),请稍后重试"));
        }
    }
    let songs = v
        .get("result")
        .and_then(|r| r.get("songs"))
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(songs.iter().filter_map(parse_netease_song).collect())
}

/// 取网易云播放直链(官方试听 CDN)。CDN 只给 http,域名本身支持 https,
/// 统一升级为 https——CSP 只放行白名单 https 源,不升级会被 WebView 拦。
/// 返回空链即版权/VIP 受限,不做任何绕过,直接报错给前端提示。
pub fn play_url_netease(id: &str) -> Result<String, String> {
    validate_id("netease", id)?;
    let client = crate::netease_client();
    let url = "https://music.163.com/api/song/enhance/player/url";
    let resp = client
        .get(url)
        .query(&[("ids", format!("[{id}]").as_str()), ("br", "128000")])
        .send()
        .map_err(|e| format!("取链请求失败: {e}"))?;
    let v: serde_json::Value = resp.json().map_err(|e| format!("取链响应解析失败: {e}"))?;
    let item = v
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|a| a.first())
        .ok_or("取链响应缺少数据")?;
    let link = item
        .get("url")
        .and_then(|u| u.as_str())
        .filter(|s| !s.is_empty())
        .ok_or("该曲目需要 VIP 或暂无版权,无法在线播放")?;
    restrict_media_url("netease", link)
}

/// 从歌词接口响应里宽松取值:(LRC 原文, 可选译文)。
/// lrc/tlyric 任一缺失或为空都不算错误,只是没有对应的词
fn parse_lyric_json(v: &serde_json::Value) -> Option<(String, Option<String>)> {
    let lrc = v
        .get("lrc")
        .and_then(|l| l.get("lyric"))
        .and_then(|s| s.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)?;
    let trans = v
        .get("tlyric")
        .and_then(|t| t.get("lyric"))
        .and_then(|s| s.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some((lrc, trans))
}

/// 取网易云歌词(公开歌词接口,无登录态无签名)。
/// 返回 None = 该曲目确实没有歌词(区别于网络错误)
pub fn lyric_netease(id: &str) -> Result<Option<(String, Option<String>)>, String> {
    validate_id("netease", id)?;
    let client = crate::netease_client();
    let resp = client
        .get("https://music.163.com/api/song/lyric")
        .query(&[("id", id), ("lv", "1"), ("kv", "1"), ("tv", "-1")])
        .send()
        .map_err(|e| format!("歌词请求失败: {e}"))?;
    let v: serde_json::Value = resp.json().map_err(|e| format!("歌词响应解析失败: {e}"))?;
    Ok(parse_lyric_json(&v))
}

/// 取网易云歌词并解析为播放器歌词载荷:
/// 外语歌自动对齐官方译文(tlyric),对齐率不足四成时宁缺毋滥只保留原文——
/// 版本错位的双语比没有译文更糟。空词与请求失败都归一为 None(播放页显示无词)
pub fn lyric_payload_netease(id: &str) -> Result<Option<crate::lyrics::LyricsPayload>, String> {
    let Some((lrc, trans)) = lyric_netease(id)? else {
        return Ok(None);
    };
    Ok(build_payload(lrc, trans))
}

/// 从歌曲详情接口宽松取专辑图直链;缺图返回 None(不算错误)
fn parse_pic_url(v: &serde_json::Value) -> Option<String> {
    v.get("songs")
        .and_then(|s| s.as_array())
        .and_then(|a| a.first())
        .and_then(|s| s.get("album"))
        .and_then(|al| al.get("picUrl"))
        .and_then(|u| u.as_str())
        .filter(|s| !s.is_empty())
        .map(|u| u.replacen("http://", "https://", 1))
}

/// 取网易云专辑图直链(歌曲详情公开接口)。
/// picUrl 支持 ?param=WxH 请求指定尺寸,这里要 512px 方图,与封面缓存档位对齐;
/// 返回 None = 该曲无图(区别于网络错误)。主机不在白名单时同样按无图处理,
/// 避免接口异常响应把 Rust 端变成任意 URL 下载器
pub fn cover_url_netease(id: &str) -> Result<Option<String>, String> {
    validate_id("netease", id)?;
    let client = crate::netease_client();
    let url = format!("https://music.163.com/api/song/detail/?id=[{id}]&ids=[{id}]");
    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("歌曲详情请求失败: {e}"))?;
    let v: serde_json::Value = resp.json().map_err(|e| format!("详情响应解析失败: {e}"))?;
    let Some(raw) = parse_pic_url(&v) else {
        return Ok(None);
    };
    // URL 可能已带查询串,统一追加(或替换)param 为 512px 方图
    let base = raw.split('?').next().unwrap_or(&raw);
    Ok(restrict_cover_url("netease", &format!("{base}?param=512y512")).ok())
}

// ===== QQ 音乐 =====

fn parse_qq_song(v: &serde_json::Value) -> Option<OnlineTrack> {
    let id = v.get("mid").and_then(|x| x.as_str())?.to_string();
    if !qq_id_ok(&id) {
        return None;
    }
    let name = crate::html_unescape(v.get("name").and_then(|x| x.as_str()).unwrap_or("未知曲目"))
        .trim()
        .to_string();
    let artist = v
        .get("singer")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                .collect::<Vec<_>>()
                .join("/")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "未知艺人".into());
    let album = crate::html_unescape(
        v.get("album")
            .and_then(|a| a.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or(""),
    )
    .trim()
    .to_string();
    let duration = v.get("interval").and_then(|d| d.as_u64()).unwrap_or(0);
    let pay_play = v
        .get("pay")
        .and_then(|p| p.get("pay_play"))
        .and_then(|p| p.as_u64())
        .unwrap_or(1);
    let fee = if pay_play == 0 { 0 } else { 1 };
    let cover = v
        .get("album")
        .and_then(|a| a.get("mid"))
        .and_then(|m| m.as_str())
        .filter(|m| !m.is_empty())
        .map(|mid| format!("https://y.gtimg.cn/music/photo_new/T002R500x500M000{mid}.jpg"));
    Some(OnlineTrack {
        id,
        name,
        artist,
        album,
        duration,
        fee,
        source: "qq".into(),
        cover,
    })
}

/// 搜索 QQ 音乐单曲。pay_play=0 才是免费曲目,其余一律标记为 VIP/付费
pub fn search_qq(keyword: &str, page: u32) -> Result<Vec<OnlineTrack>, String> {
    let keyword = keyword.trim();
    if keyword.is_empty() {
        return Ok(vec![]);
    }
    let p = (page + 1).to_string();
    let resp = qq_client()
        .get("https://c.y.qq.com/soso/fcgi-bin/client_search_cp")
        .query(&[
            ("format", "json"),
            ("p", p.as_str()),
            ("n", "30"),
            ("w", keyword),
            ("cr", "1"),
            ("new_json", "1"),
        ])
        .send()
        .map_err(|e| format!("QQ 搜索请求失败: {e}"))?;
    let v: serde_json::Value = resp
        .json()
        .map_err(|e| format!("QQ 搜索响应解析失败: {e}"))?;
    if let Some(code) = v.get("code").and_then(|c| c.as_i64()) {
        if code != 0 {
            return Err(format!("QQ 搜索失败(接口码 {code})"));
        }
    }
    let songs = v
        .get("data")
        .and_then(|d| d.get("song"))
        .and_then(|s| s.get("list"))
        .and_then(|l| l.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(songs.iter().filter_map(parse_qq_song).collect())
}

/// 取 QQ 播放直链:vkey 对 VIP 返回空 purl,不做任何绕过。
/// 直链统一从 isure.stream.qqmusic.qq.com 拼出(其 sip 列表含该 https 主机)
pub fn play_url_qq(id: &str) -> Result<String, String> {
    validate_id("qq", id)?;
    let body = serde_json::json!({
        "req": {
            "module": "CDN.SrfCdnDispatchServer",
            "method": "GetCdnDispatch",
            "param": {"guid": "3982823384", "calltype": 0, "userip": ""}
        },
        "req_0": {
            "module": "vkey.GetVkeyServer",
            "method": "CgiGetVkey",
            "param": {
                "guid": "3982823384",
                "songmid": [id],
                "songtype": [0],
                "uin": "0",
                "loginflag": 1,
                "platform": "20"
            }
        },
        "comm": {"uin": 0, "format": "json", "ct": 24, "cv": 0}
    });
    let resp = qq_client()
        .post("https://u.y.qq.com/cgi-bin/musicu.fcg")
        .json(&body)
        .send()
        .map_err(|e| format!("QQ 取链请求失败: {e}"))?;
    let v: serde_json::Value = resp
        .json()
        .map_err(|e| format!("QQ 取链响应解析失败: {e}"))?;
    let purl = v
        .pointer("/req_0/data/midurlinfo/0/purl")
        .and_then(|p| p.as_str())
        .filter(|p| !p.is_empty())
        .ok_or("该曲目需要 VIP 或暂无版权,无法在线播放")?;
    let purl = purl.trim_start_matches('/');
    restrict_media_url("qq", &format!("https://isure.stream.qqmusic.qq.com/{purl}"))
}

/// 取 QQ 歌词:公开歌词接口直接返回 LRC 文本(nobase64=1),
/// trans 存在时按与网易云相同的对齐率规则合并译文
pub fn lyric_payload_qq(id: &str) -> Result<Option<crate::lyrics::LyricsPayload>, String> {
    validate_id("qq", id)?;
    let resp = qq_client()
        .get("https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg")
        .query(&[("songmid", id), ("format", "json"), ("nobase64", "1")])
        .send()
        .map_err(|e| format!("QQ 歌词请求失败: {e}"))?;
    let v: serde_json::Value = resp
        .json()
        .map_err(|e| format!("QQ 歌词响应解析失败: {e}"))?;
    let Some(lrc) = v
        .get("lyric")
        .and_then(|l| l.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
    else {
        return Ok(None);
    };
    let trans = v
        .get("trans")
        .and_then(|t| t.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(build_payload(lrc, trans))
}

// ===== 酷狗 =====

fn kugou_cover(image: &str) -> Option<String> {
    if image.is_empty() {
        return None;
    }
    let url = image
        .replace("{size}", "480")
        .replacen("http://", "https://", 1);
    restrict_cover_url("kugou", &url).ok()
}

fn parse_kugou_song(v: &serde_json::Value) -> Option<OnlineTrack> {
    let id = v.get("FileHash").and_then(|x| x.as_str())?.to_string();
    if !kugou_id_ok(&id) {
        return None;
    }
    let singer = crate::html_unescape(v.get("SingerName").and_then(|x| x.as_str()).unwrap_or(""));
    let file_name = crate::html_unescape(
        v.get("FileName")
            .and_then(|x| x.as_str())
            .unwrap_or("未知曲目"),
    );
    let prefix = format!("{singer} - ");
    let name = file_name
        .strip_prefix(&prefix)
        .unwrap_or(&file_name)
        .trim()
        .to_string();
    let album = crate::html_unescape(v.get("AlbumName").and_then(|x| x.as_str()).unwrap_or(""))
        .trim()
        .to_string();
    let duration = v.get("Duration").and_then(|d| d.as_u64()).unwrap_or(0);
    // privilege=10 为 VIP;8 与 0 可由公开接口取到直链,按可播处理
    let privilege = v.get("Privilege").and_then(|p| p.as_u64()).unwrap_or(10);
    let fee = if privilege >= 10 { 1 } else { 0 };
    let cover = v
        .get("Image")
        .and_then(|i| i.as_str())
        .and_then(kugou_cover);
    Some(OnlineTrack {
        id: id.to_ascii_uppercase(),
        name: if name.is_empty() {
            "未知曲目".into()
        } else {
            name
        },
        artist: if singer.trim().is_empty() {
            "未知艺人".into()
        } else {
            singer.trim().to_string()
        },
        album,
        duration,
        fee,
        source: "kugou".into(),
        cover,
    })
}

/// 搜索酷狗单曲(HTTPS 公开搜索接口,返回 JSON)
pub fn search_kugou(keyword: &str, page: u32) -> Result<Vec<OnlineTrack>, String> {
    let keyword = keyword.trim();
    if keyword.is_empty() {
        return Ok(vec![]);
    }
    let p = (page + 1).to_string();
    let resp = kugou_client()
        .get("https://songsearch.kugou.com/song_search_v2")
        .query(&[
            ("keyword", keyword),
            ("page", p.as_str()),
            ("pagesize", "30"),
            ("platform", "WebFilter"),
            ("userid", "-1"),
            ("clientver", "2000"),
            ("iscorrection", "1"),
            ("privilege_filter", "0"),
            ("filter", "10"),
            ("token", ""),
            ("appid", "1014"),
        ])
        .send()
        .map_err(|e| format!("酷狗搜索请求失败: {e}"))?;
    let v: serde_json::Value = resp
        .json()
        .map_err(|e| format!("酷狗搜索响应解析失败: {e}"))?;
    let lists = v
        .get("data")
        .and_then(|d| d.get("lists"))
        .and_then(|l| l.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(lists.iter().filter_map(parse_kugou_song).collect())
}

/// 酷狗 tracker 的 key 规则:md5(小写 hash + "kgcloudv2")
fn kugou_tracker_key(hash: &str) -> String {
    format!("{:x}", md5::compute(format!("{hash}kgcloudv2")))
}

/// 取酷狗播放直链:tracker 对 VIP 返回 status=2/无 url,
/// 接口自身就是付费闸门,不做任何绕过
pub fn play_url_kugou(id: &str) -> Result<String, String> {
    validate_id("kugou", id)?;
    let hash = id.to_ascii_lowercase();
    let key = kugou_tracker_key(&hash);
    let resp = kugou_client()
        .get("http://trackercdn.kugou.com/i/v2/")
        .query(&[
            ("key", key.as_str()),
            ("hash", hash.as_str()),
            ("br", "hq"),
            ("appid", "1005"),
            ("pid", "2"),
            ("cmd", "25"),
            ("behavior", "play"),
        ])
        .send()
        .map_err(|e| format!("酷狗取链请求失败: {e}"))?;
    let v: serde_json::Value = resp
        .json()
        .map_err(|e| format!("酷狗取链响应解析失败: {e}"))?;
    if v.get("status").and_then(|s| s.as_u64()) != Some(1) {
        return Err("该曲目需要 VIP 或暂无版权,无法在线播放".into());
    }
    let link = v
        .get("url")
        .and_then(|u| u.as_array())
        .and_then(|a| a.first())
        .and_then(|u| u.as_str())
        .filter(|u| !u.is_empty())
        .ok_or("该曲目暂无可用直链")?;
    let link = link.replacen("http://", "https://", 1);
    restrict_media_url("kugou", &link)
}

/// 取酷狗歌词:先按 hash 查歌词候选,再下载 LRC(content 为 base64)
pub fn lyric_payload_kugou(id: &str) -> Result<Option<crate::lyrics::LyricsPayload>, String> {
    validate_id("kugou", id)?;
    let hash = id.to_ascii_lowercase();
    let resp = kugou_client()
        .get("https://krcs.kugou.com/search")
        .query(&[
            ("ver", "1"),
            ("man", "yes"),
            ("client", "mobi"),
            ("keyword", ""),
            ("duration", ""),
            ("hash", hash.as_str()),
        ])
        .send()
        .map_err(|e| format!("酷狗歌词搜索失败: {e}"))?;
    let v: serde_json::Value = resp
        .json()
        .map_err(|e| format!("酷狗歌词搜索解析失败: {e}"))?;
    let Some(cand) = v
        .get("candidates")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
    else {
        return Ok(None);
    };
    let lyric_id = cand
        .get("id")
        .and_then(|x| x.as_str())
        .ok_or("酷狗歌词候选缺少 id")?;
    let accesskey = cand
        .get("accesskey")
        .and_then(|x| x.as_str())
        .ok_or("酷狗歌词候选缺少 accesskey")?;
    let resp = kugou_client()
        .get("https://lyrics.kugou.com/download")
        .query(&[
            ("ver", "1"),
            ("client", "pc"),
            ("id", lyric_id),
            ("accesskey", accesskey),
            ("fmt", "lrc"),
            ("charset", "utf8"),
        ])
        .send()
        .map_err(|e| format!("酷狗歌词下载失败: {e}"))?;
    let v: serde_json::Value = resp
        .json()
        .map_err(|e| format!("酷狗歌词下载解析失败: {e}"))?;
    let Some(content) = v.get("content").and_then(|c| c.as_str()) else {
        return Ok(None);
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content)
        .map_err(|_| "酷狗歌词解码失败".to_string())?;
    let text = String::from_utf8_lossy(&bytes)
        .trim_start_matches('\u{feff}')
        .to_string();
    Ok(build_payload(text, None))
}

// ===== 酷我 =====

/// 酷我 anti.s 对 VIP 也返回直链,这里只记录本次会话搜索结果里 PAY=0 的 id;
/// 取链前必须命中白名单,避免把付费曲目变成可播放
static KUWO_FREE_IDS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
const KUWO_FREE_CAP: usize = 4096;

fn remember_kuwo_free(id: &str) {
    let cache = KUWO_FREE_IDS.get_or_init(|| Mutex::new(HashSet::new()));
    if let Ok(mut set) = cache.lock() {
        if set.len() >= KUWO_FREE_CAP {
            set.clear();
        }
        set.insert(id.to_string());
    }
}

fn kuwo_is_free(id: &str) -> bool {
    KUWO_FREE_IDS
        .get()
        .and_then(|c| c.lock().ok())
        .is_some_and(|g| g.contains(id))
}

fn parse_kuwo_song(v: &serde_json::Value) -> Option<OnlineTrack> {
    let id = v
        .get("MUSICRID")
        .and_then(|x| x.as_str())?
        .strip_prefix("MUSIC_")?
        .to_string();
    if !numeric_id(&id) {
        return None;
    }
    let name = crate::html_unescape(
        v.get("SONGNAME")
            .and_then(|x| x.as_str())
            .unwrap_or("未知曲目"),
    )
    .trim()
    .replace('\u{a0}', " ");
    let artist = crate::html_unescape(
        v.get("ARTIST")
            .and_then(|x| x.as_str())
            .unwrap_or("未知艺人"),
    )
    .trim()
    .replace('\u{a0}', " ");
    let album = crate::html_unescape(v.get("ALBUM").and_then(|x| x.as_str()).unwrap_or(""))
        .trim()
        .replace('\u{a0}', " ");
    let duration = v
        .get("DURATION")
        .and_then(|d| d.as_str())
        .and_then(|d| d.parse::<u64>().ok())
        .unwrap_or(0);
    let pay = v
        .get("PAY")
        .and_then(|p| p.as_str())
        .map(str::to_string)
        .or_else(|| v.get("PAY").and_then(|p| p.as_u64()).map(|p| p.to_string()))
        .unwrap_or_else(|| "1".into());
    let fee = if pay == "0" { 0 } else { 1 };
    if fee == 0 {
        remember_kuwo_free(&id);
    }
    Some(OnlineTrack {
        id,
        name: if name.is_empty() {
            "未知曲目".into()
        } else {
            name
        },
        artist: if artist.is_empty() {
            "未知艺人".into()
        } else {
            artist
        },
        album,
        duration,
        fee,
        source: "kuwo".into(),
        cover: None,
    })
}

/// 搜索酷我单曲(旧版公开搜索接口,返回单引号伪 JSON,复用 kuwo_fix_json)
pub fn search_kuwo(keyword: &str, page: u32) -> Result<Vec<OnlineTrack>, String> {
    let keyword = keyword.trim();
    if keyword.is_empty() {
        return Ok(vec![]);
    }
    let p = page.to_string();
    let resp = kuwo_client()
        .get("https://search.kuwo.cn/r.s")
        .query(&[
            ("all", keyword),
            ("ft", "music"),
            ("itemset", "web_2013"),
            ("client", "kt"),
            ("pn", p.as_str()),
            ("rn", "30"),
            ("rformat", "json"),
            ("encoding", "utf8"),
        ])
        .send()
        .map_err(|e| format!("酷我搜索请求失败: {e}"))?;
    let text = resp
        .text()
        .map_err(|e| format!("酷我搜索响应读取失败: {e}"))?;
    let fixed = crate::kuwo_fix_json(&text);
    let v: serde_json::Value =
        serde_json::from_str(&fixed).map_err(|e| format!("酷我搜索响应解析失败: {e}"))?;
    let list = v
        .get("abslist")
        .and_then(|l| l.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(list.iter().filter_map(parse_kuwo_song).collect())
}

/// 取酷我播放直链。anti.s 不做付费判断,因此必须先命中搜索结果白名单;
/// 未在本次会话搜索结果里出现过的 id 一律拒绝,防止绕过 VIP 限制
pub fn play_url_kuwo(id: &str) -> Result<String, String> {
    validate_id("kuwo", id)?;
    if !kuwo_is_free(id) {
        return Err("该曲目需要 VIP 或不在当前搜索结果中,无法在线播放".into());
    }
    let rid = format!("MUSIC_{id}");
    let resp = kuwo_client()
        .get("https://antiserver.kuwo.cn/anti.s")
        .query(&[
            ("type", "convert_url"),
            ("format", "mp3"),
            ("response", "url"),
            ("rid", rid.as_str()),
        ])
        .send()
        .map_err(|e| format!("酷我取链请求失败: {e}"))?;
    let link = resp
        .text()
        .map_err(|e| format!("酷我取链响应读取失败: {e}"))?
        .trim()
        .to_string();
    if link.is_empty() || !link.starts_with("http") {
        return Err("酷我暂未返回可用直链".into());
    }
    restrict_media_url("kuwo", &link)
}

/// 酷我歌词接口目前不稳定,首版不提供(播放页显示无词)
pub fn lyric_payload_kuwo(_id: &str) -> Result<Option<crate::lyrics::LyricsPayload>, String> {
    Ok(None)
}

// ===== 分发 =====

pub fn search(source: &str, keyword: &str, page: u32) -> Result<Vec<OnlineTrack>, String> {
    match source {
        "netease" => search_netease(keyword, page),
        "qq" => search_qq(keyword, page),
        "kugou" => search_kugou(keyword, page),
        "kuwo" => search_kuwo(keyword, page),
        _ => Err("未知在线音源".into()),
    }
}

pub fn play_url(source: &str, id: &str) -> Result<String, String> {
    match source {
        "netease" => play_url_netease(id),
        "qq" => play_url_qq(id),
        "kugou" => play_url_kugou(id),
        "kuwo" => play_url_kuwo(id),
        _ => Err("未知在线音源".into()),
    }
}

pub fn lyric_payload(
    source: &str,
    id: &str,
) -> Result<Option<crate::lyrics::LyricsPayload>, String> {
    match source {
        "netease" => lyric_payload_netease(id),
        "qq" => lyric_payload_qq(id),
        "kugou" => lyric_payload_kugou(id),
        "kuwo" => lyric_payload_kuwo(id),
        _ => Err("未知在线音源".into()),
    }
}

/// 封面下载前的来源统一校验:网易云走详情接口,QQ/酷狗用搜索给出的直链
pub fn validate_cover_url(source: &str, raw: &str) -> Result<String, String> {
    restrict_cover_url(source, raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_netease_song_tolerates_missing_fields() {
        // 字段缺失时不崩:artist/album 有兜底,duration 缺省 0
        let v: serde_json::Value = serde_json::json!({ "id": 123, "name": "测试" });
        let t = parse_netease_song(&v).expect("有 id/name 应解析成功");
        assert_eq!(t.id, "123");
        assert_eq!(t.artist, "未知艺人");
        assert_eq!(t.duration, 0);
        assert_eq!(t.source, "netease");
    }

    #[test]
    fn parse_netease_song_rejects_missing_id() {
        let v: serde_json::Value = serde_json::json!({ "name": "无 id" });
        assert!(parse_netease_song(&v).is_none());
    }

    #[test]
    fn parse_netease_song_maps_duration_and_fee() {
        let v: serde_json::Value =
            serde_json::json!({ "id": 1, "name": "a", "duration": 234567, "fee": 8 });
        let t = parse_netease_song(&v).unwrap();
        assert_eq!(t.duration, 234);
        assert_eq!(t.fee, 8);
    }

    #[test]
    fn parse_lyric_json_takes_lrc_and_translation() {
        let v: serde_json::Value = serde_json::json!({
            "lrc": { "lyric": "[00:01.00]hello\n" },
            "tlyric": { "lyric": "[00:01.00]你好\n" },
        });
        let (lrc, trans) = parse_lyric_json(&v).expect("有 lrc 应解析成功");
        assert_eq!(lrc, "[00:01.00]hello");
        assert_eq!(trans.as_deref(), Some("[00:01.00]你好"));
    }

    #[test]
    fn parse_lyric_json_empty_lrc_is_none() {
        let v: serde_json::Value = serde_json::json!({ "lrc": { "lyric": "  \n" } });
        assert!(parse_lyric_json(&v).is_none());
        assert!(parse_lyric_json(&serde_json::json!({})).is_none());
    }

    #[test]
    fn parse_lyric_json_translation_optional() {
        let v: serde_json::Value = serde_json::json!({ "lrc": { "lyric": "[00:01.00]hi" } });
        let (_, trans) = parse_lyric_json(&v).expect("lrc 存在");
        assert!(trans.is_none());
        let v2: serde_json::Value =
            serde_json::json!({ "lrc": { "lyric": "x" }, "tlyric": { "lyric": " " } });
        assert!(parse_lyric_json(&v2).expect("lrc 存在").1.is_none());
    }

    #[test]
    fn parse_pic_url_upgrades_to_https() {
        let v: serde_json::Value = serde_json::json!({
            "songs": [{ "album": { "picUrl": "http://p1.music.126.net/abc/1.jpg" } }]
        });
        assert_eq!(
            parse_pic_url(&v).as_deref(),
            Some("https://p1.music.126.net/abc/1.jpg")
        );
    }

    #[test]
    fn parse_pic_url_missing_is_none() {
        assert!(parse_pic_url(&serde_json::json!({})).is_none());
        assert!(parse_pic_url(&serde_json::json!({ "songs": [] })).is_none());
        let empty: serde_json::Value =
            serde_json::json!({ "songs": [{ "album": { "picUrl": "" } }] });
        assert!(parse_pic_url(&empty).is_none());
    }

    #[test]
    fn cover_url_pins_512_square() {
        assert!(cover_url_netease_stub(None).is_none());
        let u = cover_url_netease_stub(Some("https://p1.music.126.net/abc/1.jpg?param=300y300"));
        assert_eq!(
            u.as_deref(),
            Some("https://p1.music.126.net/abc/1.jpg?param=512y512")
        );
    }

    #[test]
    fn media_url_allows_only_netease_https_hosts() {
        assert!(restrict_media_url("netease", "http://m7.music.126.net/a/1.mp3").is_ok());
        assert!(
            restrict_media_url("netease", "https://music.163.com/song/media/outer/url?id=1")
                .is_ok()
        );
        assert!(restrict_media_url("netease", "https://evil.example/a.mp3").is_err());
        assert!(restrict_media_url("netease", "https://music.126.net.evil.example/a.mp3").is_err());
        assert!(restrict_media_url("netease", "ftp://m7.music.126.net/a.mp3").is_err());
        assert!(restrict_media_url("netease", "https://m7.music.126.net:8443/a.mp3").is_err());
        assert!(restrict_media_url("netease", "https://user@m7.music.126.net/a.mp3").is_err());
        assert!(restrict_media_url("netease", "https://m7.music.126.net/").is_err());
    }

    #[test]
    fn media_url_allows_provider_cdn_families() {
        assert!(restrict_media_url("qq", "https://isure.stream.qqmusic.qq.com/a.m4a").is_ok());
        assert!(restrict_media_url("kugou", "http://fsandroid.tx.kugou.com/a.mp3").is_ok());
        assert!(restrict_media_url("kuwo", "https://kw-bj.kuwo.cn/a.mp3").is_ok());
        // 跨来源域名不能互相放行
        assert!(restrict_media_url("qq", "https://kw-bj.kuwo.cn/a.mp3").is_err());
        assert!(restrict_media_url("kugou", "https://isure.stream.qqmusic.qq.com/a.m4a").is_err());
        assert!(restrict_media_url("kuwo", "https://evil.example/a.mp3").is_err());
    }

    #[test]
    fn media_url_upgrades_http_to_https() {
        let u = restrict_media_url("netease", "http://m7.music.126.net/a/1.mp3").unwrap();
        assert_eq!(u, "https://m7.music.126.net/a/1.mp3");
    }

    #[test]
    fn id_must_match_provider_shape() {
        assert!(validate_id("netease", "123456").is_ok());
        assert!(validate_id("netease", "../escape").is_err());
        assert!(validate_id("qq", "001yS0N33yPm1B").is_ok());
        assert!(validate_id("qq", "../escape").is_err());
        assert!(validate_id("kugou", "EFEC0563692CAF94C184248B1D83227F").is_ok());
        assert!(validate_id("kugou", "../escape").is_err());
        assert!(validate_id("kuwo", "51685512").is_ok());
        assert!(validate_id("kuwo", "MUSIC_1").is_err());
    }

    #[test]
    fn online_id_prefixes_roundtrip() {
        assert!(is_online_id("net:123"));
        assert!(is_online_id("qq:001yS0N33yPm1B"));
        assert!(is_online_id("kugou:EFEC0563692CAF94C184248B1D83227F"));
        assert!(is_online_id("kuwo:51685512"));
        assert!(!is_online_id("deadbeef"));
        assert_eq!(split_online_id("qq:abc"), Some(("qq", "abc")));
        assert_eq!(split_online_id("net:1"), Some(("netease", "1")));
    }

    #[test]
    fn parse_qq_song_maps_free_and_cover() {
        let v: serde_json::Value = serde_json::json!({
            "mid": "001yS0N33yPm1B",
            "name": "海阔天空",
            "interval": 324,
            "singer": [{"name": "BEYOND"}],
            "album": {"name": "乐与怒", "mid": "002qcJuX3lO3EZ"},
            "pay": {"pay_play": 0}
        });
        let t = parse_qq_song(&v).unwrap();
        assert_eq!(t.source, "qq");
        assert_eq!(t.fee, 0);
        assert_eq!(t.duration, 324);
        assert!(t
            .cover
            .unwrap()
            .contains("T002R500x500M000002qcJuX3lO3EZ.jpg"));
    }

    #[test]
    fn parse_qq_song_marks_paid() {
        let v: serde_json::Value = serde_json::json!({
            "mid": "001yS0N33yPm1B",
            "name": "x",
            "singer": [],
            "album": {},
            "pay": {"pay_play": 1}
        });
        assert_eq!(parse_qq_song(&v).unwrap().fee, 1);
    }

    #[test]
    fn parse_kugou_song_maps_hash_cover_and_fee() {
        let v: serde_json::Value = serde_json::json!({
            "FileName": "BEYOND - 海阔天空",
            "SingerName": "BEYOND",
            "FileHash": "EFEC0563692CAF94C184248B1D83227F",
            "AlbumName": "海阔天空",
            "Duration": 316,
            "Privilege": 8,
            "PayType": 3,
            "Image": "http://imge.kugou.com/stdmusic/{size}/x.jpg"
        });
        let t = parse_kugou_song(&v).unwrap();
        assert_eq!(t.id, "EFEC0563692CAF94C184248B1D83227F");
        assert_eq!(t.name, "海阔天空");
        assert_eq!(t.fee, 0);
        assert_eq!(t.duration, 316);
        assert_eq!(
            t.cover.as_deref(),
            Some("https://imge.kugou.com/stdmusic/480/x.jpg")
        );
    }

    #[test]
    fn parse_kugou_song_marks_vip() {
        let v: serde_json::Value = serde_json::json!({
            "FileHash": "EFEC0563692CAF94C184248B1D83227F",
            "FileName": "a",
            "SingerName": "b",
            "Privilege": 10
        });
        assert_eq!(parse_kugou_song(&v).unwrap().fee, 1);
    }

    #[test]
    fn kugou_tracker_key_matches_known_vector() {
        assert_eq!(
            kugou_tracker_key("efec0563692caf94c184248b1d83227f"),
            "17721ce5a7609f8811549742925fc76d"
        );
    }

    #[test]
    fn parse_kuwo_song_maps_free_and_paid() {
        let free: serde_json::Value = serde_json::json!({
            "MUSICRID": "MUSIC_51685512",
            "SONGNAME": "晴天&nbsp;(KTV版伴奏)",
            "ARTIST": "周杰伦",
            "DURATION": "263",
            "PAY": "0"
        });
        let t = parse_kuwo_song(&free).unwrap();
        assert_eq!(t.id, "51685512");
        assert_eq!(t.fee, 0);
        assert_eq!(t.duration, 263);
        assert!(t.name.contains(' '));
        assert!(kuwo_is_free("51685512"));
        let paid: serde_json::Value = serde_json::json!({
            "MUSICRID": "MUSIC_226543302",
            "SONGNAME": "最伟大的作品",
            "ARTIST": "周杰伦",
            "DURATION": "255",
            "PAY": "16711935"
        });
        assert_eq!(parse_kuwo_song(&paid).unwrap().fee, 1);
        assert!(!kuwo_is_free("226543302"));
    }

    #[test]
    fn kuwo_play_requires_search_whitelist() {
        // 未在搜索结果里出现过的 id(尤其是 VIP)一律拒绝,anti.s 本身不做付费判断
        assert!(!kuwo_is_free("226543302"));
        assert!(play_url_kuwo("226543302").is_err());
        assert!(play_url_kuwo("not-a-number").is_err());
    }

    #[test]
    fn cover_url_allows_provider_hosts() {
        assert!(restrict_cover_url("qq", "https://y.gtimg.cn/a.jpg").is_ok());
        assert!(restrict_cover_url("kugou", "https://imge.kugou.com/a.jpg").is_ok());
        assert!(restrict_cover_url("qq", "https://imge.kugou.com/a.jpg").is_err());
        assert!(restrict_cover_url("kugou", "https://y.gtimg.cn/a.jpg").is_err());
    }

    /// 尺寸归一逻辑的纯函数化测试桩:与 cover_url_netease 内的 map 分支一致
    fn cover_url_netease_stub(url: Option<&str>) -> Option<String> {
        url.and_then(|u| {
            let base = u.split('?').next().unwrap_or(u);
            restrict_cover_url("netease", &format!("{base}?param=512y512")).ok()
        })
    }
}
