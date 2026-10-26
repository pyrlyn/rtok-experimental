// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T22.3: `rtok report --format pdf` renders the same eight sections in the same
//! order as `--format html` — asserted by walking both over one store — paged
//! (a contents page plus content) with the chart labels embedded.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rtok")
}

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-t223-{name}-{}", std::process::id()));
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

/// Just enough rows for every chart: one timed call, one usage session, one
/// measurement. Headings render regardless; charts need one pair each.
fn seed(home: &Path) {
    let cfg = rtok::config::Config::load_from(home).expect("config");
    let store = rtok::store::Store::open(&cfg.core.db_path).expect("store");
    store
        .upsert_session("s1", None, None, None, Some("proxy"))
        .unwrap();
    let id = store
        .insert_call("s1", "hook", "hook", None, None, None, None, None)
        .unwrap();
    store.set_call_ms(id, 4.0).unwrap();
    let id = store
        .insert_call("s1", "proxy", "api_request", None, None, None, None, None)
        .unwrap();
    store
        .insert_call_io(id, Some(br#"{"system":"s"}"#), None, 1 << 20, None)
        .unwrap();
    store
        .insert_usage("s1", Some("m"), "anthropic", 100, 0, 0, 5, id)
        .unwrap();
    store
        .insert_measurement(
            "s1",
            &rtok::Measurement {
                plugin: "cmd",
                kind: "filter",
                before_bytes: 100,
                after_bytes: 40,
                est_before: 25,
                est_after: 10,
                ref_id: None,
                call_id: None,
            },
        )
        .unwrap();
}

fn positions(hay: &[u8], needle: &str) -> Vec<usize> {
    let n = needle.as_bytes();
    hay.windows(n.len())
        .enumerate()
        .filter(|(_, w)| *w == n)
        .map(|(i, _)| i)
        .collect()
}

/// T22.3 Check, first clause: the PDF carries the HTML's eight headings in the
/// same order — both in the contents list (first occurrences) and in the body
/// (last occurrences) — plus one chart label per series.
#[test]
fn pdf_has_the_html_headings_in_order_and_the_charts() {
    let h = home("pdf");
    seed(&h);
    let html = rtok(&["report", "--format", "html"], &h);
    let heads: Vec<String> = [
        "Window",
        "Savings",
        "Calls",
        "Cache",
        "Expand",
        "Config",
        "Doctor",
        "Recommendations",
    ]
    .into_iter()
    .map(|sec| {
        assert!(html.contains(&format!(">{sec}</h2>")), "html heading {sec}");
        format!("({sec}")
    })
    .collect();

    let path = h.join("report.pdf");
    let stdout = rtok(
        &["report", "--format", "pdf", "--out", path.to_str().unwrap()],
        &h,
    );
    assert_eq!(stdout.trim(), path.display().to_string());
    let pdf = fs::read(&path).unwrap();

    assert!(pdf.starts_with(b"%PDF-"), "magic");
    let tail = pdf.iter().rev().take(8).copied().collect::<Vec<_>>();
    assert!(
        String::from_utf8_lossy(&tail.into_iter().rev().collect::<Vec<_>>()).contains("%%EOF"),
        "trailer"
    );
    assert!(!positions(&pdf, "(Contents)").is_empty(), "contents page");
    // lopdf writes `/Type/Page` with no space; `/Type/Pages` is the tree node.
    let pages = positions(&pdf, "/Type/Page").len() - positions(&pdf, "/Type/Pages").len();
    assert!(pages >= 2, "contents plus content: {pages}");

    let first: Vec<usize> = heads.iter().map(|t| positions(&pdf, t)[0]).collect();
    let last: Vec<usize> = heads
        .iter()
        .map(|t| *positions(&pdf, t).last().expect("heading"))
        .collect();
    let mut sorted = first.clone();
    sorted.sort_unstable();
    assert_eq!(first, sorted, "contents order matches html");
    sorted = last.clone();
    sorted.sort_unstable();
    assert_eq!(last, sorted, "body order matches html");

    for label in ["(cmd)", "(hook)", "(saved tokens per plugin)"] {
        assert!(!positions(&pdf, label).is_empty(), "chart keeps {label}");
    }
    let _ = fs::remove_dir_all(&h);
}

/// An empty store still renders all eight headings (with "No rows" lines),
// so the document shape never depends on the data.
#[test]
fn empty_store_pdf_keeps_all_headings() {
    let h = home("empty");
    let path = h.join("empty.pdf");
    rtok(
        &["report", "--format", "pdf", "--out", path.to_str().unwrap()],
        &h,
    );
    let pdf = fs::read(&path).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    for sec in [
        "Window",
        "Savings",
        "Calls",
        "Cache",
        "Expand",
        "Config",
        "Doctor",
        "Recommendations",
    ] {
        assert!(
            !positions(&pdf, &format!("({sec}")).is_empty(),
            "heading {sec}"
        );
    }
    assert!(!positions(&pdf, "(No rows in window.)").is_empty());
    let _ = fs::remove_dir_all(&h);
}
