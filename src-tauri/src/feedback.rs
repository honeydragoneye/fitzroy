//! 문제 신고: 사용자의 메일 앱에 양식과 진단 정보를 채운 새 메일을 연다.
//! 앱은 메일을 직접 보내지 않는다. 사용자가 내용을 보고 고친 뒤 직접 보낸다.
//! 진단 정보에는 토큰, 계정, 대화 내용, 사용량 숫자를 넣지 않는다.

use std::process::Command;

use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::providers::{self, ProviderReport};
use crate::refresher::Refresher;
use crate::settings::SettingsStore;

/// 신고를 받을 메일 주소. 앱과 소스에 공개된다.
const REPORT_EMAIL: &str = "myyseooo@gmail.com";

const SUBJECT: &str = "[Fitzroy] 문제 신고 🕊️";

/// 메일 본문에 넣는 진단 정보
#[derive(Debug, Clone, PartialEq)]
struct Diagnostics {
    app_version: String,
    macos_version: Option<String>,
    arch: &'static str,
    page: String,
    refresh_interval_secs: u64,
    /// 서비스 이름, 한도 값이 있는지, 에러 문구
    providers: Vec<(String, bool, Option<String>)>,
}

impl Diagnostics {
    fn collect(app: &AppHandle) -> Self {
        let settings = app.state::<SettingsStore>().get();
        let reports = app.state::<Refresher>().reports();
        Diagnostics {
            app_version: app.package_info().version.to_string(),
            macos_version: macos_version(),
            arch: std::env::consts::ARCH,
            page: settings.page,
            refresh_interval_secs: settings.refresh_interval_secs,
            providers: reports.iter().map(provider_status).collect(),
        }
    }
}

fn provider_status(report: &ProviderReport) -> (String, bool, Option<String>) {
    (
        report.name.to_string(),
        !report.usage.limits.is_empty(),
        report.usage.error.clone(),
    )
}

fn macos_version() -> Option<String> {
    let output = Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    let version = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (output.status.success() && !version.is_empty()).then_some(version)
}

fn interval_label(secs: u64) -> String {
    if secs >= 60 && secs.is_multiple_of(60) {
        format!("{}분", secs / 60)
    } else {
        format!("{secs}초")
    }
}

fn page_name(page: &str) -> &str {
    providers::PAGES
        .iter()
        .find(|(id, _)| *id == page)
        .map_or(page, |(_, name)| name)
}

fn body(info: &Diagnostics) -> String {
    let mut lines = vec![
        "어떤 일이 있었나요?".to_string(),
        "(여기에 적어 주세요)".to_string(),
        String::new(),
        "기대한 동작은 무엇이었나요?".to_string(),
        "(여기에 적어 주세요)".to_string(),
        String::new(),
        "──────── 진단 정보 (자동 입력) ────────".to_string(),
        format!(
            "Fitzroy {} · macOS {} · {}",
            info.app_version,
            info.macos_version.as_deref().unwrap_or("알 수 없음"),
            info.arch
        ),
        format!(
            "보고 있는 서비스: {} · 갱신 주기: {}",
            page_name(&info.page),
            interval_label(info.refresh_interval_secs)
        ),
    ];
    if info.providers.is_empty() {
        lines.push("서비스: 아직 불러오지 않음".to_string());
    }
    for (name, has_limits, error) in &info.providers {
        let limits = if *has_limits { "한도 값 있음" } else { "한도 값 없음" };
        let error = match error {
            Some(error) => format!("에러 \"{error}\""),
            None => "에러 없음".to_string(),
        };
        lines.push(format!("{name}: {limits} · {error}"));
    }
    lines.join("\n")
}

/// mailto 주소에 넣을 수 있게 바꾼다. 영문자, 숫자, `-._~` 외에는 모두 %XX 로 쓴다.
fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.replace('\n', "\r\n").bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn mailto_url(info: &Diagnostics) -> String {
    format!(
        "mailto:{REPORT_EMAIL}?subject={}&body={}",
        encode(SUBJECT),
        encode(&body(info))
    )
}

/// 사용자의 기본 메일 앱에 양식이 채워진 새 메일을 연다.
pub fn open_mail(app: &AppHandle) {
    let url = mailto_url(&Diagnostics::collect(app));
    let _ = app.opener().open_url(url, None::<&str>);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Diagnostics {
        Diagnostics {
            app_version: "0.1.1".into(),
            macos_version: Some("15.6".into()),
            arch: "aarch64",
            page: "claude".into(),
            refresh_interval_secs: 60,
            providers: vec![
                ("Claude".into(), true, None),
                ("Codex".into(), false, Some("아직 한도 정보가 없습니다".into())),
            ],
        }
    }

    #[test]
    fn body_has_form_and_diagnostics() {
        let body = body(&sample());
        assert!(body.starts_with("어떤 일이 있었나요?\n(여기에 적어 주세요)\n\n"));
        assert!(body.contains("Fitzroy 0.1.1 · macOS 15.6 · aarch64"));
        assert!(body.contains("보고 있는 서비스: Claude · 갱신 주기: 1분"));
        assert!(body.contains("Claude: 한도 값 있음 · 에러 없음"));
        assert!(body.ends_with("Codex: 한도 값 없음 · 에러 \"아직 한도 정보가 없습니다\""));
    }

    #[test]
    fn body_without_reports_or_os_version() {
        let info = Diagnostics {
            macos_version: None,
            refresh_interval_secs: 30,
            providers: vec![],
            ..sample()
        };
        let body = body(&info);
        assert!(body.contains("macOS 알 수 없음"));
        assert!(body.contains("갱신 주기: 30초"));
        assert!(body.ends_with("서비스: 아직 불러오지 않음"));
    }

    #[test]
    fn encode_escapes_everything_but_unreserved() {
        assert_eq!(encode("a-Z_0.~"), "a-Z_0.~");
        assert_eq!(encode("a b&c=d?\n"), "a%20b%26c%3Dd%3F%0D%0A");
        assert_eq!(encode("한"), "%ED%95%9C");
    }

    #[test]
    fn mailto_url_round_trips() {
        let url = mailto_url(&sample());
        assert!(url.starts_with(&format!("mailto:{REPORT_EMAIL}?subject=")));
        let (_, body) = url.split_once("&body=").unwrap();
        // 본문에는 다른 쿼리 구분자가 그대로 남지 않는다.
        assert!(!body.contains('&') && !body.contains('=') && !body.contains(' '));
        assert!(body.contains("Fitzroy%200.1.1"));
    }
}
