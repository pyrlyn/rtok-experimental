// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T199: `ideas.md` structural lint.
//!
//! An idea must not appear twice, or in both a "still open" and a "settled" section
//! (`CLAUDE.md` / the `ideas.md` header). This checks two things statically, without a
//! Markdown parser:
//!
//! 1. Every `I-NN` id has exactly one *defining occurrence*, inside one of the four bucket
//!    sections (`## Open`, `## Later (v0.2+)`, `## Rejected`, `## Promoted`). A defining
//!    occurrence is the id leading a table row's first cell (`| I-86 | ... |`) or a
//!    Rejected-style list item (`- **I-86** ...`) — not an incidental mention in prose
//!    (e.g. "Not semantic cache (I-23)."), which would false-positive on cross-references.
//! 2. Every pipe-table has a header row immediately followed by a `| --- |`-style
//!    separator before any data row, with no blank line splitting a table into two
//!    headerless halves.

const IDEAS: &str = include_str!("../../ideas.md");

const BUCKETS: &[&str] = &["## Open", "## Later", "## Rejected", "## Promoted"];

/// The bucket (by heading prefix) a `## ` line belongs to, or `None` for a heading
/// outside the four tracked sections (e.g. `## How to add an idea`).
fn bucket_of(heading: &str) -> Option<&'static str> {
    BUCKETS.iter().copied().find(|b| heading.starts_with(b))
}

/// `Some(id)` if `line` is a defining occurrence of an `I-<digits>` idea, else `None`.
fn defining_id(line: &str) -> Option<String> {
    let t = line.trim();
    // Table row: "| I-86 | ... |" — first cell after the leading pipe.
    let cell = if let Some(rest) = t.strip_prefix('|') {
        rest.split('|').next().unwrap_or("").trim()
    } else {
        // Rejected list item: "- **I-86** text".
        let rest = t.strip_prefix("- **")?;
        rest.split("**").next().unwrap_or("").trim()
    };
    let digits = cell.strip_prefix("I-")?;
    let digits: String = digits.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        None
    } else {
        Some(format!("I-{digits}"))
    }
}

/// A `| --- | --- |`-style separator: pipes, dashes, colons and spaces only.
fn is_separator(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.contains('-') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

fn sections_per_id(body: &str) -> std::collections::BTreeMap<String, Vec<&'static str>> {
    let mut map: std::collections::BTreeMap<String, Vec<&'static str>> = Default::default();
    let mut current: Option<&'static str> = None;
    for line in body.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            current = bucket_of(&format!("## {h}"));
            continue;
        }
        if is_separator(line) {
            continue; // table separators look like list-ish noise; never an id.
        }
        if let (Some(section), Some(id)) = (current, defining_id(line)) {
            // Every occurrence counts: the same id twice in one section is a duplicate too.
            map.entry(id).or_default().push(section);
        }
    }
    map
}

/// Every pipe-table needs a header line immediately followed by a separator before any
/// data row. Returns the 1-based line numbers of tables missing one.
fn tables_missing_header(body: &str) -> Vec<usize> {
    let lines: Vec<&str> = body.lines().collect();
    let mut bad = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let starts_table = lines[i].trim_start().starts_with('|') && !is_separator(lines[i]);
        if starts_table {
            let sep_ok = lines.get(i + 1).is_some_and(|l| is_separator(l));
            if !sep_ok {
                bad.push(i + 1);
                i += 1;
                continue;
            }
            i += 2;
            while lines
                .get(i)
                .is_some_and(|l| l.trim_start().starts_with('|'))
            {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    bad
}

#[test]
fn ideas_ids_unique_and_disjoint() {
    let map = sections_per_id(IDEAS);
    let offenders: Vec<String> = map
        .into_iter()
        .filter(|(_, sections)| sections.len() != 1)
        .map(|(id, sections)| format!("{id}: {sections:?}"))
        .collect();
    assert!(
        offenders.is_empty(),
        "ideas.md: id(s) not in exactly one section: {offenders:?}"
    );

    let bad_tables = tables_missing_header(IDEAS);
    assert!(
        bad_tables.is_empty(),
        "ideas.md: table row(s) without a header + separator before them, at line(s): {bad_tables:?}"
    );
}
