// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T408: `rtok completions` on a pseudo-terminal opens the shell picker and applies what was
//! checked. The binary runs against a temporary HOME; no agent is started.

#![cfg(unix)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

fn builder(home: &Path, args: &[&str]) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_rtok"));
    cmd.args(args);
    cmd.env_clear();
    cmd.env("HOME", home);
    cmd.env("RTOK_HOME", home);
    cmd.env("TERM", "xterm");
    cmd.cwd(home);
    cmd
}

/// Run `rtok completions` on a terminal: wait for the prompt (keys sent earlier could be flushed
/// when the terminal goes raw), type `keys`, and return what was printed after the child exits.
fn on_a_terminal(home: &Path, keys: &str) -> String {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("a pseudo-terminal");
    let mut child = pair
        .slave
        .spawn_command(builder(home, &["completions"]))
        .expect("rtok starts");
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().expect("reader");
    let mut writer = pair.master.take_writer().expect("writer");
    let seen = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&seen);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            sink.lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let ready = |s: &Arc<Mutex<String>>| s.lock().unwrap().contains("esc cancels");
    while !ready(&seen) {
        assert!(
            Instant::now() < deadline,
            "no prompt: {}",
            seen.lock().unwrap()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    writer.write_all(keys.as_bytes()).expect("keys");
    // Dropping `pair.master` on a timeout hangs the child up, so a stuck run cannot leak.
    while child.try_wait().expect("wait").is_none() {
        assert!(
            Instant::now() < deadline,
            "rtok did not exit: {}",
            seen.lock().unwrap()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(100));
    seen.lock().unwrap().clone()
}

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-t408-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn bash(home: &Path) -> PathBuf {
    home.join(".local/share/bash-completion/completions/rtok")
}

fn zsh(home: &Path) -> PathBuf {
    home.join(".zfunc/_rtok")
}

#[test]
fn checking_a_shell_installs_it() {
    let home = home("install");
    // The first row is bash: space checks it, enter applies.
    let out = on_a_terminal(&home, " \r");
    assert!(bash(&home).exists(), "{out}");
    assert!(out.contains("wrote "), "{out}");
    assert!(!zsh(&home).exists());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn unchecking_an_installed_shell_removes_it() {
    let home = home("remove");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_rtok"))
        .args(["completions", "zsh", "--install"])
        .env_clear()
        .env("HOME", &home)
        .env("RTOK_HOME", &home)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(zsh(&home).exists());
    // Down to the zsh row (pre-checked), space unchecks it, enter applies.
    let out = on_a_terminal(&home, "\x1b[B \r");
    assert!(!zsh(&home).exists(), "{out}");
    assert!(out.contains("removed "), "{out}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn escape_changes_nothing() {
    let home = home("cancel");
    let out = on_a_terminal(&home, " \x1b");
    assert!(!bash(&home).exists(), "{out}");
    assert!(out.contains("cancelled"), "{out}");
    let _ = std::fs::remove_dir_all(&home);
}
