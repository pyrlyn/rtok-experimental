// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T921: a hook call that cannot finish still answers `{}` with exit 0 before the deadline
//! (`RTOK_HOOK_DEADLINE_MS`, here a fraction of a second), on every path: `rtok hook` with a
//! host that never closes stdin, and `rtok-hook` whose `rtok hook` fallback hangs.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEADLINE_MS: &str = "300";
/// Not a latency bound: a process start took 13 s on a machine at load 50, and the hook before
/// T921 never answered at all. This only tells "answered by itself" from "waited for the host".
const ANSWERED_BY: Duration = Duration::from_secs(20);

fn home(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rtd{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hung(mut cmd: Command, home: &std::path::Path) -> (std::process::Output, Duration) {
    let mut child = cmd
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("RTOK_HOME", home)
        .env("RTOK_HOOK_DEADLINE_MS", DEADLINE_MS)
        .current_dir(home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The host's stdin stays open and silent: nothing for the hook to read to the end.
    let stdin = child.stdin.take().unwrap();
    let start = Instant::now();
    let out = child.wait_with_output().unwrap();
    drop(stdin);
    (out, start.elapsed())
}

#[test]
fn rtok_hook_answers_when_the_host_never_closes_stdin() {
    let home = home("a");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rtok"));
    cmd.args(["hook", "PreToolUse"]);
    let (out, took) = hung(cmd, &home);
    assert!(out.status.success(), "{:?}", out.status);
    assert_eq!(out.stdout, b"{}");
    assert!(took < ANSWERED_BY, "{took:?}");
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn the_client_answers_when_its_rtok_hook_fallback_hangs() {
    use std::os::unix::fs::PermissionsExt;
    let home = home("b");
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_rtok-hook"), bin.join("rtok-hook")).unwrap();
    // A fallback that never answers: `exec` so the kill reaches the sleeper, not a shell.
    let fake = bin.join("rtok");
    std::fs::write(&fake, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut cmd = Command::new(bin.join("rtok-hook"));
    cmd.arg("PreToolUse");
    let (out, took) = hung(cmd, &home);
    assert!(out.status.success(), "{:?}", out.status);
    assert_eq!(out.stdout, b"{}");
    assert!(took < ANSWERED_BY, "{took:?}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_hook_inside_its_deadline_is_not_cut_short() {
    let home = home("c");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .args(["hook", "PreToolUse"])
        .env("RTOK_HOME", &home)
        .env("RTOK_HOOK_DEADLINE_MS", "30000")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let payload = serde_json::json!({"hook_event_name": "PreToolUse", "session_id": "t921",
        "cwd": home, "tool_name": "Bash", "tool_input": {"command": "git status"}});
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("rtok run"));
    let _ = std::fs::remove_dir_all(&home);
}
