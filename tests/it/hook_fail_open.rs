// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T101: the hook fail-open matrix. `tests/extra_cover.rs` checks bad and empty stdin for
//! the default host only; here every value `[hook]` host accepts × every hook event ×
//! every kind of bad stdin must exit 0, print the host's no-op reply, and never rewrite
//! the input (D1). One case per combination in the nextest list.

use crate::common;

use assert_cmd::Command as AssertCmd;
use rstest::rstest;
use std::fs;
use std::path::PathBuf;

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rtok-fail-open-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn stdin_case(kind: &str) -> Vec<u8> {
    match kind {
        "empty" => Vec::new(),
        "garbage" => b"not-json{{{".to_vec(),
        "truncated-json" => br#"{"hook_event_name":"PreToolUse","tool_nam"#.to_vec(),
        "non-utf8" => vec![0xff, 0xfe, b'{', 0x00, b'}'],
        "1mib" => vec![b'x'; 1 << 20],
        other => panic!("unknown stdin case: {other}"),
    }
}

/// The hosts with an envelope of their own (the `--host` help's `claude | cursor |
/// copilot | devin`, plus grok — T98) × the events `hooks::dispatch` and the installers
/// know × the bad-stdin kinds. Every cell: exit 0, exactly `{}`, no rewrite.
#[rstest]
fn every_host_and_event_fails_open_on_bad_stdin(
    #[values("claude", "cursor", "copilot", "devin", "grok", "commandcode")] host: &str,
    #[values(
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PreCompact",
        "PostCompact",
        "SessionEnd",
        "Stop"
    )]
    event: &str,
    #[values("empty", "garbage", "truncated-json", "non-utf8", "1mib")] stdin: &str,
) {
    let home = tmp(&format!("{host}-{event}-{stdin}"));
    let out = AssertCmd::cargo_bin("rtok")
        .unwrap()
        .args(["hook", event, "--host", host])
        .env("RTOK_HOME", &home)
        .env("HOME", &home)
        .write_stdin(stdin_case(stdin))
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "{}",
        "{host}/{event}/{stdin}: the host's no-op reply, never a rewrite"
    );
    let _ = fs::remove_dir_all(&home);
}

/// T178: a store whose writer lock another process holds must not stall the hook. Every write
/// used to wait out its own 1 s busy timeout, so Claude Code cancelled hooks at its 5 s limit;
/// now the hook waits a few ms, passes the input through unchanged and writes no row.
#[test]
fn a_locked_store_fails_the_hook_open_in_ms() {
    let home = tmp("locked-store");
    let db = home.join("rtok.db");
    let payload = serde_json::json!({
        "hook_event_name": "PreToolUse",
        "session_id": "t178",
        "cwd": home,
        "tool_name": "Bash",
        "tool_input": {"command": "git status"}
    })
    .to_string();
    let hook = || {
        let start = std::time::Instant::now();
        let out = AssertCmd::cargo_bin("rtok")
            .unwrap()
            .args(["hook", "PreToolUse"])
            .env("RTOK_HOME", &home)
            .env("HOME", &home)
            .env("RTOK_CORE_DB_PATH", &db)
            .write_stdin(payload.clone())
            .assert()
            .success()
            .get_output()
            .clone();
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (stdout, start.elapsed())
    };
    let (free, _) = hook();
    assert_ne!(free, "{}", "unlocked, this call is rewritten");
    let rows = || {
        rtok::store::Store::open(&db)
            .unwrap()
            .count_call_io()
            .unwrap()
    };
    let before = rows();
    let holder = common::hold_store_writer(&db, std::time::Duration::from_secs(2));
    let (locked, took) = hook();
    holder.join().unwrap();
    assert!(took.as_millis() < 500, "the hook waited {took:?}");
    assert_eq!(locked, "{}", "a locked store passes the input through");
    assert_eq!(rows(), before, "a skipped call writes no row");
    let _ = fs::remove_dir_all(&home);
}

/// T83.15: `SessionEnd` is the one write nothing repeats. Meeting a locked store it still
/// returns at once, but hands itself to a detached child that sets `ended_at` once the lock
/// is gone — so the session's OTel root span can ship.
#[test]
fn a_locked_session_end_is_deferred_not_lost() {
    let home = tmp("locked-session-end");
    let db = home.join("rtok.db");
    let hook = |event: &str| {
        let payload = serde_json::json!({
            "hook_event_name": event,
            "session_id": "t8315",
            "cwd": home,
            "reason": "clear"
        });
        let start = std::time::Instant::now();
        AssertCmd::cargo_bin("rtok")
            .unwrap()
            .args(["hook", event])
            .env("RTOK_HOME", &home)
            .env("HOME", &home)
            .env("RTOK_CORE_DB_PATH", &db)
            .write_stdin(payload.to_string())
            .assert()
            .success();
        start.elapsed()
    };
    hook("SessionStart");
    let ended = || {
        rtok::store::Store::open(&db)
            .unwrap()
            .sessions_ended_after(0)
            .unwrap()
            .iter()
            .any(|s| s.id == "t8315")
    };
    assert!(!ended());
    let holder = common::hold_store_writer(&db, std::time::Duration::from_millis(600));
    let took = hook("SessionEnd");
    assert!(took.as_millis() < 500, "the hook waited {took:?}");
    assert!(!ended(), "the lock is still held");
    holder.join().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !ended() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(ended(), "the deferred child set ended_at");
    let _ = fs::remove_dir_all(&home);
}
