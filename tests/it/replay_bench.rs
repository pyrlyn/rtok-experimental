// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T241: saving over a whole session mix, not one call at a time (T238/T240/T239 each
//! measure a single call). Replays `tests/fixtures/replay/session.jsonl` (~30-event corpus
//! shaped like a real Claude Code session; see its header comment for the tool-mix source)
//! through the real hook/run/mcp surfaces in a temp home, then sums the `Measurement` rows
//! `Store::list_measurements` actually recorded — no number here is invented.
//!
//! Known bug, not this task's to fix (`src/plugins/cmd/run.rs` `emit_filtered`): its
//! `[rtok <id8> · N lines]` trailer is not counted in `after_bytes`, so the `cmd`
//! row (and this bench's `cmd` share) is an upper bound until that fix lands.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Home(PathBuf);
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn tmp(n: &str) -> Home {
    let t = std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("rtok-t241-{n}-{}-{t}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Home(d)
}
fn run(home: &Home, args: &[&str], input: &str, cwd: &Path) -> String {
    let mut c = Command::new(env!("CARGO_BIN_EXE_rtok"));
    c.args(args)
        .env("RTOK_HOME", &home.0)
        .env("HOME", &home.0)
        .current_dir(cwd);
    c.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    drop(child.stdin.take().unwrap().write_all(input.as_bytes()));
    let out = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{args:?} {err}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn js(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Event {
    Bash {
        label: String,
        body: String,
    },
    Read {
        path: String,
        mode: String,
        #[serde(default)]
        body: Option<String>,
    },
    Search {
        pattern: String,
        path: String,
    },
}

fn events() -> Vec<Event> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/replay/session.jsonl");
    let raw = std::fs::read_to_string(&p).unwrap();
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{l}: {e}")))
        .collect()
}

fn rpc(id: u32, name: &str, args: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}})
        .to_string()
}

/// Bash goes through `PreToolUse` then `rtok run --`, like a real hook rewrite + host
/// execution — only when the hook actually rewrote (a disabled `cmd` plugin leaves
/// `tool_input` untouched, so nothing is measured, same as a host running the original
/// command unshortened). `read`/`search` calls are queued into one JSON-RPC batch sent to
/// a single `rtok mcp` process: re-read dedup is keyed per session (one process = one
/// session, T239's `read_dedup_on_second_mcp_read`).
fn replay(home: &Home) {
    let mut batch = String::new();
    let mut id = 0u32;
    for ev in events() {
        match ev {
            Event::Bash { label, body } => {
                let file = home.0.join(format!("out-{label}.txt"));
                std::fs::write(&file, &body).unwrap();
                let cmd = format!("cat {}", file.display());
                let pre = json!({
                    "session_id": "replay", "cwd": home.0, "tool_name": "Bash",
                    "tool_input": {"command": cmd}, "hook_event_name": "PreToolUse",
                })
                .to_string();
                let out = run(home, &["hook", "PreToolUse"], &pre, &home.0);
                let modified = js(&out)["hookSpecificOutput"]["updatedInput"]["command"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                if modified.contains("rtok run -- ") {
                    run(home, &["run", "cat", file.to_str().unwrap()], "", &home.0);
                }
            }
            Event::Read { path, mode, body } => {
                if let Some(b) = body {
                    let full = home.0.join(&path);
                    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
                    std::fs::write(&full, b).unwrap();
                }
                id += 1;
                batch.push_str(&rpc(id, "read", json!({"path": path, "mode": mode})));
                batch.push('\n');
            }
            Event::Search { pattern, path } => {
                id += 1;
                batch.push_str(&rpc(
                    id,
                    "search",
                    json!({"pattern": pattern, "path": path}),
                ));
                batch.push('\n');
            }
        }
    }
    if !batch.is_empty() {
        run(home, &["mcp"], &batch, &home.0);
    }
}

#[derive(Debug)]
struct PluginReport {
    plugin: &'static str,
    calls: usize,
    before: i64,
    after: i64,
}

/// Sums straight from `Store::list_measurements` — the only source of truth for what was
/// actually saved (`AGENTS.md`: "No `Measurement` row = no saving, in code or prose").
fn measurement_report(home: &Home) -> Vec<PluginReport> {
    let store = rtok::store::Store::open(&home.0.join("rtok.db")).unwrap();
    ["cmd", "read"]
        .iter()
        .map(|&plugin| {
            let rows = store.list_measurements(plugin).unwrap();
            let before = rows.iter().map(|r| r.est_before as i64).sum();
            let after = rows.iter().map(|r| r.est_after as i64).sum();
            PluginReport {
                plugin,
                calls: rows.len(),
                before,
                after,
            }
        })
        .collect()
}

fn pct_saved(before: i64, after: i64) -> f64 {
    if before == 0 {
        0.0
    } else {
        (before - after) as f64 * 100.0 / before as f64
    }
}

fn print_row(plugin: &str, calls: usize, before: i64, after: i64) {
    let pct = pct_saved(before, after);
    println!("{plugin:<6} {calls:>6} {before:>11} {after:>10} {pct:>7.1}%");
}

/// Prints the per-plugin table (`--nocapture`) and returns `(total_before, total_after)`.
fn print_table(report: &[PluginReport]) -> (i64, i64) {
    println!(
        "{:<6} {:>6} {:>11} {:>10} {:>8}",
        "plugin", "calls", "est_before", "est_after", "saving"
    );
    let (mut before, mut after, mut calls) = (0i64, 0i64, 0usize);
    for r in report {
        print_row(r.plugin, r.calls, r.before, r.after);
        before += r.before;
        after += r.after;
        calls += r.calls;
    }
    print_row("total", calls, before, after);
    (before, after)
}

/// First run 2026-09-24 (`cargo test --test replay_bench -- --nocapture`, recorded in
/// `research.md` §2): total saving 81.1% (`cmd` 84.2% of 23,075 est. tokens, `read` 52.3%
/// of 2,482). The floor sits a few points below that so normal estimator/formatter drift
/// doesn't flake the gate.
const FLOOR_PCT: f64 = 76.0;

#[test]
fn replay_session_saves_over_the_floor() {
    let home = tmp("replay");
    replay(&home);
    let report = measurement_report(&home);
    assert!(
        report.iter().any(|r| r.plugin == "cmd" && r.calls > 0),
        "no cmd rows: {report:?}"
    );
    assert!(
        report.iter().any(|r| r.plugin == "read" && r.calls > 0),
        "no read rows: {report:?}"
    );
    let (before, after) = print_table(&report);
    assert!(
        after <= before,
        "after {after} > before {before}: {report:?}"
    );
    let pct = pct_saved(before, after);
    assert!(
        pct >= FLOOR_PCT,
        "total saving {pct:.1}% below the {FLOOR_PCT}% floor: {report:?}"
    );
}

/// Mutation check (T241's `Check` line): with `cmd` disabled its whole share of the corpus
/// goes unshortened (`PreToolUse` never rewrites, so `replay` never calls `rtok run --`),
/// and the total must fall back under the floor — the floor is load-bearing, not
/// trivially satisfied by `read` alone.
#[test]
fn disabling_cmd_plugin_drops_the_total_below_the_floor() {
    let home = tmp("replay-mut");
    std::fs::write(
        home.0.join("config.toml"),
        "[plugins.cmd]\nenabled = false\n",
    )
    .unwrap();
    replay(&home);
    let report = measurement_report(&home);
    assert_eq!(
        report.iter().find(|r| r.plugin == "cmd").unwrap().calls,
        0,
        "cmd rows recorded although the plugin is disabled: {report:?}"
    );
    let (before, after) = print_table(&report);
    let pct = pct_saved(before, after);
    assert!(
        pct < FLOOR_PCT,
        "disabling cmd should drop the total below the floor, got {pct:.1}% (floor {FLOOR_PCT}%): {report:?}"
    );
}
