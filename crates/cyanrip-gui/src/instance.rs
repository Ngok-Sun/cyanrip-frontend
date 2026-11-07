//! 多实例登记：让同时开着的前端知道彼此存在。
//!
//! 只在 `$XDG_RUNTIME_DIR/cyanrip-frontend/` 下放 `<pid>.lock`，
//! 内容是该实例当前选中的设备。用途有两个：
//! 1. 「抓完关机」前确认没有别的实例还在干活；
//! 2. 调试时能看出同时跑了几个前端。

use std::path::{Path, PathBuf};

fn lock_dir() -> Option<PathBuf> {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("cyanrip-frontend");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// 登记本实例，返回锁文件路径（退出时删除）。
pub fn register(device: &str) -> Option<PathBuf> {
    let dir = lock_dir()?;
    let path = dir.join(format!("{}.lock", std::process::id()));
    std::fs::write(&path, device).ok()?;
    Some(path)
}

pub fn unregister(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// 还活着的其他实例数量（顺手清理已退出实例留下的锁）。
pub fn other_instances() -> usize {
    let Some(dir) = lock_dir() else {
        return 0;
    };
    let me = std::process::id();
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|x| x.to_str()) != Some("lock") {
                continue;
            }
            let Some(pid) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            if pid == me {
                continue;
            }
            if Path::new(&format!("/proc/{pid}")).exists() {
                count += 1;
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    count
}
