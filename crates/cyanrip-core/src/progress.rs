//! cyanrip 进度行解析与全碟进度折算。
//!
//! cyanrip 的进度是**单轨**百分比，用 `\r` 原地刷新，格式见上游
//! `crip_progress_report()`：
//!
//! ```text
//! Ripping track 5, progress - 47.32%, ETA - 2m
//! Ripping and encoding track 9, progress - 9.58%, ETA - 1h 3m
//! Ripping data track 2, progress - 12.00%, ETA - 45s, errors - 2
//! ```
//!
//! ETA 只有 `h m` / `m` / `s` 三种形态，不带秒的分钟值意味着秒被舍去。

use crate::model::{DiscInfo, ProgressUpdate, TrackInfo};
use regex::Regex;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

fn re_progress() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^Ripping(?: and encoding)?(?: data)? track (\d+), progress - ([0-9.]+)%, ETA - (.+?)(?:, errors - (\d+))?\s*$",
        )
        .expect("进度正则应当合法")
    })
}

/// 一行进度输出。
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressLine {
    pub track: u32,
    pub percent: f32,
    pub eta: Option<Duration>,
    pub errors: u32,
}

/// 解析单行；不是进度行时返回 `None`。
pub fn parse_progress(line: &str) -> Option<ProgressLine> {
    let caps = re_progress().captures(line.trim())?;
    let track = caps.get(1)?.as_str().parse().ok()?;
    let percent: f32 = caps.get(2)?.as_str().parse().ok()?;
    let eta = parse_eta(caps.get(3)?.as_str());
    let errors = caps
        .get(4)
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0);
    Some(ProgressLine {
        track,
        percent: percent.clamp(0.0, 100.0),
        eta,
        errors,
    })
}

/// 解析 `2m` / `1h 3m` / `45s`。
fn parse_eta(s: &str) -> Option<Duration> {
    let s = s.trim();
    let mut secs: u64 = 0;
    let mut matched = false;
    for part in s.split_whitespace() {
        if let Some(h) = part.strip_suffix('h') {
            secs += h.parse::<u64>().ok()? * 3600;
            matched = true;
        } else if let Some(m) = part.strip_suffix('m') {
            secs += m.parse::<u64>().ok()? * 60;
            matched = true;
        } else {
            let sec = part.strip_suffix('s')?;
            secs += sec.parse::<u64>().ok()?;
            matched = true;
        }
    }
    matched.then(|| Duration::from_secs(secs))
}

/// 把单轨进度折算成全碟进度，并估算实际读取倍速。
pub struct ProgressTracker {
    /// 参与抓取的音轨（按 TOC 顺序）
    tracks: Vec<TrackInfo>,
    total_frames: u64,
    started: Instant,
}

impl ProgressTracker {
    pub fn new(disc: &DiscInfo) -> Self {
        let tracks: Vec<TrackInfo> = disc.audio_tracks().cloned().collect();
        let total_frames = tracks.iter().map(|t| t.duration_frames as u64).sum();
        Self {
            tracks,
            total_frames,
            started: Instant::now(),
        }
    }

    /// 音轨号 → 在 `tracks` 中的下标
    fn index_of(&self, track_number: u32) -> Option<usize> {
        self.tracks.iter().position(|t| t.number == track_number)
    }

    /// 喂一行进度，得到 UI 可直接显示的数据。
    pub fn update(&mut self, line: &ProgressLine) -> ProgressUpdate {
        let elapsed = self.started.elapsed();
        let idx = self.index_of(line.track).unwrap_or(0);
        let before: u64 = self.tracks[..idx]
            .iter()
            .map(|t| t.duration_frames as u64)
            .sum();
        let cur = self
            .tracks
            .get(idx)
            .map(|t| t.duration_frames as u64)
            .unwrap_or(0);

        let frames_done = before + (cur as f64 * line.percent as f64 / 100.0) as u64;
        let overall = if self.total_frames > 0 {
            (frames_done as f64 / self.total_frames as f64 * 100.0) as f32
        } else {
            0.0
        };

        let secs = elapsed.as_secs_f64();
        // 1x = 75 扇区/秒；用累计值计算，比瞬时值平滑
        let speed_x = if secs > 0.5 && frames_done > 0 {
            Some((frames_done as f64 / secs / 75.0) as f32)
        } else {
            None
        };

        ProgressUpdate {
            track: line.track,
            track_pct: line.percent,
            overall_pct: overall,
            eta: line.eta,
            speed_x,
            elapsed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_line() {
        let p = parse_progress("Ripping track 5, progress - 47.32%, ETA - 2m").unwrap();
        assert_eq!(p.track, 5);
        assert!((p.percent - 47.32).abs() < 0.01);
        assert_eq!(p.eta, Some(Duration::from_secs(120)));
        assert_eq!(p.errors, 0);
    }

    #[test]
    fn parses_encoding_and_hours() {
        let p =
            parse_progress("Ripping and encoding track 9, progress - 9.58%, ETA - 1h 3m").unwrap();
        assert_eq!(p.track, 9);
        assert_eq!(p.eta, Some(Duration::from_secs(3780)));
    }

    #[test]
    fn parses_seconds_and_errors() {
        let p = parse_progress("Ripping data track 2, progress - 12.00%, ETA - 45s, errors - 2")
            .unwrap();
        assert_eq!(p.track, 2);
        assert_eq!(p.eta, Some(Duration::from_secs(45)));
        assert_eq!(p.errors, 2);
    }

    #[test]
    fn trims_trailing_spaces() {
        let p = parse_progress("Ripping track 1, progress - 100.00%, ETA - 0s     ").unwrap();
        assert_eq!(p.eta, Some(Duration::from_secs(0)));
    }

    #[test]
    fn ignores_other_lines() {
        assert!(parse_progress("Track 1 ripped and encoded successfully!").is_none());
        assert!(parse_progress("Ripping track 1, progress - 0.00%").is_none());
    }

    fn disc_with(tracks: &[(u32, u32)]) -> DiscInfo {
        DiscInfo {
            device: "/dev/sr0".into(),
            vendor: "PIONEER".into(),
            model: "DVD-RW DVR-106D".into(),
            firmware: Some("1.00".into()),
            tracks: tracks
                .iter()
                .map(|(num, frames)| TrackInfo {
                    number: *num,
                    start_lsn: 0,
                    duration_frames: *frames,
                    is_audio: true,
                    title: None,
                    isrc: None,
                })
                .collect(),
            leadout_lsn: 0,
            total_frames: tracks.iter().map(|(_, f)| *f).sum(),
            disc_id: None,
            cddb_id: None,
            mcn: None,
            ar_in_db: None,
        }
    }

    #[test]
    fn converts_track_percent_to_disc_percent() {
        let disc = disc_with(&[(1, 1000), (2, 3000)]);
        let mut t = ProgressTracker::new(&disc);
        let u = t.update(&ProgressLine {
            track: 2,
            percent: 50.0,
            eta: Some(Duration::from_secs(60)),
            errors: 0,
        });
        // 已完成 1000 + 本轨 1500 = 2500 / 4000 = 62.5%
        assert!((u.overall_pct - 62.5).abs() < 0.01, "got {}", u.overall_pct);
    }
}
