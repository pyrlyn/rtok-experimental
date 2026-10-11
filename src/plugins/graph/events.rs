// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T329.15: the start, progress and end events of one graph tool call, appended to the store
//! so `rtok web` can stream them from any process. Runs in the MCP server and `rtok mcp --call`,
//! never in a hook. Every write is fail open: a lost event is a missing row on the page, and
//! must not turn a good answer into an error.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use anyhow::Result;
use serde_json::Value;

use crate::plugin::Runtime;
use crate::store::{EventPhase, GraphEvent};
use crate::tokens::Class;

/// The tools whose backends call `tally::hit` for every row they list.
const COUNTED: [&str; 6] = [
    "symbol",
    "callers",
    "impact",
    "outline",
    "explore",
    "graph_diff",
];

static CALLS: AtomicU64 = AtomicU64::new(0);

/// One call being tracked. [`Call::start`] writes the start event; [`Call::end`] writes the
/// end event with the measurement rows the call left behind.
pub struct Call<'a> {
    cx: &'a Runtime,
    key: String,
    tool: String,
    started: Instant,
    /// Newest `measurements` id before the call: what is above it at the end belongs to it.
    mark: i32,
    total: Option<u32>,
    /// Repeated on every event: a page that folds a call into its end still names the call.
    target: Option<String>,
    project: Option<String>,
    symbols: Option<u32>,
}

impl<'a> Call<'a> {
    pub fn start(cx: &'a Runtime, tool: &str, args: &Value) -> Self {
        let key = format!(
            "{}:{}",
            cx.session,
            CALLS.fetch_add(1, Ordering::Relaxed) + 1
        );
        // T329.36: the backends count what they return into this call's record.
        super::tally::arm();
        let call = Self {
            cx,
            key,
            tool: tool.to_string(),
            started: Instant::now(),
            mark: cx.store.measurement_head().unwrap_or(0),
            total: None,
            target: target(args),
            project: project(args),
            symbols: symbols(args),
        };
        call.write(&call.event(EventPhase::Start));
        call
    }

    /// The scope is resolved: `total` projects will be asked. The graph plugin answers a
    /// scope as one block, so this is the one point at which the call reports how far along
    /// it is.
    pub fn progress(&mut self, total: usize) {
        let total = u32::try_from(total).unwrap_or(u32::MAX);
        self.total = Some(total);
        let mut e = self.event(EventPhase::Progress);
        e.done = Some(0);
        e.total = Some(total);
        self.write(&e);
    }

    pub fn end(self, result: &Result<String>) {
        let mut e = self.event(EventPhase::End);
        e.ms = Some(self.started.elapsed().as_secs_f64() * 1000.0);
        e.total = self.total;
        let found = super::tally::take();
        match result {
            Ok(text) => {
                e.done = self.total;
                e.backend = backend(text);
                e.answer_tokens = Some(self.cx.estimate(text, Class::Json));
                // A tool whose backends never tally (`graph_export`) stays NULL: a zero there
                // would read as "found nothing".
                if COUNTED.contains(&self.tool.as_str()) {
                    e.symbols_returned = Some(found.symbols);
                    e.files_touched = Some(found.files);
                    e.projects_hit = Some(found.projects);
                }
            }
            Err(err) => {
                e.ok = false;
                e.error = Some(err.to_string());
            }
        }
        e.samples = self
            .cx
            .store
            .graph_measurements_after(&self.cx.session, self.mark)
            .unwrap_or_default();
        self.write(&e);
    }

    fn event(&self, phase: EventPhase) -> GraphEvent {
        let mut e = GraphEvent::new(&self.key, phase, &self.cx.session, &self.tool);
        e.target.clone_from(&self.target);
        e.project.clone_from(&self.project);
        e.symbols = self.symbols;
        e
    }

    fn write(&self, e: &GraphEvent) {
        let _ = self.cx.store.insert_graph_event(e);
    }
}

/// What the call is about: the symbol, the first of several, the query, the path or the id.
fn target(args: &Value) -> Option<String> {
    let first_name = args["names"]
        .as_array()
        .and_then(|l| l.first())
        .and_then(Value::as_str);
    [
        args["name"].as_str(),
        first_name,
        args["query"].as_str(),
        args["path"].as_str(),
        args["id"].as_str(),
    ]
    .into_iter()
    .flatten()
    .find(|s| !s.is_empty())
    .map(str::to_string)
}

/// Symbols asked for, counted from the arguments so a name the index does not know still
/// counts. `None` when the tool takes none.
fn symbols(args: &Value) -> Option<u32> {
    let named = super::symbol_arg_names(args)
        .iter()
        .filter(|n| !n.is_empty())
        .count();
    let by_id = usize::from(args["id"].as_str().is_some_and(|id| !id.is_empty()));
    // `explore` asks one symbol per identifier of its question.
    let tokens = args["query"]
        .as_str()
        .map_or(0, |q| super::explore_tokens(q).len());
    u32::try_from(named + by_id + tokens)
        .ok()
        .filter(|n| *n > 0)
}

fn project(args: &Value) -> Option<String> {
    match args["project"].as_str().filter(|p| !p.is_empty()) {
        Some(p) => Some(p.to_string()),
        None => crate::project::project_name(&std::env::current_dir().ok()?),
    }
}

/// The header `lsp_or_tags` puts on an answer under `auto`: `(lsp)`, `(tags)` or
/// `(tags; lsp: <reason>)`. An answer without one comes from a fixed mode, so it is unnamed.
fn backend(text: &str) -> Option<String> {
    let head = text.lines().next()?;
    ["lsp", "tags", "text"]
        .into_iter()
        .find(|b| head.strip_prefix('(').is_some_and(|h| h.starts_with(b)))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn target_prefers_the_symbol_then_the_query() {
        assert_eq!(
            target(&json!({"name":"f","query":"q"})).as_deref(),
            Some("f")
        );
        assert_eq!(target(&json!({"names":["a","b"]})).as_deref(), Some("a"));
        assert_eq!(target(&json!({"query":"q"})).as_deref(), Some("q"));
        assert_eq!(target(&json!({"name":""})), None);
    }

    #[test]
    fn symbols_counts_the_names_and_the_id_asked() {
        assert_eq!(symbols(&json!({"names":["a","","b"]})), Some(2));
        assert_eq!(symbols(&json!({"name":"f"})), Some(1));
        assert_eq!(symbols(&json!({"id":"x::f#function@1"})), Some(1));
        assert_eq!(symbols(&json!({"path":"a.rs"})), None);
        // `explore` asks one symbol per identifier of its question.
        assert_eq!(symbols(&json!({"query":"how does f call g, f"})), Some(5));
        assert_eq!(symbols(&json!({"query":"?"})), None);
    }

    #[test]
    fn backend_reads_the_answer_header() {
        assert_eq!(backend("(lsp)\nx").as_deref(), Some("lsp"));
        assert_eq!(backend("(tags; lsp: down)\nx").as_deref(), Some("tags"));
        assert_eq!(backend("a.rs:1 f").as_deref(), None);
        assert_eq!(backend(""), None);
    }

    /// The events a second process writes: start, progress and end, the end carrying the
    /// `graph` rows the call wrote and not another session's.
    #[test]
    fn a_call_writes_three_events_with_its_own_rows() {
        let cx = Runtime::in_memory("mcp-1").unwrap();
        let mut call = Call::start(&cx, "symbol", &json!({"names":["f","g"]}));
        call.progress(2);
        let row = |kind| rtok_plugin_sdk::Measurement {
            plugin: "graph",
            kind,
            before_bytes: 40,
            after_bytes: 10,
            est_before: 12,
            est_after: 3,
            ref_id: Some("ab".into()),
            call_id: None,
        };
        cx.record(&row("cap")).unwrap();
        cx.store.insert_measurement("mcp-2", &row("cap")).unwrap();
        call.end(&Ok("(lsp)\na.rs:1 f".to_string()));
        let ev = cx.store.graph_events_after(0, 10).unwrap();
        let phases: Vec<_> = ev.iter().map(|e| e.phase).collect();
        assert_eq!(
            phases,
            [EventPhase::Start, EventPhase::Progress, EventPhase::End]
        );
        let end = &ev[2];
        assert_eq!(end.call, ev[0].call);
        assert_eq!(end.backend.as_deref(), Some("lsp"));
        assert_eq!((end.done, end.total), (Some(2), Some(2)));
        assert_eq!(end.samples.len(), 1);
        assert_eq!(end.samples[0].est_before, 12);
        assert_eq!(end.samples[0].ref_id.as_deref(), Some("ab"));
        assert!(
            ev.iter().all(|e| e.symbols == Some(2)),
            "every event says what was asked"
        );
        assert_eq!(ev[0].target.as_deref(), Some("f"));
        assert_eq!(
            end.target, ev[0].target,
            "a folded call still names its symbol"
        );
    }

    /// What the backends tally during the call lands on its end event, for a call about named
    /// symbols only; a call about a query or a path stays NULL instead of reading as zero.
    #[test]
    fn the_end_event_carries_what_the_backends_counted() {
        let cx = Runtime::in_memory("mcp-1").unwrap();
        let root = std::path::Path::new("/p");
        let call = Call::start(&cx, "callers", &json!({"name":"f"}));
        super::super::tally::hit(root, "f", ["a.rs", "b.rs"]);
        call.end(&Ok("a.rs ×1\nb.rs ×1\n".to_string()));
        // A tool that never tallies stays NULL; one that tallies and finds nothing is zero.
        let call = Call::start(&cx, "graph_export", &json!({}));
        super::super::tally::hit(root, "f", ["a.rs"]);
        call.end(&Ok("x".to_string()));
        let call = Call::start(&cx, "explore", &json!({"query":"f"}));
        call.end(&Ok("no symbols resolved".to_string()));
        let ends: Vec<_> = cx
            .store
            .graph_events_after(0, 10)
            .unwrap()
            .into_iter()
            .filter(|e| e.phase == EventPhase::End)
            .collect();
        let counts = |e: &GraphEvent| (e.symbols_returned, e.files_touched, e.projects_hit);
        assert_eq!(counts(&ends[0]), (Some(1), Some(2), Some(1)));
        assert_eq!(counts(&ends[1]), (None, None, None));
        assert_eq!(counts(&ends[2]), (Some(0), Some(0), Some(0)));
    }

    /// The page's counters are read off the events, so they have to equal what the ledger
    /// (`rtok stats`) holds for the same call: one `lsp_fallback` row and one cut answer.
    #[test]
    fn the_summary_counts_the_rows_the_ledger_holds_for_the_call() {
        let cx = Runtime::in_memory("mcp-1").unwrap();
        let mut call = Call::start(&cx, "callers", &json!({"name":"f"}));
        call.progress(2);
        let row = |kind, ref_id: Option<&str>| rtok_plugin_sdk::Measurement {
            plugin: "graph",
            kind,
            before_bytes: 40,
            after_bytes: 10,
            est_before: 12,
            est_after: 3,
            ref_id: ref_id.map(Into::into),
            call_id: None,
        };
        cx.record(&row("lsp_fallback", None)).unwrap();
        cx.record(&row("cap", Some("ab"))).unwrap();
        cx.record(&row("tags.callers", None)).unwrap();
        call.end(&Ok("(tags; lsp: down)\na.rs:1 f".to_string()));
        let events = cx.store.graph_events_after(0, 10).unwrap();
        let summary = crate::web::live::coalesce(events, 100).unwrap().summary;
        let ledger = cx.store.graph_measurements_after("mcp-1", 0).unwrap();
        let of = |kind: &str| ledger.iter().filter(|r| r.kind == kind).count() as u32;
        assert_eq!(
            (summary.fallbacks, summary.caps),
            (of("lsp_fallback"), of("cap"))
        );
        assert_eq!(
            (summary.fallbacks, summary.caps, summary.symbols),
            (1, 1, 1)
        );
        assert_eq!(summary.crossed, 1, "a scope of two projects crosses");
        let tokens = |f: fn(&crate::store::MeasurementSample) -> i32| -> i64 {
            ledger.iter().map(|r| i64::from(f(r))).sum()
        };
        assert_eq!(summary.est_before, tokens(|r| r.est_before));
        assert_eq!(summary.est_after, tokens(|r| r.est_after));
    }

    #[test]
    fn a_failed_call_ends_with_its_error() {
        let cx = Runtime::in_memory("mcp-1").unwrap();
        Call::start(&cx, "impact", &json!({})).end(&Err(anyhow::anyhow!("no backend")));
        let end = cx.store.graph_events_after(0, 10).unwrap().pop().unwrap();
        assert!(!end.ok);
        assert_eq!(end.error.as_deref(), Some("no backend"));
        assert!(end.samples.is_empty());
    }
}
