//! Codex 한도를 ChatGPT 서버에서 직접 읽는다. Codex CLI 가 로그인할 때 남긴 `<CODEX_HOME>/auth.json` 의
//! 토큰으로 `chatgpt.com` 에만 요청한다. 공개되지 않은 주소라 언제든 막힐 수 있어서, 실패하면 `None` 을 돌려주고
//! 호출한 쪽이 로그 값을 대신 쓴다. 토큰은 출력, 로그, 파일, 에러 문구에 넣지 않는다.

use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;

use super::{round_to_minute, Snapshot, Window};

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = "fitzroy";

/// 성공했을 때 이 간격보다 자주 묻지 않는다.
const MIN_INTERVAL_MS: i64 = 60_000;
/// 실패했을 때는 더 오래 쉰다. 그동안은 로그 값을 쓴다.
const RETRY_AFTER_FAILURE_MS: i64 = 5 * 60_000;

#[derive(Deserialize)]
struct AuthFile {
    tokens: Option<AuthTokens>,
}

#[derive(Deserialize)]
struct AuthTokens {
    access_token: Option<String>,
    account_id: Option<String>,
}

struct Credentials {
    access_token: String,
    account_id: Option<String>,
}

fn parse_credentials(json: &str) -> Option<Credentials> {
    let tokens = serde_json::from_str::<AuthFile>(json).ok()?.tokens?;
    let access_token = tokens.access_token.filter(|token| !token.is_empty())?;
    Some(Credentials {
        access_token,
        account_id: tokens.account_id.filter(|id| !id.is_empty()),
    })
}

#[derive(Deserialize)]
struct UsageResponse {
    plan_type: Option<String>,
    rate_limit: Option<RateLimit>,
}

#[derive(Deserialize)]
struct RateLimit {
    primary_window: Option<RawWindow>,
    secondary_window: Option<RawWindow>,
}

#[derive(Deserialize)]
struct RawWindow {
    used_percent: Option<f64>,
    limit_window_seconds: Option<u64>,
    /// Unix 초
    reset_at: Option<i64>,
}

fn parse_usage(json: &str) -> Option<Snapshot> {
    let response: UsageResponse = serde_json::from_str(json).ok()?;
    let limit = response.rate_limit?;
    let windows: Vec<Window> = [limit.primary_window, limit.secondary_window]
        .into_iter()
        .flatten()
        .filter_map(|raw| {
            Some(Window {
                percent: raw.used_percent?,
                minutes: raw.limit_window_seconds.map(|secs| secs / 60),
                resets_at: raw.reset_at.map(|secs| round_to_minute(secs * 1000)),
            })
        })
        .collect();
    if windows.is_empty() {
        return None;
    }
    Some(Snapshot {
        // 서버가 바로 알려 준 값이라 "…사용 기준" 시각을 붙이지 않는다.
        at: None,
        windows,
        plan: response.plan_type.as_deref().and_then(super::super::plan_name),
    })
}

fn request(credentials: &Credentials) -> Option<Snapshot> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let mut request = agent
        .get(USAGE_URL)
        .header("Authorization", format!("Bearer {}", credentials.access_token))
        .header("Accept", "application/json")
        .header("User-Agent", USER_AGENT);
    if let Some(id) = &credentials.account_id {
        request = request.header("ChatGPT-Account-Id", id.as_str());
    }
    let body = request.call().ok()?.body_mut().read_to_string().ok()?;
    parse_usage(&body)
}

/// 서버에 묻기 시작해도 되는 때인가
fn is_due(last_attempt_ms: Option<i64>, last_succeeded: bool, now_ms: i64) -> bool {
    let wait = if last_succeeded { MIN_INTERVAL_MS } else { RETRY_AFTER_FAILURE_MS };
    last_attempt_ms.is_none_or(|at| now_ms - at >= wait)
}

#[derive(Default)]
struct State {
    last_attempt_ms: Option<i64>,
    last: Option<Snapshot>,
}

#[derive(Default)]
pub struct ServerLimits {
    state: Mutex<State>,
}

impl ServerLimits {
    /// 서버의 한도 값. 너무 자주 묻지 않고, 못 가져오면 `None`.
    pub fn get(&self, codex_home: &Path, now_ms: i64) -> Option<Snapshot> {
        let mut state = self.state.lock().unwrap();
        if is_due(state.last_attempt_ms, state.last.is_some(), now_ms) {
            state.last_attempt_ms = Some(now_ms);
            state.last = fs::read_to_string(codex_home.join("auth.json"))
                .ok()
                .and_then(|json| parse_credentials(&json))
                .and_then(|credentials| request(&credentials));
        }
        state.last.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usage_response() {
        let json = r#"{
            "plan_type": "go",
            "rate_limit": {
                "allowed": true,
                "primary_window": {
                    "used_percent": 2,
                    "limit_window_seconds": 2592000,
                    "reset_after_seconds": 2591870,
                    "reset_at": 1795134233
                },
                "secondary_window": null
            },
            "credits": {"balance": null}
        }"#;
        let snapshot = parse_usage(json).unwrap();
        assert_eq!(snapshot.at, None);
        assert_eq!(snapshot.plan.as_deref(), Some("Go"));
        assert_eq!(
            snapshot.windows,
            vec![Window {
                percent: 2.0,
                minutes: Some(43_200),
                resets_at: Some(round_to_minute(1_795_134_233_000)),
            }]
        );
    }

    #[test]
    fn keeps_both_windows_when_present() {
        let json = r#"{"plan_type":"plus","rate_limit":{
            "primary_window":{"used_percent":10,"limit_window_seconds":18000,"reset_at":1000},
            "secondary_window":{"used_percent":40,"limit_window_seconds":604800,"reset_at":2000}}}"#;
        let snapshot = parse_usage(json).unwrap();
        let minutes: Vec<_> = snapshot.windows.iter().map(|w| w.minutes).collect();
        assert_eq!(minutes, vec![Some(300), Some(10_080)]);
    }

    #[test]
    fn unexpected_shapes_are_none() {
        assert!(parse_usage("not json").is_none());
        assert!(parse_usage(r#"{"rate_limit": null}"#).is_none());
        assert!(parse_usage(r#"{"rate_limit":{"primary_window":{"limit_window_seconds":60}}}"#).is_none());
    }

    #[test]
    fn credentials_need_a_token() {
        let ok = parse_credentials(r#"{"tokens":{"access_token":"t","account_id":"a"}}"#).unwrap();
        assert_eq!(ok.account_id.as_deref(), Some("a"));
        assert!(parse_credentials(r#"{"tokens":{"access_token":""}}"#).is_none());
        assert!(parse_credentials(r#"{"OPENAI_API_KEY":"k"}"#).is_none());
        assert!(parse_credentials("").is_none());
    }

    #[test]
    fn asks_rarely_and_waits_longer_after_a_failure() {
        assert!(is_due(None, false, 0));
        assert!(!is_due(Some(0), true, MIN_INTERVAL_MS - 1));
        assert!(is_due(Some(0), true, MIN_INTERVAL_MS));
        assert!(!is_due(Some(0), false, RETRY_AFTER_FAILURE_MS - 1));
        assert!(is_due(Some(0), false, RETRY_AFTER_FAILURE_MS));
    }

    /// 실제 로그인 정보로 서버 값을 읽어 본다: cargo test real_codex_server -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_codex_server() {
        let home = std::env::home_dir().unwrap().join(".codex");
        let snapshot = ServerLimits::default().get(&home, crate::providers::now_ms()).unwrap();
        println!("plan={:?} at={:?}", snapshot.plan, snapshot.at);
        for window in &snapshot.windows {
            println!("{window:?}");
        }
    }
}
