//! cyanrip-frontend 的核心层：碟片探测、抓轨编排、进度与日志解析。
//!
//! 这一层不依赖任何 GUI 工具包，可以单独用 `cargo test` 验证，也可以被
//! `cyanrip-cli` 这样的无头程序复用。

pub mod config;
pub mod disc;
pub mod event;
pub mod logparse;
pub mod model;
pub mod progress;
pub mod rip;

pub use event::{Event, Stage};
pub use model::{
    fmt_duration, ArState, DiscInfo, ProgressUpdate, RipReport, TrackInfo, TrackResult,
};
pub use progress::{parse_progress, ProgressLine, ProgressTracker};
