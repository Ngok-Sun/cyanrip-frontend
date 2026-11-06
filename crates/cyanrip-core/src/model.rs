//! 数据模型：碟片、轨道、进度、抓轨报告。
//!
//! 这些类型同时被 core、CLI 与 GUI 使用，因此保持纯数据、可序列化。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// 一条音轨（或数据轨）的基本信息，来自 TOC。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrackInfo {
    pub number: u32,
    /// 起始 LSN（0 基，单位扇区）
    pub start_lsn: u32,
    /// 长度（扇区）
    pub duration_frames: u32,
    pub is_audio: bool,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub isrc: Option<String>,
}

impl TrackInfo {
    pub fn duration_secs(&self) -> f64 {
        self.duration_frames as f64 / 75.0
    }

    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.duration_secs())
    }
}

/// 一张已探测到的碟片。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscInfo {
    pub device: PathBuf,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub firmware: Option<String>,
    pub tracks: Vec<TrackInfo>,
    /// lead-out 的 LSN
    pub leadout_lsn: u32,
    /// 全碟扇区数
    pub total_frames: u32,
    #[serde(default)]
    pub disc_id: Option<String>,
    #[serde(default)]
    pub cddb_id: Option<String>,
    #[serde(default)]
    pub mcn: Option<String>,
    #[serde(default)]
    pub ar_in_db: Option<bool>,
}

impl DiscInfo {
    pub fn audio_tracks(&self) -> impl Iterator<Item = &TrackInfo> {
        self.tracks.iter().filter(|t| t.is_audio)
    }

    pub fn audio_track_count(&self) -> usize {
        self.audio_tracks().count()
    }

    pub fn total_duration(&self) -> Duration {
        Duration::from_secs_f64(self.total_frames as f64 / 75.0)
    }

    pub fn total_duration_str(&self) -> String {
        fmt_duration(self.total_duration())
    }

    /// 给 UI 用的简短描述，例如 "PIONEER DVD-RW DVR-106D (1.00)"
    pub fn drive_label(&self) -> String {
        match &self.firmware {
            Some(fw) if !fw.is_empty() => format!("{} {} ({})", self.vendor, self.model, fw),
            _ => format!("{} {}", self.vendor, self.model),
        }
        .trim()
        .to_string()
    }
}

/// `mm:ss` 或 `h:mm:ss`，用于界面显示。
pub fn fmt_duration(d: Duration) -> String {
    let total = d.as_secs();
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// AccurateRip 的判定结果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArState {
    /// 精确命中数据库
    Accurate,
    /// 数据库中没有这一轨（常见于少见压片/新增曲目）
    NotFound,
    /// 数据库中有，但校验和不一致（真正可疑）
    Mismatch,
    /// 日志里没提到
    #[default]
    Unknown,
}

/// 单轨抓轨结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackResult {
    pub number: u32,
    #[serde(default)]
    pub crc32: Option<u32>,
    #[serde(default)]
    pub ar_v1: Option<u32>,
    #[serde(default)]
    pub ar_v2: Option<u32>,
    #[serde(default)]
    pub ar_confidence: Option<u8>,
    #[serde(default)]
    pub ar_state: ArState,
    #[serde(default)]
    pub duration: Option<Duration>,
}

impl TrackResult {
    pub fn new(number: u32) -> Self {
        Self {
            number,
            crc32: None,
            ar_v1: None,
            ar_v2: None,
            ar_confidence: None,
            ar_state: ArState::Unknown,
            duration: None,
        }
    }
}

/// 整张碟的抓轨报告，主要来自 EAC 风格日志。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RipReport {
    pub offset: Option<i32>,
    pub c2_supported: Option<bool>,
    pub paranoia_level: Option<String>,
    pub disc_tracks: Option<u32>,
    pub tracks_ripped: u32,
    pub tracks_accurate: u32,
    pub errors: u32,
    pub total_time: Option<String>,
    pub ar_id: Option<String>,
    pub ar_found: Option<bool>,
    pub disc_id: Option<String>,
    pub cddb_id: Option<String>,
    pub mcn: Option<String>,
    pub tracks: Vec<TrackResult>,
    pub log_path: Option<PathBuf>,
}

impl RipReport {
    pub fn track(&self, number: u32) -> Option<&TrackResult> {
        self.tracks.iter().find(|t| t.number == number)
    }

    /// 是否全部精确命中（且没有错误）。
    pub fn all_accurate(&self) -> bool {
        self.errors == 0 && self.tracks_ripped > 0 && self.tracks_accurate == self.tracks_ripped
    }
}

/// 进度事件的数据部分。
#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub track: u32,
    /// 本轨百分比 0.0..=100.0
    pub track_pct: f32,
    /// 全碟百分比 0.0..=100.0
    pub overall_pct: f32,
    pub eta: Option<Duration>,
    /// 读取倍速（1x = 75 扇区/秒）
    pub speed_x: Option<f32>,
    pub elapsed: Duration,
}
