//! 生成 TMC 演示音乐包到项目根目录 demo/,用于验证 .tmc 导入全链路:
//!
//! ```bash
//! cd src-tauri && cargo run --example make_tmc
//! ```
//!
//! .tmc = 标准 7z 压缩包,内含:
//! - 音频文件(任意命名,扩展名需为应用支持的音频格式)
//! - 与音频同名的 .lrc 歌词(可选)
//! - cover.png / cover.jpg 封面(可选;若音频已内嵌封面可不带)
//! - meta.json 元数据(可选,仅补齐音频缺失的标签)
//!
//! 演示内容全部为程序合成/原创,覆盖三种典型情况:
//! 1. 内嵌标签 + 内嵌封面 + 同名 .lrc 歌词
//! 2. 音频无标签 → 元数据全部由 meta.json 提供;封面走 cover.png;歌词走同名 .lrc
//! 3. 内嵌标签 + 内嵌封面 + 内嵌歌词(不依赖任何伴生文件)
use lofty::config::WriteOptions;
use lofty::file::AudioFile;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::tag::{ItemKey, ItemValue, Tag, TagItem, TagType};
use sevenz_rust2::{ArchiveEntry, ArchiveWriter};
use std::path::Path;

fn main() {
    // 调试用:make_tmc extract <包> <目标目录> 解开一个 .tmc
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s == "extract").unwrap_or(false) {
        let src = Path::new(&args[2]);
        let dest = Path::new(&args[3]);
        std::fs::create_dir_all(dest).expect("创建目标目录失败");
        sevenz_rust2::decompress_file(src, dest).expect("解包失败");
        println!("已解包 {} -> {}", src.display(), dest.display());
        return;
    }
    // 调试用:make_tmc netease <标题> <艺人> [时长秒] 验证在线匹配算法
    if args.get(1).map(|s| s == "netease").unwrap_or(false) {
        let title = args.get(2).map(|s| s.as_str()).unwrap_or("");
        let artist = args.get(3).map(|s| s.as_str()).unwrap_or("");
        let dur: Option<f64> = args.get(4).and_then(|s| s.parse().ok());
        let client = tauri_music_lib::netease_client();
        println!(
            "封面: {:?}",
            tauri_music_lib::netease_album_cover(&client, title, artist, title)
                .map(|u| u[..60].to_string())
        );
        match tauri_music_lib::netease_lyric(&client, title, artist, dur) {
            Some(l) => println!("歌词(前120字): {}", l.chars().take(120).collect::<String>()),
            None => println!("歌词: 未找到"),
        }
        return;
    }
    // 调试用:make_tmc cover <标题> <艺人> [时长秒] 验证三级封面兜底与酷我链路
    if args.get(1).map(|s| s == "cover").unwrap_or(false) {
        let title = args.get(2).map(|s| s.as_str()).unwrap_or("");
        let artist = args.get(3).map(|s| s.as_str()).unwrap_or("");
        let dur: Option<f64> = args.get(4).and_then(|s| s.parse().ok());
        let show = |u: Option<String>| {
            u.map(|u| u[..u.len().min(72)].to_string())
                .unwrap_or_else(|| "未找到".into())
        };
        println!(
            "网易云封面 : {}",
            show(tauri_music_lib::netease_album_cover(
                &tauri_music_lib::netease_client(),
                title,
                artist,
                title,
            ))
        );
        println!(
            "iTunes 封面: {}",
            show(tauri_music_lib::itunes_album_cover(title, artist))
        );
        println!(
            "酷我封面   : {}",
            show(tauri_music_lib::kuwo_album_cover(
                &tauri_music_lib::generic_client(),
                title,
                artist,
                dur,
            ))
        );
        match tauri_music_lib::kuwo_lyric(&tauri_music_lib::generic_client(), title, artist, dur) {
            Some(l) => println!("酷我歌词   : {}", l.chars().take(60).collect::<String>()),
            None => println!("酷我歌词   : 未找到"),
        }
        return;
    }

    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../demo");
    std::fs::create_dir_all(&out_dir).expect("创建 demo 目录失败");
    let cover1 = include_bytes!("assets/cover1.png").to_vec();
    let cover2 = include_bytes!("assets/cover2.png").to_vec();

    // 1) 晨光微凉 - 林夜:内嵌标签 + 内嵌封面,歌词走同名 .lrc
    build(
        &out_dir.join("晨光微凉 - 林夜.tmc"),
        "晨光微凉",
        MEL1,
        &[
            (ItemKey::TrackTitle, "晨光微凉"),
            (ItemKey::TrackArtist, "林夜"),
            (ItemKey::AlbumTitle, "TMC 演示专辑"),
            (ItemKey::AlbumArtist, "林夜"),
            (ItemKey::Year, "2025"),
            (ItemKey::TrackNumber, "1"),
            (ItemKey::Genre, "Demo"),
        ],
        Some(&cover1),
        None,
        None,
        Some(LRC1),
        Some(&[
            ("title", "晨光微凉"),
            ("artist", "林夜"),
            ("album", "TMC 演示专辑"),
            ("albumArtist", "林夜"),
            ("year", "2025"),
            ("trackNo", "1"),
        ]),
    );

    // 2) 夜航星 - 林夜:音频无任何标签 → 全部由 meta.json 提供;封面走 cover.png
    build(
        &out_dir.join("夜航星 - 林夜.tmc"),
        "夜航星",
        MEL2,
        &[],
        None,
        None,
        Some(&cover1),
        Some(LRC2),
        Some(&[
            ("title", "夜航星"),
            ("artist", "林夜"),
            ("album", "TMC 演示专辑"),
            ("albumArtist", "林夜"),
            ("year", "2025"),
            ("trackNo", "2"),
        ]),
    );

    // 3) 雨后 - 苏晴:内嵌标签 + 内嵌封面 + 内嵌歌词,不依赖任何伴生文件
    build(
        &out_dir.join("雨后 - 苏晴.tmc"),
        "雨后",
        MEL3,
        &[
            (ItemKey::TrackTitle, "雨后"),
            (ItemKey::TrackArtist, "苏晴"),
            (ItemKey::AlbumTitle, "TMC 单曲"),
            (ItemKey::AlbumArtist, "苏晴"),
            (ItemKey::Year, "2024"),
            (ItemKey::Genre, "Demo"),
        ],
        Some(&cover2),
        Some(LRC3),
        None,
        None,
        None,
    );

    println!("TMC 演示包已生成: {}", out_dir.display());
}

/// 打包一个 .tmc:合成音频 → 打标签 → 落盘歌词/封面/meta → 7z 压缩
#[allow(clippy::too_many_arguments)]
fn build(
    dest: &Path,
    title: &str,
    melody: &[(f32, f32)],
    fields: &[(ItemKey, &str)],
    embed_cover: Option<&Vec<u8>>,
    embedded_lyrics: Option<&str>,
    sidecar_cover: Option<&Vec<u8>>,
    sidecar_lrc: Option<&str>,
    meta: Option<&[(&str, &str)]>,
) {
    let work = dest.parent().unwrap().join(format!("_work_{title}"));
    std::fs::create_dir_all(&work).expect("创建工作目录失败");

    let audio = work.join(format!("{title}.wav"));
    write_melody(&audio, melody);
    if !fields.is_empty() || embed_cover.is_some() || embedded_lyrics.is_some() {
        tag_wav(&audio, fields, embed_cover, embedded_lyrics);
    }
    if let Some(text) = sidecar_lrc {
        std::fs::write(work.join(format!("{title}.lrc")), text).expect("写歌词失败");
    }
    if let Some(png) = sidecar_cover {
        std::fs::write(work.join("cover.png"), png).expect("写封面失败");
    }
    if let Some(m) = meta {
        std::fs::write(work.join("meta.json"), meta_json(m)).expect("写 meta 失败");
    }

    pack(dest, &work);
    std::fs::remove_dir_all(&work).expect("清理工作目录失败");
    println!("  {}", dest.display());
}

fn pack(dest: &Path, work: &Path) {
    let mut writer = ArchiveWriter::create(dest).expect("创建 TMC 失败");
    let mut entries: Vec<_> = std::fs::read_dir(work)
        .expect("读工作目录失败")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        writer
            .push_archive_entry(
                ArchiveEntry::from_path(&p, name),
                std::fs::File::open(&p).ok(),
            )
            .expect("打包条目失败");
    }
    writer.finish().expect("写 TMC 失败");
}

fn meta_json(fields: &[(&str, &str)]) -> Vec<u8> {
    let mut map = serde_json::Map::new();
    for (k, v) in fields {
        map.insert(k.to_string(), serde_json::Value::String(v.to_string()));
    }
    serde_json::to_vec_pretty(&serde_json::Value::Object(map)).expect("序列化 meta 失败")
}

const SR: u32 = 44100;

/// 逐音符合成 16bit 单声道 WAV(正弦 + 二次谐波,带起音/收音包络);0.0 频率表示休止
fn write_melody(path: &Path, notes: &[(f32, f32)]) {
    let mut data: Vec<u8> = Vec::new();
    for &(freq, dur) in notes {
        let n = (dur * SR as f32) as usize;
        for i in 0..n {
            let t = i as f32 / SR as f32;
            let v = if freq > 0.0 {
                (2.0 * std::f32::consts::PI * freq * t).sin() * 0.8
                    + (2.0 * std::f32::consts::PI * freq * 2.0 * t).sin() * 0.15
            } else {
                0.0
            };
            let env = (t / 0.03).min(1.0) * ((dur - t) / 0.1).clamp(0.0, 1.0);
            data.extend_from_slice(&((v * env * 0.3 * 32767.0) as i16).to_le_bytes());
        }
    }
    let mut wav = Vec::with_capacity(44 + data.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // 单声道
    wav.extend_from_slice(&SR.to_le_bytes());
    wav.extend_from_slice(&(SR * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    std::fs::write(path, wav).expect("写 WAV 失败");
}

/// 为 WAV 写入 ID3v2 标签(标题/艺人/专辑/封面/歌词)
fn tag_wav(
    path: &Path,
    fields: &[(ItemKey, &str)],
    cover_png: Option<&Vec<u8>>,
    lyrics: Option<&str>,
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
    if let Some(text) = lyrics {
        tag.insert(TagItem::new(
            ItemKey::Lyrics,
            ItemValue::Text(text.to_string()),
        ));
    }
    tagged
        .save_to_path(path, WriteOptions::default())
        .expect("写标签失败");
}

// 三段原创小旋律(频率 Hz, 时长 s);0.0 表示休止
const MEL1: &[(f32, f32)] = &[
    (523.25, 1.2),
    (659.25, 1.2),
    (783.99, 1.2),
    (880.0, 1.2),
    (783.99, 1.2),
    (659.25, 1.2),
    (587.33, 1.2),
    (523.25, 1.6),
];
const MEL2: &[(f32, f32)] = &[
    (440.0, 1.2),
    (523.25, 1.2),
    (659.25, 1.2),
    (493.88, 1.2),
    (0.0, 0.6),
    (659.25, 1.2),
    (523.25, 1.2),
    (440.0, 1.6),
];
const MEL3: &[(f32, f32)] = &[
    (587.33, 1.1),
    (739.99, 1.1),
    (880.0, 1.1),
    (739.99, 1.1),
    (659.25, 1.1),
    (587.33, 1.1),
    (523.25, 1.5),
];

const LRC1: &str = "[ti:晨光微凉]\n[ar:林夜]\n[by:TauriMusic 演示]\n\
[00:00.30]晨光落在窗台上\n[00:01.50]微凉的风翻过书页\n[00:02.70]猫还赖在被窝里\n[00:03.90]时间慢了下来\n\
[00:05.10]远处的街道刚醒来\n[00:06.30]面包房飘出甜香\n[00:07.50]新的一天刚刚好\n[00:08.70]像这首慢半拍的歌\n";

const LRC2: &str = "[ti:夜航星]\n[ar:林夜]\n[by:TauriMusic 演示]\n\
[00:00.30]把灯熄掉以后\n[00:01.50]天花板变成银河\n[00:02.70]一颗星慢慢往前走\n[00:03.90]替我巡视夜晚\n\
[00:05.10]如果它掉下来\n[00:06.30]就当是夜寄来的信\n[00:07.50]落款是一整片\n[00:08.70]安静发光的温柔\n";

const LRC3: &str = "[ti:雨后]\n[ar:苏晴]\n[by:TauriMusic 演示]\n\
[00:00.30]雨停了\n[00:01.40]屋檐还在数水滴\n[00:02.80]柏油路亮成镜子\n[00:04.20]云缝里漏下光来\n\
[00:05.60]全世界刚刚洗过\n[00:07.00]连风都是新的\n";
