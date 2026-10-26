// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T221: public-number lint. Every `N %` / `N MiB|KiB|GiB|TiB|MB|KB|GB` / `N ms` figure in
//! `README.md`/`docs/**/*.md` must share its paragraph/table-row block with evidence:
//! `research.md`, a `YYYY-MM-DD` date, a test/bench name, or prose naming a source ("X
//! claims/measured/reports Y %"). No `Measurement` row = no saving, in code or
//! prose (`AGENTS.md`) — this is that rule for the docs. Also: a README integration-target
//! count must equal `ls tests/*.rs`. `docs/report.md` is skipped (T22.0, out of scope), as are
//! the `docs/ru/` and `docs/uk/` translations; `EXEMPT_FIGURES` covers the "10 ms" fail-open
//! budget.

use crate::common;

use common::markdown_targets;
use regex::Regex;
use std::fs;
use std::path::PathBuf;

/// The fail-open hook budget (D1) — a design limit, not a measurement, so it needs no citation.
const EXEMPT_FIGURES: &[&str] = &["10 ms"];
const SKIP_FILES: &[&str] = &["docs/report.md"];
/// Translations mirror the English docs, which are linted; their prose cannot match the English
/// evidence words, so the locale trees are skipped.
const SKIP_DIRS: &[&str] = &["docs/ru/", "docs/uk/"];

#[derive(PartialEq, Clone, Copy)]
enum Kind {
    Para,
    Table,
}
/// (kind, start line, end line), 0-based, inclusive.
struct Block(Kind, usize, usize);

fn is_table(l: &str) -> bool {
    l.trim_start().starts_with('|')
}

/// Blank-line/heading-delimited paragraphs; each table row and each list item is its own
/// block, so a citation elsewhere in a long mixed table/list cannot cover an uncited row.
fn build_blocks(lines: &[&str], in_fence: &[bool], list_re: &Regex) -> Vec<Block> {
    let is_list = |l: &str| {
        let t = l.trim_start();
        t.starts_with("- ") || t.starts_with("* ") || list_re.is_match(t)
    };
    let mut blocks = Vec::new();
    let n = lines.len();
    let mut i = 0;
    while i < n {
        let line = lines[i];
        if line.trim().is_empty() || line.starts_with('#') || in_fence[i] {
            i += 1;
            continue;
        }
        if is_table(line) {
            blocks.push(Block(Kind::Table, i, i));
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i + 1;
        while j < n
            && !lines[j].trim().is_empty()
            && !lines[j].starts_with('#')
            && !is_table(lines[j])
            && !is_list(lines[j])
        {
            j += 1;
        }
        blocks.push(Block(Kind::Para, start, j - 1));
        i = j;
    }
    blocks
}

/// The text a figure on line `idx` may cite from: its whole block, and for a table also
/// the paragraph introducing it (unless a heading intervenes). A line outside every block
/// stands alone.
fn block_context(blocks: &[Block], lines: &[&str], idx: usize) -> String {
    let block_text = |bi: usize| lines[blocks[bi].1..=blocks[bi].2].join("\n");
    let Some(bi) = blocks.iter().position(|b| b.1 <= idx && idx <= b.2) else {
        return lines[idx].to_string();
    };
    let mut c = block_text(bi);
    if blocks[bi].0 != Kind::Table {
        return c;
    }
    let mut first = bi;
    while first > 0 && blocks[first - 1].0 == Kind::Table {
        first -= 1;
    }
    // inherit the paragraph before the table, unless a heading intervenes
    if first > 0 && blocks[first - 1].0 == Kind::Para {
        let prev = &blocks[first - 1];
        let crosses = (prev.2 + 1..blocks[first].1).any(|k| lines[k].starts_with('#'));
        if !crosses {
            c.push(' ');
            c.push_str(&block_text(first - 1));
        }
    }
    c
}

/// Figures with no citation anywhere in their block (see `build_blocks`; a table also
/// checks the paragraph introducing it): `(1-based line, figure text)`.
fn missing_citations(text: &str) -> Vec<(usize, String)> {
    let num_re = Regex::new(
        r"(?:^|[^A-Za-z0-9_])([±+-]?\d+(?:\s?\d{3})*(?:\.\d+)?\s?(?:%|MiB\b|KiB\b|GiB\b|TiB\b|MB\b|KB\b|GB\b|ms\b))",
    )
    .unwrap();
    let list_re = Regex::new(r"^\d+\.\s").unwrap();
    let date_re = Regex::new(r"\b20\d{2}-\d{2}-\d{2}\b").unwrap();
    let test_re = Regex::new(
        r"(?i)`[a-z0-9_./-]*(test|bench|nextest)[a-z0-9_./-]*`|cargo (nextest run|test)|tests?/[a-z0-9_./-]+\.rs",
    )
    .unwrap();
    // `(?:^|\s)` before the word, not just `\b`, so "provider-reported" isn't attribution.
    let evidence_re = Regex::new(
        r"(?i)(?:^|\s)(?:claim(s|ed)?|measur(ed|ement)|found|report(s|ed)?|vendor|own (bench|readme|blog|numbers|meter)|cit(e|es|ed))\b",
    )
    .unwrap();
    let fence_re = Regex::new(r"^\s*```").unwrap();

    let lines: Vec<&str> = text.lines().collect();
    let n = lines.len();
    let mut in_fence = vec![false; n];
    let mut open = false;
    for (idx, l) in lines.iter().enumerate() {
        if fence_re.is_match(l) {
            open = !open;
        }
        in_fence[idx] = open;
    }

    let blocks = build_blocks(&lines, &in_fence, &list_re);
    let nearest_heading = |idx: usize| {
        (0..=idx)
            .rev()
            .find(|&j| lines[j].starts_with('#'))
            .map_or("", |j| lines[j])
    };

    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if in_fence[i] {
            continue;
        }
        for cap in num_re.captures_iter(line) {
            let fig = cap[1].to_string();
            let heading = nearest_heading(i);
            let ctx = block_context(&blocks, &lines, i);
            let ctx = format!("{ctx} {heading}");
            let ok = ctx.contains("research.md")
                || date_re.is_match(&ctx)
                || test_re.is_match(&ctx)
                || evidence_re.is_match(&ctx);
            if !ok {
                out.push((i + 1, fig));
            }
        }
    }
    out
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn public_numbers_cite_their_evidence() {
    let root = manifest_dir();
    let mut violations = Vec::new();
    for rel in markdown_targets(&root) {
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if SKIP_FILES.contains(&rel_str.as_str())
            || SKIP_DIRS.iter().any(|d| rel_str.starts_with(d))
        {
            continue;
        }
        let text = fs::read_to_string(root.join(&rel)).unwrap_or_else(|e| panic!("{rel_str}: {e}"));
        for (line, fig) in missing_citations(&text) {
            if EXEMPT_FIGURES.contains(&fig.trim()) {
                continue;
            }
            violations.push(format!(
                "{rel_str}:{line}: {fig:?} has no research.md/date/test/source nearby"
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "public numbers without a citation (cite it, soften the claim, or extend EXEMPT_FIGURES with a reason):\n{}",
        violations.join("\n")
    );
}

#[test]
fn readme_integration_target_count_matches_tests_dir() {
    let root = manifest_dir();
    let readme = fs::read_to_string(root.join("README.md")).unwrap();
    let count_re = Regex::new(r"(\d+)\s+integration targets?").unwrap();
    let Some(cap) = count_re.captures(&readme) else {
        return;
    };
    let stated: usize = cap[1].parse().unwrap();
    let actual = fs::read_dir(root.join("tests"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("rs"))
        .count();
    assert_eq!(
        stated, actual,
        "README.md states {stated} integration targets but tests/*.rs has {actual}; prefer \
         stating the rule (\"one integration-test binary per file under tests/*.rs\") instead \
         of a number that goes stale"
    );
}
