// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T130: `SubagentStart` spawn brief. `research.md` §17.3(2) picked `SubagentStart`'s
//! `additionalContext` over `PreToolUse`'s `updatedInput`, which
//! <https://code.claude.com/docs/en/hooks> (checked 2026-09-22) documents as ignored by the
//! `Agent`/`Task` tools — see `plan/T130.1/`.

use assert_cmd::Command as AssertCmd;
use serde_json::{Value, json};
use std::path::PathBuf;

struct Home(PathBuf);
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp(name: &str) -> Home {
    let t = std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("rtok-t130-{name}-{}-{t}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Home(d)
}

fn hook(home: &Home, event: &str, body: &Value) -> Value {
    hook_as(home, &[], event, body)
}

/// `rtok hook <event> [extra…]`, e.g. `--host copilot`.
fn hook_as(home: &Home, extra: &[&str], event: &str, body: &Value) -> Value {
    let out = AssertCmd::cargo_bin("rtok")
        .unwrap()
        .args(["hook", event])
        .args(extra)
        .env("RTOK_HOME", &home.0)
        .env("HOME", &home.0)
        .write_stdin(serde_json::to_vec(body).unwrap())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out)))
}

fn disable_spawn_brief(home: &Home) {
    std::fs::write(
        home.0.join("config.toml"),
        "[plugins.memory]\nspawn_brief = false\n",
    )
    .unwrap();
}

fn enable_spawn_brief(home: &Home) {
    std::fs::write(
        home.0.join("config.toml"),
        "[plugins.memory]\nspawn_brief = true\nspawn_brief_tokens = 60\n",
    )
    .unwrap();
}

fn read_tool(session: &str, path: &str) -> Value {
    json!({
        "session_id": session,
        "cwd": "/tmp",
        "hook_event_name": "PreToolUse",
        "tool_name": "Read",
        "tool_input": {"file_path": path},
    })
}

fn subagent_start(session: &str) -> Value {
    json!({
        "session_id": session,
        "cwd": "/tmp",
        "hook_event_name": "SubagentStart",
        "agent_id": "agent-1",
        "agent_type": "general-purpose",
        "task_description": "look at /repo/a.rs",
    })
}

fn brief_of(v: &Value) -> Option<&str> {
    v["hookSpecificOutput"]["additionalContext"].as_str()
}

/// T283: `SubagentStart` now always leads with the sub-agent's own `rtok agent id: ...`
/// line ahead of any spawn brief. Strips it so these cases can still assert on the brief's
/// own text — empty when nothing fired, exactly as before that line existed.
fn brief_body(v: &Value) -> String {
    let ctx = brief_of(v).unwrap_or("");
    match ctx.strip_prefix("rtok agent id: ") {
        Some(rest) => rest.split_once('\n').map_or("", |(_, r)| r).to_string(),
        None => ctx.to_string(),
    }
}

/// T131: the `memory`/`brief` row from [`rtok::store::Store::measurement_totals`], if the
/// home's store has ever recorded one. Opened after the `rtok hook` subprocess exits, from
/// the same `<home>/config.toml` it wrote through (`Config::load_from` resolves the same
/// `db_path` the CLI used under `RTOK_HOME`).
fn brief_measurement(home: &Home) -> Option<rtok::store::MeasurementTotal> {
    let cfg = rtok::config::Config::load_from(&home.0).expect("config");
    let cx = rtok::plugin::Runtime::open(cfg, "check").unwrap();
    cx.store
        .measurement_totals()
        .unwrap()
        .into_iter()
        .find(|t| t.plugin == "memory" && t.kind == "brief")
}

#[test]
fn flag_off_is_a_passthrough() {
    let home = tmp("off");
    disable_spawn_brief(&home);
    let session = "s-off";
    let _ = hook(&home, "PreToolUse", &read_tool(session, "/repo/a.rs"));
    let out = hook(&home, "SubagentStart", &subagent_start(session));
    assert_eq!(brief_body(&out), "", "spawn_brief off is a passthrough");
}

#[test]
fn empty_ledger_is_a_passthrough() {
    let home = tmp("empty");
    enable_spawn_brief(&home);
    let out = hook(&home, "SubagentStart", &subagent_start("s-empty"));
    assert_eq!(brief_body(&out), "", "nothing read or edited yet");
}

#[test]
fn brief_carries_pointers_an_expand_id_and_stays_under_budget() {
    let home = tmp("brief");
    enable_spawn_brief(&home);
    let session = "s-brief";
    let _ = hook(&home, "PreToolUse", &read_tool(session, "/repo/a.rs"));
    let _ = hook(&home, "PreToolUse", &read_tool(session, "/repo/b.rs"));
    let first = hook(&home, "SubagentStart", &subagent_start(session));
    let text = brief_of(&first).expect("a non-empty ledger must offer a brief");
    assert!(text.contains("/repo/a.rs"), "{text}");
    assert!(text.contains("rtok expand"), "{text}");
    assert!(text.contains("rtok read(mode=lines"), "{text}");

    let second = hook(&home, "SubagentStart", &subagent_start(session));
    assert_eq!(first, second, "an unchanged ledger must be byte-stable");
}

/// T131: a brief that fires must leave exactly one cost `Measurement` row (`plugin: "memory"`,
/// `kind: "brief"`) with `before = 0` and `after > 0` — a saving that is not a `Measurement`
/// row does not exist, and the brief is a cost first.
#[test]
fn a_fired_brief_writes_exactly_one_cost_measurement() {
    let home = tmp("measured");
    enable_spawn_brief(&home);
    let session = "s-measured";
    let _ = hook(&home, "PreToolUse", &read_tool(session, "/repo/a.rs"));
    let out = hook(&home, "SubagentStart", &subagent_start(session));
    assert!(brief_of(&out).is_some(), "a brief must have fired");

    let m = brief_measurement(&home).expect("a fired brief must leave a Measurement row");
    assert_eq!(m.rows, 1, "{m:?}");
    assert_eq!(m.est_before, 0, "{m:?}");
    assert!(m.est_after > 0, "{m:?}");
}

/// No brief emitted (flag off, or an empty ledger) → no row: `plugin.rs`'s `record` is only
/// ever called from inside `build_brief`'s `Some` path.
#[test]
fn no_brief_leaves_no_cost_measurement() {
    let home = tmp("unmeasured");
    enable_spawn_brief(&home);
    // Empty ledger: nothing read or edited before `SubagentStart`, so `build_brief` returns
    // `None` and never reaches `cx.record`.
    let out = hook(&home, "SubagentStart", &subagent_start("s-unmeasured"));
    assert!(brief_body(&out).is_empty());
    assert!(brief_measurement(&home).is_none(), "no brief, no row");
}

#[test]
fn non_subagent_events_are_untouched() {
    let home = tmp("other");
    enable_spawn_brief(&home);
    let session = "s-other";
    let _ = hook(&home, "PreToolUse", &read_tool(session, "/repo/a.rs"));
    let out = hook(
        &home,
        "PreToolUse",
        &json!({
            "session_id": session,
            "cwd": "/tmp",
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "echo hi"},
        }),
    );
    assert!(
        out.get("hookSpecificOutput").is_none()
            || out["hookSpecificOutput"]["additionalContext"].is_null(),
        "{out}"
    );
}

/// T262.5: a host adapter's input reaches the ledger. Copilot sends camelCase (`toolName: view`,
/// `toolArgs.path`); the call row used to keep that raw body, which the ledger's `tool_name`
/// scan never matched, so a Copilot session's brief was always empty.
#[test]
fn copilot_reads_reach_the_brief() {
    let home = tmp("copilot");
    enable_spawn_brief(&home);
    let session = "s-copilot";
    let read = json!({"sessionId": session, "cwd": "/tmp", "toolName": "view", "toolArgs": {"path": "/repo/c.rs"}});
    let _ = hook_as(&home, &["--host", "copilot"], "PreToolUse", &read);
    let out = hook(&home, "SubagentStart", &subagent_start(session));
    let text = brief_of(&out).expect("the Copilot read must reach the ledger");
    assert!(text.contains("/repo/c.rs"), "{text}");

    // T262.4: Copilot's own `subagentStart` payload gets the brief as flat `additionalContext`.
    let spawn =
        json!({"sessionId": session, "timestamp": 1, "cwd": "/tmp", "agentName": "explore"});
    let out = hook_as(&home, &["--host", "copilot"], "SubagentStart", &spawn);
    let text = out["additionalContext"]
        .as_str()
        .unwrap_or_else(|| panic!("{out}"));
    assert!(text.contains("/repo/c.rs"), "{text}");
}
