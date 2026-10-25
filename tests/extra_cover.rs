// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Extra coverage for gaps the existing e2e files do not assert (new file only).
//!
//! Deliberately non-duplicative vs `tests/commands_e2e.rs` (bare unknown-id
//! expand, id-presence plugins listing, hook-exits-0 shape) and
//! `tests/plugins_e2e.rs` (guard deny shape, read dedup, tabular toon encode):
//! every case below drives the binary or the public API one step further —
//! fail-open stdin handling, the `--lines` unknown-id path, the plugins table
//! header/row count, PreToolUse rewrite output, deny-beats-rewrite merge
//! order, the `rtok expand <id>` handle inside a guard denial, the read cap
//! marker budget, and a TOON round-trip over comma-bearing cells.

use assert_cmd::Command as AssertCmd;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rtok-extra-{name}-{}-{}",
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

fn cmd(args: &[&str], home: &Path) -> AssertCmd {
    let mut c = AssertCmd::cargo_bin("rtok").unwrap();
    c.args(args).env("RTOK_HOME", home).env("HOME", home);
    // T921's hook deadline would answer `{}` on a slow CI host (cold store on Windows); these
    // tests check the hook's decision, not its latency.
    c.env("RTOK_HOOK_DEADLINE_MS", "60000");
    c
}

fn hook_stdout(event: &str, stdin: &[u8], home: &Path) -> String {
    let out = cmd(&["hook", event], home)
        .write_stdin(stdin)
        .assert()
        .success()
        .get_output()
        .clone();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Fail-open: garbage on stdin still exits 0 and prints the empty object.
#[test]
fn hook_fail_open_bad_stdin_exits_0_with_empty_object() {
    let home = tmp("fail-bad");
    let out = cmd(&["hook", "PreToolUse"], &home)
        .write_stdin(b"not-json{{{".as_slice())
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}");
    let _ = fs::remove_dir_all(&home);
}

/// Fail-open: empty stdin is the same empty object, not a crash.
#[test]
fn hook_fail_open_empty_stdin_exits_0_with_empty_object() {
    let home = tmp("fail-empty");
    let stdout = hook_stdout("PreToolUse", b"", &home);
    assert_eq!(stdout.trim(), "{}");
    let _ = fs::remove_dir_all(&home);
}

/// `expand` of an unknown id fails as unknown even when `--lines` is given,
/// so the flag path cannot mask the lookup error.
#[test]
fn expand_unknown_id_with_lines_flag_still_fails_as_unknown() {
    let home = tmp("expand-flags");
    let out = cmd(
        &["expand", "no-such-id-extra-cover", "--lines", "1-2"],
        &home,
    )
    .assert()
    .failure()
    .get_output()
    .clone();
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(err.contains("unknown archive id"), "{err}");
    assert!(err.contains("no-such-id-extra-cover"), "{err}");
    let _ = fs::remove_dir_all(&home);
}

/// `plugins` prints the header plus exactly the 13 catalogue rows, each with
/// an on/off state and a surface list.
#[test]
fn plugins_lists_thirteen_catalogue_rows_with_header_and_surfaces() {
    let home = tmp("plugins-table");
    let out = cmd(&["plugins"], &home)
        .assert()
        .success()
        .get_output()
        .clone();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut lines = text.lines();
    assert_eq!(
        lines.next().unwrap_or(""),
        "id        enabled   surfaces",
        "{text}"
    );
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), 13, "{text}");
    for id in [
        "measure",
        "cmd",
        "read",
        "json_tree",
        "archive",
        "proxy",
        "inject",
        "guard",
        "memory",
        "graph",
        "toon",
        "compress",
        "docs",
    ] {
        assert!(
            rows.iter().any(|r| r.starts_with(id)),
            "{id} missing:\n{text}"
        );
    }
    for row in &rows {
        assert!(
            row.contains(" on ") || row.contains(" off "),
            "row carries state: {row}"
        );
    }
    assert!(
        ["hook", "proxy", "mcp", "cli"]
            .iter()
            .all(|s| text.contains(s)),
        "surface names appear:\n{text}"
    );
    let _ = fs::remove_dir_all(&home);
}

/// PreToolUse on the Bash fixture rewrites (cmd wrap) instead of denying on a
/// fresh home: `updatedInput` names `rtok run --` and no deny is present.
#[test]
fn hook_pretool_bash_rewrites_via_fixture() {
    let home = tmp("rewrite");
    let stdout = hook_stdout(
        "PreToolUse",
        include_bytes!("fixtures/hooks/pre_tool_bash.json"),
        &home,
    );
    let v: Value = serde_json::from_str(&stdout).unwrap();
    let spec = &v["hookSpecificOutput"];
    assert!(spec.is_object(), "{stdout}");
    assert_eq!(spec["permissionDecision"], Value::Null, "{stdout}");
    let wrapped = spec["updatedInput"]["command"].as_str().unwrap();
    assert!(wrapped.contains("rtok run --"), "{stdout}");
    assert!(
        spec["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .contains("wrapped by rtok"),
        "{stdout}"
    );
    let _ = fs::remove_dir_all(&home);
}

/// Deny beats rewrite: after seeding a repeat Bash result, the same command
/// is denied (no `updatedInput`), even though the cmd rewrite also applies.
#[test]
fn hook_pretool_deny_wins_over_rewrite() {
    let home = tmp("deny-wins");
    let session = "extra-cover-deny-wins";
    let post = format!(
        "{{\"session_id\":\"{session}\",\"cwd\":\"/tmp\",\"tool_name\":\"Bash\",\
         \"tool_input\":{{\"command\":\"cat extra-cover-dup\"}},\
         \"hook_event_name\":\"PostToolUse\",\
         \"tool_response\":{{\"stdout\":\"dup-output-body\"}}}}"
    );
    hook_stdout("PostToolUse", post.as_bytes(), &home);
    let pre = format!(
        "{{\"session_id\":\"{session}\",\"cwd\":\"/tmp\",\"tool_name\":\"Bash\",\
         \"tool_input\":{{\"command\":\"cat extra-cover-dup\"}},\
         \"hook_event_name\":\"PreToolUse\"}}"
    );
    let stdout = hook_stdout("PreToolUse", pre.as_bytes(), &home);
    let v: Value = serde_json::from_str(&stdout).unwrap();
    let spec = &v["hookSpecificOutput"];
    assert_eq!(spec["permissionDecision"], "deny", "{stdout}");
    assert_eq!(spec["updatedInput"], Value::Null, "{stdout}");
    let _ = fs::remove_dir_all(&home);
}

/// A guard denial names `rtok expand <id>`, and that id expands back to the
/// seeded result body through the binary.
#[test]
fn guard_deny_reason_names_expandable_archive() {
    let home = tmp("deny-expand");
    let session = "extra-cover-deny-expand";
    let post = format!(
        "{{\"session_id\":\"{session}\",\"cwd\":\"/tmp\",\"tool_name\":\"Bash\",\
         \"tool_input\":{{\"command\":\"cat extra-cover-dup2\"}},\
         \"hook_event_name\":\"PostToolUse\",\
         \"tool_response\":{{\"stdout\":\"dup-output-body-2\"}}}}"
    );
    hook_stdout("PostToolUse", post.as_bytes(), &home);
    let pre = format!(
        "{{\"session_id\":\"{session}\",\"cwd\":\"/tmp\",\"tool_name\":\"Bash\",\
         \"tool_input\":{{\"command\":\"cat extra-cover-dup2\"}},\
         \"hook_event_name\":\"PreToolUse\"}}"
    );
    let stdout = hook_stdout("PreToolUse", pre.as_bytes(), &home);
    let v: Value = serde_json::from_str(&stdout).unwrap();
    let reason = v["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(reason.contains("rtok expand "), "{reason}");
    let id = reason.rsplit(' ').next().unwrap();
    let out = cmd(&["expand", id], &home)
        .assert()
        .success()
        .get_output()
        .clone();
    let full = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(full.contains("dup-output-body-2"), "{full}");
    let _ = fs::remove_dir_all(&home);
}

/// `read` caps at `max_chars` including the archive marker, and the marker id
/// fetches the full numbered body back.
#[test]
fn read_cap_marker_stays_within_max_chars() {
    let (mut cfg, dir) = rtok::testutil::config("extra-read-cap");
    cfg.plugins.read.allow_paths = vec![dir.clone()];
    cfg.plugins.read.max_chars = 350;
    let cx = rtok::plugin::Runtime::open(cfg, "extra-read-cap").unwrap();
    let blob = "0123456789".repeat(150);
    let path = dir.join("cap.txt");
    fs::write(&path, &blob).unwrap();
    let out = rtok::plugins::read::read(
        &rtok::plugin::Ctx::new(&cx),
        path.to_str().unwrap(),
        "full",
        None,
    )
    .unwrap();
    assert!(out.contains("archived"), "{out}");
    assert!(
        out.chars().count() <= 350,
        "cap includes marker: {}/350",
        out.chars().count()
    );
    let start = out.find("… archived ").expect("archive marker") + "… archived ".len();
    let rest = &out[start..];
    let shown = &rest[..rest.find(" …").expect("archive marker end")];
    assert_eq!(shown.len(), 8, "{out}");
    let id = rtok::expand::resolve_id(&cx, shown).unwrap();
    let archived = String::from_utf8(cx.store.get_archive(&id, None).unwrap().unwrap()).unwrap();
    assert_eq!(archived, format!("1:{blob}"));
    let _ = fs::remove_dir_all(dir);
}

/// TOON keeps comma-bearing cells intact: they encode quoted, one measurement
/// row is recorded, and the archived original fetches back byte-identical.
#[test]
fn toon_comma_cell_round_trips_losslessly() {
    use rtok_plugin_sdk::{Ctx, Plugin, WireRequest};
    let mut cx = rtok::plugin::Runtime::in_memory("extra-toon-comma").unwrap();
    let dir = std::env::temp_dir().join(format!(
        "rtok-extra-toon-comma-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    cx.config.core.archive_dir = dir.clone();
    cx.config.plugins.toon.enabled = true;
    cx.config.plugins.toon.min_rows = 3;
    cx.config.plugins.archive.keep_turns = 0;
    let table = serde_json::json!([
        {"a": 1, "b": "x,y", "c": 3},
        {"a": 4, "b": "p,q", "c": 6},
        {"a": 7, "b": "plain", "c": 9},
    ]);
    let original = serde_json::to_string_pretty(&table).unwrap();
    let mut body = serde_json::json!({"messages": [{
        "role": "user",
        "content": [{
            "type": "tool_result",
            "tool_use_id": "t-comma",
            "content": original,
        }],
    }]});
    let ms = rtok::plugins::toon::ToonPlugin.proxy_filter(
        &mut WireRequest::new(&rtok::proxy::anthropic::ANTHROPIC, &mut body),
        &Ctx::new(&cx),
    );
    assert_eq!(ms.len(), 1);
    assert_eq!(ms[0].plugin, "toon");
    assert!(ms[0].after_bytes < ms[0].before_bytes);
    let id = ms[0].ref_id.clone().unwrap();
    let text = body["messages"][0]["content"][0]["content"]
        .as_str()
        .unwrap();
    assert!(text.starts_with("[toon "), "{text}");
    assert!(
        text.contains("\"x,y\""),
        "comma cell must be quoted: {text}"
    );
    let fetched = rtok::expand::fetch(&cx, &id).unwrap().unwrap();
    assert_eq!(fetched, original.as_bytes());
    let _ = fs::remove_dir_all(dir);
}
