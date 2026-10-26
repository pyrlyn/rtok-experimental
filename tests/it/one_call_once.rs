// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T245 + D3: one tool call is processed once, however many hook events reach rtok for it — a
//! host that fires two events for one call (Cursor: `postToolUse` and `afterMCPExecution`), or
//! one event delivered twice (Claude Code with both the plugin's hooks and leftover
//! settings-file hooks). The second delivery adds no `Measurement` row, so savings never
//! double-count.
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Home(PathBuf);
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp(n: &str) -> Home {
    let t = std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("rtok-t245-{n}-{}-{t}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    Home(d)
}

fn hook(home: &Home, args: &[&str], stdin: &Value) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_rtok"));
    c.arg("hook").args(args).current_dir(&home.0);
    c.env("RTOK_HOME", home.0.join(".rtok"))
        .env("HOME", &home.0);
    c.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    drop(
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.to_string().as_bytes()),
    );
    let out = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{args:?} {err}");
}

fn rows(home: &Home) -> i64 {
    let db = home.0.join(".rtok/rtok.db");
    rtok::store::Store::open(&db)
        .unwrap()
        .count_measurements()
        .unwrap()
}

/// Every delivery in `events` is the same call; the first one may record, the rest may not.
fn once(home: &Home, events: &[(&[&str], Value)]) {
    let (first, rest) = events.split_first().unwrap();
    hook(home, first.0, &first.1);
    let after_first = rows(home);
    for (args, stdin) in rest {
        hook(home, args, stdin);
        assert_eq!(rows(home), after_first, "{args:?} recorded the call again");
    }
}

fn long() -> String {
    (1..=300)
        .map(|i| format!("line {i} of a long result\n"))
        .collect()
}

#[test]
fn a_cursor_mcp_call_records_one_row_across_both_events() {
    let home = tmp("cursor-mcp");
    let output = json!({"content": [{"type": "text", "text": long()}]}).to_string();
    let call = |event: &str| {
        json!({
            "hook_event_name": event, "conversation_id": "c1", "generation_id": "g1",
            "tool_name": "fetch", "mcp_server_name": "web", "tool_input": {},
            "tool_output": output, "result_json": output,
        })
    };
    let post: &[&str] = &["PostToolUse", "--host", "cursor"];
    let after: &[&str] = &["afterMCPExecution", "--host", "cursor"];
    once(
        &home,
        &[
            (post, call("postToolUse")),
            (after, call("afterMCPExecution")),
        ],
    );
    assert_eq!(
        rows(&home),
        1,
        "the long result is shortened once; the stored observation writes no row (T927)"
    );
}

fn claude(event: &str, tool: &str, input: Value, response: Value, cwd: &Path) -> Value {
    json!({
        "session_id": "s1", "transcript_path": cwd.join("t.jsonl"), "cwd": cwd,
        "hook_event_name": event, "tool_name": tool, "tool_input": input,
        "tool_use_id": format!("toolu_{event}_{tool}"), "tool_response": response,
    })
}

#[test]
fn a_claude_hook_delivered_twice_records_once() {
    let home = tmp("claude-twice");
    let file = home.0.join("big.txt");
    std::fs::write(&file, long()).unwrap();
    let read = json!({"file_path": file});
    let body = json!({"file": {"filePath": file, "content": long()}});
    let pre: &[&str] = &["PreToolUse"];
    let post: &[&str] = &["PostToolUse"];
    // A first read fills the read cache, so the next read of the same file is a delta.
    let first = claude("PostToolUse", "Read", read.clone(), body.clone(), &home.0);
    once(&home, &[(post, first.clone()), (post, first)]);
    let again = claude("PreToolUse", "Read", read, Value::Null, &home.0);
    once(&home, &[(pre, again.clone()), (pre, again)]);
    assert!(rows(&home) > 0, "the delta read is recorded, once");
    let bash = json!({"command": "cat big.txt"});
    let out = json!({"stdout": long(), "stderr": "", "interrupted": false});
    let run = claude("PostToolUse", "Bash", bash, out, &home.0);
    once(&home, &[(post, run.clone()), (post, run)]);
}
