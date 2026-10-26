// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T329.4.1, T329.4.2: the `project` argument of the five graph MCP tools and the CLI
//! `--project` flag, through `rtok` over four fixture projects: `a` links `b`, `b` links `c`, `d` stands alone.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-t3294-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs_kit::path::canonicalize(&dir).unwrap()
}

fn rtok(home: &Path, args: &[&str]) {
    let out = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .args(args)
        .current_dir(home)
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// One `rtok graph …` run in `cwd`: stdout on success, stderr otherwise.
fn cli(home: &Path, cwd: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .arg("graph")
        .args(args)
        .current_dir(cwd)
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .output()
        .unwrap();
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    if out.status.success() {
        Ok(text(&out.stdout))
    } else {
        Err(text(&out.stderr))
    }
}

/// One `tools/call` against a fresh `rtok mcp` rooted at `cwd`; the text of the result.
fn call(home: &Path, cwd: &Path, tool: &str, args: serde_json::Value) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .arg("mcp")
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
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
    let line = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(line.trim()).expect("one JSON-RPC line");
    v["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no text in {line}"))
        .to_string()
}

fn world(name: &str) -> PathBuf {
    let home = home(name);
    let src = [
        ("a", "fn a_caller() { shared(); }\n"),
        ("b", "fn b_caller() { shared(); }\n"),
        ("c", "fn shared() {}\n"),
        ("d", "fn d_caller() { shared(); }\n"),
    ];
    for (project, code) in src {
        let dir = home.join(project);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("lib.rs"), code).unwrap();
        rtok(&home, &["graph", "projects", "add", dir.to_str().unwrap()]);
    }
    let id = |n: &str| ["a", "b", "c", "d"].iter().position(|p| *p == n).unwrap() + 1;
    for (from, to) in [("a", "b"), ("b", "c")] {
        let (from, to) = (id(from).to_string(), id(to).to_string());
        rtok(&home, &["graph", "projects", "link", &to, "--from", &from]);
    }
    home
}

#[test]
fn callers_without_project_cross_the_links_and_label_each_project() {
    let home = world("cross");
    let out = call(
        &home,
        &home.join("a"),
        "callers",
        serde_json::json!({"name": "shared"}),
    );
    assert_eq!(
        out,
        "[a] lib.rs  a_caller \u{d7}1 (L1)\n[b] lib.rs  b_caller \u{d7}1 (L1)\n"
    );
}

#[test]
fn project_d_does_not_cross_and_a_bad_project_is_an_error() {
    let home = world("d");
    let a = home.join("a");
    let d = home.join("d");
    let by_path = call(
        &home,
        &a,
        "callers",
        serde_json::json!({"name": "shared", "project": d}),
    );
    assert_eq!(by_path, "lib.rs  d_caller \u{d7}1 (L1)\n");
    let by_id = call(
        &home,
        &a,
        "callers",
        serde_json::json!({"name": "shared", "project": "4"}),
    );
    assert_eq!(by_id, by_path);
    let bad = call(
        &home,
        &a,
        "symbol",
        serde_json::json!({"name": "shared", "project": "9999"}),
    );
    assert!(bad.contains("no project"), "{bad}");
}

#[test]
fn symbol_finds_the_definition_in_a_linked_project() {
    let home = world("symbol");
    let out = call(
        &home,
        &home.join("a"),
        "symbol",
        serde_json::json!({"name": "shared"}),
    );
    assert_eq!(
        out,
        "[c] lib.rs::shared#function@1 lib.rs:1 function\nfn shared() {}\n"
    );
}

#[test]
fn impact_crosses_the_links_and_the_cli_flag_agrees_with_the_mcp_argument() {
    let home = world("impact");
    let a = home.join("a");
    let expect = "1  [a] lib.rs  a_caller\n1  [b] lib.rs  b_caller\n";
    let mcp = call(&home, &a, "impact", serde_json::json!({"name": "shared"}));
    assert_eq!(mcp, expect);
    assert_eq!(cli(&home, &a, &["impact", "shared"]).unwrap(), expect);
    // From a directory outside every project, `--project` names the scope.
    let elsewhere = home.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    for project in [a.to_str().unwrap(), "1"] {
        let flag = cli(
            &home,
            &elsewhere,
            &["impact", "shared", "--project", project],
        );
        assert_eq!(flag.unwrap(), expect);
        let arg = call(
            &home,
            &elsewhere,
            "impact",
            serde_json::json!({"name": "shared", "project": project}),
        );
        assert_eq!(arg, expect);
    }
    // D is not linked: it answers for itself only.
    let d = call(
        &home,
        &a,
        "impact",
        serde_json::json!({"name": "shared", "project": "4"}),
    );
    assert_eq!(d, "1  lib.rs  d_caller\n");
    assert_eq!(
        cli(&home, &a, &["impact", "shared", "--project", "4"]).unwrap(),
        d
    );
}

#[test]
fn explore_and_outline_take_project() {
    let home = world("explore");
    let a = home.join("a");
    let out = call(&home, &a, "explore", serde_json::json!({"query": "shared"}));
    assert!(
        out.contains("= shared\n[c] lib.rs::shared#function@1 lib.rs:1 function\n"),
        "{out}"
    );
    assert!(out.contains("shared \u{2190} 2\n"), "{out}");
    let d = call(
        &home,
        &a,
        "explore",
        serde_json::json!({"query": "shared", "project": "4"}),
    );
    // `shared` is not a definition in d. Search still finds `d_caller`: its signature
    // line contains the query (T474).
    assert_eq!(
        d,
        "= d_caller\nlib.rs::d_caller#function@1 lib.rs:1 function\nfn d_caller() { shared(); }\ncalls: shared\npaths:\nnone\nimpact:\nd_caller \u{2190} 0\n"
    );
    assert!(!d.contains("[c]"), "{d}");
    let map = |project: &str| {
        call(
            &home,
            &a,
            "outline",
            serde_json::json!({"path": "lib.rs", "project": project}),
        )
    };
    assert!(map("4").contains("d_caller"));
    assert!(map("1").contains("a_caller"));
}

#[test]
fn the_cli_flag_resolves_a_project_for_the_single_project_subcommands() {
    let home = world("flag");
    let elsewhere = home.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let status = cli(&home, &elsewhere, &["status", "--project", "4", "--json"]).unwrap();
    assert!(status.contains("/d"), "{status}");
    assert!(cli(&home, &elsewhere, &["dead", "--project", "4"]).is_ok());
    assert!(cli(&home, &elsewhere, &["index", "--project", "4", "--dry-run"]).is_ok());
    assert!(cli(&home, &elsewhere, &["affected", "--project", "4"]).is_ok());
    let both = cli(&home, &elsewhere, &["status", "--project", "4", "."]);
    assert!(both.unwrap_err().contains("cannot be used with"));
    let bad = cli(&home, &elsewhere, &["impact", "x", "--project", "9999"]);
    assert!(bad.unwrap_err().contains("no project"));
}

/// T329.5: `shared` in `c` is called from `a` and `b`, so it is live in their scope and dead in its own.
#[test]
fn dead_over_a_scope_spares_what_a_linked_project_calls() {
    let home = world("dead");
    let a = home.join("a");
    assert_eq!(
        cli(&home, &a, &["dead"]).unwrap(),
        "[a] lib.rs:1 function a_caller\n[b] lib.rs:1 function b_caller\n"
    );
    assert_eq!(
        cli(&home, &a, &["dead", "--project", "3"]).unwrap(),
        "lib.rs:1 function shared\n"
    );
    let json = cli(&home, &a, &["dead", "--json"]).unwrap();
    assert!(json.contains("\"project\": \"b\""), "{json}");
    // Affected tests of a path in a linked project: none are indexed here, said once.
    let c = call(
        &home,
        &a,
        "impact",
        serde_json::json!({"path": home.join("c").join("lib.rs")}),
    );
    assert_eq!(c, "no indexed test reaches the change; run the suite");
}
