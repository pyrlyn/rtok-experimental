// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T385.8 (P28 Phase 1): the must-keep fixture an LLM compressor has to pass.
//!
//! `fixtures/p28_must_keep.toml` is hand-written (nothing from a real session): seven tool
//! results with the spans (locations, identifiers, error lines, numbers, commands, ids) that
//! a summary must not drop. These tests pin the fixture itself and record what the shipped
//! lossless `archive` pointer keeps of it, which is the bar Gate P28 compares a compressor
//! against. They run no model.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rtok::plugins::compress::must_keep::{self, KINDS};

/// The shipped `plugins.archive` pointer shows this many leading and trailing lines.
const HEAD: usize = 8;
const TAIL: usize = 4;

struct Span {
    kind: String,
    text: String,
}

struct Body {
    id: String,
    text: String,
    keep: Vec<Span>,
}

fn bodies() -> Vec<Body> {
    let raw = include_str!("../fixtures/p28_must_keep.toml");
    let doc: toml_edit::DocumentMut = raw.parse().expect("fixture parses");
    doc["body"]
        .as_array_of_tables()
        .expect("[[body]] entries")
        .iter()
        .map(|t| Body {
            id: t["id"].as_str().expect("id").to_string(),
            text: t["text"].as_str().expect("text").to_string(),
            keep: t["keep"]
                .as_array()
                .expect("keep array")
                .iter()
                .map(|v| {
                    let s = v.as_inline_table().expect("{ kind, text }");
                    Span {
                        kind: s["kind"].as_str().expect("kind").to_string(),
                        text: s["text"].as_str().expect("text").to_string(),
                    }
                })
                .collect(),
        })
        .collect()
}

/// Whether the span starts on one of the lines the pointer view shows.
fn at_edge(body: &Body, span: &Span) -> bool {
    let lines: Vec<&str> = body.text.lines().collect();
    let n = lines.len();
    lines
        .iter()
        .enumerate()
        .any(|(i, l)| l.contains(&span.text) && (i < HEAD || i >= n.saturating_sub(TAIL)))
}

#[test]
fn fixture_is_well_formed_and_not_trivially_passed_by_the_pointer() {
    let all = bodies();
    assert!(all.len() >= 6, "{} bodies", all.len());
    let mut ids = BTreeSet::new();
    let mut kinds = BTreeSet::new();
    for b in &all {
        assert!(ids.insert(b.id.clone()), "duplicate body id {}", b.id);
        let n = b.text.lines().count();
        assert!(
            n > HEAD + TAIL + 8,
            "{}: {n} lines leave no middle for a pointer to drop",
            b.id
        );
        let mut seen = BTreeSet::new();
        let mut middle = 0;
        for s in &b.keep {
            assert!(
                KINDS.contains(&s.kind.as_str()),
                "{}: kind {}",
                b.id,
                s.kind
            );
            assert!(!s.text.trim().is_empty(), "{}: empty span", b.id);
            assert!(!s.text.contains('\n'), "{}: multi-line span", b.id);
            assert!(
                b.text.lines().any(|l| l.contains(&s.text)),
                "{}: span not in the body: {}",
                b.id,
                s.text
            );
            assert!(seen.insert(&s.text), "{}: duplicate span {}", b.id, s.text);
            kinds.insert(s.kind.clone());
            middle += usize::from(!at_edge(b, s));
        }
        assert!(middle >= 2, "{}: only {middle} spans a pointer drops", b.id);
    }
    assert_eq!(
        kinds.len(),
        KINDS.len(),
        "every kind is exercised: {kinds:?}"
    );
}

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
    let dir = std::env::temp_dir().join(format!("rtok-p28-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every body as one old tool result in a pi session array, then one live turn.
fn pi_array(all: &[Body]) -> serde_json::Value {
    let mut messages = Vec::new();
    for b in all {
        messages
            .push(serde_json::json!({"role": "user", "content": [{"type": "text", "text": "go"}]}));
        messages.push(
            serde_json::json!({"role": "assistant", "content": [{"type": "text", "text": "…"}]}),
        );
        messages.push(serde_json::json!({
            "role": "toolResult", "toolCallId": format!("call_{}", b.id), "toolName": "bash",
            "content": [{"type": "text", "text": b.text}], "isError": false
        }));
    }
    messages
        .push(serde_json::json!({"role": "user", "content": [{"type": "text", "text": "now"}]}));
    serde_json::Value::Array(messages)
}

/// The baseline Gate P28 starts from: the lossless pointer drops the middle spans, and
/// `expand` recovers every one of them. A compressor has to keep more than the pointer in
/// no more space; this test prints the pointer's score and gates only the two facts.
#[test]
fn pointer_view_keeps_only_edge_spans_and_expand_recovers_all() {
    let all = bodies();
    let home = tmp("pointer");
    std::fs::write(
        home.join("config.toml"),
        "[plugins.archive]\nkeep_turns = 1\nmin_tokens = 1\n",
    )
    .unwrap();
    let out = rtok(
        &home,
        &["archive", "rewrite", "--stdin"],
        &serde_json::to_vec(&pi_array(&all)).unwrap(),
    );
    let view: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let (mut kept, mut total) = (0, 0);
    for b in &all {
        let call = format!("call_{}", b.id);
        let m = view
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["toolCallId"] == call.as_str())
            .unwrap();
        let shown = m["content"][0]["text"].as_str().unwrap();
        assert!(shown.starts_with("[archived "), "{}: not archived", b.id);
        let id = shown
            .split("expand(")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .unwrap();
        let back = String::from_utf8(rtok(&home, &["expand", id], b"")).unwrap();
        for s in &b.keep {
            total += 1;
            assert!(back.contains(&s.text), "{}: expand lost {}", b.id, s.text);
            let survives = shown.contains(&s.text);
            kept += usize::from(survives);
            if at_edge(b, s) {
                assert!(survives, "{}: pointer lost an edge span {}", b.id, s.text);
            }
        }
    }
    eprintln!("p28 baseline: the archive pointer keeps {kept} of {total} must-keep spans");
    assert!(
        kept < total,
        "the pointer already keeps every span: fixture too easy"
    );
}

/// Default `[plugins.compress.llm] max_spans`; a fixture body denser than this would never be
/// sent to the compressor.
const MAX_SPANS: usize = 64;

/// Measured on the fixture when T385.9.1 landed; the extractor may only get better.
const COVERAGE_FLOOR_PERCENT: usize = 95;

/// T385.9.1: the guard's extractor against the fixture. An annotated span is covered when the
/// extractor finds a span inside it or around it, so a summary that loses the annotated text
/// loses a span the guard checks. The share covered is the guard's reach over what Phase 1
/// judged must-keep. No model runs.
#[test]
fn the_guard_reaches_the_annotated_spans_of_the_fixture() {
    let (mut covered, mut total) = (0, 0);
    let mut missed = Vec::new();
    for b in bodies() {
        let spans = must_keep::spans(&b.text);
        assert!(spans.len() <= MAX_SPANS, "{}: {} spans", b.id, spans.len());
        assert!(must_keep::dropped(&spans, &b.text).is_empty(), "{}", b.id);
        for s in &b.keep {
            total += 1;
            if spans
                .iter()
                .any(|e| s.text.contains(&e.text) || e.text.contains(&s.text))
            {
                covered += 1;
            } else {
                missed.push(format!("{}: {}", b.id, s.text));
            }
        }
    }
    eprintln!("p28 guard: reaches {covered} of {total} annotated spans; missed: {missed:#?}");
    assert!(
        covered * 100 >= total * COVERAGE_FLOOR_PERCENT,
        "{covered} of {total}: {missed:#?}"
    );
}

/// Every fixture body goes through the client end to end against a mock upstream whose
/// summary keeps all its spans: none is skipped as too dense, all are stored (diffs once
/// `code` is on), and the same summary minus one span is rejected.
#[tokio::test]
async fn every_fixture_body_passes_the_compressor_client_and_a_lossy_summary_does_not() {
    use httpmock::prelude::*;
    use rtok::config::Config;
    use rtok::plugins::compress::llm::{Auth, Compressor, Outcome};
    use rtok::store::Store;

    let dir = tmp("client");
    let auth = Auth::new(vec![("x-api-key".into(), "test".into())]).unwrap();
    let store = Store::open_in_memory().unwrap();
    for b in bodies() {
        let spans = must_keep::spans(&b.text);
        // A span no other span contains, so leaving it out really drops it.
        let lost = spans
            .iter()
            .find(|s| {
                spans
                    .iter()
                    .all(|o| o.text == s.text || !o.text.contains(&s.text))
            })
            .expect("a free-standing span")
            .text
            .clone();
        let summary = |skip: Option<&str>| {
            let kept: Vec<&str> = spans
                .iter()
                .map(|s| s.text.as_str())
                .filter(|t| Some(*t) != skip)
                .collect();
            serde_json::json!({"content": [{"type": "text", "text": kept.join("; ")}],
                               "stop_reason": "end_turn"})
        };
        let up = MockServer::start();
        let mut cfg = Config::load_from(&dir).unwrap();
        cfg.core.archive_dir = dir.join("archive");
        cfg.plugins.compress.llm.enabled = true;
        cfg.plugins.compress.llm.url = up.base_url();
        let id = store
            .put_archive("s", b.text.as_bytes(), &cfg.core.archive_dir)
            .unwrap();
        let mut lossy = up.mock(|when, then| {
            when.method(POST).path("/v1/messages");
            then.status(200).json_body(summary(Some(&lost)));
        });

        // Diffs and grep hits on source wait for `code = true`; the rest is asked at once.
        let c = Compressor::new(&cfg).unwrap().unwrap();
        let first = c.run(&store, &auth, &id).await;
        if first == Outcome::Skipped("diff or source") {
            assert!(
                b.id == "git-diff-hunks" || b.id == "grep-records",
                "{}",
                b.id
            );
            cfg.plugins.compress.llm.code = true;
        }
        let c = Compressor::new(&cfg).unwrap().unwrap();
        assert_eq!(
            c.run(&store, &auth, &id).await,
            Outcome::Rejected(vec![lost.clone()]),
            "{}",
            b.id
        );
        lossy.delete();
        up.mock(|when, then| {
            when.method(POST).path("/v1/messages");
            then.status(200).json_body(summary(None));
        });
        assert_eq!(c.run(&store, &auth, &id).await, Outcome::Stored, "{}", b.id);
    }
}
