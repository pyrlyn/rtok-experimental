// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Heavy-dependency ownership (#632).
//!
//! While a dependency still lives in the `rtok` package, the module scan under `src/`
//! is the boundary. Once a crate is extracted, `cargo metadata` is the boundary.
//! Dev-dependencies count: a test that opens the crate is still that package depending on it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Diesel (and only Diesel's SQLite stack) belongs to `rtok-store`.
#[test]
fn only_store_depends_on_diesel() {
    assert_eq!(
        owners_of("diesel"),
        ["rtok-store"],
        "only rtok-store may depend on diesel"
    );
    assert_eq!(
        files_mentioning(&["diesel::", "#[diesel"]),
        Vec::<String>::new(),
        "src/ must not name diesel; it lives in crates/rtok-store"
    );
    let store = files_mentioning_under("crates", &["diesel::", "#[diesel"]);
    assert!(
        store.iter().all(|p| p.starts_with("crates/rtok-store/")),
        "diesel outside rtok-store: {store:?}"
    );
}

/// T668: semantic memory is the opt-in HTTP embedder over the stored hash/vector table. No
/// vector, ONNX or embedding-model crate may enter any member's manifest (default or feature).
#[test]
fn no_member_depends_on_a_vector_or_model_crate() {
    for dep in [
        "ort",
        "candle-core",
        "fastembed",
        "sqlite-vec",
        "usearch",
        "hnsw_rs",
        "qdrant-client",
        "lancedb",
        "tract-onnx",
    ] {
        assert_eq!(
            owners_of(dep),
            Vec::<String>::new(),
            "{dep} is not allowed (T668)"
        );
    }
}

/// T329.10: the graph text backend searches in process (D6, D18). Neither its module nor the walk
/// it shares with `search` may name a way to start a program, so `rg`, `grep` and `ssh` cannot be
/// run from it.
#[test]
fn graph_text_backend_starts_no_process() {
    let needles = [
        "Command",
        "std::process",
        "tokio::process",
        "spawn",
        "exec(",
    ];
    for file in ["src/plugins/graph/text.rs", "src/plugins/read/search.rs"] {
        let src =
            std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(file)).unwrap();
        // The test modules below may build fixtures; only the shipped code is bound.
        let shipped = src.split("#[cfg(test)]").next().unwrap();
        let code = strip_comments(shipped);
        for needle in needles {
            assert!(!code.contains(needle), "{file} names `{needle}`");
        }
    }
}

/// `ratatui` and `crossterm` render the operator model. They stay in `src/tui/` until a
/// tui crate exists; the only package that may depend on them until then is `rtok`.
#[test]
fn only_tui_depends_on_ratatui() {
    assert_eq!(owners_of("ratatui"), ["rtok"]);
    assert_eq!(owners_of("crossterm"), ["rtok"]);
    for dep in ["ratatui::", "crossterm::"] {
        let files = files_mentioning(&[dep]);
        assert!(
            files.iter().all(|p| p.starts_with("src/tui/")),
            "{dep} outside src/tui: {files:?}"
        );
        assert!(!files.is_empty(), "{dep} vanished from src/tui");
    }
}

/// `printpdf` is the PDF report renderer.
#[test]
fn only_report_depends_on_printpdf() {
    assert_eq!(owners_of("printpdf"), ["rtok"]);
    let files = files_mentioning(&["printpdf::"]);
    assert!(
        files.iter().all(|p| p.starts_with("src/report/")),
        "printpdf outside src/report: {files:?}"
    );
    assert!(!files.is_empty(), "printpdf vanished from src/report");
}

/// `wasmi` is the optional wasm host, one file.
#[test]
fn only_wasm_plugin_depends_on_wasmi() {
    assert_eq!(owners_of("wasmi"), ["rtok"]);
    assert_eq!(
        files_mentioning(&["wasmi::"]),
        vec!["src/plugins/wasm.rs".to_string()]
    );
}

/// T276.6: `indicatif` draws only behind `rtok_sys::proc`, so every spinner and bar obeys one
/// terminal check, `RTOK_NO_PROGRESS` and `disable_progress`.
#[test]
fn only_rtok_sys_depends_on_indicatif() {
    assert_eq!(owners_of("indicatif"), ["rtok-sys"]);
    assert_eq!(files_mentioning(&["indicatif::"]), Vec::<String>::new());
}

/// T276.6: `just lint` denies `Command::new` in libraries and binaries; only the all-targets
/// pass, which lints tests too, allows it, and only `rtok-sys` opts out in code.
#[test]
fn the_lint_gate_bans_command_new_outside_rtok_sys() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let clippy = std::fs::read_to_string(root.join("clippy.toml")).unwrap();
    assert!(
        clippy.contains(r#"path = "std::process::Command::new""#),
        "{clippy}"
    );
    let just = std::fs::read_to_string(root.join("justfile")).unwrap();
    let lint: Vec<&str> = just
        .lines()
        .skip_while(|l| *l != "lint:")
        .skip(1)
        .take_while(|l| l.starts_with("    "))
        .collect();
    assert!(
        lint.iter()
            .any(|l| l.contains("--lib --bins") && !l.contains("disallowed_methods")),
        "{lint:?}"
    );
    let allow = ["allow(clippy::disallowed_methods)"];
    assert_eq!(files_mentioning(&allow), Vec::<String>::new());
    let crates = files_mentioning_under("crates", &allow);
    assert!(
        crates.iter().all(|p| p.starts_with("crates/rtok-sys/")),
        "{crates:?}"
    );
}

/// `tree-sitter*` parses for `read` and `graph`, both under `src/plugins/`.
#[test]
fn only_plugins_depend_on_tree_sitter() {
    for name in [
        "tree-sitter",
        "tree-sitter-tags",
        "tree-sitter-rust",
        "tree-sitter-javascript",
        "tree-sitter-typescript",
        "tree-sitter-python",
        "tree-sitter-c",
        "tree-sitter-go",
        "tree-sitter-dart",
        "tree-sitter-java",
        "tree-sitter-kotlin-ng",
        "tree-sitter-swift",
        "tree-sitter-c-sharp",
        "tree-sitter-ruby",
        "tree-sitter-php",
    ] {
        assert_eq!(owners_of(name), ["rtok"], "{name}");
    }
    let files = files_mentioning_prefix("tree_sitter");
    assert!(
        files.iter().all(|p| p.starts_with("src/plugins/")),
        "tree-sitter outside src/plugins: {files:?}"
    );
    assert!(!files.is_empty(), "tree-sitter vanished from src/plugins");
}

/// `rmcp` is the MCP server. `axum` serves the proxy, the dashboard, and MCP HTTP.
#[test]
fn only_mcp_depends_on_rmcp_and_axum_stays_on_its_servers() {
    assert_eq!(owners_of("rmcp"), ["rtok"]);
    assert_eq!(owners_of("axum"), ["rtok"]);
    let rmcp = files_mentioning(&["rmcp::"]);
    assert!(
        rmcp.iter()
            .all(|p| p == "src/mcp.rs" || p.starts_with("src/mcp/")),
        "rmcp outside src/mcp: {rmcp:?}"
    );
    assert!(!rmcp.is_empty(), "rmcp vanished from src/mcp");
    let axum = files_mentioning(&["axum::"]);
    assert!(
        axum.iter().all(|p| {
            p.starts_with("src/proxy/") || p.starts_with("src/web/") || p.starts_with("src/mcp/")
        }),
        "axum outside proxy/web/mcp: {axum:?}"
    );
    assert!(!axum.is_empty(), "axum vanished");
}

/// `.env` parsing belongs to `rtok-config`.
#[test]
fn only_config_depends_on_dotenvy() {
    assert_eq!(owners_of("dotenvy"), ["rtok-config"]);
    let files = files_mentioning(&["dotenvy::"]);
    assert!(
        files.iter().all(|p| p.starts_with("crates/rtok-config/")),
        "dotenvy outside rtok-config: {files:?}"
    );
}

/// The published contract depends on no other workspace crate (D25).
#[test]
fn plugin_sdk_has_no_workspace_dependency() {
    let meta = metadata();
    let member_names = member_names(&meta);
    let sdk = package(&meta, "rtok-plugin-sdk");
    let mut workspace_deps = Vec::new();
    for dep in sdk["dependencies"].as_array().unwrap() {
        let name = dep["name"].as_str().unwrap();
        if member_names.contains(name) || dep["path"].is_string() {
            workspace_deps.push(name);
        }
    }
    assert!(
        workspace_deps.is_empty(),
        "rtok-plugin-sdk depends on workspace crates: {workspace_deps:?}"
    );
}

/// The hook client is std-only, so a host can ship it without rtok's dependency tree.
#[test]
fn rtok_hook_client_is_std_only() {
    let meta = metadata();
    let hook = package(&meta, "rtok-hook");
    let deps: Vec<&str> = hook["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert!(
        deps.is_empty(),
        "rtok-hook is std-only; it depends on {deps:?}"
    );
}

fn owners_of(dep: &str) -> Vec<String> {
    let meta = metadata();
    let members = member_ids(&meta);
    let mut owners = Vec::new();
    for pkg in meta["packages"].as_array().unwrap() {
        let id = pkg["id"].as_str().unwrap();
        if !members.contains(id) {
            continue;
        }
        let uses = pkg["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["name"].as_str() == Some(dep));
        if uses {
            owners.push(pkg["name"].as_str().unwrap().to_string());
        }
    }
    owners.sort();
    owners
}

fn member_ids(meta: &serde_json::Value) -> BTreeSet<&str> {
    meta["workspace_members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect()
}

fn member_names(meta: &serde_json::Value) -> BTreeSet<&str> {
    let ids = member_ids(meta);
    meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|pkg| ids.contains(pkg["id"].as_str().unwrap()))
        .map(|pkg| pkg["name"].as_str().unwrap())
        .collect()
}

fn package<'a>(meta: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pkg| pkg["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("workspace package {name} missing from cargo metadata"))
}

/// `--filter-platform` the host: offline, an unfiltered resolve needs every platform's crates
/// in the cache, and a host only downloads its own (resvg's fontconfig-parser is Linux-only).
/// The checks read declared dependencies of workspace members, which the filter keeps.
/// A failed offline resolve is retried online: the index cache is shared, and on the
/// self-hosted runner another job's rust-cache trims entries this workspace still needs.
fn metadata() -> serde_json::Value {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let rustc = std::process::Command::new("rustc")
        .arg("-vV")
        .output()
        .expect("rustc -vV");
    let host = String::from_utf8_lossy(&rustc.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("host: ").map(str::to_string))
        .expect("rustc -vV names the host");
    let run = |offline: bool| {
        std::process::Command::new("cargo")
            .args(["metadata", "--format-version", "1", "--locked"])
            .args(offline.then_some("--offline"))
            .args(["--filter-platform", &host])
            .current_dir(&root)
            .output()
            .expect("cargo metadata")
    };
    let mut out = run(true);
    if !out.status.success() {
        out = run(false);
    }
    assert!(
        out.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

/// `.rs` files under `src/` whose code (not comments) contains any needle.
fn files_mentioning(needles: &[&str]) -> Vec<String> {
    files_mentioning_under("src", needles)
}

fn files_mentioning_under(dir: &str, needles: &[&str]) -> Vec<String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(dir);
    let mut hits = Vec::new();
    walk(&root, &mut |path| {
        let text = std::fs::read_to_string(path).unwrap();
        let code = strip_comments(&text);
        if needles.iter().any(|n| code.contains(n)) {
            let rel = path
                .strip_prefix(env!("CARGO_MANIFEST_DIR"))
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            hits.push(rel);
        }
    });
    hits.sort();
    hits
}

/// `tree_sitter` and `tree_sitter_<lang>` path uses.
fn files_mentioning_prefix(prefix: &str) -> Vec<String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    walk(&root, &mut |path| {
        let text = std::fs::read_to_string(path).unwrap();
        let code = strip_comments(&text);
        if mentions_prefix(&code, prefix) {
            let rel = path
                .strip_prefix(env!("CARGO_MANIFEST_DIR"))
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            hits.push(rel);
        }
    });
    hits.sort();
    hits
}

fn mentions_prefix(code: &str, prefix: &str) -> bool {
    let bytes = code.as_bytes();
    let needle = prefix.as_bytes();
    let mut i = 0;
    while i + needle.len() < bytes.len() {
        if bytes[i..].starts_with(needle) {
            let before_ok = i == 0 || !is_ident(bytes[i - 1]);
            let after = i + needle.len();
            let rest = &code[after..];
            let ident_tail = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .count();
            if before_ok && rest[ident_tail..].starts_with("::") {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for ent in rd.flatten() {
        let path = ent.path();
        if path.is_dir() {
            walk(&path, f);
        } else if path.extension().is_some_and(|e| e == "rs") {
            f(&path);
        }
    }
}

/// Drop `//` line comments and `/* */` blocks so a mention in prose is not an edge.
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' && (i == 0 || b[i - 1] != b':') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(b.len());
            continue;
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}
