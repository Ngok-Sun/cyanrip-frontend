//! GUI 的应用状态。UI 单线程，用 `Rc<RefCell<AppState>>` 共享。

use cyanrip_core::config::Settings;
use cyanrip_core::event::Event;
use cyanrip_core::model::{DiscInfo, ProgressUpdate, RipReport};
use std::path::PathBuf;
use std::sync::mpsc;
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Idle,
    Ripping,
    Done,
    Failed,
}

pub struct AppState {
    pub settings: Settings,
    pub config_path: PathBuf,
    pub device: String,
    pub disc: Option<DiscInfo>,
    /// 与 `disc.audio_tracks()` 一一对应
    pub selected: Vec<bool>,
    pub phase: Phase,
    pub progress: Option<ProgressUpdate>,
    pub report: Option<RipReport>,
    /// 抓轨工作线程 → UI 的事件通道
    pub events: Option<mpsc::Receiver<Event>>,
    pub cancel: Option<watch::Sender<bool>>,
    pub log_text: String,
}

impl AppState {
    pub fn new() -> Self {
        let config_path = Settings::default_path();
        let settings = Settings::load(&config_path).unwrap_or_default();
        let device = settings.device.path.display().to_string();
        Self {
            settings,
            config_path,
            device,
            disc: None,
            selected: Vec::new(),
            phase: Phase::Idle,
            progress: None,
            report: None,
            events: None,
            cancel: None,
            log_text: String::new(),
        }
    }

    pub fn save_settings(&self) {
        if let Err(e) = self.settings.save(&self.config_path) {
            eprintln!("保存配置失败：{e:#}");
        }
    }

    /// 当前选中的音频轨号
    pub fn selected_tracks(&self) -> Vec<u32> {
        match &self.disc {
            Some(d) => d
                .audio_tracks()
                .enumerate()
                .filter(|(i, _)| self.selected.get(*i).copied().unwrap_or(true))
                .map(|(_, t)| t.number)
                .collect(),
            None => Vec::new(),
        }
    }

    /// 该光驱型号记住的偏移（没有则用设置里的默认）
    pub fn offset(&self) -> i32 {
        self.disc
            .as_ref()
            .and_then(|d| self.settings.offset_for(&d.model))
            .unwrap_or(
                self.settings
                    .device
                    .offsets
                    .values()
                    .next()
                    .copied()
                    .unwrap_or(0),
            )
    }

    pub fn append_log(&mut self, line: &str) {
        self.log_text.push_str(line);
        self.log_text.push('\n');
        // 日志面板不需要无限增长
        if self.log_text.len() > 512 * 1024 {
            let cut = self.log_text.len() - 256 * 1024;
            if let Some(pos) = self.log_text[cut..].find('\n') {
                self.log_text.drain(..cut + pos + 1);
            }
        }
    }
}
