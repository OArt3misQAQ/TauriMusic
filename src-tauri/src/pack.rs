//! 音乐包(TMC/TMCA/TMCL)导出与导入:7z 打包/解包、SHA 校验、内容去重与导入预览。

use crate::import::{ensure_import_dir, IMPORT_DIR_NAME};
use crate::model::{self, Library};
use crate::playlists::{entry_of, new_playlist_id, now_secs_f64, unique_playlist_name};
use crate::scanner;
use crate::{lock_scan_gate, AppState, LIBRARY_FILE};
use sha2::Digest as _;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{Emitter, Manager};

// ===== TMC 音乐包(.tmc = 标准 7z:音频 + 同名 .lrc 歌词 + cover 封面 + meta.json) =====

pub(crate) fn sanitize_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => out.push('_'),
            c if (c as u32) < 32 => {}
            c => out.push(c),
        }
    }
    // Windows 文件名:首尾空白与结尾的点会被静默吞掉或报错,保留名不可用
    let mut t = out.trim().trim_end_matches(['.', ' ']).to_string();
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    // Windows 保留设备名对「主名」生效:CON.mp3 与 CON 一样不可用
    let base = t.split('.').next().unwrap_or_default();
    if RESERVED.contains(&base.to_ascii_uppercase().as_str()) {
        t.insert(0, '_');
    }
    if t.is_empty() {
        "track".into()
    } else {
        t
    }
}

/// 用路径组件判断 target 是否位于 dir 之下(与字符串前缀匹配不同,
/// 不会把 F:\music2 误当成 F:\music 的子目录)。
/// 手动按 /\ 双分隔符切分组件,不依赖 std 的平台语义,
/// Windows 风格路径字符串在非 Windows 平台上同样可判定。
pub(crate) fn is_under(child: &str, dir: &str) -> bool {
    fn components(p: &str) -> impl Iterator<Item = &str> {
        p.split(['\\', '/']).filter(|s| !s.is_empty())
    }
    // Windows 路径大小写不敏感(盘符/目录),其他平台精确比较
    fn part_eq(a: &str, b: &str) -> bool {
        if cfg!(windows) {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    }
    let mut child_parts = components(child);
    let mut matched = 0usize;
    for d in components(dir) {
        matched += 1;
        match child_parts.next() {
            Some(c) if part_eq(c, d) => continue,
            _ => return false,
        }
    }
    matched > 0
}

/// 把打包完成的临时压缩包落位到用户指定路径。
/// 打包期间目标路径不存在文件,用户不会拷到半成品;
/// 同盘 rename 原子直达,跨盘(系统 Temp 与目标不同卷)退化为拷贝,源是完整封口的包。
pub(crate) fn place_archive(tmp: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if fs::rename(tmp, dest).is_ok() {
        return Ok(());
    }
    // 跨盘拷贝中途失败(磁盘满/拔盘):必须清掉目标处的半成品,
    // 否则下次落位同名文件会被 dedup_dest 改名成 "xxx (2)"
    if let Err(e) = fs::copy(tmp, dest) {
        let _ = fs::remove_file(dest);
        return Err(format!("落位到 {} 失败: {e}", dest.display()));
    }
    let _ = fs::remove_file(tmp);
    Ok(())
}

/// 目标已存在时追加序号 name (2).ext / name (3).ext,保留扩展名
pub(crate) fn dedup_dest(dest: PathBuf) -> PathBuf {
    if !dest.exists() {
        return dest;
    }
    let parent = dest.parent().unwrap_or(Path::new(".")).to_path_buf();
    let stem = dest
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let ext = dest
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let mut i = 2;
    loop {
        let candidate = parent.join(format!("{stem} ({i}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
        i += 1;
    }
}

pub(crate) fn import_tmc_file(
    src: &Path,
    app: &tauri::AppHandle,
) -> Result<PackImportReport, String> {
    if !src.is_file() {
        return Err("文件不存在".into());
    }
    let state = app.state::<AppState>();
    let import_dir = ensure_import_dir(app, &state)?;
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "tmc".into());
    let dest = import_dir.join(&stem);
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| format!("清理旧目录失败: {e}"))?;
    }
    fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    sevenz_rust2::decompress_file(src, &dest).map_err(|e| format!("解包失败: {e}"))?;

    // 新包带音频 SHA-256:解包结果与记录不符说明文件损坏,拒绝入库并清理
    let meta_path = dest.join("meta.json");
    let mut title = stem.clone();
    if meta_path.is_file() {
        if let Ok(v) =
            serde_json::from_slice::<serde_json::Value>(&fs::read(&meta_path).unwrap_or_default())
        {
            if let Some(expect) = v.get("sha256").and_then(|x| x.as_str()) {
                let audio = fs::read_dir(&dest).ok().and_then(|rd| {
                    rd.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
                        p.extension()
                            .and_then(|x| x.to_str())
                            .map(|x| scanner::EXTENSIONS.contains(&x.to_lowercase().as_str()))
                            .unwrap_or(false)
                    })
                });
                if let Some(audio) = audio {
                    let ok = sha256_hex(&audio)
                        .map(|h| h.eq_ignore_ascii_case(expect))
                        .unwrap_or(false);
                    if !ok {
                        let _ = fs::remove_dir_all(&dest);
                        return Err("音乐包校验失败:文件已损坏".into());
                    }
                }
            }
            if let Some(t) = v.get("title").and_then(|x| x.as_str()) {
                if !t.trim().is_empty() {
                    title = t.trim().to_string();
                }
            }
        }
    }
    let audio = fs::read_dir(&dest)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.extension()
                .and_then(|x| x.to_str())
                .map(|x| scanner::EXTENSIONS.contains(&x.to_lowercase().as_str()))
                .unwrap_or(false)
        })
        .ok_or("包内没有音频文件")?;
    // 内容级去重:音频 SHA-256 与曲库已有曲目相同 → 清理并跳过(单曲包此前没有去重)
    let sha = sha256_hex(&audio).map(|h| h.to_ascii_lowercase());
    if let Some(sha) = &sha {
        let dup = {
            let lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
            lib.tracks.iter().any(|t| {
                t.sha256
                    .as_deref()
                    .map(|s| s.eq_ignore_ascii_case(sha))
                    .unwrap_or(false)
            })
        };
        if dup {
            let _ = fs::remove_dir_all(&dest);
            return Ok(PackImportReport {
                name: title,
                imported: 0,
                skipped: 1,
            });
        }
    }
    scanner::run_scan(app)?;
    if let Some(sha) = &sha {
        record_track_shas(&state, &[(audio.clone(), sha.clone())])?;
    }
    let _ = app.emit("library-changed", ());
    Ok(PackImportReport {
        name: title,
        imported: 1,
        skipped: 0,
    })
}

// ===== 音乐包导入预览:解包到暂存目录解析元数据,不写库;确认后才走正式导入 =====

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreviewTrack {
    title: String,
    artist: String,
    album: String,
    duration: u32,
    /// 相对暂存根的封面路径(资产协议已放行 covers 缓存),None 渲染占位
    cover: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackagePreview {
    /// tmc(单曲包)/ tmca(专辑包)/ tmcl(歌单包)
    kind: String,
    name: String,
    /// 暂存目录绝对根:封面 = root + 相对路径,经资产协议展示
    root: String,
    /// 暂存目录名(covers/preview/<id>),前端确认/取消后调 discard_preview 清理
    staging_id: String,
    tracks: Vec<PreviewTrack>,
}

/// 读音频内嵌标签与时长(失败全部容错返回 None,预览不能因为坏文件而中断)
fn read_preview_tags(path: &Path) -> (Option<String>, Option<String>, Option<String>, Option<u32>) {
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::tag::Accessor;
    let Ok(tagged) = lofty::probe::Probe::open(path).and_then(|p| p.read()) else {
        return (None, None, None, None);
    };
    let duration = tagged.properties().duration().as_secs() as u32;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let field = |f: fn(&lofty::tag::Tag) -> Option<std::borrow::Cow<'_, str>>| -> Option<String> {
        tag.and_then(f)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let title = field(lofty::tag::Tag::title);
    let artist = field(lofty::tag::Tag::artist);
    let album = field(lofty::tag::Tag::album);
    (title, artist, album, Some(duration))
}

/// 在目录里找封面:cover.* 命名优先,否则取最大的图片文件;返回相对暂存根的路径
fn find_cover_in(dir: &Path, staging: &Path) -> Option<String> {
    let mut best: Option<(std::path::PathBuf, u64)> = None;
    for e in fs::read_dir(dir).ok()?.filter_map(|e| e.ok()) {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        let ext = p
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| x.to_lowercase())
            .unwrap_or_default();
        if !matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp") {
            continue;
        }
        let len = e.metadata().map(|m| m.len()).unwrap_or(0);
        let is_named = p
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("cover"))
            .unwrap_or(false);
        // cover.* 直接采用;否则与当前最大图比大小
        if is_named || best.as_ref().map(|(_, l)| len > *l).unwrap_or(true) {
            best = Some((p, len));
        }
    }
    let p = best?.0;
    p.strip_prefix(staging)
        .ok()
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
}

fn inspect_tmc(staging: &Path) -> Result<PackagePreview, String> {
    let meta = fs::read(staging.join("meta.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    let json_str = |k: &str| -> Option<String> {
        meta.as_ref()
            .and_then(|m| m.get(k))
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let audio = fs::read_dir(staging)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.extension()
                .and_then(|x| x.to_str())
                .map(|x| scanner::EXTENSIONS.contains(&x.to_lowercase().as_str()))
                .unwrap_or(false)
        })
        .ok_or("包内没有音频文件")?;
    let (tag_title, tag_artist, tag_album, duration) = read_preview_tags(&audio);
    let cover = find_cover_in(staging, staging);
    Ok(PackagePreview {
        kind: "tmc".into(),
        name: json_str("title")
            .or(tag_title.clone())
            .unwrap_or_else(|| "未命名单曲".into()),
        root: staging.to_string_lossy().to_string(),
        staging_id: String::new(),
        tracks: vec![PreviewTrack {
            title: json_str("title")
                .or(tag_title)
                .unwrap_or_else(|| "未知标题".into()),
            artist: json_str("artist")
                .or(tag_artist)
                .unwrap_or_else(|| "未知艺人".into()),
            album: json_str("album").or(tag_album).unwrap_or_default(),
            duration: duration.unwrap_or(0),
            cover,
        }],
    })
}

fn inspect_manifest(staging: &Path, manifest: &str, kind: &str) -> Result<PackagePreview, String> {
    let v: serde_json::Value = serde_json::from_slice(
        &fs::read(staging.join(manifest)).map_err(|e| format!("{manifest} 读取失败: {e}"))?,
    )
    .map_err(|e| format!("{manifest} 解析失败: {e}"))?;
    let name = v
        .get("name")
        .and_then(|x| x.as_str())
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .unwrap_or_else(|| "未命名".into());
    let root_cover = find_cover_in(staging, staging);
    let mut tracks: Vec<PreviewTrack> = Vec::new();
    for item in v
        .get("tracks")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
    {
        let jstr = |k: &str| -> Option<String> {
            item.get(k)
                .and_then(|x| x.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        // dir 缺省/为空 = 包根;提供时必须过 zip-slip 校验——预览是恶意包
        // 最先触达的入口,`../` 可借 read_preview_tags 读到暂存目录外的文件
        let dir_raw = item.get("dir").and_then(|x| x.as_str()).unwrap_or("");
        let dir = if dir_raw.is_empty() {
            String::new()
        } else {
            match safe_manifest_part(dir_raw) {
                Some(d) => d,
                None => continue,
            }
        };
        let audio_raw = item.get("audio").and_then(|x| x.as_str()).unwrap_or("");
        let Some(audio) = safe_manifest_part(audio_raw) else {
            continue;
        };
        let abs = staging.join(&dir).join(&audio);
        let (tag_title, tag_artist, tag_album, duration) = if abs.is_file() {
            read_preview_tags(&abs)
        } else {
            (None, None, None, None)
        };
        let track_dir = staging.join(&dir);
        let cover = find_cover_in(&track_dir, staging).or_else(|| root_cover.clone());
        tracks.push(PreviewTrack {
            title: jstr("title")
                .or(tag_title)
                .unwrap_or_else(|| "未知标题".into()),
            artist: jstr("artist")
                .or(tag_artist)
                .unwrap_or_else(|| "未知艺人".into()),
            album: jstr("album").or(tag_album).unwrap_or_default(),
            duration: duration.unwrap_or(0),
            cover,
        });
    }
    if tracks.is_empty() {
        return Err("包内没有曲目清单".into());
    }
    Ok(PackagePreview {
        kind: kind.into(),
        name,
        root: staging.to_string_lossy().to_string(),
        staging_id: String::new(),
        tracks,
    })
}

/// 解包音乐包并解析出预览(inspect_package 命令的实现):重活在阻塞线程执行
pub(crate) fn inspect_package_blocking(
    path: &str,
    app: &tauri::AppHandle,
) -> Result<PackagePreview, String> {
    let src = Path::new(&path);
    if !src.is_file() {
        return Err("文件不存在".into());
    }
    let ext = src
        .extension()
        .and_then(|x| x.to_str())
        .map(|x| x.to_lowercase())
        .unwrap_or_default();
    let kind = match ext.as_str() {
        "tmc" | "tmca" | "tmcl" => ext,
        _ => return Err("仅支持 .tmc/.tmca/.tmcl".into()),
    };
    let state = app.state::<AppState>();
    let staging_root = state.cache_dir.join("covers").join("preview");
    fs::create_dir_all(&staging_root).map_err(|e| e.to_string())?;
    let id = format!(
        "{}-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
        std::process::id()
    );
    let staging = staging_root.join(&id);
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    if let Err(e) = sevenz_rust2::decompress_file(src, &staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("解包失败: {e}"));
    }
    let parsed = match kind.as_str() {
        "tmc" => inspect_tmc(&staging),
        "tmca" => inspect_manifest(&staging, "album.json", "tmca"),
        _ => inspect_manifest(&staging, "playlist.json", "tmcl"),
    };
    match parsed {
        Ok(mut preview) => {
            preview.staging_id = id;
            Ok(preview)
        }
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

pub(crate) fn stage_tmc_blocking(
    id: &str,
    staging: &Path,
    app: &tauri::AppHandle,
) -> Result<String, String> {
    // 用户已取消(暂存目录被清理):拒绝,避免工作目录创建时把暂存目录重建出来
    if !staging.is_dir() {
        return Err("导出已取消".into());
    }
    let state = app.state::<AppState>();
    let track = {
        let lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        lib.tracks.iter().find(|t| t.id == id).cloned()
    };
    let Some(t) = track else {
        return Err("曲目不存在".into());
    };
    let audio = Path::new(&t.path);
    if !audio.is_file() {
        return Err(format!("音频文件不存在: {}", t.path));
    }
    let ext = audio
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase().to_string())
        .unwrap_or_else(|| "mp3".into());
    let stem = sanitize_name(&t.title);

    // meta.json 与 7z 都写在暂存区的工作子目录里,打包封口后移到暂存区顶层;
    // meta 的音频 SHA 来自打包时的单遍哈希,回调里才落盘,不再预读第二遍
    let workspace = staging.join(".tmp");
    fs::create_dir_all(&workspace).map_err(|e| e.to_string())?;
    let meta_path = workspace.join("meta.json");

    // 条目先收集成表再打包;打开失败必须报错,不给 SevenZWriter 静默写空条目的机会
    let mut assets: ExportAssets = Vec::new();
    let audio_key = format!("{stem}.{ext}");
    fs::File::open(audio).map_err(|e| format!("打开音频失败 {}: {e}", audio.display()))?;
    assets.push((audio_key.clone(), audio.to_path_buf()));
    if let Some(lrc) = &t.lrc_path {
        let lrc_path = Path::new(lrc);
        if lrc_path.is_file() {
            assets.push((format!("{stem}.lrc"), lrc_path.to_path_buf()));
        }
    }
    if let Some(cover) = &t.cover {
        let cover_path = Path::new(cover);
        if cover_path.is_file() {
            let cext = cover_path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase().to_string())
                .unwrap_or_else(|| "jpg".into());
            let cname = if cext == "png" {
                "cover.png"
            } else {
                "cover.jpg"
            };
            assets.push((cname.to_string(), cover_path.to_path_buf()));
        }
    }

    // 7z 先写入工作目录,打包封口后落进暂存区,用户目录不会出现半成品
    let tmp_archive = workspace.join(format!("{stem}.tmc"));
    let _ = fs::remove_file(&tmp_archive);
    pack_with_hashes(app, &tmp_archive, &assets, "TMC", &mut |hashes| {
        let Some(audio_sha) = hashes.get(&audio_key) else {
            return Err("音频哈希缺失".into());
        };
        let meta = serde_json::json!({
            "title": t.title,
            "artist": t.artist,
            "album": t.album,
            "albumArtist": t.album_artist,
            "year": t.year,
            "trackNo": t.track_no,
            "genre": t.genre,
            "sha256": audio_sha,
        });
        fs::write(
            &meta_path,
            serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        Ok(("meta.json".to_string(), meta_path.clone()))
    })?;
    // 同一批次里同标题的歌在暂存区内自动加序号,落位时再按目标目录去重
    let staged = dedup_dest(staging.join(format!("{stem}.tmc")));
    place_archive(&tmp_archive, &staged)?;
    let _ = fs::remove_dir_all(&workspace);
    Ok(staged.to_string_lossy().to_string())
}

/// 把资源表写进暂存区压缩包(进度按条目上报)。
/// 音频本身已是压缩格式,采用 7z 的 Copy(存储)方法:打包接近纯拷贝速度,
/// meta/歌词体积可忽略;标准 7z 方法,任何解压工具可读
/// 字节序列转小写十六进制
fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Read 包装:边被读取边计算 SHA-256(导出打包单遍完成"哈希收集+写入")
struct HashingReader<R: std::io::Read> {
    inner: R,
    hasher: sha2::Sha256,
}
impl<R: std::io::Read> std::io::Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.hasher.update(&buf[0..n]);
        }
        Ok(n)
    }
}

/// manifest 构建回调:输入 归档名→SHA 的收集结果,返回 (manifest 归档名, manifest 文件路径)
type ManifestBuilder<'a> =
    &'a mut dyn FnMut(&HashMap<String, String>) -> Result<(String, PathBuf), String>;

/// 打包内容资源并顺带收集各文件 SHA-256:每条资源用 HashingReader 边喂边算,
/// 内容全部打包完成后回调生成 manifest(调用方写文件),作为最后一条条目推入再封口。
/// 7z 条目顺序无语义要求;这样每个文件只读一遍盘(旧流程哈希、打包各读一遍)
fn pack_with_hashes(
    app: &tauri::AppHandle,
    tmp_archive: &Path,
    assets: &ExportAssets,
    what: &str,
    build_manifest: ManifestBuilder<'_>,
) -> Result<(), String> {
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod};
    let mut writer =
        ArchiveWriter::create(tmp_archive).map_err(|e| format!("创建 {what} 失败: {e}"))?;
    writer.set_content_methods(vec![EncoderConfiguration::new(EncoderMethod::COPY)]);
    let mut hashes: HashMap<String, String> = HashMap::new();
    let total = assets.len() + 1;
    for (i, (name, path)) in assets.iter().enumerate() {
        let _ = app.emit(
            "export-progress",
            serde_json::json!({ "done": i, "total": total }),
        );
        let file =
            fs::File::open(path).map_err(|e| format!("打开 {} 失败: {e}", path.display()))?;
        let mut hr = HashingReader {
            inner: file,
            hasher: sha2::Sha256::new(),
        };
        writer
            .push_archive_entry(ArchiveEntry::from_path(path, name.clone()), Some(&mut hr))
            .map_err(|e| e.to_string())?;
        let digest: String = to_hex(hr.hasher.finalize().as_slice());
        hashes.insert(name.clone(), digest);
    }
    let (manifest_name, manifest_path) = build_manifest(&hashes)?;
    let _ = app.emit(
        "export-progress",
        serde_json::json!({ "done": assets.len(), "total": total }),
    );
    let file = fs::File::open(&manifest_path)
        .map_err(|e| format!("打开 {} 失败: {e}", manifest_path.display()))?;
    writer
        .push_archive_entry(
            ArchiveEntry::from_path(&manifest_path, manifest_name),
            Some(file),
        )
        .map_err(|e| e.to_string())?;
    writer
        .finish()
        .map_err(|e| format!("写入 {what} 失败: {e}"))?;
    Ok(())
}

/// 把打包时收集到的哈希按归档名注入 manifest 条目:
/// audio→sha256、lrc→lrcSha256、cover→coverSha256
fn inject_manifest_shas(items: &mut [serde_json::Value], hashes: &HashMap<String, String>) {
    for item in items {
        let Some(obj) = item.as_object_mut() else {
            continue;
        };
        for (sha_key, name_key) in [
            ("sha256", "audio"),
            ("lrcSha256", "lrc"),
            ("coverSha256", "cover"),
        ] {
            if let Some(h) = obj
                .get(name_key)
                .and_then(|v| v.as_str())
                .and_then(|n| hashes.get(n))
            {
                obj.insert(sha_key.to_string(), serde_json::json!(h));
            }
        }
    }
}

/// 导出资源表:包内条目名 → 源文件路径
type ExportAssets = Vec<(String, PathBuf)>;
/// 规划结果:(资源表, manifest 的 tracks 节点)
type PlannedAssets = (ExportAssets, Vec<serde_json::Value>);

/// 把曲目按「music/NN. 名字」规划进导出资源表:每首一个子目录,内含音频、
/// 同名 .lrc、专属封面与 meta.json(供扫描器补齐无内嵌标签的文件)。
/// TMCL 与 TMCA 的打包共用此规划;SHA 由打包阶段单遍收集后注入,这里不做哈希。
/// 音频文件缺失的条目静默跳过
fn plan_track_assets(tracks: &[model::Track], workspace: &Path) -> Result<PlannedAssets, String> {
    let mut assets: Vec<(String, PathBuf)> = Vec::new();
    let mut items: Vec<serde_json::Value> = Vec::new();
    for (i, t) in tracks.iter().enumerate() {
        let audio = Path::new(&t.path);
        if !audio.is_file() {
            continue; // 文件缺失的条目静默跳过(名称已在曲库核对过)
        }
        let ext = audio
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase().to_string())
            .unwrap_or_else(|| "mp3".into());
        let stem = format!("{} - {}", sanitize_name(&t.title), sanitize_name(&t.artist));
        let folder = format!("music/{:02}. {stem}", i + 1);
        let audio_name = format!("{stem}.{ext}");

        assets.push((format!("{folder}/{audio_name}"), audio.to_path_buf()));
        let mut item = serde_json::json!({
            "dir": folder,
            "audio": audio_name,
            "title": t.title,
            "artist": t.artist,
            "album": t.album,
            "albumArtist": t.album_artist,
            "duration": t.duration,
        });
        if let Some(lrc) = &t.lrc_path {
            let lp = Path::new(lrc);
            if lp.is_file() {
                let lrc_name = format!("{stem}.lrc");
                assets.push((format!("{folder}/{lrc_name}"), lp.to_path_buf()));
                item["lrc"] = serde_json::json!(lrc_name);
            }
        }
        if let Some(cover) = &t.cover {
            let cp = Path::new(cover);
            if cp.is_file() {
                let cext = cp
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase().to_string())
                    .unwrap_or_else(|| "jpg".into());
                let cover_name = format!("{stem}.cover.{cext}");
                assets.push((format!("{folder}/{cover_name}"), cp.to_path_buf()));
                item["cover"] = serde_json::json!(cover_name);
            }
        }
        // meta.json 供扫描器补齐无内嵌标签的文件;写在临时目录再随包
        let meta_dir = workspace.join(format!("{i:02}"));
        fs::create_dir_all(&meta_dir).map_err(|e| e.to_string())?;
        let meta = serde_json::json!({
            "title": t.title,
            "artist": t.artist,
            "album": t.album,
            "albumArtist": t.album_artist,
            "year": t.year,
            "trackNo": t.track_no,
            "genre": t.genre,
        });
        let meta_path = meta_dir.join("meta.json");
        fs::write(
            &meta_path,
            serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        assets.push((format!("{folder}/meta.json"), meta_path));

        items.push(item);
    }
    Ok((assets, items))
}

// ===== TMCA 专辑包(.tmcl 的专辑版:单文件整张专辑,导入只入库不建歌单) =====

pub(crate) fn stage_tmca_blocking(
    album_key: &str,
    staging: &Path,
    app: &tauri::AppHandle,
) -> Result<String, String> {
    // 用户已取消(暂存目录被清理):拒绝,避免工作目录创建时把暂存目录重建出来
    if !staging.is_dir() {
        return Err("导出已取消".into());
    }
    let state = app.state::<AppState>();
    let mut tracks: Vec<model::Track> = {
        let lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        lib.tracks
            .iter()
            .filter(|t| format!("{}\u{1}{}", t.album_artist, t.album) == album_key)
            .cloned()
            .collect()
    };
    // 与专辑详情页一致的排序:碟号 → 曲目号 → 标题
    tracks.sort_by(|a, b| {
        (
            a.disc_no.unwrap_or(1),
            a.track_no.unwrap_or(0),
            a.title.as_str(),
        )
            .cmp(&(
                b.disc_no.unwrap_or(1),
                b.track_no.unwrap_or(0),
                b.title.as_str(),
            ))
    });
    // 音频缺失的曲目不能进专辑包,先滤掉(全部缺失时报错)
    tracks.retain(|t| Path::new(&t.path).is_file());
    if tracks.is_empty() {
        return Err("专辑中没有可导出的文件".into());
    }
    let album_name = tracks[0].album.clone();

    let workspace = staging.join(".tmp");
    let _ = fs::remove_dir_all(&workspace);
    fs::create_dir_all(&workspace).map_err(|e| e.to_string())?;

    let (mut assets, mut items) = plan_track_assets(&tracks, &workspace)?;

    // 专辑封面是专辑级属性,包内只保留一份:去重后放包根 cover.{ext},
    // 导入时再分发到各曲目目录(存在多种不同封面的特殊专辑退回逐曲目携带)
    let root_cover = dedupe_album_cover_assets(&mut assets);
    let mut items = match &root_cover {
        Some(_) => {
            for item in &mut items {
                if let Some(obj) = item.as_object_mut() {
                    obj.remove("cover");
                }
            }
            items
        }
        None => items,
    };

    // 7z 写进暂存区工作目录,写入进度按资源数上报;封口后落进暂存区顶层。
    // manifest 最后打包:曲目/封面 SHA 来自打包时的单遍哈希,每个文件只读一遍盘
    let manifest_path = workspace.join("album.json");
    let tmp_archive = workspace.join("album.tmca");
    pack_with_hashes(app, &tmp_archive, &assets, "TMCA", &mut |hashes| {
        inject_manifest_shas(&mut items, hashes);
        let cover_sha = root_cover.as_ref().and_then(|n| hashes.get(n));
        let manifest = serde_json::json!({
            "format": "tmca",
            "version": 1,
            "name": album_name,
            "albumArtist": tracks[0].album_artist,
            "year": tracks[0].year,
            "genre": tracks[0].genre,
            "cover": root_cover,
            "coverSha256": cover_sha,
            "createdAt": now_secs_f64(),
            "tracks": items,
        });
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        Ok(("album.json".to_string(), manifest_path.clone()))
    })?;
    // 包名取清洗后的专辑名;与暂存区内已有文件重名时自动加序号
    let staged = dedup_dest(staging.join(format!("{}.tmca", sanitize_name(&album_name))));
    place_archive(&tmp_archive, &staged)?;
    let _ = fs::remove_dir_all(&workspace);
    Ok(staged.to_string_lossy().to_string())
}

/// TMCA 专辑包封面去重:封面条目(名字形如 music/NN. xxx/{stem}.cover.{ext})按源文件去重,
/// 只有唯一来源时移除全部逐曲目副本、在资源表第 2 位插入包根 cover.{ext};
/// 存在多种不同封面(特殊版本)返回 None,调用方保持原样(正确性优先)
fn dedupe_album_cover_assets(assets: &mut ExportAssets) -> Option<String> {
    let cover_idxs: Vec<usize> = assets
        .iter()
        .enumerate()
        .filter(|(_, (name, _))| {
            // 封面条目必须以 ".cover." + 图片扩展结尾:标题本身含 ".cover." 的
            // 音频条目(如 "Track.cover Remix.mp3")不能被误当成封面移出资源表
            let n = Path::new(name)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            n.rsplit_once(".cover.")
                .map(|(_, ext)| matches!(ext, "jpg" | "jpeg" | "png" | "webp"))
                .unwrap_or(false)
        })
        .map(|(i, _)| i)
        .collect();
    if cover_idxs.is_empty() {
        return None;
    }
    let first_src = assets[cover_idxs[0]].1.clone();
    if cover_idxs.iter().any(|&i| assets[i].1 != first_src) {
        return None;
    }
    let ext = Path::new(&first_src)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("jpg")
        .to_lowercase();
    let root_name = format!("cover.{ext}");
    for &i in cover_idxs.iter().rev() {
        assets.remove(i);
    }
    assets.insert(1, (root_name.clone(), first_src));
    Some(root_name)
}

/// 把 TMCA 包根的单份封面复制进各曲目目录(无副本时才复制,避免重复覆盖)。
/// 扫描器按目录级 cover.{ext} 约定取用;包内无根封面(特殊多封面专辑)则不动
fn distribute_pack_cover(dest: &Path, tracks_json: &[serde_json::Value]) {
    let root = ["cover.jpg", "cover.png", "cover.webp", "cover.jpeg"]
        .iter()
        .map(|n| dest.join(n))
        .find(|p| p.is_file());
    let Some(root) = root else { return };
    let Some(name) = root.file_name() else { return };
    for item in tracks_json {
        let Some(dir) = item
            .get("dir")
            .and_then(|x| x.as_str())
            .and_then(safe_manifest_part)
        else {
            continue;
        };
        let dir_path = dest.join(dir);
        if !dir_path.is_dir() {
            continue;
        }
        let target = dir_path.join(name);
        if !target.exists() {
            let _ = fs::copy(&root, &target);
        }
    }
}

/// 依据 manifest 的 tracks 字段为每个子目录写 meta.json(扫描器据此补齐无内嵌标签的文件)。
/// 包内自带的 meta.json 信息更全(含 year/trackNo/genre),已存在时不覆盖
fn write_manifest_meta_files(dest: &Path, tracks_json: &[serde_json::Value]) {
    for item in tracks_json {
        let Some(dir) = item
            .get("dir")
            .and_then(|x| x.as_str())
            .and_then(safe_manifest_part)
        else {
            continue;
        };
        let dir_path = dest.join(dir);
        let _ = fs::create_dir_all(&dir_path);
        let meta_path = dir_path.join("meta.json");
        if meta_path.is_file() {
            continue;
        }
        let meta = serde_json::json!({
            "title": item.get("title"),
            "artist": item.get("artist"),
            "album": item.get("album"),
            "albumArtist": item.get("albumArtist"),
        });
        let _ = fs::write(
            meta_path,
            serde_json::to_string_pretty(&meta).unwrap_or_default(),
        );
    }
}

pub(crate) fn import_tmca_blocking(
    path: &str,
    app: &tauri::AppHandle,
) -> Result<PackImportReport, String> {
    let src = Path::new(path);
    if !src.is_file() {
        return Err("文件不存在".into());
    }
    let state = app.state::<AppState>();
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "album".into());
    let import_dir = state.data_dir.join(IMPORT_DIR_NAME);
    fs::create_dir_all(&import_dir).map_err(|e| e.to_string())?;
    let dest = import_dir.join(sanitize_name(&stem));
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| format!("清理旧目录失败: {e}"))?;
    }
    fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    sevenz_rust2::decompress_file(src, &dest).map_err(|e| format!("解包失败: {e}"))?;

    let manifest_path = dest.join("album.json");
    if !manifest_path.is_file() {
        return Err("不是有效的 TMCA:缺少 album.json".into());
    }
    let v: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("album.json 解析失败: {e}"))?;
    if v.get("format").and_then(|x| x.as_str()) != Some("tmca") {
        return Err("不是有效的 TMCA:format 不符".into());
    }
    let album_name = v
        .get("name")
        .and_then(|x| x.as_str())
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .unwrap_or_else(|| stem.clone());
    let tracks_json = v
        .get("tracks")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();

    // 逐文件 SHA-256 校验 + 与曲库内容去重
    let (tracks_json, skipped) = verify_and_dedupe_pack(&dest, tracks_json, &state)?;

    // 为无内嵌标签的文件补 meta.json(扫描器据此补齐元数据)
    write_manifest_meta_files(&dest, &tracks_json);

    // 包级封面分发到各曲目目录(目录级封面约定);曲目带内嵌封面时扫描器自动优先内嵌
    distribute_pack_cover(&dest, &tracks_json);

    // 扫描入库(与拖拽导入同一路径);专辑归类由 meta.json/内嵌标签成立
    scanner::run_scan(app)?;

    // 来源包记录的 SHA 写进曲库,供后续包导入做内容去重
    record_pack_shas(&state, &dest, &tracks_json)?;

    let _ = app.emit("library-changed", ());
    // 全部曲目都与曲库重复:不是错误,作为正常跳过返回(导入预览里明确提示)
    if tracks_json.is_empty() && skipped > 0 {
        return Ok(PackImportReport {
            name: album_name,
            imported: 0,
            skipped,
        });
    }
    Ok(PackImportReport {
        name: album_name,
        imported: tracks_json.len() as u32,
        skipped,
    })
}

// ===== TMCL 播放列表包(.tmcl = 标准 7z:音乐源文件 + 歌词 + 封面 + playlist.json) =====

pub(crate) fn stage_tmcl_blocking(
    playlist_id: &str,
    staging: &Path,
    app: &tauri::AppHandle,
) -> Result<String, String> {
    // 用户已取消(暂存目录被清理):拒绝,避免工作目录创建时把暂存目录重建出来
    if !staging.is_dir() {
        return Err("导出已取消".into());
    }
    let state = app.state::<AppState>();
    let (pl, tracks) = {
        let lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        let pl = lib
            .playlists
            .iter()
            .find(|p| p.id == playlist_id)
            .cloned()
            .ok_or("播放列表不存在")?;
        // 条目回查曲库:id 优先,路径兜底;两者都找不到则该曲目已不可用
        let mut by_id: std::collections::HashMap<&str, &model::Track> = HashMap::new();
        let mut by_path: std::collections::HashMap<String, &model::Track> = HashMap::new();
        for t in &lib.tracks {
            by_id.insert(t.id.as_str(), t);
            by_path.insert(norm_path(&t.path), t);
        }
        let tracks: Vec<model::Track> = pl
            .entries
            .iter()
            .filter_map(|e| {
                let t = by_id
                    .get(e.id.as_str())
                    .or_else(|| by_path.get(&norm_path(&e.path)));
                t.map(|t| (*t).clone())
            })
            .collect();
        (pl, tracks)
    };
    if tracks.is_empty() {
        return Err("播放列表为空或曲目文件均不可用".into());
    }

    let workspace = staging.join(".tmp");
    let _ = fs::remove_dir_all(&workspace);
    fs::create_dir_all(&workspace).map_err(|e| e.to_string())?;

    let (assets, mut items) = plan_track_assets(&tracks, &workspace)?;
    if items.is_empty() {
        let _ = fs::remove_dir_all(&workspace);
        return Err("播放列表中没有可导出的文件".into());
    }

    // 7z 写进暂存区工作目录,写入进度按资源数上报;封口后落进暂存区顶层。
    // manifest 最后打包:曲目 SHA 来自打包时的单遍哈希,每个文件只读一遍盘
    let manifest_path = workspace.join("playlist.json");
    let tmp_archive = workspace.join("playlist.tmcl");
    pack_with_hashes(app, &tmp_archive, &assets, "TMCL", &mut |hashes| {
        inject_manifest_shas(&mut items, hashes);
        let manifest = serde_json::json!({
            "format": "tmcl",
            "version": 1,
            "name": pl.name,
            "createdAt": pl.created_at,
            "tracks": items,
        });
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        Ok(("playlist.json".to_string(), manifest_path.clone()))
    })?;
    // 包名取清洗后的列表名;与暂存区内已有文件重名时自动加序号
    let staged = dedup_dest(staging.join(format!("{}.tmcl", sanitize_name(&pl.name))));
    place_archive(&tmp_archive, &staged)?;
    let _ = fs::remove_dir_all(&workspace);
    Ok(staged.to_string_lossy().to_string())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackImportReport {
    pub name: String,
    pub imported: u32,
    pub skipped: u32,
}

/// 路径规范化:统一分隔符为反斜杠并小写,用于跨来源路径比较
/// (manifest 里的相对路径用 '/',walkdir 产生 '';Windows 也不区分大小写)
fn norm_path(p: &str) -> String {
    p.replace('/', "\\").to_lowercase()
}

/// 校验解包后的文件与 manifest 记录的 SHA-256 是否一致,返回损坏文件相对路径清单。
/// 旧包没有哈希字段则跳过(向后兼容);名字字段先过 safe_manifest_part 防穿越
fn verify_manifest_hashes(dest: &Path, items: &[serde_json::Value]) -> Vec<String> {
    let mut bad: Vec<String> = Vec::new();
    for item in items {
        for (hash_key, name_key) in [
            ("sha256", "audio"),
            ("lrcSha256", "lrc"),
            ("coverSha256", "cover"),
        ] {
            let Some(expect) = item.get(hash_key).and_then(|x| x.as_str()) else {
                continue;
            };
            let Some(name) = item
                .get(name_key)
                .and_then(|x| x.as_str())
                .and_then(safe_manifest_part)
            else {
                continue;
            };
            let dir = item
                .get("dir")
                .and_then(|x| x.as_str())
                .and_then(safe_manifest_part)
                .unwrap_or_default();
            let file = dest.join(&dir).join(&name);
            let ok = sha256_hex(&file)
                .map(|h| h.eq_ignore_ascii_case(expect))
                .unwrap_or(false);
            if !ok {
                bad.push(format!("{dir}/{name}"));
            }
        }
    }
    bad
}

/// 包导入的哈希三合一:损坏校验(不一致→Err 拒绝入库)、内容去重(与曲库已记录
/// SHA 相同→删除解包产物并跳过)、返回保留条目与跳过数。旧包无哈希字段原样通过
fn verify_and_dedupe_pack(
    dest: &Path,
    items: Vec<serde_json::Value>,
    state: &AppState,
) -> Result<(Vec<serde_json::Value>, u32), String> {
    let bad = verify_manifest_hashes(dest, &items);
    if !bad.is_empty() {
        return Err(format!("包校验失败,文件已损坏: {}", bad.join("; ")));
    }
    let known: std::collections::HashSet<String> = {
        let lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        lib.tracks
            .iter()
            .filter_map(|t| t.sha256.as_deref().map(|s| s.to_ascii_lowercase()))
            .collect()
    };
    let mut kept: Vec<serde_json::Value> = Vec::with_capacity(items.len());
    let mut skipped = 0u32;
    for item in items {
        let sha = item
            .get("sha256")
            .and_then(|x| x.as_str())
            .map(|s| s.to_ascii_lowercase());
        let dir = item
            .get("dir")
            .and_then(|x| x.as_str())
            .and_then(safe_manifest_part);
        let _audio = item
            .get("audio")
            .and_then(|x| x.as_str())
            .and_then(safe_manifest_part);
        if let (Some(sha), Some(dir)) = (&sha, &dir) {
            if known.contains(sha) {
                // 内容与曲库已有曲目相同:每首歌一个独立目录,可安全整目录移除
                let _ = fs::remove_dir_all(dest.join(dir));
                skipped += 1;
                continue;
            }
        }
        kept.push(item);
    }
    Ok((kept, skipped))
}

/// 把 (文件路径, SHA) 补写进曲库记录(按路径匹配;路径可能因去重改名,做规范化比对,
/// 归位到「艺人\专辑」子目录的散装文件按文件名兜底)。
/// 之后扫描遇到未变化的文件会原样保留该哈希,跨包去重持续有效。
/// 包导入(record_pack_shas)与散装导入共用
pub(crate) fn record_track_shas(
    state: &AppState,
    pairs: &[(PathBuf, String)],
) -> Result<(), String> {
    if pairs.is_empty() {
        return Ok(());
    }
    let import_dir = state.data_dir.join(IMPORT_DIR_NAME);
    let import_str = import_dir.to_string_lossy().to_string();
    // 落盘要互斥扫描:此刻 lib.tracks 可能被扫描借走
    let _gate = lock_scan_gate(state);
    let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
    // 一次性建索引:此前每对 (path,sha) 都线性扫全库,还逐轨重复分配 norm_path,
    // 千级曲库 × 多文件导入时是平方级的重复分配与比较
    let mut by_norm: HashMap<String, usize> = HashMap::with_capacity(lib.tracks.len());
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, t) in lib.tracks.iter().enumerate() {
        // 精确匹配 ⇒ 规范化必匹配,故规范化索引记首个命中即可(与逐轨扫描的
        // position 语义一致);无文件名的极端路径进不了兜底索引,走不到兜底分支
        by_norm.entry(norm_path(&t.path)).or_insert(i);
        if t.sha256.is_none() && is_under(&t.path, &import_str) {
            if let Some(name) = Path::new(&t.path).file_name() {
                by_name
                    .entry(name.to_string_lossy().to_string())
                    .or_default()
                    .push(i);
            }
        }
    }
    for (path, sha) in pairs {
        let abs_str = path.to_string_lossy().replace('/', "\\");
        let abs_norm = norm_path(&abs_str);
        let sha = sha.to_ascii_lowercase();
        // 先按记录时的原始路径精确/规范化匹配;散装导入的文件随后会被
        // organize_top_level 归位到「艺人\专辑」子目录,指纹路径已失配——
        // 退一步按文件名在导入目录下重新定位(取登记顺序里尚无指纹的那个,
        // 本轮已写入指纹的要从候选里跳过)
        let idx = by_norm.get(&abs_norm).copied().or_else(|| {
            let name = path.file_name()?.to_string_lossy().to_string();
            by_name
                .get(&name)?
                .iter()
                .copied()
                .find(|&i| lib.tracks[i].sha256.is_none())
        });
        if let Some(i) = idx {
            lib.tracks[i].sha256 = Some(sha);
        }
    }
    // 锁内只序列化,磁盘 IO 在锁外
    let bytes = lib.to_bytes().map_err(|e| e.to_string())?;
    drop(lib);
    Library::write_bytes(&state.data_dir.join(LIBRARY_FILE), &bytes).map_err(|e| e.to_string())
}

fn record_pack_shas(
    state: &AppState,
    dest: &Path,
    items: &[serde_json::Value],
) -> Result<(), String> {
    let mut pairs: Vec<(PathBuf, String)> = Vec::new();
    for item in items {
        let (sha, dir, audio) = (
            item.get("sha256").and_then(|x| x.as_str()),
            item.get("dir")
                .and_then(|x| x.as_str())
                .and_then(safe_manifest_part),
            item.get("audio")
                .and_then(|x| x.as_str())
                .and_then(safe_manifest_part),
        );
        let (Some(sha), Some(dir), Some(audio)) = (sha, dir, audio) else {
            continue;
        };
        pairs.push((dest.join(dir).join(audio), sha.to_string()));
    }
    record_track_shas(state, &pairs)
}

/// 校验 TMCL manifest 里的相对目录/文件名:拒绝绝对路径、盘符前缀与 `..` 组件。
/// 这是 zip-slip 的第二道闸——解包库(sevenz-rust2 safe_join)管住压缩包条目,
/// 这里管住 playlist.json 的字段,防止恶意包把 meta.json 写到导入目录之外
fn safe_manifest_part(part: &str) -> Option<String> {
    use std::path::Component;
    let normalized = part.replace('\\', "/");
    // Windows 盘符前缀(C:/...)在非 Windows 平台不是 Prefix 组件而是普通名字,
    // 必须手动识别,否则 Linux 上会让 "C:/evil" 混过校验(冒号在 Windows
    // 文件名里本就非法,不存在误伤的合法目录名)
    let b = normalized.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return None;
    }
    let mut out: Vec<&str> = Vec::new();
    for c in Path::new(&normalized).components() {
        match c {
            Component::Normal(p) => out.push(p.to_str()?),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!out.is_empty()).then(|| out.join("/"))
}

pub(crate) fn import_playlist_tmcl_blocking(
    path: &str,
    app: &tauri::AppHandle,
) -> Result<PackImportReport, String> {
    let src = Path::new(path);
    if !src.is_file() {
        return Err("文件不存在".into());
    }
    let state = app.state::<AppState>();
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "playlist".into());
    let import_dir = state.data_dir.join(IMPORT_DIR_NAME);
    fs::create_dir_all(&import_dir).map_err(|e| e.to_string())?;
    let dest = import_dir.join(sanitize_name(&stem));
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| format!("清理旧目录失败: {e}"))?;
    }
    fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    sevenz_rust2::decompress_file(src, &dest).map_err(|e| format!("解包失败: {e}"))?;

    let manifest_path = dest.join("playlist.json");
    if !manifest_path.is_file() {
        return Err("不是有效的 TMCL:缺少 playlist.json".into());
    }
    let v: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("playlist.json 解析失败: {e}"))?;
    let list_name = v
        .get("name")
        .and_then(|x| x.as_str())
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .unwrap_or_else(|| stem.clone());
    let tracks_json = v
        .get("tracks")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();

    // 逐文件 SHA-256 校验 + 与曲库内容去重
    let (tracks_json, skipped) = verify_and_dedupe_pack(&dest, tracks_json, &state)?;

    // 为无内嵌标签的文件写 meta.json(扫描器据此补齐元数据)
    write_manifest_meta_files(&dest, &tracks_json);

    // 扫描入库(与拖拽导入同一路径)
    scanner::run_scan(app)?;

    // 来源包记录的 SHA 写进曲库,供后续包导入做内容去重
    record_pack_shas(&state, &dest, &tracks_json)?;

    // 全部曲目都与曲库重复:不建空歌单,作为正常跳过返回(导入预览里明确提示)
    if tracks_json.is_empty() && skipped > 0 {
        return Ok(PackImportReport {
            name: list_name,
            imported: 0,
            skipped,
        });
    }
    // 建立播放列表:优先用扫描后的曲库数据,否则退回 manifest 快照
    let mut entries: Vec<model::PlaylistEntry> = Vec::new();
    for item in &tracks_json {
        let Some(dir) = item
            .get("dir")
            .and_then(|x| x.as_str())
            .and_then(safe_manifest_part)
        else {
            continue;
        };
        let Some(audio) = item
            .get("audio")
            .and_then(|x| x.as_str())
            .and_then(safe_manifest_part)
        else {
            continue;
        };
        let abs = dest.join(dir).join(audio);
        if !abs.is_file() {
            continue;
        }
        // 统一分隔符:manifest 用 '/',walkdir 的 track.path 用 '\'
        let abs_str = abs.to_string_lossy().replace('/', "\\");
        let abs_norm = norm_path(&abs_str);
        let from_lib = {
            let lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
            lib.tracks
                .iter()
                .find(|t| t.path == abs_str || norm_path(&t.path) == abs_norm)
                .cloned()
        };
        entries.push(match from_lib {
            Some(t) => entry_of(&t),
            None => model::PlaylistEntry {
                id: String::new(),
                path: abs_str,
                title: item
                    .get("title")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                artist: item
                    .get("artist")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                album: item
                    .get("album")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                album_artist: item
                    .get("albumArtist")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                duration: item.get("duration").and_then(|x| x.as_f64()).unwrap_or(0.0),
            },
        });
    }
    if entries.is_empty() {
        if skipped > 0 {
            return Err("歌单内全部曲目都已存在,已跳过导入".into());
        }
        return Err("TMCL 中没有可导入的曲目".into());
    }

    let imported = entries.len() as u32;
    let final_name = {
        // 闸只罩建歌单+落盘这一段:前面已调过 run_scan(它自己拿闸),
        // 若整函数持闸,内部的 run_scan try_lock 必然失败
        let _gate = lock_scan_gate(&state);
        let mut lib = state.lib.lock().map_err(|_| "内部状态不可用".to_string())?;
        let name = unique_playlist_name(&lib.playlists, &list_name);
        lib.playlists.push(model::Playlist {
            id: new_playlist_id(&name),
            name: name.clone(),
            created_at: now_secs_f64(),
            entries,
        });
        let bytes = lib.to_bytes().map_err(|e| e.to_string())?;
        drop(lib);
        Library::write_bytes(&state.data_dir.join(LIBRARY_FILE), &bytes)
            .map_err(|e| e.to_string())?;
        name
    };
    let _ = app.emit("library-changed", ());
    Ok(PackImportReport {
        name: final_name,
        imported,
        skipped,
    })
}

/// io::Write 包装:把写入的字节流喂给 SHA-256
struct Sha256Writer(sha2::Sha256);
impl std::io::Write for Sha256Writer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// 流式计算文件 SHA-256(十六进制小写);音频可达数百 MB,io::copy 分块搬运不驻留大内存
pub(crate) fn sha256_hex(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut w = Sha256Writer(sha2::Sha256::new());
    std::io::copy(&mut file, &mut w).ok()?;
    // 逐字节十六进制编码(sha2 0.11 的摘要类型不再实现 LowerHex)
    Some(to_hex(w.0.finalize().as_slice()))
}

#[cfg(test)]
mod tests {
    use super::{is_under, place_archive, sanitize_name};
    use crate::model;
    use std::fs;

    #[test]
    fn record_track_shas_matches_normalized_path_and_falls_back_by_name() {
        use super::record_track_shas;
        use crate::import::IMPORT_DIR_NAME;
        use std::path::PathBuf;
        use std::sync::Mutex;

        let dir = std::env::temp_dir().join(format!("tm-shas-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let import_root = dir.join(IMPORT_DIR_NAME);
        let moved = import_root.join("艺人").join("专辑").join("song.mp3");
        let state = crate::AppState {
            lib: Mutex::new(model::Library {
                tracks: vec![
                    // 大小写/分隔符变体:规范化路径索引必须命中
                    model::Track {
                        id: "t1".into(),
                        title: "T1".into(),
                        path: r"F:\Music\Song.MP3".into(),
                        ..Default::default()
                    },
                    // 散装导入后已被归位到「艺人\专辑」子目录,记录时路径已失配
                    model::Track {
                        id: "t2".into(),
                        title: "T2".into(),
                        path: moved.to_string_lossy().to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            data_dir: dir.clone(),
            cache_dir: dir.join("cache"),
            scan_gate: Mutex::new(()),
            stats: Mutex::new(crate::stats::StatsData::empty()),
            pending_imports: Mutex::new(Vec::new()),
        };
        let pairs = vec![
            // 记录时路径与曲库只差大小写/分隔符
            (PathBuf::from("f:/music/song.mp3"), "AAA".to_string()),
            // 归位前的原始路径(顶层),只能按文件名在导入目录下兜底
            (import_root.join("song.mp3"), "BBB".to_string()),
        ];

        record_track_shas(&state, &pairs).unwrap();

        let lib = state.lib.lock().unwrap();
        assert_eq!(
            lib.tracks[0].sha256.as_deref(),
            Some("aaa"),
            "规范化路径命中应写入小写指纹"
        );
        assert_eq!(
            lib.tracks[1].sha256.as_deref(),
            Some("bbb"),
            "归位失配应按文件名兜底命中"
        );
        // 指纹随落库持久化
        let back = model::Library::load(&dir.join(crate::LIBRARY_FILE));
        assert_eq!(back.tracks[0].sha256.as_deref(), Some("aaa"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn is_under_uses_path_components() {
        // 字符串前缀匹配会把 F:\music2 误当成 F:\music 的子目录,组件匹配不会
        assert!(is_under(r"F:\music\a.mp3", r"F:\music"));
        assert!(is_under(r"F:\music\sub\b.mp3", r"F:\music"));
        assert!(is_under(r"F:\music", r"F:\music"));
        assert!(!is_under(r"F:\music2\a.mp3", r"F:\music"));
        assert!(!is_under(r"F:\musica\c.mp3", r"F:\music"));
        // 正斜杠形式(跨平台)
        assert!(is_under("F:/music/a.mp3", r"F:\music"));
        assert!(!is_under("/music/other/x.mp3", "/music/sub"));
        // 空目录参数不构成包含关系
        assert!(!is_under(r"F:\music\a.mp3", ""));
        // 盘符大小写不敏感仅限 Windows
        #[cfg(windows)]
        assert!(is_under(r"f:\MUSIC\a.mp3", r"F:\music"));
        #[cfg(not(windows))]
        assert!(!is_under(r"f:\MUSIC\a.mp3", r"F:\music"));
    }

    #[test]
    fn place_archive_moves_tmp_into_place() {
        let dir = std::env::temp_dir().join(format!("tm-place-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let stage = dir.join("stage");
        let target = dir.join("target");
        fs::create_dir_all(&stage).unwrap();
        fs::create_dir_all(&target).unwrap();

        // 常规:临时包落位(同盘 rename),临时文件消失
        let tmp1 = stage.join("a.tmc");
        fs::write(&tmp1, b"archive-1").unwrap();
        place_archive(&tmp1, &target.join("a.tmc")).unwrap();
        assert!(target.join("a.tmc").is_file());
        assert!(!tmp1.exists());

        // 目标已存在:rename 失败自动退化为拷贝覆盖(单文件 TMCL 用户确认覆盖的场景)
        let tmp2 = stage.join("a.tmc");
        fs::write(&tmp2, b"archive-2").unwrap();
        place_archive(&tmp2, &target.join("a.tmc")).unwrap();
        assert_eq!(fs::read(target.join("a.tmc")).unwrap(), b"archive-2");
        assert!(!tmp2.exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dedup_dest_appends_sequence() {
        use super::dedup_dest;
        let dir = std::env::temp_dir().join(format!("tm-dedupdest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // 不存在:原样返回
        assert_eq!(dedup_dest(dir.join("song.tmc")), dir.join("song.tmc"));
        // 已存在:追加序号,扩展名保留;新序号也存在时继续递增
        fs::write(dir.join("song.tmc"), b"a").unwrap();
        assert_eq!(dedup_dest(dir.join("song.tmc")), dir.join("song (2).tmc"));
        fs::write(dir.join("song (2).tmc"), b"b").unwrap();
        assert_eq!(dedup_dest(dir.join("song.tmc")), dir.join("song (3).tmc"));
        // 不同扩展名互不干扰
        fs::write(dir.join("song.tmcl"), b"c").unwrap();
        assert_eq!(dedup_dest(dir.join("song.tmcl")), dir.join("song (2).tmcl"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tmc_store_archive_roundtrips_via_own_decoder() {
        // 导出统一走 Copy(存储)模式,打包结果必须能被应用自身的解包路径读回
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod};

        let dir = std::env::temp_dir().join(format!("tm-7z-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("in.bin");
        fs::write(&src, vec![7u8; 4096]).unwrap();

        let archive = dir.join("out.tmc");
        let mut writer = ArchiveWriter::create(&archive).unwrap();
        writer.set_content_methods(vec![EncoderConfiguration::new(EncoderMethod::COPY)]);
        let file = fs::File::open(&src).unwrap();
        writer
            .push_archive_entry(
                ArchiveEntry::from_path(&src, "a.bin".to_string()),
                Some(file),
            )
            .unwrap();
        writer.finish().unwrap();

        let out = dir.join("roundtrip");
        sevenz_rust2::decompress_file(&archive, &out).unwrap();
        assert_eq!(fs::read(out.join("a.bin")).unwrap(), vec![7u8; 4096]);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tmc_import_rejects_zip_slip_entry_names() {
        // RUSTSEC-2026-0245 的回归防线:恶意包的穿越条目名必须被拒收,文件绝不落盘
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter};

        let dir = std::env::temp_dir().join(format!("tm-slip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("payload.bin");
        fs::write(&src, b"evil").unwrap();

        let archive = dir.join("evil.tmc");
        let mut writer = ArchiveWriter::create(&archive).unwrap();
        for name in [
            "..\\escaped.txt",
            "../../../escaped.txt",
            "C:\\Temp\\escaped.txt",
        ] {
            let file = fs::File::open(&src).unwrap();
            writer
                .push_archive_entry(ArchiveEntry::from_path(&src, name.to_string()), Some(file))
                .unwrap();
        }
        writer.finish().unwrap();

        let dest = dir.join("out");
        assert!(sevenz_rust2::decompress_file(&archive, &dest).is_err());
        // 无论逃逸到解包目录的哪里,都不允许出现落盘文件
        assert!(!dir.join("escaped.txt").exists());
        assert!(!dest.join("escaped.txt").exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn manifest_part_rejects_traversal_and_accepts_normal() {
        use super::safe_manifest_part;
        // 正常的 manifest 字段(与导出侧生成格式一致)
        assert_eq!(
            safe_manifest_part("music/01. 海阔天空 - Beyond"),
            Some("music/01. 海阔天空 - Beyond".to_string())
        );
        assert_eq!(safe_manifest_part("a\\b\\c"), Some("a/b/c".to_string()));
        // 穿越与绝对路径一律拒绝
        assert_eq!(safe_manifest_part("../evil"), None);
        assert_eq!(safe_manifest_part("a/../../evil"), None);
        assert_eq!(safe_manifest_part(r"..\..\evil"), None);
        assert_eq!(safe_manifest_part(r"C:\Temp\evil"), None);
        assert_eq!(safe_manifest_part("/etc/evil"), None);
        assert_eq!(safe_manifest_part(""), None);
        assert_eq!(safe_manifest_part("."), None);
    }

    #[test]
    fn sanitize_replaces_invalid_filename_chars() {
        assert_eq!(
            sanitize_name("a/b\\c:d*e?f\"g<h>i|j"),
            "a_b_c_d_e_f_g_h_i_j"
        );
        assert_eq!(sanitize_name("海阔天空"), "海阔天空");
        assert_eq!(sanitize_name("  Beyond  "), "Beyond");
        // 结尾的点与空格被 Windows 静默吞掉,一并去掉
        assert_eq!(sanitize_name("海阔天空. "), "海阔天空");
        // Windows 保留名加前缀(主名比对,带扩展名的 CON.mp3 同样保留)
        assert_eq!(sanitize_name("CON"), "_CON");
        assert_eq!(sanitize_name("com1"), "_com1");
        assert_eq!(sanitize_name("CON.mp3"), "_CON.mp3");
        assert_eq!(sanitize_name("my.CON"), "my.CON");
        // 控制字符剔除,空结果回退默认名
        assert_eq!(sanitize_name("\u{0}\u{1}"), "track");
        assert_eq!(sanitize_name(""), "track");
    }

    #[test]
    fn sha256_hex_matches_known_vectors() {
        use super::sha256_hex;
        let dir = std::env::temp_dir().join(format!("tm-sha-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // 标准测试向量:空文件与 "abc"
        let empty = dir.join("empty.bin");
        fs::write(&empty, b"").unwrap();
        assert_eq!(
            sha256_hex(&empty).as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        let abc = dir.join("abc.bin");
        fs::write(&abc, b"abc").unwrap();
        assert_eq!(
            sha256_hex(&abc).as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn verify_manifest_hashes_detects_tampering() {
        use super::{sha256_hex, verify_manifest_hashes};
        let dir = std::env::temp_dir().join(format!("tm-verify-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("music/01")).unwrap();
        let audio = dir.join("music/01").join("a.mp3");
        fs::write(&audio, b"hello").unwrap();
        let sha = sha256_hex(&audio).unwrap();
        let item = serde_json::json!({ "dir": "music/01", "audio": "a.mp3", "sha256": sha });
        assert!(verify_manifest_hashes(&dir, std::slice::from_ref(&item)).is_empty());
        // 篡改后必须被发现并报出相对路径
        fs::write(&audio, b"hellO").unwrap();
        assert_eq!(
            verify_manifest_hashes(&dir, &[item]),
            vec!["music/01/a.mp3"]
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn plan_track_assets_shapes_manifest_items() {
        use super::plan_track_assets;
        let dir = std::env::temp_dir().join(format!("tm-plan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("a.wav");
        fs::write(&audio, b"riff-wave-bytes").unwrap();
        let track = model::Track {
            path: audio.to_string_lossy().to_string(),
            title: "测试".into(),
            artist: "艺人".into(),
            ..Default::default()
        };
        let (_, items) = plan_track_assets(&[track], &dir).unwrap();
        assert_eq!(items.len(), 1);
        // SHA 字段不在规划期产生:由打包阶段单遍收集后经 inject_manifest_shas 注入
        assert_eq!(items[0]["audio"], "测试 - 艺人.wav");
        assert!(items[0]["sha256"].is_null());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn inject_manifest_shas_matches_by_archive_name() {
        use super::inject_manifest_shas;
        use std::collections::HashMap;
        let mut items = vec![serde_json::json!({
            "dir": "music/01. a - b",
            "audio": "a - b.mp3",
            "lrc": "a - b.lrc",
            "cover": "a - b.cover.jpg",
        })];
        let mut hashes = HashMap::new();
        hashes.insert("a - b.mp3".to_string(), "hash-audio".to_string());
        hashes.insert("a - b.lrc".to_string(), "hash-lrc".to_string());
        hashes.insert("a - b.cover.jpg".to_string(), "hash-cover".to_string());
        inject_manifest_shas(&mut items, &hashes);
        assert_eq!(items[0]["sha256"], "hash-audio");
        assert_eq!(items[0]["lrcSha256"], "hash-lrc");
        assert_eq!(items[0]["coverSha256"], "hash-cover");
        // 没有对应哈希的字段不注入
        let mut empty = vec![serde_json::json!({"audio": "missing.mp3"})];
        inject_manifest_shas(&mut empty, &hashes);
        assert!(empty[0]["sha256"].is_null());
    }
}
