//! 창 머리글의 메뉴 버튼으로 여는 앱 메뉴. 열 때마다 현재 설정으로 새로 만든다.

use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_autostart::ManagerExt;

use crate::menu_bar;
use crate::refresher::Refresher;
use crate::settings::SettingsStore;

/// "Fitzroy에 관하여"를 고르면 창으로 보내는 이벤트 이름
pub const SHOW_ABOUT: &str = "show-about";

const REFRESH_NOW: &str = "refresh-now";
const SHOW_PERCENT: &str = "show-percent";
const AUTOSTART: &str = "autostart";
const ABOUT: &str = "about";
const QUIT: &str = "quit";
const INTERVAL_PREFIX: &str = "interval-";

const INTERVALS: &[(u64, &str)] = &[(30, "30초"), (60, "1분"), (120, "2분"), (300, "5분")];

pub fn build(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let settings = app.state::<SettingsStore>().get();
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);

    let interval_items = INTERVALS
        .iter()
        .map(|(secs, label)| {
            let checked = settings.refresh_interval_secs == *secs;
            let id = format!("{INTERVAL_PREFIX}{secs}");
            CheckMenuItem::with_id(app, id, label, true, checked, None::<&str>)
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let interval_refs: Vec<&dyn IsMenuItem<Wry>> = interval_items
        .iter()
        .map(|item| item as &dyn IsMenuItem<Wry>)
        .collect();

    Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, REFRESH_NOW, "지금 새로 고침", true, None::<&str>)?,
            &Submenu::with_items(app, "갱신 주기", true, &interval_refs)?,
            &PredefinedMenuItem::separator(app)?,
            &CheckMenuItem::with_id(
                app,
                SHOW_PERCENT,
                "메뉴 막대에 사용률 표시",
                true,
                settings.show_percent_in_menu_bar,
                None::<&str>,
            )?,
            &CheckMenuItem::with_id(
                app,
                AUTOSTART,
                "로그인 시 자동 실행",
                true,
                autostart,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, ABOUT, "Fitzroy에 관하여", true, None::<&str>)?,
            &MenuItem::with_id(app, QUIT, "Fitzroy 종료", true, None::<&str>)?,
        ],
    )
}

pub fn handle(app: &AppHandle, id: &str) {
    match id {
        REFRESH_NOW => app.state::<Refresher>().refresh_now(),
        SHOW_PERCENT => {
            app.state::<SettingsStore>()
                .update(|s| s.show_percent_in_menu_bar = !s.show_percent_in_menu_bar);
            menu_bar::update(app, &app.state::<Refresher>().reports());
        }
        AUTOSTART => {
            let autostart = app.autolaunch();
            let _ = if autostart.is_enabled().unwrap_or(false) {
                autostart.disable()
            } else {
                autostart.enable()
            };
        }
        ABOUT => {
            let _ = app.emit(SHOW_ABOUT, ());
        }
        QUIT => app.exit(0),
        _ => {
            let Some(secs) = id
                .strip_prefix(INTERVAL_PREFIX)
                .and_then(|secs| secs.parse::<u64>().ok())
            else {
                return;
            };
            app.state::<SettingsStore>()
                .update(|s| s.refresh_interval_secs = secs);
            // 기다리던 갱신 스레드를 깨워 새 주기로 다시 기다리게 한다.
            app.state::<Refresher>().refresh_now();
        }
    }
}
