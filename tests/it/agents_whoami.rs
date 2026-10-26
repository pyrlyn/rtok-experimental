// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T283 (D34): `RTOK_AGENT_ID`, `rtok agents whoami`, and the `CLAUDE_ENV_FILE` export —
//! through the binary, over a temp `RTOK_HOME`/`HOME` (never the real state dir). The
//! SessionStart injection's own wording and stability are `src/hooks/mod.rs` unit tests;
//! this covers the CLI surface and Claude Code's env-file mechanism end to end.

use assert_cmd::Command as AssertCmd;
use std::fs;
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rtok-t283-{name}-{}-{}",
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

/// Runs `hook SessionStart` for `session_id` and returns its stdout. Clears `CLAUDE_ENV_FILE`
/// first so an ambient value on the developer's own machine never leaks in.
fn session_start(home: &Path, session_id: &str, extra_env: &[(&str, &str)]) -> String {
    let payload = serde_json::json!({
        "hook_event_name": "SessionStart",
        "session_id": session_id,
        "cwd": home,
        "source": "startup",
    });
    let mut cmd = AssertCmd::cargo_bin("rtok").unwrap();
    cmd.args(["hook", "SessionStart"])
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .env_remove("CLAUDE_ENV_FILE")
        .write_stdin(payload.to_string());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let out = cmd.assert().success().get_output().clone();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Pulls the full uuid out of the injected `rtok agent id: <short> (full: <id>). …` line.
fn agent_id_from_stdout(stdout: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(stdout).unwrap();
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    ctx.split("full: ")
        .nth(1)
        .unwrap()
        .split(')')
        .next()
        .unwrap()
        .to_string()
}

fn whoami(home: &Path, agent_id: Option<&str>, json: bool) -> std::process::Output {
    whoami_in(home, agent_id, None, json)
}

/// [`whoami`] from a shell that may carry Claude Code's `CLAUDE_CODE_SESSION_ID` (T473); the
/// developer's own session's value is always cleared first.
fn whoami_in(
    home: &Path,
    agent_id: Option<&str>,
    session: Option<&str>,
    json: bool,
) -> std::process::Output {
    let mut cmd = AssertCmd::cargo_bin("rtok").unwrap();
    cmd.arg("agents").arg("whoami");
    if json {
        cmd.arg("--json");
    }
    cmd.env("RTOK_HOME", home)
        .env("HOME", home)
        .env_remove("CLAUDE_CODE_SESSION_ID");
    if let Some(session) = session {
        cmd.env("CLAUDE_CODE_SESSION_ID", session);
    }
    match agent_id {
        Some(id) => {
            cmd.env("RTOK_AGENT_ID", id);
        }
        None => {
            cmd.env_remove("RTOK_AGENT_ID");
        }
    }
    cmd.output().unwrap()
}

// No `RTOK_AGENT_ID` at all is the deterministic `tests/trycmd/agents-whoami.trycmd` case;
// this covers the other exit-1 path, an id that does not resolve to anything.
#[test]
fn whoami_with_an_unknown_id_exits_1() {
    let home = tmp("unknown-id");
    let out = whoami(&home, Some("deadbeefdeadbeefdeadbeefdeadbeef"), false);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not inside an agent session"));
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn whoami_json_reports_the_agent_session_start_registered() {
    let home = tmp("json");
    let stdout = session_start(&home, "t283-whoami-json", &[]);
    let id = agent_id_from_stdout(&stdout);

    let out = whoami(&home, Some(&id), true);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["id"], id);
    assert_eq!(v["short"], id[..8]);
    assert_eq!(v["host"], "claude");
    assert_eq!(v["host_session_id"], "t283-whoami-json");
    assert!(v["ended_at"].is_null());
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn whoami_text_resolves_a_short_unique_prefix_too() {
    let home = tmp("text");
    let stdout = session_start(&home, "t283-whoami-text", &[]);
    let id = agent_id_from_stdout(&stdout);

    // D34: any unique prefix (>= 4 hex chars) resolves, not only the full id.
    let out = whoami(&home, Some(&id[..8]), false);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains(&format!("id: {id}")), "{text}");
    assert!(text.contains("host: claude"), "{text}");
    let _ = fs::remove_dir_all(&home);
}

/// T283: Claude Code's `SessionStart` can fire more than once for a resumed session — the
/// `export RTOK_AGENT_ID=...` line it appends to `CLAUDE_ENV_FILE` must not duplicate.
#[test]
fn session_start_writes_claude_env_file_once_across_two_runs() {
    let home = tmp("env-file");
    let env_file = home.join("env.sh");
    fs::write(&env_file, "").unwrap();
    let path = env_file.to_str().unwrap();

    session_start(&home, "t283-env-file", &[("CLAUDE_ENV_FILE", path)]);
    session_start(&home, "t283-env-file", &[("CLAUDE_ENV_FILE", path)]);

    let contents = fs::read_to_string(&env_file).unwrap();
    let export_lines: Vec<&str> = contents
        .lines()
        .filter(|l| l.starts_with("export RTOK_AGENT_ID="))
        .collect();
    assert_eq!(export_lines.len(), 1, "{contents}");
    let _ = fs::remove_dir_all(&home);
}

/// T473: Claude's desktop app often never delivers the startup `SessionStart`, so no
/// `RTOK_AGENT_ID` reaches the shell; the first tool call's hook still registers the agent, and
/// the session id Claude Code puts into every Bash command names it.
#[test]
fn whoami_finds_the_agent_by_claude_code_session_id_without_session_start() {
    let home = tmp("session-id");
    let payload = serde_json::json!({
        "hook_event_name": "PostToolUse",
        "session_id": "t473-desktop",
        "cwd": home,
        "tool_name": "Bash",
        "tool_input": {"command": "true"},
        "tool_response": {"stdout": "", "stderr": ""},
    });
    AssertCmd::cargo_bin("rtok")
        .unwrap()
        .args(["hook", "PostToolUse"])
        .env("RTOK_HOME", &home)
        .env("HOME", &home)
        .env_remove("CLAUDE_ENV_FILE")
        .write_stdin(payload.to_string())
        .assert()
        .success();

    let out = whoami_in(&home, None, Some("t473-desktop"), true);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["host"], "claude");
    assert_eq!(v["host_session_id"], "t473-desktop");

    let out = whoami_in(&home, None, Some("t473-no-such-session"), false);
    assert_eq!(out.status.code(), Some(1));
    let _ = fs::remove_dir_all(&home);
}
