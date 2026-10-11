// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T484: the `calls` frame. The poller folds every batch into one [`CallsStore`] and sends this
//! view of it, so the page renders totals instead of computing them and shows what `rtok tui`
//! shows. The page picks a window and the feed filters itself: all four windows travel in every
//! frame, and a filter only narrows the feed rows, never a total.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Serialize;

use super::calls_store::{CallsStore, Finished, Running, Spark, Totals, WINDOWS};

/// The state of the live calls panel at `now`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, JsonSchema)]
pub struct CallsView {
    /// The server's clock in epoch milliseconds: the page ages rows by `now - at` on this clock,
    /// so a skew between the two machines moves nothing.
    pub now: i64,
    pub running: Vec<Running>,
    /// Newest first.
    pub feed: Vec<Finished>,
    /// `[plugins.graph] live_heat_window_s`: how long a node stays warm on the live canvas.
    pub heat_window_s: u32,
    /// One per chip, in the order of [`WINDOWS`]; the last is "since start".
    pub windows: Vec<WindowView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct WindowView {
    pub label: String,
    pub calls: u64,
    pub failed: u64,
    pub before: i64,
    pub after: i64,
    /// Most calls first.
    pub tools: Vec<ToolView>,
    pub backends: BTreeMap<String, u64>,
    pub fallbacks: u64,
    pub caps: u64,
    pub symbols: u64,
    pub crossed: u64,
    pub symbols_returned: u64,
    pub files_touched: u64,
    pub projects_hit: u64,
    /// `None` before any call in the window has ended.
    pub latency: Option<Latency>,
    pub spark: Spark,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ToolView {
    pub tool: String,
    pub calls: u64,
    pub saved: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Latency {
    pub p50: f64,
    pub p95: f64,
}

impl WindowView {
    fn of(label: &str, t: &Totals, spark: Spark) -> Self {
        let mut tools: Vec<ToolView> = t
            .tools
            .iter()
            .map(|(tool, v)| ToolView {
                tool: tool.clone(),
                calls: v.calls,
                saved: v.saved,
            })
            .collect();
        // Stable on the name, which the map already sorts by, so equal bars do not swap places.
        tools.sort_by_key(|t| std::cmp::Reverse(t.calls));
        Self {
            label: label.to_owned(),
            calls: t.calls,
            failed: t.failed,
            before: t.before,
            after: t.after,
            tools,
            backends: t.backends.clone(),
            fallbacks: t.fallbacks,
            caps: t.caps,
            symbols: t.symbols,
            crossed: t.crossed,
            symbols_returned: t.symbols_returned,
            files_touched: t.files_touched,
            projects_hit: t.projects_hit,
            latency: t.latency().map(|(p50, p95)| Latency { p50, p95 }),
            spark,
        }
    }
}

impl CallsView {
    pub fn of(store: &CallsStore, now: i64) -> Self {
        Self {
            now,
            running: store.running.clone(),
            feed: store.feed.clone(),
            heat_window_s: store.heat_window_s,
            windows: WINDOWS
                .iter()
                .enumerate()
                .map(|(i, (label, _))| {
                    WindowView::of(label, &store.window_totals(i, now), store.spark(i, now))
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::calls_store::fixtures::{T, batch, end, event};
    use super::*;

    #[test]
    fn every_window_carries_the_totals_the_store_computes() {
        let mut s = CallsStore::default();
        s.fold(&batch(vec![end("old", 10, 5)], 0), T);
        s.fold(
            &batch(vec![end("new", 40, 5), event("run")], 0),
            T + 9 * 60_000,
        );
        let v = CallsView::of(&s, T + 9 * 60_000 + 1000);
        assert_eq!(
            v.windows.iter().map(|w| &w.label[..]).collect::<Vec<_>>(),
            ["1 min", "5 min", "15 min", "since start"]
        );
        assert_eq!(
            v.windows.iter().map(|w| w.before).collect::<Vec<_>>(),
            [40, 40, 50, 50]
        );
        assert_eq!(v.windows[3].calls, s.all.calls);
        assert_eq!(v.running.len(), 1);
        assert_eq!(v.feed.len(), 2);
    }

    #[test]
    fn tools_are_listed_most_calls_first_and_latency_is_the_measured_percentile() {
        let mut s = CallsStore::default();
        let mut impact = end("b", 2, 1);
        impact.tool = "impact".into();
        s.fold(&batch(vec![end("a", 2, 1), impact, end("c", 2, 1)], 0), T);
        let w = &CallsView::of(&s, T).windows[3];
        assert_eq!(
            w.tools
                .iter()
                .map(|t| (&t.tool[..], t.calls))
                .collect::<Vec<_>>(),
            [("callers", 2), ("impact", 1)]
        );
        assert_eq!(
            w.latency,
            Some(Latency {
                p50: 12.0,
                p95: 12.0
            })
        );
        assert_eq!(
            CallsView::of(&CallsStore::default(), T).windows[0].latency,
            None
        );
    }

    #[test]
    fn the_wire_names_are_the_fields_the_page_reads() {
        let v = serde_json::to_value(CallsView::of(&CallsStore::default(), T)).unwrap();
        assert_eq!(v["windows"][1]["label"], "5 min");
        assert_eq!(v["windows"][1]["symbols_returned"], 0);
        assert!(v["windows"][1]["latency"].is_null());
        assert_eq!(v["windows"][1]["spark"]["span_ms"], 300_000);
        assert_eq!(
            v["windows"][1]["spark"]["calls"].as_array().unwrap().len(),
            30
        );
    }
}
