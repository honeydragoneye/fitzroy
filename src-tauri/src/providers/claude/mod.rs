mod limits;
mod logs;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{Datelike, Days, Local, NaiveDate, TimeZone};

use super::{now_ms, Provider, Usage};
use limits::{Limits, LimitsError};
use logs::LogReader;

// 창을 열 때마다 갱신을 요청하므로 한도 API 는 이 간격보다 자주 부르지 않는다.
const LIMITS_TTL: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct ClaudeProvider {
    logs: LogReader,
    limits: Mutex<Option<(Instant, Limits)>>,
}

impl ClaudeProvider {
    /// 최근에 받은 값이 있으면 그대로 쓴다. 호출이 실패하면 마지막 값과 에러를 함께 돌려준다.
    fn limits(&self, config_dir: &ConfigDir, now_ms: i64) -> (Option<Limits>, Option<LimitsError>) {
        let mut cached = self.limits.lock().unwrap();
        if let Some((at, limits)) = *cached {
            if at.elapsed() < LIMITS_TTL {
                return (Some(limits), None);
            }
        }

        let token = limits::read_access_token(&config_dir.path, config_dir.is_default, now_ms);
        match token.and_then(|token| limits::fetch(&token)) {
            Ok(limits) => {
                *cached = Some((Instant::now(), limits));
                (Some(limits), None)
            }
            Err(error) => (cached.map(|(_, limits)| limits), Some(error)),
        }
    }
}

struct ConfigDir {
    path: PathBuf,
    is_default: bool,
}

/// Claude Code 설정 폴더. CLAUDE_CONFIG_DIR 이 있으면 그쪽을 쓴다.
fn config_dir() -> Option<ConfigDir> {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) if !dir.is_empty() => Some(ConfigDir {
            path: PathBuf::from(dir),
            is_default: false,
        }),
        _ => std::env::home_dir().map(|home| ConfigDir {
            path: home.join(".claude"),
            is_default: true,
        }),
    }
}

fn local_midnight_ms(date: NaiveDate) -> Option<i64> {
    let midnight = date.and_hms_opt(0, 0, 0)?;
    Some(Local.from_local_datetime(&midnight).earliest()?.timestamp_millis())
}

/// 오늘과, 이번 주(일요일~토요일) 각 날의 시작 시각(로컬 자정).
fn today_and_week_starts(now_ms: i64) -> (i64, Vec<i64>) {
    let today = Local::now().date_naive();
    let days_from_sunday = today.weekday().num_days_from_sunday() as i64;
    let today_start = local_midnight_ms(today).unwrap_or(now_ms - 24 * logs::HOUR_MS);

    let week = (0..7)
        .map(|weekday| {
            let offset = weekday - days_from_sunday;
            let date = if offset < 0 {
                today.checked_sub_days(Days::new(offset.unsigned_abs()))
            } else {
                today.checked_add_days(Days::new(offset as u64))
            };
            date.and_then(local_midnight_ms)
                .unwrap_or(today_start + offset * 24 * logs::HOUR_MS)
        })
        .collect();
    (today_start, week)
}

impl Provider for ClaudeProvider {
    fn id(&self) -> &'static str {
        "claude"
    }

    fn name(&self) -> &'static str {
        "Claude"
    }

    fn fetch(&self) -> Usage {
        let Some(config_dir) = config_dir() else {
            return Usage::error("홈 폴더를 찾지 못했습니다");
        };
        let now = now_ms();
        let mut usage = Usage::default();
        let mut errors = Vec::new();

        match self.logs.read(&config_dir.path.join("projects")) {
            Ok(entries) => {
                let (today_start, week_starts) = today_and_week_starts(now);
                let summary = logs::summarize(&entries, now, today_start, &week_starts);
                usage.tokens_today = Some(summary.today);
                usage.tokens_last_5h = Some(summary.last_5h);
                usage.tokens_last_7d = Some(summary.last_7d);
                usage.models_today = Some(summary.models_today);
                usage.daily_tokens_this_week = Some(summary.daily);
            }
            Err(_) => errors.push("Claude Code 로그 폴더를 읽지 못했습니다".to_string()),
        }

        let (limits, error) = self.limits(&config_dir, now);
        if let Some(limits) = limits {
            usage.session_percent = limits.session_percent;
            usage.session_resets_at = limits.session_resets_at;
            usage.weekly_percent = limits.weekly_percent;
            usage.weekly_resets_at = limits.weekly_resets_at;
        }
        errors.extend(error.map(|error| error.to_string()));

        if !errors.is_empty() {
            usage.error = Some(errors.join(" · "));
        }
        usage
    }
}
