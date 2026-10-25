// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Usage search for `rtok review`, on the existing graph index (no second indexer).
//!
//! * changed symbols: index definitions whose span a hunk touches, plus names declared on
//!   added or removed lines (a deleted or renamed definition is gone from the new-side index);
//! * dependents: files that reference a changed symbol (`symbol_ref_groups`, plus aliased
//!   references found through the referencing file's imports);
//! * dependencies: definitions the changed symbols call (`symbol_callees` → `symbol_defs`).
//!
//! The index links by bare name, so every link is checked before it counts:
//!
//! 1. **imports**: a referencing file that imports the name from one definition's module
//!    belongs to that definition, not to a same-name one elsewhere ([`super::imports`]);
//! 2. **qualified paths**: `store::value::Value` inline, and `use … as Alias`, count too;
//! 3. **kinds**: a `call` reference links only to a callable definition, a `type`/`class`/
//!    `implementation` reference only to a type;
//! 4. **ambiguity**: a name with more than `max_defs` definitions and no import or directory
//!    evidence is kept but marked `ambiguous` and ranked last, so a budget drops it first.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use anyhow::Result;
use rtok_plugin_sdk::Ctx;

use super::diff::{FileDiff, declared_names, is_code};
use super::imports::{self, Import};

/// Default for [`Search::max_defs`]: more same-name definitions than this, with no
/// evidence, makes a link ambiguous.
pub const MAX_DEFS: usize = 3;

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
    "cfg",
    "derive",
    "allow",
    "assert",
    "assert_eq",
    "vec",
    "matches",
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

const CALLABLE: &[&str] = &["function", "method", "macro", "constructor"];
const TYPES: &[&str] = &[
    "struct",
    "enum",
    "trait",
    "type",
    "interface",
    "class",
    "union",
];

/// Why a file is in the review context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
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
    /// Every link to this file is a guess between many same-name definitions.
    pub ambiguous: bool,
}

/// Knobs for [`related`].
#[derive(Debug, Clone, Copy)]
pub struct Search {
    pub max_defs: usize,
}

impl Default for Search {
    fn default() -> Self {
        Self { max_defs: MAX_DEFS }
    }
}

/// Changed symbols per changed file, from index spans and added/removed declarations.
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

/// Whether a reference of `ref_kind` may resolve to a definition of `def_kind`.
/// An unknown kind on either side allows the link.
pub fn kinds_match(ref_kind: Option<&str>, def_kind: &str) -> bool {
    match ref_kind {
        Some("call") => !TYPES.contains(&def_kind) || def_kind == "class" || def_kind == "struct",
        Some("type" | "class" | "implementation") => !CALLABLE.contains(&def_kind),
        _ => true,
    }
}

/// How a referencing file was tied to one definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Evidence {
    /// The file imports the name from this definition's module, or names it qualified.
    Import,
    /// The definition is in the same file or directory.
    Near,
    /// Only the name matches.
    Name,
}

/// File text and parsed imports, read once per review.
#[derive(Default)]
struct Files {
    root: std::path::PathBuf,
    cache: HashMap<String, (String, Vec<Import>)>,
}

impl Files {
    fn get(&mut self, path: &str) -> &(String, Vec<Import>) {
        let root = self.root.clone();
        self.cache.entry(path.to_string()).or_insert_with(|| {
            let text = std::fs::read_to_string(root.join(path)).unwrap_or_default();
            let imps = imports::parse(path, &text);
            (text, imps)
        })
    }
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

/// Pick the definitions `name` in `ref_file` resolves to, out of `defs` (`(path, kind)`),
/// with the evidence for them. Import evidence beats directory evidence beats the name.
fn resolve(
    files: &mut Files,
    ref_file: &str,
    name: &str,
    defs: &[(String, String)],
) -> (Vec<String>, Evidence) {
    let (text, imps) = files.get(ref_file).clone();
    let named: Vec<&Import> = imps.iter().filter(|i| i.alias == name).collect();
    let by_import: Vec<String> = defs
        .iter()
        .filter(|(p, _)| {
            named.iter().any(|i| imports::points_at(i, p))
                || imports::qualified_inline(&text, p, name)
        })
        .map(|(p, _)| p.clone())
        .collect();
    if !by_import.is_empty() {
        return (by_import, Evidence::Import);
    }
    // The file imports this name from a module no indexed definition lives in (another
    // crate, the standard library): none of the same-name definitions is meant.
    if named.iter().any(|i| !i.module().is_empty()) {
        return (Vec::new(), Evidence::Import);
    }
    if defs.len() == 1 {
        return (vec![defs[0].0.clone()], Evidence::Import);
    }
    let near: Vec<String> = defs
        .iter()
        .filter(|(p, _)| p == ref_file || parent(p) == parent(ref_file))
        .map(|(p, _)| p.clone())
        .collect();
    // One definition nearby is evidence; several nearby are as good as a bare name.
    if near.len() == 1 {
        return (near, Evidence::Near);
    }
    (
        defs.iter().map(|(p, _)| p.clone()).collect(),
        Evidence::Name,
    )
}

#[derive(Default)]
struct Acc {
    map: BTreeMap<String, Related>,
}

impl Acc {
    fn add(&mut self, path: &str, rel: Relation, sym: &str, focus: &str, ambiguous: bool) {
        let e = self.map.entry(path.to_string()).or_insert_with(|| Related {
            path: path.to_string(),
            relation: rel,
            symbols: BTreeSet::new(),
            focus: BTreeSet::new(),
            hits: 0,
            ambiguous: true,
        });
        e.relation = e.relation.min(rel);
        e.symbols.insert(sym.to_string());
        if !focus.is_empty() {
            e.focus.insert(focus.to_string());
        }
        e.hits += 1;
        e.ambiguous &= ambiguous;
    }
}

/// Files tied to the changed symbols, changed files excluded. Ranked: unambiguous before
/// ambiguous, dependents before dependencies, then by hits, then by path.
pub fn related(
    cx: &Ctx,
    key: &str,
    root: &Path,
    changed: &BTreeMap<String, BTreeSet<String>>,
    search: Search,
) -> Result<Vec<Related>> {
    let mut files = Files {
        root: root.to_path_buf(),
        ..Files::default()
    };
    let mut acc = Acc::default();
    let ref_kinds_of = |name: &str| -> HashMap<(String, String), Vec<String>> {
        let mut m: HashMap<(String, String), Vec<String>> = HashMap::new();
        for (p, s, k) in cx.symbol_ref_kinds(key, name).unwrap_or_default() {
            m.entry((p, s)).or_default().push(k);
        }
        m
    };
    for (changed_file, names) in changed {
        for name in names {
            let defs: Vec<(String, String)> = cx
                .symbol_defs(key, name)
                .unwrap_or_default()
                .into_iter()
                .map(|(p, k, ..)| (p, k))
                .collect();
            // The changed definition's kind; a removed one is not in the index any more.
            let own_kind = defs
                .iter()
                .find(|(p, _)| p == changed_file)
                .map(|(_, k)| k.clone());
            let kinds = ref_kinds_of(name);
            // Dependents: bare-name references, then aliased ones.
            let mut refs: Vec<(String, String, String)> = cx
                .symbol_ref_groups(key, name)
                .unwrap_or_default()
                .into_iter()
                .map(|(p, s, ..)| (p, s, name.clone()))
                .collect();
            for (p, _) in cx.symbol_importers(key, name).unwrap_or_default() {
                let imps = files.get(&p).1.clone();
                for i in imps.iter().filter(|i| i.item() == name && i.alias != *name) {
                    for (rp, s, ..) in cx.symbol_ref_groups(key, &i.alias).unwrap_or_default() {
                        if rp == p {
                            refs.push((rp, s, i.alias.clone()));
                        }
                    }
                }
            }
            for (path, scope, via) in refs {
                if changed.contains_key(&path) {
                    continue;
                }
                if let Some(k) = &own_kind
                    && via == *name
                {
                    let ks = kinds.get(&(path.clone(), scope.clone()));
                    if let Some(ks) = ks
                        && !ks.iter().any(|rk| kinds_match(Some(rk), k))
                    {
                        continue;
                    }
                }
                let (targets, ev) = if via != *name {
                    (vec![changed_file.clone()], Evidence::Import)
                } else if defs.is_empty() {
                    (vec![changed_file.clone()], Evidence::Name)
                } else {
                    resolve(&mut files, &path, name, &defs)
                };
                if !targets.iter().any(|t| t == changed_file) && !defs.is_empty() {
                    continue;
                }
                let ambiguous = ev == Evidence::Name && defs.len() > search.max_defs;
                acc.add(&path, Relation::Dependent, name, &scope, ambiguous);
            }
            // Dependencies: what the changed definition calls, resolved from the changed file.
            for (def_path, _, callee, _) in cx.symbol_callees(key, name).unwrap_or_default() {
                if def_path != *changed_file
                    || COMMON.contains(&callee.as_str())
                    || callee.len() < 3
                {
                    continue;
                }
                let cdefs: Vec<(String, String)> = cx
                    .symbol_defs(key, &callee)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(p, k, ..)| (p, k))
                    // A module is a namespace, not a contract worth a digest.
                    .filter(|(_, k)| k != "module")
                    .collect();
                if cdefs.is_empty() {
                    continue;
                }
                let ck = ref_kinds_of(&callee);
                let rk = ck.get(&(changed_file.clone(), name.clone()));
                let cdefs: Vec<(String, String)> = cdefs
                    .into_iter()
                    .filter(|(_, k)| rk.is_none_or(|rk| rk.iter().any(|r| kinds_match(Some(r), k))))
                    .collect();
                if cdefs.is_empty() {
                    continue;
                }
                let (targets, ev) = resolve(&mut files, changed_file, &callee, &cdefs);
                let ambiguous = ev == Evidence::Name && cdefs.len() > search.max_defs;
                for t in targets {
                    if !changed.contains_key(&t) {
                        acc.add(&t, Relation::Dependency, &callee, &callee, ambiguous);
                    }
                }
            }
        }
    }
    let mut out: Vec<Related> = acc.map.into_values().collect();
    out.sort_by(|a, b| {
        a.ambiguous
            .cmp(&b.ambiguous)
            .then(a.relation.cmp(&b.relation))
            .then(b.hits.cmp(&a.hits))
            .then(a.path.cmp(&b.path))
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_filter() {
        assert!(kinds_match(Some("call"), "function"));
        assert!(!kinds_match(Some("call"), "trait"));
        assert!(
            kinds_match(Some("call"), "struct"),
            "`Foo {{ .. }}` and `Foo(..)` construct"
        );
        assert!(!kinds_match(Some("type"), "function"));
        assert!(kinds_match(Some("type"), "struct"));
        assert!(kinds_match(None, "function"));
    }
}
