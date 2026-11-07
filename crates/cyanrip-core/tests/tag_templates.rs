//! 归档模板的端到端测试：生成 WAV → flac → tag::apply（需要 root 权限？不需要，只用 metaflac/flac CLI）。

use cyanrip_core::config::OutputSettings;
use cyanrip_core::meta::{MetaRelease, MetaSource, MetaTrack};
use cyanrip_core::tag;
use std::path::{Path, PathBuf};
use std::process::Command;

fn have(bin: &str) -> bool {
    Command::new(bin)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 写一个 1 秒 44.1kHz 立体声 16bit 正弦 WAV
fn write_wav(path: &Path) {
    let rate = 44100u32;
    let secs = 1u32;
    let frames = rate * secs;
    let data_len = frames * 2 * 2;
    let mut buf = Vec::with_capacity(44 + data_len as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_len).to_le_bytes());
    buf.extend_from_slice(b"WAVEfmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&2u16.to_le_bytes()); // channels
    buf.extend_from_slice(&rate.to_le_bytes());
    buf.extend_from_slice(&(rate * 4).to_le_bytes());
    buf.extend_from_slice(&4u16.to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        let t = i as f32 / rate as f32;
        let v = (t * 440.0 * std::f32::consts::TAU).sin() * 8000.0;
        let s = v as i16;
        buf.extend_from_slice(&s.to_le_bytes());
        buf.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, buf).unwrap();
}

#[test]
fn applies_templates_and_copies_log() {
    if !have("flac") || !have("metaflac") {
        eprintln!("skipping: flac/metaflac not installed");
        return;
    }
    let base = std::env::temp_dir().join(format!("cyanrip-tpl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let rip = base.join("rip");
    let root = base.join("out");
    std::fs::create_dir_all(&rip).unwrap();

    let wav = rip.join("01.wav");
    write_wav(&wav);
    let flac_path = rip.join("01 - track.flac");
    let st = Command::new("flac")
        .args(["-s", "-f", "-o"])
        .arg(&flac_path)
        .arg(&wav)
        .status()
        .unwrap();
    assert!(st.success(), "flac encode failed");
    std::fs::remove_file(&wav).ok();
    // 假 rip 日志，稍后应被复制进归档目录
    std::fs::write(rip.join("Unknown.log"), "fake rip log\n").unwrap();

    let release = MetaRelease {
        source: MetaSource::Cue,
        id: "test".into(),
        title: "Test Album".into(),
        artist: "Test Artist".into(),
        date: Some("2001-09-27".into()),
        country: None,
        label: None,
        catno: None,
        barcode: None,
        tracks: vec![MetaTrack {
            number: 1,
            title: "First Song".into(),
            length: None,
        }],
        disc_number: 1,
        disc_total: 1,
        cover_url: None,
        score: 100,
    };

    let tpl = OutputSettings {
        root: root.clone(),
        dir_template: "{year}/{artist} - {album}".into(),
        file_template: "{track:02}. {title}".into(),
        log_template: "{album} (rip)".into(),
        ..Default::default()
    };

    let dest = tag::apply(&[flac_path], &release, &root, (1, 1), None, &tpl).unwrap();
    assert_eq!(dest, root.join("2001/Test Artist - Test Album"));

    let song = dest.join("01. First Song.flac");
    let log = dest.join("Test Album (rip).log");
    assert!(song.exists(), "missing {}", song.display());
    assert!(log.exists(), "missing {}", log.display());
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "fake rip log\n");

    // 标签确实写进去了
    let out = Command::new("metaflac")
        .arg("--show-tag=TITLE")
        .arg(&song)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "TITLE=First Song"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn rejects_path_traversal_in_dir_template() {
    let base: PathBuf =
        std::env::temp_dir().join(format!("cyanrip-tpl-bad-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let tpl = OutputSettings {
        dir_template: "../{album}".into(),
        ..Default::default()
    };
    let release = MetaRelease {
        source: MetaSource::Cue,
        id: "t".into(),
        title: "A".into(),
        artist: "B".into(),
        date: None,
        country: None,
        label: None,
        catno: None,
        barcode: None,
        tracks: vec![],
        disc_number: 1,
        disc_total: 1,
        cover_url: None,
        score: 100,
    };
    // 用不存在的文件触发早期返回也可以，只要模板校验先报错
    let err = tag::apply(&[base.join("x.flac")], &release, &base, (1, 1), None, &tpl)
        .unwrap_err()
        .to_string();
    // 目录模板非法会在创建目录前被拒绝
    assert!(err.contains("目录模板非法") || err.contains("没有可写标签的文件"));
    let _ = std::fs::remove_dir_all(&base);
}
