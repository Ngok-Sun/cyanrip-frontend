//! 输出路径模板：`{artist}/{album}`、`{track:02} {title}` 这类占位符的渲染。
//!
//! 只支持一小撮字段，未知占位符原样保留（方便用户发现拼写错误）。
//! 字段值在代入前做文件名清理，避免 `:` `/` 之类把路径拆坏。

use crate::tag::sanitize;

#[derive(Debug, Clone)]
pub struct Ctx<'a> {
    pub artist: &'a str,
    pub album: &'a str,
    pub year: &'a str,
    pub title: &'a str,
    pub track: u32,
    pub track_total: u32,
    pub disc: u32,
    pub disc_total: u32,
}

/// 渲染模板。`{track:02}` 这种带宽度说明的写法会补零。
pub fn render(tpl: &str, ctx: &Ctx) -> String {
    let mut out = String::with_capacity(tpl.len() + 16);
    let bytes = tpl.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(end) = tpl[i..].find('}') {
                let token = &tpl[i + 1..i + end];
                if let Some(v) = value(token, ctx) {
                    out.push_str(&v);
                    i += end + 1;
                    continue;
                }
            }
        }
        // 普通字符（按 UTF-8 边界推进）
        let ch = tpl[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn value(token: &str, ctx: &Ctx) -> Option<String> {
    let (name, width) = match token.split_once(':') {
        Some((n, w)) => (n.trim(), w.trim().parse::<usize>().ok()),
        None => (token.trim(), None),
    };
    let s = match name {
        "artist" => sanitize(ctx.artist),
        "album" => sanitize(ctx.album),
        "year" => sanitize(ctx.year),
        "title" => sanitize(ctx.title),
        "track" => num(ctx.track, width),
        "track_total" => num(ctx.track_total, width),
        "disc" => num(ctx.disc, width),
        "disc_total" => num(ctx.disc_total, width),
        _ => return None,
    };
    Some(s)
}

fn num(v: u32, width: Option<usize>) -> String {
    match width {
        Some(w) => format!("{v:0w$}"),
        None => v.to_string(),
    }
}

/// 套装时给文件名补 `CDn-` 前缀（模板里已经写了 `{disc}` 就不重复加）
pub fn disc_prefix(tpl: &str, disc: u32, disc_total: u32) -> String {
    if disc_total > 1 && !tpl.contains("{disc}") {
        format!("CD{disc}-")
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>() -> Ctx<'a> {
        Ctx {
            artist: "测试艺人",
            album: "Test Album",
            year: "2001",
            title: "Main Theme",
            track: 3,
            track_total: 13,
            disc: 2,
            disc_total: 2,
        }
    }

    #[test]
    fn renders_dir_and_file() {
        let c = ctx();
        assert_eq!(render("{artist}/{album}", &c), "测试艺人/Test Album");
        assert_eq!(render("{track:02} {title}", &c), "03 Main Theme");
        assert_eq!(render("{year} - {album}", &c), "2001 - Test Album");
        assert_eq!(render("CD{disc}/{track:03}", &c), "CD2/003");
    }

    #[test]
    fn sanitizes_field_values() {
        let c = Ctx {
            title: "AC/DC: Live?",
            ..ctx()
        };
        assert_eq!(render("{title}", &c), "AC_DC_ Live_");
    }

    #[test]
    fn keeps_unknown_tokens() {
        let c = ctx();
        assert_eq!(render("{bogus}/{album}", &c), "{bogus}/Test Album");
    }

    #[test]
    fn disc_prefix_only_for_sets() {
        assert_eq!(disc_prefix("{track:02} {title}", 2, 2), "CD2-");
        assert_eq!(disc_prefix("{track:02} {title}", 1, 1), "");
        assert_eq!(disc_prefix("CD{disc}-{track:02}", 2, 2), "");
    }
}
