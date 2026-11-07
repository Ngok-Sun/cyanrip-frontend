//! 用假 cyanrip 脚本验证流式事件与日志解析的整条链路（不需要真光驱）。

use cyanrip_core::event::Event;
use cyanrip_core::model::{DiscInfo, TrackInfo};
use cyanrip_core::rip::{run, RipOptions};
use std::os::unix::fs::PermissionsExt;

fn fake_disc() -> DiscInfo {
    let mk = |number: u32, frames: u32| TrackInfo {
        number,
        start_lsn: 0,
        duration_frames: frames,
        is_audio: true,
        title: None,
        isrc: None,
    };
    DiscInfo {
        device: "/dev/null".into(),
        vendor: "FAKE".into(),
        model: "FAKE CD-ROM".into(),
        firmware: Some("0.1".into()),
        tracks: vec![mk(1, 14860), mk(2, 15705)],
        leadout_lsn: 30565,
        total_frames: 30565,
        disc_id: None,
        cddb_id: None,
        mcn: None,
        ar_in_db: Some(true),
    }
}

fn write_fake_bin(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("fake-cyanrip.sh");
    let script = r#"#!/bin/sh
printf 'cyanrip 0.9.4-rc3 (fake)\n'
printf 'Offset:         +48 samples\n'
printf 'C2 errors:      unsupported by drive\n'
printf 'Paranoia level: max\n'
printf 'Disc tracks:    2\n'
printf 'Total time:     10:11.00\n'
i=0
while [ $i -le 100 ]; do
  printf '\rRipping track 1, progress - %d.00%%, ETA - 1m' $i
  i=$((i+25))
done
printf '\nTrack 1 ripped and encoded successfully!\n'
printf '  Duration:    03:18.10\n'
printf '  EAC CRC32:     01834C83\n'
printf '  Accurip v1:  6F096812 (accurately ripped, confidence 2)\n'
printf '  Accurip v2:  17AF569B (accurately ripped, confidence 8)\n'
i=0
while [ $i -le 100 ]; do
  printf '\rRipping and encoding track 2, progress - %d.00%%, ETA - 30s' $i
  i=$((i+25))
done
printf '\nTrack 2 ripped and encoded successfully!\n'
printf '  Duration:    03:29.15\n'
printf '  EAC CRC32:     0A1B2C3D\n'
printf '  Accurip v1:  11111111 (accurately ripped, confidence 2)\n'
printf '  Accurip v2:  22222222 (accurately ripped, confidence 8)\n'
printf 'Tracks ripped accurately: 2/2\n'
printf 'Ripping errors: 0\n'
exit 0
"#;
    std::fs::write(&path, script).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

#[tokio::test]
async fn streams_events_and_parses_report() {
    let dir = std::env::temp_dir().join(format!("cyanrip-fake-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = write_fake_bin(&dir);

    let disc = fake_disc();
    let (tx, rx) = async_channel::unbounded::<Event>();
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    let opts = RipOptions {
        bin,
        device: "/dev/null".into(),
        offset: 48,
        extra_args: vec!["-Z".into(), "2".into()],
        log_path: None,
        cwd: None,
        selected: Vec::new(),
    };

    let report = run(opts, &disc, tx, cancel_rx)
        .await
        .expect("rip should succeed");

    // 收集事件
    let mut progress = 0usize;
    let mut track_done = Vec::new();
    let mut finished = false;
    let mut started = 0usize;
    let mut log_lines: Vec<String> = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        match ev {
            Event::Progress(p) => {
                progress += 1;
                assert!(p.overall_pct >= 0.0 && p.overall_pct <= 100.0);
            }
            Event::TrackStarted { .. } => started += 1,
            Event::LogLine(l) => log_lines.push(l),
            Event::TrackDone(t) => track_done.push(t.number),
            Event::RipFinished { .. } => finished = true,
            _ => {}
        }
    }

    assert!(progress > 0, "应收到进度事件");
    assert!(
        !log_lines.iter().any(|l| l.contains("progress -")),
        "进度行不应进入日志通道"
    );
    assert!(
        log_lines
            .iter()
            .any(|l| l.contains("ripped and encoded successfully")),
        "普通日志行仍应保留"
    );
    assert!(
        started >= 2,
        "两轨各应触发一次 TrackStarted，实际 {started}"
    );
    assert_eq!(track_done, vec![1, 2]);
    assert!(finished, "应收到 RipFinished");

    assert_eq!(report.offset, Some(48));
    assert_eq!(report.tracks_ripped, 2);
    assert_eq!(report.tracks_accurate, 2);
    assert_eq!(report.errors, 0);
    assert_eq!(report.track(1).unwrap().crc32, Some(0x0183_4C83));
    assert_eq!(report.track(2).unwrap().ar_v2, Some(0x2222_2222));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn cancellation_stops_child() {
    let dir = std::env::temp_dir().join(format!("cyanrip-fake-cancel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("slow.sh");
    std::fs::write(
        &bin,
        "#!/bin/sh\nprintf 'Disc tracks:    1\\n'\ni=0\nwhile [ $i -le 100 ]; do\n  printf '\\rRipping track 1, progress - 1.00%%, ETA - 9m'\n  sleep 0.1\n  i=$((i+1))\ndone\n",
    )
    .unwrap();
    let mut perms = std::fs::metadata(&bin).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&bin, perms).unwrap();

    let disc = fake_disc();
    let (tx, rx) = async_channel::unbounded::<Event>();
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    let opts = RipOptions {
        bin,
        device: "/dev/null".into(),
        offset: 0,
        extra_args: vec![],
        log_path: None,
        cwd: None,
        selected: Vec::new(),
    };

    let handle = tokio::spawn(async move { run(opts, &disc, tx, cancel_rx).await });

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    cancel_tx.send(true).unwrap();

    let report = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
        .await
        .expect("取消应及时返回")
        .expect("join ok")
        .expect("cancelled run returns Ok");

    let mut cancelled = false;
    while let Ok(ev) = rx.try_recv() {
        if matches!(ev, Event::Cancelled { .. }) {
            cancelled = true;
        }
    }
    assert!(cancelled, "应收到 Cancelled 事件");
    assert_eq!(report.tracks_ripped, 0);

    std::fs::remove_dir_all(&dir).ok();
}
