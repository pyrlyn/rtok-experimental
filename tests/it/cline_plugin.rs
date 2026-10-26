// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T95 + D21: the Cline plugin tree is one POSIX script plus its README.
//!
//! Check: `plugins/cline/hooks/rtok-hook` is executable, fails open without `rtok`
//! on `PATH` (`{}`, exit 0, no stderr; T174/T250: the ketch hint runs once, on
//! `TaskStart`, in Cline's own `context` shape), takes its event from its own
//! file name. The event names the installer links (T96: one link per event into
//! `~/Documents/Cline/Hooks/`) are held to `adapt_cline` by `tests/hook_manifests.rs`.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

use crate::it::common;

/// The one script T96 links once per event (`PreToolUse`, `PostToolUse`, …).
fn hook() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins/cline/hooks/rtok-hook")
}

#[test]
fn hook_script_is_executable_posix_sh() {
    let hook = hook();
    let meta = fs::metadata(&hook).unwrap_or_else(|e| panic!("{}: {e}", hook.display()));
    assert!(
        meta.permissions().mode() & 0o111 != 0,
        "{}: hook must be executable",
        hook.display()
    );
    let text = fs::read_to_string(&hook).expect("read hook");
    let first = text.lines().next().unwrap_or("");
    assert_eq!(first.trim(), "#!/bin/sh", "{}", hook.display());
}

/// Without `rtok` anywhere it looks the hook fails open: `{}` on stdout, exit 0,
/// no stderr (T174/T250: a noisy blob on every tool call was 380 "command not
/// found"-shaped errors/week in the field). `T96` resolves the binary the same
/// way, so this dir must stay free of any fake `rtok`.
#[test]
fn hook_fails_open_silently_without_rtok_on_path() {
    let sh = common::HookShell::new("cline-hook-plain");
    let dir = sh.home();
    // `$HOME/.ketch/bin/rtok` exists on dev machines, so point HOME at the empty
    // dir and keep PATH to bare system dirs without `rtok`.
    let out = run_hook(&hook(), dir, br#"{"hookName":"tool_call","taskId":"t"}"#);
    assert_eq!(out.status.code(), Some(0), "fail open exits 0");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "{}",
        "fail open prints {{}}"
    );
    assert!(
        out.stderr.is_empty(),
        "fail open must be silent: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// T174/T250: the ketch install hint runs exactly once, on the file linked as
/// `TaskStart` (Cline's session start), in Cline's own `context` shape — and
/// only when no `rtok` is found anywhere the script looks (T96 resolves the
/// binary the same way `plugins/claude/scripts/hook.sh` does).
#[test]
fn task_start_names_ketch_once_in_clines_own_shape() {
    let sh = common::HookShell::new("cline-hook-taskstart");
    let dir = sh.home();
    let linked = dir.join("TaskStart");
    fs::copy(hook(), &linked).unwrap();
    fs::set_permissions(&linked, fs::Permissions::from_mode(0o755)).unwrap();

    let out = run_hook(&linked, dir, br#"{"hookName":"agent_start","taskId":"t"}"#);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{stdout}: {e}"));
    assert!(
        v["context"]
            .as_str()
            .unwrap_or_default()
            .contains("ketch install pyrlyn/rtok"),
        "{stdout}"
    );

    // A `~/.ketch/bin/rtok` that does exist is still preferred over the note.
    sh.install_fake_ketch_rtok(common::KETCH_ECHO);
    let out = run_hook(&linked, dir, b"{}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "ketch TaskStart");
}

fn run_hook(
    script: &std::path::Path,
    home: &std::path::Path,
    stdin: &[u8],
) -> std::process::Output {
    use std::io::Write;
    Command::new(script)
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("HOME", home)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            drop(child.stdin.take().unwrap().write_all(stdin));
            child.wait_with_output()
        })
        .expect("run hook")
}
