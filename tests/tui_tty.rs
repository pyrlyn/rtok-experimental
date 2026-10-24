// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T15.9: `rtok tui` without a tty — piped stdin/stdout, CI, a daemon — refuses with one
//! stderr line and exit code 1, before raw mode or the alternate screen is ever touched.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn home() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-t159-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// `.output()` pipes stdin, stdout and stderr — the exact non-tty context the guard exists
/// for. Home and cwd are isolated so no stray config or `.env` adds a stderr line.
#[test]
fn tui_without_a_tty_exits_one_with_one_clean_stderr_line() {
    let home = home();
    let out = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .arg("tui")
        .env_remove("RUST_LOG")
        .env("RTOK_HOME", &home)
        .env("HOME", &home)
        .current_dir(&home)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "a refusal, not a garbled screen"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 1, "exactly one line: {stderr:?}");
    assert!(
        lines[0].contains("terminal"),
        "the line says why: {stderr:?}"
    );
    // No terminal was ever entered, so nothing escaped into either pipe.
    assert!(out.stdout.is_empty());
    assert!(!stderr.contains('\x1b'), "no escape codes: {stderr:?}");
    let _ = fs::remove_dir_all(&home);
}
