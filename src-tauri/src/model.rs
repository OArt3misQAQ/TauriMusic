use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// 一首曲目。字段名经 serde 转换后与前端 TypeScript 接口一一对应。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    /// 时长(秒)
    pub duration: f64,
    /// 音频文件绝对路径
    pub path: String,
    /// 缓存后的封面小图绝对路径(256px,列表/网格用)
    pub cover: Option<String>,
    /// 缓存后的封面大图绝对路径(512px,详情页/全屏播放页用)
    pub cover_large: Option<String>,
    /// 内容 SHA-256(仅来自音乐包导入的曲目有记录,用于跨包去重);文件变化后失效
    #[serde(default)]
    pub sha256: Option<String>,
    /// 是否存在可用歌词(内嵌或同名 .lrc)
    pub has_lyrics: bool,
    /// 同名 .lrc 伴生文件路径
    pub lrc_path: Option<String>,
    /// 首次入库时间(unix 秒)
    pub added_at: f64,
    /// 文件修改时间(unix 秒),用于增量扫描
    pub mtime: f64,
    pub size: u64,
}

/// 播放列表条目:曲库曲目引用(id 优先,路径兜底)+ 元数据快照(曲目失效时仍可展示)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlaylistEntry {
    pub id: String,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub duration: f64,
}

/// 用户播放列表
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    /// 创建时间(unix 秒)
    pub created_at: f64,
    pub entries: Vec<PlaylistEntry>,
}

/// 回收站条目:删除时音频移入应用回收站目录,元数据快照用于展示与还原。
/// 与系统回收站不同,还原由应用自己完成(把文件移回原路径并恢复曲库记录)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TrashEntry {
    /// 曲库记录快照(path 保留原路径,还原后沿用)
    pub track: Track,
    /// 回收站目录内的音频文件路径
    pub trashed_audio: String,
    /// 回收站目录内的歌词文件路径(若有)
    pub trashed_lrc: Option<String>,
    /// 删除时间(unix 秒),超过保留期自动清理
    pub deleted_at: f64,
}

/// 整个曲库,持久化为 app_data/library.json
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Library {
    pub version: u32,
    pub folders: Vec<String>,
    pub tracks: Vec<Track>,
    pub playlists: Vec<Playlist>,
    /// 应用内回收站(删除的音乐在这里保留一段时间,可还原)
    #[serde(default)]
    pub trash: Vec<TrashEntry>,
}

impl Library {
    /// 读取曲库:主文件损坏(截断/非法 JSON)时自动回退同名 .bak 备份。
    /// 两份都不可用才返回空库——此时调用方应让用户感知,而不是无声清空。
    pub fn load(path: &Path) -> Library {
        if let Some(lib) = Self::try_read(path) {
            return lib;
        }
        let bak = backup_path(path);
        if let Some(lib) = Self::try_read(&bak) {
            eprintln!("[library] 主文件损坏,已从备份恢复: {}", bak.display());
            return lib;
        }
        Library::default()
    }

    fn try_read(path: &Path) -> Option<Library> {
        let bytes = fs::read(path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// 纯序列化,不做任何 IO:调用方在持有曲库锁期间只允许调这个,
    /// 磁盘写入交给锁外的 write_bytes(整库 JSON 可达数 MB,fsync/两次 rename
    /// 在慢盘上是几十毫秒级,不该占着锁挡住其他命令)
    pub fn to_bytes(&self) -> std::io::Result<Vec<u8>> {
        serde_json::to_vec_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }

    /// 原子落盘:先写临时文件,把现有文件备份为 .bak,再 rename 落盘。
    /// 任何一步失败都不会破坏已有数据。必须在锁外调用(内部有 fsync 与两次 rename)。
    /// 与 to_bytes 配套使用:锁内 to_bytes → drop guard → write_bytes
    pub fn write_bytes(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let tmp = tmp_path(path);
        {
            // 数据先 sync 再谈 rename:掉电时 rename 可能先于内容写入持久层,
            // 最坏情况是"最新一次保存丢失"(有 .bak 兜底),不能更糟
            let mut f = fs::File::create(&tmp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
        }

        // 旧文件先转存为 .bak(首次保存没有旧文件,跳过)。
        // 备份失败必须中止:否则主文件一旦损坏将无备份可回退
        if path.exists() {
            let bak = backup_path(path);
            fs::rename(path, &bak)?;
        }
        // rename 同卷内原子替换;失败时退回直接覆盖,内容是完整 JSON 即视为保存成功
        // (若此时仍返回 Err,调用方会回滚内存,而磁盘已是新数据——内存/磁盘反而分叉)
        match fs::rename(&tmp, path) {
            Ok(()) => Ok(()),
            Err(_) => {
                fs::write(path, bytes)?;
                let _ = fs::remove_file(&tmp);
                Ok(())
            }
        }
    }
}

/// 原子写的临时文件伴生路径(同目录 <name>.tmp)。
/// library.json 与 stats.json 共用同一套 tmp→bak→rename 语义,规则必须一致,
/// 因此下沉到 model 统一维护(此前两处各写一份,已有漂移风险)
pub(crate) fn tmp_path(path: &Path) -> std::path::PathBuf {
    sibling_path(path, ".tmp")
}

/// 原子写的备份伴生路径(同目录 <name>.bak)
pub(crate) fn backup_path(path: &Path) -> std::path::PathBuf {
    sibling_path(path, ".bak")
}

fn sibling_path(path: &Path, suffix: &str) -> std::path::PathBuf {
    // 无文件名的极端情形(路径以分隔符结尾)实际不可达,给个无害占位名即可
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    path.with_file_name(format!("{name}{suffix}"))
}

/// 扫描结果报告
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub total: usize,
    pub errors: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_save_load_roundtrip() {
        let lib = Library {
            version: 2,
            folders: vec!["F:\\music".into()],
            trash: vec![],
            playlists: vec![Playlist {
                id: "pl1".into(),
                name: "我的歌单".into(),
                created_at: 1759500000.0,
                entries: vec![PlaylistEntry {
                    id: "abc123".into(),
                    path: "F:\\music\\晴天.mp3".into(),
                    title: "晴天".into(),
                    artist: "周杰伦".into(),
                    ..Default::default()
                }],
            }],
            tracks: vec![Track {
                id: "abc123".into(),
                title: "晴天".into(),
                artist: "周杰伦".into(),
                album: "叶惠美".into(),
                track_no: Some(3),
                duration: 269.2,
                path: "F:\\music\\晴天.mp3".into(),
                mtime: 1759000000.5,
                size: 1024,
                ..Default::default()
            }],
        };
        let path = std::env::temp_dir().join(format!("tm-lib-{}.json", std::process::id()));
        Library::write_bytes(&path, &lib.to_bytes().unwrap()).unwrap();
        let back = Library::load(&path);
        assert_eq!(back.version, 2);
        assert_eq!(back.folders, lib.folders);
        assert_eq!(back.tracks[0].title, "晴天");
        assert_eq!(back.tracks[0].track_no, Some(3));
        assert_eq!(back.tracks[0].mtime, 1759000000.5);
        assert_eq!(back.playlists[0].name, "我的歌单");
        assert_eq!(back.playlists[0].entries[0].title, "晴天");
        fs::remove_file(&path).ok();
        fs::remove_file(backup_path(&path)).ok();
    }

    #[test]
    fn library_load_missing_file_returns_default() {
        let lib = Library::load(Path::new("Z:\\nonexistent\\tm-lib.json"));
        assert!(lib.tracks.is_empty());
        assert_eq!(lib.version, 0);
    }

    #[test]
    fn to_bytes_is_pure_serialization_without_io() {
        // 拆分后的约定:to_bytes 不做任何 IO,落盘全部走锁外 write_bytes。
        // 若 to_bytes 悄悄碰了文件系统,锁内调用会重新引入持锁 IO
        let lib = Library {
            version: 1,
            ..Default::default()
        };
        let dir = std::env::temp_dir().join(format!("tm-lib-pure-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("library.json");
        let bytes = lib.to_bytes().unwrap();
        assert!(!path.exists(), "to_bytes 不得产生文件");
        assert!(!tmp_path(&path).exists() && !backup_path(&path).exists());
        Library::write_bytes(&path, &bytes).unwrap();
        assert_eq!(Library::load(&path).version, 1, "字节应能原样落盘读回");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn library_load_recovers_from_backup_on_corruption() {
        let dir = std::env::temp_dir().join(format!("tm-lib-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("library.json");
        let bak = backup_path(&path);

        let lib = Library {
            version: 2,
            folders: vec!["F:\\music".into()],
            trash: vec![],
            playlists: vec![],
            tracks: vec![Track {
                id: "x".into(),
                title: "测试曲目".into(),
                ..Default::default()
            }],
        };
        // 真实使用节奏:至少保存两次后主文件与备份同时存在
        let bytes = lib.to_bytes().unwrap();
        Library::write_bytes(&path, &bytes).unwrap();
        Library::write_bytes(&path, &bytes).unwrap();
        assert!(bak.exists(), "覆盖保存后应留有 .bak");

        // 模拟写一半崩溃:主文件截断损坏
        fs::write(&path, b"{\"version\":2,\"folders\":[").unwrap();
        let recovered = Library::load(&path);
        assert_eq!(recovered.version, 2);
        assert_eq!(recovered.folders, vec!["F:\\music".to_string()]);
        assert_eq!(recovered.tracks.len(), 1);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn library_save_keeps_valid_content_on_successive_writes() {
        let dir = std::env::temp_dir().join(format!("tm-lib-seq-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("library.json");

        let first = Library {
            version: 1,
            folders: vec![],
            trash: vec![],
            playlists: vec![],
            tracks: vec![Track {
                id: "a".into(),
                title: "A".into(),
                ..Default::default()
            }],
        };
        let first_bytes = first.to_bytes().unwrap();
        Library::write_bytes(&path, &first_bytes).unwrap();
        let second = Library {
            version: 2,
            folders: vec![],
            trash: vec![],
            playlists: vec![],
            tracks: vec![Track {
                id: "b".into(),
                title: "B".into(),
                ..Default::default()
            }],
        };
        let second_bytes = second.to_bytes().unwrap();
        Library::write_bytes(&path, &second_bytes).unwrap();
        Library::write_bytes(&path, &second_bytes).unwrap(); // 连续保存也稳定

        let back = Library::load(&path);
        assert_eq!(back.version, 2);
        assert_eq!(back.tracks[0].title, "B");

        fs::remove_dir_all(&dir).ok();
    }
}
