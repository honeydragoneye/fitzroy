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

use super::{now_ms, LimitWindow, Provider, Usage};

pub const ID: &str = "codex";
pub const NAME: &str = "Codex";

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
}

#[derive(Deserialize)]
struct RateLimits {
    limit_id: Option<String>,
    primary: Option<RawWindow>,
    secondary: Option<RawWindow>,
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
    Some(Snapshot { at, windows })
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
}

#[derive(Default)]
pub struct CodexProvider {
    /// 바뀌지 않은 로그 파일은 다시 읽지 않는다.
    cache: Mutex<HashMap<PathBuf, Cached>>,
}

impl CodexProvider {
    /// 최근에 바뀐 파일부터 보며 가장 최근의 한도 값을 찾는다.
    fn latest_snapshot(&self, sessions: &Path) -> io::Result<Option<Snapshot>> {
        let mut files = Vec::new();
        collect_rollouts(sessions, &mut files)?;
        files.sort_by_key(|file| std::cmp::Reverse(file.modified));

        let mut cache = self.cache.lock().unwrap();
        for file in files.into_iter().take(MAX_FILES_TO_SCAN) {
            let fresh = cache
                .get(&file.path)
                .is_some_and(|c| c.modified == file.modified && c.len == file.len);
            if !fresh {
                let snapshot = fs::read_to_string(&file.path)
                    .ok()
                    .and_then(|text| last_snapshot(&text));
                cache.insert(
                    file.path.clone(),
                    Cached {
                        modified: file.modified,
                        len: file.len,
                        snapshot,
                    },
                );
            }
            if let Some(snapshot) = cache.get(&file.path).and_then(|c| c.snapshot.clone()) {
                return Ok(Some(snapshot));
            }
        }
        Ok(None)
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
        let snapshot = match self.latest_snapshot(&home.join("sessions")) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                return Usage::error(
                    "아직 한도 정보가 없습니다. Codex 를 한 번 사용하면 표시됩니다",
                )
            }
            Err(_) => {
                return Usage::error(
                    "Codex 로그를 찾지 못했습니다. Codex CLI 에 로그인해 사용하면 표시됩니다",
                )
            }
        };

        let now = now_ms();
        Usage {
            limits: snapshot
                .windows
                .into_iter()
                .map(|window| limit_window(window, now))
                .collect(),
            limits_as_of: snapshot.at,
            ..Usage::default()
        }
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
        assert_eq!(provider.latest_snapshot(&dir).unwrap(), None);
        assert!(provider.latest_snapshot(&dir.join("missing")).is_err());

        fs::write(
            day.join("rollout-a.jsonl"),
            token_count("2026-10-14T13:57:07Z", GO_LIMITS),
        )
        .unwrap();
        // 가장 최근 파일에 한도 값이 없으면 그 전 파일의 값을 쓴다.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(day.join("rollout-b.jsonl"), "{}\n").unwrap();
        let snapshot = provider.latest_snapshot(&dir).unwrap().unwrap();
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
