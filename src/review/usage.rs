// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Usage search for `rtok review`, on the existing graph index (no second indexer).
//!
//! * changed symbols: index definitions whose span a hunk touches, plus names declared on
//!   removed lines (a deleted or renamed definition is gone from the new-side index);
//! * dependents: files that reference a changed symbol (`symbol_ref_groups`);
//! * dependencies: definitions the changed symbols call (`symbol_callees` → `symbol_defs`).

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use rtok_plugin_sdk::Ctx;

use super::diff::{FileDiff, declared_names, is_code};

/// A changed name defined in more files than this is too generic (`new`, `run`) to follow.
pub const MAX_DEF_SITES: usize = 6;
/// A callee defined in more files than this is not followed: the call is ambiguous.
pub const MAX_CALLEE_SITES: usize = 2;
/// Callee names that are almost always std or trait methods, never worth a digest.
const COMMON: &[&str] = &[
    "new",
    "default",
    "from",
    "into",
    "clone",
    "push",
    "pop",
    "get",
    "set",
    "len",
    "is_empty",
    "insert",
    "remove",
    "contains",
    "iter",
    "map",
    "unwrap",
    "expect",
    "ok",
    "err",
    "to_string",
    "as_str",
    "spawn",
    "run",
    "now",
    "args",
    "env",
    "stdin",
    "stdout",
    "stderr",
    "write",
    "read",
    "open",
    "close",
    "next",
    "fmt",
    "eq",
    "cmp",
    "hash",
    "drop",
    "build",
    "send",
    "parse",
    "join",
    "split",
    "trim",
    "format",
    "print",
    "println",
    "log",
    "main",
    "test",
];

/// Why a file is in the review context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Relation {
    /// References a changed symbol: its call sites may break.
    Dependent,
    /// Defines something the changed code calls: its contract is what the diff relies on.
    Dependency,
}

/// One related file and the symbols that tie it to the diff.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Related {
    pub path: String,
    pub relation: Relation,
    pub symbols: BTreeSet<String>,
    /// Definitions in this file worth a digest line: the callers of a changed symbol, or
    /// the callee itself.
    pub focus: BTreeSet<String>,
    pub hits: u32,
}

/// Changed symbols per changed file, from index spans and removed-line declarations.
pub fn changed_symbols(
    cx: &Ctx,
    key: &str,
    files: &[FileDiff],
) -> BTreeMap<String, BTreeSet<String>> {
    let mut out = BTreeMap::new();
    for f in files {
        let mut names = BTreeSet::new();
        if !is_code(&f.path) {
            out.insert(f.path.clone(), names);
            continue;
        }
        if !f.deleted {
            let defs = cx.symbol_file_defs(key, &f.path).unwrap_or_default();
            for h in &f.hunks {
                let (a, b) = h.new_range();
                for (name, _, line, end) in &defs {
                    let (s, e) = (*line as u32, (*end).max(*line) as u32);
                    if s <= b && a <= e {
                        names.insert(name.clone());
                    }
                }
            }
        }
        for h in &f.hunks {
            names.extend(declared_names(&h.removed));
            names.extend(declared_names(&h.added));
        }
        out.insert(f.path.clone(), names);
    }
    out
}

/// Files tied to the changed symbols, changed files excluded, ranked by hits then path.
pub fn related(
    cx: &Ctx,
    key: &str,
    changed: &BTreeMap<String, BTreeSet<String>>,
) -> Result<Vec<Related>> {
    let mut map: BTreeMap<(String, Relation), Related> = BTreeMap::new();
    let mut add = |path: &str, rel: Relation, sym: &str, focus: &str| {
        if changed.contains_key(path) {
            return;
        }
        let e = map
            .entry((path.to_string(), rel))
            .or_insert_with(|| Related {
                path: path.to_string(),
                relation: rel,
                symbols: BTreeSet::new(),
                focus: BTreeSet::new(),
                hits: 0,
            });
        e.symbols.insert(sym.to_string());
        if !focus.is_empty() {
            e.focus.insert(focus.to_string());
        }
        e.hits += 1;
    };
    let all: BTreeSet<&String> = changed.values().flatten().collect();
    for name in all {
        let sites = cx.symbol_defs(key, name).unwrap_or_default();
        if sites.len() > MAX_DEF_SITES {
            continue;
        }
        for (path, scope, _, _) in cx.symbol_ref_groups(key, name).unwrap_or_default() {
            add(&path, Relation::Dependent, name, &scope);
        }
        for (_, _, callee, _) in cx.symbol_callees(key, name).unwrap_or_default() {
            if COMMON.contains(&callee.as_str()) || callee.len() < 3 {
                continue;
            }
            let defs = cx.symbol_defs(key, &callee).unwrap_or_default();
            if defs.is_empty() || defs.len() > MAX_CALLEE_SITES {
                continue;
            }
            for (path, _, _, _) in defs {
                add(&path, Relation::Dependency, &callee, &callee);
            }
        }
    }
    // A file both dependent and dependency is kept once, as dependent (its call sites matter more).
    let mut out: Vec<Related> = Vec::new();
    for ((path, _), r) in map {
        if let Some(prev) = out.iter_mut().find(|p| p.path == path) {
            prev.symbols.extend(r.symbols);
            prev.focus.extend(r.focus);
            prev.hits += r.hits;
            prev.relation = prev.relation.min(r.relation);
        } else {
            out.push(r);
        }
    }
    out.sort_by(|a, b| {
        a.relation
            .cmp(&b.relation)
            .then(b.hits.cmp(&a.hits))
            .then(a.path.cmp(&b.path))
    });
    Ok(out)
}
