//! 配置读写。路径：`~/.config/cyanrip-frontend/config.toml`
//!
//! 设计要点：
//! - 所有字段带默认值，旧版本配置缺字段时仍可加载
//! - 读偏移按**光驱型号**记忆，换设备自动套用
//! - 密钥（Discogs token）不放这里，见 `secrets.toml`（0600）

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub device: DeviceSettings,
    pub rip: RipSettings,
    pub output: OutputSettings,
    pub meta: MetaSettings,
    pub behavior: BehaviorSettings,
    pub ui: UiSettings,
    pub advanced: AdvancedSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DeviceSettings {
    pub path: PathBuf,
    /// 光驱型号 → 读偏移（samples）
    pub offsets: BTreeMap<String, i32>,
    pub speed: Option<u16>,
}

impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            path: PathBuf::from("/dev/sr0"),
            offsets: BTreeMap::new(),
            speed: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RipSettings {
    /// -P 0..3
    pub paranoia: u8,
    /// -r
    pub retries: u32,
    /// -Z
    pub verify_passes: u8,
    /// -O
    pub overread: bool,
    /// 是否允许使用 C2 指针（本机驱动不支持，默认 false）
    pub use_c2: bool,
    /// 输出格式，可多选：flac / wav / alac / wavpack / mp3 / aac / opus / vorbis
    pub formats: Vec<String>,
    /// 单文件 + CUE
    pub image_mode: bool,
    /// 有损码率 kbps
    pub bitrate: u32,
}

impl Default for RipSettings {
    fn default() -> Self {
        Self {
            paranoia: 3,
            retries: 10,
            verify_passes: 2,
            overread: false,
            use_c2: false,
            formats: vec!["flac".into()],
            image_mode: false,
            bitrate: 256,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OutputSettings {
    pub root: PathBuf,
    pub dir_template: String,
    pub file_template: String,
    pub log_template: String,
    pub write_cue: bool,
    pub embed_cover: bool,
    pub replaygain: bool,
    pub skip_data_tracks: bool,
}

impl Default for OutputSettings {
    fn default() -> Self {
        Self {
            root: dirs_music(),
            dir_template: "{artist}/{album}".into(),
            file_template: "{track:02} {title}".into(),
            log_template: "{album}".into(),
            write_cue: true,
            embed_cover: true,
            replaygain: true,
            skip_data_tracks: false,
        }
    }
}

fn dirs_music() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|u| u.audio_dir().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("~/Music"))
}

/// 需要保密的设置，单独存 `secrets.toml`（0600），不写进 config.toml
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Secrets {
    #[serde(default)]
    pub discogs_token: String,
    /// Google 自定义搜索（图片）的 API Key / 搜索引擎 ID，用于封面兜底
    #[serde(default)]
    pub google_api_key: String,
    #[serde(default)]
    pub google_cx: String,
}

impl Secrets {
    /// `config.toml` → `secrets.toml`；其它 profile（如 second.toml）→ `second.secrets.toml`
    pub fn path_for(config_path: &Path) -> PathBuf {
        let stem = config_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("config");
        let name = if stem == "config" {
            "secrets.toml".to_string()
        } else {
            format!("{stem}.secrets.toml")
        };
        config_path.with_file_name(name)
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let s = toml::to_string_pretty(self).context("密钥序列化失败")?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, s).context("写临时密钥文件失败")?;
        // 先收紧权限再落位，避免短暂可读
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, path).context("替换密钥文件失败")?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MetaSettings {
    /// 优先级顺序：musicbrainz / acoustid / discogs
    pub priority: Vec<String>,
    pub language: String,
    pub cover_max_px: u32,
}

impl Default for MetaSettings {
    fn default() -> Self {
        Self {
            priority: vec!["musicbrainz".into(), "acoustid".into(), "discogs".into()],
            language: "zh".into(),
            cover_max_px: 1200,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BehaviorSettings {
    pub auto_tag_after_rip: bool,
    pub eject_when_done: bool,
    pub notify: bool,
    /// 抓完播放提示音
    pub sound: bool,
    /// freedesktop 主题音名称（canberra-gtk-play -i）
    pub sound_theme: String,
    /// 非空则播放自定义文件
    pub sound_file: String,
    pub shutdown_when_done: bool,
    /// continue | abort
    pub on_error: String,
    /// pause | rip_placeholder
    pub unknown_disc: String,
}

impl Default for BehaviorSettings {
    fn default() -> Self {
        Self {
            auto_tag_after_rip: true,
            eject_when_done: true,
            notify: true,
            sound: true,
            sound_theme: "complete".into(),
            sound_file: String::new(),
            shutdown_when_done: false,
            on_error: "continue".into(),
            unknown_disc: "pause".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AdvancedSettings {
    /// cyanrip 可执行文件（PATH 或绝对路径）
    pub bin: String,
    /// 额外透传参数（空格分隔）
    pub extra_args: String,
}

impl Default for AdvancedSettings {
    fn default() -> Self {
        Self {
            bin: "cyanrip".into(),
            extra_args: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiSettings {
    pub language: String,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            language: "zh".into(),
        }
    }
}

impl Settings {
    pub fn default_path() -> PathBuf {
        directories::ProjectDirs::from("", "", "cyanrip-frontend")
            .map(|d| d.config_dir().join("config.toml"))
            .unwrap_or_else(|| PathBuf::from("config.toml"))
    }

    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => {
                toml::from_str(&s).with_context(|| format!("配置文件解析失败：{}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("读取配置失败：{}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let s = toml::to_string_pretty(self).context("配置序列化失败")?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, s).context("写临时配置文件失败")?;
        std::fs::rename(&tmp, path).context("替换配置文件失败")?;
        Ok(())
    }

    /// 取该光驱型号记忆的偏移
    pub fn offset_for(&self, model: &str) -> Option<i32> {
        self.device.offsets.get(model).copied()
    }

    pub fn remember_offset(&mut self, model: &str, offset: i32) {
        self.device.offsets.insert(model.trim().to_string(), offset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_plan() {
        let s = Settings::default();
        assert_eq!(s.rip.paranoia, 3);
        assert_eq!(s.rip.verify_passes, 2);
        assert_eq!(s.rip.formats, vec!["flac".to_string()]);
        assert!(s.behavior.sound);
        assert_eq!(s.behavior.sound_theme, "complete");
        assert!(!s.behavior.shutdown_when_done);
        assert_eq!(s.ui.language, "zh");
    }

    #[test]
    fn partial_toml_uses_defaults() {
        let s: Settings = toml::from_str(
            r#"
            [rip]
            paranoia = 1
            "#,
        )
        .unwrap();
        assert_eq!(s.rip.paranoia, 1);
        assert_eq!(s.rip.retries, 10, "缺字段应回退默认值");
        assert!(s.output.write_cue);
    }

    #[test]
    fn offset_is_remembered_per_model() {
        let mut s = Settings::default();
        s.remember_offset("PIONEER DVD-RW DVR-106D", 48);
        assert_eq!(s.offset_for("PIONEER DVD-RW DVR-106D"), Some(48));
        assert_eq!(s.offset_for("other"), None);
    }

    #[test]
    fn roundtrip_save_load() {
        let dir = std::env::temp_dir().join(format!("cyanrip-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        let mut s = Settings::default();
        s.remember_offset("PIONEER DVD-RW DVR-106D", 48);
        s.save(&p).unwrap();
        let back = Settings::load(&p).unwrap();
        assert_eq!(s, back);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_file_returns_default() {
        let s = Settings::load(Path::new("/nonexistent/cyanrip-frontend/config.toml")).unwrap();
        assert_eq!(s.rip.paranoia, 3);
    }
}
