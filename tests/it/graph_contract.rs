// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T8.9: the `graph` contract, pinned through the binary (P8c).
//!
//! Every assertion here goes through `rtok mcp` on stdio, so nothing below depends on how
//! the index is stored. A storage backend is acceptable when this file passes unchanged:
//! the same five tools, byte for byte, plus the three index behaviours a caller can observe
//! — a second repo does not disturb the first, an edited file is re-read, a deleted file
//! loses its rows. The expected strings are the v0.2 (SQLite) output, copied verbatim.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const CHAIN: &str = "fn a() {\n    b();\n}\nfn b() {\n    c();\n}\nfn c() {}\n";
const OTHER: &str = "fn d() {\n    c();\n    c();\n}\n";

/// One `tools/call` against a fresh `rtok mcp` process rooted at `cwd`, sharing `home`.
fn call(home: &Path, cwd: &Path, tool: &str, args: serde_json::Value) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .arg("mcp")
        .env("RTOK_HOME", home)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtok mcp");
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
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let line = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(line.trim()).expect("one JSON-RPC line");
    v["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no text in {line}"))
        .to_string()
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-contract-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn repo(home: &Path, name: &str) -> PathBuf {
    let dir = home.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("chain.rs"), CHAIN).unwrap();
    std::fs::write(dir.join("other.rs"), OTHER).unwrap();
    dir
}

#[test]
fn explore_two_symbol_question_byte_exact() {
    let home = tmp("explore");
    let a = repo(&home, "a");
    assert_eq!(
        call(
            &home,
            &a,
            "explore",
            serde_json::json!({"query": "how do b and c interact"})
        ),
        "= b\nchain.rs::b#function@4 chain.rs:4 function\nfn b() {\n    c();\n}\ncalls: c\n\
         = c\nchain.rs::c#function@7 chain.rs:7 function\nfn c() {}\n\
         paths:\nc → b\n\
         impact:\nb ← 1\nc ← 2\n"
    );
    assert_eq!(
        call(
            &home,
            &a,
            "explore",
            serde_json::json!({"query": "zzz nothing"})
        ),
        "no symbols resolved for \"zzz nothing\""
    );
    assert_eq!(
        call(
            &home,
            &a,
            "explore",
            serde_json::json!({"query": "b and c", "path": "other"})
        ),
        "= b\nno definition of b in other\n= c\nno definition of c in other\n\
         paths:\nc → b\n\
         impact:\nb ← 0\nc ← 1\n"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn four_tools_byte_exact() {
    let home = tmp("tools");
    let a = repo(&home, "a");
    let name = |n: &str| serde_json::json!({"name": n});
    assert_eq!(
        call(&home, &a, "symbol", name("b")),
        "chain.rs::b#function@4 chain.rs:4 function\nfn b() {\n    c();\n}\ncalls: c\n"
    );
    assert_eq!(
        call(&home, &a, "callers", name("c")),
        "chain.rs  b \u{d7}1 (L5)\nother.rs  d \u{d7}2 (L2)\n"
    );
    assert_eq!(
        call(
            &home,
            &a,
            "impact",
            serde_json::json!({"name": "c", "depth": 2})
        ),
        "1  chain.rs  b\n1  other.rs  d\n2  chain.rs  a\n"
    );
    assert_eq!(
        call(
            &home,
            &a,
            "impact",
            serde_json::json!({"name": "c", "depth": 1})
        ),
        "1  chain.rs  b\n1  other.rs  d\n"
    );
    assert_eq!(
        call(
            &home,
            &a,
            "outline",
            serde_json::json!({"path": "chain.rs"})
        ),
        "fn a 1\nfn b 4\nfn c 7"
    );
    assert_eq!(
        call(&home, &a, "symbol", name("zzz")),
        "no definition of zzz"
    );
    assert_eq!(call(&home, &a, "callers", name("a")), "no references to a");
    assert_eq!(call(&home, &a, "impact", name("a")), "nothing reaches a");
    assert_eq!(
        call(
            &home,
            &a,
            "symbol",
            serde_json::json!({"names": ["b", "c"]})
        ),
        "= b\nchain.rs::b#function@4 chain.rs:4 function\nfn b() {\n    c();\n}\ncalls: c\n\
         = c\nchain.rs::c#function@7 chain.rs:7 function\nfn c() {}\n"
    );
    assert_eq!(
        call(
            &home,
            &a,
            "symbol",
            serde_json::json!({"names": ["b", "zzz"]})
        ),
        "= b\nchain.rs::b#function@4 chain.rs:4 function\nfn b() {\n    c();\n}\ncalls: c\n\
         = zzz\nno definition of zzz\n"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// T68.5: `impact` with `path` and no `name` lists tests that reach the file.
#[test]
fn impact_path_alone_lists_the_reaching_test() {
    let home = tmp("affected");
    let a = home.join("repo");
    std::fs::create_dir_all(a.join("tests")).unwrap();
    std::fs::write(
        a.join("lib.rs"),
        "fn add() {
}
",
    )
    .unwrap();
    std::fs::write(
        a.join("other.rs"),
        "fn other() {
}
",
    )
    .unwrap();
    std::fs::write(
        a.join("tests/add.rs"),
        "fn test_add() {
    add();
}
",
    )
    .unwrap();
    std::fs::write(
        a.join("tests/other.rs"),
        "fn test_other() {
    other();
}
",
    )
    .unwrap();
    let out = call(&home, &a, "impact", serde_json::json!({"path": "lib.rs"}));
    assert!(out.contains("tests/add.rs ← via test_add"), "{out}");
    assert!(out.contains("cargo test test_add"), "{out}");
    assert!(!out.contains("test_other"), "{out}");
    assert_eq!(
        call(
            &home,
            &a,
            "impact",
            serde_json::json!({"path": "missing.rs"})
        ),
        "no indexed test reaches the change; run the suite"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn second_repo_leaves_the_first_intact() {
    let home = tmp("roots");
    let a = repo(&home, "a");
    let b = home.join("b");
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(b.join("chain.rs"), "fn alpha() {}\n").unwrap();
    let name = serde_json::json!({"name": "a"});
    let first = call(&home, &a, "symbol", name.clone());
    assert_eq!(
        first,
        "chain.rs::a#function@1 chain.rs:1 function\nfn a() {\n    b();\n}\ncalls: b\n"
    );
    assert_eq!(
        call(&home, &b, "symbol", name.clone()),
        "no definition of a"
    );
    assert_eq!(
        call(&home, &a, "symbol", name),
        first,
        "repo b evicted repo a"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn edited_and_deleted_files_are_reflected() {
    let home = tmp("edit");
    let a = repo(&home, "a");
    let c = serde_json::json!({"name": "c"});
    assert_eq!(
        call(&home, &a, "callers", c.clone()),
        "chain.rs  b \u{d7}1 (L5)\nother.rs  d \u{d7}2 (L2)\n"
    );
    std::fs::write(
        a.join("other.rs"),
        "fn d() {\n    c();\n    c();\n    c();\n}\n",
    )
    .unwrap();
    assert_eq!(
        call(&home, &a, "callers", c.clone()),
        "chain.rs  b \u{d7}1 (L5)\nother.rs  d \u{d7}3 (L2)\n",
        "edited file was not re-read"
    );
    std::fs::remove_file(a.join("other.rs")).unwrap();
    assert_eq!(
        call(&home, &a, "callers", c),
        "chain.rs  b \u{d7}1 (L5)\n",
        "deleted file kept its rows"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// T52.1: optional `path` / `kind` args narrow `symbol`, `callers` and
/// `impact` to one subtree in one call; no-arg calls stay byte-exact (above).
#[test]
fn filters_narrow_to_one_subtree() {
    let home = tmp("filter");
    let a = repo(&home, "a");
    assert_eq!(
        call(
            &home,
            &a,
            "symbol",
            serde_json::json!({"name": "b", "path": "chain"})
        ),
        "chain.rs::b#function@4 chain.rs:4 function\nfn b() {\n    c();\n}\ncalls: c\n"
    );
    assert_eq!(
        call(
            &home,
            &a,
            "symbol",
            serde_json::json!({"name": "b", "kind": "function"})
        ),
        "chain.rs::b#function@4 chain.rs:4 function\nfn b() {\n    c();\n}\ncalls: c\n"
    );
    assert_eq!(
        call(
            &home,
            &a,
            "symbol",
            serde_json::json!({"name": "b", "kind": "struct"})
        ),
        "no definition of b of kind struct"
    );
    assert_eq!(
        call(
            &home,
            &a,
            "callers",
            serde_json::json!({"name": "c", "path": "other"})
        ),
        "other.rs  d ×2 (L2)\n"
    );
    assert_eq!(
        call(
            &home,
            &a,
            "impact",
            serde_json::json!({"name": "c", "depth": 2, "path": "other"})
        ),
        "1  other.rs  d\n"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// T474: a definition longer than 40 lines archives the uncut span, and `rtok expand`
/// returns it. `id` selects that one row even when `name` names something else.
#[test]
fn long_body_expands_and_symbol_id_selects_one_row() {
    let home = tmp("long");
    let a = home.join("repo");
    std::fs::create_dir_all(&a).unwrap();
    let mut src = String::from("fn long() {\n");
    for i in 0..50 {
        src.push_str(&format!("    let v{i} = {i};\n"));
    }
    src.push_str("}\nfn short() {}\n");
    std::fs::write(a.join("long.rs"), &src).unwrap();
    let out = call(&home, &a, "symbol", serde_json::json!({"name": "long"}));
    assert!(out.contains("long.rs::long#function@1"), "{out}");
    assert!(out.contains("expand "), "{out}");
    assert!(
        !out.contains("let v49"),
        "the printed head must stay cut: {out}"
    );
    let id = out
        .lines()
        .find_map(|l| l.split("expand ").nth(1))
        .expect(&out)
        .trim();
    let expanded = std::process::Command::new(env!("CARGO_BIN_EXE_rtok"))
        .args(["expand", id])
        .env("RTOK_HOME", &home)
        .output()
        .unwrap();
    assert!(
        expanded.status.success(),
        "{}",
        String::from_utf8_lossy(&expanded.stderr)
    );
    let full = String::from_utf8(expanded.stdout).unwrap();
    assert!(full.contains("fn long()"), "{full}");
    assert!(full.contains("let v49 = 49;"), "{full}");
    let by_id = call(
        &home,
        &a,
        "symbol",
        serde_json::json!({"id": "long.rs::short#function@53", "name": "long"}),
    );
    assert!(by_id.contains("fn short()"), "{by_id}");
    assert!(!by_id.contains("fn long()"), "{by_id}");
    assert_eq!(
        call(
            &home,
            &a,
            "symbol",
            serde_json::json!({"id": "long.rs::missing#function@1"})
        ),
        "no definition of long.rs::missing#function@1"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// T665: `callers` with no name lists the unreferenced definitions, narrowed by `path` and
/// `kind`; `outline` takes `kind`. Both are parameters of existing tools, not new ones.
#[test]
fn nameless_callers_lists_dead_definitions_and_outline_takes_a_kind() {
    let home = tmp("shapes");
    let a = repo(&home, "a");
    assert_eq!(
        call(&home, &a, "callers", serde_json::json!({})),
        "chain.rs:1 function a\nother.rs:1 function d\n"
    );
    assert_eq!(
        call(&home, &a, "callers", serde_json::json!({"path": "other"})),
        "other.rs:1 function d\n"
    );
    assert_eq!(
        call(
            &home,
            &a,
            "callers",
            serde_json::json!({"path": "chain", "kind": "struct"})
        ),
        "no dead code in chain of kind struct"
    );
    std::fs::write(
        a.join("mix.rs"),
        "struct S;\nfn x() {}\nenum E {\n    A,\n}\n",
    )
    .unwrap();
    assert_eq!(
        call(
            &home,
            &a,
            "outline",
            serde_json::json!({"path": "mix.rs", "kind": "struct"})
        ),
        "struct S 1"
    );
    assert_eq!(
        call(
            &home,
            &a,
            "outline",
            serde_json::json!({"path": "mix.rs", "kind": "trait"})
        ),
        "no trait in mix.rs"
    );
    assert_eq!(
        call(&home, &a, "outline", serde_json::json!({"path": "mix.rs"})),
        "struct S 1\nfn x 2\nenum E 3"
    );
}
