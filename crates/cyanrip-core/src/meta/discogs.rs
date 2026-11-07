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

fn get_json<T: for<'de> Deserialize<'de>>(url: &str) -> Result<T> {
    let mut req = ureq::get(url).set("User-Agent", UA);
    if let Some(token) = discogs_token() {
        req = req.set("Authorization", &format!("Discogs token={token}"));
    }
    req.call()
        .with_context(|| format!("Discogs 请求失败：{url}"))?
        .into_json()
        .context("Discogs 返回解析失败")
}

/// 按关键字（艺人 / 专辑 / 条码）搜索。
pub fn search(q: &str) -> Result<Vec<MetaRelease>> {
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
            cover_url: None,
            score: 0,
        })
        .collect())
}

/// 按条码搜索并取回每个候选的曲目表（最多 5 个候选，避免打爆限速）。
pub fn by_barcode(barcode: &str) -> Result<Vec<MetaRelease>> {
    let url = format!(
        "https://api.discogs.com/database/search?barcode={}&type=release&per_page=10",
        urlencode(barcode)
    );
    let resp: SearchResp = get_json(&url)?;
    let mut out = Vec::new();
    for item in resp.results.into_iter().take(5) {
        if let Ok(rel) = get_release(item.id) {
            out.push(rel);
        }
    }
    Ok(out)
}

/// 读取单个发行版（含曲目、条码、封面）。
pub fn get_release(id: u64) -> Result<MetaRelease> {
    let url = format!("https://api.discogs.com/releases/{id}");
    let r: Release = get_json(&url)?;

    let tracks: Vec<MetaTrack> = r
        .tracklist
        .iter()
        .filter(|t| t.kind.as_deref() != Some("heading"))
        .enumerate()
        .map(|(i, t)| MetaTrack {
            number: t.position.parse().unwrap_or(i as u32 + 1),
            title: t.title.clone(),
            length: parse_duration(&t.duration),
        })
        .collect();

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

    Ok(MetaRelease {
        source: MetaSource::Discogs,
        id: r.id.to_string(),
        title: r.title,
        artist: r
            .artists
            .first()
            .map(|a| a.name.clone())
            .unwrap_or_default(),
        date: r.year.map(|y| y.to_string()),
        country: r.country,
        label: r.labels.first().map(|l| l.name.clone()),
        catno: r.labels.first().and_then(|l| l.catno.clone()),
        barcode,
        tracks,
        cover_url: r
            .images
            .iter()
            .find(|i| i.kind == "primary")
            .map(|i| i.uri.clone()),
        score: 0,
    })
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
}
