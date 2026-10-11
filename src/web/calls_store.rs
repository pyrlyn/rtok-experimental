// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The live calls aggregation (T329.26, ported to Rust by T480 and made the only one by T484):
//! `rtok web` folds each poll's batch here and sends the totals in the `calls` frame
//! (`calls_view.rs`), and `rtok tui` folds the same batches with the same store, so both
//! surfaces show one computation. Totals come from the batch `summary`, which counts every
//! event of a poll, so they equal what `rtok stats` sums over the same calls.

use std::collections::{BTreeMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use schemars::JsonSchema;
use serde::Serialize;

use super::live::CallBatch;
use crate::config::Graph;
use crate::store::{EventPhase, GraphEvent};

/// A call with no end this long after its start is shown as interrupted: its process is gone.
pub const INTERRUPT_MS: i64 = 120_000;
const LONGEST_BUCKET_MS: i64 = 900_000;
/// Latencies kept for the percentiles: "since start" would otherwise grow with every call.
pub const LATENCY_KEEP: usize = 1000;
/// Slots of a sparkline. The page and the TUI draw the same series, so the count is fixed here.
pub const SPARK_SLOTS: usize = 30;

/// The window chips of the page; the last one is "since start" and reads the running total.
pub const WINDOWS: [(&str, i64); 4] = [
    ("1 min", 60_000),
    ("5 min", 300_000),
    ("15 min", LONGEST_BUCKET_MS),
    ("since start", i64::MAX),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Running {
    pub call: String,
    pub tool: String,
    pub target: Option<String>,
    pub project: Option<String>,
    pub session: String,
    /// T329.34: what the store calls the session (`claude 3f9a1c2e`), else the session's start.
    pub caller: String,
    /// The reader's clock: the interrupt timeout must not depend on the writer's.
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Finished {
    pub call: String,
    pub tool: String,
    pub target: Option<String>,
    pub project: Option<String>,
    pub session: String,
    pub caller: String,
    pub ok: bool,
    pub error: Option<String>,
    pub backend: Option<String>,
    pub ms: Option<f64>,
    pub before: i64,
    pub after: i64,
    pub at: i64,
    pub interrupted: bool,
}

/// Calls and tokens saved per slot over a window, oldest first. Summed from the same buckets as
/// the window totals, so the slots add up to them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Spark {
    /// What the slots cover. "since start" reads the buckets, which are kept for 15 minutes only.
    pub span_ms: i64,
    pub calls: Vec<u64>,
    pub saved: Vec<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolTotals {
    pub calls: u64,
    pub saved: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Totals {
    pub calls: u64,
    pub failed: u64,
    pub before: i64,
    pub after: i64,
    /// Counted from the events a batch lists, so under a burst the bars cover the listed
    /// calls; `calls` is exact.
    pub tools: BTreeMap<String, ToolTotals>,
    pub backends: BTreeMap<String, u64>,
    /// `lsp_fallback` rows, answers cut at `max_tokens`, symbols asked and calls over several
    /// projects: all four come from the batch `summary`, so they are exact under a burst.
    pub fallbacks: u64,
    pub caps: u64,
    pub symbols: u64,
    pub crossed: u64,
    /// What the graph backends counted in the answers (T329.36), from the batch `summary` too.
    pub symbols_returned: u64,
    pub files_touched: u64,
    pub projects_hit: u64,
    /// Milliseconds of the listed calls, oldest first and at most [`LATENCY_KEEP`]: a burst
    /// cut to its newest 100 events is measured on those.
    pub ms: Vec<f64>,
}

/// The poller's and the panes' clock, in epoch milliseconds.
pub fn clock_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// The value at rank `ceil(p * n)`: a latency that was really measured, not an interpolation.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

impl Totals {
    /// Median and 95th percentile of the kept latencies, `None` before any call has ended.
    pub fn latency(&self) -> Option<(f64, f64)> {
        let mut v = self.ms.clone();
        v.sort_by(f64::total_cmp);
        (!v.is_empty()).then(|| (percentile(&v, 0.5), percentile(&v, 0.95)))
    }

    fn add(&mut self, from: &Totals) {
        self.calls += from.calls;
        self.failed += from.failed;
        self.before += from.before;
        self.after += from.after;
        self.fallbacks += from.fallbacks;
        self.caps += from.caps;
        self.symbols += from.symbols;
        self.crossed += from.crossed;
        self.symbols_returned += from.symbols_returned;
        self.files_touched += from.files_touched;
        self.projects_hit += from.projects_hit;
        self.ms.extend_from_slice(&from.ms);
        self.ms.drain(..self.ms.len().saturating_sub(LATENCY_KEEP));
        for (tool, t) in &from.tools {
            let own = self.tools.entry(tool.clone()).or_default();
            own.calls += t.calls;
            own.saved += t.saved;
        }
        for (b, n) in &from.backends {
            *self.backends.entry(b.clone()).or_default() += n;
        }
    }
}

#[derive(Debug, Clone)]
pub struct CallsStore {
    pub running: Vec<Running>,
    /// Newest first.
    pub feed: Vec<Finished>,
    /// `[plugins.graph] live_feed_rows`: how long the feed grows.
    feed_rows: usize,
    /// `[plugins.graph] live_heat_window_s`, carried to the page in the view.
    pub heat_window_s: u32,
    /// One per batch, kept for the longest window.
    buckets: Vec<(i64, Totals)>,
    pub all: Totals,
}

impl Default for CallsStore {
    fn default() -> Self {
        Self::with(&Graph::default())
    }
}

/// The session as a caller column shows it when the store names nobody.
pub fn bare_caller(session: &str) -> String {
    session.chars().take(8).collect()
}

fn ended(e: &GraphEvent, at: i64, caller: String) -> Finished {
    Finished {
        call: e.call.clone(),
        tool: e.tool.clone(),
        target: e.target.clone(),
        project: e.project.clone(),
        session: e.session.clone(),
        caller,
        ok: e.ok,
        error: e.error.clone(),
        backend: e.backend.clone(),
        ms: e.ms,
        before: e.samples.iter().map(|s| i64::from(s.est_before)).sum(),
        after: e.samples.iter().map(|s| i64::from(s.est_after)).sum(),
        at,
        interrupted: false,
    }
}

impl CallsStore {
    pub fn with(cfg: &Graph) -> Self {
        Self {
            running: Vec::new(),
            feed: Vec::new(),
            feed_rows: cfg.live_feed_rows as usize,
            heat_window_s: cfg.live_heat_window_s,
            buckets: Vec::new(),
            all: Totals::default(),
        }
    }

    /// Folds one batch in. Totals come from the batch `summary`, which counts every event, so a
    /// burst cut down to the listed events still adds up to what `rtok stats` sums.
    pub fn fold(&mut self, batch: &CallBatch, now: i64) {
        let ends: Vec<&GraphEvent> = batch
            .events
            .iter()
            .filter(|e| e.phase == EventPhase::End)
            .collect();
        let done: HashSet<&str> = ends.iter().map(|e| e.call.as_str()).collect();
        for e in &batch.events {
            if e.phase == EventPhase::End {
                self.running.retain(|r| r.call != e.call);
            } else if !done.contains(e.call.as_str()) {
                // A progress event refreshes the call but keeps its start time and its place in the list.
                let at = self
                    .running
                    .iter()
                    .find(|r| r.call == e.call)
                    .map_or(now, |r| r.at);
                let r = Running {
                    call: e.call.clone(),
                    tool: e.tool.clone(),
                    target: e.target.clone(),
                    project: e.project.clone(),
                    session: e.session.clone(),
                    caller: batch.caller(&e.session),
                    at,
                };
                match self.running.iter_mut().find(|x| x.call == e.call) {
                    Some(slot) => *slot = r,
                    None => self.running.push(r),
                }
            }
        }
        let mut bucket = Totals {
            calls: u64::from(batch.summary.ends),
            failed: u64::from(batch.summary.failed),
            before: batch.summary.est_before,
            after: batch.summary.est_after,
            fallbacks: u64::from(batch.summary.fallbacks),
            caps: u64::from(batch.summary.caps),
            symbols: u64::from(batch.summary.symbols),
            crossed: u64::from(batch.summary.crossed),
            symbols_returned: u64::from(batch.summary.symbols_returned),
            files_touched: u64::from(batch.summary.files_touched),
            projects_hit: u64::from(batch.summary.projects_hit),
            ..Totals::default()
        };
        let rows: Vec<Finished> = ends
            .iter()
            .map(|e| ended(e, now, batch.caller(&e.session)))
            .collect();
        for row in &rows {
            let t = bucket.tools.entry(row.tool.clone()).or_default();
            t.calls += 1;
            t.saved += row.before - row.after;
            if let Some(b) = &row.backend {
                *bucket.backends.entry(b.clone()).or_default() += 1;
            }
            bucket.ms.extend(row.ms);
        }
        self.all.add(&bucket);
        // A late end for a call already marked interrupted replaces the mark.
        self.feed.retain(|r| !done.contains(r.call.as_str()));
        self.feed.splice(0..0, rows.into_iter().rev());
        self.feed.truncate(self.feed_rows);
        self.buckets.push((now, bucket));
        self.buckets.retain(|(at, _)| now - at <= LONGEST_BUCKET_MS);
    }

    /// Moves a call that never ended into the feed, marked, so it stops counting as running.
    pub fn sweep(&mut self, now: i64) {
        let (stale, live): (Vec<Running>, Vec<Running>) = std::mem::take(&mut self.running)
            .into_iter()
            .partition(|r| now - r.at > INTERRUPT_MS);
        self.running = live;
        let rows = stale.into_iter().rev().map(|r| Finished {
            call: r.call,
            tool: r.tool,
            target: r.target,
            project: r.project,
            session: r.session,
            caller: r.caller,
            ok: false,
            error: None,
            backend: None,
            ms: None,
            before: 0,
            after: 0,
            at: r.at,
            interrupted: true,
        });
        self.feed.splice(0..0, rows);
        self.feed.truncate(self.feed_rows);
    }

    /// Summed from the buckets on every call, never kept as a second counter.
    pub fn window_totals(&self, window: usize, now: i64) -> Totals {
        let ms = WINDOWS[window].1;
        if ms == i64::MAX {
            return self.all.clone();
        }
        let mut sum = Totals::default();
        for (at, b) in &self.buckets {
            if now - at <= ms {
                sum.add(b);
            }
        }
        sum
    }
}

impl CallsStore {
    /// The window's buckets spread over [`SPARK_SLOTS`] equal slots ending at `now`.
    pub fn spark(&self, window: usize, now: i64) -> Spark {
        let span_ms = WINDOWS[window].1.min(LONGEST_BUCKET_MS);
        let width = span_ms / SPARK_SLOTS as i64;
        let mut spark = Spark {
            span_ms,
            calls: vec![0; SPARK_SLOTS],
            saved: vec![0; SPARK_SLOTS],
        };
        for (at, b) in &self.buckets {
            let age = (now - at).max(0);
            if age > span_ms {
                continue;
            }
            // A bucket exactly `span_ms` old lands one past the first slot: it belongs in it.
            let back = ((age / width) as usize).min(SPARK_SLOTS - 1);
            spark.calls[SPARK_SLOTS - 1 - back] += b.calls;
            spark.saved[SPARK_SLOTS - 1 - back] += b.before - b.after;
        }
        spark
    }
}

/// The mark on a feed row whose project is not in the scope; the page shows the same text.
pub const OUTSIDE_SCOPE: &str = "outside scope";
/// Folded calls are those of this long ago or less (the page's heat window is a config knob,
/// this one is fixed so the TUI and the page count alike).
pub const FOLD_MS: i64 = 300_000;

/// What a sparkline is called: "calls over the last 5 min", as the page names it.
pub fn spark_label(what: &str, span_ms: i64) -> String {
    format!(
        "{what} over the last {} min",
        (span_ms as f64 / 60_000.0).round()
    )
}

/// One registry row, as far as the scope needs it.
pub struct ScopeRow<'a> {
    pub id: i32,
    pub name: &'a str,
    pub selected: bool,
    pub links: Vec<i32>,
}

/// The names of the projects the live part is scoped to: the drilled project, else the selected
/// one, with everything its links reach. `None` when there is none, so no call is "outside" a
/// scope that was never chosen (the page's `scopeNames`).
pub fn scope_names(rows: &[ScopeRow], drilled: Option<i32>) -> Option<HashSet<String>> {
    let from = drilled.or_else(|| rows.iter().find(|r| r.selected).map(|r| r.id))?;
    let mut seen = HashSet::new();
    let mut todo = vec![from];
    // A link to an id the registry no longer holds is ignored; cycles end at the first revisit.
    while let Some(id) = todo.pop() {
        if let Some(row) = rows.iter().find(|r| r.id == id)
            && seen.insert(id)
        {
            todo.extend(&row.links);
        }
    }
    let names: HashSet<String> = rows
        .iter()
        .filter(|r| seen.contains(&r.id))
        .map(|r| r.name.to_owned())
        .collect();
    (!names.is_empty()).then_some(names)
}

/// A call names its project; one the scope lacks is counted and listed but never lights the canvas.
pub fn outside_scope(scope: Option<&HashSet<String>>, project: Option<&str>) -> bool {
    matches!((scope, project), (Some(s), Some(p)) if !s.contains(p))
}

/// A node the one-project view draws: its label and the path it stands for.
pub struct DrawnNode {
    pub label: String,
    pub root: String,
}

/// Calls whose own node the cap folded into "+N more", on the node that holds them.
#[derive(Debug, PartialEq, Eq)]
pub struct FoldedCount {
    pub label: String,
    pub calls: u64,
}

impl CallsStore {
    /// The page's `foldedCounters`: with `more` nodes left out, a recent in-scope call whose
    /// target no drawn node is named after counts on the drawn node whose path equals or is a
    /// directory prefix of the target (longest first), else on the "+N more" group. Largest first.
    pub fn folded_counters(
        &self,
        drawn: &[DrawnNode],
        more: usize,
        scope: Option<&HashSet<String>>,
        now: i64,
    ) -> Vec<FoldedCount> {
        if more == 0 {
            return Vec::new();
        }
        let mut by_root: Vec<usize> = (0..drawn.len()).collect();
        by_root.sort_by_key(|&i| std::cmp::Reverse(drawn[i].root.len()));
        let mut counts = vec![0u64; drawn.len()];
        let mut group = 0u64;
        let finished = self
            .feed
            .iter()
            .filter(|r| !r.interrupted && now - r.at <= FOLD_MS)
            .map(|r| (&r.target, &r.project));
        let running = self
            .running
            .iter()
            .filter(|r| now - r.at <= FOLD_MS)
            .map(|r| (&r.target, &r.project));
        for (target, project) in finished.chain(running) {
            let Some(target) = target.as_deref() else {
                continue;
            };
            if outside_scope(scope, project.as_deref()) || drawn.iter().any(|n| n.label == target) {
                continue;
            }
            let holder = by_root.iter().find(|&&i| {
                let root = &drawn[i].root;
                !root.is_empty()
                    && (target == root
                        || target
                            .strip_prefix(root.as_str())
                            .is_some_and(|t| t.starts_with('/')))
            });
            match holder {
                Some(&i) => counts[i] += 1,
                None => group += 1,
            }
        }
        let mut out: Vec<FoldedCount> = drawn
            .iter()
            .zip(counts)
            .filter(|(_, calls)| *calls > 0)
            .map(|(n, calls)| FoldedCount {
                label: n.label.clone(),
                calls,
            })
            .collect();
        if group > 0 {
            out.push(FoldedCount {
                label: format!("+{more} more"),
                calls: group,
            });
        }
        out.sort_by_key(|a| std::cmp::Reverse(a.calls));
        out
    }
}

/// Empty strings mean "all".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeedFilter {
    pub caller: String,
    pub tool: String,
    pub project: String,
}

impl FeedFilter {
    fn keeps(&self, r: &Finished) -> bool {
        (self.caller.is_empty() || r.caller == self.caller)
            && (self.tool.is_empty() || r.tool == self.tool)
            && (self.project.is_empty() || Some(&self.project) == r.project.as_ref())
    }
}

pub fn filter_feed<'a>(feed: &'a [Finished], f: &FeedFilter) -> Vec<&'a Finished> {
    feed.iter().filter(|r| f.keeps(r)).collect()
}

/// The values a filter can pick, sorted.
pub fn distinct<'a>(
    feed: &'a [Finished],
    pick: impl Fn(&'a Finished) -> Option<&'a str>,
) -> Vec<String> {
    let mut v: Vec<String> = feed
        .iter()
        .filter_map(pick)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Fixtures shared with the TUI pane's and the view's tests.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::store::MeasurementSample;
    use crate::web::live::CallSummary;

    pub const T: i64 = 1_000_000;

    pub fn event(call: &str) -> GraphEvent {
        let mut e = GraphEvent::new(call, EventPhase::Start, "session-abcdef", "callers");
        e.project = Some("rtok".into());
        e.target = Some("open_index".into());
        e
    }

    pub fn end(call: &str, before: i32, after: i32) -> GraphEvent {
        let mut e = event(call);
        e.phase = EventPhase::End;
        e.backend = Some("tags".into());
        e.ms = Some(12.0);
        e.samples = vec![MeasurementSample {
            id: 1,
            kind: "graph.callers".into(),
            before_bytes: 0,
            after_bytes: 0,
            est_before: before,
            est_after: after,
            ref_id: None,
        }];
        e
    }

    /// A batch whose summary counts exactly the events it lists, plus `omitted` ends of 10 to 4.
    pub fn batch(events: Vec<GraphEvent>, omitted: u32) -> CallBatch {
        let ends: Vec<&GraphEvent> = events
            .iter()
            .filter(|e| e.phase == EventPhase::End)
            .collect();
        let sum = |f: fn(&MeasurementSample) -> i32| -> i64 {
            ends.iter()
                .flat_map(|e| &e.samples)
                .map(|s| i64::from(f(s)))
                .sum()
        };
        let count = |f: fn(&MeasurementSample) -> bool| -> u32 {
            ends.iter()
                .flat_map(|e| &e.samples)
                .filter(|s| f(s))
                .count() as u32
        };
        CallBatch {
            head: events.len() as i64,
            omitted,
            callers: Default::default(),
            summary: CallSummary {
                starts: events
                    .iter()
                    .filter(|e| e.phase == EventPhase::Start)
                    .count() as u32,
                ends: ends.len() as u32 + omitted,
                failed: ends.iter().filter(|e| !e.ok).count() as u32,
                est_before: sum(|s| s.est_before) + i64::from(omitted) * 10,
                est_after: sum(|s| s.est_after) + i64::from(omitted) * 4,
                fallbacks: count(|s| s.kind == "lsp_fallback"),
                caps: count(|s| s.ref_id.is_some()),
                symbols: ends.iter().filter_map(|e| e.symbols).sum(),
                symbols_returned: ends.iter().filter_map(|e| e.symbols_returned).sum(),
                files_touched: ends.iter().filter_map(|e| e.files_touched).sum(),
                projects_hit: ends.iter().filter_map(|e| e.projects_hit).sum(),
                crossed: ends
                    .iter()
                    .filter(|e| e.total.is_some_and(|t| t > 1))
                    .count() as u32,
            },
            events,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{T, batch, end, event};
    use super::*;
    use crate::store::MeasurementSample;

    fn folded(store: &mut CallsStore, events: Vec<GraphEvent>, now: i64) {
        store.fold(&batch(events, 0), now);
    }

    #[test]
    fn a_sparkline_spreads_the_buckets_over_equal_slots_and_adds_up_to_the_window() {
        let mut s = CallsStore::default();
        let now = T + 600_000;
        folded(&mut s, vec![end("old", 100, 30)], T);
        folded(&mut s, vec![end("edge", 40, 10)], now - 60_000);
        folded(
            &mut s,
            vec![end("new", 20, 5), end("new2", 8, 8)],
            now - 1000,
        );

        let long = s.spark(2, now);
        assert_eq!((long.span_ms, long.calls.len()), (900_000, SPARK_SLOTS));
        // 30 s slots: 10 minutes back is the 21st slot from the end, a second back the last one.
        assert_eq!((long.calls[9], long.saved[9]), (1, 70));
        assert_eq!((long.calls[27], long.saved[27]), (1, 30));
        assert_eq!((long.calls[29], long.saved[29]), (2, 15));
        let w = s.window_totals(2, now);
        assert_eq!(long.calls.iter().sum::<u64>(), w.calls);
        assert_eq!(long.saved.iter().sum::<i64>(), w.before - w.after);

        // A bucket exactly one span old is the first slot; an older one is not in the series.
        let short = s.spark(0, now);
        assert_eq!(
            (short.span_ms, short.calls[0], short.calls[29]),
            (60_000, 1, 2)
        );
        assert_eq!(short.calls.iter().sum::<u64>(), 3);
        // "since start" has no buckets beyond 15 minutes: it draws the last 15.
        assert_eq!(s.spark(3, now), long);
    }

    fn row<'a>(id: i32, name: &'a str, selected: bool, links: &[i32]) -> ScopeRow<'a> {
        ScopeRow {
            id,
            name,
            selected,
            links: links.to_vec(),
        }
    }

    #[test]
    fn the_scope_is_the_drilled_or_selected_project_and_what_its_links_reach() {
        let rows = [
            row(1, "a", true, &[2, 9]),
            row(2, "b", false, &[1]),
            row(3, "c", false, &[]),
        ];
        let names = scope_names(&rows, None).unwrap();
        assert_eq!(names, HashSet::from(["a".to_owned(), "b".to_owned()]));
        let drilled = scope_names(&rows, Some(3)).unwrap();
        assert_eq!(drilled, HashSet::from(["c".to_owned()]));
        assert!(scope_names(&[row(1, "a", false, &[])], None).is_none());
        assert!(scope_names(&rows, Some(42)).is_none());

        assert!(outside_scope(Some(&names), Some("c")));
        assert!(!outside_scope(Some(&names), Some("a")));
        assert!(
            !outside_scope(Some(&names), None),
            "no project is never outside"
        );
        assert!(!outside_scope(None, Some("c")), "no scope, nothing outside");
    }

    #[test]
    fn the_sparkline_is_named_with_the_pages_wording() {
        assert_eq!(spark_label("calls", 300_000), "calls over the last 5 min");
        assert_eq!(
            spark_label("tokens saved", 900_000),
            "tokens saved over the last 15 min"
        );
    }

    #[test]
    fn folded_targets_count_on_the_nearest_drawn_ancestor_else_on_the_group() {
        let target = |call: &str, t: &str, project: &str| {
            let mut e = end(call, 10, 5);
            e.target = Some(t.into());
            e.project = Some(project.into());
            e
        };
        let mut s = CallsStore::default();
        let mut running = event("run");
        running.target = Some("src/web/mod.rs".into());
        folded(
            &mut s,
            vec![
                target("a", "src/web/calls.rs", "rtok"),
                target("b", "src/web/x/deep.rs", "rtok"),
                target("c", "src/webby/other.rs", "rtok"),
                target("d", "drawn_symbol", "rtok"),
                target("e", "src/web/out.rs", "elsewhere"),
                running,
            ],
            T,
        );
        let drawn = [
            DrawnNode {
                label: "src".into(),
                root: "src".into(),
            },
            DrawnNode {
                label: "web".into(),
                root: "src/web".into(),
            },
            DrawnNode {
                label: "drawn_symbol".into(),
                root: "src/lib.rs".into(),
            },
        ];
        let scope = HashSet::from(["rtok".to_owned()]);
        let got = s.folded_counters(&drawn, 7, Some(&scope), T);
        let pairs: Vec<(&str, u64)> = got.iter().map(|f| (f.label.as_str(), f.calls)).collect();
        // web holds a, b and the running call; `src/webby` is not under `src/web`, so it is
        // under `src`; the drawn symbol and the out-of-scope call count nowhere.
        assert_eq!(pairs, [("web", 3), ("src", 1)]);
        // A target under no drawn root lands on the group, named after the cap's count.
        let mut lone = CallsStore::default();
        folded(&mut lone, vec![target("z", "other/z.rs", "rtok")], T);
        assert_eq!(
            lone.folded_counters(&drawn, 7, None, T),
            [FoldedCount {
                label: "+7 more".into(),
                calls: 1
            }]
        );
        // Nothing was folded without a cap, and calls older than five minutes do not count.
        assert!(s.folded_counters(&drawn, 0, None, T).is_empty());
        assert!(
            s.folded_counters(&drawn, 7, None, T + FOLD_MS + 1)
                .is_empty()
        );
    }

    #[test]
    fn a_start_runs_and_its_end_moves_it_to_the_feed_with_the_measured_tokens() {
        let mut s = CallsStore::default();
        folded(&mut s, vec![event("a")], T);
        assert_eq!(
            s.running.iter().map(|r| &r.call[..]).collect::<Vec<_>>(),
            ["a"]
        );
        folded(&mut s, vec![end("a", 100, 30)], T + 50);
        assert!(s.running.is_empty());
        assert_eq!((s.feed[0].before, s.feed[0].after), (100, 30));
        assert_eq!(s.feed[0].backend.as_deref(), Some("tags"));
        assert_eq!((s.all.calls, s.all.before, s.all.after), (1, 100, 30));
        assert_eq!(
            s.all.tools["callers"],
            ToolTotals {
                calls: 1,
                saved: 70
            }
        );
    }

    /// Fallbacks, caps, symbols, projects and latency percentiles, counted from the events.
    #[test]
    fn metrics_are_counted_from_the_events_and_the_percentiles_are_measured_latencies() {
        let sample = |kind: &str, ref_id: Option<&str>| MeasurementSample {
            id: 1,
            kind: kind.into(),
            before_bytes: 0,
            after_bytes: 0,
            est_before: 0,
            est_after: 0,
            ref_id: ref_id.map(Into::into),
        };
        let timed = |call: &str, ms: f64| {
            let mut e = end(call, 0, 0);
            e.ms = Some(ms);
            e.samples.clear();
            e
        };
        let mut a = timed("a", 10.0);
        a.symbols = Some(2);
        a.total = Some(3);
        (a.symbols_returned, a.files_touched, a.projects_hit) = (Some(1), Some(4), Some(2));
        a.samples = vec![sample("lsp_fallback", None)];
        let mut b = timed("b", 20.0);
        b.symbols = Some(1);
        b.total = Some(1);
        (b.symbols_returned, b.files_touched, b.projects_hit) = (Some(1), Some(2), Some(1));
        b.samples = vec![sample("cap", Some("ab")), sample("explore", None)];
        let events = vec![a, b, timed("c", 30.0), timed("d", 40.0), timed("e", 100.0)];
        let mut s = CallsStore::default();
        folded(&mut s, events, T);
        let t = s.window_totals(0, T);
        assert_eq!(
            (t.fallbacks, t.caps, t.symbols, t.crossed),
            (1, 1, 3, 1),
            "a row with no ref_id is not a cap"
        );
        assert_eq!(
            (t.symbols_returned, t.files_touched, t.projects_hit),
            (2, 6, 3),
            "a call that does not count adds nothing"
        );
        assert_eq!(t.latency(), Some((30.0, 100.0)));
        assert_eq!(s.all.latency(), t.latency());
        assert_eq!(CallsStore::default().all.latency(), None);
    }

    #[test]
    fn the_latencies_kept_are_the_newest_ones() {
        let mut s = CallsStore::default();
        for i in 0..LATENCY_KEEP + 5 {
            let mut e = end(&format!("c{i}"), 0, 0);
            e.ms = Some(i as f64);
            folded(&mut s, vec![e], T + i as i64);
        }
        assert_eq!(s.all.ms.len(), LATENCY_KEEP);
        assert_eq!(s.all.ms[0], 5.0);
    }

    #[test]
    fn a_failed_end_counts_and_keeps_its_error() {
        let mut e = end("a", 0, 0);
        e.ok = false;
        e.error = Some("no backend".into());
        let mut s = CallsStore::default();
        folded(&mut s, vec![e], T);
        assert_eq!(s.all.failed, 1);
        assert_eq!(s.feed[0].error.as_deref(), Some("no backend"));
    }

    #[test]
    fn totals_count_the_events_a_burst_batch_left_out() {
        let mut s = CallsStore::default();
        s.fold(&batch(vec![end("a", 100, 30)], 400), T);
        assert_eq!(s.all.calls, 401);
        assert_eq!(s.all.before, 100 + 4000);
        assert_eq!(s.all.after, 30 + 1600);
    }

    #[test]
    fn the_feed_keeps_the_newest_rows_first_up_to_the_configured_cap() {
        let cfg = Graph {
            live_feed_rows: 5,
            live_heat_window_s: 90,
            ..Graph::default()
        };
        let mut s = CallsStore::with(&cfg);
        for i in 0..8 {
            folded(&mut s, vec![end(&format!("c{i}"), 2, 1)], T + i as i64);
        }
        assert_eq!(s.feed.len(), 5);
        assert_eq!(s.feed[0].call, "c7");
        assert_eq!(s.all.calls, 8);
        assert_eq!(s.heat_window_s, 90);
        assert_eq!(CallsStore::default().feed_rows, 200, "the shipped default");
    }

    #[test]
    fn a_row_carries_the_name_the_batch_gives_its_session_else_the_session() {
        let mut s = CallsStore::default();
        let mut b = batch(vec![event("run"), end("a", 2, 1)], 0);
        b.callers
            .insert("session-abcdef".into(), "claude 3f9a1c2e".into());
        s.fold(&b, T);
        assert_eq!(s.running[0].caller, "claude 3f9a1c2e");
        assert_eq!(s.feed[0].caller, "claude 3f9a1c2e");
        let mut other = end("b", 2, 1);
        other.session = "mcp-1234567890".into();
        folded(&mut s, vec![other], T + 1);
        assert_eq!(s.feed[0].caller, "mcp-1234", "no name: the session's start");
        let by = |caller: &str| FeedFilter {
            caller: caller.into(),
            ..FeedFilter::default()
        };
        assert_eq!(filter_feed(&s.feed, &by("claude 3f9a1c2e")).len(), 1);
    }

    #[test]
    fn each_window_is_summed_from_the_stores_batches() {
        let mut s = CallsStore::default();
        folded(&mut s, vec![end("old", 10, 5)], T);
        folded(&mut s, vec![end("mid", 20, 5)], T + 6 * 60_000);
        folded(&mut s, vec![end("new", 40, 5)], T + 9 * 60_000);
        let now = T + 9 * 60_000 + 1000;
        let before = |w| s.window_totals(w, now).before;
        assert_eq!([0, 1, 2, 3].map(before), [40, 60, 70, 70]);
    }

    #[test]
    fn a_batch_older_than_the_longest_window_leaves_the_buckets_but_not_the_total() {
        let mut s = CallsStore::default();
        folded(&mut s, vec![end("old", 10, 5)], T);
        folded(&mut s, vec![end("new", 40, 5)], T + 16 * 60_000);
        assert_eq!(s.buckets.len(), 1);
        assert_eq!(s.window_totals(3, T + 16 * 60_000).calls, 2);
    }

    #[test]
    fn a_call_with_no_end_after_the_timeout_is_marked_and_stops_running() {
        let mut s = CallsStore::default();
        folded(&mut s, vec![event("a")], T);
        s.sweep(T + INTERRUPT_MS);
        assert_eq!(s.running.len(), 1, "the timeout is exclusive");
        s.sweep(T + INTERRUPT_MS + 1);
        assert!(s.running.is_empty());
        assert!(s.feed[0].interrupted && s.feed[0].call == "a");
        assert_eq!(s.all.calls, 0);
        folded(&mut s, vec![end("a", 10, 5)], T + INTERRUPT_MS + 2);
        let marks: Vec<_> = s
            .feed
            .iter()
            .map(|r| (&r.call[..], r.interrupted))
            .collect();
        assert_eq!(marks, [("a", false)], "a late end replaces the mark");
    }

    #[test]
    fn caller_tool_and_project_narrow_the_rows_and_blank_means_all() {
        let row = |call: &str, session: &str, tool: &str, project: &str| {
            let mut e = end(call, 2, 1);
            e.session = session.into();
            e.tool = tool.into();
            e.project = Some(project.into());
            e
        };
        let mut s = CallsStore::default();
        folded(
            &mut s,
            vec![
                row("a", "x", "callers", "p"),
                row("b", "y", "impact", "p"),
                row("c", "y", "callers", "q"),
            ],
            T,
        );
        let f = |caller: &str, tool: &str| FeedFilter {
            caller: caller.into(),
            tool: tool.into(),
            project: String::new(),
        };
        let calls = |f: &FeedFilter| -> Vec<&str> {
            filter_feed(&s.feed, f)
                .iter()
                .map(|r| &r.call[..])
                .collect()
        };
        assert_eq!(calls(&f("", "")).len(), 3);
        assert_eq!(calls(&f("y", "")), ["c", "b"]);
        assert_eq!(calls(&f("y", "callers")), ["c"]);
        assert_eq!(distinct(&s.feed, |r| r.project.as_deref()), ["p", "q"]);
    }
}
