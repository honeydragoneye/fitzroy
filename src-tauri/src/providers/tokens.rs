//! 서비스별 로그에서 읽은 요청 단위 토큰 사용량을 오늘 / 최근 5시간 / 7일 / 요일별 / 모델별로 모은다.
//! Claude 와 Codex 가 같이 쓴다.

use std::collections::HashMap;

use chrono::{Datelike, Days, Local, NaiveDate, TimeZone};

use super::{DailyTokens, ModelUsage, TokenUsage};

pub const HOUR_MS: i64 = 60 * 60 * 1000;

#[derive(Debug, Clone)]
pub struct Entry {
    /// 중복 제거 키. `message.id` 가 없으면 중복을 가려낼 수 없어 None.
    pub key: Option<String>,
    pub ts_ms: i64,
    pub model: String,
    pub tokens: TokenUsage,
}

/// 같은 키의 줄들은 사용량이 같지만, 복사된 줄에 0 이 들어간 경우가 있어 가장 큰 값을 쓴다.
/// 시각은 가장 이른 것을 쓴다.
pub fn dedupe<'a>(entries: impl Iterator<Item = &'a Entry>) -> Vec<Entry> {
    let mut by_key: HashMap<&str, Entry> = HashMap::new();
    let mut result = Vec::new();

    for entry in entries {
        let Some(key) = entry.key.as_deref() else {
            result.push(entry.clone());
            continue;
        };
        match by_key.get_mut(key) {
            Some(kept) => {
                if entry.tokens.total() > kept.tokens.total() {
                    kept.tokens = entry.tokens;
                }
                kept.ts_ms = kept.ts_ms.min(entry.ts_ms);
            }
            None => {
                by_key.insert(key, entry.clone());
            }
        }
    }

    result.extend(by_key.into_values());
    result
}

#[derive(Debug, Default)]
pub struct Summary {
    pub today: TokenUsage,
    pub last_5h: TokenUsage,
    pub last_7d: TokenUsage,
    /// 오늘 사용량이 많은 순
    pub models_today: Vec<ModelUsage>,
    /// `day_starts` 의 날짜별 토큰 합계
    pub daily: Vec<DailyTokens>,
}

/// `day_starts` 는 그래프에 넣을 날짜들의 시작 시각을 오래된 날부터 늘어놓은 것이다.
pub fn summarize(
    entries: &[Entry],
    now_ms: i64,
    today_start_ms: i64,
    day_starts: &[i64],
) -> Summary {
    let mut summary = Summary {
        daily: day_starts
            .iter()
            .map(|&day_start| DailyTokens {
                day_start,
                tokens: 0,
            })
            .collect(),
        ..Summary::default()
    };
    let mut models: HashMap<&str, TokenUsage> = HashMap::new();

    for entry in entries {
        let age = now_ms - entry.ts_ms;
        if age < 0 {
            continue;
        }
        if entry.ts_ms >= today_start_ms {
            summary.today.add(&entry.tokens);
            models.entry(&entry.model).or_default().add(&entry.tokens);
        }
        if age < 5 * HOUR_MS {
            summary.last_5h.add(&entry.tokens);
        }
        if age < 7 * 24 * HOUR_MS {
            summary.last_7d.add(&entry.tokens);
        }
        // 그 시각이 속한 날: 시작 시각이 그보다 늦지 않은 마지막 날
        if let Some(day) = day_starts.iter().rposition(|&start| start <= entry.ts_ms) {
            summary.daily[day].tokens += entry.tokens.total();
        }
    }

    summary.models_today = models
        .into_iter()
        .map(|(model, tokens)| ModelUsage {
            model: model.to_string(),
            tokens,
        })
        .collect();
    summary
        .models_today
        .sort_by(|a, b| b.tokens.total().cmp(&a.tokens.total()).then(a.model.cmp(&b.model)));
    summary
}


fn local_midnight_ms(date: NaiveDate) -> Option<i64> {
    let midnight = date.and_hms_opt(0, 0, 0)?;
    Some(Local.from_local_datetime(&midnight).earliest()?.timestamp_millis())
}

/// 오늘과, 이번 주(일요일~토요일) 각 날의 시작 시각(로컬 자정).
pub fn today_and_week_starts(now_ms: i64) -> (i64, Vec<i64>) {
    let today = Local::now().date_naive();
    let days_from_sunday = today.weekday().num_days_from_sunday() as i64;
    let today_start = local_midnight_ms(today).unwrap_or(now_ms - 24 * HOUR_MS);

    let week = (0..7)
        .map(|weekday| {
            let offset = weekday - days_from_sunday;
            let date = if offset < 0 {
                today.checked_sub_days(Days::new(offset.unsigned_abs()))
            } else {
                today.checked_add_days(Days::new(offset as u64))
            };
            date.and_then(local_midnight_ms)
                .unwrap_or(today_start + offset * 24 * HOUR_MS)
        })
        .collect();
    (today_start, week)
}
