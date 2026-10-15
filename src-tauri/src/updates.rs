//! 새 버전 알림.
//!
//! GitHub Releases 의 최신 릴리스 버전을 지금 버전과 비교한다. 설치는 하지 않고, 새 버전이 있으면
//! 창에 알려서 사용자가 릴리스 페이지에서 직접 받게 한다. 로그인 정보는 보내지 않는다.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::providers::now_ms;
use crate::settings::SettingsStore;

const LATEST_URL: &str = "https://api.github.com/repos/honeydragoneye/fitzroy/releases/latest";
/// 릴리스 페이지 주소는 이 저장소 안의 것만 연다.
const RELEASES_PAGE_PREFIX: &str = "https://github.com/honeydragoneye/fitzroy/releases/";
const USER_AGENT: &str = concat!("fitzroy/", env!("CARGO_PKG_VERSION"));
const TIMEOUT: Duration = Duration::from_secs(10);

/// 자동 확인 간격
const CHECK_INTERVAL_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// 확인할 때가 됐는지 이 간격으로 살펴본다. 요청은 `CHECK_INTERVAL_MS` 마다만 보낸다.
const WAKE_INTERVAL: Duration = Duration::from_secs(60 * 60);

const STATE_FILE: &str = "updates.json";

/// 새 버전이 있는지 창으로 보내는 이벤트 이름. 값은 `Option<Release>`.
pub const UPDATE_AVAILABLE: &str = "update-available";
/// 메뉴에서 직접 확인했을 때 진행 상황을 보내는 이벤트 이름. 값은 `CheckStatus`.
pub const UPDATE_STATUS: &str = "update-status";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Release {
    /// 앞의 "v" 를 뗀 버전. 예: "0.2.0"
    pub version: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum CheckStatus {
    Checking,
    UpToDate,
    Available { version: String },
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CheckError {
    Network,
    UnexpectedResponse,
}

/// "v1.2.3" 이나 "1.2.3" 을 숫자 세 개로 읽는다. "-beta" 같은 꼬리가 붙은 버전은 읽지 않는다.
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.strip_prefix('v').unwrap_or(text);
    let mut parts = text.split('.').map(|part| part.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: Option<String>,
    html_url: Option<String>,
}

fn parse_release(json: &str) -> Result<Release, CheckError> {
    let raw: RawRelease = serde_json::from_str(json).map_err(|_| CheckError::UnexpectedResponse)?;
    let tag = raw.tag_name.ok_or(CheckError::UnexpectedResponse)?;
    parse_version(&tag).ok_or(CheckError::UnexpectedResponse)?;
    let url = raw
        .html_url
        .filter(|url| url.starts_with(RELEASES_PAGE_PREFIX))
        .ok_or(CheckError::UnexpectedResponse)?;
    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        url,
    })
}

/// 최신 릴리스. 릴리스가 하나도 없으면 GitHub 가 404 를 주므로 None.
fn fetch_latest() -> Result<Option<Release>, CheckError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();

    let mut response = match agent
        .get(LATEST_URL)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", USER_AGENT)
        .call()
    {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(ureq::Error::StatusCode(_)) => return Err(CheckError::UnexpectedResponse),
        Err(_) => return Err(CheckError::Network),
    };
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|_| CheckError::Network)?;
    parse_release(&body).map(Some)
}

/// 파일로 저장해서 앱을 자주 다시 켜도 일주일에 한 번만 확인한다.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct State {
    last_checked_at: Option<i64>,
    latest: Option<Release>,
}

impl State {
    fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string(self) {
            let _ = fs::write(path, json);
        }
    }

    fn is_due(&self, now_ms: i64) -> bool {
        // 시계가 뒤로 바뀌어 마지막 확인 시각이 미래가 됐으면 다시 확인한다.
        self.last_checked_at
            .is_none_or(|at| now_ms - at >= CHECK_INTERVAL_MS || at > now_ms)
    }

    /// 지금 버전보다 새 릴리스. 이미 그 버전 이상으로 업데이트했으면 None.
    fn available(&self, current: &str) -> Option<Release> {
        self.latest
            .clone()
            .filter(|release| is_newer(&release.version, current))
    }
}

pub struct Updates {
    current_version: String,
    path: Option<PathBuf>,
    state: Mutex<State>,
    /// 확인 스레드를 깨운다. true 면 메뉴에서 직접 요청한 확인이다.
    wake: Sender<bool>,
}

impl Updates {
    fn new(current_version: String, cache_dir: Option<PathBuf>) -> (Self, Receiver<bool>) {
        let path = cache_dir.map(|dir| dir.join(STATE_FILE));
        let state = path.as_deref().map(State::load).unwrap_or_default();
        let (wake, wake_rx) = mpsc::channel();
        let updates = Updates {
            current_version,
            path,
            state: Mutex::new(state),
            wake,
        };
        (updates, wake_rx)
    }

    pub fn available(&self) -> Option<Release> {
        self.state.lock().unwrap().available(&self.current_version)
    }

    /// 자동 확인 설정과 상관없이 바로 확인하고 결과를 `update-status` 로 알린다.
    pub fn check_now(&self) {
        let _ = self.wake.send(true);
    }

    /// 자동 확인할 때가 지났는지 바로 다시 살펴보게 한다.
    pub fn wake(&self) {
        let _ = self.wake.send(false);
    }

    /// 확인해서 결과를 저장한다. 실패하면 마지막으로 알던 릴리스를 그대로 둔다.
    fn check(&self) -> Result<Option<Release>, CheckError> {
        let result = fetch_latest();
        let mut state = self.state.lock().unwrap();
        state.last_checked_at = Some(now_ms());
        if let Ok(latest) = &result {
            state.latest = latest.clone();
        }
        if let Some(path) = &self.path {
            state.save(path);
        }
        result.map(|_| state.available(&self.current_version))
    }

    fn is_due(&self) -> bool {
        self.state.lock().unwrap().is_due(now_ms())
    }
}

/// 확인 스레드를 시작한다. `SettingsStore` 가 등록된 뒤에 불러야 한다.
pub fn spawn(app: &AppHandle, cache_dir: Option<PathBuf>) {
    let current = app.package_info().version.to_string();
    let (updates, wake_rx) = Updates::new(current, cache_dir);
    app.manage(updates);

    let app = app.clone();
    thread::spawn(move || {
        let mut manual = false;
        loop {
            let updates = app.state::<Updates>();
            let auto = app.state::<SettingsStore>().get().check_for_updates;
            if manual || (auto && updates.is_due()) {
                if manual {
                    let _ = app.emit(UPDATE_STATUS, CheckStatus::Checking);
                }
                let result = updates.check();
                if manual {
                    let status = match &result {
                        Ok(Some(release)) => CheckStatus::Available {
                            version: release.version.clone(),
                        },
                        Ok(None) => CheckStatus::UpToDate,
                        Err(_) => CheckStatus::Failed,
                    };
                    let _ = app.emit(UPDATE_STATUS, status);
                }
                let _ = app.emit(UPDATE_AVAILABLE, updates.available());
            }

            manual = match wake_rx.recv_timeout(WAKE_INTERVAL) {
                Ok(manual) => manual,
                Err(RecvTimeoutError::Timeout) => false,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            // 기다리는 동안 쌓인 요청은 한 번으로 친다. 직접 요청이 하나라도 있으면 직접 확인이다.
            loop {
                match wake_rx.try_recv() {
                    Ok(more) => manual |= more,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_by_number() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.0"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.2.0"));
        assert!(!is_newer("0.2.0-beta", "0.1.0"));
        assert!(!is_newer("0.2", "0.1.0"));
        assert!(!is_newer("0.2.0.1", "0.1.0"));
    }

    #[test]
    fn parses_release_response() {
        let json = r#"{"tag_name":"v0.2.0","name":"Fitzroy 0.2.0","draft":false,"prerelease":false,
            "html_url":"https://github.com/honeydragoneye/fitzroy/releases/tag/v0.2.0","assets":[]}"#;
        assert_eq!(
            parse_release(json),
            Ok(Release {
                version: "0.2.0".to_string(),
                url: "https://github.com/honeydragoneye/fitzroy/releases/tag/v0.2.0".to_string(),
            })
        );
    }

    #[test]
    fn rejects_unexpected_release_response() {
        let other_site = r#"{"tag_name":"v0.2.0","html_url":"https://example.com/releases/v0.2.0"}"#;
        let bad_tag = r#"{"tag_name":"latest","html_url":"https://github.com/honeydragoneye/fitzroy/releases/tag/latest"}"#;
        for json in [other_site, bad_tag, r#"{"message":"Not Found"}"#, "not json"] {
            assert_eq!(parse_release(json), Err(CheckError::UnexpectedResponse), "{json}");
        }
    }

    #[test]
    fn checks_once_a_week() {
        let mut state = State::default();
        assert!(state.is_due(1_000));
        state.last_checked_at = Some(1_000);
        assert!(!state.is_due(1_000 + CHECK_INTERVAL_MS - 1));
        assert!(state.is_due(1_000 + CHECK_INTERVAL_MS));
        assert!(state.is_due(999));
    }

    #[test]
    fn available_only_when_newer_than_running_app() {
        let state = State {
            last_checked_at: Some(1),
            latest: Some(Release {
                version: "0.2.0".to_string(),
                url: format!("{RELEASES_PAGE_PREFIX}tag/v0.2.0"),
            }),
        };
        assert_eq!(state.available("0.1.0").unwrap().version, "0.2.0");
        assert_eq!(state.available("0.2.0"), None);
    }

    #[test]
    fn state_survives_restart() {
        let dir = std::env::temp_dir().join(format!("fitzroy-updates-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join(STATE_FILE);
        assert_eq!(State::load(&path), State::default());

        let state = State {
            last_checked_at: Some(42),
            latest: None,
        };
        state.save(&path);
        assert_eq!(State::load(&path), state);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// 실제 GitHub 응답을 출력한다. `cargo test real_latest_release -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_latest_release() {
        println!("{:?}", fetch_latest());
    }
}
