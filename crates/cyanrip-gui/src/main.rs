//! cyanrip-frontend：cyanrip 的 GTK4 / libadwaita 前端。

mod i18n;
mod state;
mod ui;

use adw::prelude::*;
use gtk::glib;
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;

const APP_ID: &str = "io.github.cyanrip.Frontend";

fn main() -> glib::ExitCode {
    i18n::init();
    let mut builder = adw::Application::builder().application_id(APP_ID);
    // 自测时需要与用户已打开的实例并存
    if std::env::var("CYANRIP_GUI_STANDALONE").as_deref() == Ok("1") {
        builder = builder.flags(gtk::gio::ApplicationFlags::NON_UNIQUE);
    }
    let app = builder.build();

    app.connect_activate(|app| {
        let state = Rc::new(RefCell::new(state::AppState::new()));
        let ui = ui::build(app, state.clone());
        ui::refresh_disc(&ui, &state);

        // 启动即尝试探测当前设备；失败只提示，不阻塞界面
        ui::probe_disc(&ui, &state);
        if let Some(d) = state.borrow().disc.clone() {
            println!(
                "DISC {} tracks, {}, device={}, leadout={}",
                d.audio_track_count(),
                cyanrip_core::model::fmt_duration(d.total_duration()),
                d.device.display(),
                d.leadout_lsn
            );
        }

        // 自动抓轨（端到端自测）：只抓第一轨，完成后打印结果并退出
        if std::env::var("CYANRIP_GUI_AUTORIP").as_deref() == Ok("1") {
            {
                let mut st = state.borrow_mut();
                if let Some(d) = st.disc.clone() {
                    for (i, _) in d.audio_tracks().enumerate() {
                        if i < st.selected.len() {
                            st.selected[i] = i == 0;
                        }
                    }
                }
            }
            // 等识别结束（最多 15 秒）再开抓，好让元数据随命令行一起传给 cyanrip
            {
                let ui3 = ui.clone();
                let st3 = state.clone();
                let ticks = std::rc::Rc::new(std::cell::Cell::new(0u32));
                glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
                    ticks.set(ticks.get() + 1);
                    let pending = st3.borrow().meta_pending;
                    if !pending || ticks.get() > 60 {
                        ui::start_rip(&ui3, &st3);
                        return glib::ControlFlow::Break;
                    }
                    glib::ControlFlow::Continue
                });
            }
            let st2 = state.clone();
            let app2 = app.clone();
            let ui2 = ui.clone();
            let wait_ticks = std::rc::Rc::new(std::cell::Cell::new(0u32));
            glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
                match st2.borrow().phase {
                    state::Phase::Done => {
                        // 给元数据识别 / 写入留出时间（最多 12 秒）
                        if wait_ticks.get() < 24 {
                            wait_ticks.set(wait_ticks.get() + 1);
                            return glib::ControlFlow::Continue;
                        }
                        let r = st2.borrow().report.clone().unwrap_or_default();
                        let widgets = ui2.result_body.observe_children().n_items();
                        println!(
                            "AUTORIP_OK {}/{} RESULT_WIDGETS={widgets} META_STATUS={}",
                            r.tracks_accurate,
                            r.tracks_ripped,
                            ui2.meta_status.text()
                        );
                        app2.quit();
                        glib::ControlFlow::Break
                    }
                    state::Phase::Failed => {
                        println!("AUTORIP_FAIL");
                        app2.quit();
                        glib::ControlFlow::Break
                    }
                    _ => glib::ControlFlow::Continue,
                }
            });
        }

        // 设置页保存流程自测：验证"改动 → 变脏 → 保存写盘 → 还原"
        if std::env::var("CYANRIP_GUI_SELFTEST").as_deref() == Ok("1") {
            let ui2 = ui.clone();
            let st2 = state.clone();
            let app2 = app.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(400), move || {
                // 设备下拉
                let devices = ui2.device_paths.borrow().len();
                let sel_idx = ui2.device_dropdown.selected() as usize;
                let sel_dev = ui2
                    .device_paths
                    .borrow()
                    .get(sel_idx)
                    .cloned()
                    .unwrap_or_default();

                // 全选 / 取消全选
                ui2.select_all.set_active(false);
                let all_off = st2.borrow().selected.iter().all(|x| !*x);
                ui2.select_all.set_active(true);
                let all_on = st2.borrow().selected.iter().all(|x| *x);

                let original = st2.borrow().settings.rip.verify_passes;
                let initial_disabled = !ui2.save_btn.is_sensitive();
                ui2.verify_spin.set_value((original + 1) as f64);
                let dirty = st2.borrow().dirty && ui2.save_btn.is_sensitive();
                ui2.save_btn.emit_clicked();
                let expected = format!("verify_passes = {}", original + 1);
                let saved = std::fs::read_to_string(&st2.borrow().config_path)
                    .map(|s| s.contains(&expected))
                    .unwrap_or(false);
                // 还原并保存
                ui2.verify_spin.set_value(original as f64);
                ui2.save_btn.emit_clicked();
                let clean = !st2.borrow().dirty && !ui2.save_btn.is_sensitive();

                // 「刷新」= 重扫设备 + 探测：同一张碟应该保持原样
                let before = st2.borrow().disc.as_ref().map(|d| d.leadout_lsn);
                let devs_before = ui2.device_paths.borrow().len();
                ui2.rescan_btn.emit_clicked();
                let after = st2.borrow().disc.as_ref().map(|d| d.leadout_lsn);
                let devs_after = ui2.device_paths.borrow().len();
                let refresh_ok = before == after && devs_before == devs_after;
                let eject_classes: Vec<String> = ui2
                    .eject_btn
                    .css_classes()
                    .iter()
                    .map(|c| c.to_string())
                    .collect();
                let refresh_classes: Vec<String> = ui2
                    .rescan_btn
                    .css_classes()
                    .iter()
                    .map(|c| c.to_string())
                    .collect();
                let icon_ok = gtk::gdk::Display::default()
                    .map(|d| gtk::IconTheme::for_display(&d).has_icon("media-eject-symbolic"))
                    .unwrap_or(false);

                println!(
                    "SELFTEST initial_disabled={initial_disabled} dirty={dirty} saved={saved} clean_after_save={clean} all_off={all_off} all_on={all_on} devices={devices} selected={sel_dev} refresh_ok={refresh_ok} eject_icon={icon_ok} eject_classes={eject_classes:?} refresh_classes={refresh_classes:?} disc_after={after:?}"
                );
                app2.quit();
                glib::ControlFlow::Break
            });
        }

        // 弹出按钮自测：模拟点击，检查托盘状态，再自动收回
        if std::env::var("CYANRIP_GUI_EJECTTEST").as_deref() == Ok("1") {
            let ui2 = ui.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(800), move || {
                ui2.eject_btn.emit_clicked();
                println!("EJECTTEST clicked");
                glib::ControlFlow::Break
            });
            let st3 = state.clone();
            let app2 = app.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(2600), move || {
                let dev = st3.borrow().device.clone();
                let res = cyanrip_core::disc::probe(std::path::Path::new(&dev));
                println!(
                    "EJECTTEST probe_after_click={:?}",
                    res.err().map(|e| e.to_string())
                );
                let closed = cyanrip_core::disc::close_tray(std::path::Path::new(&dev));
                println!("EJECTTEST close={closed:?}");
                app2.quit();
                glib::ControlFlow::Break
            });
        }

        // 冒烟测试：CYANRIP_GUI_SMOKE=1 时 2 秒后自动退出
        if std::env::var("CYANRIP_GUI_SMOKE").as_deref() == Ok("1") {
            let app = app.clone();
            glib::timeout_add_seconds_local(2, move || {
                println!("SMOKE_OK");
                app.quit();
                glib::ControlFlow::Break
            });
        }
    });

    app.run()
}
