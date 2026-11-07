//! GUI 的应用状态。UI 单线程，用 `Rc<RefCell<AppState>>` 共享。

use cyanrip_core::config::{Secrets, Settings};
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
    /// 密钥（Discogs token）单独存 secrets.toml（0600）
    pub secrets: Secrets,
    pub secrets_path: PathBuf,
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
    /// 本次抓轨开始前，输出根目录下已有的 FLAC 目录（用来精确定位本次产物）
    pub rip_dirs_before: Option<Vec<PathBuf>>,
}

impl AppState {
    pub fn new() -> Self {
        // 多开时用 CYANRIP_GUI_CONFIG 指定独立配置，避免两个实例互相覆盖设置
        let config_path = std::env::var("CYANRIP_GUI_CONFIG")
            .ok()
            .filter(|p| !p.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(Settings::default_path);
        let secrets_path = Secrets::path_for(&config_path);
        let mut secrets = Secrets::load(&secrets_path);
        if secrets.discogs_token.trim().is_empty() {
            // 迁移旧位置：~/.config/discogs/token（只回填到界面，点保存才写入 secrets.toml）
            if let Ok(home) = std::env::var("HOME") {
                let legacy = PathBuf::from(home).join(".config/discogs/token");
                if let Ok(text) = std::fs::read_to_string(legacy) {
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        secrets.discogs_token = text;
                    }
                }
            }
        }
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
            secrets,
            secrets_path,
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
            rip_dirs_before: None,
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
        if let Err(e) = self.secrets.save(&self.secrets_path) {
            eprintln!("保存密钥失败：{e:#}");
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

    /// 该光驱型号记住的偏移；未测定时返回 0，**不能**借用别的光驱的值。
    pub fn offset(&self) -> i32 {
        self.disc
            .as_ref()
            .and_then(|d| self.settings.offset_for(&d.model))
            .unwrap_or(0)
    }

    /// 当前光驱是否已测定过偏移
    pub fn offset_known(&self) -> bool {
        self.disc
            .as_ref()
            .map(|d| self.settings.offset_for(&d.model).is_some())
            .unwrap_or(false)
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
        trim_log_tail(&mut self.log_text, 512 * 1024, 256 * 1024);
    }
}

/// 日志超过 `max` 字节时，从中间砍掉一段，只保留后半部分。
/// 截断点必须落在 UTF-8 字符边界上（曲名/路径里多字节字符很常见）。
fn trim_log_tail(text: &mut String, max: usize, keep: usize) {
    if text.len() <= max {
        return;
    }
    let mut cut = text.len() - keep;
    while cut < text.len() && !text.is_char_boundary(cut) {
        cut += 1;
    }
    if let Some(pos) = text[cut..].find('\n') {
        text.drain(..cut + pos + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_log_on_char_boundary() {
        // 大量三字节字符：按字节截断很容易落在字符中间
        let mut s = String::new();
        for i in 0..40_000 {
            s.push_str(&format!("第{i}轨：久石譲 Quartet\n"));
        }
        let before = s.len();
        trim_log_tail(&mut s, 512 * 1024, 256 * 1024);
        assert!(s.len() < before);
        assert!(s.len() <= 512 * 1024);
        // 仍然可以正常按行切分（说明边界对齐了）
        assert!(s.lines().all(|l| l.contains('轨')));
    }
}
