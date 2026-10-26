// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok report` (P22, decision D24): the operator model as one document.
//!
//! [`Document`] is the format-neutral shape — T22.2 (html), T22.3 (pdf) and T22.4 (`--ai`)
//! render these same sections, in this order; [`markdown`] is the first renderer because
//! it needs none, which is what proves the *content* before any layout work starts. The
//! document is assembled from the D23 model (`crate::model`) and nothing else: a
//! number that is not in a model struct cannot appear here, and every figure carries the
//! rows it came from and the window it covers.

pub mod advice;
pub mod ai;
pub mod html;
pub mod markdown;
#[cfg(feature = "pdf")]
pub mod pdf;

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::config::Config;
use crate::model;

/// One recommendation: a rule over the ledgers that prints what triggered it and the rows
/// it read ([`advice`], T22.5; never a language-model call). Empty when no rule fires —
/// the section heading still renders, with a "no recommendations" line.
#[derive(Debug, Serialize)]
pub struct Recommendation {
    pub rule: String,
    pub finding: String,
    pub evidence: String,
}

/// The whole report: the fixed section set of P22. Ledger numbers come from
/// [`model::ReportLedgers`] (one store open), Config from [`model::config_entries`]
/// (the `config show --sources` page), Doctor from [`model::doctor`] (live probes).
#[derive(Debug)]
pub struct Document {
    pub ledgers: model::ReportLedgers,
    pub config: Vec<model::ConfigEntry>,
    /// The `rtok doctor` page — the one section whose figures are live probes rather than
    /// store rows, and the only one the report says that about.
    pub doctor: crate::doctor::Report,
    /// T22.5 rules over the ledgers, most recoverable tokens first; empty when nothing
    /// fires, and the section says so instead of filler advice.
    pub recommendations: Vec<Recommendation>,
}

/// Build the document from the model. `home` / `config_file` feed the Config page exactly
/// as `config show --sources` resolves them, so the two cannot disagree about a layer.
pub fn document(cfg: &Config, home: &Path, config_file: Option<&Path>) -> Result<Document> {
    let ledgers = model::report_ledgers(cfg)?;
    let recommendations = advice::recommendations(&ledgers, cfg);
    Ok(Document {
        ledgers,
        config: model::config_entries(home, config_file)?,
        doctor: model::doctor(cfg)?,
        recommendations,
    })
}

/// Each value's share of the largest value in `pairs`, in `0.0..=1.0` — the one
/// scale computation [`html::bars`](super::html) and [`pdf::bars`](super::pdf) both
/// need, so the two renderers can't drift apart on how a bar's length is derived.
/// Negatives count as 0 (a chart never draws a negative-width bar); an all-zero
/// series gives all 0.0 rather than dividing by zero.
pub(super) fn bar_shares(pairs: &[(String, i64)]) -> Vec<f64> {
    let max = pairs
        .iter()
        .map(|(_, v)| (*v).max(0))
        .max()
        .unwrap_or(0)
        .max(1);
    pairs
        .iter()
        .map(|(_, v)| (*v).max(0) as f64 / max as f64)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{bar_shares, shown_archive_id};

    #[test]
    fn bar_shares_scales_to_the_max_and_floors_negatives() {
        let pairs = vec![
            ("a".to_string(), 10),
            ("b".to_string(), 5),
            ("c".to_string(), -3),
        ];
        assert_eq!(bar_shares(&pairs), vec![1.0, 0.5, 0.0]);
    }

    #[test]
    fn bar_shares_all_zero_is_all_zero() {
        let pairs = vec![("a".to_string(), 0), ("b".to_string(), 0)];
        assert_eq!(bar_shares(&pairs), vec![0.0, 0.0]);
    }

    #[test]
    fn bar_shares_empty_is_empty() {
        let pairs: Vec<(String, i64)> = Vec::new();
        assert_eq!(bar_shares(&pairs), Vec::<f64>::new());
    }

    #[test]
    fn shown_archive_id_shortens_only_a_full_sha() {
        let full = "ab".repeat(32);
        assert_eq!(shown_archive_id(&full), "abababab");
        assert_eq!(shown_archive_id("abababab"), "abababab");
        assert_eq!(shown_archive_id(&"f".repeat(12)), &"f".repeat(12));
        assert_eq!(
            shown_archive_id("<script>alert(1)</script>"),
            "<script>alert(1)</script>"
        );
    }
}

/// A full archive sha prints as eight hex chars. `rtok expand` resolves that prefix.
/// A shorter id, or text that is not hex, stays whole: a stored prefix is not
/// re-cut, and the hostile-text fixture is still escaped in full.
pub(crate) fn shown_archive_id(id: &str) -> &str {
    if id.len() >= 64 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
        crate::names::short_id(id)
    } else {
        id
    }
}

/// Fixed [`Document`] models for the renderer snapshot tests (T105): zero rows, one row
/// with very large numbers and non-finite latencies, and hostile text carrying `<`, `|`,
/// backticks and a newline. Every field is a literal — no store, no clock.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::doctor::report_fixture;
    use crate::model;

    fn base() -> Document {
        Document {
            ledgers: model::ReportLedgers {
                window: model::ReportWindow {
                    since: "30d".into(),
                    from_unix: 0,
                    to_unix: 0,
                    from_date: "2026-09-01".into(),
                    to_date: "2026-10-01".into(),
                    db_path: "/tmp/rtok.db".into(),
                    calls_in_window: 0,
                    calls_total: 0,
                    measurements: 0,
                    usage: 0,
                },
                savings: model::ReportSavingsSection {
                    rows: Vec::new(),
                    total_rows: 0,
                    total_saved: 0,
                    kinds: Vec::new(),
                },
                sinks: model::ReportSinksSection { rows: Vec::new() },
                calls: model::ReportCallsSection {
                    rows: Vec::new(),
                    in_window: 0,
                    total: 0,
                    hooks: Vec::new(),
                },
                cache: model::ReportCache {
                    sessions: 0,
                    turns: 0,
                    busts: 0,
                    by_cause: Vec::new(),
                    detail: Vec::new(),
                },
                lanes: Default::default(),
                expand: model::ReportExpand {
                    decisions: 0,
                    expanded: 0,
                    rate: 0.0,
                    expanded_ids: Vec::new(),
                    cost: 0,
                    cost_rows: 0,
                },
            },
            config: Vec::new(),
            doctor: report_fixture(),
            recommendations: Vec::new(),
        }
    }

    /// Zero savings: every section says there are no rows instead of printing zeros.
    pub(crate) fn zero() -> Document {
        base()
    }

    /// One row per table, with very large numbers and the latencies a store may hold
    /// (`NaN`, `inf`) — the document must print `—`, never those.
    pub(crate) fn one_row() -> Document {
        let mut d = base();
        let w = &mut d.ledgers.window;
        w.calls_in_window = 1;
        w.calls_total = 1;
        w.measurements = u64::MAX;
        w.usage = u64::MAX;
        d.ledgers.savings.rows.push(model::ReportSavings {
            plugin: "cmd".into(),
            rows: u64::MAX,
            est_before: i64::MAX,
            est_after: i64::MIN,
            saved: i64::MAX,
        });
        d.ledgers.savings.total_rows = u64::MAX;
        d.ledgers.savings.total_saved = i64::MAX;
        d.ledgers.calls.rows.push(model::ReportCalls {
            surface: "hook".into(),
            calls: u64::MAX,
            timed: 1,
            p50_ms: Some(f64::NAN),
            p95_ms: Some(f64::INFINITY),
        });
        d.ledgers.calls.in_window = 1;
        d.ledgers.calls.total = 1;
        d.ledgers.cache.sessions = 1;
        d.ledgers.cache.turns = u64::MAX;
        d.ledgers.cache.busts = u64::MAX;
        d.ledgers.cache.by_cause.push(("tools".into(), u64::MAX));
        d.ledgers.expand = model::ReportExpand {
            decisions: i64::MAX,
            expanded: i64::MAX,
            rate: 1.0,
            expanded_ids: vec!["f".repeat(64)],
            cost: i64::MAX,
            cost_rows: u64::MAX,
        };
        d.config.push(model::ConfigEntry {
            key: "core.db_path".into(),
            value: "/tmp/rtok.db".into(),
            source: "user".into(),
        });
        d.recommendations.push(Recommendation {
            rule: "cache-bust".into(),
            finding: format!("{} tokens", i64::MAX),
            evidence: format!("{} rows", u64::MAX),
        });
        d
    }

    /// Hostile text in every free-form field: markup, a table pipe, backticks, a newline,
    /// plus a corrupt (non-finite) expand rate.
    pub(crate) fn hostile() -> Document {
        const EVIL: &str = "<script>alert(1)</script> | `tick`\nsecond line";
        let mut d = base();
        d.ledgers.window.db_path = EVIL.into();
        d.ledgers.savings.rows.push(model::ReportSavings {
            plugin: EVIL.into(),
            rows: 1,
            est_before: 1,
            est_after: 1,
            saved: 0,
        });
        d.ledgers.savings.total_rows = 1;
        d.ledgers.cache.sessions = 1;
        d.ledgers.cache.turns = 1;
        d.ledgers.cache.by_cause.push((EVIL.into(), 1));
        d.ledgers.expand = model::ReportExpand {
            decisions: 1,
            expanded: 0,
            rate: f64::NAN,
            expanded_ids: vec![EVIL.into()],
            cost: 0,
            cost_rows: 0,
        };
        d.config.push(model::ConfigEntry {
            key: EVIL.into(),
            value: EVIL.into(),
            source: EVIL.into(),
        });
        d.doctor.proxy = EVIL.into();
        d.recommendations.push(Recommendation {
            rule: EVIL.into(),
            finding: EVIL.into(),
            evidence: EVIL.into(),
        });
        d
    }
}
