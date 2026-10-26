// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T230: `rtok graph status` + `rtok graph dead` render the D23 Graph page. This file
//! pins the page's dead-symbol list against a fixture tree with exactly one confirmed
//! dead private function, so the model's store read (`graph::dead_candidates`) cannot
//! drift from what `rtok graph dead --json` reports for the same index.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rtok")
}

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-t230-{name}-{}", std::process::id()));
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

/// One private fn nothing calls (`orphan`, dead) and one private fn a pub fn calls
/// (`used`, alive) — a fixture that would false-positive on either side of the filter.
fn seed_project(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("lib.rs"),
        "fn orphan() {}\n\nfn used() {}\n\npub fn entry() { used(); }\n",
    )
    .unwrap();
}

/// T230: the Graph page's dead-symbol list is the same rows `rtok graph dead --json`
/// prints for the same index — the page reads the store as `graph::dead_candidates`
/// left it, `--json` reads through one extra (no-op here) incremental index. Also
/// checks the page carries `graph status`'s rows/files and the T68.3 pending line.
#[test]
fn graph_page_matches_dead_json_on_the_fixture_index() {
    let h = home("page");
    let project = h.join("project");
    seed_project(&project);
    let project_str = project.to_str().unwrap();

    rtok(&["graph", "index", project_str], &h);
    let dead_json = rtok(&["graph", "dead", project_str, "--json"], &h);
    let rows: Vec<serde_json::Value> = serde_json::from_str(&dead_json).expect("dead --json");
    assert_eq!(
        rows.len(),
        1,
        "fixture has exactly one dead row: {dead_json}"
    );
    let status_json = rtok(&["graph", "status", project_str, "--json"], &h);
    let status: serde_json::Value = serde_json::from_str(&status_json).expect("status --json");

    let cfg = rtok::testutil::config_file_in(&h);
    // The Graph page has no config-driven root (like `graph status`, it reads the
    // current directory), so this pins it the same way `rtok graph status`/`dead`
    // default their own `path` — via cwd. nextest runs each test in its own process,
    // so this does not leak into another test's relative paths; a plain
    // multi-threaded `cargo test` run of this file would race here.
    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(&project).unwrap();
    let page = rtok::web::model::snapshot(&cfg)
        .graph
        .expect("the graph page read the fixture index");
    std::env::set_current_dir(prev).unwrap();

    assert!(
        page.contains(&format!("rows {}", status["rows"])),
        "page is missing graph status's rows: {page}"
    );
    assert!(
        page.contains(&format!("files {}", status["files"])),
        "page is missing graph status's files: {page}"
    );
    assert!(
        page.contains("pending 0"),
        "page is missing the T68.3 pending/staleness line: {page}"
    );
    for row in &rows {
        let line = format!(
            "{}:{} {} {}",
            row["path"].as_str().unwrap(),
            row["line"].as_i64().unwrap(),
            row["kind"].as_str().unwrap(),
            row["name"].as_str().unwrap()
        );
        assert!(
            page.contains(&line),
            "page is missing a `graph dead --json` row: {line}\n---\n{page}"
        );
    }
    let _ = fs::remove_dir_all(&h);
}
