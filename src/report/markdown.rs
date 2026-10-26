// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The Markdown rendering of [`Document`](super::Document) — tables, no charts (T22.1).
//!
//! It formats; it computes nothing (D24): every number is a model value, every section
//! names the rows and the window behind it, and a section with no rows says so instead of
//! printing zeros.

use super::Document;

/// Render the whole document. The section order is the fixed P22 set:
/// Window · Savings · Calls · Cache · Expand · Config · Doctor · Recommendations.
pub fn render(doc: &Document) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let w = &doc.ledgers.window;

    let _ = writeln!(
        s,
        "# rtok report\n\nWindow {} → {} ({}) · store {}\n",
        w.from_date, w.to_date, w.since, w.db_path
    );

    // ── Window ──────────────────────────────────────────────────────────────
    let _ = writeln!(s, "## Window\n");
    let _ = writeln!(s, "| ledger | rows | covers |\n| --- | --- | --- |");
    let _ = writeln!(
        s,
        "| calls | {} of {} | window ({}) |",
        w.calls_in_window, w.calls_total, w.since
    );
    let _ = writeln!(
        s,
        "| measurements | {} | whole ledger (no row times) |",
        w.measurements
    );
    let _ = writeln!(s, "| usage | {} | whole ledger (no row times) |", w.usage);
    if w.calls_total == 0 && w.measurements == 0 && w.usage == 0 {
        let _ = writeln!(
            s,
            "\n**No rows in window.** The store has no rows to report."
        );
    }

    // ── Savings ─────────────────────────────────────────────────────────────
    let sav = &doc.ledgers.savings;
    let _ = writeln!(
        s,
        "\n## Savings\n\nEstimated tokens from `Measurement` rows only: est_before − est_after, \
net — an `expand` row counts negative, because retrieval costs tokens. A `Measurement` row \
carries no turn count, so these are floors, not context-token-turns.\n"
    );
    if sav.rows.is_empty() {
        let _ = writeln!(s, "No rows in window.");
    } else {
        let _ = writeln!(
            s,
            "| plugin | rows | est before | est after | saved |\n| --- | ---: | ---: | ---: | ---: |"
        );
        for r in &sav.rows {
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} | {} |",
                cell(&r.plugin),
                r.rows,
                r.est_before,
                r.est_after,
                r.saved
            );
        }
        let _ = writeln!(
            s,
            "\nTotal: {} est tokens over {} `Measurement` rows (whole ledger: no row times).",
            sav.total_saved, sav.total_rows
        );
    }

    // ── Calls ───────────────────────────────────────────────────────────────
    let calls = &doc.ledgers.calls;
    let _ = writeln!(
        s,
        "\n## Calls\n\nLatency per surface, nearest-rank p50/p95 over the calls that recorded \
one.\n"
    );
    if calls.total == 0 {
        let _ = writeln!(s, "No rows in window.");
    } else {
        let _ = writeln!(
            s,
            "| surface | calls | timed | p50 ms | p95 ms |\n| --- | ---: | ---: | ---: | ---: |"
        );
        for r in &calls.rows {
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} | {} |",
                cell(&r.surface),
                r.calls,
                r.timed,
                ms(r.p50_ms),
                ms(r.p95_ms)
            );
        }
        let _ = writeln!(
            s,
            "\n{} of {} `calls` rows in window ({}); surfaces beyond hook/mcp/proxy are not in \
the section set.",
            calls.in_window, calls.total, w.since
        );
    }

    // ── Cache ───────────────────────────────────────────────────────────────
    let cache = &doc.ledgers.cache;
    let _ = writeln!(
        s,
        "\n## Cache\n\nPrompt-cache busts by cause, from the proxy's `usage` rows (whole \
ledger).\n"
    );
    if cache.sessions == 0 {
        let _ = writeln!(s, "No rows in window.");
    } else {
        let _ = writeln!(s, "| cause | busts |\n| --- | ---: |");
        for (cause, n) in &cache.by_cause {
            let _ = writeln!(s, "| {} | {} |", cell(cause), n);
        }
        let _ = writeln!(
            s,
            "\nBusts: {} over {} turns in {} session(s).",
            cache.busts, cache.turns, cache.sessions
        );
    }
    if !doc.ledgers.lanes.is_empty() {
        let _ = writeln!(
            s,
            "\nPrompt-cache hit rate per proxy lane and service tier:\n\n| {} |\n| --- | ---: | ---: | ---: | ---: | ---: |",
            crate::model::LANE_HEADS.join(" | ")
        );
        for row in doc.ledgers.lane_cells() {
            let _ = writeln!(
                s,
                "| {} |",
                row.iter().map(|c| cell(c)).collect::<Vec<_>>().join(" | ")
            );
        }
    }

    // ── Expand ──────────────────────────────────────────────────────────────
    let exp = &doc.ledgers.expand;
    let _ = writeln!(
        s,
        "\n## Expand\n\nHow often a live-zone pointer had to be expanded (`archive_decisions` \
rows).\n"
    );
    if exp.decisions == 0 {
        let _ = writeln!(s, "No rows in window.");
    } else {
        let what = if exp.expanded_ids.is_empty() {
            "none".to_string()
        } else {
            exp.expanded_ids
                .iter()
                .map(|id| cell(super::shown_archive_id(id)))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let _ = writeln!(
            s,
            "`rtok expand` froze {} of {} live-zone pointers ({}%). Expanded: {}.",
            exp.expanded,
            exp.decisions,
            dec(Some(100.0 * exp.rate)),
            what
        );
    }

    // ── Config ──────────────────────────────────────────────────────────────
    let _ = writeln!(
        s,
        "\n## Config\n\nEvery effective key with its origin — the `config show --sources` \
page.\n"
    );
    let _ = writeln!(s, "| key | value | source |\n| --- | --- | --- |");
    for e in &doc.config {
        let _ = writeln!(
            s,
            "| {} | {} | {} |",
            cell(&e.key),
            cell(&e.value),
            cell(&e.source)
        );
    }
    let _ = writeln!(s, "\n{} keys.", doc.config.len());

    // ── Doctor ──────────────────────────────────────────────────────────────
    let _ = writeln!(
        s,
        "\n## Doctor\n\nLive probes (the `rtok doctor` page), not store rows.\n\n```text\n{}```\n",
        doc.doctor.to_text()
    );

    // ── Recommendations ─────────────────────────────────────────────────────
    let _ = writeln!(s, "## Recommendations\n");
    if doc.recommendations.is_empty() {
        let _ = writeln!(s, "No recommendations.");
    } else {
        for r in &doc.recommendations {
            let _ = writeln!(
                s,
                "- **{}**: {} ({})",
                cell(&r.rule),
                cell(&r.finding),
                cell(&r.evidence)
            );
        }
    }
    s
}

/// `4.0` for a recorded latency, `—` for none or a non-finite one — an untimed surface
/// is not a zero-ms one, and `NaN`/`inf` must never reach the document (T105).
/// `pub(crate)` so the `--ai` rendering (T22.4) formats latencies the same way.
pub(crate) fn ms(v: Option<f64>) -> String {
    dec(v)
}

/// One decimal for a latency or a share; `—` for none or non-finite (T105).
pub(crate) fn dec(v: Option<f64>) -> String {
    match v {
        Some(v) if v.is_finite() => format!("{v:.1}"),
        _ => "—".into(),
    }
}

/// A table cell: `|` would end the cell early, so it is escaped; a newline would end the
/// row, so it folds to a space (the `--ai` rendering's `flat`).
fn cell(s: &str) -> String {
    super::ai::flat(s).replace('|', "\\|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::fixtures;

    /// T105 Check: no `NaN`/`inf` reaches the document and no table breaks — every
    /// table keeps the width of its header row.
    fn holds(out: &str) {
        assert!(!out.contains("NaN"), "no NaN: {out}");
        assert!(!out.to_lowercase().contains("inf"), "no inf: {out}");
        let mut width = 0;
        for line in out.lines() {
            if line.starts_with('|') {
                let pipes = line.matches('|').count() - line.matches("\\|").count();
                if width == 0 {
                    width = pipes;
                } else {
                    assert_eq!(pipes, width, "table row broken: {line}");
                }
            } else {
                width = 0;
            }
        }
    }

    #[test]
    fn zero_rows_snapshot() {
        let out = render(&fixtures::zero());
        holds(&out);
        insta::assert_snapshot!("zero", out);
    }

    #[test]
    fn one_row_snapshot() {
        let out = render(&fixtures::one_row());
        holds(&out);
        assert!(
            out.contains("9223372036854775807"),
            "very large numbers: {out}"
        );
        insta::assert_snapshot!("one_row", out);
    }

    #[test]
    fn a_full_archive_id_in_the_expand_list_prints_eight_hex_chars() {
        let mut doc = fixtures::one_row();
        let id = "ab".repeat(32);
        doc.ledgers.expand.expanded_ids = vec![id.clone()];
        let short = crate::names::short_id(&id);
        let out = render(&doc);
        assert!(out.contains(&format!("Expanded: {short}.")), "{out}");
        assert!(!out.contains(&id), "{out}");
        let html = crate::report::html::render(&doc);
        assert!(html.contains(&format!("<code>{short}</code>")), "{html}");
        assert!(!html.contains(&id), "{html}");
        let ai = crate::report::ai::render(&doc, &crate::config::Config::default());
        assert!(ai.contains(&format!("expanded_ids={short}")), "{ai}");
        assert!(!ai.contains(&id), "{ai}");
        #[cfg(feature = "pdf")]
        {
            let pdf = crate::report::pdf::render(&doc);
            assert!(
                pdf.windows(short.len()).any(|w| w == short.as_bytes()),
                "pdf dropped the prefix"
            );
            assert!(
                !pdf.windows(id.len()).any(|w| w == id.as_bytes()),
                "pdf kept the full sha"
            );
        }
    }

    #[test]
    fn hostile_text_snapshot() {
        let out = render(&fixtures::hostile());
        holds(&out);
        assert!(out.contains("\\|"), "the table pipe is escaped: {out}");
        for line in out
            .lines()
            .filter(|l| l.starts_with('|') && l.contains("second line"))
        {
            assert!(
                line.contains("script"),
                "the cell folded to one row: {line}"
            );
        }
        insta::assert_snapshot!("hostile", out);
    }
}
