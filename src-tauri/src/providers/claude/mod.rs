mod logs;

use std::path::PathBuf;

use chrono::{Local, TimeZone};

use super::{now_ms, Provider, Usage};
use logs::LogReader;

#[derive(Default)]
pub struct ClaudeProvider {
    logs: LogReader,
}

/// Claude Code 설정 폴더. CLAUDE_CONFIG_DIR 이 있으면 그쪽을 쓴다.
fn config_dir() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => std::env::home_dir().map(|home| home.join(".claude")),
    }
}

fn today_start_ms(now_ms: i64) -> i64 {
    Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|midnight| Local.from_local_datetime(&midnight).earliest())
        .map(|t| t.timestamp_millis())
        .unwrap_or(now_ms - 24 * logs::HOUR_MS)
}

impl Provider for ClaudeProvider {
    fn id(&self) -> &'static str {
        "claude"
    }

    fn name(&self) -> &'static str {
        "Claude"
    }

    fn fetch(&self) -> Usage {
        let Some(projects) = config_dir().map(|dir| dir.join("projects")) else {
            return Usage::error("홈 폴더를 찾지 못했습니다");
        };

        let entries = match self.logs.read(&projects) {
            Ok(entries) => entries,
            Err(_) => return Usage::error("Claude Code 로그 폴더를 읽지 못했습니다"),
        };

        let now = now_ms();
        let summary = logs::summarize(&entries, now, today_start_ms(now));

        Usage {
            tokens_today: Some(summary.today),
            tokens_last_5h: Some(summary.last_5h),
            tokens_last_7d: Some(summary.last_7d),
            models_today: Some(summary.models_today),
            hourly_tokens_24h: Some(summary.hourly_24h),
            ..Usage::default()
        }
    }
}
