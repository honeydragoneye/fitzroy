mod claude;
mod codex;

use serde::Serialize;

pub use claude::ClaudeProvider;
pub use codex::CodexProvider;

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

/// Provider가 돌려주는 사용량. 서비스마다 얻을 수 있는 정보가 달라서 모두 옵션이다.
/// 시각은 Unix epoch 밀리초.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub session_percent: Option<f64>,
    pub session_resets_at: Option<i64>,
    pub weekly_percent: Option<f64>,
    pub weekly_resets_at: Option<i64>,
    pub tokens_today: Option<TokenUsage>,
    pub tokens_last_5h: Option<TokenUsage>,
    pub tokens_last_7d: Option<TokenUsage>,
    pub models_today: Option<Vec<ModelUsage>>,
    /// 최근 24시간의 시간별 토큰 합계. 오래된 것부터 24칸.
    pub hourly_tokens_24h: Option<Vec<u64>>,
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
    #[serde(flatten)]
    pub usage: Usage,
}

pub fn all() -> Vec<Box<dyn Provider>> {
    vec![Box::new(ClaudeProvider::default()), Box::new(CodexProvider)]
}

pub fn fetch_all(providers: &[Box<dyn Provider>]) -> Vec<ProviderReport> {
    providers
        .iter()
        .map(|p| ProviderReport {
            id: p.id(),
            name: p.name(),
            usage: p.fetch(),
        })
        .collect()
}

fn now_ms() -> i64 {
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
            usage: Usage {
                session_percent: Some(27.0),
                session_resets_at: Some(1_700_000_000_000),
                tokens_today: Some(TokenUsage {
                    cache_read: 5,
                    ..TokenUsage::default()
                }),
                hourly_tokens_24h: Some(vec![0; 24]),
                ..Usage::default()
            },
        };
        let json = serde_json::to_value(&report).unwrap();

        assert_eq!(json["id"], "sample");
        assert_eq!(json["sessionPercent"], 27.0);
        assert_eq!(json["sessionResetsAt"], 1_700_000_000_000i64);
        assert_eq!(json["tokensToday"]["cacheRead"], 5);
        assert_eq!(json["hourlyTokens24h"].as_array().unwrap().len(), 24);
        assert!(json["weeklyPercent"].is_null());
        assert!(json["tokensLast5h"].is_null());
        assert!(json["modelsToday"].is_null());
        assert!(json["error"].is_null());
    }
}
