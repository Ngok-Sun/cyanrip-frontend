//! Google 自定义搜索（图片）——封面兜底。
//!
//! 需要两项配置：API Key 与搜索引擎 ID（CX）。到
//! <https://console.cloud.google.com/apis/credentials> 建 key，
//! 再到 <https://programmablesearchengine.google.com/> 建一个「搜索整个网络 + 图片」的引擎拿 CX。
//! 没配置时整步跳过，不会报错。

use super::MetaRelease;
use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Deserialize)]
struct Resp {
    #[serde(default)]
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    #[serde(default)]
    link: Option<String>,
    #[serde(default, rename = "mime")]
    mime: Option<String>,
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 搜索一张图片，返回 URL。
pub fn search_image(query: &str, api_key: &str, cx: &str) -> Result<Option<String>> {
    let url = format!(
        "https://www.googleapis.com/customsearch/v1?key={}&cx={}&q={}&searchType=image&num=5&imgSize=large&safe=off",
        urlencode(api_key),
        urlencode(cx),
        urlencode(query)
    );
    let resp: Resp = ureq::get(&url)
        .set("User-Agent", super::UA)
        .call()
        .with_context(|| "Google 图片搜索失败".to_string())?
        .into_json()
        .context("Google 返回解析失败")?;
    Ok(resp
        .items
        .into_iter()
        .filter(|i| {
            i.mime
                .as_deref()
                .map(|m| m.starts_with("image/"))
                .unwrap_or(true)
        })
        .find_map(|i| i.link)
        .filter(|l| l.starts_with("http")))
}

/// 为候选构造查询并搜索
pub fn cover_for(release: &MetaRelease, api_key: &str, cx: &str) -> Option<String> {
    let query = if release.artist.trim().is_empty() {
        format!("{} CD cover", release.title)
    } else {
        format!("{} {} CD cover", release.artist, release.title)
    };
    match search_image(&query, api_key, cx) {
        Ok(url) => url,
        Err(e) => {
            if std::env::var("CYANRIP_GUI_DEBUG").as_deref() == Ok("1") {
                eprintln!("GOOGLE err={e:#}");
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_first_image_link() {
        let json = r#"{
          "items": [
            {"link": "https://example.com/a.jpg", "mime": "image/jpeg"},
            {"link": "https://example.com/b.png", "mime": "image/png"}
          ]
        }"#;
        let r: Resp = serde_json::from_str(json).unwrap();
        assert_eq!(
            r.items.into_iter().find_map(|i| i.link).as_deref(),
            Some("https://example.com/a.jpg")
        );
    }

    #[test]
    fn encodes_query() {
        assert_eq!(urlencode("a b久"), "a+b%E4%B9%85");
    }
}
