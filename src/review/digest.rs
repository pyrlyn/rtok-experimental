// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Digests for `rtok review`: imports, doc comments, signatures and data types of one file,
//! never a function body.
//!
//! Spans come from the graph index (`symbol_file_defs`), so every language the index
//! understands gets a digest; the text is cut from the file by line range. A file the
//! index has no definitions for digests to its imports only.

use std::collections::BTreeSet;

/// One definition as stored by the index: `(name, kind, line, end_line)`, 1-based.
pub type Def = (String, String, i32, i32);

/// Kinds whose whole span is a contract (fields, variants, aliases), not behaviour.
const DATA_KINDS: &[&str] = &["struct", "enum", "type", "interface"];
/// A data definition longer than this is cut to its signature like a function.
const MAX_DATA_SPAN: i32 = 40;
/// A signature that never reaches `{`, `;`, `:` or `=>` is cut after this many lines.
const MAX_SIG_LINES: usize = 8;
/// Doc lines kept above one definition.
const MAX_DOC_LINES: usize = 12;

fn is_import(t: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "use ",
        "pub use ",
        "pub(crate) use ",
        "import ",
        "from ",
        "#include",
        "require ",
        "package ",
        "extern crate ",
    ];
    PREFIXES.iter().any(|p| t.starts_with(p))
        || (t.starts_with("mod ") || t.starts_with("pub mod ")) && t.ends_with(';')
        || t.contains("= require(")
}

fn is_doc(t: &str) -> bool {
    t.starts_with("///")
        || t.starts_with("//!")
        || t.starts_with("/**")
        || t.starts_with("* ")
        || t == "*"
        || t.starts_with("*/")
        || t.starts_with("#[")
        || t.starts_with('@')
        || t.starts_with("# ") && !t.starts_with("# -*-")
}

/// Whether a data kind's whole span is safe to show: Rust tags a struct as `class`, while a
/// Python or JS class holds method bodies.
fn whole_span(kind: &str, path: &str) -> bool {
    DATA_KINDS.contains(&kind)
        || kind == "class" && (path.ends_with(".rs") || path.ends_with(".go"))
}

/// Signature lines from `start` (0-based): up to the line that opens the body or ends the item.
fn signature(lines: &[&str], start: usize) -> Vec<String> {
    let mut out = Vec::new();
    for l in lines.iter().skip(start).take(MAX_SIG_LINES) {
        let t = l.trim_end();
        if let Some(i) = body_open(t) {
            let head = t[..i].trim_end();
            out.push(format!("{head} {{ … }}"));
            return out;
        }
        out.push(t.to_string());
        let tt = t.trim();
        if tt.ends_with(';') || tt.ends_with(':') || tt.ends_with("=>") || tt.ends_with('}') {
            return out;
        }
    }
    out
}

/// Byte index of the `{` that opens a body on this line, ignoring `{` inside strings and
/// generic-free `{}` pairs that close on the same line (`struct A {}`).
fn body_open(t: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut open = None;
    let mut in_str = false;
    for (i, c) in t.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '{' if !in_str => {
                if depth == 0 {
                    open = Some(i);
                }
                depth += 1;
            }
            '}' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    open = None;
                }
            }
            _ => {}
        }
    }
    open
}

/// Python docstring right after a `def`/`class` signature.
fn docstring(lines: &[&str], after: usize) -> Vec<String> {
    let Some(first) = lines.get(after) else {
        return Vec::new();
    };
    let t = first.trim();
    let quote = if t.starts_with("\"\"\"") {
        "\"\"\""
    } else if t.starts_with("'''") {
        "'''"
    } else {
        return Vec::new();
    };
    let mut out = vec![first.trim_end().to_string()];
    if t.len() > 3 && t[3..].contains(quote) {
        return out;
    }
    for l in lines.iter().skip(after + 1).take(MAX_DOC_LINES) {
        out.push(l.trim_end().to_string());
        if l.contains(quote) {
            break;
        }
    }
    out
}

/// Digest of `src` (the file at `path`) from its index definitions. `focus`, when set,
/// keeps only those definitions (plus data types and imports): a related file is cut down
/// to the symbols the diff touches.
pub fn digest(path: &str, src: &str, defs: &[Def], focus: Option<&BTreeSet<String>>) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let mut keep: BTreeSet<usize> = BTreeSet::new();
    let mut text: std::collections::BTreeMap<usize, Vec<String>> = Default::default();
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim_start();
        let indent = l.len() - t.len();
        if indent <= 4 && is_import(t) {
            keep.insert(i);
            text.insert(i, vec![l.trim_end().to_string()]);
        }
    }
    let mut defs: Vec<&Def> = defs.iter().collect();
    defs.sort_by_key(|d| (d.2, d.0.clone()));
    let mut covered_until = 0usize;
    for (name, kind, line, end) in defs {
        let start = (*line as usize).saturating_sub(1);
        if start >= lines.len() || start < covered_until {
            continue;
        }
        let data = whole_span(kind, path) && end - line <= MAX_DATA_SPAN;
        if let Some(f) = focus
            && !f.contains(name)
            && !data
        {
            continue;
        }
        let mut block = Vec::new();
        let mut d = start;
        let mut doc = Vec::new();
        while d > 0 && doc.len() < MAX_DOC_LINES && is_doc(lines[d - 1].trim_start()) {
            d -= 1;
            doc.push(lines[d].trim_end().to_string());
        }
        doc.reverse();
        block.extend(doc);
        if data {
            let last = ((*end as usize).max(start + 1)).min(lines.len());
            block.extend(lines[start..last].iter().map(|l| l.trim_end().to_string()));
            covered_until = last;
        } else {
            let sig = signature(&lines, start);
            let n = sig.len();
            block.extend(sig);
            block.extend(docstring(&lines, start + n));
        }
        if !keep.contains(&d) {
            keep.insert(d);
            text.insert(d, block);
        }
    }
    let mut out = String::new();
    for (_, block) in text {
        for l in block {
            out.push_str(&l);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(name: &str, kind: &str, line: i32, end: i32) -> Def {
        (name.into(), kind.into(), line, end)
    }

    const RS: &str = "use std::io;
use crate::store::Store;

/// Parses one record.
/// Errors on bad input.
#[inline]
pub fn parse(input: &str) -> Result<Record, Error> {
    let secret_body_marker = input.len();
    Ok(Record { n: secret_body_marker })
}

/// A record.
pub struct Record {
    pub n: usize,
}

impl Record {
    pub fn len(&self) -> usize {
        self.n + 41
    }
}
";

    #[test]
    fn rust_digest_keeps_contract_and_drops_bodies() {
        let defs = vec![
            d("parse", "function", 7, 10),
            d("Record", "struct", 13, 15),
            d("Record", "impl", 17, 21),
            d("len", "method", 18, 20),
        ];
        let out = digest("src/lib.rs", RS, &defs, None);
        assert!(out.contains("use std::io;"), "{out}");
        assert!(out.contains("/// Parses one record."), "{out}");
        assert!(out.contains("#[inline]"), "{out}");
        assert!(
            out.contains("pub fn parse(input: &str) -> Result<Record, Error> { … }"),
            "{out}"
        );
        assert!(out.contains("pub n: usize,"), "{out}");
        assert!(out.contains("pub fn len(&self) -> usize { … }"), "{out}");
        assert!(!out.contains("secret_body_marker"), "{out}");
        assert!(!out.contains("41"), "{out}");
        assert!(out.len() < RS.len());
    }

    #[test]
    fn python_digest_keeps_docstring_not_body() {
        let py = "import os\nfrom a import b\n\ndef run(x: int) -> int:\n    \"\"\"Run it.\"\"\"\n    hidden = x * 2\n    return hidden\n";
        let out = digest("m.py", py, &[d("run", "function", 4, 7)], None);
        assert!(
            out.contains("import os") && out.contains("from a import b"),
            "{out}"
        );
        assert!(out.contains("def run(x: int) -> int:"), "{out}");
        assert!(out.contains("\"\"\"Run it.\"\"\""), "{out}");
        assert!(!out.contains("hidden"), "{out}");
    }

    #[test]
    fn focus_keeps_only_named_functions_and_unknown_files_keep_imports() {
        let defs = vec![d("parse", "function", 7, 10), d("len", "method", 18, 20)];
        let focus: BTreeSet<String> = ["len".to_string()].into();
        let out = digest("src/lib.rs", RS, &defs, Some(&focus));
        assert!(out.contains("fn len") && !out.contains("fn parse"), "{out}");
        let out = digest("x.rs", "use a::b;\nlet x = 1;\n", &[], None);
        assert_eq!(out, "use a::b;\n");
    }
}
