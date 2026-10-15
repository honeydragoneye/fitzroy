mod app_menu;
mod feedback;
mod menu_bar;
mod providers;
mod refresher;
mod settings;
mod updates;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, LogicalPosition, Manager, State, WebviewWindow, WindowEvent};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_positioner::{Position, WindowExt};

use menu_bar::MenuBarState;
use providers::ProviderReport;
use refresher::Refresher;
use settings::SettingsStore;
use updates::{Release, Updates};

const MAIN_WINDOW: &str = "main";
const REPOSITORY_URL: &str = "https://github.com/honeydragoneye/fitzroy";

// 창이 열린 상태에서 트레이 아이콘을 누르면 포커스를 잃어 먼저 숨겨진 뒤 클릭 이벤트가 온다.
// 이 간격 안에 온 클릭은 "닫기"로 보고 다시 열지 않는다.
const REOPEN_GUARD: Duration = Duration::from_millis(250);

#[derive(Default)]
struct PanelState {
    hidden_on_blur_at: Mutex<Option<Instant>>,
}

/// 가장 최근에 가져온 사용량. 새 값은 `usage-updated` 이벤트로 온다.
#[tauri::command]
fn get_usage(refresher: State<'_, Refresher>) -> Vec<ProviderReport> {
    refresher.reports()
}

/// 서비스의 사용량 화면을 연다. 주소 목록은 앱이 정해 둔 것만 쓰고, 화면에서 받은 주소는 열지 않는다.
#[tauri::command]
fn open_usage_page(app: AppHandle, id: String) -> Result<(), String> {
    let opener = app.opener();
    for url in providers::usage_page_urls(&id) {
        if opener.open_url(*url, None::<&str>).is_ok() {
            return Ok(());
        }
    }
    Err("사용량 화면을 열 수 없습니다".into())
}

/// 창 머리글의 메뉴 버튼 아래에 앱 메뉴를 띄운다. 위치는 창 왼쪽 위 기준이다.
#[tauri::command]
fn show_app_menu(app: AppHandle, window: WebviewWindow, x: f64, y: f64) -> Result<(), String> {
    let menu = app_menu::build(&app).map_err(|e| e.to_string())?;
    window
        .popup_menu_at(&menu, LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())
}

/// 창에 보여줄 서비스 id. 메뉴에서 바꾸면 `show-page` 이벤트로 온다.
#[tauri::command]
fn get_page(settings: State<'_, SettingsStore>) -> String {
    settings.get().page
}

/// 지금 버전보다 새 릴리스. 새로 찾으면 `update-available` 이벤트로 온다.
#[tauri::command]
fn get_update(updates: State<'_, Updates>) -> Option<Release> {
    updates.available()
}

/// 새 릴리스 페이지를 연다. 주소는 앱이 GitHub 에서 받아 확인한 것만 쓴다.
#[tauri::command]
fn open_release(app: AppHandle, updates: State<'_, Updates>) -> Result<(), String> {
    let release = updates.available().ok_or("새 버전이 없습니다")?;
    app.opener()
        .open_url(release.url, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_repository(app: AppHandle) -> Result<(), String> {
    app.opener()
        .open_url(REPOSITORY_URL, None::<&str>)
        .map_err(|e| e.to_string())
}

fn toggle_panel(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };

    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }

    let just_closed = app
        .state::<PanelState>()
        .hidden_on_blur_at
        .lock()
        .unwrap()
        .take()
        .is_some_and(|at| at.elapsed() < REOPEN_GUARD);
    if just_closed {
        return;
    }

    app.state::<Refresher>().refresh_if_stale();
    let _ = window.move_window(Position::TrayBottomCenter);
    let _ = window.show();
    let _ = window.set_focus();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .manage(PanelState::default())
        .manage(MenuBarState::default())
        .invoke_handler(tauri::generate_handler![
            get_page,
            get_update,
            get_usage,
            open_release,
            open_repository,
            open_usage_page,
            show_app_menu
        ])
        .on_menu_event(|app, event| app_menu::handle(app, event.id().as_ref()))
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let settings_path = app
                .path()
                .app_config_dir()
                .ok()
                .map(|dir| dir.join("settings.json"));
            app.manage(SettingsStore::load(settings_path));
            let (refresher, wake_rx) = Refresher::new();
            app.manage(refresher);

            let tray = TrayIconBuilder::with_id(menu_bar::TRAY_ID)
                .icon(menu_bar::template_icon())
                .icon_as_template(true)
                // 개발 모드로 띄운 앱은 이름 뒤에 *를 붙여 설치한 앱과 구별한다.
                .tooltip(if cfg!(debug_assertions) { "Fitzroy *" } else { "Fitzroy" })
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    let app = tray.app_handle();
                    tauri_plugin_positioner::on_tray_event(app, &event);

                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_panel(app);
                    }
                })
                .build(app)?;
            menu_bar::shrink_title_font(&tray);

            let cache_dir = app.path().app_cache_dir().ok();
            updates::spawn(app.handle(), cache_dir.clone());
            refresher::spawn(app.handle().clone(), providers::all(cache_dir), wake_rx);

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != MAIN_WINDOW {
                return;
            }
            if let WindowEvent::Focused(false) = event {
                let state = window.state::<PanelState>();
                *state.hidden_on_blur_at.lock().unwrap() = Some(Instant::now());
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
