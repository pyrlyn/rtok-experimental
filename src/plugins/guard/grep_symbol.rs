// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T369: answer a symbol-shaped native `Grep` from the symbol index.
//!
//! `Grep pattern="fn parse_since"` is a question `symbol` already answers, so the deny carries
//! the answer instead of a pointer to a second call. Index lookup only: no walk and no
//! indexing (`index_for` would break the hook's 10 ms budget). Every other outcome (flag off,
//! a regex, a scoped search, no or too many definitions, an index that no longer matches the
//! file) returns `None` and the call proceeds as before.

use std::path::{Path, PathBuf};

use rtok_plugin_sdk::{Class, Ctx, Measurement, PreToolDecision, PreToolUse};
use serde_json::Value;

use crate::plugins::graph::{self, index};

/// Above this many definitions the name is a common word, not a symbol worth answering.
const MAX_DEFS: usize = 5;

/// Words that may precede the identifier in a definition-shaped pattern.
const DEF_WORDS: [&str; 7] = ["fn", "def", "class", "struct", "type", "func", "interface"];

/// In every answer's head line; `rtok stats` finds the answers in transcripts by it (T369.1).
pub(crate) const ANSWERED: &str = "answered from the rtok index, Grep not run";

pub(super) fn answer(ev: &PreToolUse, cx: &Ctx) -> Option<PreToolDecision> {
    if !cx
        .plugin_config::<crate::config::Guard>("guard")
        .grep_symbol
        || !cx.plugin_config::<crate::config::Graph>("graph").enabled
    {
        return None;
    }
    let name = symbol_name(ev.tool_input)?;
    let cwd = Path::new(cx.cwd()?);
    let scope = ev.tool_input.get("path").and_then(Value::as_str);
    let reason = reason(cx, cwd, scope, name)?;
    // Countable but claims no saving: the Grep output it replaces was never produced (D3).
    let _ = cx.record(&Measurement {
        plugin: "guard",
        kind: "grep_symbol",
        before_bytes: 0,
        after_bytes: 0,
        est_before: 0,
        est_after: 0,
        ref_id: None,
        call_id: None,
    });
    Some(PreToolDecision::Deny { reason })
}

/// The identifier a `Grep` input looks for, when that is all it asks for: a bare identifier
/// or `<fn|def|class|struct|type|func|interface> <ident>`, with no option that narrows or
/// changes the match (`glob`, `type`, `-i`, `multiline`).
pub(crate) fn symbol_name(input: &Value) -> Option<&str> {
    let set = |k: &str| match input.get(k) {
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Bool(b)) => *b,
        _ => false,
    };
    if ["glob", "type", "-i", "multiline"].into_iter().any(set) {
        return None;
    }
    let pattern = input.get("pattern")?.as_str()?.trim();
    let rest = match pattern.split_once(char::is_whitespace) {
        Some((word, rest)) if DEF_WORDS.contains(&word) => rest.trim_start(),
        _ => pattern,
    };
    // `\bfoo\b`, `\bfoo\(`, `foo\(` and `\bfoo` match the same word a bare `foo` does; any
    // other metacharacter is a real regex and stays with Grep.
    let rest = rest.strip_prefix("\\b").unwrap_or(rest);
    let name = ["\\b", "\\("]
        .into_iter()
        .find_map(|tail| rest.strip_suffix(tail))
        .unwrap_or(rest);
    is_identifier(name).then_some(name)
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    s.len() >= 2
        && s.len() <= 64
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The text the deny carries, or `None` to let the Grep run.
fn reason(cx: &Ctx, cwd: &Path, scope: Option<&str>, name: &str) -> Option<String> {
    let (key, rows) = definitions(cx, cwd, name)?;
    let root = Path::new(&key);
    // A `path` that is a directory inside the project keeps only what lies under it; a file,
    // a missing path or one outside the project asks something the index cannot answer.
    let under = match scope.map(str::trim).filter(|p| !p.is_empty()) {
        None => String::new(),
        Some(p) => {
            let abs = fs_kit::path::canonicalize(cwd.join(p)).ok()?;
            let rel = abs.strip_prefix(root).ok()?;
            if !abs.is_dir() {
                return None;
            }
            let rel = rel.to_string_lossy().replace('\\', "/");
            if rel.is_empty() {
                rel
            } else {
                format!("{rel}/")
            }
        }
    };
    let rows: Defs = rows
        .into_iter()
        .filter(|(path, ..)| path.starts_with(&under))
        .collect();
    if rows.is_empty() || rows.len() > MAX_DEFS {
        return None;
    }
    let budget = cx.plugin_config::<crate::config::Graph>("graph").body_lines as usize;
    let mut defs = String::new();
    let mut src: Option<(&str, String)> = None;
    for (path, kind, line, end_line) in &rows {
        if src.as_ref().is_none_or(|(p, _)| p != path) {
            src = Some((path, std::fs::read_to_string(root.join(path)).ok()?));
        }
        let text = &src.as_ref()?.1;
        // The index lags the files between watcher runs; a row that no longer names the
        // symbol on its line must not turn into a confident wrong answer.
        let at = usize::try_from(*line).ok()?.checked_sub(1)?;
        if !text.lines().nth(at)?.contains(name) {
            return None;
        }
        defs.push_str(&graph::def_text(
            text,
            name,
            (path, kind, *line, *end_line),
            budget,
            &mut |bytes| cx.put_archive(bytes).ok(),
        ));
    }
    let refs: i64 = cx
        .symbol_ref_groups(&key, name)
        .ok()?
        .iter()
        .filter(|g| g.0.starts_with(&under))
        .map(|g| g.2)
        .sum();
    let head = format!(
        "Grep `{name}` {ANSWERED}: {} def, {refs} refs (`callers`)\n",
        rows.len()
    );
    Some(fit(cx, &(head + &defs)))
}

type Defs = Vec<(String, String, i32, i32)>;

/// The definitions of `name` under the session's project: the index key of `cwd` itself, else
/// of its checkout root (the index is keyed by the root it was built for, a hook's cwd may be a
/// subdirectory of it). Returns that key with the rows.
fn definitions(cx: &Ctx, cwd: &Path, name: &str) -> Option<(String, Defs)> {
    let mut roots: Vec<PathBuf> = vec![cwd.to_path_buf()];
    // `checkout`, not `main`: the index is built per checkout, and a worktree's files differ.
    roots.extend(rtok_config::root::ProjectRoot::find(cwd).map(|r| r.checkout));
    for root in roots {
        let key = index::canon(&root);
        let rows = cx.symbol_defs(&key, name).ok()?;
        if !rows.is_empty() {
            return Some((key, rows));
        }
    }
    None
}

/// `text` within `plugins.inject.budget_tokens`: whole lines dropped from the end, never the
/// head and the first definition line.
fn fit(cx: &Ctx, text: &str) -> String {
    let budget = cx
        .plugin_config::<crate::config::Inject>("inject")
        .budget_tokens;
    let mut lines: Vec<&str> = text.lines().collect();
    let mut out = text.to_string();
    while lines.len() > 2 && cx.estimate(&out, Class::Code) > budget {
        lines.pop();
        out = lines.join("\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::guard::GuardPlugin;
    use rstest::rstest;
    use rtok_plugin_sdk::Plugin;
    use serde_json::json;
    use std::fs;

    #[rstest]
    #[case(json!({"pattern": "parse_since"}), Some("parse_since"))]
    #[case(json!({"pattern": " parse_since "}), Some("parse_since"))]
    #[case(json!({"pattern": "fn parse_since"}), Some("parse_since"))]
    #[case(json!({"pattern": "class  Foo"}), Some("Foo"))]
    #[case(json!({"pattern": "interface IFoo", "output_mode": "content"}), Some("IFoo"))]
    #[case(json!({"pattern": "func Run", "path": "."}), Some("Run"))]
    #[case(json!({"pattern": "TODO|FIXME"}), None)]
    #[case(json!({"pattern": "\\bfoo\\b"}), Some("foo"))]
    #[case(json!({"pattern": "\\bfoo\\("}), Some("foo"))]
    #[case(json!({"pattern": "foo\\("}), Some("foo"))]
    #[case(json!({"pattern": "\\bfoo"}), Some("foo"))]
    #[case(json!({"pattern": "fn foo\\("}), Some("foo"))]
    #[case(json!({"pattern": "foo\\b"}), Some("foo"))]
    #[case(json!({"pattern": "\\bfoo|bar\\b"}), None)]
    #[case(json!({"pattern": "\\b\\bfoo"}), None)]
    #[case(json!({"pattern": "^foo$"}), None)]
    #[case(json!({"pattern": "foo.*"}), None)]
    #[case(json!({"pattern": "\\bfoo.*\\b"}), None)]
    #[case(json!({"pattern": "[fF]oo"}), None)]
    #[case(json!({"pattern": "\\b\\b"}), None)]
    #[case(json!({"pattern": "fn parse_since("}), None)]
    #[case(json!({"pattern": "foo.bar"}), None)]
    #[case(json!({"pattern": "impl Foo"}), None)]
    #[case(json!({"pattern": "fn foo bar"}), None)]
    #[case(json!({"pattern": "x"}), None)]
    #[case(json!({"pattern": "9lives"}), None)]
    #[case(json!({"pattern": ""}), None)]
    #[case(json!({"pattern": "foo", "glob": "*.rs"}), None)]
    #[case(json!({"pattern": "foo", "type": "rust"}), None)]
    #[case(json!({"pattern": "foo", "-i": true}), None)]
    #[case(json!({"pattern": "foo", "-i": false}), Some("foo"))]
    #[case(json!({"pattern": "foo", "multiline": true}), None)]
    #[case(json!({}), None)]
    fn classifier(#[case] input: Value, #[case] want: Option<&str>) {
        assert_eq!(symbol_name(&input), want, "{input}");
    }

    /// An indexed project in a temp dir, `grep_symbol` set as asked.
    fn project(tag: &str, on: bool, files: &[(&str, &str)]) -> (crate::plugin::Runtime, PathBuf) {
        let (mut c, dir) = crate::testutil::config(tag);
        c.plugins.guard.grep_symbol = on;
        let mut rt = crate::plugin::Runtime::open(c, tag).unwrap();
        let root = dir.join("proj");
        fs::create_dir_all(&root).unwrap();
        for (name, body) in files {
            fs::write(root.join(name), body).unwrap();
        }
        graph::index::run(&Ctx::new(&rt), &root, false).unwrap();
        rt.cwd = Some(root.to_string_lossy().into_owned());
        (rt, root)
    }

    fn grep(rt: &crate::plugin::Runtime, input: Value) -> Option<String> {
        let ev = PreToolUse {
            tool_name: "Grep",
            tool_input: &input,
        };
        match GuardPlugin.pre_tool(&ev, &Ctx::new(rt)) {
            Some(PreToolDecision::Deny { reason }) => Some(reason),
            _ => None,
        }
    }

    const LIB: &str = "pub fn parse_since(s: &str) -> u32 {\n    s.len() as u32\n}\n\npub fn caller() -> u32 {\n    parse_since(\"1d\") + parse_since(\"2d\")\n}\n";

    #[test]
    fn answers_one_definition_with_source_and_reference_count() {
        let (rt, root) = project("t369-one", true, &[("lib.rs", LIB)]);
        for pattern in ["fn parse_since", "parse_since"] {
            let reason = grep(&rt, json!({"pattern": pattern})).expect(pattern);
            assert!(reason.contains("lib.rs:1 function"), "{reason}");
            assert!(
                reason.contains("pub fn parse_since(s: &str) -> u32 {"),
                "{reason}"
            );
            assert!(reason.contains("2 refs"), "{reason}");
        }
        // The same project-root scope, spelled out, is not a narrowing.
        let at_root = json!({"pattern": "parse_since", "path": root.to_str().unwrap()});
        assert!(grep(&rt, at_root).is_some());
        let rows = rt.store.list_measurements("guard").unwrap();
        assert!(
            rows.iter()
                .filter(|r| r.kind == "grep_symbol")
                .all(|r| r.before_bytes == 0 && r.after_bytes == 0),
            "{rows:?}"
        );
        assert_eq!(rows.iter().filter(|r| r.kind == "grep_symbol").count(), 3);
    }

    #[test]
    fn falls_through_when_it_cannot_answer() {
        let many = "fn dup() {}\n".repeat(6);
        let (rt, root) = project("t369-skip", true, &[("lib.rs", LIB), ("d.rs", &many)]);
        let skipped = [
            json!({"pattern": "TODO|FIXME"}),
            json!({"pattern": "never_defined"}),
            json!({"pattern": "dup"}),
            json!({"pattern": "parse_since", "glob": "*.rs"}),
            json!({"pattern": "parse_since", "path": "src"}),
            json!({"pattern": "parse_since", "path": root.join("lib.rs").to_str().unwrap()}),
        ];
        for input in skipped {
            assert_eq!(grep(&rt, input.clone()), None, "{input}");
        }
        assert_eq!(rt.store.measurement_count("guard").unwrap(), 0);
    }

    #[test]
    fn flag_off_by_default_and_a_stale_index_fall_open() {
        let (rt, _) = project("t369-off", false, &[("lib.rs", LIB)]);
        assert_eq!(grep(&rt, json!({"pattern": "parse_since"})), None);

        let (rt, root) = project("t369-stale", true, &[("lib.rs", LIB)]);
        assert!(grep(&rt, json!({"pattern": "parse_since"})).is_some());
        // The file changed after indexing: the row's line no longer names the symbol.
        fs::write(root.join("lib.rs"), "// moved\n".repeat(4)).unwrap();
        assert_eq!(grep(&rt, json!({"pattern": "parse_since"})), None);
        fs::remove_file(root.join("lib.rs")).unwrap();
        assert_eq!(grep(&rt, json!({"pattern": "parse_since"})), None);
    }

    #[test]
    fn a_directory_path_keeps_only_the_definitions_under_it() {
        let (rt, root) = project("t369-dir", true, &[("lib.rs", LIB)]);
        let sub = root.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("m.rs"), "pub fn parse_since() {}\n").unwrap();
        fs::create_dir_all(root.join("other")).unwrap();
        graph::index::run(&Ctx::new(&rt), &root, false).unwrap();
        let reason = |path: &str| grep(&rt, json!({"pattern": "parse_since", "path": path}));
        // Two definitions project-wide, one under `sub`: relative and absolute spellings agree.
        let wide = grep(&rt, json!({"pattern": "parse_since"})).unwrap();
        assert!(
            wide.contains("lib.rs:1") && wide.contains("sub/m.rs:1"),
            "{wide}"
        );
        for spelling in ["sub", "sub/", "./sub", sub.to_str().unwrap()] {
            let r = reason(spelling).expect(spelling);
            assert!(r.contains("sub/m.rs:1") && !r.contains("lib.rs:1"), "{r}");
        }
        // Nothing under the directory, outside the project, or a file: the Grep runs.
        assert_eq!(reason("other"), None);
        assert_eq!(reason("../outside"), None);
        assert_eq!(reason("/"), None);
        assert_eq!(reason("sub/m.rs"), None);
        assert_eq!(reason("missing"), None);
    }

    #[test]
    fn word_boundary_and_call_spellings_are_answered() {
        let (rt, _) = project("t369-spell", true, &[("lib.rs", LIB)]);
        for pattern in [
            "\\bparse_since\\b",
            "\\bparse_since\\(",
            "parse_since\\(",
            "\\bparse_since",
        ] {
            let r = grep(&rt, json!({"pattern": pattern})).expect(pattern);
            assert!(r.contains("lib.rs:1 function"), "{pattern}: {r}");
        }
        assert_eq!(
            grep(&rt, json!({"pattern": "\\bparse_since|caller\\b"})),
            None
        );
    }

    #[test]
    fn answer_is_capped_at_the_inject_budget() {
        let body = format!("fn big() {{\n{}}}\n", "    let value = 1 + 1;\n".repeat(30));
        let (mut c, dir) = crate::testutil::config("t369-cap");
        c.plugins.guard.grep_symbol = true;
        c.plugins.inject.budget_tokens = 60;
        let mut rt = crate::plugin::Runtime::open(c, "t369-cap").unwrap();
        let root = dir.join("proj");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("b.rs"), body).unwrap();
        graph::index::run(&Ctx::new(&rt), &root, false).unwrap();
        rt.cwd = Some(root.to_string_lossy().into_owned());
        let reason = grep(&rt, json!({"pattern": "big"})).unwrap();
        assert!(
            Ctx::new(&rt).estimate(&reason, Class::Code) <= 60,
            "{reason}"
        );
        assert!(reason.contains("b.rs:1 function"), "{reason}");
    }

    #[test]
    fn subdirectory_cwd_finds_the_git_root_index() {
        let (mut rt, root) = project("t369-sub", true, &[("lib.rs", LIB)]);
        fs::create_dir_all(root.join(".git")).unwrap();
        let sub = root.join("deep");
        fs::create_dir_all(&sub).unwrap();
        rt.cwd = Some(sub.to_string_lossy().into_owned());
        assert!(grep(&rt, json!({"pattern": "parse_since"})).is_some());
    }

    /// T660.2: from inside a linked worktree the index of that worktree is found, not the main one's.
    #[test]
    fn linked_worktree_subdirectory_finds_its_own_index() {
        let (mut rt, root) = project("t660-sub", true, &[("lib.rs", LIB)]);
        let admin = root.parent().unwrap().join("main/.git/worktrees/proj");
        fs::create_dir_all(&admin).unwrap();
        fs::write(admin.join("commondir"), "../..\n").unwrap();
        fs::write(root.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
        let sub = root.join("deep");
        fs::create_dir_all(&sub).unwrap();
        rt.cwd = Some(sub.to_string_lossy().into_owned());
        assert!(grep(&rt, json!({"pattern": "parse_since"})).is_some());
    }
}
