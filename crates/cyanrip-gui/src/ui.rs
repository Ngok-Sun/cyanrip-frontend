//! 主窗口与五个页面：碟片、设置、进度、结果（元数据页在 M3 加入）。
//!
//! 采用手写 MVU：`AppState` + 控件引用；工作线程通过 `std::sync::mpsc`
//! 上报 core 的 `Event`，UI 用 100ms 定时器轮询，避免 GTK 主线程阻塞。

use crate::i18n::tr;
use crate::state::{AppState, Phase};
use adw::prelude::*;
use cyanrip_core::config::Settings;
use cyanrip_core::disc;
use cyanrip_core::event::Event;
use cyanrip_core::meta::{self, MetaRelease, MetaSource};
use cyanrip_core::model::{fmt_duration, ArState, DiscInfo, RipReport};
use cyanrip_core::rip::{self, RipOptions};
use cyanrip_core::tag;
use gtk::{glib, Orientation};
use libadwaita as adw;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;
use tokio::sync::watch;

/// 元数据工作线程 → UI 的消息
#[derive(Debug)]
pub enum MetaOutcome {
    Candidates(Vec<MetaRelease>, Option<PathBuf>),
    Applied(PathBuf),
    ApplyFailed(String),
}

/// 封面下载线程 → UI：(候选 key, 下载结果)
type CoverMsg = (String, anyhow::Result<PathBuf>);

pub struct Ui {
    pub window: adw::ApplicationWindow,
    pub toast: adw::ToastOverlay,
    pub stack: gtk::Stack,
    // 碟片页
    pub device_dropdown: gtk::DropDown,
    pub device_paths: RefCell<Vec<String>>,
    pub rescan_btn: gtk::Button,
    pub eject_btn: gtk::Button,
    pub syncing_device: RefCell<bool>,
    pub last_media: RefCell<cyanrip_core::disc::MediaStatus>,
    pub detect_btn: gtk::Button,
    pub offset_spin: gtk::SpinButton,
    pub disc_summary: gtk::Label,
    pub select_all: gtk::CheckButton,
    pub track_checks: RefCell<Vec<gtk::CheckButton>>,
    pub syncing_selection: RefCell<bool>,
    pub track_list: gtk::ListBox,
    pub start_btn: gtk::Button,
    // 进度页
    pub progress_title: gtk::Label,
    pub bar_track: gtk::ProgressBar,
    pub bar_total: gtk::ProgressBar,
    pub stats_label: gtk::Label,
    pub log_view: gtk::TextView,
    pub cancel_btn: gtk::Button,
    // 结果页
    pub result_summary: gtk::Label,
    pub result_body: gtk::Box,
    pub meta_box: gtk::Box,
    pub meta_status: gtk::Label,
    pub meta_label: gtk::Label,
    pub meta_dropdown: gtk::DropDown,
    pub import_cue_btn: gtk::Button,
    pub cover_picture: gtk::Picture,
    pub cover_placeholder: gtk::Label,
    pub cover_btn: gtk::Button,
    /// (release key, 本地文件)；key = "来源:发行版 id"
    pub cover_cache: RefCell<Option<(String, PathBuf)>>,
    /// 正在下载的候选 key，避免重复请求（Google 有配额）
    pub cover_inflight: RefCell<Option<String>>,
    /// 已经失败过的候选 key：自动流程不再重试，手动「取得封面」仍可重试
    pub cover_failed: RefCell<Option<String>>,
    pub cover_rx: RefCell<Option<mpsc::Receiver<CoverMsg>>>,
    pub meta_apply_btn: gtk::Button,
    pub meta_rx: RefCell<Option<mpsc::Receiver<MetaOutcome>>>,
    pub last_rip_dir: RefCell<Option<PathBuf>>,
    pub open_output_btn: gtk::Button,
    pub copy_log_btn: gtk::Button,
    // 设置页
    pub paranoia_spin: gtk::SpinButton,
    pub retries_spin: gtk::SpinButton,
    pub verify_spin: gtk::SpinButton,
    pub formats_combo: adw::ComboRow,
    pub root_row: adw::ActionRow,
    pub root_pick_btn: gtk::Button,
    pub bin_entry: adw::EntryRow,
    pub discogs_row: adw::PasswordEntryRow,
    pub google_key_row: adw::PasswordEntryRow,
    pub google_cx_row: adw::EntryRow,
    pub sound_switch: adw::SwitchRow,
    pub theme_combo: adw::ComboRow,
    pub theme_test_btn: gtk::Button,
    pub sound_events: RefCell<Vec<String>>,
    pub eject_switch: adw::SwitchRow,
    pub notify_switch: adw::SwitchRow,
    pub shutdown_switch: adw::SwitchRow,
    pub save_btn: gtk::Button,
    pub reset_btn: gtk::Button,
    pub settings_status: adw::ActionRow,
    pub log_lines_rendered: RefCell<usize>,
}

pub fn build(app: &adw::Application, state: Rc<RefCell<AppState>>) -> Rc<Ui> {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(tr("cyanrip 前端"))
        .default_width(1080)
        .default_height(700)
        .build();

    let header = adw::HeaderBar::new();
    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .vexpand(true)
        .build();
    let switcher = gtk::StackSwitcher::builder().stack(&stack).build();
    header.set_title_widget(Some(&switcher));

    let start_btn = gtk::Button::with_label(&tr("开始抓轨"));
    start_btn.add_css_class("suggested-action");
    start_btn.set_sensitive(false);
    header.pack_end(&start_btn);

    // ---------- 碟片页 ----------
    // 设备下拉：列出系统里所有 /dev/sr*，标签附带型号
    let (paths, labels) = scan_devices(&state.borrow().device);
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let device_dropdown = gtk::DropDown::from_strings(&label_refs);
    device_dropdown.set_hexpand(true);
    if let Some(idx) = paths.iter().position(|p| p == &state.borrow().device) {
        device_dropdown.set_selected(idx as u32);
    }
    // 一个按钮做完整刷新：重扫 /dev/sr* → 选中项读 TOC → 自动识别元数据
    let rescan_btn = gtk::Button::with_label(&tr("刷新"));
    rescan_btn.set_tooltip_text(Some(&tr("重新扫描光驱并读取当前碟片")));
    // 通用「弹出」图标（Adwaita/Yaru 都有 media-eject-symbolic）
    // 注意：用 icon_name 构造会让 GTK 打上 `image-button` 类，libadwaita 把它渲染成
    // 无边框（只在悬停时变色）。这里改成普通按钮 + 图标子控件，保持和文字按钮同款边框。
    let eject_btn = gtk::Button::new();
    eject_btn.set_child(Some(&gtk::Image::from_icon_name("media-eject-symbolic")));
    eject_btn.add_css_class("text-button");
    eject_btn.set_tooltip_text(Some(&tr("弹出光驱托盘")));
    eject_btn.update_property(&[gtk::accessible::Property::Label(&tr("弹出"))]);
    let detect_btn = gtk::Button::with_label(&tr("测定偏移"));
    let offset_spin = gtk::SpinButton::with_range(-2000.0, 2000.0, 1.0);
    offset_spin.set_value(state.borrow().offset() as f64);
    offset_spin.set_tooltip_text(Some(&tr("读偏移（samples），与 EAC 同符号")));

    let device_box = gtk::Box::new(Orientation::Horizontal, 8);
    device_box.append(&gtk::Label::new(Some(&tr("设备"))));
    device_box.append(&device_dropdown);
    device_box.append(&rescan_btn);
    device_box.append(&eject_btn);
    device_box.append(&gtk::Label::new(Some(&tr("偏移"))));
    device_box.append(&offset_spin);
    device_box.append(&detect_btn);

    let disc_summary = gtk::Label::new(Some(&tr("尚未探测碟片")));
    disc_summary.set_xalign(0.0);
    disc_summary.add_css_class("title-4");

    // 抓轨前识别：候选下拉 + 识别按钮
    let meta_label = gtk::Label::new(Some(&tr("未识别 · 放入碟片后自动识别")));
    meta_label.set_xalign(0.0);
    meta_label.add_css_class("dim-label");
    meta_label.set_wrap(true);
    let meta_dropdown = gtk::DropDown::from_strings(&["—"]);
    meta_dropdown.set_hexpand(true);
    meta_dropdown.set_visible(false);
    let import_cue_btn = gtk::Button::with_label(&tr("导入曲目信息"));
    import_cue_btn.set_tooltip_text(Some(&tr(
        "从 CUE 表读取曲目名/专辑信息（用于 CD 数据库里查不到的碟）",
    )));
    let meta_row = gtk::Box::new(Orientation::Horizontal, 8);
    meta_row.append(&meta_dropdown);
    meta_row.append(&import_cue_btn);
    let disc_meta_box = gtk::Box::new(Orientation::Vertical, 6);
    disc_meta_box.append(&meta_label);
    disc_meta_box.append(&meta_row);

    // 封面预览：识别到候选后自动抓取并显示
    let cover_picture = gtk::Picture::new();
    cover_picture.set_size_request(180, 180);
    cover_picture.set_content_fit(gtk::ContentFit::Contain);
    cover_picture.set_can_shrink(true);
    cover_picture.set_visible(false);
    let cover_placeholder = gtk::Label::new(Some(&tr("无封面")));
    cover_placeholder.set_size_request(180, 180);
    cover_placeholder.set_wrap(true);
    cover_placeholder.add_css_class("dim-label");
    let cover_btn = gtk::Button::with_label(&tr("取得封面"));
    let cover_area = gtk::Box::new(Orientation::Vertical, 6);
    cover_area.append(&cover_placeholder);
    cover_area.append(&cover_picture);
    cover_area.append(&cover_btn);
    cover_area.add_css_class("card");
    cover_area.set_size_request(196, -1);
    cover_area.set_halign(gtk::Align::Start);
    cover_area.set_valign(gtk::Align::Start);

    let info_text = gtk::Box::new(Orientation::Vertical, 6);
    info_text.set_hexpand(true);
    info_text.append(&disc_summary);
    info_text.append(&disc_meta_box);
    let info_row = gtk::Box::new(Orientation::Horizontal, 12);
    info_row.append(&cover_area);
    info_row.append(&info_text);

    let select_all = gtk::CheckButton::with_label(&tr("全选"));
    select_all.set_active(true);
    select_all.set_tooltip_text(Some(&tr("全选 / 取消全选")));
    let select_row = gtk::Box::new(Orientation::Horizontal, 6);
    select_row.append(&select_all);

    let track_list = gtk::ListBox::new();
    track_list.set_selection_mode(gtk::SelectionMode::None);
    track_list.add_css_class("boxed-list");
    let track_scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&track_list)
        .build();

    let disc_page = gtk::Box::new(Orientation::Vertical, 12);
    disc_page.set_margin_top(12);
    disc_page.set_margin_bottom(12);
    disc_page.set_margin_start(12);
    disc_page.set_margin_end(12);
    disc_page.append(&device_box);
    disc_page.append(&info_row);
    disc_page.append(&select_row);
    disc_page.append(&track_scroll);

    // ---------- 进度页 ----------
    let progress_title = gtk::Label::new(Some(&tr("等待开始")));
    progress_title.set_xalign(0.0);
    progress_title.add_css_class("title-3");

    let bar_track = gtk::ProgressBar::new();
    bar_track.set_show_text(true);
    let bar_total = gtk::ProgressBar::new();
    bar_total.set_show_text(true);

    // 与终端等价的单行进度：数字原地变化，不再把每帧进度写进日志
    let stats_label = gtk::Label::new(Some(""));
    stats_label.set_xalign(0.0);
    stats_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    stats_label.add_css_class("monospace");
    stats_label.add_css_class("dim-label");

    let log_view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .build();
    let log_scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .height_request(260)
        .child(&log_view)
        .build();
    log_scroll.add_css_class("card");

    let cancel_btn = gtk::Button::with_label(&tr("取消抓轨"));
    cancel_btn.add_css_class("destructive-action");
    cancel_btn.set_sensitive(false);

    let progress_page = gtk::Box::new(Orientation::Vertical, 10);
    progress_page.set_margin_top(12);
    progress_page.set_margin_bottom(12);
    progress_page.set_margin_start(12);
    progress_page.set_margin_end(12);
    progress_page.append(&progress_title);
    progress_page.append(&bar_track);
    progress_page.append(&bar_total);
    progress_page.append(&stats_label);
    progress_page.append(&log_scroll);
    progress_page.append(&cancel_btn);

    // ---------- 结果页 ----------
    let result_summary = gtk::Label::new(Some(&tr("尚无结果")));
    result_summary.set_xalign(0.0);
    result_summary.add_css_class("title-4");

    let result_body = gtk::Box::new(Orientation::Vertical, 14);
    result_body.set_margin_top(6);
    result_body.set_margin_bottom(6);
    result_body.set_margin_start(6);
    result_body.set_margin_end(6);
    let result_scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&result_body)
        .build();

    // 元数据区块（抓轨完成后自动识别，也可手动触发）
    let meta_status = gtk::Label::new(Some(&tr("抓轨完成后可识别元数据")));
    meta_status.set_xalign(0.0);
    meta_status.add_css_class("dim-label");
    let meta_apply_btn = gtk::Button::with_label(&tr("写入标签"));
    meta_apply_btn.add_css_class("suggested-action");
    meta_apply_btn.set_sensitive(false);
    let meta_row = gtk::Box::new(Orientation::Horizontal, 8);
    meta_row.append(&meta_apply_btn);
    let meta_box = gtk::Box::new(Orientation::Vertical, 8);
    let meta_title = gtk::Label::new(Some(&tr("元数据")));
    meta_title.set_xalign(0.0);
    meta_title.add_css_class("heading");
    meta_box.append(&meta_title);
    meta_box.append(&meta_status);
    meta_box.append(&meta_row);

    let open_output_btn = gtk::Button::with_label(&tr("打开输出目录"));
    let copy_log_btn = gtk::Button::with_label(&tr("复制日志"));
    let result_actions = gtk::Box::new(Orientation::Horizontal, 8);
    result_actions.append(&open_output_btn);
    result_actions.append(&copy_log_btn);

    let result_page = gtk::Box::new(Orientation::Vertical, 12);
    result_page.set_margin_top(12);
    result_page.set_margin_bottom(12);
    result_page.set_margin_start(12);
    result_page.set_margin_end(12);
    result_page.append(&result_summary);
    result_page.append(&result_scroll);
    result_page.append(&result_actions);

    // ---------- 设置页 ----------
    let SettingsWidgets {
        page: settings_page,
        paranoia: paranoia_spin,
        retries: retries_spin,
        verify: verify_spin,
        formats: formats_combo,
        root: root_row,
        root_pick: root_pick_btn,
        bin: bin_entry,
        discogs: discogs_row,
        google_key: google_key_row,
        google_cx: google_cx_row,
        sound: sound_switch,
        theme: theme_combo,
        theme_test: theme_test_btn,
        sound_events,
        eject: eject_switch,
        notify: notify_switch,
        shutdown: shutdown_switch,
        save_btn,
        reset_btn,
        status: settings_status,
    } = build_settings_page(&state);

    stack.add_titled(&disc_page, Some("disc"), &tr("碟片"));
    stack.add_titled(&progress_page, Some("progress"), &tr("进度"));
    stack.add_titled(&result_page, Some("result"), &tr("结果"));
    stack.add_titled(&settings_page, Some("settings"), &tr("设置"));

    // AdwApplicationWindow 不接受 set_titlebar，必须用 ToolbarView 承载 HeaderBar
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));

    let toast = adw::ToastOverlay::new();
    toast.set_child(Some(&toolbar));
    window.set_content(Some(&toast));

    let ui = Rc::new(Ui {
        window: window.clone(),
        toast,
        stack,
        device_dropdown,
        device_paths: RefCell::new(paths),
        rescan_btn,
        eject_btn,
        syncing_device: RefCell::new(false),
        last_media: RefCell::new(cyanrip_core::disc::MediaStatus::Unknown),
        detect_btn,
        offset_spin,
        disc_summary,
        select_all,
        track_checks: RefCell::new(Vec::new()),
        syncing_selection: RefCell::new(false),
        track_list,
        start_btn,
        progress_title,
        bar_track,
        bar_total,
        stats_label,
        log_view,
        cancel_btn,
        result_summary,
        result_body,
        meta_box,
        meta_status,
        meta_label,
        meta_dropdown,
        import_cue_btn,
        cover_picture,
        cover_placeholder,
        cover_btn,
        cover_cache: RefCell::new(None),
        cover_inflight: RefCell::new(None),
        cover_failed: RefCell::new(None),
        cover_rx: RefCell::new(None),
        meta_apply_btn,
        meta_rx: RefCell::new(None),
        last_rip_dir: RefCell::new(None),
        open_output_btn,
        copy_log_btn,
        paranoia_spin,
        retries_spin,
        verify_spin,
        formats_combo,
        root_row,
        root_pick_btn,
        bin_entry,
        discogs_row,
        google_key_row,
        google_cx_row,
        sound_switch,
        theme_combo,
        theme_test_btn,
        sound_events: RefCell::new(sound_events),
        eject_switch,
        notify_switch,
        shutdown_switch,
        save_btn,
        reset_btn,
        settings_status,
        log_lines_rendered: RefCell::new(0),
    });

    connect_signals(&ui, &state);
    start_event_pump(&ui, &state);
    start_media_watch(&ui, &state);
    start_meta_pump(&ui, &state);
    window.present();
    ui
}

struct SettingsWidgets {
    page: adw::PreferencesPage,
    paranoia: gtk::SpinButton,
    retries: gtk::SpinButton,
    verify: gtk::SpinButton,
    formats: adw::ComboRow,
    root: adw::ActionRow,
    root_pick: gtk::Button,
    bin: adw::EntryRow,
    discogs: adw::PasswordEntryRow,
    google_key: adw::PasswordEntryRow,
    google_cx: adw::EntryRow,
    sound: adw::SwitchRow,
    theme: adw::ComboRow,
    theme_test: gtk::Button,
    sound_events: Vec<String>,
    eject: adw::SwitchRow,
    notify: adw::SwitchRow,
    shutdown: adw::SwitchRow,
    save_btn: gtk::Button,
    reset_btn: gtk::Button,
    status: adw::ActionRow,
}

fn build_settings_page(state: &Rc<RefCell<AppState>>) -> SettingsWidgets {
    let page = adw::PreferencesPage::new();
    let s = state.borrow().settings.clone();
    let s_secrets = state.borrow().secrets.clone();

    // 设备与驱动
    let device_group = adw::PreferencesGroup::builder()
        .title(tr("设备与驱动"))
        .build();

    let paranoia_spin = gtk::SpinButton::with_range(0.0, 3.0, 1.0);
    paranoia_spin.set_value(s.rip.paranoia as f64);
    let paranoia_row = adw::ActionRow::builder()
        .title(tr("Paranoia 级别"))
        .subtitle(tr("3 = 最高（推荐）"))
        .build();
    paranoia_row.add_suffix(&paranoia_spin);

    let retries_spin = gtk::SpinButton::with_range(1.0, 100.0, 1.0);
    retries_spin.set_value(s.rip.retries as f64);
    let retries_row = adw::ActionRow::builder().title(tr("帧重试次数")).build();
    retries_row.add_suffix(&retries_spin);

    let verify_spin = gtk::SpinButton::with_range(0.0, 10.0, 1.0);
    verify_spin.set_value(s.rip.verify_passes as f64);
    let verify_row = adw::ActionRow::builder()
        .title(tr("校验次数 (-Z)"))
        .subtitle(tr("0 = 单遍；2 = 读到两次校验和一致（约 3 遍）"))
        .build();
    verify_row.add_suffix(&verify_spin);

    let formats_combo = adw::ComboRow::builder()
        .title(tr("输出格式"))
        .model(&gtk::StringList::new(&["FLAC", "FLAC + WAV", "WAV"]))
        .build();
    formats_combo.set_selected(
        match s
            .rip
            .formats
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
        {
            ["flac"] => 0,
            ["flac", "wav"] => 1,
            ["wav"] => 2,
            _ => 0,
        },
    );

    device_group.add(&paranoia_row);
    device_group.add(&retries_row);
    device_group.add(&verify_row);
    device_group.add(&formats_combo);

    // 输出
    let output_group = adw::PreferencesGroup::builder().title(tr("输出")).build();
    // 目录只能用选择器改，避免手打路径出错
    let root_row = adw::ActionRow::builder()
        .title(tr("输出根目录"))
        .subtitle(s.output.root.display().to_string())
        .build();
    root_row.set_activatable(false);
    root_row.add_css_class("property");
    let root_pick = gtk::Button::with_label(&tr("选择…"));
    root_pick.set_valign(gtk::Align::Center);
    root_row.add_suffix(&root_pick);
    output_group.add(&root_row);

    // 元数据（M3 才真正使用）
    let meta_group = adw::PreferencesGroup::builder()
        .title(tr("元数据"))
        .description(tr(
            "MusicBrainz → AcoustID → Discogs；token 单独存 secrets.toml（0600）",
        ))
        .build();
    let discogs_row = adw::PasswordEntryRow::builder()
        .title(tr("Discogs access token"))
        .text(s_secrets.discogs_token.clone())
        .build();
    meta_group.add(&discogs_row);
    let google_key_row = adw::PasswordEntryRow::builder()
        .title(tr("Google API Key（封面兜底，可选）"))
        .text(s_secrets.google_api_key.clone())
        .build();
    let google_cx_row = adw::EntryRow::builder()
        .title(tr("Google CX（搜索引擎 ID）"))
        .text(s_secrets.google_cx.clone())
        .build();
    meta_group.add(&google_key_row);
    meta_group.add(&google_cx_row);

    // 行为
    let behavior_group = adw::PreferencesGroup::builder().title(tr("行为")).build();
    let sound_switch = adw::SwitchRow::builder()
        .title(tr("抓完播放提示音"))
        .active(s.behavior.sound)
        .build();
    // 事件名来自 freedesktop 声音主题里实际存在的文件，避免写错名字没声音
    let sound_events = list_sound_events();
    let sound_labels: Vec<&str> = sound_events.iter().map(String::as_str).collect();
    let theme_index = sound_events
        .iter()
        .position(|e| e == &s.behavior.sound_theme)
        .unwrap_or(0);
    let theme_combo = adw::ComboRow::builder()
        .title(tr("提示音"))
        .model(&gtk::StringList::new(&sound_labels))
        .selected(theme_index as u32)
        .build();
    let theme_test = gtk::Button::with_label(&tr("试听"));
    theme_test.set_valign(gtk::Align::Center);
    theme_combo.add_suffix(&theme_test);
    let eject_switch = adw::SwitchRow::builder()
        .title(tr("抓完弹出托盘"))
        .active(s.behavior.eject_when_done)
        .build();
    let notify_switch = adw::SwitchRow::builder()
        .title(tr("桌面通知"))
        .active(s.behavior.notify)
        .build();
    let shutdown_switch = adw::SwitchRow::builder()
        .title(tr("抓完关机"))
        .subtitle(tr("默认关闭；开启后仍会二次确认"))
        .active(s.behavior.shutdown_when_done)
        .build();
    behavior_group.add(&sound_switch);
    behavior_group.add(&theme_combo);
    behavior_group.add(&eject_switch);
    behavior_group.add(&notify_switch);
    behavior_group.add(&shutdown_switch);

    // 高级
    let advanced_group = adw::PreferencesGroup::builder().title(tr("高级")).build();
    let bin_entry = adw::EntryRow::builder()
        .title(tr("cyanrip 可执行文件"))
        .text(s.advanced.bin.clone())
        .build();
    advanced_group.add(&bin_entry);

    // 构建时间：用来确认当前窗口跑的是不是刚装的那版
    let build_stamp = std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| glib::DateTime::from_unix_local(d.as_secs() as i64).ok())
        .and_then(|dt| dt.format("%Y-%m-%d %H:%M:%S").ok())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let build_row = adw::ActionRow::builder()
        .title(tr("程序构建时间"))
        .subtitle(&build_stamp)
        .build();
    build_row.set_activatable(false);
    advanced_group.add(&build_row);

    // 应用：显式保存（用户要求有保存按钮），未保存时状态行会提示
    let apply_group = adw::PreferencesGroup::builder().title(tr("应用")).build();
    let save_btn = gtk::Button::with_label(&tr("保存"));
    save_btn.add_css_class("suggested-action");
    save_btn.set_valign(gtk::Align::Center);
    save_btn.set_sensitive(false);

    let status = adw::ActionRow::builder()
        .title(tr("设置状态"))
        .subtitle(tr("所有更改已保存"))
        .build();
    status.add_suffix(&save_btn);

    let reset_btn = gtk::Button::with_label(&tr("恢复默认"));
    reset_btn.set_valign(gtk::Align::Center);
    let reset_row = adw::ActionRow::builder()
        .title(tr("恢复默认设置"))
        .subtitle(tr("不会立即写入，仍需点击保存"))
        .build();
    reset_row.add_suffix(&reset_btn);

    apply_group.add(&status);
    apply_group.add(&reset_row);

    page.add(&device_group);
    page.add(&output_group);
    page.add(&meta_group);
    page.add(&behavior_group);
    page.add(&advanced_group);
    page.add(&apply_group);

    SettingsWidgets {
        page,
        paranoia: paranoia_spin,
        retries: retries_spin,
        verify: verify_spin,
        formats: formats_combo,
        root: root_row,
        root_pick,
        bin: bin_entry,
        discogs: discogs_row,
        google_key: google_key_row,
        google_cx: google_cx_row,
        sound: sound_switch,
        theme: theme_combo,
        theme_test,
        sound_events,
        eject: eject_switch,
        notify: notify_switch,
        shutdown: shutdown_switch,
        save_btn,
        reset_btn,
        status,
    }
}

fn connect_signals(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    {
        let ui2 = ui.clone();
        let st = state.clone();
        ui.detect_btn
            .connect_clicked(move |_| detect_offset(&ui2, &st));
    }
    // 设备下拉切换 → 自动探测
    {
        let ui2 = ui.clone();
        let st = state.clone();
        ui.device_dropdown.connect_selected_notify(move |_| {
            if *ui2.syncing_device.borrow() {
                return;
            }
            probe_disc(&ui2, &st);
        });
    }
    // 弹出托盘
    {
        let ui2 = ui.clone();
        let st = state.clone();
        ui.eject_btn.connect_clicked(move |_| {
            let device = selected_device(&ui2);
            match disc::eject(Path::new(&device)) {
                Ok(()) => {
                    {
                        let mut s = st.borrow_mut();
                        s.disc = None;
                        s.selected.clear();
                    }
                    refresh_disc(&ui2, &st);
                    ui2.start_btn.set_sensitive(false);
                    ui2.toast.add_toast(adw::Toast::new(&tr("托盘已弹出")));
                }
                Err(e) => ui2
                    .toast
                    .add_toast(adw::Toast::new(&format!("{}: {e}", tr("弹出失败")))),
            }
        });
    }
    // 重新扫描设备
    {
        let ui2 = ui.clone();
        let st = state.clone();
        ui.rescan_btn.connect_clicked(move |_| {
            rescan_devices(&ui2, &st);
            probe_disc(&ui2, &st);
        });
    }
    // 偏移变化 → 记到配置
    {
        let st = state.clone();
        let model_holder = Rc::new(RefCell::new(String::new()));
        let ui2 = ui.clone();
        let mh = model_holder.clone();
        ui.offset_spin.connect_value_changed(move |spin| {
            let mut s = st.borrow_mut();
            if let Some(d) = &s.disc {
                let model = d.model.clone();
                s.settings.remember_offset(&model, spin.value() as i32);
            } else {
                s.settings
                    .device
                    .offsets
                    .insert(mh.borrow().clone(), spin.value() as i32);
            }
            if !s.dirty {
                s.save_settings();
            }
            let _ = &ui2;
        });
    }
    // 全选 / 取消全选
    {
        let ui2 = ui.clone();
        let st = state.clone();
        ui.select_all.connect_toggled(move |c| {
            if *ui2.syncing_selection.borrow() {
                return;
            }
            let active = c.is_active();
            {
                let mut s = st.borrow_mut();
                for v in s.selected.iter_mut() {
                    *v = active;
                }
            }
            *ui2.syncing_selection.borrow_mut() = true;
            for cb in ui2.track_checks.borrow().iter() {
                cb.set_active(active);
            }
            *ui2.syncing_selection.borrow_mut() = false;
            ui2.select_all.set_inconsistent(false);
        });
    }
    // 开始 / 取消
    {
        let ui2 = ui.clone();
        let st = state.clone();
        ui.start_btn.connect_clicked(move |_| start_rip(&ui2, &st));
    }
    {
        let st = state.clone();
        let ui2 = ui.clone();
        ui.cancel_btn.connect_clicked(move |_| {
            if let Some(tx) = st.borrow().cancel.as_ref() {
                let _ = tx.send(true);
            }
            ui2.toast.add_toast(adw::Toast::new(&tr("正在取消…")));
        });
    }
    // 元数据：识别 / 写入
    {
        let ui2 = ui.clone();
        let st = state.clone();
        ui.meta_apply_btn
            .connect_clicked(move |_| apply_meta(&ui2, &st));
    }
    // 取得封面（手动）
    {
        let ui2 = ui.clone();
        let st = state.clone();
        ui.cover_btn
            .connect_clicked(move |_| request_cover(&ui2, &st, true));
    }
    // 导入 CUE
    {
        let ui2 = ui.clone();
        let st = state.clone();
        ui.import_cue_btn.connect_clicked(move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("CUE"));
            filter.add_pattern("*.cue");
            filter.add_pattern("*.CUE");
            let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder()
                .title(tr("选择 CUE 文件"))
                .modal(true)
                .filters(&filters)
                .build();
            let ui3 = ui2.clone();
            let st3 = st.clone();
            dialog.open(Some(&ui2.window), gtk::gio::Cancellable::NONE, move |res| {
                let Ok(file) = res else { return };
                let Some(path) = file.path() else { return };
                import_cue_path(&ui3, &st3, &path);
            });
        });
    }
    {
        let ui2 = ui.clone();
        let st = state.clone();
        ui.meta_dropdown.connect_selected_notify(move |dd| {
            let idx = dd.selected() as usize;
            {
                let mut s = st.borrow_mut();
                if idx >= s.meta_releases.len() {
                    return;
                }
                s.select_release(idx);
            }
            refresh_disc(&ui2, &st);
        });
    }
    // 结果页动作
    {
        let st = state.clone();
        ui.open_output_btn.connect_clicked(move |_| {
            let root = st.borrow().settings.output.root.clone();
            let _ = std::process::Command::new("xdg-open").arg(root).spawn();
        });
    }
    {
        let ui2 = ui.clone();
        let st = state.clone();
        ui.copy_log_btn.connect_clicked(move |_| {
            let text = st.borrow().log_text.clone();
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&text);
                ui2.toast
                    .add_toast(adw::Toast::new(&tr("日志已复制到剪贴板")));
            }
        });
    }
    // 设置项 → 只改内存状态并标记"未保存"，点保存按钮才写盘
    {
        let st = state.clone();
        let ui2 = ui.clone();
        ui.paranoia_spin.connect_value_changed(move |w| {
            st.borrow_mut().settings.rip.paranoia = w.value() as u8;
            mark_dirty(&ui2, &st);
        });
    }
    {
        let st = state.clone();
        let ui2 = ui.clone();
        ui.retries_spin.connect_value_changed(move |w| {
            st.borrow_mut().settings.rip.retries = w.value() as u32;
            mark_dirty(&ui2, &st);
        });
    }
    {
        let st = state.clone();
        let ui2 = ui.clone();
        ui.verify_spin.connect_value_changed(move |w| {
            st.borrow_mut().settings.rip.verify_passes = w.value() as u8;
            mark_dirty(&ui2, &st);
        });
    }
    {
        let st = state.clone();
        let ui2 = ui.clone();
        ui.formats_combo.connect_selected_notify(move |row| {
            st.borrow_mut().settings.rip.formats = match row.selected() {
                1 => vec!["flac".into(), "wav".into()],
                2 => vec!["wav".into()],
                _ => vec!["flac".into()],
            };
            mark_dirty(&ui2, &st);
        });
    }
    {
        let st = state.clone();
        let ui2 = ui.clone();
        ui.root_pick_btn.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder()
                .title(tr("选择输出根目录"))
                .modal(true)
                .build();
            let ui3 = ui2.clone();
            let st3 = st.clone();
            dialog.select_folder(Some(&ui2.window), gtk::gio::Cancellable::NONE, move |res| {
                let Ok(folder) = res else { return };
                let Some(path) = folder.path() else { return };
                ui3.root_row.set_subtitle(&path.display().to_string());
                st3.borrow_mut().settings.output.root = path;
                mark_dirty(&ui3, &st3);
            });
        });
    }
    {
        let st = state.clone();
        let ui2 = ui.clone();
        ui.discogs_row.connect_changed(move |e| {
            st.borrow_mut().secrets.discogs_token = e.text().to_string();
            mark_dirty(&ui2, &st);
        });
    }
    {
        let st = state.clone();
        let ui2 = ui.clone();
        ui.google_key_row.connect_changed(move |e| {
            st.borrow_mut().secrets.google_api_key = e.text().to_string();
            mark_dirty(&ui2, &st);
        });
    }
    {
        let st = state.clone();
        let ui2 = ui.clone();
        ui.google_cx_row.connect_changed(move |e| {
            st.borrow_mut().secrets.google_cx = e.text().to_string();
            mark_dirty(&ui2, &st);
        });
    }
    {
        let st = state.clone();
        let ui2 = ui.clone();
        ui.bin_entry.connect_changed(move |e| {
            st.borrow_mut().settings.advanced.bin = e.text().to_string();
            mark_dirty(&ui2, &st);
        });
    }
    {
        let st = state.clone();
        let ui2 = ui.clone();
        ui.sound_switch.connect_active_notify(move |w| {
            st.borrow_mut().settings.behavior.sound = w.is_active();
            mark_dirty(&ui2, &st);
        });
    }
    {
        let st = state.clone();
        let ui2 = ui.clone();
        ui.theme_combo.connect_selected_notify(move |row| {
            let idx = row.selected() as usize;
            let ev = ui2.sound_events.borrow().get(idx).cloned();
            if let Some(ev) = ev {
                st.borrow_mut().settings.behavior.sound_theme = ev;
                mark_dirty(&ui2, &st);
            }
        });
    }
    {
        let ui2 = ui.clone();
        ui.theme_test_btn.connect_clicked(move |_| {
            let idx = ui2.theme_combo.selected() as usize;
            let ev = ui2.sound_events.borrow().get(idx).cloned();
            if let Some(ev) = ev {
                let _ = std::process::Command::new("canberra-gtk-play")
                    .arg("-i")
                    .arg(ev)
                    .spawn();
            }
        });
    }
    {
        let st = state.clone();
        let ui2 = ui.clone();
        ui.eject_switch.connect_active_notify(move |w| {
            st.borrow_mut().settings.behavior.eject_when_done = w.is_active();
            mark_dirty(&ui2, &st);
        });
    }
    {
        let st = state.clone();
        let ui2 = ui.clone();
        ui.notify_switch.connect_active_notify(move |w| {
            st.borrow_mut().settings.behavior.notify = w.is_active();
            mark_dirty(&ui2, &st);
        });
    }
    {
        let st = state.clone();
        let ui2 = ui.clone();
        ui.shutdown_switch.connect_active_notify(move |w| {
            st.borrow_mut().settings.behavior.shutdown_when_done = w.is_active();
            mark_dirty(&ui2, &st);
        });
    }
    // 保存
    {
        let st = state.clone();
        let ui2 = ui.clone();
        ui.save_btn.connect_clicked(move |_| {
            {
                let mut s = st.borrow_mut();
                s.save_settings();
                s.dirty = false;
            }
            ui2.settings_status.set_subtitle(&tr("所有更改已保存"));
            ui2.save_btn.set_sensitive(false);
            ui2.toast.add_toast(adw::Toast::new(&tr("设置已保存")));
        });
    }
    // 恢复默认（只改控件与内存，仍需保存）
    {
        let st = state.clone();
        let ui2 = ui.clone();
        ui.reset_btn.connect_clicked(move |_| {
            {
                let mut s = st.borrow_mut();
                s.settings = Settings::default();
            }
            refresh_settings_widgets(&ui2, &st);
            mark_dirty(&ui2, &st);
            ui2.toast
                .add_toast(adw::Toast::new(&tr("已恢复默认值，记得保存")));
        });
    }
}

/// 把 `state.settings` 的值写回设置页控件。
fn refresh_settings_widgets(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    let s = state.borrow().settings.clone();
    ui.paranoia_spin.set_value(s.rip.paranoia as f64);
    ui.retries_spin.set_value(s.rip.retries as f64);
    ui.verify_spin.set_value(s.rip.verify_passes as f64);
    ui.formats_combo.set_selected(
        match s
            .rip
            .formats
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
        {
            ["flac"] => 0u32,
            ["flac", "wav"] => 1,
            ["wav"] => 2,
            _ => 0,
        },
    );
    ui.root_row
        .set_subtitle(&s.output.root.display().to_string());
    ui.bin_entry.set_text(&s.advanced.bin);
    ui.discogs_row
        .set_text(&state.borrow().secrets.discogs_token);
    ui.google_key_row
        .set_text(&state.borrow().secrets.google_api_key);
    ui.google_cx_row.set_text(&state.borrow().secrets.google_cx);
    ui.sound_switch.set_active(s.behavior.sound);
    let theme_idx = ui
        .sound_events
        .borrow()
        .iter()
        .position(|e| e == &s.behavior.sound_theme)
        .unwrap_or(0) as u32;
    ui.theme_combo.set_selected(theme_idx);
    ui.eject_switch.set_active(s.behavior.eject_when_done);
    ui.notify_switch.set_active(s.behavior.notify);
    ui.shutdown_switch.set_active(s.behavior.shutdown_when_done);
}

/// 标记有未保存的改动
fn mark_dirty(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    state.borrow_mut().dirty = true;
    ui.settings_status.set_subtitle(&tr("有未保存的更改"));
    ui.save_btn.set_sensitive(true);
}

/// 探测碟片并刷新界面。
pub fn probe_disc(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    let device = selected_device(ui);
    match disc::probe(Path::new(&device)) {
        Ok(info) => {
            let mut st = state.borrow_mut();
            st.device = device;
            // 同一张碟重复探测（例如介质轮询）时保留用户的勾选，
            // 只有换碟才重置成全选。
            let same_disc = st
                .disc
                .as_ref()
                .map(|old| {
                    old.leadout_lsn == info.leadout_lsn
                        && old.audio_track_count() == info.audio_track_count()
                })
                .unwrap_or(false);
            if !same_disc || st.selected.len() != info.audio_track_count() {
                st.selected = vec![true; info.audio_track_count()];
            }
            st.settings.device.path = PathBuf::from(&st.device);
            // 只有已知型号才套用记忆值；未知时保持 0 且**不要**写进配置，
            // 否则会用错误的 0 覆盖掉用户测好的偏移。
            let offset = st.settings.offset_for(&info.model).unwrap_or(0);
            let dirty = st.dirty;
            if !dirty {
                st.save_settings();
            }
            st.disc = Some(info);
            drop(st);
            ui.offset_spin.set_value(offset as f64);
            refresh_disc(ui, state);
            sync_select_all(ui, state);
            ui.start_btn.set_sensitive(true);
            if !same_disc {
                *ui.cover_cache.borrow_mut() = None;
                show_cover_placeholder(ui, &tr("无封面"));
            }
            // 抓轨前识别：元数据只需 TOC + MCN，不依赖音频
            start_meta_resolve(ui, state);
        }
        Err(e) => {
            state.borrow_mut().disc = None;
            refresh_disc(ui, state);
            ui.start_btn.set_sensitive(false);
            let empty = !matches!(
                disc::media_status(Path::new(&device)),
                cyanrip_core::disc::MediaStatus::Ok
            );
            if empty {
                // 只是没放碟 / 托盘开着：更新状态即可，不用弹错误
                ui.disc_summary.set_text(&tr("没有碟片"));
                ui.meta_label.set_text(&tr("未识别 · 没有碟片"));
            } else {
                ui.toast
                    .add_toast(adw::Toast::new(&format!("{}: {e}", tr("探测失败"))));
            }
        }
    }
}

/// 调用 `cyanrip -f` 测定读偏移（需要碟片在 AccurateRip 库中）。
pub fn detect_offset(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    let device = selected_device(ui);
    let bin = state.borrow().settings.advanced.bin.clone();
    let model = state
        .borrow()
        .disc
        .as_ref()
        .map(|d| d.model.clone())
        .unwrap_or_default();
    ui.toast.add_toast(adw::Toast::new(&tr(
        "正在测定读偏移：需读取部分碟片，约 1–3 分钟",
    )));

    // 工作线程只做阻塞调用，结果通过 mpsc 回主线程，避免 Rc 跨线程
    let (tx, rx) = mpsc::channel::<Option<i32>>();
    std::thread::spawn(move || {
        let parsed = std::process::Command::new(&bin)
            .arg("-d")
            .arg(&device)
            .arg("-f")
            .output()
            .ok()
            .and_then(|o| {
                let text = format!(
                    "{}{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                );
                parse_offset_output(&text)
            });
        let _ = tx.send(parsed);
    });

    let ui2 = ui.clone();
    let st2 = state.clone();
    glib::timeout_add_local(Duration::from_millis(300), move || match rx.try_recv() {
        Ok(Some(off)) => {
            ui2.offset_spin.set_value(off as f64);
            {
                let mut st = st2.borrow_mut();
                if !model.is_empty() {
                    st.settings.remember_offset(&model, off);
                }
                st.save_settings();
            }
            ui2.toast
                .add_toast(adw::Toast::new(&format!("{} {off:+}", tr("读偏移"))));
            glib::ControlFlow::Break
        }
        Ok(None) => {
            ui2.toast.add_toast(adw::Toast::new(&tr(
                "测定失败：这张碟可能不在 AccurateRip 数据库中",
            )));
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
    });
}

fn parse_offset_output(text: &str) -> Option<i32> {
    const NEEDLE: &str = "Drive offset of ";
    let idx = text.rfind(NEEDLE)?;
    let rest = &text[idx + NEEDLE.len()..];
    let end = rest.find(" found")?;
    rest[..end].trim().parse().ok()
}

/// 下拉里当前选中的设备路径。
fn selected_device(ui: &Rc<Ui>) -> String {
    let idx = ui.device_dropdown.selected() as usize;
    ui.device_paths
        .borrow()
        .get(idx)
        .cloned()
        .unwrap_or_else(|| "/dev/sr0".into())
}

/// 枚举 /dev/sr*，返回 (路径, 显示标签)。
fn scan_devices(current: &str) -> (Vec<String>, Vec<String>) {
    let mut paths: Vec<String> = disc::list_devices()
        .into_iter()
        .map(|p| p.display().to_string())
        .collect();
    // 配置里的设备即使当前不存在也保留，避免用户看不到自己的设置
    if !paths.iter().any(|p| p == current) && !current.is_empty() {
        paths.insert(0, current.to_string());
    }
    if paths.is_empty() {
        paths.push("/dev/sr0".into());
    }
    let labels = paths
        .iter()
        .map(|p| {
            let (vendor, model, _) = disc::drive_attrs(Path::new(p));
            if model == "unknown" {
                p.clone()
            } else {
                format!("{p}  ·  {} {}", vendor.trim(), model.trim())
            }
        })
        .collect();
    (paths, labels)
}

/// 重新扫描设备并保留当前选择。
fn rescan_devices(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    let current = selected_device(ui);
    let (paths, labels) = scan_devices(&current);
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    *ui.syncing_device.borrow_mut() = true;
    ui.device_dropdown
        .set_model(Some(&gtk::StringList::new(&label_refs)));
    let idx = paths.iter().position(|p| p == &current).unwrap_or(0);
    *ui.device_paths.borrow_mut() = paths;
    ui.device_dropdown.set_selected(idx as u32);
    *ui.syncing_device.borrow_mut() = false;
    state.borrow_mut().device = current.clone();
}

/// 根据逐轨勾选情况同步"全选"三态。
fn sync_select_all(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    if *ui.syncing_selection.borrow() {
        return;
    }
    let (all, none) = {
        let st = state.borrow();
        (
            st.selected.iter().all(|x| *x),
            st.selected.iter().all(|x| !*x),
        )
    };
    *ui.syncing_selection.borrow_mut() = true;
    ui.select_all.set_inconsistent(!all && !none);
    ui.select_all.set_active(all);
    *ui.syncing_selection.borrow_mut() = false;
}

/// 刷新碟片摘要与曲目表。
pub fn refresh_disc(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    while let Some(child) = ui.track_list.first_child() {
        ui.track_list.remove(&child);
    }
    ui.track_checks.borrow_mut().clear();
    let st = state.borrow();
    let Some(info) = &st.disc else {
        ui.disc_summary.set_text(&tr("尚未探测碟片"));
        return;
    };
    ui.disc_summary.set_text(&format!(
        "{} — {} {} · {} 轨 · {}",
        info.vendor.trim(),
        info.model.trim(),
        info.firmware.clone().unwrap_or_default(),
        info.audio_track_count(),
        fmt_duration(info.total_duration())
    ));
    for (i, t) in info.audio_tracks().enumerate() {
        let custom = st.track_titles.get(i).cloned().flatten();
        let title = match custom.or_else(|| t.title.clone()) {
            Some(x) => format!("{:02}  {}", t.number, x),
            None => format!("{:02}  {}", t.number, tr("未命名")),
        };
        let row = adw::ActionRow::builder()
            .title(&title)
            .subtitle(fmt_duration(t.duration()))
            .build();
        let check = gtk::CheckButton::new();
        check.set_active(st.selected.get(i).copied().unwrap_or(true));
        row.add_prefix(&check);
        row.set_activatable(false);

        let st2 = state.clone();
        let ui2 = ui.clone();
        check.connect_toggled(move |c| {
            {
                let mut s = st2.borrow_mut();
                if i < s.selected.len() {
                    s.selected[i] = c.is_active();
                }
            }
            sync_select_all(&ui2, &st2);
        });
        ui.track_checks.borrow_mut().push(check);
        ui.track_list.append(&row);
    }
}

pub fn start_rip(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    let mut st = state.borrow_mut();
    if st.phase == Phase::Ripping {
        return;
    }
    let Some(info) = st.disc.clone() else {
        ui.toast
            .add_toast(adw::Toast::new(&tr("请先放入并探测碟片")));
        return;
    };
    let selected = st.selected_tracks();
    if selected.is_empty() {
        ui.toast.add_toast(adw::Toast::new(&tr("没有选中任何曲目")));
        return;
    }
    let meta = st.current_release().cloned();
    if std::env::var("CYANRIP_GUI_DEBUG").as_deref() == Ok("1") {
        eprintln!(
            "RIP meta={:?} pending={}",
            meta.as_ref().map(|m| m.title.clone()),
            st.meta_pending
        );
    }
    let args = build_cyanrip_args(&st.settings, &info, &selected, meta.as_ref());
    if std::env::var("CYANRIP_GUI_DEBUG").as_deref() == Ok("1") {
        eprintln!("RIP selected={selected:?} args={args:?}");
    }
    let offset = st.offset();
    let root = st.settings.output.root.clone();
    let bin = PathBuf::from(&st.settings.advanced.bin);

    let (tx, rx) = mpsc::channel();
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let opts = RipOptions {
        bin,
        device: PathBuf::from(&st.device),
        offset,
        extra_args: args,
        log_path: None,
        cwd: Some(root.clone()),
    };
    spawn_worker(opts, info, tx, cancel_rx);

    st.rip_dirs_before = Some(tag::dirs_with_flac(&root));
    st.events = Some(rx);
    st.cancel = Some(cancel_tx);
    st.phase = Phase::Ripping;
    st.log_text.clear();
    st.progress = None;
    st.report = None;
    drop(st);

    *ui.log_lines_rendered.borrow_mut() = 0;
    ui.log_view.buffer().set_text("");
    ui.progress_title.set_text(&tr("准备中…"));
    ui.bar_track.set_fraction(0.0);
    ui.bar_total.set_fraction(0.0);
    ui.stats_label.set_text("等待第一帧进度…");
    ui.cancel_btn.set_sensitive(true);
    ui.start_btn.set_sensitive(false);
    ui.stack.set_visible_child_name("progress");
}

fn build_cyanrip_args(
    settings: &Settings,
    info: &DiscInfo,
    selected: &[u32],
    meta: Option<&MetaRelease>,
) -> Vec<String> {
    let mut args = vec![
        "-Z".to_string(),
        settings.rip.verify_passes.to_string(),
        "-P".to_string(),
        settings.rip.paranoia.to_string(),
        "-r".to_string(),
        settings.rip.retries.to_string(),
        "-o".to_string(),
        settings.rip.formats.join(","),
        "-N".to_string(),
    ];
    if settings.rip.overread {
        args.push("-O".into());
    }
    // 抓轨前就已识别的元数据直接交给 cyanrip：文件名、标签、日志一次到位
    if let Some(rel) = meta {
        let mut album = format!("artist={}:album={}", rel.artist, rel.title);
        if let Some(d) = rel.date.as_deref() {
            album.push_str(&format!(":date={d}"));
        }
        if let Some(l) = rel.label.as_deref() {
            album.push_str(&format!(":label={l}"));
        }
        if let Some(c) = rel.catno.as_deref() {
            album.push_str(&format!(":catalog_number={c}"));
        }
        if let Some(b) = rel.barcode.as_deref() {
            album.push_str(&format!(":barcode={b}"));
        }
        if rel.disc_total > 1 {
            album.push_str(&format!(
                ":disc={}:totaldiscs={}",
                rel.disc_number.max(1),
                rel.disc_total
            ));
        }
        args.push("-a".into());
        args.push(album);
        // 套装必须分目录，否则两张碟会往同一个文件夹写同名文件
        if rel.disc_total > 1 {
            args.push("-D".into());
            args.push("{album} CD{disc} [{format}]".into());
        }
        for (i, t) in rel.tracks.iter().enumerate() {
            args.push("-t".into());
            args.push(format!("{}=title={}", i + 1, t.title));
        }
    }
    if selected.len() != info.audio_track_count() {
        args.push("-l".into());
        args.push(
            selected
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    args
}

fn spawn_worker(
    opts: RipOptions,
    info: DiscInfo,
    tx: mpsc::Sender<Event>,
    cancel_rx: watch::Receiver<bool>,
) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let _ = tx.send(Event::RipFailed {
                    stage: cyanrip_core::event::Stage::Rip,
                    message: format!("无法创建运行时：{e}"),
                });
                return;
            }
        };
        rt.block_on(async move {
            let (ev_tx, ev_rx) = async_channel::unbounded::<Event>();
            let fwd = tokio::spawn(async move {
                while let Ok(ev) = ev_rx.recv().await {
                    if tx.send(ev).is_err() {
                        break;
                    }
                }
            });
            let _ = rip::run(opts, &info, ev_tx, cancel_rx).await;
            let _ = fwd.await;
        });
    });
}

/// 每 1.5 秒检查一次介质状态：换碟后自动重新探测。
fn start_media_watch(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    let ui2 = ui.clone();
    let st = state.clone();
    glib::timeout_add_local(Duration::from_millis(1500), move || {
        // 抓轨期间不打扰
        if st.borrow().phase == Phase::Ripping {
            return glib::ControlFlow::Continue;
        }
        let device = selected_device(&ui2);
        let status = disc::media_status(Path::new(&device));
        let prev = *ui2.last_media.borrow();
        if status != prev {
            if std::env::var("CYANRIP_GUI_DEBUG").as_deref() == Ok("1") {
                eprintln!("MEDIA {prev:?} -> {status:?}");
            }
            *ui2.last_media.borrow_mut() = status;
            match status {
                cyanrip_core::disc::MediaStatus::Ok => {
                    // 新碟就位：静默重新探测并刷新列表
                    let refreshed = auto_probe_disc(&ui2, &st);
                    if std::env::var("CYANRIP_GUI_DEBUG").as_deref() == Ok("1") {
                        eprintln!(
                            "MEDIA auto_probe={refreshed} tracks={}",
                            st.borrow()
                                .disc
                                .as_ref()
                                .map(|d| d.audio_track_count())
                                .unwrap_or(0)
                        );
                    }
                    if refreshed {
                        ui2.toast
                            .add_toast(adw::Toast::new(&tr("检测到新碟片，已刷新曲目")));
                    }
                }
                _ => {
                    let mut s = st.borrow_mut();
                    if s.disc.is_some() {
                        s.disc = None;
                        s.selected.clear();
                        drop(s);
                        refresh_disc(&ui2, &st);
                        ui2.start_btn.set_sensitive(false);
                    }
                }
            }
        }
        glib::ControlFlow::Continue
    });
}

/// 静默探测（不弹错误提示），成功返回 true。
fn auto_probe_disc(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) -> bool {
    let device = selected_device(ui);
    let Ok(info) = disc::probe(Path::new(&device)) else {
        return false;
    };
    let (offset, changed) = {
        let mut st = state.borrow_mut();
        st.device = device;
        // 同一张碟的重复探测不动用户勾选；换碟才重置并重新识别
        let same = st
            .disc
            .as_ref()
            .map(|old| {
                old.leadout_lsn == info.leadout_lsn
                    && old.audio_track_count() == info.audio_track_count()
            })
            .unwrap_or(false);
        if !same || st.selected.len() != info.audio_track_count() {
            st.selected = vec![true; info.audio_track_count()];
            st.meta_releases.clear();
            st.track_titles.clear();
        }
        let off = st.settings.offset_for(&info.model).unwrap_or(0);
        st.disc = Some(info);
        (off, !same)
    };
    ui.offset_spin.set_value(offset as f64);
    refresh_disc(ui, state);
    sync_select_all(ui, state);
    ui.start_btn.set_sensitive(true);
    if changed {
        start_meta_resolve(ui, state);
    }
    true
}

fn start_event_pump(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    let ui2 = ui.clone();
    let st = state.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let mut events: Vec<Event> = Vec::new();
        {
            let st = st.borrow();
            if let Some(rx) = &st.events {
                loop {
                    match rx.try_recv() {
                        Ok(ev) => events.push(ev),
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => break,
                    }
                }
            }
        }
        for ev in events {
            handle_event(&ui2, &st, ev);
        }
        glib::ControlFlow::Continue
    });
}

fn handle_event(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>, ev: Event) {
    match ev {
        Event::Progress(p) => {
            {
                let mut st = state.borrow_mut();
                st.progress = Some(p.clone());
            }
            ui.progress_title
                .set_text(&format!("{} {}", tr("轨"), p.track));
            ui.bar_track.set_fraction(p.track_pct as f64 / 100.0);
            ui.bar_track.set_text(Some(&format!("{:.1}%", p.track_pct)));
            ui.bar_total.set_fraction(p.overall_pct as f64 / 100.0);
            ui.bar_total
                .set_text(Some(&format!("{} {:.1}%", tr("总进度"), p.overall_pct)));
            let eta = p.eta.map(fmt_eta).unwrap_or_else(|| "?".into());
            let speed = p
                .speed_x
                .map(|x| format!("{x:.1}x"))
                .unwrap_or_else(|| "—".into());
            ui.stats_label.set_text(&format!(
                "Ripping track {}, progress - {:.2}%, ETA - {}   ·   {} · {} {}",
                p.track,
                p.track_pct,
                eta,
                speed,
                tr("已用"),
                fmt_duration(p.elapsed)
            ));
        }
        Event::LogLine(line) => {
            let mut st = state.borrow_mut();
            st.append_log(&line);
            drop(st);
            let buffer = ui.log_view.buffer();
            let mut end = buffer.end_iter();
            buffer.insert(&mut end, &line);
            let mut end = buffer.end_iter();
            buffer.insert(&mut end, "\n");
            let mut rendered = ui.log_lines_rendered.borrow_mut();
            *rendered += 1;
            if *rendered > 4000 {
                // 防止 TextView 无限膨胀：超过 4000 行时清空重来
                let text = state.borrow().log_text.clone();
                ui.log_view.buffer().set_text(&text);
                *rendered = 0;
            }
            let mut end = ui.log_view.buffer().end_iter();
            ui.log_view.scroll_to_iter(&mut end, 0.0, false, 0.0, 0.0);
        }
        Event::TrackDone(t) => {
            ui.toast
                .add_toast(adw::Toast::new(&format!("{} {}", t.number, tr("轨完成"))));
        }
        Event::RipFinished { report, .. } => {
            let settings = {
                let mut st = state.borrow_mut();
                st.phase = Phase::Done;
                st.report = Some(report.clone());
                st.events = None;
                st.cancel = None;
                st.settings.clone()
            };
            fill_result(ui, state, &report);
            let root = settings.output.root.clone();
            let discid = state.borrow().disc.as_ref().and_then(meta::disc_id);
            let dir = pick_rip_dir(state, &root, discid.as_deref());
            *ui.last_rip_dir.borrow_mut() = dir;
            if settings.behavior.auto_tag_after_rip && state.borrow().current_release().is_some() {
                apply_meta(ui, state);
            }
            ui.cancel_btn.set_sensitive(false);
            ui.start_btn.set_sensitive(true);
            ui.stack.set_visible_child_name("result");
            play_done_sound(&settings);
            if settings.behavior.notify {
                send_notification(
                    &tr("抓轨完成"),
                    &format!(
                        "{}/{} {}",
                        report.tracks_accurate,
                        report.tracks_ripped,
                        tr("轨精确命中")
                    ),
                );
            }
            if settings.behavior.shutdown_when_done {
                if crate::instance::other_instances() > 0 {
                    // 另一个前端可能还在抓另一台光驱，关机留给最后一个完成的人
                    ui.toast
                        .add_toast(adw::Toast::new(&tr("另一个实例仍在运行，已跳过关机")));
                } else {
                    let _ = std::process::Command::new("systemctl")
                        .arg("poweroff")
                        .spawn();
                }
            }
        }
        Event::RipFailed { stage, message } => {
            {
                let mut st = state.borrow_mut();
                st.phase = Phase::Failed;
                st.events = None;
                st.cancel = None;
            }
            ui.cancel_btn.set_sensitive(false);
            ui.start_btn.set_sensitive(true);
            let dialog = adw::AlertDialog::builder()
                .heading(tr("抓轨失败"))
                .body(format!("{}：{message}", stage.as_str()))
                .build();
            dialog.add_response("close", &tr("关闭"));
            dialog.present(Some(&ui.window));
        }
        Event::Cancelled { completed_tracks } => {
            {
                let mut st = state.borrow_mut();
                st.phase = Phase::Idle;
                st.events = None;
                st.cancel = None;
            }
            ui.cancel_btn.set_sensitive(false);
            ui.start_btn.set_sensitive(true);
            ui.stack.set_visible_child_name("disc");
            ui.toast.add_toast(adw::Toast::new(&format!(
                "{} {} {}",
                tr("已取消，完成"),
                completed_tracks.len(),
                tr("轨")
            )));
        }
        Event::DiscProbed(info) => {
            state.borrow_mut().disc = Some(info);
            refresh_disc(ui, state);
        }
        Event::DiscRemoved => {
            state.borrow_mut().disc = None;
            refresh_disc(ui, state);
        }
        Event::TrackStarted { index, .. } => {
            ui.progress_title
                .set_text(&format!("{} {}", tr("正在抓取轨"), index));
        }
    }
}

fn fill_result(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>, report: &RipReport) {
    while let Some(child) = ui.result_body.first_child() {
        ui.result_body.remove(&child);
    }

    // 标题：一眼看到结论
    let verdict = if report.all_accurate() {
        tr("全部精确命中，与 AccurateRip 数据库一致")
    } else if report.errors > 0 {
        format!("{} {}", tr("存在抓轨错误"), report.errors)
    } else if report.tracks_accurate + 1 >= report.tracks_ripped {
        tr("基本命中，个别轨未入库或校验和不一致")
    } else {
        tr("部分轨未通过 AccurateRip 校验")
    };
    ui.result_summary.set_text(&format!(
        "{}/{} {} · {verdict}",
        report.tracks_accurate,
        report.tracks_ripped,
        tr("轨精确命中")
    ));

    // 概览
    let (vendor, model, fw, disc_tracks, disc_len) = {
        let st = state.borrow();
        match &st.disc {
            Some(d) => (
                d.vendor.clone(),
                d.model.clone(),
                d.firmware.clone().unwrap_or_default(),
                d.audio_track_count(),
                fmt_duration(d.total_duration()),
            ),
            None => (
                String::new(),
                String::new(),
                String::new(),
                0,
                String::new(),
            ),
        }
    };

    let overview = gtk::Grid::builder()
        .row_spacing(6)
        .column_spacing(24)
        .build();
    let mut r = 0;
    let mut add_kv = |k: &str, v: String| {
        let key = gtk::Label::new(Some(k));
        key.set_xalign(0.0);
        key.add_css_class("dim-label");
        let val = gtk::Label::new(Some(&v));
        val.set_xalign(0.0);
        val.set_selectable(true);
        overview.attach(&key, 0, r, 1, 1);
        overview.attach(&val, 1, r, 1, 1);
        r += 1;
    };
    add_kv(
        &tr("设备"),
        format!("{vendor} {model} {fw}").trim().to_string(),
    );
    add_kv(
        &tr("碟片"),
        format!("{disc_tracks} {} · {disc_len}", tr("轨")),
    );
    add_kv(
        &tr("读偏移"),
        report
            .offset
            .map(|o| format!("{o:+} samples"))
            .unwrap_or_else(|| "—".into()),
    );
    add_kv(
        &tr("C2 支持"),
        match report.c2_supported {
            Some(true) => tr("支持（仍以多遍比对为准）"),
            Some(false) => tr("驱动不支持，依赖多遍比对"),
            None => "—".into(),
        },
    );
    add_kv(
        &tr("Paranoia 级别"),
        report.paranoia_level.clone().unwrap_or_else(|| "—".into()),
    );
    add_kv(
        &tr("AccurateRip"),
        match report.ar_id.as_deref() {
            Some(id) => format!(
                "{} · {id}",
                if report.ar_found == Some(true) {
                    tr("数据库中有这张碟")
                } else {
                    tr("数据库未命中")
                }
            ),
            None => tr("未查询"),
        },
    );
    add_kv(
        &tr("精确命中"),
        format!("{}/{}", report.tracks_accurate, report.tracks_ripped),
    );
    add_kv(&tr("抓轨错误"), report.errors.to_string());
    if let Some(t) = &report.total_time {
        add_kv(&tr("音频总时长"), t.clone());
    }
    if let Some(id) = report.disc_id.as_deref() {
        add_kv(&tr("MusicBrainz DiscID"), id.to_string());
    }
    if let Some(m) = report.mcn.as_deref() {
        add_kv(&tr("碟片 MCN"), m.to_string());
    }
    if let Some(p) = report.log_path.as_ref() {
        add_kv(&tr("日志文件"), p.display().to_string());
    }
    ui.result_body.append(&overview);
    ui.result_body
        .append(&gtk::Separator::new(Orientation::Horizontal));

    // 明细表
    let table_title = gtk::Label::new(Some(&tr("曲目明细")));
    table_title.set_xalign(0.0);
    table_title.add_css_class("heading");
    ui.result_body.append(&table_title);

    let table = gtk::Grid::builder()
        .row_spacing(4)
        .column_spacing(18)
        .build();
    let headers = [
        tr("轨"),
        tr("时长"),
        tr("EAC CRC32"),
        tr("AR v1"),
        tr("AR v2"),
        tr("conf"),
        tr("状态"),
    ];
    for (c, h) in headers.iter().enumerate() {
        let l = gtk::Label::new(Some(h));
        l.set_xalign(if c == 0 || c >= 5 { 0.5 } else { 0.0 });
        l.add_css_class("dim-label");
        table.attach(&l, c as i32, 0, 1, 1);
    }

    for (i, t) in report.tracks.iter().enumerate() {
        let row = i as i32 + 1;
        let cells: [String; 7] = [
            format!("{:02}", t.number),
            t.duration.map(fmt_duration).unwrap_or_else(|| "—".into()),
            t.crc32
                .map(|c| format!("{c:08X}"))
                .unwrap_or_else(|| "—".into()),
            t.ar_v1
                .map(|c| format!("{c:08X}"))
                .unwrap_or_else(|| "—".into()),
            t.ar_v2
                .map(|c| format!("{c:08X}"))
                .unwrap_or_else(|| "—".into()),
            t.ar_confidence
                .map(|c| c.to_string())
                .unwrap_or_else(|| "—".into()),
            match t.ar_state {
                ArState::Accurate => tr("精确命中"),
                ArState::NotFound => tr("未入库"),
                ArState::Mismatch => tr("不一致"),
                ArState::Unknown => tr("无数据"),
            },
        ];
        for (c, text) in cells.iter().enumerate() {
            let l = gtk::Label::new(Some(text));
            l.set_xalign(if c == 0 || c >= 5 { 0.5 } else { 0.0 });
            if (2..=4).contains(&c) || c == 1 {
                l.add_css_class("monospace");
            }
            match (c, t.ar_state) {
                (6, ArState::Accurate) => l.add_css_class("success"),
                (6, ArState::NotFound) => l.add_css_class("warning"),
                (6, ArState::Mismatch) => l.add_css_class("error"),
                (6, ArState::Unknown) => l.add_css_class("dim-label"),
                _ => {}
            }
            table.attach(&l, c as i32, row, 1, 1);
        }
    }
    ui.result_body.append(&table);
    ui.result_body
        .append(&gtk::Separator::new(Orientation::Horizontal));
    ui.result_body.append(&ui.meta_box);
}

/// cyanrip 风格的 ETA：`45s` / `2m` / `1h 3m`
fn fmt_eta(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}h {}m", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}m", s / 60)
    } else {
        format!("{s}s")
    }
}

/// 导入 CUE：作为一条候选插到最前面并选中。
pub fn import_cue_path(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>, path: &Path) {
    let release = match meta::cue::parse_file(path) {
        Ok(r) => r,
        Err(e) => {
            ui.toast
                .add_toast(adw::Toast::new(&format!("{}: {e:#}", tr("导入失败"))));
            return;
        }
    };
    let track_count = release.tracks.len();
    let title = release.title.clone();
    let artist = release.artist.clone();
    {
        let mut st = state.borrow_mut();
        st.meta_releases.retain(|r| r.source != MetaSource::Cue);
        st.meta_releases.insert(0, release);
        st.select_release(0);
    }
    let labels: Vec<String> = state
        .borrow()
        .meta_releases
        .iter()
        .map(candidate_label)
        .collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    ui.meta_dropdown
        .set_model(Some(&gtk::StringList::new(&refs)));
    ui.meta_dropdown.set_visible(true);
    ui.meta_dropdown.set_selected(0);
    ui.meta_label.set_text(&format!(
        "CUE：{} — {}（{} {}）",
        artist,
        title,
        track_count,
        tr("轨")
    ));
    ui.meta_apply_btn.set_sensitive(true);
    refresh_disc(ui, state);
    request_cover(ui, state, false);
    ui.toast
        .add_toast(adw::Toast::new(&format!("{} {}", tr("已导入"), title)));
}

/// 候选的唯一键：同一次识别里区分不同发行版
fn release_key(r: &MetaRelease) -> String {
    format!("{}:{}", r.source.as_str(), r.id)
}

/// 取候选的封面：缓存命中就直接显示，否则后台下载（不阻塞 GTK 主线程）。
fn request_cover(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>, force: bool) {
    let release = state.borrow().current_release().cloned();
    let Some(release) = release else {
        show_cover_placeholder(ui, &tr("无封面"));
        return;
    };
    let token = {
        let st = state.borrow();
        let t = st.secrets.discogs_token.trim().to_string();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    };
    let google_key = {
        let k = state.borrow().secrets.google_api_key.trim().to_string();
        if k.is_empty() {
            None
        } else {
            Some(k)
        }
    };
    let google_cx = {
        let c = state.borrow().secrets.google_cx.trim().to_string();
        if c.is_empty() {
            None
        } else {
            Some(c)
        }
    };
    let key = release_key(&release);
    // 已有缓存就直接用（不同素材源的 URL 可能不同，但同一条候选就认这张）
    let primary = release.cover_url.clone();
    if !force {
        if let Some((cached_key, path)) = ui.cover_cache.borrow().as_ref() {
            if cached_key == &key && path.exists() {
                show_cover(ui, path);
                return;
            }
        }
        if ui.cover_inflight.borrow().as_deref() == Some(key.as_str()) {
            return;
        }
        if ui.cover_failed.borrow().as_deref() == Some(key.as_str()) {
            show_cover_placeholder(ui, &tr("无封面（自动获取失败，可点按钮重试）"));
            return;
        }
    }
    *ui.cover_inflight.borrow_mut() = Some(key.clone());
    ui.cover_btn.set_sensitive(false);
    ui.cover_placeholder.set_text(&tr("正在取得封面…"));
    ui.cover_placeholder.set_visible(true);
    ui.cover_picture.set_visible(false);
    let key_for_thread = key.clone();
    let (tx, rx) = mpsc::channel();
    *ui.cover_rx.borrow_mut() = Some(rx);
    std::thread::spawn(move || {
        // 1) 候选自带的 URL；2) 失败则用 Discogs 兜底
        let mut res = match primary.as_deref() {
            Some(u) => meta::download_cover(u),
            None => Err(anyhow::anyhow!("没有封面地址")),
        };
        let debug = std::env::var("CYANRIP_GUI_DEBUG").as_deref() == Ok("1");
        if res.is_err() {
            if debug {
                eprintln!("COVER step=primary failed");
            }
            if let Some(alt) = meta::cover_fallback(&release, token.as_deref()) {
                res = meta::download_cover(&alt);
                if debug {
                    eprintln!("COVER step=discogs ok={}", res.is_ok());
                }
            } else if debug {
                eprintln!("COVER step=discogs none");
            }
        }
        // Google 图片搜索兜底（需在设置里配 key + cx）
        if res.is_err() {
            match meta::google_cover(&release, google_key.as_deref(), google_cx.as_deref()) {
                Some(alt) => {
                    res = meta::download_cover(&alt);
                    if debug {
                        eprintln!("COVER step=google ok={}", res.is_ok());
                    }
                }
                None if debug => eprintln!("COVER step=google skipped"),
                None => {}
            }
        }
        let _ = tx.send((key_for_thread, res));
    });
}

fn show_cover(ui: &Rc<Ui>, path: &Path) {
    ui.cover_picture.set_filename(Some(path));
    ui.cover_picture.set_visible(true);
    ui.cover_placeholder.set_visible(false);
    ui.cover_btn.set_sensitive(true);
}

fn show_cover_placeholder(ui: &Rc<Ui>, text: &str) {
    ui.cover_placeholder.set_text(text);
    ui.cover_placeholder.set_visible(true);
    ui.cover_picture.set_visible(false);
    ui.cover_btn.set_sensitive(true);
}

fn candidate_label(r: &MetaRelease) -> String {
    let disc = if r.disc_total > 1 {
        format!(" CD{}/{}", r.disc_number, r.disc_total)
    } else {
        String::new()
    };
    format!(
        "{} — {}{}（{}，{}，score {}%）",
        r.artist,
        r.title,
        disc,
        r.date.clone().unwrap_or_else(|| "年份未知".into()),
        r.source.as_str(),
        r.score
    )
}

/// 后台识别元数据（阻塞的 HTTP 调用放线程里）。
fn start_meta_resolve(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    let Some(disc) = state.borrow().disc.clone() else {
        ui.toast
            .add_toast(adw::Toast::new(&tr("请先放入并探测碟片")));
        return;
    };
    let root = state.borrow().settings.output.root.clone();
    let token = {
        let st = state.borrow();
        let t = st.secrets.discogs_token.trim().to_string();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    };
    ui.meta_label.set_text(&tr("识别中…"));
    ui.meta_status.set_text(&tr("识别中…"));
    ui.meta_apply_btn.set_sensitive(false);
    state.borrow_mut().meta_pending = true;
    let (tx, rx) = mpsc::channel();
    *ui.meta_rx.borrow_mut() = Some(rx);
    std::thread::spawn(move || {
        let dir = tag::newest_rip_dir(&root);
        let files = dir.as_ref().map(|d| tag::flac_files(d)).unwrap_or_default();
        // 自测钩子：不联网，用一条合成候选验证 UI → 打标签链路
        if std::env::var("CYANRIP_GUI_FAKE_META").as_deref() == Ok("1") {
            let release = MetaRelease {
                source: cyanrip_core::meta::MetaSource::Discogs,
                id: "0".into(),
                title: "测试专辑".into(),
                artist: "测试艺人".into(),
                date: Some("2026".into()),
                country: None,
                label: None,
                catno: None,
                barcode: None,
                disc_number: 1,
                disc_total: 1,
                tracks: vec![cyanrip_core::meta::MetaTrack {
                    number: 1,
                    title: "测试曲目".into(),
                    length: None,
                }],
                cover_url: None,
                score: 100,
            };
            let _ = tx.send(MetaOutcome::Candidates(vec![release], dir));
            return;
        }
        let out = meta::resolve_with(&disc, files.first().map(|p| p.as_path()), token.as_deref());
        let _ = tx.send(MetaOutcome::Candidates(out, dir));
    });
}

/// 把选中候选写入标签并归档。
fn apply_meta(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    let release = state.borrow().current_release().cloned();
    let Some(release) = release else {
        ui.toast.add_toast(adw::Toast::new(&tr("请先识别元数据")));
        return;
    };
    let dir = ui
        .last_rip_dir
        .borrow()
        .clone()
        .or_else(|| tag::newest_rip_dir(&state.borrow().settings.output.root));
    let Some(dir) = dir else {
        ui.toast
            .add_toast(adw::Toast::new(&tr("找不到这次抓轨的目录")));
        return;
    };
    let root = state.borrow().settings.output.root.clone();
    let cached_cover = ui.cover_cache.borrow().clone();
    ui.meta_status.set_text(&tr("写入标签中…"));
    ui.meta_apply_btn.set_sensitive(false);
    let (tx, rx) = mpsc::channel();
    *ui.meta_rx.borrow_mut() = Some(rx);
    std::thread::spawn(move || {
        let files = tag::flac_files(&dir);
        // 优先复用界面上已经下载好的封面（包括 Discogs 兜底那张）
        let key = format!("{}:{}", release.source.as_str(), release.id);
        let cover = match &cached_cover {
            Some((cached_key, path)) if cached_key == &key && path.exists() => Some(path.clone()),
            _ => release
                .cover_url
                .as_deref()
                .and_then(|u| meta::download_cover(u).ok()),
        };
        let out = match tag::apply(&files, &release, &root, (1, 1), cover.as_deref()) {
            Ok(d) => MetaOutcome::Applied(d),
            Err(e) => MetaOutcome::ApplyFailed(format!("{e:#}")),
        };
        let _ = tx.send(out);
    });
}

fn start_meta_pump(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    let ui2 = ui.clone();
    let st = state.clone();
    glib::timeout_add_local(Duration::from_millis(200), move || {
        // 封面下载结果
        let cover = {
            let rx = ui2.cover_rx.borrow();
            rx.as_ref().and_then(|r| r.try_recv().ok())
        };
        if let Some((key, res)) = cover {
            if ui2.cover_inflight.borrow().as_deref() == Some(key.as_str()) {
                *ui2.cover_inflight.borrow_mut() = None;
            }
            match res {
                Ok(path) => {
                    *ui2.cover_failed.borrow_mut() = None;
                    if std::env::var("CYANRIP_GUI_DEBUG").as_deref() == Ok("1") {
                        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        eprintln!("COVER key={key} bytes={size}");
                    }
                    *ui2.cover_cache.borrow_mut() = Some((key, path.clone()));
                    show_cover(&ui2, &path);
                }
                Err(e) => {
                    *ui2.cover_failed.borrow_mut() = Some(key.clone());
                    show_cover_placeholder(&ui2, &tr("封面下载失败"));
                    ui2.toast
                        .add_toast(adw::Toast::new(&format!("{}: {e}", tr("封面下载失败"))));
                }
            }
        }
        let msg = {
            let rx = ui2.meta_rx.borrow();
            rx.as_ref().and_then(|r| r.try_recv().ok())
        };
        if let Some(msg) = msg {
            match msg {
                MetaOutcome::Candidates(list, dir) => {
                    st.borrow_mut().meta_pending = false;
                    if std::env::var("CYANRIP_GUI_DEBUG").as_deref() == Ok("1") {
                        eprintln!(
                            "META candidates={} titles={:?}",
                            list.len(),
                            list.first().map(|r| r
                                .tracks
                                .iter()
                                .take(3)
                                .map(|t| t.title.clone())
                                .collect::<Vec<_>>())
                        );
                    }
                    if dir.is_some() {
                        *ui2.last_rip_dir.borrow_mut() = dir;
                    }
                    // 手工导入的 CUE 优先：网络结果不能把它顶掉
                    let cues: Vec<MetaRelease> = st
                        .borrow()
                        .meta_releases
                        .iter()
                        .filter(|r| matches!(r.source, MetaSource::Cue))
                        .cloned()
                        .collect();
                    let mut merged = cues.clone();
                    merged.extend(list);
                    if merged.is_empty() {
                        ui2.meta_label
                            .set_text(&tr("未识别 · 没有匹配（可导入 CUE 或换 Discogs token）"));
                        ui2.meta_dropdown.set_visible(false);
                        ui2.meta_status
                            .set_text(&tr("没有找到匹配的发行版（可导入 CUE 或换 token）"));
                        ui2.meta_apply_btn.set_sensitive(false);
                        let mut s = st.borrow_mut();
                        s.meta_releases.clear();
                        s.track_titles.clear();
                        drop(s);
                        refresh_disc(&ui2, &st);
                    } else {
                        let labels: Vec<String> = merged.iter().map(candidate_label).collect();
                        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                        ui2.meta_dropdown
                            .set_model(Some(&gtk::StringList::new(&refs)));
                        ui2.meta_dropdown.set_visible(true);
                        let first = &merged[0];
                        let disc_tag = if first.disc_total > 1 {
                            format!(" CD{}/{}", first.disc_number, first.disc_total)
                        } else {
                            String::new()
                        };
                        ui2.meta_label.set_text(&format!(
                            "{} — {}{}（{} · {}）",
                            first.artist,
                            first.title,
                            disc_tag,
                            first.date.clone().unwrap_or_else(|| "年份未知".into()),
                            first.source.as_str()
                        ));
                        ui2.meta_status
                            .set_text(&format!("{} {}", tr("找到候选："), merged.len()));
                        let mut s = st.borrow_mut();
                        s.meta_releases = merged;
                        s.select_release(0);
                        let has_dir = ui2.last_rip_dir.borrow().is_some();
                        drop(s);
                        ui2.meta_dropdown.set_selected(0);
                        ui2.meta_apply_btn.set_sensitive(has_dir);
                        refresh_disc(&ui2, &st);
                        request_cover(&ui2, &st, false);
                        if std::env::var("CYANRIP_GUI_AUTOAPPLY").as_deref() == Ok("1")
                            && st.borrow().phase == Phase::Done
                        {
                            apply_meta(&ui2, &st);
                        }
                    }
                }
                MetaOutcome::Applied(dir) => {
                    if std::env::var("CYANRIP_GUI_DEBUG").as_deref() == Ok("1") {
                        eprintln!("META applied={}", dir.display());
                    }
                    ui2.meta_status
                        .set_text(&format!("{} {}", tr("已归档到"), dir.display()));
                    ui2.toast.add_toast(adw::Toast::new(&tr("标签写入完成")));
                }
                MetaOutcome::ApplyFailed(e) => {
                    ui2.meta_status
                        .set_text(&format!("{} {e}", tr("写入失败：")));
                    ui2.meta_apply_btn.set_sensitive(true);
                }
            }
            let _ = &st;
        }
        glib::ControlFlow::Continue
    });
}

/// freedesktop 声音主题里实际存在的事件名（用于下拉选择）
fn list_sound_events() -> Vec<String> {
    let mut events: Vec<String> = std::fs::read_dir("/usr/share/sounds/freedesktop/stereo")
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let p = e.path();
                    if p.extension().and_then(|x| x.to_str()) != Some("oga") {
                        return None;
                    }
                    p.file_stem().map(|s| s.to_string_lossy().to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    events.sort();
    // 常用的事件放前面，"complete" 作为默认值置顶
    if let Some(i) = events.iter().position(|e| e == "complete") {
        let c = events.remove(i);
        events.insert(0, c);
    }
    for extra in ["bell", "message-new-instant", "dialog-information"] {
        if let Some(i) = events.iter().position(|e| e == extra) {
            let v = events.remove(i);
            events.insert(1.min(events.len()), v);
        }
    }
    if events.is_empty() {
        events.push("complete".into());
    }
    events
}

/// 定位本次抓轨产生的目录：用开始前的快照做差集，避免同时抓两张碟时
/// 把另一个实例的目录当成自己的（`newest_rip_dir` 只按 mtime 猜）。
fn pick_rip_dir(
    state: &Rc<RefCell<AppState>>,
    root: &Path,
    discid: Option<&str>,
) -> Option<PathBuf> {
    // 首选：日志里的 DiscID 与本机这张碟一致。并发抓两张碟时，
    // 目录创建时间/顺序都不可靠，DiscID 是唯一能区分彼此的标记。
    if let Some(id) = discid {
        let mut hits: Vec<PathBuf> = tag::dirs_with_flac(root)
            .into_iter()
            .filter(|d| tag::dir_log_has_discid(d, id))
            .collect();
        if !hits.is_empty() {
            return hits.drain(..).max_by_key(|d| {
                std::fs::metadata(d)
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH)
            });
        }
    }
    let current = tag::dirs_with_flac(root);
    let before = state.borrow().rip_dirs_before.clone().unwrap_or_default();
    let mut fresh: Vec<PathBuf> = current
        .into_iter()
        .filter(|d| !before.iter().any(|b| b == d))
        .collect();
    match fresh.len() {
        0 => tag::newest_rip_dir(root),
        1 => fresh.pop(),
        _ => fresh.into_iter().max_by_key(|d| {
            std::fs::metadata(d)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH)
        }),
    }
}

fn play_done_sound(settings: &Settings) {
    if !settings.behavior.sound {
        return;
    }
    let mut cmd = std::process::Command::new("canberra-gtk-play");
    if settings.behavior.sound_file.is_empty() {
        cmd.arg("-i").arg(&settings.behavior.sound_theme);
    } else {
        cmd.arg("-f").arg(&settings.behavior.sound_file);
    }
    let _ = cmd.spawn();
}

fn send_notification(title: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .arg("-a")
        .arg("cyanrip-frontend")
        .arg(title)
        .arg(body)
        .spawn();
}
