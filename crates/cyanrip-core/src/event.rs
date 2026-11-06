//! core 向 UI 上报的事件。UI 只依赖这一组类型，不直接接触 tokio。

use crate::model::{DiscInfo, ProgressUpdate, RipReport, TrackResult};
use std::path::PathBuf;

/// 出错阶段，便于 UI 给出针对性提示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Probe,
    Rip,
    Parse,
    Meta,
    Tag,
}

impl Stage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Stage::Probe => "探测碟片",
            Stage::Rip => "抓轨",
            Stage::Parse => "解析日志",
            Stage::Meta => "元数据",
            Stage::Tag => "写标签",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    /// 碟片已探测
    DiscProbed(DiscInfo),
    /// 碟片被取出或介质变更
    DiscRemoved,
    /// 开始处理某一轨
    TrackStarted { index: u32, title: Option<String> },
    /// 进度更新（core 已节流到 ≤10Hz）
    Progress(ProgressUpdate),
    /// cyanrip 的原始输出行（已按 \r 拆分）
    LogLine(String),
    /// 单轨完成（来自日志解析）
    TrackDone(TrackResult),
    /// 整张碟完成
    RipFinished {
        report: RipReport,
        log_path: Option<PathBuf>,
    },
    /// 失败
    RipFailed { stage: Stage, message: String },
    /// 用户取消，已清理残片
    Cancelled { completed_tracks: Vec<u32> },
}
