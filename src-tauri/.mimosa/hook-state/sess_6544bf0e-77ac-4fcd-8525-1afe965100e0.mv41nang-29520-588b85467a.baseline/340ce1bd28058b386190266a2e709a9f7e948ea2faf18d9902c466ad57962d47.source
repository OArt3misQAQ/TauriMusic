//! 生成示例曲库到项目根目录的 test-music/,用于本地体验与自测:
//!
//! ```bash
//! cd src-tauri && cargo run --example make_fixture
//! ```
//!
//! 生成内容覆盖三类典型情况:
//! 1. 内嵌封面 + 内嵌同步歌词(WAV + ID3v2)
//! 2. 内嵌封面 + 同名 .lrc 伴生歌词文件
//! 3. 完全无标签(仅靠文件名 "未命名歌曲" 回退)
use lofty::config::WriteOptions;
use lofty::file::AudioFile;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::tag::{ItemKey, ItemValue, Tag, TagItem, TagType};
use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-music");
    let album1 = root.join("霓虹夜晚 - 林夜");
    let album2 = root.join("北方的海 - 苏晴");
    let album3 = root.join("杂项");
    for dir in [&album1, &album2, &album3] {
        std::fs::create_dir_all(dir).expect("创建目录失败");
    }

    let cover1 = include_bytes!("assets/cover1.png").to_vec();
    let cover2 = include_bytes!("assets/cover2.png").to_vec();

    // 专辑一:霓虹夜晚 / 林夜 —— 内嵌封面 + 内嵌同步歌词
    write_wav(&album1.join("01 霓虹夜晚.wav"), 8.0, 330.0);
    tag_wav(
        &album1.join("01 霓虹夜晚.wav"),
        &[
            (ItemKey::TrackTitle, "霓虹夜晚"),
            (ItemKey::TrackArtist, "林夜"),
            (ItemKey::AlbumTitle, "霓虹夜晚"),
            (ItemKey::AlbumArtist, "林夜"),
            (ItemKey::Year, "2025"),
            (ItemKey::Genre, "Synthwave"),
            (ItemKey::TrackNumber, "1"),
        ],
        Some(&cover1),
        Some("[00:00.50]城市的霓虹漫过窗沿\n[00:02.50]晚风把时间调得很慢\n[00:04.50]收音机里放着老唱片\n[00:06.50]我们在夜里飞行\n[00:07.80]飞过整座霓虹夜晚"),
        0,
    );
    write_wav(&album1.join("02 城市星光.wav"), 8.0, 392.0);
    tag_wav(
        &album1.join("02 城市星光.wav"),
        &[
            (ItemKey::TrackTitle, "城市星光"),
            (ItemKey::TrackArtist, "林夜"),
            (ItemKey::AlbumTitle, "霓虹夜晚"),
            (ItemKey::AlbumArtist, "林夜"),
            (ItemKey::Year, "2025"),
            (ItemKey::TrackNumber, "2"),
        ],
        Some(&cover1),
        None,
        0,
    );

    // 专辑二:北方的海 / 苏晴 —— 内嵌封面 + 同名 .lrc 伴生文件
    write_wav(&album2.join("01 北方的海.wav"), 8.0, 262.0);
    tag_wav(
        &album2.join("01 北方的海.wav"),
        &[
            (ItemKey::TrackTitle, "北方的海"),
            (ItemKey::TrackArtist, "苏晴"),
            (ItemKey::AlbumTitle, "北方的海"),
            (ItemKey::AlbumArtist, "苏晴"),
            (ItemKey::Year, "2024"),
            (ItemKey::TrackNumber, "1"),
        ],
        Some(&cover2),
        None,
        1, // 歌词放伴生 .lrc
    );
    std::fs::write(
        album2.join("01 北方的海.lrc"),
        "[ti:北方的海]\n[ar:苏晴]\n[00:00.50]列车穿过北方的平原\n[00:02.50]窗外是结冰的海面\n[00:04.50]你说冬天就要来了\n[00:06.50]而海还记得夏天\n",
    )
    .unwrap();
    write_wav(&album2.join("02 灯塔.wav"), 8.0, 294.0);
    tag_wav(
        &album2.join("02 灯塔.wav"),
        &[
            (ItemKey::TrackTitle, "灯塔"),
            (ItemKey::TrackArtist, "苏晴"),
            (ItemKey::AlbumTitle, "北方的海"),
            (ItemKey::AlbumArtist, "苏晴"),
            (ItemKey::Year, "2024"),
            (ItemKey::TrackNumber, "2"),
        ],
        Some(&cover2),
        None,
        0,
    );

    // 无任何标签的文件:标题回退到文件名,归入"未知专辑"
    write_wav(&album3.join("未命名歌曲.wav"), 8.0, 220.0);

    println!("示例曲库已生成: {}", root.display());
}

/// 写一段 16bit 单声道正弦波 WAV
fn write_wav(path: &Path, seconds: f32, freq: f32) {
    const SAMPLE_RATE: u32 = 44100;
    let n = (seconds * SAMPLE_RATE as f32) as usize;
    let mut data = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let v = (2.0 * std::f32::consts::PI * freq * t).sin();
        let fade = (t / 0.05).min(1.0) * ((seconds - t) / 0.05).min(1.0);
        let sample = (v * 0.35 * fade * 32767.0) as i16;
        data.extend_from_slice(&sample.to_le_bytes());
    }

    let mut wav = Vec::with_capacity(44 + data.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // 单声道
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    std::fs::write(path, wav).expect("写 WAV 失败");
}

/// 为 WAV 写入 ID3v2 标签
fn tag_wav(
    path: &Path,
    fields: &[(ItemKey, &str)],
    cover_png: Option<&Vec<u8>>,
    embedded_lyrics: Option<&str>,
    lrc_only: u8,
) {
    let mut tagged = lofty::read_from_path(path).expect("读取音频失败");
    if tagged.primary_tag_mut().is_none() {
        tagged.insert_tag(Tag::new(TagType::Id3v2));
    }
    let tag = tagged.primary_tag_mut().expect("标签不可用");
    for (key, value) in fields {
        tag.insert(TagItem::new(
            key.clone(),
            ItemValue::Text((*value).to_string()),
        ));
    }
    if let Some(png) = cover_png {
        tag.push_picture(Picture::new_unchecked(
            PictureType::CoverFront,
            Some(MimeType::Png),
            None,
            png.clone(),
        ));
    }
    if let Some(text) = embedded_lyrics {
        tag.insert(TagItem::new(
            ItemKey::Lyrics,
            ItemValue::Text(text.to_string()),
        ));
    }
    let _ = lrc_only; // 仅用于标记该曲目歌词走伴生 .lrc
    tagged
        .save_to_path(path, WriteOptions::default())
        .expect("写标签失败");
}
