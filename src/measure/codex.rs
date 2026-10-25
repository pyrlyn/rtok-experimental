// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Codex CLI session usage (plan T49.2). `~/.codex/sessions/**/*.jsonl` writes one
//! `event_msg` line of `payload.type == "token_count"` per API request; its
//! `last_token_usage` is that request's counters (`input_tokens` includes
//! `cached_input_tokens`, `output_tokens` includes reasoning). Read on the fly like the
//! Claude Code transcripts, never written to the store, so a re-read is idempotent by
//! construction. Surveyed 2026-09-17: Cursor's `state.vscdb` carries no token counts, so
//! `codex` was the one host row here. The OpenCode and Kilo databases do carry them in
//! `message.data` (re-surveyed 2026-10-03, `research.md`) and `measure::usage` reads those
//! for `rtok agents usage`; this module stays the Codex reader.

use super::stats::ApiRow;
use serde_json::Value;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Every `*.jsonl` under `dir` (recursive) modified at or after `cutoff`. Shared with the
/// Claude Code transcript walk in `stats::collect` and the doctor skills audit (T61.3).
pub(crate) fn jsonl_paths(dir: &Path, cutoff: SystemTime) -> Vec<PathBuf> {
    paths_with_ext(dir, cutoff, "jsonl")
}

/// [`jsonl_paths`] for any one extension (`rtok agents usage` reads Gemini's legacy `.json`).
///
/// Symlinks are followed (a linked root or project folder is a real layout), but every
/// directory and file is identified by its canonical path, so an alias never yields the
/// same physical transcript twice and a cycle ends. Real entries are walked before linked
/// ones, so the path reported for a file is its real one whatever `read_dir` order is.
pub(crate) fn paths_with_ext(dir: &Path, cutoff: SystemTime, ext: &str) -> Vec<PathBuf> {
    use std::collections::HashSet;
    let Ok(root) = dir.canonicalize() else {
        return Vec::new();
    };
    let mut seen_dirs = HashSet::new();
    let mut seen_files = HashSet::new();
    let mut out = Vec::new();
    let mut late = Vec::new();
    // (path as reported, canonical path); `linked` is drained only once `real` is empty.
    let mut real = vec![(dir.to_path_buf(), root)];
    let mut linked: Vec<(PathBuf, PathBuf)> = Vec::new();
    while let Some((d, canon)) = real.pop().or_else(|| linked.pop()) {
        if !seen_dirs.insert(canon.clone()) {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let is_link = e.file_type().is_ok_and(|t| t.is_symlink());
            // A dangling or unreadable link is skipped like any inaccessible entry.
            let meta = if is_link {
                std::fs::metadata(&p)
            } else {
                e.metadata()
            };
            let Ok(meta) = meta else {
                continue;
            };
            let canon_child = if is_link {
                match p.canonicalize() {
                    Ok(c) => c,
                    Err(_) => continue,
                }
            } else {
                canon.join(e.file_name())
            };
            if meta.is_dir() {
                let stack = if is_link { &mut linked } else { &mut real };
                stack.push((p, canon_child));
                continue;
            }
            // Opening a FIFO can wait forever; linked targets must be regular files too.
            if !meta.is_file() || p.extension().and_then(OsStr::to_str) != Some(ext) {
                continue;
            }
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if mtime < cutoff {
                continue;
            }
            if is_link {
                late.push((p, canon_child));
            } else if seen_files.insert(canon_child) {
                out.push(p);
            }
        }
    }
    // File links last, so a link never shadows the real path of the same transcript.
    out.extend(
        late.into_iter()
            .filter(|(_, c)| seen_files.insert(c.clone()))
            .map(|(p, _)| p),
    );
    out
}

/// One `token_count` line: a single API request, in the proxy's vocabulary (`input` is the
/// uncached part, `cache_read` the cached part, `cache_create` Codex's
/// `cache_write_input_tokens`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Request {
    /// The `session_meta` id, else the rollout file's stem.
    pub session: String,
    /// The latest `turn_context` (or `session_meta`) model seen before this line.
    pub model: Option<String>,
    /// Unix seconds of the line; 0 when it carries none.
    pub ts: i64,
    pub input: i64,
    pub cache_read: i64,
    pub cache_create: i64,
    pub output: i64,
}

/// Every `token_count` request under `dir`, oldest file first, and the first rollout that
/// could not be read or held no JSON line at all (T358.6: `rtok agents usage` names it).
/// `collect` sums these and `measure::usage` buckets them (T358.2), so there is one parser
/// for the format.
pub fn requests(dir: &Path, cutoff: SystemTime) -> (Vec<Request>, Option<PathBuf>) {
    let mut paths = jsonl_paths(dir, cutoff);
    paths.sort();
    let mut out = Vec::new();
    let mut unreadable = None;
    for p in paths {
        let Ok(text) = std::fs::read_to_string(&p) else {
            unreadable.get_or_insert(p);
            continue;
        };
        let mut json_lines = 0;
        let stem = p
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_string();
        let (mut session, mut model) = (stem, None);
        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            json_lines += 1;
            let kind = v.pointer("/payload/type").and_then(Value::as_str);
            if let Some(m) = v.pointer("/payload/model").and_then(Value::as_str) {
                model = Some(m.to_string());
            }
            if v.get("type").and_then(Value::as_str) == Some("session_meta")
                && let Some(id) = v.pointer("/payload/id").and_then(Value::as_str)
            {
                session = id.to_string();
            }
            let Some(u) = v.pointer("/payload/info/last_token_usage") else {
                continue;
            };
            if kind != Some("token_count") {
                continue;
            }
            let n = |k: &str| u.get(k).and_then(Value::as_i64).unwrap_or(0);
            let cached = n("cached_input_tokens");
            out.push(Request {
                session: session.clone(),
                model: model.clone(),
                ts: super::jsonl::line_ts(&v),
                input: n("input_tokens").saturating_sub(cached),
                cache_read: cached,
                cache_create: n("cache_write_input_tokens"),
                output: n("output_tokens"),
            });
        }
        if json_lines == 0 && !text.trim().is_empty() {
            unreadable.get_or_insert(p);
        }
    }
    (out, unreadable)
}

/// Sum of `last_token_usage` over every `token_count` line, as an `ApiRow` in the
/// proxy's vocabulary. `None` when no line was found.
pub fn collect(dir: &Path, cutoff: SystemTime) -> Option<ApiRow> {
    let mut row = ApiRow::default();
    let mut lines = 0u64;
    for r in requests(dir, cutoff).0 {
        row.input += r.input;
        row.cache_read += r.cache_read;
        row.cache_create += r.cache_create;
        row.output += r.output;
        lines += 1;
    }
    if lines == 0 {
        return None;
    }
    let denom = row.cache_read + row.cache_create + row.input;
    row.hit = if denom == 0 {
        0.0
    } else {
        row.cache_read as f64 / denom as f64
    };
    Some(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn line(input: i64, cached: i64, write: i64, output: i64) -> String {
        json!({"timestamp":"2026-09-02T21:33:02.049Z","type":"event_msg","payload":{"type":"token_count",
            "info":{"last_token_usage":{"input_tokens":input,"cached_input_tokens":cached,
            "cache_write_input_tokens":write,"output_tokens":output,"reasoning_output_tokens":1},
            "total_token_usage":{"input_tokens":999}}}})
        .to_string()
    }

    #[test]
    fn sums_last_token_usage_and_splits_cached_input() {
        let dir = std::env::temp_dir().join(format!("rtok-codex-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("2026/09/03")).unwrap();
        let body = [
            json!({"type":"session_meta","payload":{"id":"s1","model":null}}).to_string(),
            line(20776, 20480, 0, 187),
            line(21324, 20608, 10, 46),
            // `total_token_usage` alone (no `last_token_usage`) must not count.
            json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":5}}}}).to_string(),
            "not json".into(),
        ]
        .join("\n");
        fs::write(dir.join("2026/09/03/rollout-a.jsonl"), body).unwrap();
        fs::write(dir.join("notes.txt"), "ignored").unwrap();
        let row = collect(&dir, SystemTime::UNIX_EPOCH).unwrap();
        assert_eq!(
            (row.input, row.cache_read, row.cache_create, row.output),
            (296 + 716, 20480 + 20608, 10, 233)
        );
        let denom = (296 + 716 + 20480 + 20608 + 10) as f64;
        assert!((row.hit - (20480.0 + 20608.0) / denom).abs() < 1e-9);
        assert!(
            collect(
                &dir,
                SystemTime::now() + std::time::Duration::from_secs(3600)
            )
            .is_none()
        );
        assert!(collect(&dir.join("absent"), SystemTime::UNIX_EPOCH).is_none());
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn non_regular_transcripts_and_their_aliases_are_skipped() {
        use std::os::unix::{fs::symlink, net::UnixListener};

        let dir = crate::testutil::tmp_dir("transcript-special");
        let socket = dir.join("socket.jsonl");
        let _listener = UnixListener::bind(&socket).unwrap();
        symlink(&socket, dir.join("alias.jsonl")).unwrap();
        let regular = dir.join("session.jsonl");
        fs::write(&regular, "{}\n").unwrap();

        assert_eq!(jsonl_paths(&dir, SystemTime::UNIX_EPOCH), [regular]);
        fs::remove_dir_all(dir).unwrap();
    }

    /// T579: a directory alias, a file alias and a symlink cycle never make one physical
    /// transcript count twice, while an unlinked sibling and a linked root still read.
    #[cfg(unix)]
    #[test]
    fn aliases_and_cycles_count_a_transcript_once() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!("rtok-codex-alias-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("real/sub")).unwrap();
        fs::create_dir_all(dir.join("other")).unwrap();
        fs::write(dir.join("real/s.jsonl"), line(10, 4, 0, 3)).unwrap();
        fs::write(dir.join("other/o.jsonl"), line(7, 0, 0, 1)).unwrap();
        symlink(dir.join("real"), dir.join("alias")).unwrap();
        symlink(dir.join("real/s.jsonl"), dir.join("other/file-alias.jsonl")).unwrap();
        symlink(&dir, dir.join("real/sub/loop")).unwrap();
        symlink(dir.join("missing"), dir.join("dangling")).unwrap();

        let paths = jsonl_paths(&dir, SystemTime::UNIX_EPOCH);
        assert_eq!(paths.len(), 2, "{paths:?}");
        assert!(paths.contains(&dir.join("real/s.jsonl")), "{paths:?}");
        let row = collect(&dir, SystemTime::UNIX_EPOCH).unwrap();
        assert_eq!((row.input, row.cache_read, row.output), (6 + 7, 4, 4));
        assert_eq!(requests(&dir, SystemTime::UNIX_EPOCH).0.len(), 2);
        let parsed = crate::measure::jsonl::parse_dir(&dir).unwrap();
        assert_eq!(parsed.lines, 2);

        // A symlinked root is still a readable root.
        let linked = std::env::temp_dir().join(format!("rtok-codex-root-{}", std::process::id()));
        let _ = fs::remove_file(&linked);
        symlink(&dir, &linked).unwrap();
        assert_eq!(jsonl_paths(&linked, SystemTime::UNIX_EPOCH).len(), 2);
        fs::remove_file(&linked).ok();
        fs::remove_dir_all(&dir).ok();
    }
}
