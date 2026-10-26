// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Reviewer prompt for `rtok review`: instructions, the diff, then digests in a fixed order,
//! deduplicated and cut to a token budget.

use std::collections::BTreeSet;

/// Finding categories the reviewer may use, in this order. `load`, `speed` and `cut` cover
/// resource use, latency and removable code next to the correctness categories.
pub const CATEGORIES: &[(&str, &str)] = &[
    ("bug", "wrong result, crash, data loss"),
    ("contract", "breaks a signature, type or documented promise of the code shown"),
    ("error-handling", "an error is ignored, swallowed or reported wrongly"),
    ("security", "injection, secret exposure, missing permission check"),
    ("test", "changed behaviour no test covers"),
    ("load", "extra memory, CPU, I/O, allocations, queries or processes per call"),
    ("speed", "slower hot path, blocking call, avoidable latency"),
    ("cut", "dead, duplicated or redundant code that can be removed or simplified"),
];

/// The four lines every finding carries, in this order.
pub const FINDING_FIELDS: &[&str] = &["What the code does", "What goes wrong", "How to fix", "Impact if not fixed"];

/// The instructions every prompt starts with: role, digest caveat, categories, finding format.
pub fn instructions() -> String {
    let mut s = String::from(
        "You are reviewing a code change. The unified diff is below, followed by digests of the \
changed files and of files that use or are used by them. A digest lists imports, doc comments, \
signatures and data types only; function bodies are elided as `{ … }` on purpose, so do not report \
missing bodies. Report only real problems, each in one of these categories:\n",
    );
    for (name, what) in CATEGORIES {
        s.push_str(&format!("- `{name}`: {what}\n"));
    }
    s.push_str("\nFormat every finding exactly like this:\n\n");
    s.push_str("### `file:line` · category · severity (high/medium/low)\n");
    for f in FINDING_FIELDS {
        s.push_str(&format!("- **{f}:** …\n"));
    }
    s.push_str("\nIf you find nothing, say `No issues found.`");
    s
}

/// One context block the budget may keep or drop.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Section {
    pub path: String,
    /// `changed`, `dependent`, `dependency` or `full`.
    pub role: String,
    pub body: String,
}

/// The assembled prompt and its accounting.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Prompt {
    #[serde(skip)]
    pub text: String,
    pub tokens: u32,
    pub diff_tokens: u32,
    pub context_tokens: u32,
    pub included: Vec<String>,
    pub dropped: Vec<String>,
    pub duplicates: Vec<String>,
    /// The diff alone is larger than the budget; it is still sent whole.
    pub over_budget: bool,
}

fn fence(s: &str) -> &'static str {
    if s.contains("```") { "````" } else { "```" }
}

fn block(title: &str, body: &str) -> String {
    let f = fence(body);
    format!(
        "### {title}\n{f}\n{}{}{f}\n\n",
        body,
        if body.ends_with('\n') { "" } else { "\n" }
    )
}

/// Build the prompt. `sections` arrive in priority order (changed files first, then related
/// files by rank) and are kept in that order. A section whose path or body was already kept
/// is a duplicate. With `budget`, sections that would push the total past it are dropped;
/// the diff and instructions are always kept.
pub fn assemble(
    diff: &str,
    sections: &[Section],
    budget: Option<u32>,
    estimate: impl Fn(&str) -> u32,
) -> Prompt {
    let mut text = String::new();
    text.push_str(&instructions());
    text.push_str("\n\n## Diff\n");
    text.push_str(&block("diff", diff));
    let diff_tokens = estimate(&text);
    let mut p = Prompt {
        diff_tokens,
        over_budget: budget.is_some_and(|b| diff_tokens > b),
        ..Prompt::default()
    };
    let mut total = diff_tokens;
    let mut seen_paths = BTreeSet::new();
    let mut seen_bodies = BTreeSet::new();
    let mut ctx = String::new();
    for s in sections {
        if s.body.trim().is_empty() {
            continue;
        }
        if !seen_paths.insert(s.path.clone()) || !seen_bodies.insert(s.body.clone()) {
            p.duplicates.push(s.path.clone());
            continue;
        }
        let b = block(&format!("{} ({})", s.path, s.role), &s.body);
        let t = estimate(&b);
        if budget.is_some_and(|max| total + t > max) {
            p.dropped.push(s.path.clone());
            continue;
        }
        total += t;
        p.included.push(s.path.clone());
        ctx.push_str(&b);
    }
    if !ctx.is_empty() {
        text.push_str("## Context digests\n");
        text.push_str(&ctx);
    }
    if !p.dropped.is_empty() {
        text.push_str(&format!(
            "_{} related file(s) omitted by the token budget._\n",
            p.dropped.len()
        ));
    }
    p.context_tokens = total - diff_tokens;
    p.tokens = estimate(&text);
    p.text = text;
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn est(s: &str) -> u32 {
        s.len().div_ceil(4) as u32
    }

    fn sec(path: &str, role: &str, body: &str) -> Section {
        Section {
            path: path.into(),
            role: role.into(),
            body: body.into(),
        }
    }

    #[test]
    fn diff_comes_first_then_sections_in_order_and_duplicates_are_dropped() {
        let s = [
            sec("a.rs", "changed", "pub fn a() { … }\n"),
            sec("b.rs", "dependent", "pub fn b() { … }\n"),
            sec("a.rs", "dependency", "other\n"),
            sec("c.rs", "dependency", "pub fn b() { … }\n"),
        ];
        let p = assemble("+x\n", &s, None, est);
        let (d, a, b) = (
            p.text.find("+x").unwrap(),
            p.text.find("a.rs").unwrap(),
            p.text.find("b.rs").unwrap(),
        );
        assert!(d < a && a < b, "{}", p.text);
        assert_eq!(p.included, ["a.rs", "b.rs"]);
        assert_eq!(p.duplicates, ["a.rs", "c.rs"]);
        assert_eq!(
            assemble("+x\n", &s, None, est).text,
            p.text,
            "deterministic"
        );
    }

    #[test]
    fn instructions_list_every_category_and_the_finding_format() {
        let t = instructions();
        for c in ["`bug`", "`contract`", "`error-handling`", "`security`", "`test`", "`load`", "`speed`", "`cut`"] {
            assert!(t.contains(c), "{c}: {t}");
        }
        let at = |s: &str| t.find(s).unwrap_or_else(|| panic!("{s}: {t}"));
        assert!(at("What the code does") < at("What goes wrong"));
        assert!(at("What goes wrong") < at("How to fix"));
        assert!(at("How to fix") < at("Impact if not fixed"));
        assert!(t.ends_with("`No issues found.`"));
        let p = assemble("+x\n", &[], None, |s| s.len() as u32);
        assert!(p.text.starts_with(&t), "{}", p.text);
    }

    #[test]
    fn budget_drops_what_does_not_fit_and_keeps_the_diff() {
        let big = "x".repeat(4000);
        let s = [
            sec("big.rs", "dependent", &big),
            sec("small.rs", "dependent", "fn s();\n"),
        ];
        let base = assemble("+x\n", &[], None, est).tokens;
        let p = assemble("+x\n", &s, Some(base + 100), est);
        assert_eq!(p.dropped, ["big.rs"]);
        assert_eq!(p.included, ["small.rs"]);
        assert!(p.tokens <= base + 100 + 20, "{}", p.tokens);
        assert!(p.text.contains("1 related file(s) omitted"));
        let tiny = assemble("+x\n", &s, Some(1), est);
        assert!(tiny.over_budget && tiny.text.contains("+x") && tiny.included.is_empty());
    }
}
