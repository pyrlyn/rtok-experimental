// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T331.7: `rtok doctor --fix` on a pseudo-terminal asks what to remove, and writes only what the
//! user confirmed. The binary runs against a temporary HOME; no agent is started.

#![cfg(unix)]

use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

/// Run `rtok doctor --fix` in `home` on a terminal, type `input`, and return what it printed.
fn on_a_terminal(home: &Path, input: &str) -> String {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 40,
            cols: 160,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("a pseudo-terminal");
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_rtok"));
    cmd.args(["doctor", "--fix"]);
    cmd.env("HOME", home);
    cmd.env("RTOK_HOME", home);
    cmd.cwd(home);
    let mut child = pair.slave.spawn_command(cmd).expect("rtok starts");
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().expect("reader");
    let mut writer = pair.master.take_writer().expect("writer");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = String::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            out.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        let _ = tx.send(out);
    });
    writer.write_all(input.as_bytes()).expect("keys");
    child.wait().expect("rtok exits");
    drop(writer);
    drop(pair.master);
    rx.recv_timeout(Duration::from_secs(20))
        .expect("the output ends")
}

fn settings(home: &Path) -> std::path::PathBuf {
    let dir = home.join(".claude");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("settings.json")
}

fn broken(home: &Path) -> String {
    format!(
        r#"{{"hooks": {{"Stop": [{{"hooks": [{{"type": "command", "command": "{}/gone.sh"}}]}}]}}}}"#,
        home.display()
    )
}

fn home(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-t331-7-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_checklist_writes_only_after_the_last_confirmation() {
    let home = home("write");
    let file = settings(&home);
    let raw = broken(&home);
    std::fs::write(&file, &raw).unwrap();
    let out = on_a_terminal(&home, "y\ny\n");
    assert!(out.contains("Fix selected"), "{out}");
    assert!(out.contains("[x] broken hook Stop"), "{out}");
    assert!(out.contains("1 entry removed, 0 left"), "{out}");
    assert!(!std::fs::read_to_string(&file).unwrap().contains("gone.sh"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn quitting_the_checklist_writes_nothing() {
    let home = home("quit");
    let file = settings(&home);
    let raw = broken(&home);
    std::fs::write(&file, &raw).unwrap();
    let out = on_a_terminal(&home, "q\n");
    assert!(out.contains("cancelled: nothing was written"), "{out}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), raw);
    let _ = std::fs::remove_dir_all(&home);
}
