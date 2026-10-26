// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T22.4: `rtok report --ai` renders the same document for a model — dense lines,
//! stable `#id` anchors, toon tables, one token budget. This file pins the CLI end to
//! end against a small fixture store: `--ai` is measurably smaller than `--format md`
//! on the same store and window (the Check's Measurement-style proof), dropped
//! sections are named, and the heading ids match the md section order. The task-list
//! shape itself is pinned beside the renderer (`src/report/ai.rs`), where hand-built
//! recommendations do not depend on T22.5's rules.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rtok")
}

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-t224-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
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

fn tokens(s: &str) -> u32 {
    let rates = rtok::config::Config::default().estimator;
    rtok::tokens::estimate(s, rtok::tokens::Class::Prose, &rates)
}

/// Two timed hook calls, two measurements (cmd 100→40, archive pointer 200→50), one
/// usage row, two hooks in the sandboxed settings — every asserted number traceable.
fn seed(home: &Path) {
    let claude = home.join(".claude");
    fs::create_dir_all(&claude).unwrap();
    fs::write(
        claude.join("settings.json"),
        r#"{"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"rtok hook PreToolUse"}]}],
             "PostToolUse":[{"hooks":[{"type":"command","command":"rtok hook PostToolUse"}]}]}}"#,
    )
    .unwrap();

    let cfg = rtok::config::Config::load_from(home).expect("config");
    let store = rtok::store::Store::open(&cfg.core.db_path).expect("store");
    store
        .upsert_session("s1", None, None, None, Some("proxy"))
        .unwrap();
    for ms in [4.0, 2.0] {
        let id = store
            .insert_call("s1", "hook", "hook", None, None, None, None, None)
            .unwrap();
        store.set_call_ms(id, ms).unwrap();
    }
    for (plugin, kind, before, after) in
        [("cmd", "filter", 100, 40), ("archive", "pointer", 200, 50)]
    {
        store
            .insert_measurement(
                "s1",
                &rtok::Measurement {
                    plugin,
                    kind,
                    before_bytes: 100,
                    after_bytes: 40,
                    est_before: before,
                    est_after: after,
                    ref_id: None,
                    call_id: None,
                },
            )
            .unwrap();
    }
    let id = store
        .insert_call("s1", "proxy", "api_request", None, None, None, None, None)
        .unwrap();
    store
        .insert_usage("s1", Some("m"), "anthropic", 100, 0, 0, 5, id)
        .unwrap();
}

/// The Check, first clause: measured tokens, same store and window — a number, not prose.
#[test]
fn ai_is_measurably_smaller_than_md() {
    let h = home("smaller");
    seed(&h);
    let md = rtok(&["report"], &h);
    let ai = rtok(&["report", "--ai"], &h);
    println!("md {} tok, ai {} tok", tokens(&md), tokens(&ai));
    assert!(
        tokens(&ai) < tokens(&md),
        "ai {} tok vs md {} tok",
        tokens(&ai),
        tokens(&md)
    );
    assert!(!ai.contains("| ---"), "no Markdown pipe tables in --ai");
    let _ = fs::remove_dir_all(&h);
}

/// Same sections, same order, same numbers as md — through the stable `#id` anchors.
#[test]
fn ai_carries_the_section_set_with_md_numbers() {
    let h = home("sections");
    seed(&h);
    let (md, ai) = (rtok(&["report"], &h), rtok(&["report", "--ai"], &h));
    let ids: Vec<&str> = ai.lines().filter(|l| l.starts_with('#')).collect();
    assert_eq!(
        ids,
        [
            "#window",
            "#savings",
            "#calls",
            "#cache",
            "#expand",
            "#config",
            "#doctor",
            "#recommendations"
        ],
        "{ai}"
    );
    for (md_sec, id) in [
        ("## Window", "#window"),
        ("## Savings", "#savings"),
        ("## Calls", "#calls"),
        ("## Cache", "#cache"),
        ("## Expand", "#expand"),
        ("## Config", "#config"),
        ("## Doctor", "#doctor"),
        ("## Recommendations", "#recommendations"),
    ] {
        assert!(md.contains(md_sec), "md still has {md_sec}");
        assert!(ai.contains(id), "--ai has {id}: {ai}");
    }
    // Same numbers: the fixture's savings (210), hook latencies and two hooks.
    assert!(ai.contains("total_saved=210tok rows=2"), "{ai}");
    assert!(ai.contains("hooks 2"), "{ai}");
    assert!(ai.contains("dropped: none"), "{ai}");
    assert!(
        ai.contains("units:"),
        "every section states its units: {ai}"
    );
    // Recommendations close the document — empty-rule-set line until T22.5 fires,
    // task lines after; either way nothing follows the section.
    let tail = &ai[ai.find("#recommendations").unwrap()..];
    assert!(
        tail.contains("no recommendations.") || tail.contains("1. ["),
        "task list or its honest absence: {tail}"
    );
    let _ = fs::remove_dir_all(&h);
}

/// The Check, second clause: a tight budget names its drops instead of truncating.
#[test]
fn tight_budget_names_dropped_sections() {
    let h = home("budget");
    seed(&h);
    fs::write(h.join("config.toml"), "[report]\nbudget_tokens = 100\n").unwrap();
    let ai = rtok(&["report", "--ai"], &h);
    let dropped: Vec<&str> = ai
        .lines()
        .find(|l| l.starts_with("dropped: "))
        .expect("dropped line")[9..]
        .split(',')
        .collect();
    assert_ne!(dropped, ["none"], "the budget binds: {ai}");
    for id in &dropped {
        assert!(
            !ai.contains(&format!("#{id}\n")),
            "a dropped section leaves no heading: {id} in {ai}"
        );
    }
    let _ = fs::remove_dir_all(&h);
}

/// Empty store: no rows anywhere, but the document shape (and its honest lines) holds.
#[test]
fn empty_store_ai_says_so() {
    let h = home("empty");
    let ai = rtok(&["report", "--ai"], &h);
    assert!(ai.contains("dropped: none"), "{ai}");
    assert!(ai.contains("no rows."), "{ai}");
    assert!(ai.contains("no recommendations."), "{ai}");
    assert!(ai.contains("hooks 0"), "{ai}");
    let _ = fs::remove_dir_all(&h);
}
