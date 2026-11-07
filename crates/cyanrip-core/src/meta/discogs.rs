//! Discogs：按条码搜索、读取发行版与曲目表。

use super::{discogs_token, MetaRelease, MetaSource, MetaTrack, UA};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize)]
struct SearchResp {
    #[serde(default)]
    results: Vec<SearchItem>,
}

#[derive(Deserialize)]
struct SearchItem {
    id: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    year: Option<String>,
    #[serde(default)]
    country: Option<String>,
}

#[derive(Deserialize)]
struct Release {
    id: u64,
    title: String,
    #[serde(default)]
    year: Option<u32>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    artists: Vec<Artist>,
    #[serde(default)]
    labels: Vec<Label>,
    #[serde(default)]
    identifiers: Vec<Identifier>,
    #[serde(default)]
    tracklist: Vec<DiscogsTrack>,
    #[serde(default)]
    images: Vec<Image>,
}

#[derive(Deserialize)]
struct Artist {
    name: String,
}

#[derive(Deserialize)]
struct Label {
    name: String,
    #[serde(default)]
    catno: Option<String>,
}

#[derive(Deserialize)]
struct Identifier {
    #[serde(rename = "type")]
    kind: String,
    value: String,
}

#[derive(Deserialize)]
struct DiscogsTrack {
    #[serde(default)]
    position: String,
    title: String,
    #[serde(default)]
    duration: String,
    #[serde(default, rename = "type_")]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct Image {
    uri: String,
    #[serde(rename = "type")]
    kind: String,
}

/// 界面里设置的 token（由 `meta::resolve_with` 注入），优先于环境变量/旧文件
static TOKEN_OVERRIDE: std::sync::OnceLock<std::sync::Mutex<Option<String>>> =
    std::sync::OnceLock::new();

pub fn set_token(token: Option<String>) {
    let slot = TOKEN_OVERRIDE.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut guard) = slot.lock() {
        *guard = token;
    }
}

fn effective_token() -> Option<String> {
    TOKEN_OVERRIDE
        .get()
        .and_then(|m| m.lock().ok().and_then(|g| g.clone()))
        .filter(|t| !t.trim().is_empty())
        .or_else(discogs_token)
}

fn get_json<T: for<'de> Deserialize<'de>>(url: &str) -> Result<T> {
    let mut req = ureq::get(url).set("User-Agent", UA);
    if let Some(token) = effective_token() {
        req = req.set("Authorization", &format!("Discogs token={token}"));
    }
    req.call()
        .with_context(|| format!("Discogs 请求失败：{url}"))?
        .into_json()
        .context("Discogs 返回解析失败")
}

/// 按关键字（艺人 / 专辑 / 条码）搜索。
pub fn search(q: &str, track_count: usize) -> Result<Vec<MetaRelease>> {
    let _ = track_count;
    let url = format!(
        "https://api.discogs.com/database/search?q={}&type=release&per_page=15",
        urlencode(q)
    );
    let resp: SearchResp = get_json(&url)?;
    Ok(resp
        .results
        .into_iter()
        .map(|item| MetaRelease {
            source: MetaSource::Discogs,
            id: item.id.to_string(),
            title: item.title.clone(),
            artist: item
                .title
                .split_once(" - ")
                .map(|(a, _)| a.to_string())
                .unwrap_or_default(),
            date: item.year,
            country: item.country,
            label: None,
            catno: None,
            barcode: None,
            tracks: Vec::new(),
            disc_number: 0,
            disc_total: 1,
            cover_url: None,
            score: 0,
        })
        .collect())
}

/// 按条码搜索并取回每个候选的曲目表（最多 5 个候选，避免打爆限速）。
pub fn by_barcode(barcode: &str, track_count: usize) -> Result<Vec<MetaRelease>> {
    let url = format!(
        "https://api.discogs.com/database/search?barcode={}&type=release&per_page=10",
        urlencode(barcode)
    );
    let resp: SearchResp = get_json(&url)?;
    let mut out = Vec::new();
    for item in resp.results.into_iter().take(5) {
        if let Ok(mut rels) = get_release(item.id, track_count) {
            out.append(&mut rels);
        }
    }
    Ok(out)
}

/// Discogs 的 track position：套装写成 "1-03"，单碟写成 "03"；黑胶是 "A1"。
/// 返回 (碟号, 碟内轨号)。
fn split_position(pos: &str, fallback: u32) -> (u32, u32) {
    if let Some((disc, track)) = pos.split_once('-') {
        (
            disc.trim().parse().unwrap_or(1),
            track.trim().parse().unwrap_or(fallback),
        )
    } else {
        (1, pos.trim().parse().unwrap_or(fallback))
    }
}

/// 读取单个发行版（含曲目、条码、封面）。
///
/// 返回**每张碟一个**候选：`track_count` 是本机这张碟的音轨数，用来挑出对应的碟。
pub fn get_release(id: u64, track_count: usize) -> Result<Vec<MetaRelease>> {
    let url = format!("https://api.discogs.com/releases/{id}");
    let r: Release = get_json(&url)?;

    let mut groups: std::collections::BTreeMap<u32, Vec<MetaTrack>> =
        std::collections::BTreeMap::new();
    for (i, t) in r.tracklist.iter().enumerate() {
        if t.kind.as_deref() == Some("heading") {
            continue;
        }
        let (disc, number) = split_position(&t.position, i as u32 + 1);
        groups.entry(disc).or_default().push(MetaTrack {
            number,
            title: t.title.clone(),
            length: parse_duration(&t.duration),
        });
    }
    let disc_total = groups.keys().copied().max().unwrap_or(1).max(1);

    let barcode = r
        .identifiers
        .iter()
        .find(|i| i.kind.eq_ignore_ascii_case("barcode"))
        .map(|i| i.value.replace(' ', ""))
        .or_else(|| {
            r.identifiers
                .iter()
                .find(|i| i.kind.eq_ignore_ascii_case("isrc"))
                .map(|i| i.value.clone())
        });

    let mut out = Vec::new();
    for (disc_no, tracks) in groups {
        if track_count > 0 && tracks.len() != track_count {
            continue;
        }
        out.push(MetaRelease {
            source: MetaSource::Discogs,
            id: r.id.to_string(),
            title: r.title.clone(),
            artist: r
                .artists
                .first()
                .map(|a| a.name.clone())
                .unwrap_or_default(),
            date: r.year.map(|y| y.to_string()),
            country: r.country.clone(),
            label: r.labels.first().map(|l| l.name.clone()),
            catno: r.labels.first().and_then(|l| l.catno.clone()),
            barcode: barcode.clone(),
            tracks,
            disc_number: disc_no,
            disc_total,
            cover_url: r
                .images
                .iter()
                .find(|i| i.kind == "primary")
                .map(|i| i.uri.clone()),
            score: 0,
        });
    }
    Ok(out)
}

fn parse_duration(s: &str) -> Option<Duration> {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        2 => Some(Duration::from_secs(
            parts[0].parse::<u64>().ok()? * 60 + parts[1].parse::<u64>().ok()?,
        )),
        3 => Some(Duration::from_secs(
            parts[0].parse::<u64>().ok()? * 3600
                + parts[1].parse::<u64>().ok()? * 60
                + parts[2].parse::<u64>().ok()?,
        )),
        _ => None,
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration("3:18"), Some(Duration::from_secs(198)));
        assert_eq!(parse_duration("1:03:18"), Some(Duration::from_secs(3798)));
        assert_eq!(parse_duration(""), None);
    }

    #[test]
    fn urlencodes_spaces_and_unicode() {
        assert_eq!(urlencode("a b"), "a%20b");
        assert_eq!(urlencode("久石譲"), "%E4%B9%85%E7%9F%B3%E8%AD%B2");
    }

    #[test]
    fn splits_disc_position() {
        assert_eq!(split_position("1-03", 9), (1, 3));
        assert_eq!(split_position("2-01", 9), (2, 1));
        assert_eq!(split_position("07", 9), (1, 7));
        assert_eq!(split_position("A1", 9), (1, 9));
    }
}
