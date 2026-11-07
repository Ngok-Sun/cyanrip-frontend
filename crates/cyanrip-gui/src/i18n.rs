//! 极薄的 i18n 层。
//!
//! 所有用户可见字符串都以中文作为 msgid，经 gettext 查表；没有安装 `.mo`
//! 时 gettext 原样返回 msgid，所以中文界面无需任何构建产物即可运行。
//! 以后新增语言只需放 `po/<lang>.po` 并编译成 `.mo`，不用改代码。

use gettextrs::LocaleCategory;

pub const DOMAIN: &str = "cyanrip-frontend";

pub fn init() {
    // 允许从源码树（cargo run）与系统安装目录两处加载翻译
    let _ = gettextrs::bindtextdomain(DOMAIN, concat!(env!("CARGO_MANIFEST_DIR"), "/../../po"));
    let _ = gettextrs::bindtextdomain(DOMAIN, "/usr/share/locale");
    // 用户级安装（不污染系统目录）
    if let Ok(home) = std::env::var("HOME") {
        let _ = gettextrs::bindtextdomain(DOMAIN, format!("{home}/.local/share/locale"));
    }
    let _ = gettextrs::textdomain(DOMAIN);
    let _ = gettextrs::setlocale(LocaleCategory::LcAll, "");
}

/// 取翻译；参数是中文原文。
pub fn tr(msgid: &str) -> String {
    gettextrs::gettext(msgid)
}
