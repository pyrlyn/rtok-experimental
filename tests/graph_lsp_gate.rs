// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T30.2 / Gate P30: same MCP names; tags hit the type-position fixture since
//! T52.5 (`RUST_EXTRA_REF`); macro bodies stay a tags miss. LSP coverage is unchanged.
//!
//! Skips the rust-analyzer / dart path when `lsp::on_path` is false (a real binary, not a
//! mise shim or rustup proxy — never `… --version`, which can hang). Language-server state
//! is confined under the temp crate by `lsp::Session::spawn` (`target/rtok-lsp-xdg` or
//! `.dart_tool/rtok-lsp-xdg`); this file asserts that after each LSP run.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rtok::plugin::{Ctx, Plugin, Runtime};
use rtok::plugins::graph::lsp::on_path;
use rtok::plugins::graph::{GraphPlugin, callers, outline, symbol};

const CHAIN: &str = "fn a() {\n    b();\n}\nfn b() {\n    c();\n}\nfn c() {}\n";
const OTHER: &str = "fn d() {\n    c();\n    c();\n}\n";
const FIXTURE: &str = "\
pub struct OnlyTyped;
pub fn user(_t: Vec<OnlyTyped>) {}
";

fn open(tag: &str, backend: &str) -> (Runtime, PathBuf) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rtok-{tag}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let mut cfg = rtok::testutil::config_in(&dir);
    cfg.plugins.graph.backend = backend.into();
    // The fixture crate lives outside cwd; `outline` checks paths against `read`'s roots.
    cfg.plugins.read.allow_paths = vec![dir.clone()];
    (Runtime::open(cfg, tag).unwrap(), dir)
}

fn onlytyped_crate(dir: &Path) -> PathBuf {
    let root = dir.join("crate");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"lsp_gate\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), FIXTURE).unwrap();
    root
}

/// Every path under `root` whose name is an XDG / pub-cache state dir must stay inside `dir`.
fn assert_lsp_state_stays_in(dir: &Path, root: &Path) {
    let expected = if root.join("Cargo.toml").is_file() {
        root.join("target").join("rtok-lsp-xdg")
    } else if root.join("pubspec.yaml").is_file() {
        root.join(".dart_tool").join("rtok-lsp-xdg")
    } else {
        root.join(".rtok-lsp-xdg")
    };
    assert!(
        expected.starts_with(dir),
        "lsp state root {} escapes temp {}",
        expected.display(),
        dir.display()
    );
    assert!(
        expected.is_dir(),
        "lsp state dir was not created under the temp crate: {}",
        expected.display()
    );
}

#[test]
fn mcp_tool_names_are_unchanged() {
    let names: Vec<_> = GraphPlugin
        .mcp_tools()
        .into_iter()
        .map(|t| t.name)
        .collect();
    // T68.1 added `explore` as the fifth tool, T329.18 `graph_diff` as the sixth and T329.16
    // `graph_export` as the seventh; the surface-token gate lives in
    // `graph::tests::graph_surface_is_seven_tools_under_budget`.
    assert_eq!(
        names,
        [
            "symbol",
            "callers",
            "impact",
            "outline",
            "explore",
            "graph_diff",
            "graph_export"
        ]
    );
}

/// Gate P30: `backend = "tags"` keeps the T8.9 contract bytes.
#[test]
fn tags_backend_callers_bytes_match_contract() {
    let (cx, dir) = open("p30-tags-bytes", "tags");
    let a = dir.join("a");
    fs::create_dir_all(&a).unwrap();
    fs::write(a.join("chain.rs"), CHAIN).unwrap();
    fs::write(a.join("other.rs"), OTHER).unwrap();
    let ctx = Ctx::new(&cx);
    assert_eq!(
        symbol(&ctx, &a, "b").unwrap(),
        "chain.rs::b#function@4 chain.rs:4 function\nfn b() {\n    c();\n}\ncalls: c\n"
    );
    assert_eq!(
        callers(&ctx, &a, "c").unwrap(),
        "chain.rs  b ×1 (L5)\nother.rs  d ×2 (L2)\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// T52.5: type-position `OnlyTyped` is a tags hit (`RUST_EXTRA_REF`); before it,
/// this was the P30 "tags miss, LSP hit" fixture (T8.8).
#[test]
fn tags_backend_hits_onlytyped_type_position() {
    let (cx, dir) = open("p30-tags-hit", "tags");
    let root = onlytyped_crate(&dir);
    let out = callers(&Ctx::new(&cx), &root, "OnlyTyped").unwrap();
    assert!(out.contains("user"), "tags callers should hit user: {out}");
    let _ = fs::remove_dir_all(&dir);
}

/// Macro arguments parse as an opaque `token_tree`; since T663 rtok's own pattern reads an
/// identifier followed by a parenthesised group inside it, so `macro_callee`, only invoked inside
/// `assert!(..)`, is a tags hit. It was the P30 "tags miss, LSP hit" fixture before.
#[test]
fn tags_backend_hits_macro_body() {
    let (cx, dir) = open("p30-tags-miss", "tags");
    let root = dir.join("crate");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"lsp_gate\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        root.join("src/lib.rs"),
        "pub fn macro_callee() -> bool { true }\npub fn user() {\n    assert!(macro_callee());\n}\n",
    )
    .unwrap();
    let out = callers(&Ctx::new(&cx), &root, "macro_callee").unwrap();
    assert!(out.contains("user"), "tags callers should hit user: {out}");
    let _ = fs::remove_dir_all(&dir);
}

/// Gate P30: rust-analyzer `textDocument/references` hits `user`'s `Vec<OnlyTyped>`.
#[test]
fn lsp_backend_hits_onlytyped_type_position() {
    if !on_path("rust-analyzer") {
        eprintln!("skip: rust-analyzer not on PATH");
        return;
    }
    let (cx, dir) = open("p30-lsp-hit", "lsp");
    let root = onlytyped_crate(&dir);
    let ctx = Ctx::new(&cx);
    let out = callers(&ctx, &root, "OnlyTyped").unwrap();
    assert!(
        out.contains("user"),
        "LSP callers should hit user(Vec<OnlyTyped>): {out}"
    );
    assert!(!out.starts_with("no references to OnlyTyped"), "{out}");
    let kinds: Vec<_> = cx
        .store
        .list_measurements("graph")
        .unwrap()
        .into_iter()
        .map(|m| m.kind)
        .collect();
    assert!(
        kinds.iter().any(|k| k.starts_with("lsp")),
        "expected plugin=graph lsp measurement, got {kinds:?}"
    );
    let sym = symbol(&ctx, &root, "OnlyTyped").unwrap();
    assert!(sym.contains("OnlyTyped"), "{sym}");
    let map = outline(&ctx, &root.join("src/lib.rs").to_string_lossy()).unwrap();
    assert!(map.contains("OnlyTyped") || map.contains("user"), "{map}");
    assert_lsp_state_stays_in(&dir, &root);
    let _ = fs::remove_dir_all(&dir);
}

/// T41.1: a `pubspec.yaml` root outlines a two-symbol `lib/main.dart` through
/// `dart language-server`. Skips when `dart` is not on PATH.
#[test]
fn lsp_backend_outlines_dart_main() {
    if !on_path("dart") {
        eprintln!("skip: dart not on PATH");
        return;
    }
    let (cx, dir) = open("p41-dart", "lsp");
    let root = dir.join("pkg");
    fs::create_dir_all(root.join("lib")).unwrap();
    fs::write(root.join("pubspec.yaml"), "name: dart_gate\n").unwrap();
    fs::write(
        root.join("lib/main.dart"),
        "void helper() {}\nvoid main() {\n  helper();\n}\n",
    )
    .unwrap();
    let ctx = Ctx::new(&cx);
    let res = outline(&ctx, &root.join("lib/main.dart").to_string_lossy());
    let text = match &res {
        Ok(m) => m.clone(),
        Err(e) => e.to_string(),
    };
    if dart_not_analyzing(&text, cfg!(windows)) {
        eprintln!("skip: dart language-server does not analyze the temp project: {text}");
        let _ = fs::remove_dir_all(&dir);
        return;
    }
    let map = res.unwrap();
    assert!(map.contains("helper"), "{map}");
    assert!(map.contains("main"), "{map}");
    assert_lsp_state_stays_in(&dir, &root);
    let _ = fs::remove_dir_all(&dir);
}

/// T901: dart 3.9 on Windows answers `File is not being analyzed` for this temp project
/// (seen on the first local Windows gate; CI has no dart, so the cause is not pinned down).
/// Only that Windows answer skips; the same text on another OS still fails the test.
fn dart_not_analyzing(text: &str, windows: bool) -> bool {
    windows && text.contains("not being analyzed")
}

#[test]
fn dart_skip_is_windows_only_and_names_the_exact_answer() {
    let msg = "File is not being analyzed: C:\\t\\lib\\main.dart";
    assert!(dart_not_analyzing(msg, true));
    assert!(!dart_not_analyzing(msg, false));
    assert!(!dart_not_analyzing("no symbols", true));
}
