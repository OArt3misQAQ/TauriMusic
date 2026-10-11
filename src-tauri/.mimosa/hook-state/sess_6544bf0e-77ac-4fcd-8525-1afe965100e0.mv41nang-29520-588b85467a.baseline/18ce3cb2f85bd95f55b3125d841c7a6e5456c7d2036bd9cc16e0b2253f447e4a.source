use encoding_rs::GBK;
use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use serde::Serialize;

use crate::model::Track;
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    pub time_ms: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsPayload {
    pub synced: Option<Vec<LyricLine>>,
    pub plain: Option<String>,
}

impl LyricsPayload {
    fn is_empty(&self) -> bool {
        self.synced.as_ref().is_none_or(|l| l.is_empty())
            && self.plain.as_deref().is_none_or(|p| p.trim().is_empty())
    }
}

/// 按曲目取歌词:优先同名 .lrc 文件,其次读取内嵌歌词标签。
pub fn load_for_track(track: &Track) -> Option<LyricsPayload> {
    if let Some(lrc) = &track.lrc_path {
        if let Ok(bytes) = std::fs::read(lrc) {
            let content = decode_bytes(&bytes);
            let payload = parse(&content);
            if !payload.is_empty() {
                return Some(payload);
            }
        }
    }

    let tagged = Probe::open(&track.path).ok()?.read().ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let text = tag
        .and_then(|t| t.get_string(&ItemKey::Lyrics))
        .map(fix_mojibake)?;
    let payload = parse(&text);
    if payload.is_empty() {
        None
    } else {
        Some(payload)
    }
}

/// 解析 LRC 文本。带时间戳的行进入 synced,其余非空行进入 plain。
pub fn parse(content: &str) -> LyricsPayload {
    let mut lines: Vec<LyricLine> = Vec::new();
    let mut plain: Vec<String> = Vec::new();

    for raw in content.lines() {
        let mut s = raw.trim();
        let mut times: Vec<f64> = Vec::new();

        while let Some(rest) = s.strip_prefix('[') {
            let Some(end) = rest.find(']') else { break };
            let inner = &rest[..end];
            match parse_time(inner) {
                Some(ms) => {
                    times.push(ms);
                    s = rest[end + 1..].trim_start();
                }
                None => break,
            }
        }

        if !times.is_empty() {
            let text = s.trim().to_string();
            for t in times {
                lines.push(LyricLine {
                    time_ms: t,
                    text: text.clone(),
                });
            }
        } else if s.starts_with('[') && s.contains(']') {
            // [ti:xxx] 之类的元数据行,跳过
            continue;
        } else if !s.is_empty() {
            plain.push(s.to_string());
        }
    }

    if !lines.is_empty() {
        lines.sort_by(|a, b| {
            a.time_ms
                .partial_cmp(&b.time_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let plain = if plain.is_empty() {
            None
        } else {
            Some(plain.join("\n"))
        };
        LyricsPayload {
            synced: Some(lines),
            plain,
        }
    } else if plain.is_empty() {
        LyricsPayload {
            synced: None,
            plain: None,
        }
    } else {
        LyricsPayload {
            synced: None,
            plain: Some(plain.join("\n")),
        }
    }
}

/// 解析 [mm:ss.xx] 形式的时间标签
fn parse_time(s: &str) -> Option<f64> {
    let (main, frac) = match s.split_once('.') {
        Some((m, f)) => (m, Some(f)),
        None => (s, None),
    };
    let mut parts = main.split(':');
    let minutes: f64 = parts.next()?.trim().parse().ok()?;
    let seconds: f64 = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let mut ms = minutes * 60_000.0 + seconds * 1000.0;
    if let Some(f) = frac {
        let v: f64 = format!("0.{f}").parse().ok()?;
        ms += v * 1000.0;
    }
    if ms.is_finite() && ms >= 0.0 {
        Some(ms)
    } else {
        None
    }
}

/// 字节流解码:优先 UTF-8(含 BOM),失败后按 GBK/GB18030 兜底。
/// 中文圈的 .lrc 文件很多是 GBK 编码。
pub fn decode_bytes(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let enc = if bytes.starts_with(&[0xFF, 0xFE]) {
            encoding_rs::UTF_16LE
        } else {
            encoding_rs::UTF_16BE
        };
        let (cow, _, _) = enc.decode(bytes);
        return cow.into_owned();
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    let (cow, _, had_errors) = GBK.decode(bytes);
    if had_errors {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        cow.into_owned()
    }
}

/// 修复 "GBK 内容被标记为 Latin-1" 造成的乱码:
/// 若字符串全部字符都落在 U+00FF 以内且含非 ASCII,按 Latin-1 还原字节后再用 GBK 解码。
pub fn fix_mojibake(s: &str) -> String {
    if s.is_ascii() {
        return s.to_string();
    }
    if s.chars().all(|c| (c as u32) < 0x100) {
        let bytes: Vec<u8> = s.chars().map(|c| c as u8).collect();
        let (cow, _, had_errors) = GBK.decode(&bytes);
        if !had_errors {
            return cow.into_owned();
        }
    }
    s.to_string()
}
