// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T70.2: `rtok archive rewrite --stdin` — the pi `context` carrier, through the binary.
//!
//! The card's Check: a 3-turn fixture keeps turns 0–1 whole, the pointer appears
//! exactly outside `keep_turns`, `rtok expand <id>` returns the original, a
//! replay is byte-identical, and one new turn ages exactly one more block.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn rtok(home: &Path, args: &[&str], stdin: &[u8]) -> Vec<u8> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .args(args)
        .env("RTOK_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtok");
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-arw-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn big(tag: &str) -> String {
    (1..=400)
        .map(|i| format!("{tag} line {i}: some shell output with words"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A pi session array: user/assistant/toolResult triples, newest last — the
/// exact message shape of a real `~/.pi/agent/sessions/**.jsonl` session.
fn pi_array(tags: &[&str]) -> serde_json::Value {
    let mut messages = Vec::new();
    for (i, tag) in tags.iter().enumerate() {
        messages.push(serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": format!("prompt {i}")}]
        }));
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "working…"}]
        }));
        messages.push(serde_json::json!({
            "role": "toolResult",
            "toolCallId": format!("call_{tag}"),
            "toolName": "bash",
            "content": [{"type": "text", "text": big(tag)}],
            "isError": false
        }));
    }
    serde_json::Value::Array(messages)
}

fn results(v: &serde_json::Value) -> Vec<&serde_json::Value> {
    v.as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "toolResult")
        .collect()
}

fn result_text<'a>(v: &'a serde_json::Value, tag: &str) -> &'a str {
    let m = v
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["toolCallId"] == format!("call_{tag}"))
        .unwrap();
    m["content"][0]["text"].as_str().unwrap()
}

fn expand_id(pointer: &str) -> &str {
    pointer
        .split("expand(")
        .nth(1)
        .unwrap()
        .split(')')
        .next()
        .unwrap()
}

#[test]
fn rewrite_shrinks_only_outside_keep_turns_expand_recovers_and_replays_stable() {
    let home = tmp("three-turn");
    std::fs::write(
        home.join("config.toml"),
        "[plugins.archive]\nkeep_turns = 1\n",
    )
    .unwrap();
    let input = serde_json::to_vec(&pi_array(&["a", "b", "c"])).unwrap();

    let out1 = rtok(&home, &["archive", "rewrite", "--stdin"], &input);
    assert_ne!(out1, input, "the two oldest turns are rewritten");
    let v1: serde_json::Value = serde_json::from_slice(&out1).unwrap();
    let rs = results(&v1);
    assert_eq!(rs.len(), 3);
    assert!(
        result_text(&v1, "a").starts_with("[archived "),
        "turn 2 (oldest) carries the pointer"
    );
    assert!(
        result_text(&v1, "b").starts_with("[archived "),
        "turn 1 is outside keep_turns = 1, so it carries the pointer too"
    );
    assert!(
        result_text(&v1, "c").starts_with("c line 1:"),
        "turn 0 stays whole"
    );

    // Lossless: `rtok expand <id>` returns the original bytes.
    let id = expand_id(result_text(&v1, "a")).to_string();
    let back = rtok(&home, &["expand", &id], b"");
    assert_eq!(String::from_utf8(back).unwrap(), big("a"));

    // Byte-stability: the same array in rewrites to the same bytes out.
    let out2 = rtok(&home, &["archive", "rewrite", "--stdin"], &out1);
    assert_eq!(out1, out2, "replay is byte-identical");
    let out3 = rtok(&home, &["archive", "rewrite", "--stdin"], &out2);
    assert_eq!(out2, out3, "third replay is byte-identical");

    // One new turn: exactly one more block ages into a pointer.
    let mut grown: serde_json::Value = serde_json::from_slice(&out3).unwrap();
    grown.as_array_mut().unwrap().extend([
        serde_json::json!({"role": "user", "content": [{"type": "text", "text": "prompt 3"}]}),
        serde_json::json!({"role": "assistant", "content": [{"type": "text", "text": "…"}]}),
        serde_json::json!({"role": "toolResult", "toolCallId": "call_d", "toolName": "bash",
            "content": [{"type": "text", "text": big("d")}], "isError": false}),
    ]);
    let out4 = rtok(
        &home,
        &["archive", "rewrite", "--stdin"],
        &serde_json::to_vec(&grown).unwrap(),
    );
    let v4: serde_json::Value = serde_json::from_slice(&out4).unwrap();
    assert!(
        result_text(&v4, "a").starts_with("[archived "),
        "the earlier pointer is replayed, not re-archived"
    );
    assert!(
        result_text(&v4, "b").starts_with("[archived "),
        "the persisted pointer survives the new turn"
    );
    assert!(
        result_text(&v4, "c").starts_with("[archived "),
        "the newly-aged block is the only new pointer"
    );
    assert!(
        result_text(&v4, "d").starts_with("d line 1:"),
        "the live edge stays whole"
    );
    let id_b = expand_id(result_text(&v4, "b")).to_string();
    let back_b = rtok(&home, &["expand", &id_b], b"");
    assert_eq!(String::from_utf8(back_b).unwrap(), big("b"));

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn quiet_or_invalid_input_is_echoed_verbatim() {
    let home = tmp("echo");
    let junk = b"this is not json";
    assert_eq!(
        rtok(&home, &["archive", "rewrite", "--stdin"], junk),
        junk,
        "invalid JSON fails open with the input bytes"
    );
    // Valid JSON, but nothing outside keep_turns: echoed too (no-change contract).
    std::fs::write(
        home.join("config.toml"),
        "[plugins.archive]\nkeep_turns = 1\n",
    )
    .unwrap();
    let quiet = serde_json::to_vec(&pi_array(&["a"])).unwrap();
    assert_eq!(
        rtok(&home, &["archive", "rewrite", "--stdin"], &quiet),
        quiet,
        "a single-turn array has nothing outside the live zone"
    );
    let _ = std::fs::remove_dir_all(&home);
}
