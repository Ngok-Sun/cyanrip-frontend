//! 碟片探测：直接读 Linux cdrom ioctl 的 TOC。
//!
//! 走 ioctl 而不是解析 `cdrdao`/`cd-info` 的文本，原因：
//! - 启动零开销，没有外部进程
//! - LSN 与 control 位是原始值，不会因为工具输出格式变化而失效
//! - ISRC 仍需子通道读取（M3 用 cdrdao 补）
//!
//! 注意：本模块只在 Linux 上编译。

use crate::model::{DiscInfo, TrackInfo};
use anyhow::{bail, Context, Result};
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

// 注意：0x5302 是 CDROMRESUME，TOCHDR 是 0x5305（linux/cdrom.h）
const CDROMREADTOCHDR: libc::c_ulong = 0x5305;
const CDROMREADTOCENTRY: libc::c_ulong = 0x5306;
const CDROM_DRIVE_STATUS: libc::c_ulong = 0x5326;
const CDROMEJECT: libc::c_ulong = 0x5309;
const CDROMCLOSETRAY: libc::c_ulong = 0x5319;

const CDROM_LBA: u8 = 0x01;
const CDROM_LEADOUT: u8 = 0xAA;

const CDS_NO_DISC: i32 = 1;
const CDS_TRAY_OPEN: i32 = 2;
const CDS_DRIVE_NOT_READY: i32 = 3;

/// `struct cdrom_tochdr`
#[repr(C)]
#[derive(Default, Debug, Clone, Copy)]
struct TocHdr {
    trk0: u8,
    trk1: u8,
}

/// `struct cdrom_tocentry`（x86_64/aarch64 布局一致：track, adr:4|ctrl:4, format, pad, lba, datamode, pad）
#[repr(C)]
#[derive(Default, Debug, Clone, Copy)]
struct TocEntry {
    track: u8,
    /// 低 4 位 adr，高 4 位 control
    adr_ctrl: u8,
    format: u8,
    _pad0: u8,
    lba: u32,
    datamode: u8,
    _pad1: [u8; 3],
}

impl TocEntry {
    fn control(&self) -> u8 {
        self.adr_ctrl >> 4
    }
}

struct Device {
    fd: libc::c_int,
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

fn open_device(device: &Path) -> Result<Device> {
    let c = CString::new(device.as_os_str().as_bytes())
        .with_context(|| format!("设备路径包含 NUL: {}", device.display()))?;
    let fd = unsafe {
        libc::open(
            c.as_ptr(),
            libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("无法打开设备 {}", device.display()));
    }
    Ok(Device { fd })
}

fn read_toc_header(dev: &Device) -> Result<TocHdr> {
    let mut hdr = TocHdr::default();
    let r = unsafe { libc::ioctl(dev.fd, CDROMREADTOCHDR, &mut hdr as *mut TocHdr) };
    if r < 0 {
        return Err(std::io::Error::last_os_error()).context("CDROMREADTOCHDR 失败");
    }
    Ok(hdr)
}

fn read_toc_entry(dev: &Device, track: u8) -> Result<TocEntry> {
    let mut e = TocEntry {
        track,
        format: CDROM_LBA,
        ..Default::default()
    };
    let r = unsafe { libc::ioctl(dev.fd, CDROMREADTOCENTRY, &mut e as *mut TocEntry) };
    if r < 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("CDROMREADTOCENTRY 失败（track {track}）"));
    }
    Ok(e)
}

/// 探测设备中的碟片。
pub fn probe(device: &Path) -> Result<DiscInfo> {
    let dev = open_device(device)?;

    let status = unsafe { libc::ioctl(dev.fd, CDROM_DRIVE_STATUS, 0) } as i32;
    match status {
        CDS_NO_DISC => bail!("光驱中没有碟片"),
        CDS_TRAY_OPEN => bail!("光驱托盘已打开"),
        CDS_DRIVE_NOT_READY => bail!("光驱尚未就绪"),
        s if s < 0 => {
            return Err(std::io::Error::last_os_error()).context("CDROM_DRIVE_STATUS 失败")
        }
        _ => {}
    }

    let hdr = read_toc_header(&dev)?;
    if hdr.trk0 == 0 || hdr.trk1 < hdr.trk0 {
        bail!("TOC 异常：first={} last={}", hdr.trk0, hdr.trk1);
    }

    let mut raw = Vec::new();
    for n in hdr.trk0..=hdr.trk1 {
        let e = read_toc_entry(&dev, n)?;
        raw.push((n as u32, e.lba, e.control()));
    }
    let lead = read_toc_entry(&dev, CDROM_LEADOUT)?;
    if lead.lba == 0 {
        bail!("无法读取 lead-out（返回 LBA 0）");
    }

    let (vendor, model, firmware) = drive_attrs(device);
    let tracks = build_tracks(&raw, lead.lba);
    let total_frames = lead.lba;

    Ok(DiscInfo {
        device: device.to_path_buf(),
        vendor,
        model,
        firmware,
        tracks,
        leadout_lsn: lead.lba,
        total_frames,
        disc_id: None,
        cddb_id: None,
        mcn: None,
        ar_in_db: None,
    })
}

/// 从原始 TOC 条目构造轨道列表。
///
/// `raw` 为 `(track_number, start_lba, control)`，按轨号升序；
/// `leadout` 为 lead-out LBA。长度由相邻起点相减得到。
pub fn build_tracks(raw: &[(u32, u32, u8)], leadout: u32) -> Vec<TrackInfo> {
    let mut out = Vec::with_capacity(raw.len());
    for (i, (num, start, ctrl)) in raw.iter().enumerate() {
        let next = raw.get(i + 1).map(|r| r.1).unwrap_or(leadout);
        out.push(TrackInfo {
            number: *num,
            start_lsn: *start,
            duration_frames: next.saturating_sub(*start),
            is_audio: ctrl & 0x04 == 0,
            title: None,
            isrc: None,
        });
    }
    out
}

/// 从 sysfs 读取光驱厂商/型号/固件（GUI 用来给设备下拉生成标签）。
pub fn drive_attrs(device: &Path) -> (String, String, Option<String>) {
    let name = device
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    let base = PathBuf::from("/sys/block").join(&name).join("device");
    let get = |f: &str| -> Option<String> {
        std::fs::read_to_string(base.join(f))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    (
        get("vendor").unwrap_or_else(|| "unknown".into()),
        get("model").unwrap_or_else(|| "unknown".into()),
        get("rev"),
    )
}

/// 光驱/介质状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaStatus {
    /// 有碟且可读
    Ok,
    NoDisc,
    TrayOpen,
    NotReady,
    Unknown,
}

/// 查询当前介质状态（GUI 用来轮询换碟）。
pub fn media_status(device: &Path) -> MediaStatus {
    let Ok(dev) = open_device(device) else {
        return MediaStatus::Unknown;
    };
    let status = unsafe { libc::ioctl(dev.fd, CDROM_DRIVE_STATUS, 0) } as i32;
    match status {
        CDS_NO_DISC => MediaStatus::NoDisc,
        CDS_TRAY_OPEN => MediaStatus::TrayOpen,
        CDS_DRIVE_NOT_READY => MediaStatus::NotReady,
        s if s < 0 => MediaStatus::Unknown,
        _ => MediaStatus::Ok,
    }
}

/// 弹出托盘。
///
/// 这台机器的光驱经 SiI3811 桥接，`CDROMEJECT` ioctl 会被驱动/桥接层以
/// `MEDIUM REMOVAL PREVENTED (0x53/0x02)` 拒绝——ioctl 甚至可能返回 0 但托盘不动。
/// util-linux 的 `eject` 会先试 CDROMEJECT，失败再走 SCSI `START STOP UNIT`，
/// 实测只有后一条路径有效，所以优先用它，最后才回退到裸 ioctl。
pub fn eject(device: &Path) -> Result<()> {
    if let Ok(status) = std::process::Command::new("eject").arg(device).status() {
        if status.success() {
            return Ok(());
        }
    }
    let dev = open_device(device)?;
    let r = unsafe { libc::ioctl(dev.fd, CDROMEJECT, 0) };
    if r < 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("弹出 {} 失败", device.display()));
    }
    Ok(())
}

/// 收回托盘（部分吸入式光驱不支持）。
pub fn close_tray(device: &Path) -> Result<()> {
    if let Ok(status) = std::process::Command::new("eject")
        .arg("-t")
        .arg(device)
        .status()
    {
        if status.success() {
            return Ok(());
        }
    }
    let dev = open_device(device)?;
    let r = unsafe { libc::ioctl(dev.fd, CDROMCLOSETRAY, 0) };
    if r < 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("收回 {} 失败", device.display()));
    }
    Ok(())
}

/// 列出系统里的光驱设备（/dev/sr*），用于 GUI 的设备下拉。
pub fn list_devices() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir("/dev")
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .map(|n| n.starts_with("sr") && n[2..].chars().all(|c| c.is_ascii_digit()))
                        .unwrap_or(false)
                })
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_durations_from_neighbouring_starts() {
        // 三个音轨 + 一条数据轨，lead-out 在 10000
        let raw = [
            (1, 0, 0x00),
            (2, 1500, 0x00),
            (3, 4000, 0x04),
            (4, 8000, 0x00),
        ];
        let tracks = build_tracks(&raw, 10000);
        assert_eq!(tracks.len(), 4);
        assert_eq!(tracks[0].duration_frames, 1500);
        assert_eq!(tracks[1].duration_frames, 2500);
        assert_eq!(tracks[2].duration_frames, 4000);
        assert_eq!(tracks[3].duration_frames, 2000);
        assert!(tracks[0].is_audio);
        assert!(!tracks[2].is_audio, "control bit 2 置位应判为数据轨");
        assert_eq!(tracks[0].duration_secs(), 20.0);
    }

    #[test]
    fn empty_toc_is_empty() {
        assert!(build_tracks(&[], 0).is_empty());
    }
}
