//! 抓轨编排：启动 cyanrip、切分输出流、产出事件、结束时解析日志。
//!
//! 关键设计：
//! - stdout / stderr 分别读取，按 `\r` 与 `\n` 切行（cyanrip 的进度行用 `\r`）
//! - 进度事件节流到 ≤10Hz，避免刷爆 GTK 主循环
//! - 取消通过 `watch` 通道触发，先 kill 子进程再返回，已完成轨的信息不丢
//! - 结束时优先解析 cyanrip 自己写的日志文件（比 stdout 完整）

use crate::event::{Event, Stage};
use crate::logparse::parse_log;
use crate::model::{DiscInfo, RipReport, TrackResult};
use crate::progress::{parse_progress, ProgressTracker};
use anyhow::{Context, Result};
use async_channel::Sender;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::watch;

pub const DEFAULT_BIN: &str = "cyanrip";
const PROGRESS_THROTTLE: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct RipOptions {
    pub bin: PathBuf,
    pub device: PathBuf,
    pub offset: i32,
    /// 透传给 cyanrip 的参数（-Z/-P/-o/...）
    pub extra_args: Vec<String>,
    /// 结束时优先解析这份日志
    pub log_path: Option<PathBuf>,
}

impl RipOptions {
    pub fn new(device: impl Into<PathBuf>, offset: i32) -> Self {
        Self {
            bin: PathBuf::from(DEFAULT_BIN),
            device: device.into(),
            offset,
            extra_args: Vec::new(),
            log_path: None,
        }
    }

    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.extra_args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn command_args(&self) -> Vec<String> {
        let mut v = vec![
            "-d".to_string(),
            self.device.display().to_string(),
            "-s".to_string(),
            self.offset.to_string(),
        ];
        v.extend(self.extra_args.iter().cloned());
        v
    }
}

/// 把字节流按 `\r` / `\n` 切行，处理跨 chunk 的半行。
#[derive(Default)]
pub struct LineSplitter {
    buf: Vec<u8>,
}

impl LineSplitter {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        let mut start = 0usize;
        for (i, b) in self.buf.iter().enumerate() {
            if *b == b'\r' || *b == b'\n' {
                if i > start {
                    out.push(String::from_utf8_lossy(&self.buf[start..i]).into_owned());
                }
                start = i + 1;
            }
        }
        if start > 0 {
            self.buf.drain(..start);
        }
        out
    }

    pub fn finish(&mut self) -> Option<String> {
        if self.buf.is_empty() {
            None
        } else {
            let s = String::from_utf8_lossy(&self.buf).into_owned();
            self.buf.clear();
            Some(s)
        }
    }
}

/// 从透传参数里取 `-Z N`（每轨最少读取 N+1 遍），用于折算全碟进度。
pub fn verify_passes_from_args(args: &[String]) -> u8 {
    args.iter()
        .position(|a| a == "-Z")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u8>().ok())
        .unwrap_or(0)
}

/// `Track 5 ripped and encoded successfully!` → Some(5)
pub fn parse_track_done(line: &str) -> Option<u32> {
    let rest = line.strip_prefix("Track ")?;
    let (num, tail) = rest.split_once(' ')?;
    tail.starts_with("ripped and encoded successfully")
        .then(|| num.parse().ok())
        .flatten()
}

fn spawn_reader<R>(mut r: R, tx: Sender<String>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut splitter = LineSplitter::default();
        let mut buf = [0u8; 8192];
        loop {
            match r.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    for line in splitter.push(&buf[..n]) {
                        if tx.send(line).await.is_err() {
                            return;
                        }
                    }
                }
                Err(_) => break,
            }
        }
        if let Some(line) = splitter.finish() {
            let _ = tx.send(line).await;
        }
    });
}

/// 跑一次抓轨。
///
/// 事件会按到达顺序推送到 `tx`；函数返回时 `tx` 仍可用（调用方自行关闭）。
/// `cancel` 变为 `true` 时终止子进程并返回已解析的部分结果。
pub async fn run(
    opts: RipOptions,
    disc: &DiscInfo,
    tx: Sender<Event>,
    mut cancel: watch::Receiver<bool>,
) -> Result<RipReport> {
    let mut cmd = Command::new(&opts.bin);
    cmd.args(opts.command_args())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().with_context(|| {
        format!(
            "无法启动 {}（参数 {:?}）",
            opts.bin.display(),
            opts.command_args()
        )
    })?;

    let (line_tx, line_rx) = async_channel::unbounded::<String>();
    if let Some(out) = child.stdout.take() {
        spawn_reader(out, line_tx.clone());
    }
    if let Some(err) = child.stderr.take() {
        spawn_reader(err, line_tx.clone());
    }
    drop(line_tx);

    let passes = verify_passes_from_args(&opts.extra_args);
    let mut tracker = ProgressTracker::with_passes(disc, passes);
    let mut captured = String::new();
    let mut last_emit = Instant::now() - PROGRESS_THROTTLE;
    let mut current_track = 0u32;
    let mut cancelled = false;

    loop {
        tokio::select! {
            biased;
            changed = cancel.changed() => {
                if changed.is_ok() && *cancel.borrow() {
                    cancelled = true;
                    let _ = child.kill().await;
                    break;
                }
            }
            line = line_rx.recv() => {
                let Ok(line) = line else { break };
                captured.push_str(&line);
                captured.push('\n');
                let _ = tx.send(Event::LogLine(line.clone())).await;

                if let Some(p) = parse_progress(&line) {
                    if p.track != current_track {
                        current_track = p.track;
                        let _ = tx.send(Event::TrackStarted { index: p.track, title: None }).await;
                    }
                    if last_emit.elapsed() >= PROGRESS_THROTTLE {
                        let update = tracker.update(&p);
                        let _ = tx.send(Event::Progress(update)).await;
                        last_emit = Instant::now();
                    }
                } else if let Some(n) = parse_track_done(&line) {
                    let _ = tx.send(Event::TrackDone(TrackResult::new(n))).await;
                }
            }
        }
    }

    let status = child.wait().await.context("等待 cyanrip 退出失败")?;

    let mut report = match &opts.log_path {
        Some(p) if p.exists() => std::fs::read_to_string(p)
            .map(|s| parse_log(&s))
            .unwrap_or_else(|_| parse_log(&captured)),
        _ => parse_log(&captured),
    };
    report.log_path = opts.log_path.clone();

    if cancelled {
        let done: Vec<u32> = report.tracks.iter().map(|t| t.number).collect();
        let _ = tx
            .send(Event::Cancelled {
                completed_tracks: done,
            })
            .await;
        return Ok(report);
    }

    if !status.success() {
        let msg = format!(
            "cyanrip 退出码 {:?}；日志尾部：{}",
            status.code(),
            captured
                .lines()
                .rev()
                .take(3)
                .collect::<Vec<_>>()
                .join(" / ")
        );
        let _ = tx
            .send(Event::RipFailed {
                stage: Stage::Rip,
                message: msg.clone(),
            })
            .await;
        anyhow::bail!(msg);
    }

    let _ = tx
        .send(Event::RipFinished {
            report: report.clone(),
            log_path: opts.log_path.clone(),
        })
        .await;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitter_handles_cr_and_lf() {
        let mut s = LineSplitter::default();
        assert_eq!(
            s.push(b"Ripping track 1, progress - 10.00%\r"),
            vec!["Ripping track 1, progress - 10.00%"]
        );
        assert_eq!(
            s.push(b"Track 1 ripped and encoded successfully!\nnext"),
            vec!["Track 1 ripped and encoded successfully!"]
        );
        assert_eq!(s.finish().as_deref(), Some("next"));
        assert_eq!(s.finish(), None);
    }

    #[test]
    fn splitter_handles_chunk_boundaries() {
        let mut s = LineSplitter::default();
        assert!(s.push(b"hello wo").is_empty());
        assert_eq!(s.push(b"rld\rsecond"), vec!["hello world"]);
        assert_eq!(s.finish().as_deref(), Some("second"));
    }

    #[test]
    fn parses_track_done_lines() {
        assert_eq!(
            parse_track_done("Track 12 ripped and encoded successfully!"),
            Some(12)
        );
        assert_eq!(parse_track_done("Track 1 ripped and encoded"), None);
        assert_eq!(parse_track_done("Ripping track 1"), None);
    }

    #[test]
    fn reads_verify_passes_from_args() {
        let args = vec![
            "-Z".to_string(),
            "2".to_string(),
            "-P".to_string(),
            "3".to_string(),
        ];
        assert_eq!(verify_passes_from_args(&args), 2);
        assert_eq!(verify_passes_from_args(&[]), 0);
        assert_eq!(verify_passes_from_args(&["-P".to_string()]), 0);
    }

    #[test]
    fn command_args_put_offset_first() {
        let o = RipOptions::new("/dev/sr0", 48).with_args(["-Z", "2", "-P", "3"]);
        assert_eq!(
            o.command_args(),
            vec!["-d", "/dev/sr0", "-s", "48", "-Z", "2", "-P", "3"]
        );
    }
}
