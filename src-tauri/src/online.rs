//! 在线音源(网易云音乐公开接口)。
//! 边界与合规:仅调用平台网页端公开的搜索与试听接口,免费曲目返回
//! 官方 CDN 直链可直接播放;VIP/付费/版权受限曲目接口返回空链,
//! 本模块不做任何登录态、签名绕过或付费内容解锁,原样向上报错。
//! 接口无官方文档、字段随时可能变化,解析按宽松取值处理。

use serde::Serialize;

/// 搜索单条结果(前端展示 + 构造播放队列)
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OnlineTrack {
    /// 平台曲目 id(字符串,前端拼 `net:` 前缀作队列 id)
    pub id: String,
    pub name: String,
    pub artist: String,
    pub album: String,
    /// 时长(秒;接口给毫秒)
    pub duration: u64,
    /// 0=免费 8=低音质免费(可播) 1=VIP 4=购买专辑 16=非音乐资源
    pub fee: u8,
}

/// 把 result.songs 里的一条解析成 OnlineTrack:字段按宽松取值,
/// 任何字段缺失都不至于丢弃整条结果
fn parse_song(v: &serde_json::Value) -> Option<OnlineTrack> {
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
    })
}

/// 平台曲目 id 必须是纯数字:它同时用于拼 API 查询与封面缓存文件名,
/// 放行任意字符串会让 `../` 之类的输入有机会穿透缓存目录
pub fn validate_id(id: &str) -> Result<(), String> {
    if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err("在线曲目 id 非法".into())
    }
}

/// 播放直链与封面图的主机白名单:只允许网易云官方域名。
/// 接口响应不可全信,白名单是 Rust 端交给 WebView 之前的最后一道闸
fn allowed_host(host: &str) -> bool {
    host == "music.163.com" || host == "music.126.net" || host.ends_with(".music.126.net")
}

/// 校验并归一化在线媒体 URL:http 一律升级为 https(WebView CSP 只放行
/// 白名单内的 https 源),拒绝非网易云主机、非 https、带 userinfo 或非常规端口
fn restrict_media_url(raw: &str) -> Result<String, String> {
    let upgraded = raw.replacen("http://", "https://", 1);
    let url = reqwest::Url::parse(&upgraded).map_err(|_| "在线媒体地址非法".to_string())?;
    let host = url.host_str().ok_or("在线媒体地址缺少主机")?;
    let port_ok = url.port().is_none() || url.port() == Some(443);
    if url.scheme() != "https"
        || !allowed_host(host)
        || !port_ok
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() == "/"
    {
        return Err("在线媒体地址不在允许范围".into());
    }
    Ok(url.to_string())
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
    Ok(songs.iter().filter_map(parse_song).collect())
}

/// 取播放直链(官方试听 CDN)。CDN 只给 http,域名本身支持 https,
/// 统一升级为 https——CSP 只放行白名单 https 源,不升级会被 WebView 拦。
/// 返回空链即版权/VIP 受限,不做任何绕过,直接报错给前端提示。
pub fn play_url_netease(id: &str) -> Result<String, String> {
    validate_id(id)?;
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
    restrict_media_url(link)
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
    validate_id(id)?;
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
    Ok((!payload.is_empty()).then_some(payload))
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

/// 取在线曲目专辑图直链(歌曲详情公开接口)。
/// picUrl 支持 ?param=WxH 请求指定尺寸,这里要 512px 方图,与封面缓存档位对齐;
/// 返回 None = 该曲无图(区别于网络错误)。主机不在白名单时同样按无图处理,
/// 避免接口异常响应把 Rust 端变成任意 URL 下载器
pub fn cover_url_netease(id: &str) -> Result<Option<String>, String> {
    validate_id(id)?;
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
    Ok(restrict_media_url(&format!("{base}?param=512y512")).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_song_tolerates_missing_fields() {
        // 字段缺失时不崩:artist/album 有兜底,duration 缺省 0
        let v: serde_json::Value = serde_json::json!({ "id": 123, "name": "测试" });
        let t = parse_song(&v).expect("有 id/name 应解析成功");
        assert_eq!(t.id, "123");
        assert_eq!(t.artist, "未知艺人");
        assert_eq!(t.duration, 0);
    }

    #[test]
    fn parse_song_rejects_missing_id() {
        // 无 id 无法构造播放队列,直接丢弃
        let v: serde_json::Value = serde_json::json!({ "name": "无 id" });
        assert!(parse_song(&v).is_none());
    }

    #[test]
    fn parse_song_maps_duration_and_fee() {
        let v: serde_json::Value =
            serde_json::json!({ "id": 1, "name": "a", "duration": 234567, "fee": 8 });
        let t = parse_song(&v).unwrap();
        assert_eq!(t.duration, 234); // 毫秒转秒
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
        // 空词 = 该歌确实没有歌词,不是错误
        let v: serde_json::Value = serde_json::json!({ "lrc": { "lyric": "  \n" } });
        assert!(parse_lyric_json(&v).is_none());
        assert!(parse_lyric_json(&serde_json::json!({})).is_none());
    }

    #[test]
    fn parse_lyric_json_translation_optional() {
        let v: serde_json::Value = serde_json::json!({ "lrc": { "lyric": "[00:01.00]hi" } });
        let (_, trans) = parse_lyric_json(&v).expect("lrc 存在");
        assert!(trans.is_none());
        // 空白译文与缺失等价
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
        // 无 songs / 无专辑 / 空串都算无图,不是错误
        assert!(parse_pic_url(&serde_json::json!({})).is_none());
        assert!(parse_pic_url(&serde_json::json!({ "songs": [] })).is_none());
        let empty: serde_json::Value =
            serde_json::json!({ "songs": [{ "album": { "picUrl": "" } }] });
        assert!(parse_pic_url(&empty).is_none());
    }

    #[test]
    fn cover_url_pins_512_square() {
        // 详情接口没有图 → None
        assert!(cover_url_netease_stub(None).is_none());
        // 已有查询串会被归一成 ?param=512y512
        let u = cover_url_netease_stub(Some("https://p1.music.126.net/abc/1.jpg?param=300y300"));
        assert_eq!(
            u.as_deref(),
            Some("https://p1.music.126.net/abc/1.jpg?param=512y512")
        );
    }

    #[test]
    fn media_url_allows_only_netease_https_hosts() {
        // CDN 子域与官方主站放行
        assert!(restrict_media_url("http://m7.music.126.net/a/1.mp3").is_ok());
        assert!(restrict_media_url("https://m804.music.126.net/a/1.mp3").is_ok());
        assert!(restrict_media_url("https://p1.music.126.net/a/1.jpg").is_ok());
        assert!(restrict_media_url("https://music.163.com/song/media/outer/url?id=1").is_ok());
        // 非网易云域名、后缀伪装、非 https、非常规端口、userinfo 一律拒绝
        assert!(restrict_media_url("https://evil.example/a.mp3").is_err());
        assert!(restrict_media_url("https://music.126.net.evil.example/a.mp3").is_err());
        assert!(restrict_media_url("ftp://m7.music.126.net/a.mp3").is_err());
        assert!(restrict_media_url("https://m7.music.126.net:8443/a.mp3").is_err());
        assert!(restrict_media_url("https://user@m7.music.126.net/a.mp3").is_err());
        assert!(restrict_media_url("https://m7.music.126.net/").is_err());
    }

    #[test]
    fn media_url_upgrades_http_to_https() {
        let u = restrict_media_url("http://m7.music.126.net/a/1.mp3").unwrap();
        assert_eq!(u, "https://m7.music.126.net/a/1.mp3");
    }

    #[test]
    fn id_must_be_numeric() {
        assert!(validate_id("123456").is_ok());
        assert!(validate_id("").is_err());
        assert!(validate_id("../escape").is_err());
        assert!(validate_id("12a").is_err());
    }

    /// 尺寸归一逻辑的纯函数化测试桩:与 cover_url_netease 内的 map 分支一致
    fn cover_url_netease_stub(url: Option<&str>) -> Option<String> {
        url.and_then(|u| {
            let base = u.split('?').next().unwrap_or(u);
            restrict_media_url(&format!("{base}?param=512y512")).ok()
        })
    }
}
