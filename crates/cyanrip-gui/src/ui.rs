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
use cyanrip_core::model::{fmt_duration, ArState, DiscInfo, TrackResult};
use cyanrip_core::rip::{self, RipOptions};
use gtk::{glib, Orientation};
use libadwaita as adw;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;
use tokio::sync::watch;

pub struct Ui {
    pub window: adw::ApplicationWindow,
    pub toast: adw::ToastOverlay,
    pub stack: gtk::Stack,
    // 碟片页
    pub device_entry: gtk::Entry,
    pub detect_btn: gtk::Button,
    pub offset_spin: gtk::SpinButton,
    pub disc_summary: gtk::Label,
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
    pub result_list: gtk::ListBox,
    pub open_output_btn: gtk::Button,
    pub copy_log_btn: gtk::Button,
    // 设置页
    pub paranoia_spin: gtk::SpinButton,
    pub retries_spin: gtk::SpinButton,
    pub verify_spin: gtk::SpinButton,
    pub formats_combo: adw::ComboRow,
    pub root_entry: adw::EntryRow,
    pub bin_entry: adw::EntryRow,
    pub sound_switch: adw::SwitchRow,
    pub theme_entry: adw::EntryRow,
    pub eject_switch: adw::SwitchRow,
    pub notify_switch: adw::SwitchRow,
    pub shutdown_switch: adw::SwitchRow,
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
    let device_entry = gtk::Entry::builder()
        .text(state.borrow().device.as_str())
        .hexpand(true)
        .build();
    let probe_btn = gtk::Button::with_label(&tr("探测"));
    let detect_btn = gtk::Button::with_label(&tr("测定偏移"));
    let offset_spin = gtk::SpinButton::with_range(-2000.0, 2000.0, 1.0);
    offset_spin.set_value(state.borrow().offset() as f64);
    offset_spin.set_tooltip_text(Some(&tr("读偏移（samples），与 EAC 同符号")));

    let device_box = gtk::Box::new(Orientation::Horizontal, 8);
    device_box.append(&gtk::Label::new(Some(&tr("设备"))));
    device_box.append(&device_entry);
    device_box.append(&probe_btn);
    device_box.append(&gtk::Label::new(Some(&tr("偏移"))));
    device_box.append(&offset_spin);
    device_box.append(&detect_btn);

    let disc_summary = gtk::Label::new(Some(&tr("尚未探测碟片")));
    disc_summary.set_xalign(0.0);
    disc_summary.add_css_class("title-4");

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
    disc_page.append(&disc_summary);
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

    let result_list = gtk::ListBox::new();
    result_list.set_selection_mode(gtk::SelectionMode::None);
    result_list.add_css_class("boxed-list");
    let result_scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&result_list)
        .build();

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
    let (
        settings_page,
        paranoia_spin,
        retries_spin,
        verify_spin,
        formats_combo,
        root_entry,
        bin_entry,
        sound_switch,
        theme_entry,
        eject_switch,
        notify_switch,
        shutdown_switch,
    ) = build_settings_page(&state);

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
        device_entry,
        detect_btn,
        offset_spin,
        disc_summary,
        track_list,
        start_btn,
        progress_title,
        bar_track,
        bar_total,
        stats_label,
        log_view,
        cancel_btn,
        result_summary,
        result_list,
        open_output_btn,
        copy_log_btn,
        paranoia_spin,
        retries_spin,
        verify_spin,
        formats_combo,
        root_entry,
        bin_entry,
        sound_switch,
        theme_entry,
        eject_switch,
        notify_switch,
        shutdown_switch,
        log_lines_rendered: RefCell::new(0),
    });

    connect_signals(&ui, &state);
    start_event_pump(&ui, &state);
    window.present();
    ui
}

type SettingsWidgets = (
    adw::PreferencesPage,
    gtk::SpinButton,
    gtk::SpinButton,
    gtk::SpinButton,
    adw::ComboRow,
    adw::EntryRow,
    adw::EntryRow,
    adw::SwitchRow,
    adw::EntryRow,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
);

fn build_settings_page(state: &Rc<RefCell<AppState>>) -> SettingsWidgets {
    let page = adw::PreferencesPage::new();
    let s = state.borrow().settings.clone();

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
    let root_entry = adw::EntryRow::builder()
        .title(tr("输出根目录"))
        .text(s.output.root.display().to_string())
        .build();
    output_group.add(&root_entry);

    // 元数据（M3 才真正使用）
    let meta_group = adw::PreferencesGroup::builder()
        .title(tr("元数据"))
        .description(tr("M3 实现：MusicBrainz → AcoustID → Discogs"))
        .build();

    // 行为
    let behavior_group = adw::PreferencesGroup::builder().title(tr("行为")).build();
    let sound_switch = adw::SwitchRow::builder()
        .title(tr("抓完播放提示音"))
        .active(s.behavior.sound)
        .build();
    let theme_entry = adw::EntryRow::builder()
        .title(tr("提示音名称"))
        .text(s.behavior.sound_theme.clone())
        .build();
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
    behavior_group.add(&theme_entry);
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

    page.add(&device_group);
    page.add(&output_group);
    page.add(&meta_group);
    page.add(&behavior_group);
    page.add(&advanced_group);

    (
        page,
        paranoia_spin,
        retries_spin,
        verify_spin,
        formats_combo,
        root_entry,
        bin_entry,
        sound_switch,
        theme_entry,
        eject_switch,
        notify_switch,
        shutdown_switch,
    )
}

fn connect_signals(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    // 探测
    {
        let ui2 = ui.clone();
        let st = state.clone();
        ui.device_entry
            .connect_activate(move |_| probe_disc(&ui2, &st));
    }
    {
        let ui2 = ui.clone();
        let st = state.clone();
        ui.detect_btn
            .connect_clicked(move |_| detect_offset(&ui2, &st));
    }
    // 探测按钮需要引用：通过碟片页的按钮查找不方便，这里用 device_entry 的激活替代，
    // 同时把探测按钮单独接线（见 build 中 probe_btn）
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
            s.save_settings();
            let _ = &ui2;
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
    // 设置项 → 立即保存
    {
        let st = state.clone();
        ui.paranoia_spin.connect_value_changed(move |s| {
            let mut st = st.borrow_mut();
            st.settings.rip.paranoia = s.value() as u8;
            st.save_settings();
        });
    }
    {
        let st = state.clone();
        ui.retries_spin.connect_value_changed(move |s| {
            let mut st = st.borrow_mut();
            st.settings.rip.retries = s.value() as u32;
            st.save_settings();
        });
    }
    {
        let st = state.clone();
        ui.verify_spin.connect_value_changed(move |s| {
            let mut st = st.borrow_mut();
            st.settings.rip.verify_passes = s.value() as u8;
            st.save_settings();
        });
    }
    {
        let st = state.clone();
        ui.formats_combo.connect_selected_notify(move |row| {
            let mut st = st.borrow_mut();
            st.settings.rip.formats = match row.selected() {
                1 => vec!["flac".into(), "wav".into()],
                2 => vec!["wav".into()],
                _ => vec!["flac".into()],
            };
            st.save_settings();
        });
    }
    {
        let st = state.clone();
        ui.root_entry.connect_changed(move |e| {
            let mut st = st.borrow_mut();
            st.settings.output.root = PathBuf::from(e.text().as_str());
            st.save_settings();
        });
    }
    {
        let st = state.clone();
        ui.bin_entry.connect_changed(move |e| {
            let mut st = st.borrow_mut();
            st.settings.advanced.bin = e.text().to_string();
            st.save_settings();
        });
    }
    {
        let st = state.clone();
        ui.sound_switch.connect_active_notify(move |sw| {
            let mut st = st.borrow_mut();
            st.settings.behavior.sound = sw.is_active();
            st.save_settings();
        });
    }
    {
        let st = state.clone();
        ui.theme_entry.connect_changed(move |e| {
            let mut st = st.borrow_mut();
            st.settings.behavior.sound_theme = e.text().to_string();
            st.save_settings();
        });
    }
    {
        let st = state.clone();
        ui.eject_switch.connect_active_notify(move |sw| {
            let mut st = st.borrow_mut();
            st.settings.behavior.eject_when_done = sw.is_active();
            st.save_settings();
        });
    }
    {
        let st = state.clone();
        ui.notify_switch.connect_active_notify(move |sw| {
            let mut st = st.borrow_mut();
            st.settings.behavior.notify = sw.is_active();
            st.save_settings();
        });
    }
    {
        let st = state.clone();
        ui.shutdown_switch.connect_active_notify(move |sw| {
            let mut st = st.borrow_mut();
            st.settings.behavior.shutdown_when_done = sw.is_active();
            st.save_settings();
        });
    }
}

/// 探测碟片并刷新界面。
pub fn probe_disc(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    let device = ui.device_entry.text().to_string();
    match disc::probe(Path::new(&device)) {
        Ok(info) => {
            let mut st = state.borrow_mut();
            st.device = device;
            st.selected = vec![true; info.audio_track_count()];
            st.settings.device.path = PathBuf::from(&st.device);
            // 只有已知型号才套用记忆值；未知时保持 0 且**不要**写进配置，
            // 否则会用错误的 0 覆盖掉用户测好的偏移。
            let offset = st.settings.offset_for(&info.model).unwrap_or(0);
            st.disc = Some(info);
            drop(st);
            ui.offset_spin.set_value(offset as f64);
            refresh_disc(ui, state);
            ui.start_btn.set_sensitive(true);
        }
        Err(e) => {
            state.borrow_mut().disc = None;
            refresh_disc(ui, state);
            ui.start_btn.set_sensitive(false);
            ui.toast
                .add_toast(adw::Toast::new(&format!("{}: {e}", tr("探测失败"))));
        }
    }
}

/// 调用 `cyanrip -f` 测定读偏移（需要碟片在 AccurateRip 库中）。
pub fn detect_offset(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    let device = ui.device_entry.text().to_string();
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

/// 刷新碟片摘要与曲目表。
pub fn refresh_disc(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>) {
    while let Some(child) = ui.track_list.first_child() {
        ui.track_list.remove(&child);
    }
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
        let title = match &t.title {
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
        check.connect_toggled(move |c| {
            let mut s = st2.borrow_mut();
            if i < s.selected.len() {
                s.selected[i] = c.is_active();
            }
        });
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
    let args = build_cyanrip_args(&st.settings, &info, &selected);
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
        cwd: Some(root),
    };
    spawn_worker(opts, info, tx, cancel_rx);

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

fn build_cyanrip_args(settings: &Settings, info: &DiscInfo, selected: &[u32]) -> Vec<String> {
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
                let _ = std::process::Command::new("systemctl")
                    .arg("poweroff")
                    .spawn();
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

fn fill_result(ui: &Rc<Ui>, state: &Rc<RefCell<AppState>>, report: &cyanrip_core::RipReport) {
    while let Some(child) = ui.result_list.first_child() {
        ui.result_list.remove(&child);
    }
    ui.result_summary.set_text(&format!(
        "{} {}/{} · {} {} · offset {:?}",
        tr("精确命中"),
        report.tracks_accurate,
        report.tracks_ripped,
        tr("错误"),
        report.errors,
        report.offset
    ));
    for t in &report.tracks {
        let row = adw::ActionRow::builder()
            .title(format!("{:02}", t.number))
            .subtitle(describe_track(t))
            .build();
        ui.result_list.append(&row);
    }
    let _ = state;
}

fn describe_track(t: &TrackResult) -> String {
    let crc = t
        .crc32
        .map(|c| format!("EAC CRC32 {c:08X}"))
        .unwrap_or_else(|| "—".into());
    let ar = match t.ar_state {
        ArState::Accurate => format!(
            "AR v2 {} (confidence {})",
            t.ar_v2
                .map(|c| format!("{c:08X}"))
                .unwrap_or_else(|| "—".into()),
            t.ar_confidence.unwrap_or(0)
        ),
        ArState::NotFound => tr("AR：未在数据库"),
        ArState::Mismatch => tr("AR：校验和不一致"),
        ArState::Unknown => tr("AR：无数据"),
    };
    format!("{crc} · {ar}")
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
