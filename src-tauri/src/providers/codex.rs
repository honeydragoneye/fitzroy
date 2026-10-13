use super::{now_ms, Provider, TokenUsage, Usage};

pub struct CodexProvider;

impl Provider for CodexProvider {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn name(&self) -> &'static str {
        "Codex"
    }

    // TODO: 가짜 데이터. 실제 세션 로그 형식을 확인한 뒤 교체한다.
    // 값이 일부만 있을 때와 에러가 있을 때의 화면을 보려고 주간 값과 모델별 사용량은 비워 둔다.
    fn fetch(&self) -> Usage {
        const HOUR: i64 = 60 * 60 * 1000;

        Usage {
            session_percent: Some(86.0),
            session_resets_at: Some(now_ms() + HOUR),
            tokens_today: Some(TokenUsage {
                input: 210_000,
                output: 36_400,
                cache_write: 0,
                cache_read: 1_150_000,
            }),
            error: Some("주간 한도 정보를 찾지 못했습니다 (가짜 데이터)".into()),
            ..Usage::default()
        }
    }
}
