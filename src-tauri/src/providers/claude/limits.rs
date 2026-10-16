//! Claude 세션/주간 한도 사용률.
//!
//! Claude Code 가 저장해 둔 OAuth 토큰으로 사용량 API 를 호출한다. 비공식 API 라서
//! 언제든 바뀔 수 있고, 응답에서 필요한 값만 느슨하게 읽는다.
//!
//! 토큰은 Anthropic API 서버에만 보낸다. 로그나 에러 문구에 넣지 않는다.

use std::fmt;
use std::fs;
use std::path::Path;
use std::time::Duration;

use chrono::DateTime;
use serde::{Deserialize, Serialize};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const USER_AGENT: &str = concat!("fitzroy/", env!("CARGO_PKG_VERSION"));
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub session_percent: Option<f64>,
    pub session_resets_at: Option<i64>,
    pub weekly_percent: Option<f64>,
    pub weekly_resets_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LimitsError {
    NotLoggedIn,
    TokenExpired,
    RateLimited,
    Network,
    UnexpectedResponse,
}

impl Limits {
    /// 재설정 시각이 지난 값은 더 이상 맞지 않으므로 비운다.
    pub fn without_expired(mut self, now_ms: i64) -> Self {
        if self.session_resets_at.is_some_and(|at| at <= now_ms) {
            self.session_percent = None;
            self.session_resets_at = None;
        }
        if self.weekly_resets_at.is_some_and(|at| at <= now_ms) {
            self.weekly_percent = None;
            self.weekly_resets_at = None;
        }
        self
    }
}

/// 가장 최근에 지난 (세션, 주간) 재설정 시각
pub type PassedResets = (Option<i64>, Option<i64>);

/// 갱신 요청이 몰려도 한도 API 는 이 간격보다 자주 부르지 않는다.
const MIN_INTERVAL_MS: i64 = 30_000;

/// 한도 조회의 진행 상태. 파일로 저장해서 앱을 다시 켜도 마지막 값과 대기 시간이 이어진다.
/// 앱을 켤 때마다 API 를 부르면 금방 요청이 거부되기 때문이다.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LimitsState {
    pub last: Option<Limits>,
    /// 이 시각(Unix 밀리초) 전에는 API 를 부르지 않는다.
    next_attempt_at: i64,
    rate_limited_in_a_row: u32,
    /// 가장 최근에 지난 재설정 시각. 재설정 뒤 사용률이 0이면 API 가 재설정 시각을 주지 않아서,
    /// 전에 받아 둔 시각이 지나면 여기에 남겨 둔다.
    session_reset_at: Option<i64>,
    weekly_reset_at: Option<i64>,
    #[serde(skip)]
    pub last_error: Option<LimitsError>,
}

impl LimitsState {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string(self) {
            let _ = fs::write(path, json);
        }
    }

    pub fn should_attempt(&self, now_ms: i64) -> bool {
        // 시계가 뒤로 바뀌어 대기 시간이 터무니없이 길어졌으면 무시한다.
        let longest_wait = rate_limit_delay(u32::MAX).as_millis() as i64;
        now_ms >= self.next_attempt_at || self.next_attempt_at - now_ms > longest_wait
    }

    /// 가장 최근에 지난 (세션, 주간) 재설정 시각.
    pub fn passed_resets(&self, now_ms: i64) -> PassedResets {
        let passed = |at: Option<i64>| at.filter(|&at| at <= now_ms);
        let last = self.last.unwrap_or_default();
        (
            passed(last.session_resets_at).or(self.session_reset_at),
            passed(last.weekly_resets_at).or(self.weekly_reset_at),
        )
    }

    pub fn record(&mut self, result: Result<Limits, LimitsError>, now_ms: i64) {
        (self.session_reset_at, self.weekly_reset_at) = self.passed_resets(now_ms);
        let mut wait_ms = MIN_INTERVAL_MS;
        match result {
            Ok(limits) => {
                self.last = Some(limits);
                self.last_error = None;
                self.rate_limited_in_a_row = 0;
            }
            Err(error) => {
                self.last_error = Some(error);
                if error == LimitsError::RateLimited {
                    self.rate_limited_in_a_row += 1;
                    wait_ms = rate_limit_delay(self.rate_limited_in_a_row).as_millis() as i64;
                }
            }
        }
        self.next_attempt_at = now_ms + wait_ms;
    }
}

/// 요청이 거부(429)됐을 때 다음 시도까지 기다릴 시간. 거부가 이어질수록 2분, 4분, 8분, 최대 10분.
pub fn rate_limit_delay(consecutive: u32) -> Duration {
    const BASE: Duration = Duration::from_secs(120);
    const MAX: Duration = Duration::from_secs(600);
    BASE.saturating_mul(1 << consecutive.saturating_sub(1).min(8)).min(MAX)
}

impl fmt::Display for LimitsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LimitsError::NotLoggedIn => "Claude Code 로그인 정보를 찾지 못했습니다",
            LimitsError::TokenExpired => {
                "로그인 토큰이 만료됐습니다. Claude Code 를 실행하면 갱신됩니다"
            }
            LimitsError::RateLimited => "한도 조회 요청이 너무 많습니다. 잠시 뒤 다시 시도합니다",
            LimitsError::Network => "한도 정보를 가져오지 못했습니다 (네트워크)",
            LimitsError::UnexpectedResponse => "한도 정보의 형식이 예상과 다릅니다",
        })
    }
}

#[derive(Deserialize)]
struct CredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<OAuthCredentials>,
}

#[derive(Deserialize)]
struct OAuthCredentials {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
}

// 토큰이 실수로 출력되지 않도록 Debug 를 구현하지 않는다.
pub struct AccessToken(String);

fn parse_credentials(json: &str, now_ms: i64) -> Result<AccessToken, LimitsError> {
    let oauth = serde_json::from_str::<CredentialsFile>(json)
        .ok()
        .and_then(|file| file.claude_ai_oauth)
        .filter(|oauth| !oauth.access_token.is_empty())
        .ok_or(LimitsError::NotLoggedIn)?;

    // 토큰 갱신은 Claude Code 에 맡긴다. 여기서 갱신하면 Claude Code 쪽 로그인이 풀릴 수 있다.
    if oauth.expires_at.is_some_and(|expires_at| expires_at <= now_ms) {
        return Err(LimitsError::TokenExpired);
    }
    Ok(AccessToken(oauth.access_token))
}

#[derive(Deserialize)]
struct PlanFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<PlanOAuth>,
}

#[derive(Deserialize)]
struct PlanOAuth {
    #[serde(rename = "subscriptionType")]
    subscription_type: Option<String>,
}

/// 로그인 정보의 `subscriptionType`("pro", "max" 등)을 화면에 쓸 이름으로 바꾼다.
fn parse_plan(json: &str) -> Option<String> {
    let raw = serde_json::from_str::<PlanFile>(json)
        .ok()?
        .claude_ai_oauth?
        .subscription_type?;
    crate::providers::plan_name(&raw)
}

const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

/// macOS Keychain 에 저장된 자격 증명 JSON. 파일과 같은 형식이다.
/// Claude Code 와 같은 방식(`security` 명령)으로 읽어서 접근 허용 창이 뜨지 않게 한다.
#[cfg(target_os = "macos")]
fn read_keychain() -> Option<String> {
    use std::process::{Command, Stdio};

    let output = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

#[cfg(not(target_os = "macos"))]
fn read_keychain() -> Option<String> {
    None
}

/// 파일의 토큰을 먼저 쓰고, 없거나 만료됐으면 Keychain 을 본다.
/// 둘 다 못 쓰면 "만료"를 "로그인 정보 없음"보다 우선해서 알린다.
fn first_usable(
    file: Result<AccessToken, LimitsError>,
    keychain: impl FnOnce() -> Result<AccessToken, LimitsError>,
) -> Result<AccessToken, LimitsError> {
    let file_error = match file {
        Ok(token) => return Ok(token),
        Err(error) => error,
    };
    match keychain() {
        Ok(token) => Ok(token),
        Err(LimitsError::TokenExpired) => Err(LimitsError::TokenExpired),
        Err(_) => Err(file_error),
    }
}

/// `<설정 폴더>/.credentials.json` 또는 Keychain 에서 토큰을 읽는다.
/// Keychain 항목 이름은 기본 설정 폴더일 때만 확인된 것이라, 그때만 `use_keychain` 을 켠다.
pub fn read_access_token(
    config_dir: &Path,
    use_keychain: bool,
    now_ms: i64,
) -> Result<AccessToken, LimitsError> {
    let file = fs::read_to_string(config_dir.join(".credentials.json"))
        .map_err(|_| LimitsError::NotLoggedIn)
        .and_then(|json| parse_credentials(&json, now_ms));

    first_usable(file, || {
        if !use_keychain {
            return Err(LimitsError::NotLoggedIn);
        }
        let json = read_keychain().ok_or(LimitsError::NotLoggedIn)?;
        parse_credentials(&json, now_ms)
    })
}

/// 로그인 정보에서 구독 플랜 이름을 읽는다. 토큰은 읽지 않는다.
/// 만료 여부와 상관없이 파일을 먼저 보고, 없으면 Keychain 을 본다.
pub fn read_plan(config_dir: &Path, use_keychain: bool) -> Option<String> {
    fs::read_to_string(config_dir.join(".credentials.json"))
        .ok()
        .and_then(|json| parse_plan(&json))
        .or_else(|| {
            if use_keychain {
                read_keychain().and_then(|json| parse_plan(&json))
            } else {
                None
            }
        })
}

#[derive(Deserialize)]
struct UsageResponse {
    five_hour: Option<Window>,
    seven_day: Option<Window>,
}

#[derive(Deserialize)]
struct Window {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

/// 재설정 시각이 "…:29:59.597" 처럼 정각 직전으로 오므로 가까운 분으로 맞춘다.
fn round_to_minute(ms: i64) -> i64 {
    const MINUTE_MS: i64 = 60_000;
    (ms + MINUTE_MS / 2).div_euclid(MINUTE_MS) * MINUTE_MS
}

fn window_values(window: Option<Window>) -> (Option<f64>, Option<i64>) {
    let Some(window) = window else {
        return (None, None);
    };
    let resets_at = window
        .resets_at
        .and_then(|text| DateTime::parse_from_rfc3339(&text).ok())
        .map(|time| round_to_minute(time.timestamp_millis()));
    (window.utilization, resets_at)
}

fn parse_usage(json: &str) -> Result<Limits, LimitsError> {
    let response: UsageResponse =
        serde_json::from_str(json).map_err(|_| LimitsError::UnexpectedResponse)?;
    let (session_percent, session_resets_at) = window_values(response.five_hour);
    let (weekly_percent, weekly_resets_at) = window_values(response.seven_day);

    if session_percent.is_none() && weekly_percent.is_none() {
        return Err(LimitsError::UnexpectedResponse);
    }
    Ok(Limits {
        session_percent,
        session_resets_at,
        weekly_percent,
        weekly_resets_at,
    })
}

pub fn fetch(token: &AccessToken) -> Result<Limits, LimitsError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();

    let mut response = agent
        .get(USAGE_URL)
        .header("Authorization", format!("Bearer {}", token.0))
        .header("anthropic-beta", OAUTH_BETA)
        .header("Accept", "application/json")
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|error| match error {
            ureq::Error::StatusCode(401 | 403) => LimitsError::TokenExpired,
            ureq::Error::StatusCode(429) => LimitsError::RateLimited,
            ureq::Error::StatusCode(_) => LimitsError::UnexpectedResponse,
            _ => LimitsError::Network,
        })?;

    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|_| LimitsError::Network)?;
    parse_usage(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plan_name() {
        let json = |plan: &str| format!(r#"{{"claudeAiOauth":{{"accessToken":"t","subscriptionType":"{plan}"}}}}"#);
        assert_eq!(parse_plan(&json("pro")).as_deref(), Some("Pro"));
        assert_eq!(parse_plan(&json("max")).as_deref(), Some("Max"));
        assert_eq!(parse_plan(&json("")), None);
        assert_eq!(parse_plan(r#"{"claudeAiOauth":{"accessToken":"t"}}"#), None);
        assert_eq!(parse_plan("not json"), None);
    }

    #[test]
    fn parses_usage_response() {
        // 실제 응답에서 필요한 부분과 형식이 다른 주변 필드 일부를 남긴 것
        let json = r#"{
            "five_hour": {"utilization": 10.0, "resets_at": "2026-10-13T17:29:59.597362+00:00", "limit_dollars": null},
            "seven_day": {"utilization": 17.0, "resets_at": "2026-10-18T15:59:59.597385+00:00"},
            "seven_day_opus": null,
            "extra_usage": {"is_enabled": false},
            "limits": [{"kind": "session", "percent": 10}]
        }"#;

        let limits = parse_usage(json).unwrap();
        assert_eq!(limits.session_percent, Some(10.0));
        assert_eq!(limits.session_resets_at, Some(1_791_912_600_000));
        assert_eq!(limits.weekly_percent, Some(17.0));
        assert_eq!(limits.weekly_resets_at, Some(1_792_339_200_000));
    }

    #[test]
    fn missing_windows_are_optional_but_not_both() {
        let limits = parse_usage(r#"{"five_hour": null, "seven_day": {"utilization": 3}}"#).unwrap();
        assert_eq!(limits.session_percent, None);
        assert_eq!(limits.weekly_percent, Some(3.0));
        assert_eq!(limits.weekly_resets_at, None);

        assert_eq!(parse_usage("{}"), Err(LimitsError::UnexpectedResponse));
        assert_eq!(parse_usage("<html>"), Err(LimitsError::UnexpectedResponse));
    }

    #[test]
    fn reads_token_and_rejects_expired_or_missing() {
        let json = |expires_at: i64| {
            format!(r#"{{"mcpOAuth":{{}},"claudeAiOauth":{{"accessToken":"t","refreshToken":"r","expiresAt":{expires_at},"scopes":[]}}}}"#)
        };
        assert_eq!(parse_credentials(&json(2_000), 1_000).unwrap().0, "t");
        assert!(matches!(parse_credentials(&json(1_000), 1_000), Err(LimitsError::TokenExpired)));
        assert!(matches!(parse_credentials(r#"{"mcpOAuth":{}}"#, 0), Err(LimitsError::NotLoggedIn)));
        assert!(matches!(parse_credentials("", 0), Err(LimitsError::NotLoggedIn)));
    }

    #[test]
    fn falls_back_to_keychain_only_when_file_is_unusable() {
        let token = |text: &str| Ok(AccessToken(text.to_string()));
        let value = |result: Result<AccessToken, LimitsError>| result.map(|t| t.0);

        let unused = || -> Result<AccessToken, LimitsError> { panic!("keychain 을 읽으면 안 된다") };
        assert_eq!(value(first_usable(token("file"), unused)), Ok("file".to_string()));

        assert_eq!(
            value(first_usable(Err(LimitsError::TokenExpired), || token("keychain"))),
            Ok("keychain".to_string())
        );
        assert_eq!(
            value(first_usable(Err(LimitsError::NotLoggedIn), || Err(LimitsError::TokenExpired))),
            Err(LimitsError::TokenExpired)
        );
        assert_eq!(
            value(first_usable(Err(LimitsError::TokenExpired), || Err(LimitsError::NotLoggedIn))),
            Err(LimitsError::TokenExpired)
        );
        assert_eq!(
            value(first_usable(Err(LimitsError::NotLoggedIn), || Err(LimitsError::NotLoggedIn))),
            Err(LimitsError::NotLoggedIn)
        );
    }

    #[test]
    fn expired_windows_are_cleared() {
        let limits = Limits {
            session_percent: Some(40.0),
            session_resets_at: Some(1_000),
            weekly_percent: Some(60.0),
            weekly_resets_at: Some(5_000),
        };
        assert_eq!(limits.without_expired(999), limits);

        let after_session_reset = limits.without_expired(1_000);
        assert_eq!(after_session_reset.session_percent, None);
        assert_eq!(after_session_reset.session_resets_at, None);
        assert_eq!(after_session_reset.weekly_percent, Some(60.0));

        assert_eq!(limits.without_expired(5_000), Limits::default());
    }

    #[test]
    fn remembers_passed_resets_after_new_values() {
        let mut state = LimitsState::default();
        assert_eq!(state.passed_resets(0), (None, None));

        state.record(
            Ok(Limits {
                session_percent: Some(10.0),
                session_resets_at: Some(1_000),
                weekly_percent: Some(20.0),
                weekly_resets_at: Some(9_000),
            }),
            0,
        );
        assert_eq!(state.passed_resets(999), (None, None));
        assert_eq!(state.passed_resets(1_000), (Some(1_000), None));

        // 재설정 뒤 0%가 되면 재설정 시각이 비어서 오지만, 지난 시각은 남는다.
        state.record(
            Ok(Limits {
                session_percent: Some(0.0),
                session_resets_at: None,
                weekly_percent: Some(20.0),
                weekly_resets_at: Some(9_000),
            }),
            2_000,
        );
        assert_eq!(state.passed_resets(2_000), (Some(1_000), None));
        assert_eq!(state.passed_resets(9_000), (Some(1_000), Some(9_000)));
    }

    #[test]
    fn rate_limit_delay_doubles_up_to_ten_minutes() {
        let secs = |n| rate_limit_delay(n).as_secs();
        assert_eq!([secs(1), secs(2), secs(3), secs(4), secs(50)], [120, 240, 480, 600, 600]);
    }

    #[test]
    fn state_throttles_and_backs_off() {
        let ok = Limits {
            session_percent: Some(10.0),
            ..Limits::default()
        };
        let mut state = LimitsState::default();
        assert!(state.should_attempt(0));

        state.record(Ok(ok), 1_000);
        assert_eq!(state.last, Some(ok));
        assert!(!state.should_attempt(30_999));
        assert!(state.should_attempt(31_000));

        // 거부가 이어지면 대기가 길어지고, 마지막으로 받은 값은 남는다.
        state.record(Err(LimitsError::RateLimited), 31_000);
        assert!(!state.should_attempt(31_000 + 119_999));
        assert!(state.should_attempt(31_000 + 120_000));
        state.record(Err(LimitsError::RateLimited), 200_000);
        assert!(!state.should_attempt(200_000 + 239_999));
        assert_eq!(state.last, Some(ok));
        assert_eq!(state.last_error, Some(LimitsError::RateLimited));

        // 다른 에러는 짧게만 쉬고, 성공하면 처음으로 돌아간다.
        state.record(Err(LimitsError::Network), 500_000);
        assert!(state.should_attempt(530_000));
        state.record(Ok(ok), 530_000);
        assert_eq!(state.last_error, None);
        state.record(Err(LimitsError::RateLimited), 600_000);
        assert!(state.should_attempt(600_000 + 120_000));

        // 시계가 크게 뒤로 가면 기다리지 않는다.
        assert!(state.should_attempt(0));
    }

    #[test]
    fn state_survives_restart() {
        let dir = std::env::temp_dir().join(format!("fitzroy-limits-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("cache/claude-limits.json");
        assert_eq!(LimitsState::load(&path), LimitsState::default());

        let mut state = LimitsState::default();
        state.record(
            Ok(Limits {
                session_percent: Some(12.0),
                session_resets_at: Some(9_000_000),
                ..Limits::default()
            }),
            1_000,
        );
        state.record(Err(LimitsError::RateLimited), 40_000);
        state.save(&path);

        let loaded = LimitsState::load(&path);
        assert_eq!(loaded.last, state.last);
        assert!(!loaded.should_attempt(40_000 + 119_999));
        assert!(loaded.should_attempt(40_000 + 120_000));
        // 에러 문구는 저장하지 않는다.
        assert_eq!(loaded.last_error, None);

        fs::write(&path, "broken").unwrap();
        assert_eq!(LimitsState::load(&path), LimitsState::default());
        fs::remove_dir_all(&dir).unwrap();
    }

    /// 실제 토큰으로 API 를 호출해 값을 출력한다. claude.ai 설정 화면과 대조할 때 쓴다.
    /// `cargo test real_limits -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_limits() {
        let config_dir = std::env::home_dir().unwrap().join(".claude");
        let token = read_access_token(&config_dir, true, crate::providers::now_ms()).unwrap();
        println!("{:?}", fetch(&token));
    }
}
