//! 极薄的 i18n 层。
//!
//! 所有用户可见字符串都以中文作为 msgid，经 gettext 查表；没有安装 `.mo`
//! 时 gettext 原样返回 msgid，所以中文界面无需任何构建产物即可运行。
//! 以后新增语言只需放 `po/<lang>.po` 并编译成 `.mo`，不用改代码。

use gettextrs::LocaleCategory;

pub const DOMAIN: &str = "cyanrip-frontend";

/// 按设置里的语言初始化；`auto` 表示跟随系统。
///
/// 注意：`LANGUAGE` 在系统没生成对应 locale 时不生效（本机只有 en_US），
/// 所以这里直接切 LC_MESSAGES：英文用 en_US（gettext 会回落到 `po/en`），
/// 中文用 `C`（没有 .mo，直接返回中文 msgid）。
pub fn init_with(lang: &str) {
    init();
    apply_language(lang);
}

/// GTK 启动时会自己 `setlocale(LC_ALL, "")`，把语言覆盖掉，
/// 所以界面构建前必须再调一次。
pub fn apply_language(lang: &str) {
    match lang {
        "en" => {
            std::env::set_var("LANGUAGE", "en");
            let _ = gettextrs::setlocale(LocaleCategory::LcMessages, "en_US.UTF-8");
        }
        "zh" => {
            std::env::remove_var("LANGUAGE");
            let _ = gettextrs::setlocale(LocaleCategory::LcMessages, "C");
        }
        _ => {}
    }
}

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
