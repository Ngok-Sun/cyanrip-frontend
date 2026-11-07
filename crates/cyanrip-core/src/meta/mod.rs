//! 元数据源：MusicBrainz / AcoustID / Discogs，以及统一的发行版结构。

pub mod acoustid;
pub mod cue;
pub mod discid;
pub mod discogs;
pub mod google;
pub mod musicbrainz;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use discid::disc_id;

pub const UA: &str = "cyanrip-frontend/0.1 ( https://github.com/cyanrip-frontend )";

/// 元数据来源
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetaSource {
    MusicBrainz,
    AcoustId,
    Discogs,
    /// 手工导入的 CUE 表
    Cue,
}

impl MetaSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            MetaSource::MusicBrainz => "MusicBrainz",
            MetaSource::AcoustId => "AcoustID",
            MetaSource::Discogs => "Discogs",
            MetaSource::Cue => "CUE",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaTrack {
    pub number: u32,
    pub title: String,
    #[serde(default)]
    pub length: Option<Duration>,
}

fn one() -> u32 {
    1
}

/// 一张候选发行版（来自任一数据源）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaRelease {
    pub source: MetaSource,
    /// MusicBrainz release id 或 Discogs release id
    pub id: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub catno: Option<String>,
    #[serde(default)]
    pub barcode: Option<String>,
    pub tracks: Vec<MetaTrack>,
    /// 本候选对应的碟号（1 起）；0 表示未知
    #[serde(default)]
    pub disc_number: u32,
    /// 套装总碟数（单碟为 1）
    #[serde(default = "one")]
    pub disc_total: u32,
    #[serde(default)]
    pub cover_url: Option<String>,
    /// 与碟片 TOC 的吻合度（0–100），用于排序
    #[serde(default)]
    pub score: u8,
}

impl MetaRelease {
    /// 与碟片的时长比对，返回 0–100 的吻合度。
    pub fn score_against(&self, durations: &[Duration]) -> u8 {
        if self.tracks.is_empty() || self.tracks.len() != durations.len() {
            return 0;
        }
        let mut hits = 0usize;
        let mut known = 0usize;
        for (t, d) in self.tracks.iter().zip(durations) {
            if let Some(len) = t.length {
                known += 1;
                let diff = len.as_secs_f64() - d.as_secs_f64();
                if diff.abs() <= 4.0 {
                    hits += 1;
                }
            }
        }
        if known == 0 {
            // Discogs 很多发行版不填时长，只能按轨数确认；
            // 套装里同轨数的两张碟会并列，靠碟序再分一次（见 sort 前的 tie-break）
            return 60;
        }
        (hits * 100 / durations.len().max(1)) as u8
    }
}

/// 从碟片 TOC 得到每轨时长，供打分使用。
pub fn track_durations(info: &crate::model::DiscInfo) -> Vec<Duration> {
    info.audio_tracks().map(|t| t.duration()).collect()
}

/// 带显式 token 的解析入口（界面把设置里的 token 传进来）。
pub fn resolve_with(
    info: &crate::model::DiscInfo,
    first_audio_file: Option<&Path>,
    discogs_token: Option<&str>,
) -> Vec<MetaRelease> {
    discogs::set_token(discogs_token.map(|s| s.to_string()));
    resolve(info, first_audio_file)
}

/// 读取 Discogs token：环境变量优先，其次 `~/.config/discogs/token`。
pub fn discogs_token() -> Option<String> {
    if let Ok(t) = std::env::var("CYANRIP_DISCOGS_TOKEN") {
        if !t.trim().is_empty() {
            return Some(t.trim().to_string());
        }
    }
    let path = directories::BaseDirs::new().map(|b| b.home_dir().join(".config/discogs/token"))?;
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Google 图片搜索兜底（需 key + cx，缺一不可）
pub fn google_cover(
    release: &MetaRelease,
    api_key: Option<&str>,
    cx: Option<&str>,
) -> Option<String> {
    let key = api_key?.trim();
    let cx = cx?.trim();
    if key.is_empty() || cx.is_empty() {
        return None;
    }
    google::cover_for(release, key, cx)
}

/// 为候选找封面：先用它自己的 URL，失败再查 Discogs（条码/目录号/艺人+专辑）。
pub fn cover_fallback(release: &MetaRelease, token: Option<&str>) -> Option<String> {
    discogs::find_cover(
        release.barcode.as_deref(),
        release.catno.as_deref(),
        &format!("{} {}", release.artist, release.title),
        token,
    )
}

/// 下载封面到临时文件，返回路径。
pub fn download_cover(url: &str) -> anyhow::Result<PathBuf> {
    let resp = ureq::get(url).set("User-Agent", UA).call()?;
    let mut bytes = Vec::new();
    resp.into_reader().read_to_end(&mut bytes)?;
    let path = std::env::temp_dir().join(format!("cyanrip-cover-{}.jpg", std::process::id()));
    std::fs::write(&path, &bytes)?;
    Ok(path)
}

/// 解析主流程：MusicBrainz（discid）→ 结果为空时用 AcoustID + Discogs 兜底。
pub fn resolve(info: &crate::model::DiscInfo, first_audio_file: Option<&Path>) -> Vec<MetaRelease> {
    let durations = track_durations(info);
    let mut out = Vec::new();

    if let Some(id) = disc_id(info) {
        if let Ok(mut rels) = musicbrainz::lookup_discid(&id, durations.len()) {
            for r in &mut rels {
                r.score = r.score_against(&durations);
            }
            out.append(&mut rels);
        }
    }

    // 碟片 MCN 往往就是条码：直接查 Discogs
    if out.is_empty() {
        if let Some(mcn) = info.mcn.as_deref() {
            if let Ok(mut rels) = discogs::by_barcode(mcn, durations.len()) {
                for r in &mut rels {
                    r.score = r.score_against(&durations);
                }
                out.append(&mut rels);
            }
        }
    }

    // AcoustID 兜底：指纹识别第一轨，再去 Discogs 搜
    if out.is_empty() {
        if let Some(file) = first_audio_file {
            if let Ok(m) = acoustid::identify(file) {
                if let Ok(mut rels) =
                    discogs::search(&format!("{} {}", m.artist, m.title), durations.len())
                {
                    for r in &mut rels {
                        r.score = r.score_against(&durations);
                    }
                    out.append(&mut rels);
                }
            }
        }
    }

    out.sort_by_key(|r| std::cmp::Reverse(r.score));
    out
}
