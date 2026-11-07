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
                "DISC {} tracks, {}",
                d.audio_track_count(),
                cyanrip_core::model::fmt_duration(d.total_duration())
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
            ui::start_rip(&ui, &state);
            let st2 = state.clone();
            let app2 = app.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
                match st2.borrow().phase {
                    state::Phase::Done => {
                        let r = st2.borrow().report.clone().unwrap_or_default();
                        println!("AUTORIP_OK {}/{}", r.tracks_accurate, r.tracks_ripped);
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
