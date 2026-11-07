//! 把元数据写进 FLAC：重命名、归档到 `艺人/专辑`、写 Vorbis comment、内嵌封面。
//!
//! 用 `metaflac` 而不是自己改 FLAC 块：它已经被验证过不会动音频流，
//! 而且 `--remove-all-tags` + 重新写入的顺序能保住 ReplayGain / AcoustID 指纹。

use crate::meta::MetaRelease;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// 文件名清洗：去掉路径分隔符与 Windows 非法字符。
pub fn sanitize(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').trim();
    if trimmed.is_empty() {
        "未命名".into()
    } else {
        trimmed.to_string()
    }
}

fn keep_tag(line: &str) -> bool {
    let key = line.split('=').next().unwrap_or("").to_ascii_lowercase();
    key.starts_with("replaygain")
        || key.starts_with("r128")
        || key.starts_with("acoustid")
        || key.starts_with("musicbrainz_discid")
        || key == "cddb"
        || key == "media"
        || key == "encoder"
        || key == "creation_time"
}

/// 对一批文件应用元数据，返回归档目录。
///
/// `files` 必须按音轨顺序排列；`disc` 为 (本碟号, 总碟数)。
pub fn apply(
    files: &[PathBuf],
    release: &MetaRelease,
    root: &Path,
    disc: (u32, u32),
    cover: Option<&Path>,
) -> Result<PathBuf> {
    if files.is_empty() {
        bail!("没有可写标签的文件");
    }
    // 套装：以发行版解析出的碟号为准（调用方传的 (1,1) 只是单碟默认值）
    let disc = if release.disc_total > 1 {
        (release.disc_number.max(1), release.disc_total)
    } else {
        disc
    };
    let artist = sanitize(&release.artist);
    let album = sanitize(&release.title);
    let dest_dir = if artist.is_empty() || artist == "未命名" {
        root.join(&album)
    } else {
        root.join(&artist).join(&album)
    };
    std::fs::create_dir_all(&dest_dir)
        .with_context(|| format!("创建目录失败：{}", dest_dir.display()))?;

    for (i, src) in files.iter().enumerate() {
        let meta = release.tracks.get(i);
        let title = meta
            .map(|t| sanitize(&t.title))
            .unwrap_or_else(|| format!("Track {:02}", i + 1));
        let number = meta.map(|t| t.number).unwrap_or(i as u32 + 1);
        let name = if disc.1 > 1 {
            format!("CD{}-{number:02} {title}.flac", disc.0)
        } else {
            format!("{number:02} {title}.flac")
        };
        let dest = dest_dir.join(name);

        if src != &dest {
            if dest.exists() {
                bail!("目标文件已存在：{}", dest.display());
            }
            // 抓轨刚结束时文件可能还在写：不是 FLAC 就拒绝搬动
            if !is_complete_flac(src) {
                bail!("文件不完整（可能还在写入）：{}", src.display());
            }
            if std::fs::rename(src, &dest).is_err() {
                std::fs::copy(src, &dest).context("复制文件失败")?;
                std::fs::remove_file(src).ok();
            }
        }

        let track_total = if release.tracks.is_empty() {
            files.len() as u32
        } else {
            release.tracks.len() as u32
        };
        write_tags(&dest, release, title.as_str(), number, disc, track_total)?;
        if let Some(cover) = cover {
            let _ = Command::new("metaflac")
                .arg("--import-picture-from")
                .arg(cover)
                .arg(&dest)
                .status();
        }
    }
    Ok(dest_dir)
}

fn write_tags(
    file: &Path,
    release: &MetaRelease,
    title: &str,
    number: u32,
    disc: (u32, u32),
    track_total: u32,
) -> Result<()> {
    let existing = Command::new("metaflac")
        .arg("--export-tags-to=-")
        .arg(file)
        .output()
        .context("读取现有标签失败")?;
    let kept: Vec<String> = String::from_utf8_lossy(&existing.stdout)
        .lines()
        .filter(|l| keep_tag(l))
        .map(|s| s.to_string())
        .collect();

    let mut cmd = Command::new("metaflac");
    cmd.arg("--remove-all-tags");
    let mut tags: Vec<(String, String)> = vec![
        ("TITLE".into(), title.to_string()),
        ("ARTIST".into(), release.artist.clone()),
        ("ALBUM".into(), release.title.clone()),
        ("ALBUMARTIST".into(), release.artist.clone()),
        ("TRACKNUMBER".into(), number.to_string()),
        ("TRACKTOTAL".into(), track_total.to_string()),
        ("DISCNUMBER".into(), disc.0.to_string()),
        ("DISCTOTAL".into(), disc.1.to_string()),
    ];
    if let Some(d) = &release.date {
        tags.push(("DATE".into(), d.clone()));
    }
    if let Some(l) = &release.label {
        tags.push(("LABEL".into(), l.clone()));
    }
    if let Some(c) = &release.catno {
        tags.push(("CATALOGNUMBER".into(), c.clone()));
    }
    if let Some(b) = &release.barcode {
        tags.push(("BARCODE".into(), b.clone()));
    }
    match release.source {
        crate::meta::MetaSource::MusicBrainz => {
            tags.push(("MUSICBRAINZ_ALBUMID".into(), release.id.clone()))
        }
        crate::meta::MetaSource::Discogs => {
            tags.push(("DISCOGS_RELEASE".into(), release.id.clone()))
        }
        crate::meta::MetaSource::AcoustId | crate::meta::MetaSource::Cue => {}
    }
    tags.push((
        "COMMENT".into(),
        format!(
            "cyanrip-frontend · {} · {}",
            release.source.as_str(),
            release.id
        ),
    ));

    for (k, v) in &tags {
        cmd.arg(format!("--set-tag={k}={v}"));
    }
    for line in &kept {
        cmd.arg(format!("--set-tag={line}"));
    }
    cmd.arg(file);
    let st = cmd.status().context("metaflac 执行失败")?;
    if !st.success() {
        bail!("metaflac 写入失败：{}", file.display());
    }
    Ok(())
}

/// 在输出根目录下找最近一次抓轨产生的目录（含 .flac 的目录）。
/// 输出根目录下所有含 FLAC 的子目录
/// 文件头是不是 "fLaC"，并且不是空文件
pub fn is_complete_flac(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return false;
    }
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    &magic == b"fLaC" && len > 1024
}

/// 目录里的 rip 日志是否属于指定 DiscID
pub fn dir_log_has_discid(dir: &Path, discid: &str) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|x| x.to_str()) != Some("log") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            if text.contains("DiscID") && text.contains(discid) {
                return true;
            }
        }
    }
    false
}

pub fn dirs_with_flac(root: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            std::fs::read_dir(p)
                .map(|rd| {
                    rd.flatten()
                        .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("flac"))
                })
                .unwrap_or(false)
        })
        .collect()
}

pub fn newest_rip_dir(root: &Path) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let has_flac = std::fs::read_dir(&path)
            .map(|rd| {
                rd.flatten()
                    .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("flac"))
            })
            .unwrap_or(false);
        if !has_flac {
            continue;
        }
        if let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) {
            if best.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
                best = Some((mtime, path));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// 列出目录里的 FLAC，按文件名排序（cyanrip 的 01/02… 命名可直接用）。
pub fn flac_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("flac"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_path_and_windows_chars() {
        assert_eq!(sanitize("A/B:C*D?E\"F<G>H|I"), "A_B_C_D_E_F_G_H_I");
        assert_eq!(sanitize("  normal  "), "normal");
        assert_eq!(sanitize(""), "未命名");
    }

    #[test]
    fn keeps_replaygain_and_discid() {
        assert!(keep_tag("REPLAYGAIN_TRACK_GAIN=-7.44 dB"));
        assert!(keep_tag("r128_track_gain=-625"));
        assert!(keep_tag("musicbrainz_discid=abc"));
        assert!(!keep_tag("TITLE=旧标题"));
        assert!(!keep_tag("ALBUM=Unknown"));
    }
}
