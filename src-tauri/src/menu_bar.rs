//! 메뉴 막대 아이콘 옆의 사용률 글자와 경고 색.

use std::sync::Mutex;

use tauri::image::Image;
use tauri::{AppHandle, Manager};

use crate::providers::ProviderReport;
use crate::settings::SettingsStore;

pub const TRAY_ID: &str = "main";

const WARNING_PERCENT: f64 = 80.0;
const CRITICAL_PERCENT: f64 = 95.0;
const WARNING_COLOR: [u8; 3] = [255, 149, 0];
const CRITICAL_COLOR: [u8; 3] = [255, 59, 48];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Severity {
    Normal,
    Warning,
    Critical,
}

pub fn severity(percent: Option<f64>) -> Severity {
    match percent {
        Some(p) if p >= CRITICAL_PERCENT => Severity::Critical,
        Some(p) if p >= WARNING_PERCENT => Severity::Warning,
        _ => Severity::Normal,
    }
}

/// 메뉴 막대에 띄울 값: 현재 세션 사용률
pub fn primary_percent(reports: &[ProviderReport]) -> Option<f64> {
    reports.iter().find_map(|report| report.usage.session_percent)
}

/// 표시를 껐으면 글자를 없애고, 켰는데 값이 없으면 "–" 를 보여준다.
pub fn title(percent: Option<f64>, show: bool) -> Option<String> {
    if !show {
        return None;
    }
    Some(match percent {
        Some(p) => format!("{}%", p.round() as i64),
        None => "–".to_string(),
    })
}

pub fn template_icon() -> Image<'static> {
    tauri::include_image!("icons/tray.png")
}

/// 템플릿 아이콘의 모양은 그대로 두고 한 가지 색으로 칠한다.
fn tinted_icon(color: [u8; 3]) -> Image<'static> {
    let icon = template_icon();
    let mut rgba = icon.rgba().to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[..3].copy_from_slice(&color);
    }
    Image::new_owned(rgba, icon.width(), icon.height())
}

/// 메뉴 막대 글자를 시스템 기본 크기보다 이만큼 작게 한다.
#[cfg(target_os = "macos")]
const TITLE_FONT_SHRINK_PT: f64 = 2.0;

/// 사용률 글자의 크기를 줄인다. 트레이 아이콘을 만든 직후 한 번 부른다.
#[cfg(target_os = "macos")]
pub fn shrink_title_font(tray: &tauri::tray::TrayIcon) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSFont;

    let _ = tray.with_inner_tray_icon(|inner| {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(button) = inner.ns_status_item().and_then(|item| item.button(mtm)) else {
            return;
        };
        let font = button
            .font()
            .unwrap_or_else(|| NSFont::menuBarFontOfSize(0.0));
        let smaller = font.fontWithSize(font.pointSize() - TITLE_FONT_SHRINK_PT);
        button.setFont(Some(&smaller));
    });
}

#[cfg(not(target_os = "macos"))]
pub fn shrink_title_font(_tray: &tauri::tray::TrayIcon) {}

/// 마지막으로 적용한 경고 단계. 단계가 바뀔 때만 아이콘을 갈아 끼운다.
#[derive(Default)]
pub struct MenuBarState {
    severity: Mutex<Option<Severity>>,
}

pub fn update(app: &AppHandle, reports: &[ProviderReport]) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let percent = primary_percent(reports);
    let show = app.state::<SettingsStore>().get().show_percent_in_menu_bar;
    let _ = tray.set_title(title(percent, show));

    let next = severity(percent);
    let state = app.state::<MenuBarState>();
    let mut applied = state.severity.lock().unwrap();
    if *applied == Some(next) {
        return;
    }
    // 평소에는 메뉴 막대 밝기를 따라가는 템플릿 아이콘, 경고일 때는 색을 입힌 아이콘
    let (icon, as_template) = match next {
        Severity::Normal => (template_icon(), true),
        Severity::Warning => (tinted_icon(WARNING_COLOR), false),
        Severity::Critical => (tinted_icon(CRITICAL_COLOR), false),
    };
    let _ = tray.set_icon(Some(icon));
    let _ = tray.set_icon_as_template(as_template);
    *applied = Some(next);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_thresholds() {
        assert_eq!(severity(None), Severity::Normal);
        assert_eq!(severity(Some(79.9)), Severity::Normal);
        assert_eq!(severity(Some(80.0)), Severity::Warning);
        assert_eq!(severity(Some(94.9)), Severity::Warning);
        assert_eq!(severity(Some(95.0)), Severity::Critical);
        assert_eq!(severity(Some(100.0)), Severity::Critical);
    }

    #[test]
    fn title_follows_setting_and_value() {
        assert_eq!(title(Some(12.6), true).as_deref(), Some("13%"));
        assert_eq!(title(Some(0.0), true).as_deref(), Some("0%"));
        assert_eq!(title(None, true).as_deref(), Some("–"));
        assert_eq!(title(Some(50.0), false), None);
        assert_eq!(title(None, false), None);
    }

    #[test]
    fn tinted_icon_keeps_shape() {
        let base = template_icon();
        let tinted = tinted_icon(WARNING_COLOR);
        assert_eq!((tinted.width(), tinted.height()), (base.width(), base.height()));
        for (a, b) in base.rgba().chunks_exact(4).zip(tinted.rgba().chunks_exact(4)) {
            assert_eq!(a[3], b[3]);
            assert_eq!(&b[..3], &WARNING_COLOR);
        }
    }
}
