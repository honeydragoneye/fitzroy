mod limits;
mod logs;

use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{Datelike, Days, Local, NaiveDate, TimeZone};

use super::{now_ms, LimitWindow, Provider, Usage};
use limits::{Limits, LimitsError, LimitsState, PassedResets};
use logs::LogReader;

pub const ID: &str = "claude";
pub const NAME: &str = "Claude";

/// 설정 > 사용량 화면. 설치된 Claude 앱으로 먼저 열고, 앱이 없으면 웹으로 연다.
pub const USAGE_PAGE_URLS: &[&str] = &[
    "claude://claude.ai/settings/usage",
    "https://claude.ai/settings/usage",
];

const LIMITS_CACHE_FILE: &str = "claude-limits.json";

pub struct ClaudeProvider {
    logs: LogReader,
    limits: Mutex<LimitsState>,
    /// 구독 플랜 이름("Pro" 등). 로그인 정보를 읽을 때마다 갱신하고, 못 읽으면 마지막 값을 쓴다.
    plan: Mutex<Option<String>>,
    /// 한도 조회 상태를 저장할 파일. 캐시 폴더를 알 수 없으면 저장하지 않는다.
    limits_cache: Option<PathBuf>,
}

impl ClaudeProvider {
    pub fn new(cache_dir: Option<PathBuf>) -> Self {
        let limits_cache = cache_dir.map(|dir| dir.join(LIMITS_CACHE_FILE));
        let limits = limits_cache
            .as_deref()
            .map(LimitsState::load)
            .unwrap_or_default();
        ClaudeProvider {
            logs: LogReader::default(),
            limits: Mutex::new(limits),
            plan: Mutex::new(None),
            limits_cache,
        }
    }

    /// 한도, 지난 (세션, 주간) 재설정 시각, 에러를 돌려준다. 호출을 쉬는 동안이나 실패했을 때는
    /// 마지막으로 받은 값을 쓰되, 재설정 시각이 지난 값은 비운다.
    fn limits(
        &self,
        config_dir: &ConfigDir,
        now_ms: i64,
    ) -> (Option<Limits>, PassedResets, Option<LimitsError>) {
        let mut state = self.limits.lock().unwrap();

        if state.should_attempt(now_ms) {
            if let Some(plan) = limits::read_plan(&config_dir.path, config_dir.is_default) {
                *self.plan.lock().unwrap() = Some(plan);
            }
            let token = limits::read_access_token(&config_dir.path, config_dir.is_default, now_ms);
            state.record(token.and_then(|token| limits::fetch(&token)), now_ms);
            if let Some(path) = &self.limits_cache {
                state.save(path);
            }
        }

        let limits = state.last.map(|limits| limits.without_expired(now_ms));
        (limits, state.passed_resets(now_ms), state.last_error)
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
        ID
    }

    fn name(&self) -> &'static str {
        NAME
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

        let (limits, (session_reset_at, weekly_reset_at), error) = self.limits(&config_dir, now);
        usage.plan = self.plan.lock().unwrap().clone();
        if let Some(limits) = limits {
            let windows = [
                ("현재 세션", limits.session_percent, limits.session_resets_at, session_reset_at),
                ("이번 주", limits.weekly_percent, limits.weekly_resets_at, weekly_reset_at),
            ];
            usage.limits = windows
                .into_iter()
                .filter_map(|(label, percent, resets_at, last_reset_at)| {
                    Some(LimitWindow {
                        label: label.to_string(),
                        percent: percent?,
                        resets_at,
                        last_reset_at,
                    })
                })
                .collect();
        }
        errors.extend(error.map(|error| error.to_string()));

        if !errors.is_empty() {
            usage.error = Some(errors.join(" · "));
        }
        usage
    }
}
