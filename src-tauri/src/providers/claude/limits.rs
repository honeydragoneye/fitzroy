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
use serde::Deserialize;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const USER_AGENT: &str = concat!("fitzroy/", env!("CARGO_PKG_VERSION"));
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Limits {
    pub session_percent: Option<f64>,
    pub session_resets_at: Option<i64>,
    pub weekly_percent: Option<f64>,
    pub weekly_resets_at: Option<i64>,
}

#[derive(Debug, PartialEq)]
pub enum LimitsError {
    NotLoggedIn,
    TokenExpired,
    RateLimited,
    Network,
    UnexpectedResponse,
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
