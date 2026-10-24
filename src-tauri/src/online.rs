//! 在线音源(网易云 / QQ 音乐 / 酷狗 / 酷我公开接口)。
//! 合规边界:仅调用各平台网页端公开的搜索与试听接口,免费曲目返回官方
//! CDN 直链可直接播放;VIP/付费/版权受限曲目不做登录态、签名绕过或付费
//! 内容解锁,原样向上报错。
//! 各家付费闸门不同:网易云/QQ/酷狗由接口自身对 VIP 返回空链;酷我 anti.s
//! 对 VIP 也会返回直链,因此酷我只允许本次会话搜索结果里 PAY=0 的曲目取链。
//! 音质只取匿名免费上限:网易云 br=320000(无 320k 源时接口自动降级)、
//! 酷狗走搜索结果里的 HQFileHash;QQ/酷我匿名最高就是 128k,不再试探付费档。
//! 接口无官方文档、字段随时可能变化,解析按宽松取值处理。

use base64::Engine as _;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
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

// ===== HTTP 客户端统一构造 =====

/// 模拟 Chrome 浏览器 UA:部分平台接口对非浏览器 UA 拒答或返回降级结果
pub(crate) const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// 公开曲库接口(LRCLIB / iTunes)的礼貌标识 UA:无反爬门槛,如实亮明应用身份
pub(crate) const APP_UA: &str = concat!(
    "TauriMusic/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/OArt3misQAQ/TauriMusic)"
);

/// 各在线客户端的差异项:超时 / UA / 默认头 / 重定向策略。
/// 差异集中声明,构造收敛在 build_http_client,防止 UA 与超时配置在
/// 多个调用点各自复制后悄悄漂移
pub(crate) struct HttpClientSpec {
    /// 整请求超时(连接+读体);下载整首歌单独用 300s 长档
    pub(crate) timeout: std::time::Duration,
    pub(crate) ua: &'static str,
    pub(crate) referer: Option<&'static str>,
    pub(crate) cookie: Option<&'static str>,
    /// false = 禁止跟随 30x:封面/直链抓取不能被 30x 带去白名单外或内网地址
    pub(crate) follow_redirects: bool,
}

/// 在线 HTTP 客户端统一构造器。
/// builder 失败只可能源于 TLS 后端初始化异常,此时 Client::new() 同样会 panic
/// 却还丢了超时配置(静默挂死 spawn_blocking 线程),不如首次调用时显式失败
pub(crate) fn build_http_client(spec: HttpClientSpec) -> reqwest::blocking::Client {
    let mut builder = reqwest::blocking::Client::builder()
        .user_agent(spec.ua)
        .timeout(spec.timeout);
    if !spec.follow_redirects {
        builder = builder.redirect(reqwest::redirect::Policy::none());
    }
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(r) = spec.referer {
        headers.insert(
            reqwest::header::REFERER,
            r.parse().expect("预设 Referer 常量必然可解析"),
        );
    }
    if let Some(c) = spec.cookie {
        headers.insert(
            reqwest::header::COOKIE,
            c.parse().expect("预设 Cookie 常量必然可解析"),
        );
    }
    if !headers.is_empty() {
        builder = builder.default_headers(headers);
    }
    builder
        .build()
        .expect("在线 HTTP 客户端构造失败(TLS 后端初始化异常)")
}

fn qq_client() -> reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            build_http_client(HttpClientSpec {
                timeout: std::time::Duration::from_secs(15),
                ua: BROWSER_UA,
                referer: Some("https://y.qq.com/"),
                cookie: None,
                follow_redirects: true,
            })
        })
        .clone()
}

fn kugou_client() -> reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            build_http_client(HttpClientSpec {
                timeout: std::time::Duration::from_secs(15),
                ua: BROWSER_UA,
                referer: Some("https://www.kugou.com/"),
                cookie: None,
                follow_redirects: true,
            })
        })
        .clone()
}

fn kuwo_client() -> reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            build_http_client(HttpClientSpec {
                timeout: std::time::Duration::from_secs(15),
                ua: BROWSER_UA,
                referer: Some("https://www.kuwo.cn/"),
                cookie: None,
                follow_redirects: true,
            })
        })
        .clone()
}

/// 网易云接口客户端:须带 os=pc Cookie,否则部分接口按移动网页端降级返回
/// (lib.rs 根重导出,examples/make_tmc 的在线匹配调试入口也用它)
pub fn netease_client() -> reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            build_http_client(HttpClientSpec {
                timeout: std::time::Duration::from_secs(15),
                ua: BROWSER_UA,
                referer: Some("https://music.163.com"),
                cookie: Some("os=pc; appver=2.9.7"),
                follow_redirects: true,
            })
        })
        .clone()
}

// ===== 传输层错误与状态码统一文案 =====

/// 传输层错误统一转译:reqwest 原始错误串(带 URL/IO 细节)对用户不可操作,
/// 超时单列,让用户能区分「网络不通」与「对端过慢」
pub(crate) fn net_err(label: &str, e: &reqwest::Error) -> String {
    net_err_text(label, e.is_timeout())
}

/// 超时与断连的文案映射(拆出纯函数便于单测)
fn net_err_text(label: &str, timeout: bool) -> String {
    if timeout {
        format!("{label}: 请求超时,请检查网络后重试")
    } else {
        format!("{label}: 网络异常,请检查网络后重试")
    }
}

/// 非 2xx 的统一文案(拆出纯函数便于单测):
/// 限流/反爬通常直接回 403/502,必须按状态码拦下而不是硬解析错误页
fn api_status_err(label: &str, code: u16) -> String {
    format!("{label}接口异常(HTTP {code}),请稍后再试")
}

/// 平台接口 JSON 响应统一收口:非 2xx 直接报「接口异常(HTTP 码)」,
/// 不再盲目 resp.json()——HTML 错误页硬解析只会得到
/// 「expected value at line 1」这类不可操作的报错。
/// label 为「平台+动作」短语(如「酷狗搜索」),用于拼装两类错误文案
fn api_json(resp: reqwest::blocking::Response, label: &str) -> Result<serde_json::Value, String> {
    let status = resp.status();
    if !status.is_success() {
        return Err(api_status_err(label, status.as_u16()));
    }
    resp.json::<serde_json::Value>()
        .map_err(|_| format!("{label}响应解析失败"))
}

// ===== 跨源标签归一化与字段解析(lib.rs 在线匹配链共用,全部纯函数) =====

/// 繁体转简体(进程级单例转换器):normalize_tag 与现场版识别共用
pub(crate) fn to_simplified(s: &str) -> String {
    use character_converter::CharacterConverter;
    static CONVERTER: std::sync::OnceLock<CharacterConverter> = std::sync::OnceLock::new();
    CONVERTER
        .get_or_init(CharacterConverter::new)
        .traditional_to_simplified(s)
}

/// 归一化标签用于跨源比对:繁体转简体 + 小写 + 去空白(本地标签常见繁体/大小写差异)
pub(crate) fn normalize_tag(s: &str) -> String {
    to_simplified(s)
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// 归一化后的双向包含比对
pub(crate) fn name_matches(a: &str, b: &str) -> bool {
    let (a, b) = (normalize_tag(a), normalize_tag(b));
    !a.is_empty() && !b.is_empty() && (a.contains(&b) || b.contains(&a))
}

/// 艺人组任一分段与候选名比对成功即通过
pub(crate) fn artist_matches(artist: &str, candidate: &str) -> bool {
    artist
        .split(['/', '&', ',', '、'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .any(|seg| name_matches(seg, candidate))
}

/// 标题是否带现场/演唱会标记:live 需独立成词(避免 Alive/Livin' 误判),
/// 中文无词边界直接子串判。现场版与录音室版的时间轴完全不同,选错版歌词必错位
pub(crate) fn title_is_live(s: &str) -> bool {
    let norm = to_simplified(s).to_lowercase();
    if norm.contains("现场") || norm.contains("演唱会") || norm.contains("实况") {
        return true;
    }
    norm.split(|c: char| !c.is_ascii_alphabetic())
        .any(|w| w == "live")
}

/// 去掉标题里的括号段(全角/半角):酷我搜索对括号敏感,
/// 「八千里路明月照（挥笔生花问桀骜）」整串搜索命中不了原曲,去掉后能精确命中
pub(crate) fn strip_bracketed(s: &str) -> String {
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
pub(crate) fn kuwo_keywords(title: &str, artist: &str) -> Vec<String> {
    let mut keys = vec![format!("{title} {artist}")];
    let stripped = strip_bracketed(title);
    if !stripped.is_empty() && stripped != title {
        keys.push(format!("{stripped} {artist}"));
    }
    keys
}

/// 酷我字段里的 HTML 实体解码(&apos; &nbsp; &amp; 等,数字实体一并处理)
pub(crate) fn html_unescape(s: &str) -> String {
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

/// 酷我 r.s 返回单引号伪 JSON,转成严格 JSON:
/// 仅当引号紧邻结构性字符(前一字符为 {[,: 或后一字符为 }],:)时视为定界符,
/// 值内部的撇号(Don't)原样保留;找不到定界符时解析会失败并走优雅降级
pub(crate) fn kuwo_fix_json(s: &str) -> String {
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

/// 把酷我 lrclist(time 为秒字符串)拼成标准 LRC 文本
pub(crate) fn build_lrc_from_kuwo(lines: &[serde_json::Value]) -> Option<String> {
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
    let client = netease_client();
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
        .map_err(|e| net_err("网易云搜索请求失败", &e))?;
    let v = api_json(resp, "网易云搜索")?;
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

/// 网易云 `br` 参数固定为匿名可取的最高免费档 320k(level=exhigh)。
/// 再高(640k/999k,即无损/Hi-Res)对匿名请求一律被接口截回 320k,
/// 试探更高档没有意义,也可能把请求语义带到付费文件上,故封顶 320000。
const NETEASE_BR: &str = "320000";

/// 取网易云播放直链(官方试听 CDN)。CDN 只给 http,域名本身支持 https,
/// 统一升级为 https——CSP 只放行白名单 https 源,不升级会被 WebView 拦。
/// 返回空链即版权/VIP 受限,不做任何绕过,直接报错给前端提示。
pub fn play_url_netease(id: &str) -> Result<String, String> {
    validate_id("netease", id)?;
    let client = netease_client();
    let url = "https://music.163.com/api/song/enhance/player/url";
    let resp = client
        .get(url)
        .query(&[("ids", format!("[{id}]").as_str()), ("br", NETEASE_BR)])
        .send()
        .map_err(|e| net_err("网易云取链请求失败", &e))?;
    let v = api_json(resp, "网易云取链")?;
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
    let client = netease_client();
    let resp = client
        .get("https://music.163.com/api/song/lyric")
        .query(&[("id", id), ("lv", "1"), ("kv", "1"), ("tv", "-1")])
        .send()
        .map_err(|e| net_err("网易云歌词请求失败", &e))?;
    let v = api_json(resp, "网易云歌词")?;
    Ok(parse_lyric_json(&v))
}

/// 取网易云歌词并解析为播放器歌词载荷:
/// 外语歌自动对齐官方译文(tlyric),对齐率不足八成时宁缺毋滥只保留原文——
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

/// 网易云 picUrl 尺寸归一:?param=WxH 会整体替换服务端出图尺寸,
/// 已带的旧参数必须剥掉再拼 512 方图(与封面缓存档位对齐)。
/// 纯函数,生产与单测共用同一份实现
fn netease_cover_512(raw: &str) -> String {
    let base = raw.split('?').next().unwrap_or(raw);
    format!("{base}?param=512y512")
}

/// 取网易云专辑图直链(歌曲详情公开接口)。
/// picUrl 支持 ?param=WxH 请求指定尺寸,这里要 512px 方图,与封面缓存档位对齐;
/// 返回 None = 该曲无图(区别于网络错误)。主机不在白名单时同样按无图处理,
/// 避免接口异常响应把 Rust 端变成任意 URL 下载器
pub fn cover_url_netease(id: &str) -> Result<Option<String>, String> {
    validate_id("netease", id)?;
    let client = netease_client();
    let url = format!("https://music.163.com/api/song/detail/?id=[{id}]&ids=[{id}]");
    let resp = client
        .get(url)
        .send()
        .map_err(|e| net_err("网易云歌曲详情请求失败", &e))?;
    let v = api_json(resp, "网易云歌曲详情")?;
    let Some(raw) = parse_pic_url(&v) else {
        return Ok(None);
    };
    // URL 可能已带查询串,统一追加(或替换)param 为 512px 方图
    Ok(restrict_cover_url("netease", &netease_cover_512(&raw)).ok())
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
        .map_err(|e| net_err("QQ 搜索请求失败", &e))?;
    let v = api_json(resp, "QQ 搜索")?;
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
/// 直链统一从 isure.stream.qqmusic.qq.com 拼出(其 sip 列表含该 https 主机)。
/// `M500`(128k mp3)之外,`M800`/`S000` 等更高档对匿名请求一律空 purl,
/// 所以接口默认给的那条就是 QQ 的最高免费档
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
        .map_err(|e| net_err("QQ 取链请求失败", &e))?;
    let v = api_json(resp, "QQ 取链")?;
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
        .map_err(|e| net_err("QQ 歌词请求失败", &e))?;
    let v = api_json(resp, "QQ 歌词")?;
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
    // HQPrivilege 是独立的一档:存在"整首免费但 320k 收费"的曲目(如部分翻唱),
    // 只有接口判定 HQ 也免费时才记下 hash,取链时才有 320k 可走
    if fee == 0 && v.get("HQPrivilege").and_then(|p| p.as_u64()).unwrap_or(10) < 10 {
        if let Some(hq) = v.get("HQFileHash").and_then(|x| x.as_str()) {
            remember_kugou_hq(&id, hq);
        }
    }
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
        .map_err(|e| net_err("酷狗搜索请求失败", &e))?;
    let v = api_json(resp, "酷狗搜索")?;
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

/// 酷狗的码率档位由 **hash 决定**,`br` 参数并不能升档:搜索结果里
/// `FileHash` 是 128k、`HQFileHash` 才是 320k。HQ hash 只在搜索时拿得到,
/// 且只记 `HQPrivilege < 10`(接口自己判定 HQ 免费)的那批,与酷我白名单同理。
/// key 为小写的标准 hash,play_url_kugou 用同一个小写形式查表。
static KUGOU_HQ_HASHES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
const KUGOU_HQ_CAP: usize = 4096;

fn remember_kugou_hq(std_hash: &str, hq_hash: &str) {
    if !kugou_id_ok(hq_hash) {
        return;
    }
    let map = KUGOU_HQ_HASHES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut g) = map.lock() {
        if g.len() >= KUGOU_HQ_CAP {
            g.clear();
        }
        g.insert(std_hash.to_ascii_lowercase(), hq_hash.to_string());
    }
}

fn kugou_hq_hash(std_hash: &str) -> Option<String> {
    KUGOU_HQ_HASHES
        .get()
        .and_then(|m| m.lock().ok())
        .and_then(|g| g.get(&std_hash.to_ascii_lowercase()).cloned())
}

/// 按 hash 向 tracker 取直链:受限曲目接口回 status != 1,不做任何绕过
fn kugou_tracker_url(hash_lower: &str) -> Result<String, String> {
    let key = kugou_tracker_key(hash_lower);
    let resp = kugou_client()
        .get("http://trackercdn.kugou.com/i/v2/")
        .query(&[
            ("key", key.as_str()),
            ("hash", hash_lower),
            ("br", "hq"),
            ("appid", "1005"),
            ("pid", "2"),
            ("cmd", "25"),
            ("behavior", "play"),
        ])
        .send()
        .map_err(|e| net_err("酷狗取链请求失败", &e))?;
    let v = api_json(resp, "酷狗取链")?;
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

/// 取酷狗播放直链:优先走搜索时记下的 HQ hash(320k),接口拒绝(HQ 单独收费或
/// 结果已过期)时回落到标准 hash——高音质是上限而不是硬性条件,
/// 不该因为拿不到 320k 就把本来能播的曲目判死
pub fn play_url_kugou(id: &str) -> Result<String, String> {
    validate_id("kugou", id)?;
    let std_hash = id.to_ascii_lowercase();
    if let Some(hq) = kugou_hq_hash(&std_hash) {
        if let Ok(url) = kugou_tracker_url(&hq.to_ascii_lowercase()) {
            return Ok(url);
        }
    }
    kugou_tracker_url(&std_hash)
}

/// 从歌词候选里取 (id, accesskey)。任一缺失按无词处理返回 None:
/// 接口改版出坏候选时若按 Err 上抛,get_lyrics 会按「失败不缓存」处理,
/// 每次切歌都重打两跳接口,与全模块的宽松取值约定相悖
fn kugou_candidate_key(cand: &serde_json::Value) -> Option<(&str, &str)> {
    Some((cand.get("id")?.as_str()?, cand.get("accesskey")?.as_str()?))
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
        .map_err(|e| net_err("酷狗歌词搜索失败", &e))?;
    let v = api_json(resp, "酷狗歌词搜索")?;
    let Some(cand) = v
        .get("candidates")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
    else {
        return Ok(None);
    };
    let Some((lyric_id, accesskey)) = kugou_candidate_key(cand) else {
        return Ok(None);
    };
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
        .map_err(|e| net_err("酷狗歌词下载失败", &e))?;
    let v = api_json(resp, "酷狗歌词下载")?;
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

/// 酷我 r.s 搜索统一实现(在线搜索与本 crate 的本地匹配链共用):
/// 返回 abslist 数组;单引号伪 JSON 先经 kuwo_fix_json 修复再解析。
/// 两条调用链的参数分叉(client 档 / vipver / 每页条数)是历史现状,
/// 这里只收敛代码不改变线上行为:在线搜索走 kt/30,本地匹配走 mp+vipver/10
pub(crate) fn kuwo_rs_search(
    client: &reqwest::blocking::Client,
    keyword: &str,
    page: u32,
    page_size: u32,
    client_tag: &str,
    vipver: Option<&str>,
) -> Result<Vec<serde_json::Value>, String> {
    let mut query = vec![
        ("all", keyword.to_string()),
        ("ft", "music".to_string()),
        ("itemset", "web_2013".to_string()),
        ("client", client_tag.to_string()),
        ("pn", page.to_string()),
        ("rn", page_size.to_string()),
        ("rformat", "json".to_string()),
        ("encoding", "utf8".to_string()),
    ];
    if let Some(v) = vipver {
        query.push(("vipver", v.to_string()));
    }
    let resp = client
        .get("https://search.kuwo.cn/r.s")
        .query(&query)
        .send()
        .map_err(|e| net_err("酷我搜索请求失败", &e))?;
    if !resp.status().is_success() {
        return Err(api_status_err("酷我搜索", resp.status().as_u16()));
    }
    let raw = resp
        .text()
        .map_err(|_| "酷我搜索响应读取失败".to_string())?;
    let v: serde_json::Value = serde_json::from_str(&kuwo_fix_json(&raw))
        .map_err(|_| "酷我搜索响应解析失败".to_string())?;
    Ok(v.get("abslist")
        .and_then(|l| l.as_array())
        .cloned()
        .unwrap_or_default())
}

/// 搜索酷我单曲(旧版公开搜索接口,走 kuwo_rs_search 的 kt/30 档)
pub fn search_kuwo(keyword: &str, page: u32) -> Result<Vec<OnlineTrack>, String> {
    let keyword = keyword.trim();
    if keyword.is_empty() {
        return Ok(vec![]);
    }
    let list = kuwo_rs_search(&kuwo_client(), keyword, page, 30, "kt", None)?;
    Ok(list.iter().filter_map(parse_kuwo_song).collect())
}

/// 取酷我播放直链。anti.s 不做付费判断,因此必须先命中搜索结果白名单;
/// 未在本次会话搜索结果里出现过的 id 一律拒绝,防止绕过 VIP 限制。
/// 直链固定取免费完整文件:实测 anti.s 只给 128k 那份文件,塞 `rate`/`br`
/// 并不会升档(MINFO 里更高码率的 size 与实际下发文件对不上),
/// 且 `br` 反而会把请求换成 30 秒试听片段,故按原样取
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
        .map_err(|e| net_err("酷我取链请求失败", &e))?;
    // anti.s 成功时直接回 URL 纯文本,失败回错误页/空串:先按状态码拦一道,
    // 不看状态码会把错误页文本当成直链落进「酷我暂未返回可用直链」
    if !resp.status().is_success() {
        return Err(api_status_err("酷我取链", resp.status().as_u16()));
    }
    let link = resp
        .text()
        .map_err(|_| "酷我取链响应读取失败".to_string())?
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
        // 已带其他尺寸参数时整体替换为 512 方图
        assert_eq!(
            netease_cover_512("https://p1.music.126.net/abc/1.jpg?param=300y300"),
            "https://p1.music.126.net/abc/1.jpg?param=512y512"
        );
        // 无查询串的原样拼参数
        assert_eq!(
            netease_cover_512("https://p1.music.126.net/abc/1.jpg"),
            "https://p1.music.126.net/abc/1.jpg?param=512y512"
        );
        // 生产链路收口:归一后的 URL 仍要过封面主机白名单(http 一并升 https)
        let pinned = netease_cover_512("http://p1.music.126.net/abc/1.jpg?param=300y300");
        assert_eq!(
            restrict_cover_url("netease", &pinned).ok().as_deref(),
            Some("https://p1.music.126.net/abc/1.jpg?param=512y512")
        );
    }

    #[test]
    fn build_payload_merges_translation_when_alignment_is_high() {
        // 全部配对(≥八成):官方译文并进双语载荷
        let lrc = "[00:01.00]hello\n[00:02.00]world\n";
        let trans = "[00:01.00]你好\n[00:02.00]世界\n";
        let payload = build_payload(lrc.into(), Some(trans.into())).expect("合并后应有词");
        let synced = payload.synced.expect("应有同步行");
        assert_eq!(synced.len(), 2);
        assert_eq!(synced[0].translation.as_deref(), Some("你好"));
        assert_eq!(synced[1].translation.as_deref(), Some("世界"));
    }

    #[test]
    fn build_payload_discards_misaligned_translation() {
        // 译文时间轴整体错位(配对率 0 < 八成):宁缺毋滥,只保留原文
        let lrc = "[00:01.00]hello\n[00:02.00]world\n";
        let trans = "[01:30.00]你好\n[03:00.00]世界\n";
        let payload = build_payload(lrc.into(), Some(trans.into())).expect("原文仍在");
        let synced = payload.synced.expect("应有同步行");
        assert_eq!(synced.len(), 2);
        assert!(synced.iter().all(|l| l.translation.is_none()));
    }

    #[test]
    fn build_payload_passthrough_without_translation() {
        // 无译文时原文直通;空词解析为空载荷返回 None(播放页显示无词)
        let payload = build_payload("[00:01.00]hi".into(), None).expect("有词");
        assert_eq!(payload.synced.as_ref().unwrap()[0].text, "hi");
        assert!(build_payload(String::new(), None).is_none());
    }

    #[test]
    fn kugou_candidate_key_requires_id_and_accesskey() {
        let ok = serde_json::json!({ "id": "123", "accesskey": "abc" });
        assert_eq!(kugou_candidate_key(&ok), Some(("123", "abc")));
        // 接口改版出的坏候选按无词处理(可缓存),不再当作错误反复重试
        assert_eq!(
            kugou_candidate_key(&serde_json::json!({ "id": "123" })),
            None
        );
        assert_eq!(kugou_candidate_key(&serde_json::json!({})), None);
    }

    #[test]
    fn http_error_messages_are_actionable() {
        // 非 2xx:报平台与状态码,不再把 HTML 错误页硬解析成「expected value」
        assert_eq!(
            api_status_err("酷狗搜索", 502),
            "酷狗搜索接口异常(HTTP 502),请稍后再试"
        );
        // 传输层:超时与断连分开提示,不暴露 reqwest 原始错误串
        assert_eq!(
            net_err_text("酷狗搜索请求失败", true),
            "酷狗搜索请求失败: 请求超时,请检查网络后重试"
        );
        assert_eq!(
            net_err_text("QQ 搜索请求失败", false),
            "QQ 搜索请求失败: 网络异常,请检查网络后重试"
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
    fn kugou_records_hq_hash_only_when_hq_is_free() {
        // 整首免费且 HQ 也免费:记下 HQ hash,取链才可能走 320k
        let free_hq: serde_json::Value = serde_json::json!({
            "FileHash": "48C685F679FFC7CF08B8A8341CA9DB44",
            "HQFileHash": "28F83BBD8CAB043895039E98366AD2B4",
            "FileName": "x - 晴天",
            "SingerName": "x",
            "Privilege": 0,
            "HQPrivilege": 0
        });
        parse_kugou_song(&free_hq).unwrap();
        assert_eq!(
            kugou_hq_hash("48c685f679ffc7cf08b8a8341ca9db44").as_deref(),
            Some("28F83BBD8CAB043895039E98366AD2B4")
        );
        // 整首免费但 320k 单独收费(翻唱常见):不记,只能拿 128k
        let hq_paid: serde_json::Value = serde_json::json!({
            "FileHash": "0DF69FAEAEAC7358E4665BC7ACD72B5F",
            "HQFileHash": "7FC97AABCCA66129F3BCA012B81D544D",
            "FileName": "y - 晴天",
            "SingerName": "y",
            "Privilege": 8,
            "HQPrivilege": 10
        });
        parse_kugou_song(&hq_paid).unwrap();
        assert_eq!(kugou_hq_hash("0df69faeaeac7358e4665bc7acd72b5f"), None);
    }

    #[test]
    fn kugou_hq_hash_rejects_malformed_value() {
        // HQ hash 同样受 id 形状约束:非法值不进表,免得污染查询串与缓存文件名
        let std_hash = "11111111111111111111111111111111";
        remember_kugou_hq(std_hash, "../../etc/passwd");
        assert_eq!(kugou_hq_hash(std_hash), None);
        remember_kugou_hq(std_hash, "22222222222222222222222222222222");
        assert_eq!(
            kugou_hq_hash(std_hash).as_deref(),
            Some("22222222222222222222222222222222")
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

    #[test]
    fn kuwo_helpers_parse_pseudo_json_and_lrc() {
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
    fn strip_bracketed_removes_parenthetical_for_search() {
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
}
