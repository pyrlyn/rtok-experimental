// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The Command Code plugin tree is the install source `rtok agents install commandcode`
//! links from: one `rtok-hook` script (event from its own link name) plus `mcp.sh`.
//! The script must fail open with an empty PATH and no binary; the events it serves are held
//! to the `hook_events` table by `tests/hook_manifests.rs`.
//!
//! POSIX scripts (`arg0`, executable bits). Windows CI does not run them.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

fn tree() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins/commandcode")
}

#[test]
fn hook_script_is_executable_and_fails_open_without_rtok() {
    let hook = tree().join("hooks/rtok-hook");
    let mode = std::fs::metadata(&hook).unwrap().permissions().mode();
    assert!(mode & 0o111 != 0, "rtok-hook must be executable");
    // No `rtok` anywhere: empty PATH plus a HOME without ~/.ketch/bin.
    let home = std::env::temp_dir().join(format!("rtok-cc-nohome-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    for event in ["PreToolUse", "PostToolUse"] {
        let out = Command::new(&hook)
            .arg0(event)
            .env_clear()
            .env("PATH", "")
            .env("HOME", &home)
            .output()
            .unwrap();
        assert!(out.status.success(), "{event}");
        assert!(out.stdout.is_empty(), "{event}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("ketch install pyrlyn/rtok"), "{event}: {err}");
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn mcp_script_is_executable_and_names_ketch_without_rtok() {
    let mcp = tree().join("scripts/mcp.sh");
    let mode = std::fs::metadata(&mcp).unwrap().permissions().mode();
    assert!(mode & 0o111 != 0, "mcp.sh must be executable");
    let home = std::env::temp_dir().join(format!("rtok-cc-nohome2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let out = Command::new(&mcp)
        .env_clear()
        .env("PATH", "")
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "no binary: the server must not start"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("ketch install pyrlyn/rtok"), "{err}");
    let _ = fs::remove_dir_all(&home);
}
