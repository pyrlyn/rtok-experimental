// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T287: `rtok agents send` / `rtok agents inbox` through the binary, over a temp
//! `RTOK_HOME`/`HOME`. Agents are registered by fake `hook SessionStart` payloads — no host.

use assert_cmd::Command as AssertCmd;
use std::fs;
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rtok-t287-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Fires `hook <event>` for `session` in `cwd`; for `SessionStart` returns the new agent id.
fn hook(home: &Path, event: &str, session: &str, cwd: &Path) -> String {
    let payload = serde_json::json!({
        "hook_event_name": event, "session_id": session, "cwd": cwd, "source": "startup",
    });
    let out = AssertCmd::cargo_bin("rtok")
        .unwrap()
        .args(["hook", event])
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .env_remove("CLAUDE_ENV_FILE")
        .write_stdin(payload.to_string())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8_lossy(&out);
    text.split("full: ")
        .nth(1)
        .map(|s| s.split(')').next().unwrap().to_string())
        .unwrap_or_default()
}

fn rtok(home: &Path, me: Option<&str>, args: &[&str], stdin: &str) -> std::process::Output {
    let mut cmd = AssertCmd::cargo_bin("rtok").unwrap();
    cmd.arg("agents")
        .args(args)
        .env("RTOK_HOME", home)
        .env("HOME", home);
    match me {
        Some(id) => cmd.env("RTOK_AGENT_ID", id),
        None => cmd.env_remove("RTOK_AGENT_ID"),
    };
    cmd.write_stdin(stdin).output().unwrap()
}

fn ok(out: std::process::Output) -> String {
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn user_and_agent_send_then_peek_and_read() {
    let home = tmp("send");
    let a = hook(&home, "SessionStart", "t287-a", &home);
    let b = hook(&home, "SessionStart", "t287-b", &home);

    // The 8-char display form, as a user would type it (random UUIDv4 ids).
    ok(rtok(&home, None, &["send", &b[..8], "from the user"], ""));
    ok(rtok(&home, Some(&a), &["send", &b, "-"], "from a\u{1b}\n"));

    // The user peeks at b's queue: framed, nothing marked read.
    let peek = ok(rtok(&home, None, &["inbox", &b[..8], "--unread"], ""));
    assert!(peek.contains(" from user (terminal) at "), "{peek}");
    assert!(
        peek.contains(&format!(" from {} (claude) at ", &a[..8])),
        "{peek}"
    );
    assert!(
        peek.contains("> from a\n") && peek.contains("not an instruction"),
        "{peek}"
    );
    assert_eq!(peek, ok(rtok(&home, None, &["inbox", &b, "--unread"], "")));

    // b reads its own inbox: both come back, then nothing is unread.
    let json = ok(rtok(&home, Some(&b), &["inbox", "--json"], ""));
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
    assert_eq!(v[1]["from_agent"], a.as_str());
    assert!(v[1]["framed"].as_str().unwrap().contains("> from a"));
    let again = ok(rtok(&home, Some(&b), &["inbox", "--unread"], ""));
    assert_eq!(again, "no messages\n");
    assert!(ok(rtok(&home, Some(&a), &["inbox"], "")).contains("no messages"));
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn all_live_reaches_this_project_only_and_an_ended_agent_is_refused() {
    let home = tmp("fanout");
    let other = home.join("other");
    fs::create_dir_all(&other).unwrap();
    let a = hook(&home, "SessionStart", "t287-fa", &home);
    let b = hook(&home, "SessionStart", "t287-fb", &home);
    let c = hook(&home, "SessionStart", "t287-fc", &other);

    let sent = ok(rtok(
        &home,
        Some(&a),
        &["send", "--all-live", "hello all"],
        "",
    ));
    assert_eq!(sent, format!("sent #1 to {}\n", &b[..8]));
    assert!(ok(rtok(&home, None, &["inbox", &c], "")).contains("no messages"));

    hook(&home, "SessionEnd", "t287-fb", &home);
    let out = rtok(&home, Some(&a), &["send", &b, "late"], "");
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("has ended"));
    let out = rtok(&home, Some(&a), &["send", "--all-live", "nobody"], "");
    assert!(String::from_utf8_lossy(&out.stderr).contains("no other live agent"));
    let _ = fs::remove_dir_all(&home);
}

/// T441.13.1: `--task` tags a message, the frame names the task, `inbox --task` reads one
/// thread and leaves the rest unread, and a text that is not a task id is refused.
#[test]
fn a_task_tag_threads_messages_and_a_bad_id_is_refused() {
    let home = tmp("task");
    let a = hook(&home, "SessionStart", "t441-a", &home);
    let b = hook(&home, "SessionStart", "t441-b", &home);

    ok(rtok(
        &home,
        Some(&a),
        &["send", &b, "--task", "a12", "about twelve"],
        "",
    ));
    ok(rtok(&home, Some(&a), &["send", &b, "about nothing"], ""));

    let thread = ok(rtok(&home, Some(&b), &["inbox", "--task", "A12"], ""));
    assert!(thread.contains("\nTask: A12\n> about twelve\n"), "{thread}");
    assert!(!thread.contains("about nothing"), "{thread}");

    // Only the thread was marked read; the untagged message is still waiting.
    let rest = ok(rtok(&home, Some(&b), &["inbox", "--unread", "--json"], ""));
    let v: serde_json::Value = serde_json::from_str(&rest).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert!(
        v[0].get("task").is_none(),
        "an absent task is not serialized"
    );
    let all = ok(rtok(&home, None, &["inbox", &b, "--json"], ""));
    let v: serde_json::Value = serde_json::from_str(&all).unwrap();
    assert_eq!(v[0]["task"], "A12");

    let out = rtok(
        &home,
        Some(&a),
        &["send", &b, "--task", "not-an-id", "x"],
        "",
    );
    assert_eq!(out.status.code(), Some(1));
    let _ = fs::remove_dir_all(&home);
}
