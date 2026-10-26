// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T24.2: `rtok logs` and `rtok logs export` against the real binary and rotated files on disk.
//! T24.3: `rtok logs watch` against the real binary, a line written by this (other) process, and
//! a rotation mid-watch.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rtok")
}

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-t242-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("logs")).unwrap();
    dir
}

fn rtok(args: &[&str], home: &Path) -> String {
    let out = Command::new(bin())
        .args(args)
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "rtok {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// 4 lines per file, newest last within each file — the shape rotation leaves behind.
fn seed_rotated(home: &Path) {
    let log = home.join("logs/rtok.log");
    fs::write(&log, "live-1\nlive-2\nlive-3\nlive-4\n").unwrap();
    fs::write(log.with_extension("log.1"), "r1-1\nr1-2\nr1-3\nr1-4\n").unwrap();
    fs::write(log.with_extension("log.2"), "r2-1\nr2-2\nr2-3\nr2-4\n").unwrap();
    fs::write(log.with_extension("log.3"), "r3-1\nr3-2\nr3-3\nr3-4\n").unwrap();
}

const EXPECT: [&str; 10] = [
    "live-4", "live-3", "live-2", "live-1", "r1-4", "r1-3", "r1-2", "r1-1", "r2-4", "r2-3",
];

#[test]
fn logs_prints_newest_first_numbered_across_the_rotation_boundary() {
    let home = home("print");
    seed_rotated(&home);
    let out = rtok(&["logs", "--lines", "10"], &home);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 10, "{out}");
    // First line is the newest written; the tenth is ten lines back, past two file boundaries.
    for (i, want) in EXPECT.iter().enumerate() {
        assert_eq!(lines[i], format!("{} {want}", i + 1), "{out}");
    }
}

#[test]
fn export_is_the_same_lines_with_numbering_and_colour_stripped() {
    let home = home("export");
    seed_rotated(&home);
    let out = rtok(&["logs", "export", "--lines", "10"], &home);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, EXPECT.to_vec(), "{out}");
    // No terminal in a test run, so there is no ANSI to strip either: the numbered screen minus
    // its leading "<n> " is byte-identical to export, in both directions.
    let numbered = rtok(&["logs", "--lines", "10"], &home);
    for (n, e) in numbered.lines().zip(out.lines()) {
        let (_, rest) = n.split_once(' ').unwrap();
        assert_eq!(rest, e);
    }
}

#[test]
fn both_commands_say_so_when_nothing_has_been_logged() {
    let home = home("empty");
    assert_eq!(rtok(&["logs"], &home).trim(), "no logs yet");
    assert_eq!(rtok(&["logs", "export"], &home).trim(), "no logs yet");
}

/// The child watches; this process is the "another process" of T24.3's Check. Its stdout is a
/// pipe, so the run also proves the non-TTY half: plain rows, no escape codes.
#[test]
fn watch_streams_a_line_from_another_process_and_survives_a_rotation() {
    let home = home("watch");
    let log = home.join("logs/rtok.log");
    fs::write(&log, "seed-1\nseed-2\n").unwrap();

    let mut child = Command::new(bin())
        .args(["logs", "watch", "--lines", "5"])
        .env("RTOK_HOME", &home)
        .env("HOME", &home)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            match line {
                Ok(l) => sink.lock().unwrap().push(l),
                Err(_) => break,
            }
        }
    });

    // 20 poll intervals of slack for CI, not part of the contract: the contract is one.
    let wait = |want: &str| {
        let deadline = Instant::now() + rtok::log::WATCH_POLL * 20;
        loop {
            if seen.lock().unwrap().iter().any(|l| l == want) {
                return;
            }
            if Instant::now() > deadline {
                panic!("watch never said {want:?}: {:?}", seen.lock().unwrap());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    };

    // The same screen `rtok logs` prints, newest first, then the followed lines counted past it.
    wait("1 seed-2");
    wait("2 seed-1");
    writeln!(
        OpenOptions::new().append(true).open(&log).unwrap(),
        "watch-a"
    )
    .unwrap();
    wait("3 watch-a");

    // Rotation the way the sink does it (T24.0): rename, and the next write recreates `path`.
    fs::rename(&log, log.with_extension("log.1")).unwrap();
    writeln!(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .unwrap(),
        "watch-b"
    )
    .unwrap();
    wait("4 watch-b");

    child.kill().unwrap();
    child.wait().unwrap();
    let got = seen.lock().unwrap().join("\n");
    for once in ["seed-2", "seed-1", "watch-a", "watch-b"] {
        assert_eq!(got.matches(once).count(), 1, "each line once: {got}");
    }
    assert!(
        !got.contains('\x1b'),
        "a pipe gets plain rows, not escapes: {got:?}"
    );
}

/// T225: the stderr debug log is `RUST_LOG`-gated — silent without it, argv with it.
fn rtok_stderr(home: &Path, rust_log: Option<&str>) -> String {
    let mut cmd = Command::new(bin());
    cmd.args(["logs", "--lines", "1"])
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .env("RUST_LOG_STYLE", "never")
        .env_remove("RUST_LOG");
    if let Some(filter) = rust_log {
        cmd.env("RUST_LOG", filter);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn rust_log_off_leaves_stderr_empty() {
    let home = home("rust-log-off");
    seed_rotated(&home);
    assert_eq!(rtok_stderr(&home, None), "");
}

#[test]
fn rust_log_debug_prints_argv_on_stderr() {
    let home = home("rust-log-debug");
    seed_rotated(&home);
    let err = rtok_stderr(&home, Some("rtok=debug"));
    assert!(err.contains("DEBUG rtok::cli] argv"), "{err}");
    assert!(err.contains("\"--lines\""), "{err}");
}

/// T225.1: `[log] tspin` hands the numbered rows to the `tspin` on `PATH`; a fake tailspin tags
/// each row so the pipe is visible. `off` — and `auto` off a terminal — keep rtok's rendering.
#[cfg(unix)]
fn fake_tspin(home: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = home.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let exe = bin.join("tspin");
    fs::write(
        &exe,
        "#!/bin/sh\n[ \"$1\" = --print ] || exit 2\nsed 's/^/TSPIN /'\n",
    )
    .unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

#[cfg(unix)]
fn rtok_logs_with_tspin(home: &Path, mode: &str) -> String {
    let path = std::env::var("PATH").unwrap_or_default();
    let out = Command::new(bin())
        .args(["logs", "--lines", "2"])
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .env("PATH", format!("{}:{path}", fake_tspin(home).display()))
        .env("RTOK_LOG_TSPIN", mode)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[cfg(unix)]
#[test]
fn tspin_always_pipes_the_numbered_rows_through_the_viewer_on_path() {
    let home = home("tspin-always");
    seed_rotated(&home);
    let out = rtok_logs_with_tspin(&home, "always");
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        ["TSPIN 1 live-4", "TSPIN 2 live-3"],
        "{out}"
    );
}

#[cfg(unix)]
#[test]
fn tspin_auto_and_off_keep_the_builtin_rendering_on_a_pipe() {
    let home = home("tspin-off");
    seed_rotated(&home);
    for mode in ["auto", "off"] {
        let out = rtok_logs_with_tspin(&home, mode);
        assert!(!out.contains("TSPIN"), "{mode}: {out}");
        assert_eq!(out.lines().next(), Some("1 live-4"), "{mode}: {out}");
    }
}

/// T235.3: a `logs watch` whose parent exits stops on its own. `sh` backgrounds the watch
/// (stdout to `/dev/null`, so no failing write can end it) and exits at once; the watch,
/// reparented, must be gone within a few polls.
#[cfg(unix)]
#[test]
fn watch_exits_once_its_parent_is_gone() {
    let home = home("orphan");
    let out = Command::new("sh")
        .args([
            "-c",
            "\"$0\" logs watch >/dev/null 2>&1 </dev/null & echo $!",
            bin(),
        ])
        .env("RTOK_HOME", &home)
        .env("HOME", &home)
        .output()
        .unwrap();
    let pid: i32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while rtok_sys::process_alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let alive = rtok_sys::process_alive(pid);
    if alive {
        rtok_sys::process_kill(pid);
    }
    assert!(!alive, "logs watch {pid} outlived its parent");
    let _ = fs::remove_dir_all(&home);
}
