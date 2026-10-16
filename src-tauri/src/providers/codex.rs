//! Codex 한도 사용률.
//!
//! Codex CLI 가 남기는 세션 로그(`<CODEX_HOME>/sessions/**/rollout-*.jsonl`)에서 가장 최근의
//! 한도 값을 읽는다. 값은 Codex 를 마지막으로 쓴 시점의 것이고, 네트워크와 로그인 정보는 쓰지 않는다.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use chrono::DateTime;
use serde::Deserialize;

use super::tokens::{dedupe, summarize, today_and_week_starts, Entry};
use super::{now_ms, LimitWindow, Provider, TokenUsage, Usage};

pub const ID: &str = "codex";
pub const NAME: &str = "Codex";

/// ChatGPT 설정 > 사용량 화면. Codex 앱으로 바로 여는 주소는 없어서 웹으로 연다.
pub const USAGE_PAGE_URLS: &[&str] = &["https://chatgpt.com/settings/usage?tab=overview"];

/// 최근 파일에 한도 값이 없을 때(대화를 막 시작했을 때 등) 이만큼 더 이전 파일까지 본다.
const MAX_FILES_TO_SCAN: usize = 5;

#[derive(Deserialize)]
struct Line {
    timestamp: Option<String>,
    payload: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    #[serde(rename = "type")]
    kind: Option<String>,
    rate_limits: Option<RateLimits>,
    /// `turn_context` 의 모델 이름
    model: Option<String>,
    /// `token_usage_record` 의 응답 하나를 가리키는 id. 중복 제거에 쓴다.
    response_id: Option<String>,
    usage: Option<RawTokens>,
}

/// 응답 하나의 토큰 수. `input_tokens` 는 캐시된 부분을 포함하고 `output_tokens` 는 추론 토큰을 포함한다.
#[derive(Deserialize, Default)]
#[serde(default)]
struct RawTokens {
    input_tokens: u64,
    cached_input_tokens: u64,
    cache_write_input_tokens: u64,
    output_tokens: u64,
}

#[derive(Deserialize)]
struct RateLimits {
    limit_id: Option<String>,
    primary: Option<RawWindow>,
    secondary: Option<RawWindow>,
    /// 구독 플랜. "go", "plus" 등
    plan_type: Option<String>,
}

#[derive(Deserialize)]
struct RawWindow {
    used_percent: Option<f64>,
    window_minutes: Option<u64>,
    /// Unix 초
    resets_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Window {
    percent: f64,
    minutes: Option<u64>,
    /// Unix 밀리초
    resets_at: Option<i64>,
}

/// 로그 한 줄에 담긴 한도 값들
#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    at: Option<i64>,
    windows: Vec<Window>,
    plan: Option<String>,
}

/// 재설정 시각이 기록마다 몇 초씩 달라서 가까운 분으로 맞춘다.
fn round_to_minute(ms: i64) -> i64 {
    const MINUTE_MS: i64 = 60_000;
    (ms + MINUTE_MS / 2).div_euclid(MINUTE_MS) * MINUTE_MS
}

fn parse_snapshot(line: &str) -> Option<Snapshot> {
    let line: Line = serde_json::from_str(line).ok()?;
    let payload = line.payload?;
    if payload.kind.as_deref() != Some("token_count") {
        return None;
    }
    let limits = payload.rate_limits?;
    // 다른 이름의 한도가 섞여 오면 값이 오락가락하지 않도록 Codex 기본 한도만 쓴다.
    if limits.limit_id.as_deref().is_some_and(|id| id != "codex") {
        return None;
    }
    let plan = limits.plan_type.as_deref().and_then(super::plan_name);
    let windows: Vec<Window> = [limits.primary, limits.secondary]
        .into_iter()
        .flatten()
        .filter_map(|raw| {
            Some(Window {
                percent: raw.used_percent?,
                minutes: raw.window_minutes,
                resets_at: raw.resets_at.map(|secs| round_to_minute(secs * 1000)),
            })
        })
        .collect();
    if windows.is_empty() {
        return None;
    }
    let at = line
        .timestamp
        .and_then(|text| DateTime::parse_from_rfc3339(&text).ok())
        .map(|time| time.timestamp_millis());
    Some(Snapshot { at, windows, plan })
}

/// 화면에서 Claude 와 같이 더해 보이도록 입력에서 캐시 부분을 뺀다.
fn to_tokens(raw: &RawTokens) -> TokenUsage {
    TokenUsage {
        input: raw
            .input_tokens
            .saturating_sub(raw.cached_input_tokens)
            .saturating_sub(raw.cache_write_input_tokens),
        output: raw.output_tokens,
        cache_write: raw.cache_write_input_tokens,
        cache_read: raw.cached_input_tokens,
    }
}

const UNKNOWN_MODEL: &str = "알 수 없음";

/// 로그에서 응답별 토큰 사용량을 읽는다. 모델은 앞서 나온 `turn_context` 의 것을 쓴다.
/// 같은 `response_id` 는 이어 연 세션에 복사되어 여러 파일에 있을 수 있어 `dedupe` 로 합친다.
fn parse_entries(text: &str) -> Vec<Entry> {
    let mut model = UNKNOWN_MODEL.to_string();
    let mut entries = Vec::new();
    for line in text.lines() {
        let is_context = line.contains("\"turn_context\"");
        let is_record = line.contains("\"token_usage_record\"");
        if !is_context && !is_record {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<Line>(line) else {
            continue;
        };
        let Some(payload) = parsed.payload else {
            continue;
        };
        if is_context {
            if let Some(name) = payload.model.filter(|name| !name.is_empty()) {
                model = name;
            }
            continue;
        }
        let (Some(usage), Some(at)) = (
            payload.usage,
            parsed
                .timestamp
                .and_then(|text| DateTime::parse_from_rfc3339(&text).ok()),
        ) else {
            continue;
        };
        entries.push(Entry {
            key: payload.response_id,
            ts_ms: at.timestamp_millis(),
            model: model.clone(),
            tokens: to_tokens(&usage),
        });
    }
    entries
}

fn last_snapshot(text: &str) -> Option<Snapshot> {
    text.lines()
        .rev()
        .filter(|line| line.contains("\"rate_limits\""))
        .find_map(parse_snapshot)
}

/// 한도 기간에 맞는 이름. 5시간과 1주일은 Claude 카드와 같은 말을 쓴다.
fn window_label(minutes: Option<u64>) -> String {
    const DAY: u64 = 24 * 60;
    match minutes {
        Some(300) => "현재 세션".to_string(),
        Some(m) if m == 7 * DAY => "이번 주".to_string(),
        Some(m) if m > 0 && m % DAY == 0 => format!("{}일", m / DAY),
        Some(m) if m > 0 && m % 60 == 0 => format!("{}시간", m / 60),
        Some(m) if m > 0 => format!("{m}분"),
        _ => "한도".to_string(),
    }
}

/// 재설정 시각이 지났으면 그 뒤로는 쓰지 않은 것이므로 0%로 보고, 지난 재설정 시각을 남긴다.
fn limit_window(window: Window, now_ms: i64) -> LimitWindow {
    let label = window_label(window.minutes);
    match window.resets_at {
        Some(at) if at <= now_ms => LimitWindow {
            label,
            percent: 0.0,
            resets_at: None,
            last_reset_at: Some(at),
        },
        resets_at => LimitWindow {
            label,
            percent: window.percent,
            resets_at,
            last_reset_at: None,
        },
    }
}

/// Codex 설정 폴더. CODEX_HOME 이 있으면 그쪽을 쓴다.
fn codex_home() -> Option<PathBuf> {
    match std::env::var_os("CODEX_HOME") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => std::env::home_dir().map(|home| home.join(".codex")),
    }
}

struct SessionFile {
    path: PathBuf,
    modified: Option<SystemTime>,
    len: u64,
}

fn collect_rollouts(dir: &Path, out: &mut Vec<SessionFile>) -> io::Result<()> {
    for entry in fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            let _ = collect_rollouts(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            out.push(SessionFile {
                path,
                modified: meta.modified().ok(),
                len: meta.len(),
            });
        }
    }
    Ok(())
}

struct Cached {
    modified: Option<SystemTime>,
    len: u64,
    snapshot: Option<Snapshot>,
    entries: Vec<Entry>,
}

/// 토큰 사용량을 모을 때 이만큼 지난 파일은 보지 않는다. 7일 합계와 이번 주 막대에 충분하다.
const ENTRY_LOOKBACK_MS: i64 = 8 * 24 * 60 * 60 * 1000;

/// 로그를 읽은 결과
struct Scan {
    snapshot: Option<Snapshot>,
    entries: Vec<Entry>,
}

#[derive(Default)]
pub struct CodexProvider {
    /// 바뀌지 않은 로그 파일은 다시 읽지 않는다.
    cache: Mutex<HashMap<PathBuf, Cached>>,
}

impl CodexProvider {
    /// 최근에 바뀐 파일부터 보며 가장 최근의 한도 값을 찾고,
    /// 최근 8일 안에 바뀐 파일에서 토큰 사용량을 모은다.
    fn read(&self, sessions: &Path, now_ms: i64) -> io::Result<Scan> {
        let mut files = Vec::new();
        collect_rollouts(sessions, &mut files)?;
        files.sort_by_key(|file| std::cmp::Reverse(file.modified));

        let cutoff = now_ms - ENTRY_LOOKBACK_MS;
        let recent = |file: &SessionFile| {
            file.modified
                .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
                .is_some_and(|age| age.as_millis() as i64 >= cutoff)
        };

        let mut cache = self.cache.lock().unwrap();
        let present: std::collections::HashSet<&PathBuf> = files.iter().map(|f| &f.path).collect();
        cache.retain(|path, _| present.contains(path));

        let mut snapshot = None;
        let mut entries = Vec::new();
        for (index, file) in files.iter().enumerate() {
            let wants_snapshot = snapshot.is_none() && index < MAX_FILES_TO_SCAN;
            let wants_entries = recent(file);
            if !wants_snapshot && !wants_entries {
                continue;
            }
            let fresh = cache
                .get(&file.path)
                .is_some_and(|c| c.modified == file.modified && c.len == file.len);
            if !fresh {
                let text = fs::read_to_string(&file.path).unwrap_or_default();
                cache.insert(
                    file.path.clone(),
                    Cached {
                        modified: file.modified,
                        len: file.len,
                        snapshot: last_snapshot(&text),
                        entries: parse_entries(&text),
                    },
                );
            }
            let cached = &cache[&file.path];
            if wants_snapshot {
                snapshot = cached.snapshot.clone();
            }
            if wants_entries {
                entries.extend(cached.entries.iter().cloned());
            }
        }
        Ok(Scan {
            snapshot,
            entries: dedupe(entries.iter()),
        })
    }
}

impl Provider for CodexProvider {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn fetch(&self) -> Usage {
        let Some(home) = codex_home() else {
            return Usage::error("홈 폴더를 찾지 못했습니다");
        };
        let now = now_ms();
        let Scan { snapshot, entries } = match self.read(&home.join("sessions"), now) {
            Ok(scan) => scan,
            Err(_) => {
                return Usage::error(
                    "Codex 로그를 찾지 못했습니다. Codex CLI 에 로그인해 사용하면 표시됩니다",
                )
            }
        };

        let mut usage = Usage {
            tokens_source: Some("Codex".to_string()),
            ..Usage::default()
        };
        match snapshot {
            Some(snapshot) => {
                usage.plan = snapshot.plan;
                usage.limits = snapshot
                    .windows
                    .into_iter()
                    .map(|window| limit_window(window, now))
                    .collect();
                usage.limits_as_of = snapshot.at;
            }
            None => {
                usage.error = Some(
                    "아직 한도 정보가 없습니다. Codex 를 한 번 사용하면 표시됩니다".to_string(),
                );
            }
        }
        if usage.limits.is_empty() && entries.is_empty() {
            return Usage::error("아직 사용 기록이 없습니다. Codex 를 한 번 사용하면 표시됩니다");
        }

        let (today_start, week_starts) = today_and_week_starts(now);
        let summary = summarize(&entries, now, today_start, &week_starts);
        usage.tokens_today = Some(summary.today);
        usage.tokens_last_5h = Some(summary.last_5h);
        usage.tokens_last_7d = Some(summary.last_7d);
        usage.models_today = Some(summary.models_today);
        usage.daily_tokens_this_week = Some(summary.daily);
        usage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 실제 로그 줄에서 필요한 부분과 주변 필드 일부를 남긴 것
    fn token_count(timestamp: &str, rate_limits: &str) -> String {
        format!(
            r#"{{"timestamp":"{timestamp}","ordinal":13,"type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":13828,"output_tokens":15}}}},"rate_limits":{rate_limits}}}}}"#
        )
    }

    const GO_LIMITS: &str = r#"{"limit_id":"codex","limit_name":null,"primary":{"used_percent":12.0,"window_minutes":43200,"resets_at":1794578204},"secondary":null,"credits":{"has_credits":false},"plan_type":"go"}"#;

    #[test]
    fn reads_latest_limits_from_log() {
        let text = [
            r#"{"timestamp":"2026-10-14T13:56:24.000Z","type":"session_meta","payload":{"id":"x"}}"#.to_string(),
            token_count("2026-10-14T13:56:45.636Z", &GO_LIMITS.replace("12.0", "3.0")),
            r#"{"timestamp":"2026-10-14T13:56:46.000Z","type":"token_usage_record","payload":{"usage":{"input_tokens":1}}}"#.to_string(),
            token_count("2026-10-14T13:57:07.322Z", GO_LIMITS),
            token_count("2026-10-14T13:57:08.000Z", "null"),
        ]
        .join("\n");

        let snapshot = last_snapshot(&text).unwrap();
        assert_eq!(snapshot.at, Some(1_791_986_227_322));
        assert_eq!(snapshot.plan.as_deref(), Some("Go"));
        assert_eq!(
            snapshot.windows,
            vec![Window {
                percent: 12.0,
                minutes: Some(43200),
                resets_at: Some(1_794_578_220_000),
            }]
        );
    }

    #[test]
    fn ignores_other_limits_and_empty_windows() {
        let other = GO_LIMITS.replace(r#""codex""#, r#""codex_other""#);
        assert_eq!(
            last_snapshot(&token_count("2026-10-14T13:57:07Z", &other)),
            None
        );
        let empty = r#"{"limit_id":"codex","primary":null,"secondary":null}"#;
        assert_eq!(
            last_snapshot(&token_count("2026-10-14T13:57:07Z", empty)),
            None
        );
        assert_eq!(last_snapshot("not json\n{}"), None);
    }

    fn context(model: &str) -> String {
        format!(r#"{{"timestamp":"2026-10-14T13:56:30.000Z","type":"turn_context","payload":{{"model":"{model}"}}}}"#)
    }

    fn record(timestamp: &str, id: &str, input: u64, cached: u64, output: u64) -> String {
        format!(
            r#"{{"timestamp":"{timestamp}","type":"token_usage_record","payload":{{"response_id":"{id}","usage":{{"input_tokens":{input},"cached_input_tokens":{cached},"cache_write_input_tokens":0,"output_tokens":{output},"reasoning_output_tokens":3,"total_tokens":{}}}}}}}"#,
            input + output
        )
    }

    #[test]
    fn reads_tokens_per_response_with_model() {
        let text = [
            context("gpt-a"),
            record("2026-10-14T13:57:00.000Z", "r1", 13_828, 11_008, 15),
            context("gpt-b"),
            record("2026-10-14T13:58:00.000Z", "r2", 100, 0, 20),
            "not json token_usage_record".to_string(),
        ]
        .join("\n");

        let entries = parse_entries(&text);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].model, "gpt-a");
        assert_eq!(entries[0].key.as_deref(), Some("r1"));
        // 입력에서 캐시된 부분을 빼서 Claude 와 같이 더해 보이게 한다.
        let t = entries[0].tokens;
        assert_eq!((t.input, t.output, t.cache_read, t.cache_write), (2_820, 15, 11_008, 0));
        assert_eq!(t.total(), 13_843);
        assert_eq!(entries[1].model, "gpt-b");
        assert_eq!(entries[1].tokens.total(), 120);
    }

    #[test]
    fn copied_responses_in_other_files_count_once() {
        let dir = std::env::temp_dir().join(format!("fitzroy-codex-dup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let day = dir.join("2026/10/14");
        fs::create_dir_all(&day).unwrap();
        let line = record("2026-10-14T13:57:00.000Z", "r1", 100, 0, 20);
        fs::write(day.join("rollout-a.jsonl"), format!("{line}\n")).unwrap();
        fs::write(
            day.join("rollout-b.jsonl"),
            format!("{line}\n{}\n", record("2026-10-14T13:58:00.000Z", "r2", 10, 0, 5)),
        )
        .unwrap();

        let scan = CodexProvider::default().read(&dir, 1_791_986_300_000).unwrap();
        let total: u64 = scan.entries.iter().map(|e| e.tokens.total()).sum();
        assert_eq!(scan.entries.len(), 2);
        assert_eq!(total, 120 + 15);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn labels_windows_by_length() {
        assert_eq!(window_label(Some(300)), "현재 세션");
        assert_eq!(window_label(Some(10080)), "이번 주");
        assert_eq!(window_label(Some(43200)), "30일");
        assert_eq!(window_label(Some(120)), "2시간");
        assert_eq!(window_label(Some(45)), "45분");
        assert_eq!(window_label(None), "한도");
    }

    #[test]
    fn passed_reset_counts_as_zero() {
        let window = Window {
            percent: 40.0,
            minutes: Some(43200),
            resets_at: Some(5_000),
        };
        let before = limit_window(window, 4_999);
        assert_eq!(
            (before.percent, before.resets_at, before.last_reset_at),
            (40.0, Some(5_000), None)
        );
        let after = limit_window(window, 5_000);
        assert_eq!(
            (after.percent, after.resets_at, after.last_reset_at),
            (0.0, None, Some(5_000))
        );
    }

    #[test]
    fn finds_limits_in_recent_files_and_caches() {
        let dir = std::env::temp_dir().join(format!("fitzroy-codex-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let day = dir.join("2026/10/14");
        fs::create_dir_all(&day).unwrap();

        let provider = CodexProvider::default();
        let now = 1_791_986_300_000;
        assert_eq!(provider.read(&dir, now).unwrap().snapshot, None);
        assert!(provider.read(&dir.join("missing"), now).is_err());

        fs::write(
            day.join("rollout-a.jsonl"),
            token_count("2026-10-14T13:57:07Z", GO_LIMITS),
        )
        .unwrap();
        // 가장 최근 파일에 한도 값이 없으면 그 전 파일의 값을 쓴다.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(day.join("rollout-b.jsonl"), "{}\n").unwrap();
        let snapshot = provider.read(&dir, now).unwrap().snapshot.unwrap();
        assert_eq!(snapshot.windows[0].percent, 12.0);
        assert_eq!(provider.cache.lock().unwrap().len(), 2);

        fs::remove_dir_all(&dir).unwrap();
    }

    /// 이 Mac 의 실제 Codex 로그에서 읽은 값을 출력한다.
    /// `cargo test real_codex_limits -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_codex_limits() {
        println!("{:?}", CodexProvider::default().fetch());
    }
}
