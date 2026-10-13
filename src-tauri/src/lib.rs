mod providers;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_positioner::{Position, WindowExt};

use providers::{Provider, ProviderReport};

const MAIN_WINDOW: &str = "main";

// 창이 열린 상태에서 트레이 아이콘을 누르면 포커스를 잃어 먼저 숨겨진 뒤 클릭 이벤트가 온다.
// 이 간격 안에 온 클릭은 "닫기"로 보고 다시 열지 않는다.
const REOPEN_GUARD: Duration = Duration::from_millis(250);

#[derive(Default)]
struct PanelState {
    hidden_on_blur_at: Mutex<Option<Instant>>,
}

struct Providers(Arc<Vec<Box<dyn Provider>>>);

#[tauri::command]
async fn get_usage(providers: tauri::State<'_, Providers>) -> Result<Vec<ProviderReport>, String> {
    let providers = providers.0.clone();
    tauri::async_runtime::spawn_blocking(move || providers::fetch_all(&providers))
        .await
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
        .manage(Providers(Arc::new(providers::all())))
        .invoke_handler(tauri::generate_handler![get_usage])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            TrayIconBuilder::with_id("main")
                .icon(tauri::include_image!("icons/tray.png"))
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
