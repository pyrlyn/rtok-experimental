// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Unified diff parsing for `rtok review`: changed files, hunks, added and removed lines.
//!
//! Accepts `git diff` output and plain `diff -u` output. Binary patches and mode-only
//! changes produce a file entry with no hunks.

/// One hunk: old and new line ranges plus the lines it removes and adds (without the sign).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

impl Hunk {
    /// The new-side range as inclusive `(first, last)`; a pure deletion maps to the line
    /// it sits after, so it still touches the definition around it.
    pub fn new_range(&self) -> (u32, u32) {
        let first = self.new_start.max(1);
        (first, first + self.new_len.saturating_sub(1))
    }
}

/// One file in the diff. `path` is the new path, or the old one when the file was deleted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub added: bool,
    pub deleted: bool,
    pub hunks: Vec<Hunk>,
}

/// Strip the `a/` / `b/` prefix git adds, and any tab-separated timestamp `diff -u` adds.
fn clean_path(raw: &str) -> Option<String> {
    let raw = raw.split('\t').next().unwrap_or(raw).trim();
    if raw == "/dev/null" {
        return None;
    }
    let raw = raw.trim_matches('"');
    let p = raw
        .strip_prefix("a/")
        .or_else(|| raw.strip_prefix("b/"))
        .unwrap_or(raw);
    Some(p.to_string())
}

fn parse_range(s: &str) -> (u32, u32) {
    let s = s.trim_start_matches(['-', '+']);
    match s.split_once(',') {
        Some((a, b)) => (a.parse().unwrap_or(0), b.parse().unwrap_or(0)),
        None => (s.parse().unwrap_or(0), 1),
    }
}

/// `@@ -a,b +c,d @@ ...` → `(a, b, c, d)`.
fn parse_hunk_header(line: &str) -> Option<(u32, u32, u32, u32)> {
    let rest = line.strip_prefix("@@ ")?;
    let mut it = rest.split_whitespace();
    let (os, ol) = parse_range(it.next()?);
    let (ns, nl) = parse_range(it.next()?);
    Some((os, ol, ns, nl))
}

/// Parse a unified diff. Files come back in diff order; an empty or non-diff input is empty.
pub fn parse(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut cur: Option<FileDiff> = None;
    let mut old: Option<String> = None;
    let mut in_hunk = false;
    // Old and new lines still owed by the open hunk; at zero the hunk is closed.
    let (mut left_old, mut left_new) = (0u32, 0u32);
    let flush = |cur: &mut Option<FileDiff>, files: &mut Vec<FileDiff>| {
        if let Some(f) = cur.take()
            && !f.path.is_empty()
        {
            files.push(f);
        }
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            flush(&mut cur, &mut files);
            in_hunk = false;
            // Fallback path from the header; `+++` overrides it when present.
            let path = rest
                .rsplit_once(" b/")
                .map(|(_, b)| b.to_string())
                .unwrap_or_default();
            cur = Some(FileDiff {
                path,
                ..FileDiff::default()
            });
            old = None;
            continue;
        }
        if !in_hunk && let Some(rest) = line.strip_prefix("--- ") {
            if cur.as_ref().is_some_and(|f| !f.hunks.is_empty()) {
                flush(&mut cur, &mut files);
            }
            old = clean_path(rest);
            let f = cur.get_or_insert_with(FileDiff::default);
            if old.is_none() {
                f.added = true;
            }
            continue;
        }
        if !in_hunk && let Some(rest) = line.strip_prefix("+++ ") {
            let f = cur.get_or_insert_with(FileDiff::default);
            match clean_path(rest) {
                Some(p) => f.path = p,
                None => {
                    f.deleted = true;
                    if let Some(o) = &old {
                        f.path = o.clone();
                    }
                }
            }
            if old.is_some() && old.as_deref() != Some(f.path.as_str()) {
                f.old_path = old.clone();
            }
            continue;
        }
        if line.starts_with("new file mode") {
            if let Some(f) = cur.as_mut() {
                f.added = true;
            }
            continue;
        }
        if line.starts_with("deleted file mode") {
            if let Some(f) = cur.as_mut() {
                f.deleted = true;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("rename from ") {
            if let Some(f) = cur.as_mut() {
                f.old_path = Some(rest.to_string());
            }
            continue;
        }
        if let Some((os, ol, ns, nl)) = parse_hunk_header(line) {
            in_hunk = ol > 0 || nl > 0;
            (left_old, left_new) = (ol, nl);
            let f = cur.get_or_insert_with(FileDiff::default);
            f.hunks.push(Hunk {
                old_start: os,
                old_len: ol,
                new_start: ns,
                new_len: nl,
                ..Hunk::default()
            });
            continue;
        }
        if in_hunk && let Some(h) = cur.as_mut().and_then(|f| f.hunks.last_mut()) {
            if let Some(a) = line.strip_prefix('+') {
                h.added.push(a.to_string());
                left_new = left_new.saturating_sub(1);
            } else if let Some(r) = line.strip_prefix('-') {
                h.removed.push(r.to_string());
                left_old = left_old.saturating_sub(1);
            } else if !line.starts_with('\\') {
                left_old = left_old.saturating_sub(1);
                left_new = left_new.saturating_sub(1);
            }
            if left_old == 0 && left_new == 0 {
                in_hunk = false;
            }
        }
    }
    flush(&mut cur, &mut files);
    files
}

/// Source extensions whose declarations [`declared_names`] reads; prose and data files
/// have none.
pub const CODE_EXTS: &[&str] = &[
    "rs", "py", "ts", "tsx", "js", "jsx", "mjs", "go", "java", "kt", "swift", "c", "h", "cc",
    "cpp", "hpp", "rb", "cs", "scala", "php",
];

/// Whether `path` is source code by extension.
pub fn is_code(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| CODE_EXTS.contains(&e))
}

/// Names declared on removed or added lines (`fn x`, `struct X`, `def x`, `class X`, ...).
/// Catches a removed or renamed definition the new-side index no longer has. Only a line
/// that starts with a declaration (after modifiers such as `pub` or `async`) counts.
pub fn declared_names(lines: &[String]) -> Vec<String> {
    const KW: &[&str] = &[
        "fn",
        "struct",
        "enum",
        "trait",
        "type",
        "def",
        "class",
        "function",
        "interface",
        "func",
    ];
    const MODS: &[&str] = &[
        "pub",
        "crate",
        "super",
        "async",
        "unsafe",
        "const",
        "export",
        "default",
        "static",
        "private",
        "public",
        "protected",
        "abstract",
        "final",
        "extern",
    ];
    let mut out = Vec::new();
    for l in lines {
        let toks = l
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|t| !t.is_empty())
            .skip_while(|t| MODS.contains(t));
        let toks: Vec<&str> = toks.take(2).collect();
        if let [kw, name] = toks[..]
            && KW.contains(&kw)
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && !l.trim_start().starts_with(['/', '#', '*'])
        {
            out.push(name.to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIT: &str = "diff --git a/src/lib.rs b/src/lib.rs
index 1..2 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,4 @@
 use std::fmt;
-pub fn old_name() {}
+pub fn new_name() {}
+pub fn extra() {}
 fn keep() {}
@@ -10 +11,0 @@ fn keep() {}
-fn gone() {}
diff --git a/b.py b/b.py
new file mode 100644
--- /dev/null
+++ b/b.py
@@ -0,0 +1,2 @@
+def hello():
+    return 1
diff --git a/c.txt b/c.txt
deleted file mode 100644
--- a/c.txt
+++ /dev/null
@@ -1 +0,0 @@
--- a line that looks like a header
";

    #[test]
    fn parses_files_hunks_and_flags() {
        let files = parse(GIT);
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["src/lib.rs", "b.py", "c.txt"]);
        let lib = &files[0];
        assert_eq!(lib.hunks.len(), 2);
        assert_eq!(lib.hunks[0].new_range(), (1, 4));
        assert_eq!(lib.hunks[0].removed, ["pub fn old_name() {}"]);
        assert_eq!(lib.hunks[0].added.len(), 2);
        assert_eq!(lib.hunks[1].new_range(), (11, 11));
        assert!(files[1].added && !files[1].deleted);
        assert!(files[2].deleted);
        assert_eq!(
            files[2].hunks[0].removed,
            ["-- a line that looks like a header"]
        );
    }

    #[test]
    fn plain_diff_u_and_rename() {
        let plain = "--- a.rs\t2026-01-01\n+++ a.rs\t2026-01-02\n@@ -1 +1 @@\n-x\n+y\n";
        let f = parse(plain);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].path, "a.rs");
        let ren =
            "diff --git a/x.rs b/y.rs\nsimilarity index 90%\nrename from x.rs\nrename to y.rs\n";
        let f = parse(ren);
        assert_eq!(f[0].path, "y.rs");
        assert_eq!(f[0].old_path.as_deref(), Some("x.rs"));
        assert!(parse("not a diff\n").is_empty());
    }

    #[test]
    fn declared_names_reads_keywords() {
        let lines = vec![
            "pub fn old_name() {}".to_string(),
            "def hello(x):".to_string(),
            "let a = 1;".to_string(),
            "pub struct Thing {".to_string(),
            "    pub(crate) async fn inner(&self) {}".to_string(),
            "/// the fn folds the type is".to_string(),
            "text where fn appears late".to_string(),
        ];
        assert_eq!(
            declared_names(&lines),
            ["Thing", "hello", "inner", "old_name"]
        );
        assert!(is_code("a/b.rs") && !is_code("plan/done.md"));
    }
}
