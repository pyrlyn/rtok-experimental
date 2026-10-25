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

// ---------- usage-search noise filters ----------

/// Same-name `Value` in several modules; who uses which is only told apart by imports.
fn noisy(tag: &str, extra: &[(&str, &str)]) -> (Runtime, PathBuf) {
    let (cx, dir) = crate::plugins::graph::index::tests::cx(tag);
    let mut files: Vec<(&str, &str)> = vec![
        (
            "src/store/value.rs",
            "/// Stored value.\npub struct Value {\n    pub n: u32,\n}\n\npub fn store_it() -> u32 {\n    let store_body = 1;\n    store_body\n}\n",
        ),
        (
            "src/web/value.rs",
            "pub struct Value {\n    pub s: String,\n}\n",
        ),
        (
            "src/user_store.rs",
            "use crate::store::value::Value;\n\npub fn keep(v: Value) -> u32 {\n    v.n\n}\n",
        ),
        (
            "src/user_web.rs",
            "use crate::web::value::Value;\n\npub fn show(v: Value) -> String {\n    v.s\n}\n",
        ),
        (
            "src/inline.rs",
            "pub fn make() -> store::value::Value {\n    todo!()\n}\n",
        ),
        (
            "src/aliased.rs",
            "use crate::store::value::Value as StoreValue;\n\npub fn wrap(v: StoreValue) -> u32 {\n    v.n\n}\n",
        ),
    ];
    files.extend_from_slice(extra);
    for (p, s) in files {
        fs::create_dir_all(dir.join(p).parent().unwrap()).unwrap();
        fs::write(dir.join(p), s).unwrap();
    }
    (cx, dir)
}

const VALUE_DIFF: &str = "diff --git a/src/store/value.rs b/src/store/value.rs
--- a/src/store/value.rs
+++ b/src/store/value.rs
@@ -1,4 +1,4 @@
 /// Stored value.
 pub struct Value {
-    pub n: u64,
+    pub n: u32,
 }
";

fn paths(b: &Built) -> Vec<&str> {
    b.related.iter().map(|r| r.path.as_str()).collect()
}

#[test]
fn import_filter_keeps_the_importer_of_this_module_only() {
    let (rt, dir) = noisy("review-imports", &[]);
    let b = build(&Ctx::new(&rt), &dir, VALUE_DIFF, &Options::default()).unwrap();
    let p = paths(&b);
    assert!(p.contains(&"src/user_store.rs"), "{p:?}");
    assert!(
        !p.contains(&"src/user_web.rs"),
        "{:?} {:?}",
        b.related,
        b.changed
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn qualified_paths_and_use_as_aliases_count() {
    let (rt, dir) = noisy("review-qualified", &[]);
    let b = build(&Ctx::new(&rt), &dir, VALUE_DIFF, &Options::default()).unwrap();
    let p = paths(&b);
    assert!(p.contains(&"src/inline.rs"), "{p:?}");
    assert!(p.contains(&"src/aliased.rs"), "{p:?}");
    let a = b
        .related
        .iter()
        .find(|r| r.path == "src/aliased.rs")
        .unwrap();
    assert!(a.focus.contains("wrap"), "{a:?}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_call_does_not_link_to_a_same_name_trait() {
    let extra = [
        ("src/a/h.rs", "pub fn helper_x() -> u32 {\n    1\n}\n"),
        ("src/b/h.rs", "pub trait helper_x {}\n"),
        (
            "src/store/caller.rs",
            "pub fn caller() -> u32 {\n    let c = helper_x();\n    c + 1\n}\n",
        ),
    ];
    let (rt, dir) = noisy("review-kinds", &extra);
    let diff = "diff --git a/src/store/caller.rs b/src/store/caller.rs
--- a/src/store/caller.rs
+++ b/src/store/caller.rs
@@ -1,4 +1,4 @@
 pub fn caller() -> u32 {
     let c = helper_x();
-    c
+    c + 1
 }
";
    let b = build(&Ctx::new(&rt), &dir, diff, &Options::default()).unwrap();
    let p = paths(&b);
    assert!(p.contains(&"src/a/h.rs"), "{p:?}");
    assert!(!p.contains(&"src/b/h.rs"), "{p:?}");
    assert!(
        !p.contains(&"src/c/h.rs"),
        "a module is not a dependency: {p:?}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn many_unproven_same_name_defs_are_ambiguous_and_ranked_last() {
    let extra = [
        ("src/x/value.rs", "pub struct Value;\n"),
        ("src/y/value.rs", "pub struct Value;\n"),
        ("lone/guess.rs", "pub fn guess(v: Value) {}\n"),
    ];
    let (rt, dir) = noisy("review-ambiguous", &extra);
    let cx = Ctx::new(&rt);
    let b = build(&cx, &dir, VALUE_DIFF, &Options::default()).unwrap();
    let g = b
        .related
        .iter()
        .find(|r| r.path == "lone/guess.rs")
        .expect("kept, not dropped");
    assert!(g.ambiguous, "{g:?}");
    assert_eq!(b.related.last().unwrap().path, "lone/guess.rs");
    assert!(
        !b.related
            .iter()
            .find(|r| r.path == "src/user_store.rs")
            .unwrap()
            .ambiguous
    );
    let lax = build(
        &cx,
        &dir,
        VALUE_DIFF,
        &Options {
            max_defs: 10,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(
        !lax.related
            .iter()
            .find(|r| r.path == "lone/guess.rs")
            .unwrap()
            .ambiguous
    );
    let _ = fs::remove_dir_all(dir);
}

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    #[allow(clippy::disallowed_methods)]
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .envs([
            ("GIT_AUTHOR_NAME", "t"),
            ("GIT_AUTHOR_EMAIL", "t@t"),
            ("GIT_COMMITTER_NAME", "t"),
            ("GIT_COMMITTER_EMAIL", "t@t"),
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn range_indexes_the_right_hand_commit_not_the_checkout() {
    assert_eq!(rhs_of("a..b").as_deref(), Some("b"));
    assert_eq!(rhs_of("a...").as_deref(), Some("HEAD"));
    assert_eq!(rhs_of("HEAD~1"), None);
    let (_rt, dir) = crate::plugins::graph::index::tests::cx("review-range");
    let repo = dir.join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    fs::write(repo.join("lib.rs"), "pub fn v1() {}\n").unwrap();
    git(&repo, &["add", "lib.rs"]);
    git(&repo, &["commit", "-q", "-m", "one"]);
    fs::write(repo.join("lib.rs"), "pub fn v2() {}\n").unwrap();
    git(&repo, &["commit", "-q", "-am", "two"]);
    let two = git(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["checkout", "-q", "HEAD~1"]);
    let cache = dir.join("trees");
    let t = tree_for_in(&repo, "HEAD..main", &cache).unwrap();
    assert!(
        t.starts_with(&cache) && t.ends_with(&two[..16]),
        "{}",
        t.display()
    );
    assert_eq!(
        fs::read_to_string(t.join("lib.rs")).unwrap(),
        "pub fn v2() {}\n"
    );
    // Cached by SHA: a second call reuses the checkout.
    assert_eq!(tree_for_in(&repo, "HEAD~0..main", &cache).unwrap(), t);
    // A clean checkout at the right-hand commit, or the working tree, is used as is.
    git(&repo, &["checkout", "-q", "main"]);
    assert_eq!(tree_for_in(&repo, "HEAD~1..HEAD", &cache).unwrap(), repo);
    assert_eq!(tree_for_in(&repo, "HEAD~1", &cache).unwrap(), repo);
    let _ = fs::remove_dir_all(dir);
}
