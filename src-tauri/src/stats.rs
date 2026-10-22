//! 听歌统计:仅本机落盘(app_data/stats.json),全链路零联网。
//! 采集方在前端(只有 WebView 知道音频是否真的在播),后端只负责合并与
//! 原子持久化(与 library.json 同一套 tmp→bak→rename + 损坏回退)。
//! 本地歌手/专辑聚合不落盘,由前端按曲库即时推导——标签修改后统计自动跟随;
//! 在线曲目不在曲库,元数据随冲刷写入 onlineMeta,供统计页榜单渲染。

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const STATS_FILE: &str = "stats.json";

/// 单曲统计:累计收听秒数、播放次数(前端按「≥半曲或 ≥4 分钟」判定)、最后收听时刻
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TrackStat {
    pub sec: f64,
    pub count: u32,
    /// unix 秒
    pub last: u64,
}

/// 在线曲目元数据:在线曲目不在曲库,统计页的榜单只能靠播放时随冲刷落盘
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct OnlineTrackMeta {
    pub title: String,
    pub artist: String,
    pub album: String,
}

/// 当前结构版本:2 起每日时长按本地/在线分桶,并保存在线曲目元数据
const STATS_VERSION: u32 = 2;

/// 统计数据:每日总时长 + 本地/在线分桶 + 单曲明细
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct StatsData {
    /// 结构版本(当前 2,未来字段演进用)
    pub version: u32,
    /// 本地日期(YYYY-MM-DD)→ 当日收听秒数(本地+在线总和,向后兼容)
    pub days: BTreeMap<String, f64>,
    /// Track.id(本地为路径 hash;在线为 net:/qq:/kugou:/kuwo: 前缀)→ 单曲统计
    pub tracks: BTreeMap<String, TrackStat>,
    /// 本地日期 → 本地曲目当日秒数
    pub days_local: BTreeMap<String, f64>,
    /// 本地日期 → 在线曲目当日秒数
    pub days_online: BTreeMap<String, f64>,
    /// 在线曲目 id(`net:` 前缀)→ 展示用元数据
    pub online_meta: BTreeMap<String, OnlineTrackMeta>,
}

/// 当前 unix 秒;时钟早于 1970 等异常情形按 0 处理
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl StatsData {
    /// 空统计(当前结构版本):新装、清除或两份文件都损坏时使用
    pub fn empty() -> StatsData {
        StatsData {
            version: STATS_VERSION,
            ..Default::default()
        }
    }

    /// 读取统计:主文件损坏(截断/非法 JSON)时回退 .bak,两份都坏则从零开始。
    /// 统计是可再生的观测数据,损坏不值得让用户感知,静默重开即可
    pub fn load(path: &Path) -> StatsData {
        if let Some(s) = Self::try_read(path) {
            return s.migrated();
        }
        let bak = backup_path(path);
        if let Some(s) = Self::try_read(&bak) {
            eprintln!("[stats] 主文件损坏,已从备份恢复: {}", bak.display());
            return s.migrated();
        }
        StatsData::empty()
    }

    /// v1 → v2:旧数据没有本地/在线分桶,而 v1 时代只有本地播放,
    /// 因此把历史每日总时长整体归入本地桶,在线桶空
    fn migrated(mut self) -> StatsData {
        if self.version < STATS_VERSION {
            if self.days_local.is_empty() && !self.days.is_empty() {
                self.days_local = self.days.clone();
            }
            self.version = STATS_VERSION;
        }
        self
    }

    fn try_read(path: &Path) -> Option<StatsData> {
        let bytes = fs::read(path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// 原子保存:先写临时文件,现有文件备份为 .bak,再 rename 落盘
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_vec(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let tmp = tmp_path(path);
        fs::write(&tmp, &json)?;
        if path.exists() {
            let _ = fs::rename(path, backup_path(path));
        }
        match fs::rename(&tmp, path) {
            Ok(()) => Ok(()),
            // rename 失败退回直接覆盖:至少落盘的是完整 JSON
            Err(e) => {
                fs::write(path, &json)?;
                let _ = fs::remove_file(&tmp);
                Err(e)
            }
        }
    }

    /// 合并一次冲刷:每曲累加收听秒数与播放次数,当日时长累加。
    /// last 取后端时钟,不信任前端传值
    pub fn merge(
        &mut self,
        samples: &HashMap<String, f64>,
        plays: &[String],
        day: &str,
        day_sec: f64,
        online_meta: &HashMap<String, OnlineTrackMeta>,
        now: u64,
    ) {
        let mut local_sec = 0.0;
        let mut online_sec = 0.0;
        for (id, sec) in samples {
            if *sec <= 0.0 {
                continue;
            }
            let s = self.tracks.entry(id.clone()).or_default();
            s.sec += sec;
            s.last = s.last.max(now);
            if crate::online::is_online_id(id) {
                online_sec += sec;
            } else {
                local_sec += sec;
            }
        }
        for id in plays {
            let s = self.tracks.entry(id.clone()).or_default();
            s.count = s.count.saturating_add(1);
            s.last = s.last.max(now);
        }
        // 只接受在线曲目元数据,避免前端误传本地 id 污染在线榜单
        for (id, meta) in online_meta {
            if crate::online::is_online_id(id) {
                self.online_meta.insert(id.clone(), meta.clone());
            }
        }
        if day_sec > 0.0 {
            *self.days.entry(day.to_string()).or_insert(0.0) += day_sec;
        }
        if local_sec > 0.0 {
            *self.days_local.entry(day.to_string()).or_insert(0.0) += local_sec;
        }
        if online_sec > 0.0 {
            *self.days_online.entry(day.to_string()).or_insert(0.0) += online_sec;
        }
    }
}

fn tmp_path(path: &Path) -> std::path::PathBuf {
    let mut name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "stats.json".into());
    name.push_str(".tmp");
    path.with_file_name(name)
}

fn backup_path(path: &Path) -> std::path::PathBuf {
    let mut name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "stats.json".into());
    name.push_str(".bak");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str, sec: f64, count: u32, last: u64) -> (String, TrackStat) {
        (id.to_string(), TrackStat { sec, count, last })
    }

    #[test]
    fn merge_accumulates_seconds_counts_and_daily() {
        use super::StatsData;
        let mut s = StatsData::default();
        let mut samples = HashMap::new();
        samples.insert("t1".to_string(), 90.5);
        samples.insert("t2".to_string(), 30.0);
        s.merge(
            &samples,
            &["t1".to_string()],
            "2026-10-11",
            120.5,
            &HashMap::new(),
            1000,
        );
        s.merge(
            &samples,
            &["t1".to_string()],
            "2026-10-11",
            90.5,
            &HashMap::new(),
            2000,
        );
        // 单曲秒数跨次累加
        assert!((s.tracks["t1"].sec - 181.0).abs() < 1e-9);
        assert_eq!(s.tracks["t1"].count, 2);
        // last 取最大值
        assert_eq!(s.tracks["t1"].last, 2000);
        // 当日时长跨次累加
        assert!((s.days["2026-10-11"] - 211.0).abs() < 1e-9);
        // 非法(≤0)秒数被丢弃,不产生条目
        let mut bad = HashMap::new();
        bad.insert("t3".to_string(), 0.0);
        s.merge(&bad, &[], "2026-10-11", 0.0, &HashMap::new(), 3000);
        assert!(!s.tracks.contains_key("t3"));
    }

    #[test]
    fn stats_roundtrip_via_file_with_bak_fallback() {
        use super::StatsData;
        let dir = std::env::temp_dir().join(format!("tm-stats-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(STATS_FILE);

        let mut s = StatsData {
            version: 1,
            tracks: BTreeMap::from([sample("12345", 600.0, 3, 42)]),
            days: BTreeMap::from([("2026-10-11".to_string(), 600.0)]),
            ..Default::default()
        };
        // 保存两次:首次保存没有旧文件不会产生 .bak,第二次起才有
        s.save(&path).unwrap();
        s.tracks.get_mut("12345").unwrap().count = 4;
        s.save(&path).unwrap();

        // 正常读回(最新内容)
        let reread = StatsData::load(&path);
        assert_eq!(reread.tracks["12345"].count, 4);
        // 主文件损坏 → 回退 .bak(上一版内容)
        fs::write(&path, b"{broken").unwrap();
        let recovered = StatsData::load(&path);
        assert_eq!(recovered.tracks["12345"].count, 3);
        // 两份全坏 → 空数据(统计可再生,静默重开)
        let bak = backup_path(&path);
        fs::write(&bak, b"garbage").unwrap();
        assert_eq!(StatsData::load(&path).tracks.len(), 0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn track_id_string_keys_survive_json_roundtrip() {
        // 前端 Track.id 是字符串路径 hash,统计键必须原样往返
        use super::StatsData;
        let mut s = StatsData::default();
        let (k, v) = sample("987654321098765432", 1.0, 1, 1);
        s.tracks.insert(k, v);
        let json = serde_json::to_string(&s).unwrap();
        let back: StatsData = serde_json::from_str(&json).unwrap();
        assert!(back.tracks.contains_key("987654321098765432"));
    }

    #[test]
    fn merge_splits_local_and_online_daily_seconds() {
        use super::StatsData;
        let mut s = StatsData::default();
        let mut samples = HashMap::new();
        samples.insert("local-hash".to_string(), 30.0);
        samples.insert("net:123".to_string(), 12.0);
        samples.insert("qq:001yS0N33yPm1B".to_string(), 6.0);
        s.merge(&samples, &[], "2026-10-23", 48.0, &HashMap::new(), 1000);
        assert!((s.days["2026-10-23"] - 48.0).abs() < 1e-9);
        assert!((s.days_local["2026-10-23"] - 30.0).abs() < 1e-9);
        assert!((s.days_online["2026-10-23"] - 18.0).abs() < 1e-9);
    }

    #[test]
    fn merge_keeps_online_metadata_only_for_net_ids() {
        use super::StatsData;
        let mut s = StatsData::default();
        let mut meta = HashMap::new();
        meta.insert(
            "net:123".to_string(),
            OnlineTrackMeta {
                title: "在线曲".into(),
                artist: "歌手".into(),
                album: "专辑".into(),
            },
        );
        // 其他在线来源同样接受;本地 id 即使误传元数据也不能进在线榜单
        meta.insert("qq:001yS0N33yPm1B".to_string(), OnlineTrackMeta::default());
        meta.insert("local-hash".to_string(), OnlineTrackMeta::default());
        s.merge(&HashMap::new(), &[], "2026-10-23", 0.0, &meta, 1000);
        assert_eq!(s.online_meta["net:123"].title, "在线曲");
        assert!(s.online_meta.contains_key("qq:001yS0N33yPm1B"));
        assert!(!s.online_meta.contains_key("local-hash"));
    }

    #[test]
    fn load_migrates_v1_days_into_local_bucket() {
        use super::StatsData;
        let dir = std::env::temp_dir().join(format!("tm-stats-migrate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(STATS_FILE);
        // v1 时代还没有在线播放,历史总时长应整体归入本地桶
        let v1 = serde_json::json!({
            "version": 1,
            "days": { "2026-10-01": 321.0 },
            "tracks": { "old-hash": { "sec": 321.0, "count": 1, "last": 42 } }
        });
        fs::write(&path, serde_json::to_vec(&v1).unwrap()).unwrap();
        let s = StatsData::load(&path);
        assert_eq!(s.version, 2);
        assert!(s.days_online.is_empty());
        assert!((s.days_local["2026-10-01"] - 321.0).abs() < 1e-9);
        let _ = fs::remove_dir_all(&dir);
    }
}
