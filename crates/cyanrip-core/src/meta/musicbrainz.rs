//! MusicBrainz：按 Disc ID 查发行版。

use super::{MetaRelease, MetaSource, MetaTrack, UA};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize)]
struct Resp {
    #[serde(default)]
    releases: Vec<Release>,
}

#[derive(Deserialize)]
struct Release {
    id: String,
    title: String,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    barcode: Option<String>,
    #[serde(default, rename = "artist-credit")]
    artist_credit: Vec<ArtistCredit>,
    #[serde(default, rename = "label-info")]
    label_info: Vec<LabelInfo>,
    #[serde(default)]
    media: Vec<Medium>,
}

#[derive(Deserialize)]
struct ArtistCredit {
    name: String,
}

#[derive(Deserialize)]
struct LabelInfo {
    #[serde(default)]
    label: Option<Label>,
    #[serde(default, rename = "catalog-number")]
    catno: Option<String>,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

#[derive(Deserialize)]
struct Medium {
    #[serde(default)]
    tracks: Vec<MbTrack>,
}

#[derive(Deserialize)]
struct MbTrack {
    #[serde(default)]
    number: String,
    title: String,
    #[serde(default)]
    length: Option<u64>,
}

/// 按 Disc ID 查询；返回所有匹配的发行版（通常只有 1–5 个）。
pub fn lookup_discid(discid: &str) -> Result<Vec<MetaRelease>> {
    // inc 用 `+` 连接，必须手拼 URL，ureq 的 query() 会把它编码成 %2B
    let url = format!(
        "https://musicbrainz.org/ws/2/discid/{discid}?inc=artists+labels+recordings&fmt=json"
    );
    let resp: Resp = ureq::get(&url)
        .set("User-Agent", UA)
        .call()
        .with_context(|| format!("MusicBrainz 查询失败：{discid}"))?
        .into_json()
        .context("MusicBrainz 返回解析失败")?;

    let mut out = Vec::new();
    for r in resp.releases {
        let artist = r
            .artist_credit
            .first()
            .map(|a| a.name.clone())
            .unwrap_or_default();
        let (label, catno) = r
            .label_info
            .first()
            .map(|li| (li.label.as_ref().map(|l| l.name.clone()), li.catno.clone()))
            .unwrap_or((None, None));
        let tracks: Vec<MetaTrack> = r
            .media
            .iter()
            .flat_map(|m| m.tracks.iter())
            .enumerate()
            .map(|(i, t)| MetaTrack {
                number: t.number.parse().unwrap_or(i as u32 + 1),
                title: t.title.clone(),
                length: t.length.map(Duration::from_millis),
            })
            .collect();
        out.push(MetaRelease {
            source: MetaSource::MusicBrainz,
            id: r.id.clone(),
            title: r.title,
            artist,
            date: r.date,
            country: r.country,
            label,
            catno,
            barcode: r.barcode,
            tracks,
            cover_url: Some(format!(
                "https://coverartarchive.org/release/{}/front-500",
                r.id
            )),
            score: 0,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_release_json() {
        let json = r#"{
          "releases": [{
            "id": "abc", "title": "Quartet", "date": "2001-09-27", "country": "JP",
            "barcode": "4988005280619",
            "artist-credit": [{"name": "久石譲"}],
            "label-info": [{"label": {"name": "Polydor"}, "catalog-number": "UPCH-1105"}],
            "media": [{"tracks": [
               {"number": "1", "title": "Main Theme", "length": 198133},
               {"number": "2", "title": "Student Quartet", "length": 209400}
            ]}]
          }]
        }"#;
        let resp: Resp = serde_json::from_str(json).unwrap();
        assert_eq!(resp.releases.len(), 1);
        let r = &resp.releases[0];
        assert_eq!(r.artist_credit[0].name, "久石譲");
        assert_eq!(r.media[0].tracks.len(), 2);
        assert_eq!(r.media[0].tracks[0].length, Some(198133));
    }
}
