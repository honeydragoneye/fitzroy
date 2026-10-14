mod menu_bar;
mod providers;
mod refresher;
mod settings;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, WindowEvent};
use tauri_plugin_positioner::{Position, WindowExt};

use menu_bar::MenuBarState;
use providers::ProviderReport;
use refresher::Refresher;
use settings::{Settings, SettingsStore};

const MAIN_WINDOW: &str = "main";

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

#[tauri::command]
fn get_settings(settings: State<'_, SettingsStore>) -> Settings {
    settings.get()
}

#[tauri::command]
fn set_show_percent_in_menu_bar(
    app: AppHandle,
    settings: State<'_, SettingsStore>,
    refresher: State<'_, Refresher>,
    show: bool,
) -> Settings {
    let updated = settings.update(|s| s.show_percent_in_menu_bar = show);
    menu_bar::update(&app, &refresher.reports());
    updated
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
        .manage(PanelState::default())
        .manage(MenuBarState::default())
        .invoke_handler(tauri::generate_handler![
            get_usage,
            get_settings,
            set_show_percent_in_menu_bar
        ])
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
                .tooltip("Fitzroy")
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
