// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Import statements and module paths for `rtok review`'s usage search.
//!
//! The graph index links references to definitions by bare name. This module adds the
//! evidence that tells same-name definitions apart: which module a file imports a name
//! from (Rust `use`, Python `from … import`, JS/TS `import … from`), under which alias, and
//! which module path a definition file has. Text-level parsing, one statement at a time;
//! good enough to rank evidence, not a compiler.

/// One imported item: its full path segments (module then item) and the local name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    pub path: Vec<String>,
    pub alias: String,
}

impl Import {
    /// The imported item's own name (the last path segment).
    pub fn item(&self) -> &str {
        self.path.last().map(String::as_str).unwrap_or("")
    }

    /// Module segments with relative markers (`crate`, `self`, `super`, dots) removed.
    pub fn module(&self) -> Vec<&str> {
        let n = self.path.len().saturating_sub(1);
        self.path[..n]
            .iter()
            .map(String::as_str)
            .filter(|s| !matches!(*s, "crate" | "self" | "super" | "" | "."))
            .collect()
    }
}

fn ext(path: &str) -> &str {
    path.rsplit_once('.').map(|(_, e)| e).unwrap_or("")
}

/// Join a Rust `use` statement that spans lines, then expand its braces.
fn rust_uses(text: &str) -> Vec<Import> {
    let mut out = Vec::new();
    let mut buf = String::new();
    for line in text.lines() {
        let t = line.trim();
        if buf.is_empty() {
            let body = t
                .strip_prefix("pub use ")
                .or_else(|| t.strip_prefix("pub(crate) use "))
                .or_else(|| t.strip_prefix("use "));
            let Some(body) = body else { continue };
            buf.push_str(body);
        } else {
            buf.push(' ');
            buf.push_str(t);
        }
        if buf.contains(';') {
            let stmt = buf.split(';').next().unwrap_or("").to_string();
            expand_rust(&[], &stmt, &mut out);
            buf.clear();
        }
    }
    out
}

/// Split at top-level commas.
fn split_top(s: &str) -> Vec<&str> {
    let (mut depth, mut start, mut parts) = (0i32, 0usize, Vec::new());
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

fn expand_rust(prefix: &[String], tree: &str, out: &mut Vec<Import>) {
    let tree = tree.trim();
    if tree.is_empty() {
        return;
    }
    if let Some(open) = tree.find('{') {
        let head: Vec<String> = tree[..open]
            .split("::")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let close = tree.rfind('}').unwrap_or(tree.len());
        let mut p = prefix.to_vec();
        p.extend(head);
        for part in split_top(&tree[open + 1..close]) {
            expand_rust(&p, part, out);
        }
        return;
    }
    let (path, alias) = match tree.split_once(" as ") {
        Some((p, a)) => (p.trim(), Some(a.trim().to_string())),
        None => (tree, None),
    };
    let mut segs = prefix.to_vec();
    segs.extend(
        path.split("::")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    );
    if segs.last().is_some_and(|s| s == "self") {
        segs.pop();
    }
    let Some(last) = segs.last().cloned() else {
        return;
    };
    if last == "*" {
        return;
    }
    out.push(Import {
        alias: alias.unwrap_or(last),
        path: segs,
    });
}

fn python_imports(text: &str) -> Vec<Import> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("from ")
            && let Some((module, names)) = rest.split_once(" import ")
        {
            let module: Vec<String> = module
                .trim()
                .trim_start_matches('.')
                .split('.')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            for n in names
                .trim_matches(|c| c == '(' || c == ')' || c == ' ')
                .split(',')
            {
                let n = n.trim();
                let (item, alias) = match n.split_once(" as ") {
                    Some((i, a)) => (i.trim(), a.trim()),
                    None => (n, n),
                };
                if item.is_empty() || item == "*" {
                    continue;
                }
                let mut path = module.clone();
                path.push(item.to_string());
                out.push(Import {
                    path,
                    alias: alias.to_string(),
                });
            }
        }
    }
    out
}

fn js_imports(text: &str) -> Vec<Import> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if !t.starts_with("import ") && !t.starts_with("export ") {
            continue;
        }
        let (Some(open), Some(close)) = (t.find('{'), t.find('}')) else {
            continue;
        };
        let Some(from) = t.rsplit_once(" from ").map(|(_, f)| f) else {
            continue;
        };
        let spec = from
            .trim()
            .trim_end_matches(';')
            .trim_matches(|c| c == '"' || c == '\'');
        let module: Vec<String> = spec
            .split('/')
            .filter(|s| !s.is_empty() && *s != "." && *s != "..")
            .map(str::to_string)
            .collect();
        for n in t[open + 1..close].split(',') {
            let n = n.trim().trim_start_matches("type ").trim();
            let (item, alias) = match n.split_once(" as ") {
                Some((i, a)) => (i.trim(), a.trim()),
                None => (n, n),
            };
            if item.is_empty() {
                continue;
            }
            let mut path = module.clone();
            path.push(item.to_string());
            out.push(Import {
                path,
                alias: alias.to_string(),
            });
        }
    }
    out
}

/// Imports of one file, by language (extension). Unknown languages have none.
pub fn parse(path: &str, text: &str) -> Vec<Import> {
    match ext(path) {
        "rs" => rust_uses(text),
        "py" => python_imports(text),
        "ts" | "tsx" | "js" | "jsx" | "mjs" => js_imports(text),
        _ => Vec::new(),
    }
}

/// Module path of a definition file, outermost first: `crates/rtok-store/src/symbols.rs` →
/// `[rtok_store, symbols]`, `src/plugins/graph/mod.rs` → `[plugins, graph]`,
/// `pkg/a/b.py` → `[pkg, a, b]`, `web/src/api/x.ts` → `[web, src, api, x]`.
pub fn module_path(path: &str) -> Vec<String> {
    let stem = path.rsplit_once('.').map(|(s, _)| s).unwrap_or(path);
    let mut segs: Vec<&str> = stem.split('/').filter(|s| !s.is_empty()).collect();
    let mut out: Vec<String> = Vec::new();
    if ext(path) == "rs" {
        let mut in_src = false;
        if let Some(i) = segs.iter().rposition(|s| *s == "src") {
            if i > 0 {
                out.push(segs[i - 1].replace('-', "_"));
            }
            segs.drain(..=i);
            in_src = true;
        }
        // `mod.rs` is its directory; `lib.rs` / `main.rs` are the crate root only under `src`.
        if segs
            .last()
            .is_some_and(|s| *s == "mod" || in_src && matches!(*s, "lib" | "main"))
        {
            segs.pop();
        }
    } else if segs
        .last()
        .is_some_and(|s| matches!(*s, "__init__" | "index"))
    {
        segs.pop();
    }
    out.extend(segs.iter().map(|s| s.to_string()));
    out
}

/// Whether `imp`'s module points at the definition file `def_path`: its module segments
/// (a crate name included) end the file's module path or one of its parents. An import whose module
/// is only relative markers (`use super::X`) matches nothing here; the directory rule
/// covers it.
pub fn points_at(imp: &Import, def_path: &str) -> bool {
    let module = imp.module();
    if module.is_empty() {
        return false;
    }
    let target = module_path(def_path);
    let target: Vec<&str> = target.iter().map(String::as_str).collect();
    // The module itself, or a parent that re-exports it (`crate::store::Value` with `Value`
    // in `store/value.rs`); a crate name alone stands for the whole crate.
    (1..=target.len()).any(|k| target[..k].ends_with(&module))
}

/// Whether `text` names `name` through the module of `def_path` inline
/// (`store::value::Value`, `value::Value`), without a `use`. The whole path around each
/// occurrence must point at `def_path`, so `crate::web::value::Value` does not count for
/// `store/value.rs`.
pub fn qualified_inline(text: &str, def_path: &str, name: &str) -> bool {
    let Some(m) = module_path(def_path).last().cloned() else {
        return false;
    };
    let needle = format!("{m}::{name}");
    let is_path = |c: char| c.is_alphanumeric() || c == '_' || c == ':';
    let mut from = 0;
    while let Some(i) = text[from..].find(&needle) {
        let at = from + i;
        let end = at + needle.len();
        from = end;
        if text[end..].starts_with(|c: char| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let start = text[..at]
            .char_indices()
            .rev()
            .find(|(_, c)| !is_path(*c))
            .map(|(j, c)| j + c.len_utf8())
            .unwrap_or(0);
        let imp = Import {
            path: text[start..end].split("::").map(str::to_string).collect(),
            alias: name.to_string(),
        };
        if points_at(&imp, def_path) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(v: &[Import]) -> Vec<(String, String)> {
        v.iter()
            .map(|i| (i.path.join("::"), i.alias.clone()))
            .collect()
    }

    #[test]
    fn rust_use_trees_aliases_and_multiline() {
        let src = "use std::io;\nuse crate::store::{Store, value::Value as V,\n    self};\npub use super::x::*;\nfn f() {}\n";
        let got = paths(&parse("a.rs", src));
        assert_eq!(
            got,
            [
                ("std::io".into(), "io".into()),
                ("crate::store::Store".into(), "Store".into()),
                ("crate::store::value::Value".into(), "V".into()),
                ("crate::store".into(), "store".into()),
            ]
        );
    }

    #[test]
    fn python_and_js_imports() {
        let py = parse("m.py", "from pkg.a import b, c as d\nfrom . import e\n");
        assert_eq!(
            paths(&py),
            [
                ("pkg::a::b".into(), "b".into()),
                ("pkg::a::c".into(), "d".into()),
                ("e".into(), "e".into())
            ]
        );
        let js = parse("m.ts", "import { a, b as c } from './lib/x';\n");
        assert_eq!(
            paths(&js),
            [
                ("lib::x::a".into(), "a".into()),
                ("lib::x::b".into(), "c".into())
            ]
        );
    }

    #[test]
    fn module_paths_and_pointing() {
        assert_eq!(
            module_path("crates/rtok-store/src/symbols.rs"),
            ["rtok_store", "symbols"]
        );
        assert_eq!(
            module_path("src/plugins/graph/mod.rs"),
            ["plugins", "graph"]
        );
        assert_eq!(module_path("pkg/a/__init__.py"), ["pkg", "a"]);
        let imp = |p: &str| Import {
            path: p.split("::").map(str::to_string).collect(),
            alias: String::new(),
        };
        assert!(points_at(&imp("crate::store::Value"), "src/store/mod.rs"));
        assert!(points_at(&imp("crate::store::Value"), "src/store/value.rs"));
        assert!(!points_at(&imp("crate::web::Value"), "src/store/value.rs"));
        assert!(points_at(
            &imp("rtok_store::symbols::X"),
            "crates/rtok-store/src/symbols.rs"
        ));
        assert!(!points_at(&imp("super::Value"), "src/store/value.rs"));
        assert!(qualified_inline(
            "let v = value::Value::new();",
            "src/store/value.rs",
            "Value"
        ));
    }
}
