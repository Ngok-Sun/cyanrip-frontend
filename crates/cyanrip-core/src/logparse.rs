//! EAC 风格日志解析。
//!
//! cyanrip 的日志结构：头部是整碟参数，之后每轨一段，段的结尾是
//! `Track N ripped and encoded successfully!`，紧随其后的 `Summary:` 块
//! 才包含该轨的 CRC / AccurateRip 结果——所以解析时要把"当前轨"记成
//! 最近一次出现的完成行。

use crate::model::{ArState, RipReport, TrackResult};
use regex::Regex;
use std::sync::OnceLock;
use std::time::Duration;

macro_rules! regex_fn {
    ($name:ident, $pat:expr) => {
        fn $name() -> &'static Regex {
            static RE: OnceLock<Regex> = OnceLock::new();
            RE.get_or_init(|| Regex::new($pat).expect("正则应当合法"))
        }
    };
}

regex_fn!(re_offset, r"^Offset:\s+([+-]?\d+) samples");
regex_fn!(re_c2, r"^C2 errors:\s+(.+)$");
regex_fn!(re_tracks, r"^Disc tracks:\s+(\d+)");
regex_fn!(re_total, r"^Total time:\s+(\S+)");
regex_fn!(re_discid, r"^DiscID:\s+(\S+)");
regex_fn!(re_cddb, r"^CDDB ID:\s+(\S+)");
regex_fn!(re_mcn, r"^Disc MCN:\s+(\S+)");
regex_fn!(re_ar_id, r"^AccurateRip ID:\s+(\S+)");
regex_fn!(re_ar_found, r"^AccurateRip:\s+(found|not found)");
regex_fn!(re_paranoia, r"^Paranoia level:\s+(.+)$");
regex_fn!(
    re_track_done,
    r"^Track (\d+) ripped and encoded successfully!"
);
regex_fn!(re_duration, r"^\s+Duration:\s+(\d+:\d{2}\.\d{2})\s*$");
regex_fn!(re_crc, r"^\s+EAC CRC32:\s+([0-9A-Fa-f]{8})");
regex_fn!(
    re_ar_v1,
    r"^\s+Accurip v1:\s+([0-9A-Fa-f]{8})(?:\s+\((.*)\))?"
);
regex_fn!(
    re_ar_v2,
    r"^\s+Accurip v2:\s+([0-9A-Fa-f]{8})(?:\s+\((.*)\))?"
);
regex_fn!(re_accurate, r"^Tracks ripped accurately:\s+(\d+)/(\d+)");
regex_fn!(re_errors, r"^Ripping errors:\s+(\d+)");
regex_fn!(re_confidence, r"confidence:?\s*(\d+)");
regex_fn!(re_mmssff, r"^(\d+):(\d{2})\.(\d{2})$");

fn hex32(s: &str) -> Option<u32> {
    u32::from_str_radix(s, 16).ok()
}

fn ar_state_of(text: &str) -> ArState {
    let t = text.to_ascii_lowercase();
    if t.contains("accurately ripped") {
        ArState::Accurate
    } else if t.contains("not found") {
        ArState::NotFound
    } else if t.contains("mismatch") || t.contains("different") || t.contains("inaccurate") {
        ArState::Mismatch
    } else {
        ArState::Unknown
    }
}

fn confidence_of(text: &str) -> Option<u8> {
    re_confidence()
        .captures(text)?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}

/// `mm:ss.ff`（ff 是 1/75 秒的帧）→ Duration
fn parse_mmssff(s: &str) -> Option<Duration> {
    let c = re_mmssff().captures(s.trim())?;
    let m: u64 = c.get(1)?.as_str().parse().ok()?;
    let sec: u64 = c.get(2)?.as_str().parse().ok()?;
    let ff: u64 = c.get(3)?.as_str().parse().ok()?;
    Some(Duration::from_millis(
        m * 60_000 + sec * 1000 + ff * 1000 / 75,
    ))
}

/// 解析整份日志。缺字段时保持默认值，不报错——日志可能被截断（用户取消）。
pub fn parse_log(text: &str) -> RipReport {
    let mut r = RipReport::default();
    let mut current: Option<u32> = None;

    for line in text.lines() {
        let line = line.trim_end_matches('\r');

        if let Some(c) = re_offset().captures(line) {
            r.offset = c.get(1).and_then(|m| m.as_str().parse().ok());
            continue;
        }
        if let Some(c) = re_c2().captures(line) {
            let v = c
                .get(1)
                .map(|m| m.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            r.c2_supported = if v.contains("unsupported") || v.contains("not supported") {
                Some(false)
            } else if v.contains("supported") {
                Some(true)
            } else {
                None
            };
            continue;
        }
        if let Some(c) = re_tracks().captures(line) {
            r.disc_tracks = c.get(1).and_then(|m| m.as_str().parse().ok());
            continue;
        }
        if let Some(c) = re_total().captures(line) {
            r.total_time = c.get(1).map(|m| m.as_str().to_string());
            continue;
        }
        if let Some(c) = re_discid().captures(line) {
            r.disc_id = c.get(1).map(|m| m.as_str().to_string());
            continue;
        }
        if let Some(c) = re_cddb().captures(line) {
            r.cddb_id = c.get(1).map(|m| m.as_str().to_string());
            continue;
        }
        if let Some(c) = re_mcn().captures(line) {
            r.mcn = c.get(1).map(|m| m.as_str().to_string());
            continue;
        }
        if let Some(c) = re_ar_id().captures(line) {
            r.ar_id = c.get(1).map(|m| m.as_str().to_string());
            continue;
        }
        if let Some(c) = re_ar_found().captures(line) {
            r.ar_found = c.get(1).map(|m| m.as_str() == "found");
            continue;
        }
        if let Some(c) = re_paranoia().captures(line) {
            r.paranoia_level = c.get(1).map(|m| m.as_str().trim().to_string());
            continue;
        }
        if let Some(c) = re_track_done().captures(line) {
            let n: u32 = c.get(1).unwrap().as_str().parse().unwrap_or(0);
            current = Some(n);
            if !r.tracks.iter().any(|t| t.number == n) {
                r.tracks.push(TrackResult::new(n));
                r.tracks_ripped += 1;
            }
            continue;
        }
        if let Some(c) = re_accurate().captures(line) {
            r.tracks_accurate = c.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
            continue;
        }
        if let Some(c) = re_errors().captures(line) {
            r.errors = c.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
            continue;
        }

        let Some(n) = current else { continue };
        let Some(t) = r.tracks.iter_mut().find(|t| t.number == n) else {
            continue;
        };

        if let Some(c) = re_duration().captures(line) {
            t.duration = c.get(1).and_then(|m| parse_mmssff(m.as_str()));
            continue;
        }
        if let Some(c) = re_crc().captures(line) {
            t.crc32 = c.get(1).and_then(|m| hex32(m.as_str()));
            continue;
        }
        if let Some(c) = re_ar_v1().captures(line) {
            t.ar_v1 = c.get(1).and_then(|m| hex32(m.as_str()));
            if let Some(st) = c.get(2) {
                if t.ar_state == ArState::Unknown {
                    t.ar_state = ar_state_of(st.as_str());
                }
                if let Some(cf) = confidence_of(st.as_str()) {
                    t.ar_confidence = Some(cf);
                }
            }
            continue;
        }
        if let Some(c) = re_ar_v2().captures(line) {
            t.ar_v2 = c.get(1).and_then(|m| hex32(m.as_str()));
            if let Some(st) = c.get(2) {
                let state = ar_state_of(st.as_str());
                // v2 未入库不该抹掉 v1 的精确命中（老压片常见）
                if state == ArState::Accurate || t.ar_state == ArState::Unknown {
                    t.ar_state = state;
                }
                if let Some(cf) = confidence_of(st.as_str()) {
                    t.ar_confidence = Some(cf);
                }
            }
            continue;
        }
    }

    r.tracks.sort_by_key(|t| t.number);
    if r.tracks_ripped == 0 {
        r.tracks_ripped = r.tracks.len() as u32;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_not_found_keeps_v1_accurate() {
        let log = [
            "Disc tracks:    1",
            "Track 1 ripped and encoded successfully!",
            "  EAC CRC32:     971CD8BF",
            "  Accurip v1:    971CD8BF (accurately ripped, confidence 5)",
            "  Accurip v2:    056580F7 (not found, either a new pressing, or bad rip)",
        ]
        .join("\n");
        let r = parse_log(&log);
        let t = r.track(1).expect("track 1");
        assert_eq!(t.ar_state, ArState::Accurate, "v1 命中应保留");
        assert_eq!(t.ar_confidence, Some(5));
    }

    const QUARTET: &str = include_str!("../tests/fixtures/log_quartet.txt");
    const FLOWER_CD1: &str = include_str!("../tests/fixtures/log_flower_cd1.txt");
    const FLOWER_CD2: &str = include_str!("../tests/fixtures/log_flower_cd2.txt");

    #[test]
    fn parses_quartet_header_and_tracks() {
        let r = parse_log(QUARTET);
        assert_eq!(r.offset, Some(48));
        assert_eq!(r.c2_supported, Some(false));
        assert_eq!(r.disc_tracks, Some(13));
        assert_eq!(r.tracks_ripped, 13);
        assert_eq!(r.tracks_accurate, 13);
        assert_eq!(r.errors, 0);
        assert_eq!(r.total_time.as_deref(), Some("41:22.28"));
        assert_eq!(r.disc_id.as_deref(), Some("kC9g0ZX9IMTELLEHE.ETxsZyaMg-"));
        assert_eq!(r.cddb_id.as_deref(), Some("9909B20D"));
        assert_eq!(r.mcn.as_deref(), Some("4988005280619"));
        assert_eq!(r.ar_id.as_deref(), Some("001333f2-00bfe5ad-9909b20d"));
        assert_eq!(r.ar_found, Some(true));
        assert_eq!(r.paranoia_level.as_deref(), Some("max"));
        assert!(r.all_accurate());

        let t1 = r.track(1).expect("track 1");
        assert_eq!(t1.crc32, Some(0x0183_4C83));
        assert_eq!(t1.ar_v1, Some(0x6F09_6812));
        assert_eq!(t1.ar_v2, Some(0x17AF_569B));
        assert_eq!(t1.ar_confidence, Some(8));
        assert_eq!(t1.ar_state, ArState::Accurate);
        // 03:18.10 → 198s + 10/75
        let d = t1.duration.unwrap();
        assert!((d.as_secs_f64() - 198.133).abs() < 0.01, "got {:?}", d);
    }

    #[test]
    fn flower_cd1_has_one_not_found() {
        let r = parse_log(FLOWER_CD1);
        assert_eq!(r.tracks_ripped, 16);
        assert_eq!(r.tracks_accurate, 15);
        assert_eq!(r.errors, 0);
        assert_eq!(r.offset, Some(48));
        assert_eq!(r.track(16).unwrap().ar_state, ArState::NotFound);
        assert_eq!(r.track(15).unwrap().ar_state, ArState::Accurate);
    }

    #[test]
    fn flower_cd2_all_accurate() {
        let r = parse_log(FLOWER_CD2);
        assert_eq!(r.tracks_ripped, 8);
        assert_eq!(r.tracks_accurate, 8);
        assert_eq!(r.errors, 0);
        assert!(r.all_accurate());
    }

    #[test]
    fn truncated_log_does_not_panic() {
        let cut: String = QUARTET.lines().take(60).collect::<Vec<_>>().join("\n");
        let r = parse_log(&cut);
        assert_eq!(r.offset, Some(48));
        assert!(r.tracks_ripped >= 1);
    }

    #[test]
    fn empty_log_is_default() {
        let r = parse_log("");
        assert_eq!(r.tracks_ripped, 0);
        assert!(r.tracks.is_empty());
        assert!(!r.all_accurate());
    }
}
