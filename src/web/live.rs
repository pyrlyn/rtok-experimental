// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T329.15: graph call events on `/ws`. Any rtok process appends events to the store
//! (`plugins::graph::events`); this module is the one reader in `rtok web`. A poller runs only
//! while a socket is subscribed, reads `id > cursor` every [`POLL`], folds what it found into
//! one [`CallBatch`], adds that to its [`CallsStore`] and publishes the store's [`CallsView`]
//! when it changed (T484), so the work and the traffic per second are bounded however many calls
//! arrive and every page shows the totals `rtok tui` computes.
//!
//! [`Reader`] is that one reader without the async: `rtok tui` drives it from a thread, so both
//! surfaces fold the same rows through the same cursor and [`coalesce`].
//!
//! The totals cover the events written since `rtok web` started (T329.34): a poller that starts
//! (the first socket, or the first one after a page reload) replays the store's events above the
//! id the server noted at its start, so a reload shows the same figures as the page it replaced.
//! The table keeps the newest 5000 rows, so an older call is gone from the store and from the
//! totals. A socket that subscribes while a poller runs starts from the totals so far.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use schemars::JsonSchema;
use serde::Serialize;
use tokio::sync::watch;

use super::calls_store::{CallsStore, clock_ms};
use super::calls_view::CallsView;
use super::protocol::ServerFrame;
use crate::config::Config;
use crate::config::Graph;
use crate::store::{AgentDetail, EventPhase, GraphEvent, Store};

/// How often the store is asked for new events; with the write and the render this keeps a
/// call in another process well inside the second the page promises.
pub const POLL: Duration = Duration::from_millis(250);
/// Events read per poll. A burst larger than this drains over the next polls.
const FETCH: i64 = 2000;

/// `[plugins.graph] live_max_events_per_s` as the events one poll lists; the rest of a burst is
/// counted in `omitted` and in `summary`.
pub fn events_per_poll(cfg: &Graph) -> usize {
    let per_poll = u64::from(cfg.live_max_events_per_s) * POLL.as_millis() as u64;
    usize::try_from(per_poll.div_ceil(1000))
        .unwrap_or(usize::MAX)
        .max(1)
}

/// What one poll found, as one frame. `summary` counts every event of the poll (including
/// the ones left out of `events`), so a page that adds summaries up never undercounts a burst.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct CallBatch {
    /// Newest last. A call's start and progress events are left out when its end is in the
    /// batch, since the end carries the same call and tool.
    pub events: Vec<GraphEvent>,
    /// Events cut because the batch held more than the poll's cap (the oldest go first).
    pub omitted: u32,
    pub summary: CallSummary,
    /// The newest event id the batch covers.
    pub head: i64,
    /// What the store calls the sessions of `events` (T329.34); a session it knows nothing about
    /// is absent.
    pub callers: BTreeMap<String, String>,
}

impl CallBatch {
    /// The caller column of `session`: the store's name, else the session id.
    pub fn caller(&self, session: &str) -> String {
        self.callers
            .get(session)
            .cloned()
            .unwrap_or_else(|| super::calls_store::bare_caller(session))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CallSummary {
    pub starts: u32,
    pub ends: u32,
    pub failed: u32,
    /// Sums of the `samples` of the end events: the `Measurement` columns `rtok stats` adds up.
    pub est_before: i64,
    pub est_after: i64,
    /// `lsp_fallback` rows of the ended calls: each is one answer the tags index gave after the
    /// language server could not.
    pub fallbacks: u32,
    /// Rows with a `ref_id`: answers cut at `max_tokens`.
    pub caps: u32,
    /// Symbols the ended calls asked for.
    pub symbols: u32,
    /// Ended calls whose scope held more than one project.
    pub crossed: u32,
    /// What the ended calls returned, counted by the graph backends (T329.36): asked symbols
    /// the answers list, distinct files per call and projects with a row per call.
    pub symbols_returned: u32,
    pub files_touched: u32,
    pub projects_hit: u32,
}

/// Folds `events` (oldest first) into a frame listing at most `cap` of them, or `None` when
/// there are none.
pub fn coalesce(events: Vec<GraphEvent>, cap: usize) -> Option<CallBatch> {
    let head = events.last()?.id;
    let mut summary = CallSummary::default();
    let ended: HashSet<String> = events
        .iter()
        .filter(|e| e.phase == EventPhase::End)
        .map(|e| e.call.clone())
        .collect();
    for e in &events {
        match e.phase {
            EventPhase::Start => summary.starts += 1,
            EventPhase::Progress => {}
            EventPhase::End => {
                summary.ends += 1;
                summary.failed += u32::from(!e.ok);
                summary.symbols += e.symbols.unwrap_or(0);
                summary.crossed += u32::from(e.total.is_some_and(|t| t > 1));
                summary.symbols_returned += e.symbols_returned.unwrap_or(0);
                summary.files_touched += e.files_touched.unwrap_or(0);
                summary.projects_hit += e.projects_hit.unwrap_or(0);
                for s in &e.samples {
                    summary.est_before += i64::from(s.est_before);
                    summary.est_after += i64::from(s.est_after);
                    summary.fallbacks += u32::from(s.kind == "lsp_fallback");
                    summary.caps += u32::from(s.ref_id.is_some());
                }
            }
        }
    }
    let mut kept: Vec<GraphEvent> = events
        .into_iter()
        .filter(|e| e.phase == EventPhase::End || !ended.contains(&e.call))
        .collect();
    let omitted = kept.len().saturating_sub(cap);
    kept.drain(..omitted);
    Some(CallBatch {
        events: kept,
        omitted: u32::try_from(omitted).unwrap_or(u32::MAX),
        summary,
        head,
        callers: BTreeMap::new(),
    })
}

/// The poller and its fan-out. One per [`super::DashState`].
pub struct LiveCalls {
    /// The newest `calls` frame as JSON. A watch, not a queue: every socket reads the current
    /// state first and then each change, and one that is slow skips frames instead of lagging.
    tx: watch::Sender<Arc<str>>,
    running: AtomicBool,
    /// The newest event id when `rtok web` started: "since started" counts the events above it.
    /// `None` when the store would not open, which leaves the totals starting at the poller.
    started: Option<i64>,
}

fn frame_of(view: CallsView) -> Arc<str> {
    Arc::from(ServerFrame::Calls { calls: view }.to_json())
}

impl LiveCalls {
    pub(super) fn new(cfg: &Config) -> Self {
        let idle = frame_of(CallsView::of(
            &CallsStore::with(&cfg.plugins.graph),
            clock_ms(),
        ));
        Self {
            tx: watch::channel(idle).0,
            running: AtomicBool::new(false),
            started: head_of(&cfg.core.db_path),
        }
    }

    /// The receiver and the frame to send first: the totals so far. The first subscriber starts
    /// the poller, which first folds what the store holds since `rtok web` started; it stops
    /// when the last receiver is dropped. Must run inside the tokio runtime.
    pub async fn subscribe(
        self: &Arc<Self>,
        cfg: &Config,
    ) -> (watch::Receiver<Arc<str>>, Arc<str>) {
        // Receiver first: a frame the poller sends while the store opens is not lost.
        let mut rx = self.tx.subscribe();
        let db_path = cfg.core.db_path.clone();
        let graph = cfg.plugins.graph.clone();
        if !self.running.swap(true, Ordering::AcqRel) {
            let (reader, store) = replayed(&db_path, &graph, self.started).await;
            self.tx
                .send_replace(frame_of(CallsView::of(&store, clock_ms())));
            tokio::spawn(poll(self.clone(), db_path, graph, reader, store));
        }
        let first = rx.borrow_and_update().clone();
        (rx, first)
    }
}

/// The newest event id in the store at `path`, or `None` when it will not open: where "since
/// this surface started" begins.
pub fn head_of(path: &std::path::Path) -> Option<i64> {
    Store::open(path).ok()?.graph_event_head().ok()
}

/// A store handle and a cursor: where a reader that wants only new events starts.
pub struct Reader {
    store: Store,
    cursor: i64,
    /// Events one poll lists.
    cap: usize,
    /// Sessions the store gave an agent: that name does not change, so it is asked once.
    named: HashMap<String, String>,
}

/// T329.34: `claude 3f9a1c2e` for a session an agent row belongs to (`rtok agents list`'s id and
/// host), `claude mcp-4242` for one the store only knows by its host (an `rtok mcp` process
/// has a `sessions` row but no agent row of its own). `None`, with `fixed` false, when it
/// knows neither: the column then shows the session id. A name with no agent can still gain one
/// when a hook registers later, so only `fixed` names are remembered.
fn caller_name(store: &Store, session: &str, agents: &[AgentDetail]) -> Option<(String, bool)> {
    if let Some(a) = agents
        .iter()
        .find(|a| a.host_session_id == session && a.parent_id.is_none())
    {
        return Some((format!("{} {}", a.host, a.short), true));
    }
    let (host, ..) = store.session_row(session).ok().flatten()?;
    let tail: String = session.chars().take(12).collect();
    Some((format!("{} {tail}", host?), false))
}

impl Reader {
    /// `None` when the file will not open; callers try again on their next pass, so a missing
    /// or locked store costs a retry and never a failed socket.
    pub fn open(path: &std::path::Path, cfg: &Graph) -> Option<Self> {
        let store = Store::open(path).ok()?;
        let cursor = store.graph_event_head().ok()?;
        Some(Self {
            store,
            cursor,
            cap: events_per_poll(cfg),
            named: HashMap::new(),
        })
    }

    /// What was written since the last call, folded; `None` when nothing was, or the read
    /// failed (the cursor stays, so the events come on the next try).
    pub fn poll(&mut self) -> Option<CallBatch> {
        let events = self.store.graph_events_after(self.cursor, FETCH).ok()?;
        let batch = self.batch_of(events)?;
        self.cursor = batch.head;
        Some(batch)
    }

    /// The events above `since` up to where this reader opened, as batches of at most one
    /// poll's cap with the time of their newest event, so a surface that starts later folds the
    /// calls made since `since` into the same totals and windows a live one has. Chunks are as
    /// big as the cap, so each lists all its calls and the per-tool bars stay exact.
    pub fn replay(&mut self, since: i64) -> Vec<(CallBatch, i64)> {
        let (mut at, end) = (since, self.cursor);
        let mut out = Vec::new();
        while at < end {
            let Ok(mut events) = self.store.graph_events_after(at, self.cap as i64) else {
                break;
            };
            events.retain(|e| e.id <= end);
            let Some(ts) = events.last().map(|e| e.ts_ms) else {
                break;
            };
            let Some(batch) = self.batch_of(events) else {
                break;
            };
            at = batch.head;
            out.push((batch, ts));
        }
        out
    }

    fn batch_of(&mut self, events: Vec<GraphEvent>) -> Option<CallBatch> {
        let mut batch = coalesce(events, self.cap)?;
        batch.callers = self.names_of(&batch.events);
        Some(batch)
    }

    fn names_of(&mut self, events: &[GraphEvent]) -> BTreeMap<String, String> {
        let sessions: BTreeSet<&str> = events.iter().map(|e| e.session.as_str()).collect();
        let asked: Vec<String> = sessions
            .iter()
            .filter(|s| !self.named.contains_key(**s))
            .map(|s| (*s).to_owned())
            .collect();
        let agents = self.store.agents_of_sessions(&asked).unwrap_or_default();
        let mut names = BTreeMap::new();
        for s in sessions {
            if let Some(n) = self.named.get(s) {
                names.insert(s.to_owned(), n.clone());
            } else if let Some((n, fixed)) = caller_name(&self.store, s, &agents) {
                if fixed {
                    self.named.insert(s.to_owned(), n.clone());
                }
                names.insert(s.to_owned(), n);
            }
        }
        names
    }
}

/// The reader, opened at the newest event, and a store holding what the events since `started`
/// add up to (empty without a `started` or a reader).
async fn replayed(
    path: &std::path::Path,
    graph: &Graph,
    started: Option<i64>,
) -> (Option<Reader>, CallsStore) {
    let (path, graph) = (path.to_owned(), graph.clone());
    tokio::task::spawn_blocking(move || {
        let mut store = CallsStore::with(&graph);
        let mut reader = Reader::open(&path, &graph);
        if let (Some(r), Some(since)) = (reader.as_mut(), started) {
            for (batch, at) in r.replay(since) {
                store.fold(&batch, at);
            }
            store.sweep(clock_ms());
        }
        (reader, store)
    })
    .await
    .unwrap_or_else(|_| (None, CallsStore::with(&Graph::default())))
}

async fn open_reader(path: std::path::PathBuf, graph: Graph) -> Option<Reader> {
    tokio::task::spawn_blocking(move || Reader::open(&path, &graph))
        .await
        .ok()
        .flatten()
}

async fn poll(
    live: Arc<LiveCalls>,
    db_path: std::path::PathBuf,
    graph: Graph,
    mut reader: Option<Reader>,
    mut store: CallsStore,
) {
    let mut sent = CallsView::of(&store, clock_ms());
    loop {
        tokio::time::sleep(POLL).await;
        if live.tx.receiver_count() == 0 {
            live.running.store(false, Ordering::Release);
            // A socket that subscribed between the check and the store would find the flag
            // still set and start no poller; take the role back if one did.
            if live.tx.receiver_count() == 0 || live.running.swap(true, Ordering::AcqRel) {
                return;
            }
        }
        let mut r = match reader.take() {
            Some(r) => r,
            None => match open_reader(db_path.clone(), graph.clone()).await {
                Some(r) => r,
                None => continue,
            },
        };
        // A failed join drops the reader; the next pass opens a fresh one.
        let Ok((r, batch)) = tokio::task::spawn_blocking(move || {
            let batch = r.poll();
            (r, batch)
        })
        .await
        else {
            continue;
        };
        reader = Some(r);
        let now = clock_ms();
        if let Some(batch) = batch {
            store.fold(&batch, now);
        }
        store.sweep(now);
        let view = CallsView::of(&store, now);
        // The clock moving is not a change: the page keeps time between frames.
        sent.now = now;
        if view != sent {
            live.tx.send_replace(frame_of(view.clone()));
            sent = view;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MeasurementSample;

    const CAP: usize = 100;

    fn ev(id: i64, call: &str, phase: EventPhase) -> GraphEvent {
        let mut e = GraphEvent::new(call, phase, "mcp-1", "callers");
        e.id = id;
        e
    }

    #[test]
    fn nothing_found_sends_nothing() {
        assert!(coalesce(Vec::new(), CAP).is_none());
    }

    #[test]
    fn an_ended_call_is_listed_once_and_still_counted() {
        let mut end = ev(3, "a", EventPhase::End);
        let row = |kind: &str, before, after, ref_id: Option<&str>| MeasurementSample {
            id: 1,
            kind: kind.into(),
            before_bytes: 9,
            after_bytes: 3,
            est_before: before,
            est_after: after,
            ref_id: ref_id.map(Into::into),
        };
        end.samples = vec![
            row("cap", 10, 4, Some("ab")),
            row("lsp_fallback", 0, 0, None),
        ];
        end.symbols = Some(3);
        end.symbols_returned = Some(2);
        end.files_touched = Some(4);
        end.projects_hit = Some(2);
        end.total = Some(2);
        let b = coalesce(
            vec![
                ev(1, "a", EventPhase::Start),
                ev(2, "a", EventPhase::Progress),
                end,
                ev(4, "b", EventPhase::Start),
            ],
            CAP,
        )
        .unwrap();
        assert_eq!(
            b.events.iter().map(|e| e.id).collect::<Vec<_>>(),
            [3, 4],
            "a's start and progress are folded into its end; b still runs"
        );
        assert_eq!(b.head, 4);
        assert_eq!(b.omitted, 0);
        assert_eq!(
            b.summary,
            CallSummary {
                starts: 2,
                ends: 1,
                failed: 0,
                est_before: 10,
                est_after: 4,
                fallbacks: 1,
                caps: 1,
                symbols: 3,
                crossed: 1,
                symbols_returned: 2,
                files_touched: 4,
                projects_hit: 2,
            }
        );
    }

    #[test]
    fn a_burst_is_capped_but_its_summary_counts_every_call() {
        let mut events = Vec::new();
        let mut id = 0;
        for n in 0..500 {
            let call = format!("c{n}");
            for phase in [EventPhase::Start, EventPhase::Progress, EventPhase::End] {
                id += 1;
                let mut e = ev(id, &call, phase);
                if phase == EventPhase::End {
                    e.ok = n % 10 != 0;
                }
                events.push(e);
            }
        }
        let b = coalesce(events, CAP).unwrap();
        assert_eq!(b.events.len(), CAP);
        assert_eq!(b.omitted, 400);
        assert_eq!(b.events.last().unwrap().call, "c499", "the newest survive");
        assert_eq!(
            (b.summary.starts, b.summary.ends, b.summary.failed),
            (500, 500, 50)
        );
    }
}
