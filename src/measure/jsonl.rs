// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Claude Code transcript parser (plan T1.1).
//!
//! Port of the `scratchpad/token-research/measure_sessions.py` logic: one JSON object
//! per line; skip malformed lines and count them. Turn index increments on each
//! `user` / `assistant` message.

use serde_json::Value;
use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolUse {
    pub id: String,
    pub name: String,
    pub input: Value,
    pub turn: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String,
    pub turn: u32,
}

/// T61.1: a skill body rides as an `isMeta` user record whose top-level
/// `sourceToolUseID` keys it to the `Skill` tool_use — `stats` used to see only the
/// 22-byte tool_result while the body (median 8.9 KB, max 248 KB) re-sent whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Injected {
    pub tool_use_id: String,
    pub bytes: u64,
    pub turn: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Usage {
    pub input_tokens: u32,
    pub cache_creation_input_tokens: u32,
    pub cache_read_input_tokens: u32,
    pub output_tokens: u32,
    pub turn: u32,
    /// Unix seconds of the line (T358.2: day and month buckets); 0 when it has none.
    pub ts: i64,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThinkingBlock {
    pub bytes: u64,
    pub turn: u32,
}

/// T137: an `image` content block — inside a tool_result (`tool_use_id` set) or a user
/// message (empty). `tokens` is 0 when the header is not PNG or JPEG (`sized` false).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageBlock {
    pub tool_use_id: String,
    pub bytes: u64,
    pub tokens: u64,
    pub sized: bool,
    pub turn: u32,
}

#[derive(Debug, Default)]
pub struct Parsed {
    pub lines: u64,
    pub malformed: u64,
    /// Lines that repeat an earlier `message.id`: Claude Code writes one line per streamed
    /// content block, each carrying the whole message's `usage` again. They are one turn.
    pub duplicates: u64,
    pub tool_uses: Vec<ToolUse>,
    pub tool_results: Vec<ToolResult>,
    pub injected: Vec<Injected>,
    pub assistant_texts: Vec<String>,
    pub usages: Vec<Usage>,
    pub thinking: Vec<ThinkingBlock>,
    pub images: Vec<ImageBlock>,
    pub turns: u32,
    /// T392: the next turn index at each `compact_boundary`; a tool_use whose turn is at or
    /// past an entry ran after that compaction.
    pub compactions: Vec<u32>,
    /// T131: the first `user`-role turn's flattened content, whole — the fallback for a
    /// host that folds a spawn brief into the task prompt.
    pub first_user_text: Option<String>,
    /// T131: every `SubagentStart` hook's `additionalContext`, joined. Claude Code 2.1.304
    /// writes it as its own `attachment` record (`hook_additional_context`, `hookEvent:
    /// SubagentStart`) after the task prompt, never inside a `user` message, so
    /// `first_user_text` alone never saw a spawn brief (research.md §17.5).
    pub subagent_start_context: String,
    seen_ids: HashSet<String>,
}

pub fn parse_jsonl(text: &str) -> Parsed {
    parse_lines(text.lines())
}

pub fn parse_path(path: &Path) -> std::io::Result<Parsed> {
    let f = std::fs::File::open(path)?;
    parse_reader(BufReader::new(f))
}

fn parse_reader(mut reader: impl BufRead) -> std::io::Result<Parsed> {
    let mut out = Parsed::default();
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        // Decode separately: malformed UTF-8 consumes one line, but an I/O error may
        // persist forever without advancing the reader and must stop the scan.
        if reader.read_until(b'\n', &mut bytes)? == 0 {
            break;
        }
        match std::str::from_utf8(&bytes) {
            Ok(line) => count_line(line, &mut out),
            Err(_) => {
                out.lines += 1;
                out.malformed += 1;
            }
        }
    }
    Ok(out)
}

/// Walk `dir` recursively for `*.jsonl`. Used to assert 0 parse failures on real logs.
/// Shares the transcript walk, so an alias cannot make a file count twice.
pub fn parse_dir(dir: &Path) -> std::io::Result<Parsed> {
    let mut acc = Parsed::default();
    for p in super::codex::jsonl_paths(dir, std::time::SystemTime::UNIX_EPOCH) {
        let one = parse_path(&p)?;
        acc.lines += one.lines;
        acc.malformed += one.malformed;
        acc.duplicates += one.duplicates;
        acc.turns += one.turns;
        acc.tool_uses.extend(one.tool_uses);
        acc.tool_results.extend(one.tool_results);
        acc.injected.extend(one.injected);
        acc.assistant_texts.extend(one.assistant_texts);
        acc.usages.extend(one.usages);
        acc.thinking.extend(one.thinking);
        acc.images.extend(one.images);
    }
    Ok(acc)
}

fn parse_lines<I, S>(lines: I) -> Parsed
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out = Parsed::default();
    for line in lines {
        count_line(line.as_ref(), &mut out);
    }
    out
}

/// Fold one transcript line into `out`: blank lines are skipped, unparsable ones are
/// counted so `rtok stats` can say how much of a file it understood.
fn count_line(line: &str, out: &mut Parsed) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    out.lines += 1;
    match serde_json::from_str::<Value>(line) {
        Ok(v) => ingest(&v, out),
        Err(_) => out.malformed += 1,
    }
}

fn ingest(v: &Value, out: &mut Parsed) {
    let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
    let msg = v.get("message").unwrap_or(v);
    // A repeated `message.id` is another block of the same turn, not a new one; its `usage`
    // is the same block already summed, so `rtok stats` counted every turn several times.
    let repeat = msg
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| !out.seen_ids.insert(id.to_string()));
    let is_turn = (ty == "user" || ty == "assistant") && !repeat;
    if is_turn {
        out.turns += 1;
    } else if repeat {
        out.duplicates += 1;
    }
    let turn = out.turns.saturating_sub(1);
    if is_compact_boundary(v) {
        out.compactions.push(out.turns);
    }
    if ty == "user" && turn == 0 && out.first_user_text.is_none() {
        out.first_user_text = Some(flatten_content(msg.get("content")));
    }
    if ty == "attachment"
        && let Some(a) = v.get("attachment")
        && a.get("type").and_then(Value::as_str) == Some("hook_additional_context")
        && a.get("hookEvent").and_then(Value::as_str) == Some("SubagentStart")
    {
        if !out.subagent_start_context.is_empty() {
            out.subagent_start_context.push('\n');
        }
        out.subagent_start_context
            .push_str(&flatten_content(a.get("content")));
    }
    if ty == "user"
        && v.get("isMeta").and_then(Value::as_bool).unwrap_or(false)
        && let Some(id) = v.get("sourceToolUseID").and_then(Value::as_str)
        && !id.is_empty()
    {
        out.injected.push(Injected {
            tool_use_id: id.to_string(),
            bytes: flatten_content(msg.get("content")).len() as u64,
            turn,
        });
    }
    if let Some(u) = usage_of(msg).or_else(|| usage_of(v))
        && is_turn
    {
        out.usages.push(Usage {
            turn,
            ts: line_ts(v),
            model: msg.get("model").and_then(Value::as_str).map(str::to_string),
            ..u
        });
    }
    match msg.get("content") {
        Some(Value::String(s)) if ty == "assistant" && !s.is_empty() => {
            out.assistant_texts.push(s.clone());
        }
        Some(Value::Array(blocks)) => {
            for b in blocks {
                ingest_block(b, ty, turn, out, repeat);
            }
        }
        _ => {}
    }
}

/// One Claude Code / Codex compaction: a `system` line with `subtype=compact_boundary`.
/// `isCompactSummary` rides the same event and is not counted again.
pub(crate) fn is_compact_boundary(v: &Value) -> bool {
    v.get("subtype").and_then(Value::as_str) == Some("compact_boundary")
}

fn ingest_block(b: &Value, ty: &str, turn: u32, out: &mut Parsed, repeat: bool) {
    match b.get("type").and_then(Value::as_str) {
        Some("tool_use") => {
            let id = b
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let name = b
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let input = b.get("input").cloned().unwrap_or(Value::Null);
            out.tool_uses.push(ToolUse {
                id,
                name,
                input,
                turn,
            });
        }
        Some("tool_result") => {
            let tool_use_id = b
                .get("tool_use_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if let Some(Value::Array(a)) = b.get("content")
                && !repeat
            {
                for x in a {
                    push_image(x, &tool_use_id, turn, out);
                }
            }
            out.tool_results.push(ToolResult {
                tool_use_id,
                content: flatten_content(b.get("content")),
                turn,
            });
        }
        Some("text") if ty == "assistant" => {
            if let Some(t) = b.get("text").and_then(Value::as_str)
                && !t.is_empty()
            {
                out.assistant_texts.push(t.to_string());
            }
        }
        Some("image") if ty == "user" && !repeat => push_image(b, "", turn, out),
        Some("thinking") | Some("redacted_thinking") if ty == "assistant" && !repeat => {
            let data = b
                .get("thinking")
                .or_else(|| b.get("data"))
                .and_then(Value::as_str)
                .unwrap_or("");
            out.thinking.push(ThinkingBlock {
                bytes: data.len() as u64,
                turn,
            });
        }
        _ => {}
    }
}

/// T137: record `x` when it is an `image` block; base64 sources get bytes and pixel size.
fn push_image(x: &Value, tool_use_id: &str, turn: u32, out: &mut Parsed) {
    if x.get("type").and_then(Value::as_str) != Some("image") {
        return;
    }
    let data = x
        .pointer("/source/data")
        .and_then(Value::as_str)
        .unwrap_or("");
    let dims = super::image::dims(data);
    out.images.push(ImageBlock {
        tool_use_id: tool_use_id.to_string(),
        bytes: super::image::decoded_len(data),
        tokens: dims.map_or(0, |(w, h)| super::image::tokens(w, h)),
        sized: dims.is_some(),
        turn,
    });
}

/// A line'"'"'s RFC 3339 `timestamp` as unix seconds; 0 when it has none or it does not parse.
pub(crate) fn line_ts(v: &Value) -> i64 {
    v.get("timestamp")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<jiff::Timestamp>().ok())
        .map_or(0, |t| t.as_second())
}

fn usage_of(v: &Value) -> Option<Usage> {
    let u = v.get("usage")?.as_object()?;
    Some(Usage {
        input_tokens: num(&u.get("input_tokens")),
        cache_creation_input_tokens: num(&u.get("cache_creation_input_tokens")),
        cache_read_input_tokens: num(&u.get("cache_read_input_tokens")),
        output_tokens: num(&u.get("output_tokens")),
        turn: 0,
        ts: 0,
        model: None,
    })
}

/// One provider counter as `u32`. A value the column cannot hold saturates instead of
/// wrapping to a small, plausible-looking number (4 294 967 296 used to become 0).
fn num(v: &Option<&Value>) -> u32 {
    u32::try_from(v.and_then(Value::as_u64).unwrap_or(0)).unwrap_or(u32::MAX)
}

fn flatten_content(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| {
                x.as_str()
                    .map(str::to_string)
                    .or_else(|| x.get("text").and_then(Value::as_str).map(str::to_string))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture_200() -> (String, Expected) {
        let mut lines = Vec::new();
        for i in 0..40 {
            lines.push(
                json!({
                    "type": "assistant",
                    "message": {
                        "role": "assistant",
                        "content": [
                            {"type": "text", "text": format!("ok {i}")},
                            {"type": "tool_use", "id": format!("t{i}"), "name": "Bash", "input": {"command": "ls"}}
                        ],
                        "usage": {
                            "input_tokens": 10,
                            "cache_creation_input_tokens": 20,
                            "cache_read_input_tokens": 30,
                            "output_tokens": 40
                        }
                    }
                })
                .to_string(),
            );
            lines.push(
                json!({
                    "type": "user",
                    "message": {
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": format!("t{i}"),
                            "content": format!("out {i}")
                        }]
                    }
                })
                .to_string(),
            );
        }
        for i in 0..40 {
            lines.push(
                json!({
                    "type": "assistant",
                    "message": {
                        "role": "assistant",
                        "content": [{"type": "text", "text": format!("hi {i}")}],
                        "usage": {"input_tokens": 1, "output_tokens": 2}
                    }
                })
                .to_string(),
            );
        }
        for i in 0..60 {
            lines.push(json!({"type": "attachment", "i": i}).to_string());
        }
        for i in 0..20 {
            lines.push(format!("not-json {i}"));
        }
        assert_eq!(lines.len(), 200);
        (
            lines.join("\n"),
            Expected {
                lines: 200,
                malformed: 20,
                tool_uses: 40,
                tool_results: 40,
                assistant_texts: 80,
                usages: 80,
                turns: 120,
            },
        )
    }

    struct Expected {
        lines: u64,
        malformed: u64,
        tool_uses: usize,
        tool_results: usize,
        assistant_texts: usize,
        usages: usize,
        turns: u32,
    }

    #[test]
    fn fixture_200_expected_counts() {
        let (text, exp) = fixture_200();
        let p = parse_jsonl(&text);
        assert_eq!(p.lines, exp.lines);
        assert_eq!(p.malformed, exp.malformed);
        assert_eq!(p.tool_uses.len(), exp.tool_uses);
        assert_eq!(p.tool_results.len(), exp.tool_results);
        assert_eq!(p.assistant_texts.len(), exp.assistant_texts);
        assert_eq!(p.usages.len(), exp.usages);
        assert_eq!(p.turns, exp.turns);
        assert_eq!(p.tool_uses[0].id, "t0");
        assert_eq!(p.tool_uses[0].name, "Bash");
        assert_eq!(p.tool_results[0].tool_use_id, "t0");
        assert_eq!(p.tool_results[0].content, "out 0");
        assert_eq!(p.usages[0].cache_read_input_tokens, 30);
        assert_eq!(p.tool_uses[0].turn, 0);
        assert_eq!(p.tool_results[0].turn, 1);
    }

    /// One streamed message is several lines sharing `message.id`, each repeating `usage`:
    /// one turn, one usage row, every block kept.
    #[test]
    fn lines_sharing_a_message_id_are_one_turn() {
        let usage = json!({"input_tokens": 10, "output_tokens": 4});
        let a = json!({"type": "assistant", "message": {"id": "msg_1", "content": [
            {"type": "text", "text": "hi"}], "usage": usage}});
        let b = json!({"type": "assistant", "message": {"id": "msg_1", "content": [
            {"type": "tool_use", "id": "t1", "name": "Bash", "input": {}}], "usage": usage}});
        let c = json!({"type": "assistant", "message": {"id": "msg_2", "content": [
            {"type": "text", "text": "bye"}], "usage": usage}});
        let p = parse_jsonl(&[a, b, c].map(|v| v.to_string()).join("\n"));
        assert_eq!((p.lines, p.malformed, p.duplicates), (3, 0, 1));
        assert_eq!(p.turns, 2);
        assert_eq!(p.usages.len(), 2);
        assert_eq!(p.usages.iter().map(|u| u.input_tokens).sum::<u32>(), 20);
        assert_eq!(p.tool_uses.len(), 1);
        assert_eq!(p.tool_uses[0].turn, 0, "the block stays in its turn");
        assert_eq!(p.assistant_texts, ["hi", "bye"]);
    }

    #[test]
    fn skips_malformed_and_counts_them() {
        let p = parse_jsonl("{\n{}\n");
        assert_eq!(p.lines, 2);
        assert_eq!(p.malformed, 1);
    }

    /// A line that is not valid UTF-8 is one malformed line, not the end of the transcript:
    /// the reader stopped there and `rtok stats` silently under-reported the session.
    #[test]
    fn a_non_utf8_line_does_not_truncate_the_file() {
        let dir = std::env::temp_dir().join(format!("rtok-jsonl-bytes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");
        let mut body = Vec::new();
        body.extend_from_slice(br#"{"type":"user","message":{"content":"hi"}}"#);
        body.push(b'\n');
        body.extend_from_slice(b"{\"type\":\"user\",\"message\":\"\xff\xfe\"}\n");
        body.extend_from_slice(br#"{"type":"assistant","message":{"content":"there"}}"#);
        body.push(b'\n');
        std::fs::write(&path, &body).unwrap();
        let p = parse_path(&path).unwrap();
        assert_eq!(p.lines, 3, "every line counted");
        assert_eq!(
            p.malformed, 1,
            "the invalid line is reported, not swallowed"
        );
        assert_eq!(p.turns, 2, "the line after it is still parsed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reader_errors_stop_the_scan_instead_of_retrying_forever() {
        use std::io::{self, Read};

        struct Broken {
            kind: io::ErrorKind,
            attempted: bool,
        }

        impl Read for Broken {
            fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
                assert!(!self.attempted, "a persistent error must not be retried");
                self.attempted = true;
                Err(io::Error::from(self.kind))
            }
        }

        for kind in [io::ErrorKind::Other, io::ErrorKind::InvalidData] {
            let reader = io::Cursor::new(b"{}\n").chain(Broken {
                kind,
                attempted: false,
            });
            assert_eq!(
                parse_reader(BufReader::new(reader)).unwrap_err().kind(),
                kind
            );
        }
    }

    #[test]
    fn a_directory_is_a_read_error() {
        let dir = crate::testutil::tmp_dir("jsonl-directory");
        assert!(parse_path(&dir).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn thinking_blocks_counted_once_per_message_id() {
        let usage = json!({"input_tokens": 10, "output_tokens": 4});
        let thinking = json!({"type": "assistant", "message": {"id": "m1", "content": [
            {"type": "thinking", "thinking": "reasoning here"}], "usage": usage}});
        let redacted = json!({"type": "assistant", "message": {"id": "m2", "content": [
            {"type": "redacted_thinking", "data": "x"}], "usage": usage}});
        let dup = json!({"type": "assistant", "message": {"id": "m1", "content": [
            {"type": "text", "text": "hi"}], "usage": usage}});
        let p = parse_jsonl(&[thinking, redacted, dup].map(|v| v.to_string()).join("\n"));
        assert_eq!(p.thinking.len(), 2);
        assert_eq!(p.thinking[0].bytes, 14);
        assert_eq!(p.thinking[1].bytes, 1);
        assert_eq!(p.duplicates, 1);
    }

    /// T131: only a `SubagentStart` hook's `additionalContext` attachment is kept; another
    /// event's context and a `hook_success` attachment are not, and nothing becomes a turn.
    #[test]
    fn subagent_start_context_comes_from_its_attachment_only() {
        let att = |kind: &str, event: &str, text: &str| {
            json!({"type": "attachment", "attachment": {"type": kind, "hookEvent": event,
                "content": [text]}})
            .to_string()
        };
        let p = parse_jsonl(
            &[
                json!({"type": "user", "message": {"role": "user", "content": "task"}}).to_string(),
                att("hook_success", "SubagentStart", "ok"),
                att("hook_additional_context", "SessionStart", "recall"),
                att("hook_additional_context", "SubagentStart", "id"),
                att("hook_additional_context", "SubagentStart", "brief"),
            ]
            .join("\n"),
        );
        assert_eq!(p.subagent_start_context, "id\nbrief");
        assert_eq!(p.first_user_text.as_deref(), Some("task"));
        assert_eq!(p.turns, 1);
    }

    #[test]
    fn real_claude_projects_zero_parse_failures() {
        let Some(home) = crate::config::env_user_home() else {
            return;
        };
        let dir = home.join(".claude/projects/-Users-listepo-GitHub-rtok");
        if !dir.is_dir() {
            return;
        }
        let p = parse_dir(&dir).expect("read jsonl");
        assert!(p.lines > 0, "expected jsonl under {}", dir.display());
        assert_eq!(
            p.malformed, 0,
            "{} parse failures in {} lines",
            p.malformed, p.lines
        );
    }
}
