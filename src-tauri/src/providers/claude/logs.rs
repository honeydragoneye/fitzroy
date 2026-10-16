//! Claude Code 로컬 로그(`projects/**/*.jsonl`)에서 토큰 사용량을 읽는다.
//!
//! 한 번의 응답이 내용 블록마다 여러 줄로 기록되고, 이어서 연 세션에는 이전 줄이 복사되어
//! 다른 파일에도 들어 있다. 그래서 `message.id` + `requestId` 로 중복을 제거한다.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use chrono::DateTime;
use serde::Deserialize;

use crate::providers::tokens::{dedupe, Entry};
use crate::providers::TokenUsage;

const SYNTHETIC_MODEL: &str = "<synthetic>";

#[derive(Deserialize)]
struct RawLine {
    timestamp: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    message: Option<RawMessage>,
}

#[derive(Deserialize)]
struct RawMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<RawUsage>,
}

#[derive(Deserialize)]
struct RawUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
}


fn parse_line(line: &[u8]) -> Option<Entry> {
    // 대부분의 줄은 사용량이 없다. JSON 파싱 전에 걸러 낸다.
    memchr::memmem::find(line, b"\"usage\"")?;

    let raw: RawLine = serde_json::from_slice(line).ok()?;
    let message = raw.message?;
    let usage = message.usage?;
    let model = message.model?;
    if model == SYNTHETIC_MODEL {
        return None;
    }
    let ts_ms = DateTime::parse_from_rfc3339(raw.timestamp.as_deref()?)
        .ok()?
        .timestamp_millis();

    Some(Entry {
        key: message
            .id
            .map(|id| format!("{id}:{}", raw.request_id.unwrap_or_default())),
        ts_ms,
        model,
        tokens: TokenUsage {
            input: usage.input_tokens.unwrap_or(0),
            output: usage.output_tokens.unwrap_or(0),
            cache_write: usage.cache_creation_input_tokens.unwrap_or(0),
            cache_read: usage.cache_read_input_tokens.unwrap_or(0),
        },
    })
}

/// `offset` 부터 끝까지 읽어 항목과 다음에 읽을 위치를 돌려준다.
/// 줄바꿈으로 끝나지 않은 마지막 줄은 아직 쓰는 중일 수 있어 다음 번으로 미룬다.
fn parse_from(path: &Path, offset: u64) -> io::Result<(Vec<Entry>, u64)> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut reader = BufReader::with_capacity(1 << 16, file);

    let mut entries = Vec::new();
    let mut pos = offset;
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 || line.last() != Some(&b'\n') {
            break;
        }
        pos += read as u64;
        entries.extend(parse_line(&line));
    }
    Ok((entries, pos))
}

fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)?.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            // 하위 폴더 하나를 못 읽어도 나머지는 계속 읽는다.
            let _ = collect_jsonl(&path, out);
        } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "jsonl") {
            out.push(path);
        }
    }
    Ok(())
}

#[derive(Default)]
struct CachedFile {
    modified: Option<SystemTime>,
    len: u64,
    parsed_len: u64,
    entries: Vec<Entry>,
}

/// 파일별 파싱 결과를 기억해 두고, 바뀐 파일만 다시 읽는다.
/// 로그는 뒤에 덧붙여지기만 하므로 커진 파일은 늘어난 부분만 읽는다.
#[derive(Default)]
pub struct LogReader {
    files: Mutex<HashMap<PathBuf, CachedFile>>,
}

impl LogReader {
    /// `root` 아래 모든 jsonl 의 사용량 항목을 중복 없이 돌려준다.
    pub fn read(&self, root: &Path) -> io::Result<Vec<Entry>> {
        let mut paths = Vec::new();
        collect_jsonl(root, &mut paths)?;

        let mut files = self.files.lock().unwrap();
        let present: HashSet<&PathBuf> = paths.iter().collect();
        files.retain(|path, _| present.contains(path));

        for path in &paths {
            let Ok(meta) = fs::metadata(path) else {
                continue;
            };
            let len = meta.len();
            let modified = meta.modified().ok();

            let start = match files.get(path) {
                Some(cached) if cached.len == len && cached.modified == modified => continue,
                Some(cached) if len > cached.len => cached.parsed_len,
                _ => 0,
            };
            let Ok((entries, parsed_len)) = parse_from(path, start) else {
                continue;
            };

            let cached = files.entry(path.clone()).or_default();
            if start == 0 {
                cached.entries.clear();
            }
            cached.entries.extend(entries);
            cached.modified = modified;
            cached.len = len;
            cached.parsed_len = parsed_len;
        }

        Ok(dedupe(files.values().flat_map(|cached| &cached.entries)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::tokens::{summarize, HOUR_MS};
    use std::io::Write;

    fn line(id: &str, request: &str, model: &str, ts: &str, tokens: [u64; 4]) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","requestId":"{request}","message":{{"id":"{id}","model":"{model}","content":[],"usage":{{"input_tokens":{},"output_tokens":{},"cache_creation_input_tokens":{},"cache_read_input_tokens":{},"service_tier":null}}}}}}"#,
            tokens[0], tokens[1], tokens[2], tokens[3]
        ) + "\n"
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fitzroy-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("project")).unwrap();
        dir
    }

    fn append(path: &Path, text: &str) {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(text.as_bytes()).unwrap();
    }

    fn sum(entries: &[Entry]) -> u64 {
        entries.iter().map(|e| e.tokens.total()).sum()
    }

    #[test]
    fn dedupes_across_lines_and_files_and_skips_synthetic() {
        let dir = temp_dir("dedupe");
        let ts = "2026-01-01T00:00:00.000Z";
        let a = dir.join("project/a.jsonl");
        append(&a, &line("m1", "r1", "model-a", ts, [1, 2, 3, 4]));
        append(&a, &line("m1", "r1", "model-a", ts, [1, 2, 3, 4]));
        append(&a, &line("m2", "r2", "model-a", ts, [10, 0, 0, 0]));
        append(&a, &line("m3", "r3", "<synthetic>", ts, [0, 0, 0, 0]));
        append(&a, "{\"type\":\"user\",\"message\":{\"content\":\"usage\"}}\n");
        append(&a, "not json \"usage\"\n");
        // 다른 파일에 복사된 줄. 0 으로 기록된 복사본이 원래 값을 덮어쓰면 안 된다.
        let b = dir.join("project/b.jsonl");
        append(&b, &line("m1", "r1", "model-a", ts, [0, 0, 0, 0]));
        append(&b, &line("m2", "r2", "model-a", ts, [10, 0, 0, 0]));

        let entries = LogReader::default().read(&dir).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(sum(&entries), 20);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_only_appended_part_and_waits_for_complete_lines() {
        let dir = temp_dir("append");
        let ts = "2026-01-01T00:00:00.000Z";
        let path = dir.join("project/a.jsonl");
        let reader = LogReader::default();

        append(&path, &line("m1", "r1", "model-a", ts, [1, 0, 0, 0]));
        assert_eq!(sum(&reader.read(&dir).unwrap()), 1);

        let next = line("m2", "r2", "model-a", ts, [2, 0, 0, 0]);
        let (head, tail) = next.split_at(next.len() / 2);
        append(&path, head);
        assert_eq!(sum(&reader.read(&dir).unwrap()), 1);
        append(&path, tail);
        assert_eq!(sum(&reader.read(&dir).unwrap()), 3);

        // 파일이 줄어들면 처음부터 다시 읽는다.
        fs::write(&path, line("m9", "r9", "model-a", ts, [5, 0, 0, 0])).unwrap();
        assert_eq!(sum(&reader.read(&dir).unwrap()), 5);

        fs::remove_file(&path).unwrap();
        assert!(reader.read(&dir).unwrap().is_empty());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_root_is_an_error() {
        let dir = std::env::temp_dir().join("fitzroy-test-does-not-exist");
        assert!(LogReader::default().read(&dir).is_err());
    }

    #[test]
    fn summarizes_by_window_and_model() {
        let now = 100 * 24 * HOUR_MS;
        let today_start = now - 10 * HOUR_MS;
        // 그제, 어제, 오늘, 내일
        let day_starts = [-48, -24, 0, 24].map(|hours| today_start + hours * HOUR_MS);
        let entry = |age_ms: i64, model: &str, input: u64| Entry {
            key: None,
            ts_ms: now - age_ms,
            model: model.to_string(),
            tokens: TokenUsage {
                input,
                ..TokenUsage::default()
            },
        };
        let entries = [
            entry(0, "a", 1),
            entry(HOUR_MS / 2, "b", 2),
            entry(6 * HOUR_MS, "b", 4),
            entry(23 * HOUR_MS + 1, "a", 8),
            entry(3 * 24 * HOUR_MS, "a", 16),
            entry(8 * 24 * HOUR_MS, "a", 32),
            entry(-HOUR_MS, "a", 64),
        ];

        let s = summarize(&entries, now, today_start, &day_starts);
        assert_eq!(s.last_5h.input, 3);
        assert_eq!(s.today.input, 7);
        assert_eq!(s.last_7d.input, 31);
        let daily: Vec<u64> = s.daily.iter().map(|d| d.tokens).collect();
        assert_eq!(daily, [0, 8, 7, 0]);
        assert_eq!(s.daily[0].day_start, day_starts[0]);
        assert_eq!(s.models_today.len(), 2);
        assert_eq!(s.models_today[0].model, "b");
        assert_eq!(s.models_today[0].tokens.input, 6);
    }

    /// 실제 로그를 읽어 합계를 출력한다. 수작업 계산과 대조할 때 쓴다.
    /// `cargo test real_logs -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_logs_totals() {
        let root = std::env::home_dir().unwrap().join(".claude/projects");
        let reader = LogReader::default();
        let started = std::time::Instant::now();
        let entries = reader.read(&root).unwrap();
        let first = started.elapsed();
        let started = std::time::Instant::now();
        reader.read(&root).unwrap();

        let mut total = TokenUsage::default();
        entries.iter().for_each(|e| total.add(&e.tokens));
        println!(
            "unique={} input={} output={} cache_write={} cache_read={}",
            entries.len(),
            total.input,
            total.output,
            total.cache_write,
            total.cache_read
        );
        println!("first read {first:?}, cached read {:?}", started.elapsed());
    }
}
