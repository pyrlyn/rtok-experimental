// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Toolchain row coverage (plan T222).
//!
//! Every non-path crate named in a workspace manifest dependency table must
//! have a row in the `## cargo` table of `toolchain.md`, and vice versa.
//! Pattern of `tests/config_coverage.rs`: read files at test time and assert
//! coverage, no blessing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// All manifests whose direct deps `toolchain.md` claims to list.
/// The `fuzz/` crate is excluded from the cargo workspace (its own workspace, nightly
/// only) but its crates still need `toolchain.md` rows, so it is listed here explicitly.
const MANIFESTS: &[&str] = &[
    "Cargo.toml",
    "crates/rtok-agent-sdk/Cargo.toml",
    "crates/rtok-config/Cargo.toml",
    "crates/rtok-plugin-sdk/Cargo.toml",
    "crates/rtok-store/Cargo.toml",
    "crates/rtok-sys/Cargo.toml",
    "crates/rtok-wasm-demo-guest/Cargo.toml",
    "fuzz/Cargo.toml",
];

#[test]
fn toolchain_rows_cover_manifests() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut manifest = BTreeSet::new();
    for name in MANIFESTS {
        manifest.extend(manifest_deps(&root.join(name)));
    }
    let table = toolchain_pkgs(&root.join("toolchain.md"));

    let missing: Vec<&String> = manifest.difference(&table).collect();
    let extra: Vec<&String> = table.difference(&manifest).collect();
    assert!(
        missing.is_empty(),
        "manifest deps with no `toolchain.md` cargo row: {missing:?}"
    );
    assert!(
        extra.is_empty(),
        "`toolchain.md` cargo rows with no manifest dep: {extra:?}"
    );
}

/// Names of all crates in dependency tables at any nesting level
/// (`[dependencies]`, `[dev-dependencies]`, `[build-dependencies]` and the
/// `[target.*]` variants), minus path (workspace-local) crates.
fn manifest_deps(path: &Path) -> BTreeSet<String> {
    let text = std::fs::read_to_string(path).unwrap();
    let doc: toml_edit::DocumentMut = text.parse().unwrap();
    let mut out = BTreeSet::new();
    collect_deps(doc.as_item(), &mut out);
    out
}

fn collect_deps(item: &toml_edit::Item, out: &mut BTreeSet<String>) {
    if let toml_edit::Item::Table(table) = item {
        for (key, value) in table {
            if matches!(
                key,
                "dependencies" | "dev-dependencies" | "build-dependencies"
            ) {
                if let toml_edit::Item::Table(deps) = value {
                    for (name, spec) in deps {
                        if !is_path_dep(spec) {
                            out.insert(name.to_string());
                        }
                    }
                }
            } else {
                collect_deps(value, out);
            }
        }
    }
}

fn is_path_dep(spec: &toml_edit::Item) -> bool {
    match spec {
        toml_edit::Item::Value(toml_edit::Value::InlineTable(table)) => table.contains_key("path"),
        toml_edit::Item::Table(table) => table.contains_key("path"),
        _ => false,
    }
}

/// Names in the first column of the `## cargo` table in `toolchain.md`.
fn toolchain_pkgs(path: &Path) -> BTreeSet<String> {
    let text = std::fs::read_to_string(path).unwrap();
    let mut in_cargo = false;
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(level) = trimmed.strip_prefix("## ") {
            in_cargo = level == "cargo";
            continue;
        }
        if !in_cargo || !trimmed.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = trimmed.split('|').map(str::trim).collect();
        let Some(name) = cells.get(1) else {
            continue;
        };
        if name.is_empty() || *name == "Package" || name.starts_with("---") {
            continue;
        }
        out.insert((*name).to_string());
    }
    out
}
