//! MusicBrainz Disc ID 计算。
//!
//! 算法（libdiscid 兼容）：
//! ```text
//! string = fmt("%02X", 首轨) + fmt("%02X", 末轨) + fmt("%08X", leadout)
//!          + 99 × fmt("%08X", 各轨起始偏移)
//! id = base64(sha1(string))，'+'→'.'  '/'→'_'  '='→'-'
//! ```
//! 偏移与 leadout 都以扇区为单位、已加 150（CD 的 2 秒前导）。

use crate::model::DiscInfo;
use sha1::{Digest, Sha1};

/// 计算 MusicBrainz Disc ID。
pub fn disc_id(info: &DiscInfo) -> Option<String> {
    let audio: Vec<&crate::model::TrackInfo> = info.audio_tracks().collect();
    if audio.is_empty() {
        return None;
    }
    let first = audio.first()?.number as u8;
    let last = audio.last()?.number as u8;
    let leadout = info.leadout_lsn + 150;

    let mut s = String::with_capacity(2 + 2 + 8 + 99 * 8);
    s.push_str(&format!("{first:02X}"));
    s.push_str(&format!("{last:02X}"));
    s.push_str(&format!("{leadout:08X}"));
    for i in 0..99 {
        let off = audio.get(i).map(|t| t.start_lsn + 150).unwrap_or(0);
        s.push_str(&format!("{off:08X}"));
    }

    let digest = Sha1::digest(s.as_bytes());
    Some(base64_mb(&digest))
}

/// base64（标准表）+ MusicBrainz 的字符替换。
fn base64_mb(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 0x3F] as char);
        out.push(TABLE[(n >> 12) as usize & 0x3F] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(n >> 6) as usize & 0x3F] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[n as usize & 0x3F] as char);
        } else {
            out.push('=');
        }
    }
    out.replace('+', ".").replace('/', "_").replace('=', "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TrackInfo;

    fn quartet() -> DiscInfo {
        // 实测 TOC：13 轨，leadout 186178，起始偏移见下
        let starts = [
            0, 14860, 30568, 40210, 61528, 67808, 81048, 99150, 104438, 120783, 129910, 150403,
            171598,
        ];
        let mut tracks = Vec::new();
        for (i, s) in starts.iter().enumerate() {
            let next = starts.get(i + 1).copied().unwrap_or(186178);
            tracks.push(TrackInfo {
                number: i as u32 + 1,
                start_lsn: *s,
                duration_frames: next - s,
                is_audio: true,
                title: None,
                isrc: None,
            });
        }
        DiscInfo {
            device: "/dev/sr0".into(),
            vendor: "PIONEER".into(),
            model: "DVD-RW DVR-106D".into(),
            firmware: Some("1.00".into()),
            tracks,
            leadout_lsn: 186178,
            total_frames: 186178,
            disc_id: None,
            cddb_id: None,
            mcn: None,
            ar_in_db: None,
        }
    }

    #[test]
    fn matches_cyanrip_discid_for_quartet() {
        // cyanrip 日志里的真实 DiscID
        assert_eq!(
            disc_id(&quartet()).as_deref(),
            Some("kC9g0ZX9IMTELLEHE.ETxsZyaMg-")
        );
    }

    #[test]
    fn base64_uses_musicbrainz_alphabet() {
        // 0xFF 0xFF 0xFF → "////" → "__" 后接填充
        let s = base64_mb(&[0xFF, 0xFF, 0xFF]);
        assert!(!s.contains('/') && !s.contains('+') && !s.contains('='));
        assert!(s.starts_with("__"));
    }
}
