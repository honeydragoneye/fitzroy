mod claude;
// Codex 는 0.1.0 에서 뺐다. 코드는 남겨 두고 `all` 과 `PAGES` 에 넣으면 다시 켜진다.
#[allow(dead_code)]
mod codex;

use std::path::PathBuf;

use serde::Serialize;

pub use claude::ClaudeProvider;

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
}

impl TokenUsage {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_write + self.cache_read
    }

    pub fn add(&mut self, other: &TokenUsage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_write += other.cache_write;
        self.cache_read += other.cache_read;
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub model: String,
    pub tokens: TokenUsage,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyTokens {
    /// 그날이 시작하는 시각(로컬 자정)
    pub day_start: i64,
    pub tokens: u64,
}

/// 한도 하나의 사용률. 이름은 "현재 세션", "이번 주", "30일" 처럼 화면에 그대로 쓴다.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitWindow {
    pub label: String,
    pub percent: f64,
    pub resets_at: Option<i64>,
    /// 가장 최근에 지난 재설정 시각. 다음 재설정 시각을 모를 때(사용률 0%) 대신 보여준다.
    pub last_reset_at: Option<i64>,
}

/// Provider가 돌려주는 사용량. 서비스마다 얻을 수 있는 정보가 달라서 모두 옵션이다.
/// 시각은 Unix epoch 밀리초.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// 구독 플랜 이름("Pro" 등). 이름 옆에 보여 준다. 알 수 없으면 None.
    pub plan: Option<String>,
    /// 한도별 사용률. 앞의 것이 메뉴 막대에 쓰는 대표 값이다.
    pub limits: Vec<LimitWindow>,
    /// 한도 값이 언제 기준인지. 로그에서 읽어 사용할 때만 갱신되는 서비스에서 쓴다.
    pub limits_as_of: Option<i64>,
    pub tokens_today: Option<TokenUsage>,
    pub tokens_last_5h: Option<TokenUsage>,
    pub tokens_last_7d: Option<TokenUsage>,
    pub models_today: Option<Vec<ModelUsage>>,
    /// 이번 주(일요일~토요일)의 날짜별 토큰 합계. 아직 오지 않은 날은 0.
    pub daily_tokens_this_week: Option<Vec<DailyTokens>>,
    pub error: Option<String>,
}

impl Usage {
    pub fn error(message: impl Into<String>) -> Self {
        Usage {
            error: Some(message.into()),
            ..Usage::default()
        }
    }
}

pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    /// 파일 읽기나 네트워크 호출을 할 수 있으므로 백그라운드 스레드에서 부른다.
    fn fetch(&self) -> Usage;
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderReport {
    pub id: &'static str,
    pub name: &'static str,
    /// 이름을 눌러 사용량 화면을 열 수 있는지
    pub has_usage_page: bool,
    #[serde(flatten)]
    pub usage: Usage,
}

/// `cache_dir` 는 Provider 가 앱을 다시 켜도 이어 갈 상태를 저장하는 폴더다.
pub fn all(cache_dir: Option<PathBuf>) -> Vec<Box<dyn Provider>> {
    vec![Box::new(ClaudeProvider::new(cache_dir))]
}

/// 창에서 고를 수 있는 서비스 (id, 이름). `all` 과 같은 순서다.
pub const PAGES: &[(&str, &str)] = &[(claude::ID, claude::NAME)];

/// `id` 서비스의 사용량 화면 주소들. 앞의 것부터 열어 보고, 열 수 없으면 다음 것을 쓴다.
/// 모르는 서비스면 비어 있다.
pub fn usage_page_urls(id: &str) -> &'static [&'static str] {
    match id {
        claude::ID => claude::USAGE_PAGE_URLS,
        _ => &[],
    }
}

pub fn fetch_all(providers: &[Box<dyn Provider>]) -> Vec<ProviderReport> {
    providers
        .iter()
        .map(|p| ProviderReport {
            id: p.id(),
            name: p.name(),
            has_usage_page: !usage_page_urls(p.id()).is_empty(),
            usage: p.fetch(),
        })
        .collect()
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_serializes_flat_camel_case_with_nulls() {
        let report = ProviderReport {
            id: "sample",
            name: "Sample",
            has_usage_page: true,
            usage: Usage {
                limits: vec![LimitWindow {
                    label: "현재 세션".to_string(),
                    percent: 27.0,
                    resets_at: Some(1_700_000_000_000),
                    last_reset_at: None,
                }],
                tokens_today: Some(TokenUsage {
                    cache_read: 5,
                    ..TokenUsage::default()
                }),
                daily_tokens_this_week: Some(vec![DailyTokens {
                    day_start: 1_700_000_000_000,
                    tokens: 7,
                }]),
                ..Usage::default()
            },
        };
        let json = serde_json::to_value(&report).unwrap();

        assert_eq!(json["id"], "sample");
        assert_eq!(json["hasUsagePage"], true);
        assert_eq!(json["limits"][0]["label"], "현재 세션");
        assert_eq!(json["limits"][0]["percent"], 27.0);
        assert_eq!(json["limits"][0]["resetsAt"], 1_700_000_000_000i64);
        assert!(json["limits"][0]["lastResetAt"].is_null());
        assert!(json["limitsAsOf"].is_null());
        assert!(json["plan"].is_null());
        assert_eq!(json["tokensToday"]["cacheRead"], 5);
        assert_eq!(json["dailyTokensThisWeek"][0]["dayStart"], 1_700_000_000_000i64);
        assert_eq!(json["dailyTokensThisWeek"][0]["tokens"], 7);
        assert!(json["tokensLast5h"].is_null());
        assert!(json["modelsToday"].is_null());
        assert!(json["error"].is_null());
    }
}
