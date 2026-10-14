//! 메뉴 막대 아이콘 옆의 사용률 글자와, 사용률에 따라 낮에서 노을로 물드는 아이콘.

use std::sync::Mutex;

use tauri::image::Image;
use tauri::{AppHandle, Manager};

use crate::providers::ProviderReport;
use crate::settings::SettingsStore;

pub const TRAY_ID: &str = "main";

/// 낮(노을 없음)의 바위: 햇빛 받은 화강암 꼭대기와 푸른 그늘이 진 아래쪽
const DAY_TOP: [u8; 3] = [214, 184, 150];
const DAY_BOTTOM: [u8; 3] = [104, 116, 134];

/// 이 사용률까지는 낮이고, 노을 시작까지 낮에서 노을로 넘어간다.
const DAY_UNTIL_PERCENT: f64 = 40.0;
const GLOW_START_PERCENT: f64 = 50.0;

/// 사용률별 노을 색. 사이 값은 이웃한 두 색을 섞는다.
const GLOW_STOPS: &[(f64, [u8; 3])] = &[
    (50.0, [240, 150, 0]),
    (60.0, [236, 118, 0]),
    (70.0, [230, 90, 0]),
    (80.0, [218, 64, 0]),
    (90.0, [200, 44, 0]),
    (95.0, [184, 34, 0]),
    (100.0, [168, 26, 0]),
];

/// 노을 바위의 꼭대기는 노을 색보다 밝게, 바닥은 어둡게 칠한다.
const TOP_LIGHT: f64 = 1.15;
const BOTTOM_SHADE: f64 = 0.7;

/// 봉우리 안쪽에서 능선을 나누는 틈에 채우는 눈 색
const SNOW: [u8; 3] = [255, 255, 255];

/// 틈 아래 바위 턱에 눈을 이만큼(픽셀) 더 쌓는다.
const SNOW_DEPTH: usize = 2;

/// 틈으로 보려면 좌우나 위아래 양쪽 모두 이 거리(픽셀) 안에 봉우리가 있어야 한다.
/// 봉우리 사이의 하늘이나 바깥 윤곽의 가장자리를 걸러낸다.
const RIDGE_GAP_REACH: usize = 4;

/// 이보다 불투명한 픽셀부터 봉우리로 본다.
const RIDGE_ALPHA: u8 = 40;

/// 아이콘을 칠할 단계(1% 단위). 사용률을 모르면 None이고, 그때는 기본 아이콘을 쓴다.
pub fn icon_level(percent: Option<f64>) -> Option<u8> {
    Some(percent?.round().clamp(0.0, 100.0) as u8)
}

fn glow_color(level: u8) -> [u8; 3] {
    let percent = f64::from(level);
    let mut color = GLOW_STOPS[0].1;
    for pair in GLOW_STOPS.windows(2) {
        let ((from, a), (to, b)) = (pair[0], pair[1]);
        if percent < from {
            break;
        }
        let t = ((percent - from) / (to - from)).min(1.0);
        color = mix_color(a, b, t);
    }
    color
}

fn mix(a: u8, b: u8, t: f64) -> u8 {
    (f64::from(a) * (1.0 - t) + f64::from(b) * t).round() as u8
}

fn mix_color(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    [0, 1, 2].map(|i| mix(a[i], b[i], t))
}

fn scale(color: [u8; 3], factor: f64) -> [u8; 3] {
    color.map(|c| (f64::from(c) * factor).round().min(255.0) as u8)
}

/// 사용률에 따른 바위의 꼭대기 색과 바닥 색
fn rock_colors(level: u8) -> ([u8; 3], [u8; 3]) {
    let glow = glow_color(level);
    let (glow_top, glow_bottom) = (scale(glow, TOP_LIGHT), scale(glow, BOTTOM_SHADE));
    let t = ((f64::from(level) - DAY_UNTIL_PERCENT) / (GLOW_START_PERCENT - DAY_UNTIL_PERCENT))
        .clamp(0.0, 1.0);
    (
        mix_color(DAY_TOP, glow_top, t),
        mix_color(DAY_BOTTOM, glow_bottom, t),
    )
}

/// 메뉴 막대에 띄울 값: 현재 세션 사용률
pub fn primary_percent(reports: &[ProviderReport]) -> Option<f64> {
    reports
        .iter()
        .find_map(|report| report.usage.session_percent)
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

/// 템플릿 아이콘의 모양에 바위 색을 칠한다. 봉우리마다 꼭대기는 밝고 아래로 갈수록 어둡다.
/// 원래 아이콘에서 능선을 나누던 틈은 흰 눈으로 채우고, 그 아래 바위 턱에 눈을 조금 더 쌓는다.
fn painted_icon(level: u8) -> Image<'static> {
    let icon = template_icon();
    let (width, height) = (icon.width() as usize, icon.height() as usize);
    let mut rgba = icon.rgba().to_vec();
    let (top_color, bottom_color) = rock_colors(level);

    let solid: Vec<bool> = rgba
        .chunks_exact(4)
        .map(|pixel| pixel[3] > RIDGE_ALPHA)
        .collect();
    let solid_in_row = |y: usize, from: usize, to: usize| (from..to).any(|x| solid[y * width + x]);
    let solid_in_column =
        |x: usize, from: usize, to: usize| (from..to).any(|y| solid[y * width + x]);
    let mut snowy = vec![false; width * height];

    for x in 0..width {
        let ridge = (0..height).find(|&y| solid[y * width + x]);
        for y in 0..height {
            let t = ridge.map_or(0.0, |ridge| {
                (y.saturating_sub(ridge) as f64 / (height - 1 - ridge).max(1) as f64).min(1.0)
            });
            let pixel = &mut rgba[(y * width + x) * 4..][..4];
            pixel[..3].copy_from_slice(&mix_color(top_color, bottom_color, t));

            // 능선 아래의 얇은 틈만 고른다: 좌우가 가까이 막혔거나 위아래가 가까이 막힌 곳
            let reach = RIDGE_GAP_REACH;
            let gap = ridge.is_some_and(|ridge| ridge < y)
                && ((solid_in_row(y, x.saturating_sub(reach), x)
                    && solid_in_row(y, x + 1, (x + 1 + reach).min(width)))
                    || (solid_in_column(x, y.saturating_sub(reach), y)
                        && solid_in_column(x, y + 1, (y + 1 + reach).min(height))));
            if gap && pixel[3] < 255 {
                let opacity = f64::from(pixel[3]) / 255.0;
                let rock = [pixel[0], pixel[1], pixel[2]];
                pixel[..3].copy_from_slice(&mix_color(SNOW, rock, opacity));
                pixel[3] = 255;
                snowy[y * width + x] = true;
            }
        }
    }

    // 원래 비어 있던 틈에서만 아래로 쌓는다. 윤곽의 흐릿한 가장자리에서 쌓으면 테두리가 하얗게 된다.
    let away_from_outline = |x: usize, y: usize| {
        (y.saturating_sub(1)..(y + 2).min(height)).all(|yy| {
            (x.saturating_sub(1)..(x + 2).min(width))
                .all(|xx| solid[yy * width + xx] || snowy[yy * width + xx])
        })
    };
    let mut piled = Vec::new();
    for x in 0..width {
        for y in 0..height {
            if !snowy[y * width + x] || solid[y * width + x] {
                continue;
            }
            for below in y + 1..(y + 1 + SNOW_DEPTH).min(height) {
                if solid[below * width + x]
                    && !snowy[below * width + x]
                    && away_from_outline(x, below)
                {
                    piled.push(below * width + x);
                }
            }
        }
    }
    for i in piled {
        rgba[i * 4..i * 4 + 3].copy_from_slice(&SNOW);
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

/// 마지막으로 적용한 아이콘 단계. 단계가 바뀔 때만 아이콘을 갈아 끼운다.
#[derive(Default)]
pub struct MenuBarState {
    level: Mutex<Option<Option<u8>>>,
}

pub fn update(app: &AppHandle, reports: &[ProviderReport]) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let percent = primary_percent(reports);
    let show = app.state::<SettingsStore>().get().show_percent_in_menu_bar;
    let _ = tray.set_title(title(percent, show));

    let next = icon_level(percent);
    let state = app.state::<MenuBarState>();
    let mut applied = state.level.lock().unwrap();
    if *applied == Some(next) {
        return;
    }
    // 값을 모를 때는 메뉴 막대 밝기를 따라가는 템플릿 아이콘, 알면 색을 칠한 아이콘
    let (icon, as_template) = match next {
        Some(level) => (painted_icon(level), false),
        None => (template_icon(), true),
    };
    let _ = tray.set_icon(Some(icon));
    let _ = tray.set_icon_as_template(as_template);
    *applied = Some(next);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_level_rounds_and_clamps() {
        assert_eq!(icon_level(None), None);
        assert_eq!(icon_level(Some(0.0)), Some(0));
        assert_eq!(icon_level(Some(84.6)), Some(85));
        assert_eq!(icon_level(Some(120.0)), Some(100));
    }

    #[test]
    fn glow_color_follows_stops() {
        assert_eq!(glow_color(50), [240, 150, 0]);
        assert_eq!(glow_color(55), [238, 134, 0]);
        assert_eq!(glow_color(80), [218, 64, 0]);
        assert_eq!(glow_color(100), [168, 26, 0]);
    }

    #[test]
    fn rock_turns_from_day_to_sunset() {
        assert_eq!(rock_colors(0), (DAY_TOP, DAY_BOTTOM));
        assert_eq!(rock_colors(40), (DAY_TOP, DAY_BOTTOM));
        let glow = glow_color(50);
        assert_eq!(
            rock_colors(50),
            (scale(glow, TOP_LIGHT), scale(glow, BOTTOM_SHADE))
        );
        let (top, _) = rock_colors(45);
        assert_eq!(top, mix_color(DAY_TOP, scale(glow, TOP_LIGHT), 0.5));
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
    fn painted_icon_keeps_outline() {
        let base = template_icon();
        let (width, height) = (base.width() as usize, base.height() as usize);
        let icon = painted_icon(100);
        assert_eq!((icon.width(), icon.height()), (base.width(), base.height()));
        // 봉우리 안쪽 틈만 메워지므로 불투명도는 그대로이거나 꽉 찬다.
        for (a, b) in base.rgba().chunks_exact(4).zip(icon.rgba().chunks_exact(4)) {
            assert!(b[3] == a[3] || b[3] == 255);
        }

        // 가장 높은 봉우리의 꼭대기는 그 단계의 꼭대기 색
        let alpha = |x: usize, y: usize| base.rgba()[(y * width + x) * 4 + 3];
        let (x, top) = (0..width)
            .filter_map(|x| {
                (0..height)
                    .find(|&y| alpha(x, y) > RIDGE_ALPHA)
                    .map(|y| (x, y))
            })
            .min_by_key(|&(_, y)| y)
            .unwrap();
        let i = (top * width + x) * 4;
        assert_eq!(&icon.rgba()[i..i + 3], &rock_colors(100).0);
    }
}
