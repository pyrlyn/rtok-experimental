// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Fixture-repo tests for `rtok review`: usage search on the real index and the prompt
//! that comes out of a diff.

use super::*;
use crate::plugin::Runtime;
use std::fs;
use std::path::PathBuf;

const LIB: &str = "use crate::util::helper;

/// Parses a config line.
pub fn parse(line: &str) -> usize {
    let lib_body_marker = helper(line);
    lib_body_marker + 1
}
";
const UTIL: &str = "/// Counts bytes.
pub fn helper(s: &str) -> usize {
    let util_body_marker = s.len();
    util_body_marker
}
";
const USER: &str = "use crate::lib::parse;

/// Loads everything.
pub fn load() -> usize {
    let user_body_marker = parse(\"a\");
    user_body_marker
}
";
const LONELY: &str = "pub fn unrelated() -> u8 {\n    7\n}\n";

fn fixture(tag: &str) -> (Runtime, PathBuf) {
    let (cx, dir) = crate::plugins::graph::index::tests::cx(tag);
    for (p, s) in [
        ("lib.rs", LIB),
        ("util.rs", UTIL),
        ("user.rs", USER),
        ("lonely.rs", LONELY),
    ] {
        fs::write(dir.join(p), s).unwrap();
    }
    (cx, dir)
}

const DIFF: &str = "diff --git a/lib.rs b/lib.rs
--- a/lib.rs
+++ b/lib.rs
@@ -5,3 +5,3 @@ pub fn parse(line: &str) -> usize {
     let lib_body_marker = helper(line);
-    lib_body_marker + 1
+    lib_body_marker + 2
 }
";

#[test]
fn usage_search_finds_callers_and_callees_on_the_index() {
    let (rt, dir) = fixture("review-usage");
    let cx = Ctx::new(&rt);
    let b = build(&cx, &dir, DIFF, &Options::default()).unwrap();
    assert!(b.changed["lib.rs"].contains("parse"), "{:?}", b.changed);
    let rel: Vec<(&str, usage::Relation)> = b
        .related
        .iter()
        .map(|r| (r.path.as_str(), r.relation))
        .collect();
    assert!(
        rel.contains(&("user.rs", usage::Relation::Dependent)),
        "{rel:?}"
    );
    assert!(
        rel.contains(&("util.rs", usage::Relation::Dependency)),
        "{rel:?}"
    );
    assert!(
        !rel.iter().any(|r| r.0 == "lonely.rs" || r.0 == "lib.rs"),
        "{rel:?}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn prompt_carries_diff_and_digests_without_bodies() {
    let (rt, dir) = fixture("review-prompt");
    let cx = Ctx::new(&rt);
    let b = build(&cx, &dir, DIFF, &Options::default()).unwrap();
    let t = &b.prompt.text;
    assert!(t.contains("+    lib_body_marker + 2"), "{t}");
    assert!(t.contains("### user.rs (dependent)"), "{t}");
    assert!(t.contains("pub fn load() -> usize { … }"), "{t}");
    assert!(t.contains("pub fn helper(s: &str) -> usize { … }"), "{t}");
    assert!(t.contains("/// Counts bytes."), "{t}");
    assert!(
        !t.contains("user_body_marker") && !t.contains("util_body_marker"),
        "{t}"
    );
    assert!(!t.contains("lonely.rs"), "{t}");
    // Same input, same prompt.
    assert_eq!(
        build(&cx, &dir, DIFF, &Options::default())
            .unwrap()
            .prompt
            .text,
        *t
    );
    // The full-file baseline holds the bodies and costs more.
    let full = build(
        &cx,
        &dir,
        DIFF,
        &Options {
            mode: ContextMode::Full,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(full.prompt.text.contains("user_body_marker"));
    assert!(full.prompt.tokens > b.prompt.tokens);
    let none = build(
        &cx,
        &dir,
        DIFF,
        &Options {
            mode: ContextMode::None,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(none.prompt.included.is_empty() && none.prompt.tokens < b.prompt.tokens);
    let js = report(&b, None);
    assert_eq!(js["files"][0]["path"], "lib.rs");
    assert!(js["prompt"]["tokens"].as_u64().unwrap() > 0);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn budget_limits_related_context() {
    let (rt, dir) = fixture("review-budget");
    let cx = Ctx::new(&rt);
    let none = build(
        &cx,
        &dir,
        DIFF,
        &Options {
            mode: ContextMode::None,
            ..Options::default()
        },
    )
    .unwrap();
    let b = build(
        &cx,
        &dir,
        DIFF,
        &Options {
            budget: Some(none.prompt.diff_tokens + 5),
            ..Options::default()
        },
    )
    .unwrap();
    assert!(b.prompt.included.is_empty(), "{:?}", b.prompt.included);
    assert!(!b.prompt.dropped.is_empty());
    let _ = fs::remove_dir_all(dir);
}

struct Fake;
impl provider::Reviewer for Fake {
    fn name(&self) -> String {
        "fake".into()
    }
    fn review(&self, prompt: &str) -> anyhow::Result<String> {
        Ok(format!("saw {} bytes", prompt.len()))
    }
}

#[test]
fn a_pluggable_reviewer_gets_the_prompt() {
    use provider::Reviewer;
    let (rt, dir) = fixture("review-fake");
    let cx = Ctx::new(&rt);
    let b = build(&cx, &dir, DIFF, &Options::default()).unwrap();
    let out = Fake.review(&b.prompt.text).unwrap();
    assert_eq!(out, format!("saw {} bytes", b.prompt.text.len()));
    let js = report(&b, Some(("fake", &out)));
    assert_eq!(js["reviewer"], "fake");
    let _ = fs::remove_dir_all(dir);
}
