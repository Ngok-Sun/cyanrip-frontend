//! GUI 的应用状态。UI 单线程，用 `Rc<RefCell<AppState>>` 共享。

use cyanrip_core::config::Settings;
use cyanrip_core::event::Event;
use cyanrip_core::meta::MetaRelease;
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
    /// 设置页有未保存的改动
    pub dirty: bool,
    /// 识别到的候选发行版（抓轨前就已就绪）
    pub meta_releases: Vec<MetaRelease>,
    /// 当前选中的候选
    pub meta_selected: usize,
    /// 与音频轨一一对应的标题（来自选中的候选）
    pub track_titles: Vec<Option<String>>,
    /// 识别请求还在路上
    pub meta_pending: bool,
    /// 输出根目录被 `CYANRIP_GUI_ROOT` 临时覆盖（自测用）：不要写回配置
    pub ephemeral_root: bool,
}

impl AppState {
    pub fn new() -> Self {
        let config_path = Settings::default_path();
        let mut settings = Settings::load(&config_path).unwrap_or_default();
        // 自测用：临时把输出根目录指到别处，避免污染真实音乐库
        let mut ephemeral_root = false;
        if let Ok(root) = std::env::var("CYANRIP_GUI_ROOT") {
            if !root.trim().is_empty() {
                settings.output.root = PathBuf::from(root);
                ephemeral_root = true;
            }
        }
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
            dirty: false,
            meta_releases: Vec::new(),
            meta_selected: 0,
            track_titles: Vec::new(),
            meta_pending: false,
            ephemeral_root,
        }
    }

    pub fn save_settings(&self) {
        // 根目录是测试覆盖的：整份配置都别写，免得把临时路径带进真实设置
        if self.ephemeral_root {
            return;
        }
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

    /// 选中第 `idx` 个候选，并刷新每轨标题
    pub fn select_release(&mut self, idx: usize) {
        self.meta_selected = idx;
        self.track_titles = self
            .meta_releases
            .get(idx)
            .map(|r| r.tracks.iter().map(|t| Some(t.title.clone())).collect())
            .unwrap_or_default();
    }

    pub fn current_release(&self) -> Option<&MetaRelease> {
        self.meta_releases.get(self.meta_selected)
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
