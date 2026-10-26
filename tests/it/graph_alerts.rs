// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T329.17: a linked project that disappears is reported by the health check of a running
//! `rtok mcp`, seen from another process (`rtok graph projects --json`) and in the answer of a
//! graph tool, and the alert clears when the directory comes back.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-t32917-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs_kit::path::canonicalize(&dir).unwrap()
}

fn rtok(home: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .args(args)
        .current_dir(home)
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .output()
        .expect("rtok runs");
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn alerts_of(home: &Path, name: &str) -> Vec<Value> {
    let rows: Vec<Value> =
        serde_json::from_str(&rtok(home, &["graph", "projects", "--json"])).unwrap();
    let row = rows.iter().find(|r| r["name"] == name).unwrap();
    row["alerts"].as_array().cloned().unwrap_or_default()
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// A long-lived `rtok mcp` rooted at `cwd`; closing its stdin ends it.
struct Mcp {
    child: Child,
    stdin: Option<ChildStdin>,
    out: BufReader<ChildStdout>,
}

impl Mcp {
    fn start(home: &Path, cwd: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
            .arg("mcp")
            .env("RTOK_HOME", home)
            .env("HOME", home)
            .env("RTOK_PLUGINS_GRAPH_HEALTH_CHECK_INTERVAL_S", "1")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn rtok mcp");
        let stdin = child.stdin.take();
        let out = BufReader::new(child.stdout.take().unwrap());
        Self { child, stdin, out }
    }

    fn call(&mut self, tool: &str, args: Value) -> String {
        let req = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": tool, "arguments": args}
        });
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{req}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.out.read_line(&mut line).unwrap();
        let v: Value = serde_json::from_str(line.trim()).expect("one JSON-RPC line");
        v["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    fn finish(mut self) {
        drop(self.stdin.take());
        let _ = self.child.wait();
    }
}

#[test]
fn a_moved_project_is_reported_everywhere_and_clears_when_it_returns() {
    let home = fixture("moved");
    let (a, b) = (home.join("alpha"), home.join("beta"));
    for (dir, src) in [
        (&a, "fn used() {}\nfn main() { used(); }\n"),
        (&b, "fn beta() { }\n"),
    ] {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("lib.rs"), src).unwrap();
        rtok(&home, &["graph", "projects", "add", dir.to_str().unwrap()]);
    }
    rtok(
        &home,
        &[
            "graph",
            "projects",
            "link",
            b.to_str().unwrap(),
            "--from",
            a.to_str().unwrap(),
        ],
    );

    let mut mcp = Mcp::start(&home, &a);
    let moved = home.join("beta.moved");
    fs::rename(&b, &moved).unwrap();
    // Another process sees what the running server's health check raised, after two checks.
    wait_until("the alert", || !alerts_of(&home, "beta").is_empty());
    assert_eq!(alerts_of(&home, "beta")[0]["kind"], "missing");

    let answer = mcp.call("callers", serde_json::json!({"name": "used"}));
    assert!(
        answer.starts_with("notice: beta missing since "),
        "{answer}"
    );
    assert!(answer.contains("; results exclude beta\n"), "{answer}");

    fs::rename(&moved, &b).unwrap();
    wait_until("the alert to clear", || alerts_of(&home, "beta").is_empty());
    let answer = mcp.call("callers", serde_json::json!({"name": "used"}));
    assert!(!answer.contains("notice:"), "{answer}");
    mcp.finish();
    let _ = fs::remove_dir_all(&home);
}
