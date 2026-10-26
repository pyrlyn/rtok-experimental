// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T359: structure lint for `README.md` and `docs/**/*.md`, parsed as CommonMark. The docs are
//! synced to the landing site, whose `check:seo` step rejects a heading level that is skipped,
//! and a renderer silently turns prose into code when a fence is left open. A raw backtick
//! count misses the latter (an even number of fences can still be mis-nested), so the test asks
//! a real parser: every fenced block must end on its own closing fence, no fence may be opened
//! inside another, and a heading may be at most one level deeper than the previous one.

use crate::it::common;

use common::markdown_targets;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};
use std::fs;
use std::path::PathBuf;

fn line_of(text: &str, offset: usize) -> usize {
    text[..offset].matches('\n').count() + 1
}

/// Drops blockquote markers so a fence nested in `>` is judged by its own text.
fn bare(line: &str) -> &str {
    line.trim_start_matches(|c: char| c == '>' || c.is_whitespace())
}

fn fence_marker(line: &str) -> Option<(char, usize, &str)> {
    let line = bare(line);
    let ch = line.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let len = line.chars().take_while(|&c| c == ch).count();
    (len >= 3).then(|| (ch, len, line[len..].trim()))
}

/// Every structural problem in one Markdown document, as `line N: message`.
fn problems(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    // A page may open at h2: the title (h1) comes from the front matter or an HTML header.
    let mut prev_level = 1usize;
    for (event, range) in Parser::new_ext(text, Options::empty()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let level = level as usize;
                if level > prev_level + 1 {
                    let title = text[range.clone()].lines().next().unwrap_or("").trim();
                    out.push(format!(
                        "line {}: heading skips a level: h{prev_level} -> h{level} ({title})",
                        line_of(text, range.start)
                    ));
                }
                prev_level = level;
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))) => {
                check_fence(text, range.start, range.end, &mut out);
            }
            _ => {}
        }
    }
    out
}

fn check_fence(text: &str, start: usize, end: usize, out: &mut Vec<String>) {
    let block = &text[start..end];
    let mut lines = block.lines();
    let opener = lines.next().unwrap_or("");
    let Some((ch, open_len, _)) = fence_marker(opener) else {
        return;
    };
    let body: Vec<&str> = lines.collect();
    let closed = body
        .last()
        .and_then(|l| fence_marker(l))
        .is_some_and(|(c, len, info)| c == ch && len >= open_len && info.is_empty());
    let line = line_of(text, start);
    if !closed {
        out.push(format!("line {line}: fenced block is never closed"));
    }
    let content = if closed {
        &body[..body.len() - 1]
    } else {
        &body[..]
    };
    // An opener with an info string cannot close a fence, so seeing one inside means the
    // author thought the previous block was already shut.
    for (i, l) in content.iter().enumerate() {
        if fence_marker(l)
            .is_some_and(|(c, len, info)| c == ch && len >= open_len && !info.is_empty())
        {
            out.push(format!(
                "line {}: fence opened inside the block that starts at line {line}",
                line + 1 + i
            ));
        }
    }
}

#[test]
fn docs_have_closed_fences_and_unskipped_headings() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();
    for rel in markdown_targets(&root) {
        let text = fs::read_to_string(root.join(&rel))
            .unwrap_or_else(|e| panic!("{}: {e}", rel.display()));
        violations.extend(
            problems(&text)
                .into_iter()
                .map(|p| format!("{}: {p}", rel.display())),
        );
    }
    assert!(
        violations.is_empty(),
        "docs structure:\n{}",
        violations.join("\n")
    );
}

#[test]
fn detects_a_fence_opened_inside_another() {
    // The shape of the T359 bug: the bare fence that was meant to close the first block is
    // missing, so the second opener (with an info string) lands inside it and the later bare
    // fence closes the first block. The raw fence count stays even.
    let doc = "# T\n\n## Ref\n\n```toml\na = 1\n\n### Section\n\n```toml\nb = 2\n```\n\n## Next\n";
    let found = problems(doc);
    assert!(
        found.iter().any(|p| p.contains("fence opened inside")),
        "{found:?}"
    );
}

#[test]
fn detects_an_unclosed_fence() {
    let found = problems("# T\n\n```bash\nls\n");
    assert!(
        found.iter().any(|p| p.contains("never closed")),
        "{found:?}"
    );
}

#[test]
fn detects_a_skipped_heading_level() {
    let found = problems("# T\n\n## A\n\n#### B\n");
    assert!(found.iter().any(|p| p.contains("h2 -> h4")), "{found:?}");
}

#[test]
fn accepts_well_formed_documents() {
    let doc = "# T\n\n## A\n\n```toml\n# [x]\na = 1\n```\n\n### B\n\n````md\n```toml\n```\n````\n\n## C\n";
    assert_eq!(problems(doc), Vec::<String>::new());
}
