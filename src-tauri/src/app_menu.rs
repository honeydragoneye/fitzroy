//! 창 머리글의 메뉴 버튼으로 여는 앱 메뉴. 열 때마다 현재 설정으로 새로 만든다.

use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_autostart::ManagerExt;

use crate::feedback;
use crate::menu_bar;
use crate::providers;
use crate::refresher::Refresher;
use crate::settings::SettingsStore;
use crate::updates::Updates;

/// "Fitzroy에 관하여"를 고르면 창으로 보내는 이벤트 이름
pub const SHOW_ABOUT: &str = "show-about";
/// 보여줄 서비스를 바꾸면 창으로 보내는 이벤트 이름. 값은 서비스 id.
pub const SHOW_PAGE: &str = "show-page";

const REFRESH_NOW: &str = "refresh-now";
const SHOW_PERCENT: &str = "show-percent";
const AUTOSTART: &str = "autostart";
const CHECK_UPDATES_NOW: &str = "check-updates-now";
const SEND_FEEDBACK: &str = "send-feedback";
const AUTO_CHECK_UPDATES: &str = "auto-check-updates";
const ABOUT: &str = "about";
const QUIT: &str = "quit";
const INTERVAL_PREFIX: &str = "interval-";
const PAGE_PREFIX: &str = "page-";

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

    let menu = Menu::with_items(
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
            &CheckMenuItem::with_id(
                app,
                AUTO_CHECK_UPDATES,
                "업데이트 자동 확인",
                true,
                settings.check_for_updates,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, CHECK_UPDATES_NOW, "업데이트 확인", true, None::<&str>)?,
            &MenuItem::with_id(app, SEND_FEEDBACK, "문제 신고 🕊️", true, None::<&str>)?,
            &MenuItem::with_id(app, ABOUT, "Fitzroy에 관하여", true, None::<&str>)?,
            &MenuItem::with_id(app, QUIT, "Fitzroy 종료", true, None::<&str>)?,
        ],
    )?;

    // 맨 위에 창에 보여줄 서비스를 고르는 항목을 둔다.
    let page_items = providers::PAGES
        .iter()
        .map(|(id, name)| {
            let checked = settings.page == *id;
            let id = format!("{PAGE_PREFIX}{id}");
            CheckMenuItem::with_id(app, id, name, true, checked, None::<&str>)
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let separator = PredefinedMenuItem::separator(app)?;
    let mut page_refs: Vec<&dyn IsMenuItem<Wry>> = page_items
        .iter()
        .map(|item| item as &dyn IsMenuItem<Wry>)
        .collect();
    page_refs.push(&separator);
    menu.prepend_items(&page_refs)?;
    Ok(menu)
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
        AUTO_CHECK_UPDATES => {
            let settings = app
                .state::<SettingsStore>()
                .update(|s| s.check_for_updates = !s.check_for_updates);
            // 켰을 때 확인할 때가 지났으면 바로 확인하게 한다.
            if settings.check_for_updates {
                app.state::<Updates>().wake();
            }
        }
        CHECK_UPDATES_NOW => {
            // 결과는 앱 정보 화면에 보여준다.
            let _ = app.emit(SHOW_ABOUT, ());
            app.state::<Updates>().check_now();
        }
        SEND_FEEDBACK => feedback::open_mail(app),
        ABOUT => {
            let _ = app.emit(SHOW_ABOUT, ());
        }
        QUIT => app.exit(0),
        _ if id.starts_with(PAGE_PREFIX) => {
            let page = &id[PAGE_PREFIX.len()..];
            if providers::PAGES.iter().any(|(known, _)| *known == page) {
                app.state::<SettingsStore>()
                    .update(|s| s.page = page.to_string());
                menu_bar::update(app, &app.state::<Refresher>().reports());
                let _ = app.emit(SHOW_PAGE, page);
            }
        }
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
