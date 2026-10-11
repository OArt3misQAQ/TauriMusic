//! 听歌统计:仅本机落盘(app_data/stats.json),全链路零联网。
//! 采集方在前端(只有 WebView 知道音频是否真的在播),后端只负责合并与
//! 原子持久化(与 library.json 同一套 tmp→bak→rename + 损坏回退)。
//! 歌手/专辑聚合不落盘,由前端按曲库即时推导——标签修改后统计自动跟随。

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

/// 统计数据:每日总时长 + 单曲明细
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StatsData {
    /// 结构版本(当前 1,未来字段演进用)
    pub version: u32,
    /// 本地日期(YYYY-MM-DD)→ 当日收听秒数
    pub days: BTreeMap<String, f64>,
    /// Track.id(路径 hash 字符串)→ 单曲统计
    pub tracks: BTreeMap<String, TrackStat>,
}

/// 当前 unix 秒;时钟早于 1970 等异常情形按 0 处理
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl StatsData {
    /// 读取统计:主文件损坏(截断/非法 JSON)时回退 .bak,两份都坏则从零开始。
    /// 统计是可再生的观测数据,损坏不值得让用户感知,静默重开即可
    pub fn load(path: &Path) -> StatsData {
        if let Some(s) = Self::try_read(path) {
            return s;
        }
        let bak = backup_path(path);
        if let Some(s) = Self::try_read(&bak) {
            eprintln!("[stats] 主文件损坏,已从备份恢复: {}", bak.display());
            return s;
        }
        StatsData::default()
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
        now: u64,
    ) {
        for (id, sec) in samples {
            if *sec <= 0.0 {
                continue;
            }
            let s = self.tracks.entry(id.clone()).or_default();
            s.sec += sec;
            s.last = s.last.max(now);
        }
        for id in plays {
            let s = self.tracks.entry(id.clone()).or_default();
            s.count = s.count.saturating_add(1);
            s.last = s.last.max(now);
        }
        if day_sec > 0.0 {
            *self.days.entry(day.to_string()).or_insert(0.0) += day_sec;
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
        s.merge(&samples, &["t1".to_string()], "2026-10-11", 120.5, 1000);
        s.merge(&samples, &["t1".to_string()], "2026-10-11", 90.5, 2000);
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
        s.merge(&bad, &[], "2026-10-11", 0.0, 3000);
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
}
