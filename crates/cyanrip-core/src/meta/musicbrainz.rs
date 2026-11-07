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
    position: Option<u32>,
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
///
/// `track_count` 是本地这张碟的音轨数：套装里每张碟各有一条 medium，
/// 必须挑出轨数相符的那条，否则 CD2 会拿到 CD1 的曲名。
pub fn lookup_discid(discid: &str, track_count: usize) -> Result<Vec<MetaRelease>> {
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

    Ok(to_releases(resp, track_count))
}

/// 把 API 响应转成候选：套装按 medium 拆分，只保留轨数相符的那张。
fn to_releases(resp: Resp, track_count: usize) -> Vec<MetaRelease> {
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
        let total = r.media.len().max(1) as u32;
        // 找出轨数相符的 medium；没有就放弃这个发行版（通常说明压片不同）
        let mut matched: Vec<(u32, &Medium)> = Vec::new();
        for (idx, m) in r.media.iter().enumerate() {
            if track_count == 0 || m.tracks.len() == track_count {
                matched.push((m.position.unwrap_or(idx as u32 + 1), m));
            }
        }
        for (disc_no, medium) in matched {
            let tracks: Vec<MetaTrack> = medium
                .tracks
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let number = t.number.parse().unwrap_or(i as u32 + 1);
                    let title = if t.title.trim().is_empty() {
                        format!("Track {number:02}")
                    } else {
                        t.title.clone()
                    };
                    MetaTrack {
                        number,
                        title,
                        length: t.length.map(Duration::from_millis),
                    }
                })
                .collect();
            out.push(MetaRelease {
                source: MetaSource::MusicBrainz,
                id: r.id.clone(),
                title: r.title.clone(),
                artist: artist.clone(),
                date: r.date.clone(),
                country: r.country.clone(),
                label: label.clone(),
                catno: catno.clone(),
                barcode: r.barcode.clone(),
                tracks,
                disc_number: disc_no,
                disc_total: total,
                cover_url: Some(format!(
                    "https://coverartarchive.org/release/{}/front-500",
                    r.id
                )),
                score: 0,
            });
        }
    }
    out
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

    const TWO_DISC: &str = r#"{
      "releases": [{
        "id": "rel-1", "title": "Works for Violin and Orchestra", "date": "1992",
        "artist-credit": [{"name": "Paganini"}],
        "media": [
          {"position": 1, "tracks": [
            {"number": "1", "title": "Variazioni", "length": 725000},
            {"number": "2", "title": "Larghetto", "length": 237000}
          ]},
          {"position": 2, "tracks": [
            {"number": "1", "title": "Sonata Varsavia", "length": 925000}
          ]}
        ]
      }]
    }"#;

    #[test]
    fn picks_medium_matching_track_count() {
        let resp: Resp = serde_json::from_str(TWO_DISC).unwrap();
        // 第二张碟：1 轨
        let rels = to_releases(resp, 1);
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].disc_number, 2);
        assert_eq!(rels[0].disc_total, 2);
        assert_eq!(rels[0].tracks[0].title, "Sonata Varsavia");

        // 第一张碟：2 轨
        let resp: Resp = serde_json::from_str(TWO_DISC).unwrap();
        let rels = to_releases(resp, 2);
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].disc_number, 1);
        assert_eq!(rels[0].tracks[0].title, "Variazioni");

        // 轨数对不上：不要瞎给候选
        let resp: Resp = serde_json::from_str(TWO_DISC).unwrap();
        assert!(to_releases(resp, 7).is_empty());
    }
}
