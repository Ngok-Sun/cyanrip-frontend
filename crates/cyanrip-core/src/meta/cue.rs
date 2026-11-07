//! 解析 CUE 表（用于 CD 数据库里查不到的碟）。
//!
//! 只取元数据部分：PERFORMER / TITLE / REM DATE / REM DISCNUMBER /
//! REM TOTALDISCS / CATALOG / 每条 TRACK 的 TITLE。音频文件、INDEX、PREGAP 全部忽略。
//!
//! 编码：优先 UTF-8（去 BOM），失败则按 GBK 解码——中文碟的 CUE 大量是 GBK。

use super::{MetaRelease, MetaSource, MetaTrack};
use anyhow::{Context, Result};
use std::path::Path;

#[derive(Default)]
struct TrackBuilder {
    number: u32,
    title: Option<String>,
}

/// 读文件并按 UTF-8 → GBK 的顺序解码
fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    let (cow, _, _) = encoding_rs::GBK.decode(bytes);
    cow.into_owned()
}

fn unquote(raw: &str) -> String {
    let s = raw.trim();
    let s = s
        .strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .unwrap_or(s);
    s.replace("\\\"", "\"")
}

/// 解析 CUE 文本。`id` 用作候选标识（一般传文件路径）。
pub fn parse_str(text: &str, id: &str) -> MetaRelease {
    let mut artist = String::new();
    let mut album = String::new();
    let mut date: Option<String> = None;
    let mut catno: Option<String> = None;
    let mut barcode: Option<String> = None;
    let mut disc_number = 1u32;
    let mut disc_total = 1u32;
    let mut tracks: Vec<MetaTrack> = Vec::new();
    let mut current: Option<TrackBuilder> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let upper = line.to_ascii_uppercase();
        let (key, rest) = match line.split_once(char::is_whitespace) {
            Some((k, r)) => (k.to_ascii_uppercase(), r.trim()),
            None => (upper.clone(), ""),
        };

        match key.as_str() {
            "TRACK" => {
                if let Some(t) = current.take() {
                    push_track(&mut tracks, t);
                }
                let number = rest
                    .split_whitespace()
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(tracks.len() as u32 + 1);
                current = Some(TrackBuilder {
                    number,
                    title: None,
                });
            }
            "TITLE" => {
                let v = unquote(rest);
                match current.as_mut() {
                    Some(t) => t.title = Some(v),
                    None => album = v,
                }
            }
            "PERFORMER" => {
                let v = unquote(rest);
                // 轨级 PERFORMER 通常与碟级相同，没必要逐轨保存
                if current.is_none() && artist.is_empty() {
                    artist = v;
                }
            }
            "CATALOG" => {
                let v = unquote(rest);
                if !v.is_empty() {
                    catno = Some(v);
                }
            }
            "REM" => {
                // REM DATE 2001 / REM DISCNUMBER 2 / REM TOTALDISCS 2 / REM BARCODE ...
                let mut parts = rest.splitn(2, char::is_whitespace);
                let sub = parts.next().unwrap_or("").to_ascii_uppercase();
                let value = unquote(parts.next().unwrap_or(""));
                match sub.as_str() {
                    "DATE" => date = Some(value),
                    "BARCODE" => barcode = Some(value.replace(['-', ' '], "")),
                    "DISCNUMBER" => disc_number = value.parse().unwrap_or(1),
                    "TOTALDISCS" | "TOTALDISCS " => disc_total = value.parse().unwrap_or(1),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    if let Some(t) = current.take() {
        push_track(&mut tracks, t);
    }

    MetaRelease {
        source: MetaSource::Cue,
        id: id.to_string(),
        title: if album.is_empty() {
            "未命名专辑".into()
        } else {
            album
        },
        artist,
        date,
        country: None,
        label: None,
        catno,
        barcode,
        tracks,
        disc_number,
        disc_total: disc_total.max(disc_number).max(1),
        cover_url: None,
        score: 100,
    }
}

fn push_track(tracks: &mut Vec<MetaTrack>, t: TrackBuilder) {
    // TITLE "" 这种空值也要回退，否则界面/文件名会是一段空白
    let title = t
        .title
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("Track {:02}", t.number));
    tracks.push(MetaTrack {
        number: t.number,
        title,
        length: None,
    });
}

/// 读取并解析 CUE 文件
pub fn parse_file(path: &Path) -> Result<MetaRelease> {
    let bytes =
        std::fs::read(path).with_context(|| format!("读取 CUE 失败：{}", path.display()))?;
    let text = decode(&bytes);
    let rel = parse_str(&text, &path.display().to_string());
    if rel.tracks.is_empty() {
        anyhow::bail!("CUE 里没有 TRACK 条目：{}", path.display());
    }
    Ok(rel)
}

/// 生成 CUE 文本（单镜像风格：一条 FILE，INDEX 从第一轨起算）。
pub fn render_cue(info: &crate::model::DiscInfo, release: &MetaRelease) -> String {
    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }
    let mut out = String::new();
    if !release.artist.is_empty() {
        out.push_str(&format!("PERFORMER \"{}\"\n", esc(&release.artist)));
    }
    out.push_str(&format!("TITLE \"{}\"\n", esc(&release.title)));
    if let Some(d) = release.date.as_deref() {
        out.push_str(&format!("REM DATE {d}\n"));
    }
    if release.disc_total > 1 {
        out.push_str(&format!("REM DISCNUMBER {}\n", release.disc_number.max(1)));
        out.push_str(&format!("REM TOTALDISCS {}\n", release.disc_total));
    }
    if let Some(c) = release.catno.as_deref() {
        out.push_str(&format!("CATALOG {c}\n"));
    }
    if let Some(b) = release.barcode.as_deref() {
        out.push_str(&format!("REM BARCODE {b}\n"));
    }
    match release.source {
        MetaSource::MusicBrainz => {
            out.push_str(&format!("REM MUSICBRAINZ_ID \"{}\"\n", release.id))
        }
        MetaSource::Discogs => out.push_str(&format!("REM DISCOGS_RELEASE {}\n", release.id)),
        _ => {}
    }
    out.push_str(&format!(
        "REM COMMENT \"cyanrip-frontend export, {} track(s)\"\n",
        release.tracks.len()
    ));
    out.push_str(&format!("FILE \"{}.flac\" WAVE\n", esc(&release.title)));

    let audio: Vec<&crate::model::TrackInfo> = info.tracks.iter().filter(|t| t.is_audio).collect();
    let first_lsn = audio.first().map(|t| t.start_lsn).unwrap_or(0);
    for (i, t) in audio.iter().enumerate() {
        let title = release
            .tracks
            .get(i)
            .map(|m| m.title.clone())
            .or_else(|| t.title.clone())
            .unwrap_or_else(|| format!("Track {:02}", t.number));
        let rel = t.start_lsn.saturating_sub(first_lsn);
        let frames = rel % 75;
        let secs = (rel / 75) % 60;
        let mins = rel / 75 / 60;
        out.push_str(&format!("  TRACK {:02} AUDIO\n", t.number));
        out.push_str(&format!("    TITLE \"{}\"\n", esc(&title)));
        if !release.artist.is_empty() {
            out.push_str(&format!("    PERFORMER \"{}\"\n", esc(&release.artist)));
        }
        out.push_str(&format!("    INDEX 01 {mins:02}:{secs:02}:{frames:02}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cyanrip_style_cue() {
        let text = r#"PERFORMER "花儿乐队"
TITLE "幸福的旁边"
REM DATE 1999-01-18
REM DISCNUMBER 2
REM TOTALDISCS 2
REM DISCOGS_RELEASE 14540494
FILE "CD2-01 稻草上的火鸡.flac" WAVE
  TRACK 01 AUDIO
    TITLE "稻草上的火鸡"
    PERFORMER "花儿乐队"
    INDEX 01 00:00:00
FILE "CD2-02 静止.flac" WAVE
  TRACK 02 AUDIO
    TITLE "静止"
"#;
        let r = parse_str(text, "test.cue");
        assert_eq!(r.source, MetaSource::Cue);
        assert_eq!(r.artist, "花儿乐队");
        assert_eq!(r.title, "幸福的旁边");
        assert_eq!(r.date.as_deref(), Some("1999-01-18"));
        assert_eq!(r.disc_number, 2);
        assert_eq!(r.disc_total, 2);
        assert_eq!(r.tracks.len(), 2);
        assert_eq!(r.tracks[0].number, 1);
        assert_eq!(r.tracks[0].title, "稻草上的火鸡");
        assert_eq!(r.tracks[1].title, "静止");
    }

    #[test]
    fn renders_cue_with_index_times() {
        use crate::model::{DiscInfo, TrackInfo};
        let info = DiscInfo {
            device: "x".into(),
            vendor: String::new(),
            model: String::new(),
            firmware: None,
            tracks: vec![
                TrackInfo {
                    number: 1,
                    start_lsn: 0,
                    duration_frames: 7500,
                    is_audio: true,
                    title: None,
                    isrc: None,
                },
                TrackInfo {
                    number: 2,
                    start_lsn: 7500 + 150,
                    duration_frames: 7500,
                    is_audio: true,
                    title: None,
                    isrc: None,
                },
            ],
            leadout_lsn: 15150,
            total_frames: 15150,
            disc_id: None,
            cddb_id: None,
            mcn: None,
            ar_in_db: None,
        };
        let rel = parse_str("PERFORMER \"A\"\nTITLE \"B\"\nTRACK 01 AUDIO\nTITLE \"one\"\nTRACK 02 AUDIO\nTITLE \"two\"\n", "x");
        let text = render_cue(&info, &rel);
        assert!(text.contains("PERFORMER \"A\""));
        assert!(text.contains("TRACK 01 AUDIO"));
        assert!(text.contains("INDEX 01 00:00:00"));
        // 第 2 轨从 7650 扇区开始 = 102 秒
        assert!(text.contains("INDEX 01 01:42:00"));
        // 导出后能被自己的解析器读回
        let back = parse_str(&text, "y");
        assert_eq!(back.tracks.len(), 2);
    }

    #[test]
    fn single_disc_defaults() {
        let text = "PERFORMER \"A\"\nTITLE \"B\"\nTRACK 01 AUDIO\nTITLE \"x\"\n";
        let r = parse_str(text, "x.cue");
        assert_eq!(r.disc_number, 1);
        assert_eq!(r.disc_total, 1);
        assert_eq!(r.tracks[0].title, "x");
    }

    #[test]
    fn decodes_gbk() {
        // "歌手" 的 GBK 字节
        let gbk = [0xB8, 0xE8, 0xCA, 0xD6];
        assert_eq!(decode(&gbk), "歌手");
    }
}
