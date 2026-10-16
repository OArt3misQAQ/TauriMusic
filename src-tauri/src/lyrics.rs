use encoding_rs::{GB18030, GBK};
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
    // 时间标签里只会出现数字与 :. ——负号/科学计数是畸形输入(如 [01:-30.5]),
    // f64 解析会让它们凑出"看似合法"的时间戳,先按字符挡掉
    if s.contains(['-', 'e', 'E']) {
        return None;
    }
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
    // GB18030 是 GBK 的超集(覆盖四字节生僻字),失败时才退回替换字符
    let (cow, _, had_errors) = GB18030.decode(bytes);
    if had_errors {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        cow.into_owned()
    }
}

/// CJK 汉字占比(不计空白):判定解码结果"像不像中文文本"
fn cjk_ratio(s: &str) -> f64 {
    let mut total = 0usize;
    let mut cjk = 0usize;
    for c in s.chars() {
        if c.is_ascii_whitespace() {
            continue;
        }
        total += 1;
        let u = c as u32;
        if (0x4E00..=0x9FFF).contains(&u) || (0x3400..=0x4DBF).contains(&u) {
            cjk += 1;
        }
    }
    if total == 0 {
        0.0
    } else {
        cjk as f64 / total as f64
    }
}

/// 修复 "GBK 内容被标记为 Latin-1" 造成的乱码:
/// 若字符串全部字符都落在 U+00FF 以内且含非 ASCII,按 Latin-1 还原字节后再用 GBK 解码。
/// 采纳门槛:原串高位字符足够密(GBK 中文每个汉字对应两个 ≥0x80 的字符,
/// 西文变音符密度远低于此)且解码结果以汉字为主——否则法语/德语等正常西文
/// 会被误判成乱码、二次破坏成汉字噪声
pub fn fix_mojibake(s: &str) -> String {
    if s.is_ascii() {
        return s.to_string();
    }
    let high = s.chars().filter(|c| !c.is_ascii()).count();
    if s.chars().all(|c| (c as u32) < 0x100) && high * 10 >= s.chars().count() * 3 {
        let bytes: Vec<u8> = s.chars().map(|c| c as u8).collect();
        let (cow, _, had_errors) = GBK.decode(&bytes);
        if !had_errors && cjk_ratio(&cow) >= 0.5 {
            return cow.into_owned();
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_time_accepts_normal_labels() {
        assert_eq!(parse_time("01:30.5"), Some(90_500.0));
        assert_eq!(parse_time("0:00"), Some(0.0));
        assert_eq!(parse_time("2:05.25"), Some(125_250.0));
    }

    #[test]
    fn parse_time_rejects_malformed() {
        // 负分量会凑出"看似合法"的总和,科学计数同理,必须整串拒绝
        assert_eq!(parse_time("01:-30.5"), None);
        assert_eq!(parse_time("1e2:00"), None);
        assert_eq!(parse_time("00:1e1"), None);
        assert_eq!(parse_time("1:2:3:4"), None);
        assert_eq!(parse_time("abc"), None);
        assert_eq!(parse_time(""), None);
    }

    #[test]
    fn parse_splits_multi_timestamp_lines_and_sorts() {
        let p = parse("[00:10.0][00:20.0]副歌\n[ti:元数据]\n没有时间戳的行\n");
        let synced = p.synced.expect("两行时间戳");
        assert_eq!(synced.len(), 2);
        assert_eq!(synced[0].time_ms, 10_000.0);
        assert_eq!(synced[0].text, "副歌");
        // 时间戳乱序输入也要按时间排好
        let p2 = parse("[00:30.0]b\n[00:05.0]a\n");
        let s2 = p2.synced.expect("synced");
        assert_eq!(s2[0].text, "a");
    }

    #[test]
    fn decode_bytes_handles_gbk_and_gb18030() {
        // GBK 两字节中文(用编码器生成期望字节,避免手写十六进制出错)
        let (gbk, _, _) = GBK.encode("中文歌词");
        assert_eq!(decode_bytes(&gbk), "中文歌词");
        // GB18030 四字节生僻字:GBK 编码器表示不了,必须由 GB18030 兜底解码
        let (gb18030, _, had_err) = GB18030.encode("𠀀");
        assert!(!had_err, "𠀀 应能被 GB18030 编码");
        assert_eq!(decode_bytes(&gb18030), "𠀀");
        // UTF-8 BOM 剥离
        let bom = [0xEF, 0xBB, 0xBF, b'h', b'i'];
        assert_eq!(decode_bytes(&bom), "hi");
    }

    #[test]
    fn fix_mojibake_recovers_gbk_and_keeps_western_text() {
        // "中文" 的 GBK 字节被"按 Latin-1 逐字节读成字符"的乱码形态
        // (from_utf8_lossy 会把非法字节换成 U+FFFD,构造不出这种乱码)
        let mojibake: String = [0xD6u8, 0xD0, 0xCE, 0xC4]
            .iter()
            .map(|&b| b as char)
            .collect();
        assert_eq!(fix_mojibake(&mojibake), "中文");
        // 法语变音符文本:高位字符密度低,不应被"修复"成汉字噪声
        let french = "Voilà l'été, c'est la vie";
        assert_eq!(fix_mojibake(french), french);
        // 纯 ASCII 与含 CJK 的正常文本原样返回
        assert_eq!(fix_mojibake("plain ascii"), "plain ascii");
        assert_eq!(fix_mojibake("正常中文"), "正常中文");
    }
}
