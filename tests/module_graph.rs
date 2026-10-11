// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The production module graph is a DAG with the layers in `architecture.md` §2.
//!
//! Comments, string literals and `#[cfg(test)]` items are not edges. A new
//! upward edge fails here with the file and line that introduced it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const SURFACES: &[&str] = &["cli", "hooks", "mcp", "proxy", "web", "tui", "demon"];
const APP: &[&str] = &["plugins", "agents", "measure", "report", "doctor"];
const CORE_FOUR: &[&str] = &["config", "store", "log", "tokens"];
const CORE: &[&str] = &["plugin", "config", "store", "log", "tokens"];
type EdgeSet = BTreeMap<String, BTreeSet<String>>;
type EdgeLocs = BTreeMap<(String, String), Vec<String>>;

const FOUNDATION: &[&str] = &[
    "fs",
    "sanitize",
    "proc",
    "tls",
    "names",
    "lane",
    "diff",
    "since",
    "stdio",
    "task_id",
    "logfile",
    "progress",
    "bytes",
    "project",
    "disk_usage",
    "removal_preview",
];

#[test]
fn production_modules_are_a_layered_dag() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mods = modules(&root);
    let modset: BTreeSet<&str> = mods.iter().map(String::as_str).collect();
    let (edges, locs) = edges(&root, &mods);

    let sccs = strongly_connected(&mods, &edges);
    let cycles: Vec<_> = sccs.into_iter().filter(|s| s.len() > 1).collect();
    assert!(
        cycles.is_empty(),
        "production module cycle:\n{}",
        cycles
            .iter()
            .map(|c| format!("  {}", c.join(" → ")))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let mut problems = Vec::new();
    for (src, dsts) in &edges {
        for dst in dsts {
            let at = locs
                .get(&(src.clone(), dst.clone()))
                .map(|v| v.join(", "))
                .unwrap_or_default();
            if !is_surface(src) && matches!(dst.as_str(), "cli" | "web" | "tui") {
                problems.push(format!("{src} → {dst} ({at}): below surfaces"));
            }
            if is_app(src) && is_surface(dst) {
                problems.push(format!("{src} → {dst} ({at}): app → surface"));
            }
            if is_core(src) && (is_surface(dst) || is_app(dst)) {
                problems.push(format!("{src} → {dst} ({at}): core → above"));
            }
            if CORE_FOUR.contains(&src.as_str()) && !allowed_for_core_four(dst, &modset) {
                problems.push(format!(
                    "{src} → {dst} ({at}): config/store/log/tokens may only use core and foundation"
                ));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "upward module edges:\n{}",
        problems.join("\n")
    );
}

fn is_surface(name: &str) -> bool {
    SURFACES.contains(&name)
}
fn is_app(name: &str) -> bool {
    APP.contains(&name)
}
fn is_core(name: &str) -> bool {
    CORE.contains(&name)
}
fn allowed_for_core_four(dst: &str, mods: &BTreeSet<&str>) -> bool {
    let _ = mods;
    CORE.contains(&dst) || FOUNDATION.contains(&dst)
}

fn modules(root: &Path) -> Vec<String> {
    let mut mods = Vec::new();
    for ent in std::fs::read_dir(root).unwrap() {
        let ent = ent.unwrap();
        let name = ent.file_name().to_string_lossy().into_owned();
        if name == "lib.rs" || name == "main.rs" || name == "bin" || name == "fuzzing.rs" {
            continue;
        }
        let path = ent.path();
        if path.extension().is_some_and(|e| e == "rs") {
            mods.push(name.trim_end_matches(".rs").to_string());
        } else if path.is_dir() && has_rs(&path) {
            mods.push(name);
        }
    }
    mods.sort();
    mods
}

fn has_rs(dir: &Path) -> bool {
    fn walk(dir: &Path) -> bool {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return false;
        };
        for ent in rd.flatten() {
            let p = ent.path();
            if p.extension().is_some_and(|e| e == "rs") || (p.is_dir() && walk(&p)) {
                return true;
            }
        }
        false
    }
    walk(dir)
}

fn edges(root: &Path, mods: &[String]) -> (EdgeSet, EdgeLocs) {
    let modset: BTreeSet<&str> = mods.iter().map(String::as_str).collect();
    let mut edges: EdgeSet = BTreeMap::new();
    let mut locs: EdgeLocs = BTreeMap::new();
    for m in mods {
        let mut files = Vec::new();
        let file = root.join(format!("{m}.rs"));
        let dir = root.join(m);
        if file.is_file() {
            files.push(file);
        }
        if dir.is_dir() {
            collect_rs(&dir, &mut files);
        }
        for path in files {
            let raw = std::fs::read_to_string(&path).unwrap();
            let cleaned = strip_cfg_test(&lex_mask(&raw));
            let rel = path.strip_prefix(root).unwrap().display().to_string();
            for (line, dst) in crate_refs(&cleaned) {
                if dst == *m || !modset.contains(dst.as_str()) {
                    continue;
                }
                edges.entry(m.clone()).or_default().insert(dst.clone());
                let slot = locs.entry((m.clone(), dst.clone())).or_default();
                if slot.len() < 8 {
                    slot.push(format!("{rel}:{line}"));
                }
            }
        }
    }
    (edges, locs)
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut ents: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    ents.sort_by_key(|e| e.file_name());
    for ent in ents {
        let p = ent.path();
        if p.is_dir() {
            collect_rs(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

fn crate_refs(text: &str) -> Vec<(usize, String)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 7 < b.len() {
        if b[i..].starts_with(b"crate::") {
            let start = i + 7;
            let mut j = start;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            if j > start {
                let name = text[start..j].to_string();
                let line = text[..i].bytes().filter(|c| *c == b'\n').count() + 1;
                out.push((line, name));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Comments and string contents become spaces. Newlines stay, so line numbers match the file.
fn lex_mask(text: &str) -> String {
    let b = text.as_bytes();
    let n = b.len();
    let mut out = b.to_vec();
    let mut i = 0;
    let blank = |out: &mut [u8], a: usize, c: usize| {
        for slot in out.iter_mut().take(c).skip(a) {
            if *slot != b'\n' {
                *slot = b' ';
            }
        }
    };
    while i < n {
        if b[i] == b'/' && i + 1 < n && b[i + 1] == b'/' {
            let j = b[i..]
                .iter()
                .position(|c| *c == b'\n')
                .map(|p| i + p)
                .unwrap_or(n);
            blank(&mut out, i, j);
            i = j;
            continue;
        }
        if b[i] == b'/' && i + 1 < n && b[i + 1] == b'*' {
            let j = b[i + 2..]
                .windows(2)
                .position(|w| w == b"*/")
                .map(|p| i + 2 + p + 2)
                .unwrap_or(n);
            blank(&mut out, i, j);
            i = j;
            continue;
        }
        if b[i] == b'b' && i + 1 < n && matches!(b[i + 1], b'r' | b'\'' | b'"') {
            i += 1;
        }
        if b[i] == b'r' && i + 1 < n && matches!(b[i + 1], b'"' | b'#') {
            let mut j = i + 1;
            let mut hashes = 0usize;
            while j < n && b[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < n && b[j] == b'"' {
                j += 1;
                let end = {
                    let mut e = vec![b'"'];
                    e.extend(std::iter::repeat_n(b'#', hashes));
                    e
                };
                let k = b[j..]
                    .windows(end.len())
                    .position(|w| w == end.as_slice())
                    .map(|p| j + p + end.len())
                    .unwrap_or(n);
                blank(&mut out, i, k);
                i = k;
                continue;
            }
        }
        if b[i] == b'"' {
            let mut j = i + 1;
            while j < n {
                if b[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if b[j] == b'"' {
                    j += 1;
                    break;
                }
                j += 1;
            }
            blank(&mut out, i, j);
            i = j;
            continue;
        }
        if b[i] == b'\'' {
            let mut j = i + 1;
            if j < n && (b[j].is_ascii_alphabetic() || b[j] == b'_') {
                let mut k = j;
                while k < n && (b[k].is_ascii_alphanumeric() || b[k] == b'_') {
                    k += 1;
                }
                if k < n && b[k] != b'\'' && k > j {
                    i = k;
                    continue;
                }
            }
            j = i + 1;
            while j < n && j - i < 8 {
                if b[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if b[j] == b'\'' {
                    j += 1;
                    blank(&mut out, i, j);
                    break;
                }
                j += 1;
            }
            i = j.max(i + 1);
            continue;
        }
        i += 1;
    }
    String::from_utf8(out).expect("mask keeps utf-8")
}

fn strip_cfg_test(text: &str) -> String {
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].trim_start().starts_with("#[") {
            i += 1;
            continue;
        }
        let start = i;
        let mut attrs = Vec::new();
        while i < lines.len() && lines[i].trim_start().starts_with("#[") {
            attrs.push(lines[i].clone());
            i += 1;
        }
        while i < lines.len() && lines[i].trim().is_empty() {
            i += 1;
        }
        if !attrs.iter().any(|a| is_cfg_test(a)) {
            continue;
        }
        if i >= lines.len() {
            blank_lines(&mut lines, start, i);
            break;
        }
        let mut buf = lines[i].clone();
        i += 1;
        let mut brace = brace_delta(&buf);
        if buf.contains('{') || (i < lines.len() && lines[i].contains('{') && !buf.contains(';')) {
            while brace <= 0 && i < lines.len() && !buf.contains(';') {
                buf.push_str(&lines[i]);
                brace += brace_delta(&lines[i]);
                i += 1;
                if buf.contains('{') {
                    break;
                }
            }
            while i < lines.len() && brace > 0 {
                brace += brace_delta(&lines[i]);
                i += 1;
            }
        } else {
            while !buf.contains(';') && i < lines.len() {
                buf.push_str(&lines[i]);
                i += 1;
            }
        }
        blank_lines(&mut lines, start, i);
    }
    lines.concat()
}

fn is_cfg_test(attr: &str) -> bool {
    let flat: String = attr.chars().filter(|c| !c.is_whitespace()).collect();
    flat.contains("#[cfg(test)]") || flat.contains("cfg(all(test") || flat.contains("cfg(any(test")
}

fn brace_delta(s: &str) -> i32 {
    let mut n = 0i32;
    for c in s.chars() {
        if c == '{' {
            n += 1;
        } else if c == '}' {
            n -= 1;
        }
    }
    n
}

fn blank_lines(lines: &mut [String], start: usize, end: usize) {
    for line in lines.iter_mut().take(end).skip(start) {
        let nl = line.ends_with('\n');
        let spaces = " ".repeat(line.trim_end_matches('\n').len());
        *line = if nl { format!("{spaces}\n") } else { spaces };
    }
}

struct Tarjan<'a> {
    edges: &'a EdgeSet,
    index: usize,
    stack: Vec<String>,
    on: BTreeSet<String>,
    idx: BTreeMap<String, usize>,
    low: BTreeMap<String, usize>,
    sccs: Vec<Vec<String>>,
}

impl Tarjan<'_> {
    fn visit(&mut self, v: &str) {
        let n = self.index;
        self.idx.insert(v.to_string(), n);
        self.low.insert(v.to_string(), n);
        self.index += 1;
        self.stack.push(v.to_string());
        self.on.insert(v.to_string());
        let dsts: Vec<String> = self
            .edges
            .get(v)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        for w in dsts {
            if !self.idx.contains_key(&w) {
                self.visit(&w);
                let lw = self.low[&w];
                if lw < self.low[v] {
                    self.low.insert(v.to_string(), lw);
                }
            } else if self.on.contains(&w) && self.idx[&w] < self.low[v] {
                let iw = self.idx[&w];
                self.low.insert(v.to_string(), iw);
            }
        }
        if self.low[v] == self.idx[v] {
            let mut comp = Vec::new();
            while let Some(w) = self.stack.pop() {
                self.on.remove(&w);
                let done = w == v;
                comp.push(w);
                if done {
                    break;
                }
            }
            comp.sort();
            self.sccs.push(comp);
        }
    }
}

fn strongly_connected(mods: &[String], edges: &EdgeSet) -> Vec<Vec<String>> {
    let mut tarjan = Tarjan {
        edges,
        index: 0,
        stack: Vec::new(),
        on: BTreeSet::new(),
        idx: BTreeMap::new(),
        low: BTreeMap::new(),
        sccs: Vec::new(),
    };
    for m in mods {
        if !tarjan.idx.contains_key(m) {
            tarjan.visit(m);
        }
    }
    tarjan.sccs
}
