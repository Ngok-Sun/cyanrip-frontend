//! 保证英文翻译不漏条目：pot 里每个 msgid 在 en.po 里都要有非空 msgstr。

use std::collections::HashMap;
use std::path::PathBuf;

fn read(path: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读取 {} 失败：{e}", p.display()))
}

fn msgids(pot: &str) -> Vec<String> {
    pot.lines()
        .filter_map(|l| l.strip_prefix("msgid \""))
        .filter_map(|l| l.strip_suffix('"'))
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn every_msgid_is_translated() {
    let pot = read("../../po/cyanrip-frontend.pot");
    let po = read("../../po/en.po");

    // msgid -> msgstr
    let mut map: HashMap<String, String> = HashMap::new();
    let mut cur: Option<String> = None;
    for line in po.lines() {
        if let Some(rest) = line.strip_prefix("msgid \"") {
            if let Some(m) = rest.strip_suffix('"') {
                cur = Some(m.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("msgstr \"") {
            if let (Some(m), Some(v)) = (cur.take(), rest.strip_suffix('"')) {
                map.insert(m, v.to_string());
            }
        }
    }

    let missing: Vec<String> = msgids(&pot)
        .into_iter()
        .filter(|m| map.get(m).map(|v| v.is_empty()).unwrap_or(true))
        .collect();
    assert!(missing.is_empty(), "未翻译的 msgid：{missing:#?}");
}
