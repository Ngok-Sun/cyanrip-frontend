//! AcoustID：对音频文件做指纹识别（MusicBrainz 未命中时的兜底）。
//!
//! 需要 `fpcalc`（chromaprint-tools）与一个客户端 key：
//! 在 <https://acoustid.org/new-application> 申请后设置环境变量
//! `CYANRIP_ACOUSTID_KEY`。没有 key 时直接返回错误，不静默失败。

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct AcoustIdMatch {
    pub artist: String,
    pub title: String,
    pub score: f32,
}

#[derive(Deserialize)]
struct Fpcalc {
    duration: f64,
    fingerprint: String,
}

#[derive(Deserialize)]
struct Resp {
    status: String,
    #[serde(default)]
    results: Vec<ResultItem>,
}

#[derive(Deserialize)]
struct ResultItem {
    score: f64,
    #[serde(default)]
    recordings: Vec<Recording>,
}

#[derive(Deserialize)]
struct Recording {
    title: String,
    #[serde(default)]
    artists: Vec<Artist>,
}

#[derive(Deserialize)]
struct Artist {
    name: String,
}

pub fn identify(file: &Path) -> Result<AcoustIdMatch> {
    let key = std::env::var("CYANRIP_ACOUSTID_KEY")
        .context("未设置 CYANRIP_ACOUSTID_KEY，跳过 AcoustID 指纹识别")?;

    let out = Command::new("fpcalc")
        .args(["-json", "-length", "120"])
        .arg(file)
        .output()
        .context("运行 fpcalc 失败（需要 libchromaprint-tools）")?;
    if !out.status.success() {
        bail!("fpcalc 退出码 {:?}", out.status.code());
    }
    let fp: Fpcalc = serde_json::from_slice(&out.stdout).context("fpcalc 输出解析失败")?;

    let body = format!(
        "client={}&meta=recordings&duration={}&fingerprint={}",
        urlencode(&key),
        fp.duration as u64,
        urlencode(&fp.fingerprint)
    );
    let resp: Resp = ureq::post("https://api.acoustid.org/v2/lookup")
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_string(&body)
        .context("AcoustID 请求失败")?
        .into_json()
        .context("AcoustID 返回解析失败")?;
    if resp.status != "ok" {
        bail!("AcoustID 返回状态 {}", resp.status);
    }

    let best = resp
        .results
        .into_iter()
        .max_by(|a, b| a.score.total_cmp(&b.score))
        .context("AcoustID 没有匹配结果")?;
    let rec = best
        .recordings
        .first()
        .context("AcoustID 结果缺少录音信息")?;
    Ok(AcoustIdMatch {
        artist: rec
            .artists
            .first()
            .map(|a| a.name.clone())
            .unwrap_or_default(),
        title: rec.title.clone(),
        score: best.score as f32,
    })
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
