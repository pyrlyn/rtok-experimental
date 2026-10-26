// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T69.4: `rtok memory status` and the Memory plugin page fields.

use rtok::config::Config;
use rtok::plugin::{Measurement, Runtime};
use rtok::web::model;

fn home(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-mem-status-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn memory_page_carries_store_counts() {
    let dir = home("page");
    let cfg = Config::load_from(&dir).expect("config");
    let cx = Runtime::open(cfg.clone(), "mem-page").unwrap();
    cx.store
        .upsert_note(Some("rtok"), "decision", "one", "alpha")
        .unwrap();
    cx.store
        .upsert_note(Some("rtok"), "note", "two", "beta")
        .unwrap();
    cx.store
        .upsert_note(Some("other"), "note", "three", "gamma")
        .unwrap();
    let mem = model::Model::new(&cfg, Some(&cx.store))
        .plugins()
        .into_iter()
        .find(|p| p.id == "memory")
        .unwrap();
    assert!(
        mem.fields
            .iter()
            .any(|(k, v)| k == "notes live" && v == "3"),
        "{:?}",
        mem.fields
    );
}

/// T419: checkpoint prompt counts written through the host reach the memory page summed
/// per session, and leave the plugin's savings total exactly as it was.
#[test]
fn memory_page_sums_checkpoint_counts_without_touching_savings() {
    use rtok_plugin_sdk::Host;
    let dir = home("counts");
    let cfg = Config::load_from(&dir).expect("config");
    let cx = Runtime::open(cfg.clone(), "mem-counts").unwrap();
    cx.record(&Measurement {
        plugin: "memory",
        kind: "handoff",
        before_bytes: 40,
        after_bytes: 10,
        est_before: 10,
        est_after: 3,
        ref_id: None,
        call_id: None,
    })
    .unwrap();
    let memory = || {
        model::Model::new(&cfg, Some(&cx.store))
            .plugins()
            .into_iter()
            .find(|p| p.id == "memory")
            .unwrap()
    };
    let field = |p: &model::PluginPage, key: &str| {
        p.fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| panic!("no {key} in {:?}", p.fields))
    };
    let before = memory();
    assert_eq!(field(&before, "checkpoint prompts typed"), "0");

    cx.plugin_state_set("memory", "checkpoint:s1", r#"{"typed":2,"skipped":1}"#)
        .unwrap();
    cx.plugin_state_set("memory", "session:s1", r#"{"typed":3,"skipped":4}"#)
        .unwrap();
    cx.plugin_state_set("memory", "session:s2", r#"{"typed":1,"skipped":2}"#)
        .unwrap();
    let after = memory();
    assert_eq!(field(&after, "checkpoint prompts typed"), "4");
    assert_eq!(field(&after, "checkpoint host records skipped"), "6");
    assert_eq!(
        serde_json::to_value(before.stats).unwrap(),
        serde_json::to_value(after.stats).unwrap(),
        "the counts are not savings"
    );
}

#[test]
fn memory_status_json_matches_model_type() {
    let dir = home("json");
    let cfg = Config::load_from(&dir).expect("config");
    let cx = Runtime::open(cfg.clone(), "mem-json").unwrap();
    cx.record(&Measurement {
        plugin: "memory",
        kind: "recall",
        before_bytes: 12,
        after_bytes: 4,
        est_before: 3,
        est_after: 1,
        ref_id: Some("sess".into()),
        call_id: None,
    })
    .unwrap();
    let status = model::memory_status(&cfg, None, Some("9999d")).unwrap();
    assert_eq!(status.recall.recalls, 1);
    let v: serde_json::Value = serde_json::to_value(&status).unwrap();
    assert_eq!(v["recall"]["recalls"], 1);
}

/// T304: `session:<id>` rows (`checkpoint.rs`'s per-session handoff note) are unique per
/// session, so `memory status` must exclude them the same way it already excludes
/// `checkpoint:*` — otherwise the aggregate grows one row per historical session forever.
#[test]
fn memory_status_excludes_session_handoff_notes() {
    let dir = home("session-kind");
    let cfg = Config::load_from(&dir).expect("config");
    let cx = Runtime::open(cfg.clone(), "mem-session-kind").unwrap();
    cx.store
        .upsert_note(Some("rtok"), "note", "kept", "alpha")
        .unwrap();
    cx.store
        .upsert_note(Some("rtok"), "session:abc", "handoff", "resume here")
        .unwrap();
    let status = model::memory_status(&cfg, None, Some("9999d")).unwrap();
    assert_eq!(status.notes.live, 1, "{status:?}");
    assert!(
        status
            .by_project
            .iter()
            .flat_map(|p| &p.kinds)
            .all(|k| k.kind != "session:abc"),
        "{status:?}"
    );
}

/// T308: the kind filter matches `checkpoint:<id>` and `session:<id>` with the colon, like
/// `list_notes`: a kind that only starts with `checkpoint` is a real note kind and stays in
/// `memory status` (the old `checkpoint%` pattern dropped it).
#[test]
fn note_aggs_skip_session_and_checkpoint_kinds() {
    let dir = home("aggs-kinds");
    let cfg = Config::load_from(&dir).expect("config");
    let cx = Runtime::open(cfg, "mem-aggs").unwrap();
    for kind in [
        "decision",
        "session:s1",
        "session:s2",
        "checkpoint:c1",
        "checkpointer",
    ] {
        cx.store
            .upsert_note(Some("rtok"), kind, "t", "body")
            .unwrap();
    }
    let kinds: Vec<String> = cx
        .store
        .memory_note_aggs(Some("rtok"))
        .unwrap()
        .into_iter()
        .map(|a| a.kind)
        .collect();
    assert_eq!(kinds, ["checkpointer", "decision"]);
}
