// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T38.3: e2e for `memory import` and `graph index` through the binary.
//!
//! Check: an imported JSONL note is found by `mem_search`, and an indexed
//! tree answers `symbol` — both driven over `rtok mcp` stdio JSON-RPC.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rtok")
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rtok-t383-{name}-{}-{}",
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

fn rtok(args: &[&str], home: &Path) -> (String, String, i32) {
    let out = Command::new(bin())
        .args(args)
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .output()
        .unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn ok(args: &[&str], home: &Path) -> String {
    let (stdout, stderr, code) = rtok(args, home);
    assert_eq!(code, 0, "rtok {args:?} exit {code}: {stderr}");
    stdout
}

/// One `tools/call` against a fresh `rtok mcp` process rooted at `cwd`.
fn call(home: &Path, cwd: &Path, tool: &str, args: serde_json::Value) -> String {
    let mut child = Command::new(bin())
        .arg("mcp")
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtok mcp");
    let req = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": tool, "arguments": args}
    });
    child
        .stdin
        .take()
        .unwrap()
        .write_all(req.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("one JSON-RPC line");
    v["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no text in {stdout}"))
        .to_string()
}

#[test]
fn memory_import_is_searchable() {
    let home = tmp("memory");
    let fixture = home.join("notes.jsonl");
    fs::write(
        &fixture,
        "{\"kind\":\"note\",\"title\":\"t383 quokka title\",\"body\":\"quokka beacon body t383\"}\n",
    )
    .unwrap();
    let arg = fixture.to_string_lossy().into_owned();
    let out = ok(&["memory", "import", &arg], &home);
    assert!(out.contains("inserted 1"), "{out}");
    let text = call(
        &home,
        &home,
        "mem_search",
        serde_json::json!({"query": "quokka"}),
    );
    assert!(text.contains("t383 quokka title"), "{text}");
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn graph_index_finds_symbol() {
    let home = tmp("graph");
    let repo = home.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("a.rs"), "fn t383_beacon() {}\n").unwrap();
    fs::write(
        repo.join("b.rs"),
        "fn t383_other() {\n    t383_beacon();\n}\n",
    )
    .unwrap();
    let arg = repo.to_string_lossy().into_owned();
    let out = ok(&["graph", "index", &arg], &home);
    assert!(out.contains("indexed"), "{out}");
    let text = call(
        &home,
        &repo,
        "symbol",
        serde_json::json!({"name": "t383_beacon"}),
    );
    assert!(
        text.contains("a.rs:1") && text.contains("fn t383_beacon"),
        "{text}"
    );
    let _ = fs::remove_dir_all(&home);
}
