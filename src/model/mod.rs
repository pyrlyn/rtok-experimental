// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The one operator model behind `rtok web` and `rtok tui` (D23, T15.0).
//!
//! Both surfaces render *these* values; neither owns data. Since T15.11 the reading
//! commands are renderers of the same pages (D27): this module is the only place either
//! surface *or* a reading command touches `Store` / `stats` / `doctor`, so a page cannot
//! grow a query of its own and two windows cannot disagree about the same session.

mod agents;
pub use agents::{
    AgentState, AgentView, SessionView, agent_sessions, agent_show, session_views, set_status,
};

mod tasks;
pub use tasks::{TaskBoard, TaskCard, TaskColumn, TaskLink, TasksPage, TasksState};

use anyhow::Result;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::agents::junk_web::JunkCard;
use crate::agents::usage;
use crate::config::{Config, layers};
use crate::demon::{self, Service};
use crate::doctor;
use crate::measure::{cache, stats};
use crate::plugins::Registry;
use crate::store::{CallRow, MeasurementTotal, SessionTotals, Store};

/// Everything a surface needs for one refresh. `Default` is the empty frame a surface
/// paints while its first read is still running.
#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct Snapshot {
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// Overview page: provider usage totals, CTT and the per-turn series.
    pub usage: Overview,
    /// Plugins page: one entry per catalogue plugin.
    pub plugins: Vec<PluginPage>,
    /// Calls page (T15.5): the last [`CALLS_ROWS`] ledger rows, newest first.
    pub calls: Vec<CallRow>,
    /// Sessions page (T25.1): one row per session, newest first.
    pub sessions: Vec<SessionTotals>,
    /// Doctor page (T15.6): what `rtok doctor` reports — hooks, MCP servers, proxy
    /// chains. `None` when this tick's probes failed; the page renders the failure
    /// rather than zeros, the way an unreadable store renders an empty page.
    pub doctor: Option<doctor::Report>,
    /// Logs page (T15.7): the last `[log] lines` log lines, newest first — the same
    /// selection `rtok logs` screens ([`Model::log_lines`], T15.11). Riding the snapshot
    /// makes the page both surfaces' (D23); `[log] lines` is the frame's bound too.
    pub logs: Vec<String>,
    /// Set when the store will not open or this tick's doctor probe failed (T60.6). Both
    /// surfaces render it as a banner instead of an empty page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Skills page (T63.1): T61.3 listing joined to T61.1 resident/invocations.
    pub skills: SkillsPage,
    /// Archive ids keyed by `calls[].id` (T60.4). Both surfaces read this map; neither
    /// queries the store for an expand handle (D23 / D27).
    #[serde(default)]
    pub ref_ids: BTreeMap<i32, String>,
    /// Stats page (T227): `stats --price`'s table plus `stats --cache`'s table, folded
    /// into one page (D27) — [`stats_page_text`], built from the same transcript scan
    /// [`stats_skills`] already runs for the Skills page (no second aggregation). `None`
    /// when this tick's transcripts read failed.
    pub stats: Option<String>,
    /// Graph page (T230): `graph status`'s index health (rows, files, the T68.3
    /// pending/staleness set, `indexed_at`) plus `graph dead`'s unreferenced-definition
    /// list, folded into one page (D27) — [`graph_page_text`], read from the store on
    /// this tick (no re-indexing). `None` when the `graph` feature is off or the store
    /// read failed.
    pub graph: Option<String>,
    /// Project registry (T329.12): every registered project with its index state and links,
    /// the same rows `rtok graph projects --json` prints. `None` when the `graph` feature is
    /// off or the store read failed.
    pub projects: Option<Vec<ProjectRow>>,
    /// Hosts page (T231): `agents list`'s blocks — kind, detected version, installed
    /// surfaces, config path — one per known host variant (D27), so `agents list` /
    /// `agents info` can join `COMMAND_PAGES`. [`hosts_page_text`] reuses the same
    /// probe `agents_list`'s JSON form calls (T168's `--version` noise filter and all)
    /// behind a cache: never blocks a 2 s tick on a cold or stale probe — the tick
    /// renders the last known text, or "probing hosts…" before the first one lands.
    pub hosts: String,
    /// The Hosts page's junk card (T330.7): the numbers of the `junk` section of `hosts`, as data
    /// ([`junk_page`] reads the one cached report). `None` while the first measurement runs.
    pub junk: Option<JunkCard>,
    /// Config page (T228): `rtok config show --sources`'s rows, through
    /// [`config_page_text`] (D27, no second layering). `None` on a failed tick.
    pub config: Option<String>,
    /// Services page (T229): `demon status`'s per-service rows plus `otel status`'s
    /// exporter health, through [`services_page_text`] (D27, no second reader —
    /// [`Model::demon`] and [`otel_status`] already build both). `None` on a failed
    /// tick.
    pub services: Option<String>,
    /// Worktrees page (T232): `worktree list`'s rows — path, branch, owner, age,
    /// `target/` size and state — through [`worktrees_page_text`], the same
    /// [`crate::worktree::list::rows`]/[`crate::worktree::list::to_table`] `worktree
    /// list` already calls (D27, no second reader or directory walk); `gc`/`clean`
    /// stay CLI-only verdicts. `None` only when the current directory is unreadable.
    pub worktrees: Option<String>,
    /// Usage page (T358.5): what `rtok agents usage` reports, through [`usage_page`]. The
    /// overview's own `usage` key is the proxy's totals, so this one carries the page's name
    /// in its own words.
    pub agent_usage: UsagePage,
    /// Tasks page (T441.13.7): the project's `rtok task list --all` as a board, through
    /// [`tasks::page`]. Read-only; the slow adapters are read off the tick.
    pub tasks: TasksPage,
}

/// The Usage page: one [`usage::Report`] — the call `rtok agents usage` makes — read once and
/// carried twice. `text` is that command's screen ([`usage::Report::to_text`]) for the tui and
/// the SPA's status line; `report` is the same rows as data for the SPA's tables. Nothing is summed
/// a second time (D27). Both are empty-handed while the first read runs or after it fails:
/// `report` is `None` and `text` says why.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct UsagePage {
    pub text: String,
    pub report: Option<usage::Report>,
}

/// The shared stats widget: `usage` rows for the overview, `Measurement` rows per plugin.
#[derive(Debug, Default, Clone, Copy, Serialize, JsonSchema)]
pub struct Stats {
    pub input: i64,
    pub output: i64,
    pub cache_create: i64,
    pub cache_read: i64,
    pub est_before: i64,
    pub est_after: i64,
    pub rows: u64,
}

/// One day of the Δtok savings trend (T414.13).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SavingsDay {
    /// `YYYY-MM-DD` in `[agents.usage] tz`, so the trend and the Usage page cut the same days.
    pub day: String,
    /// `Measurement` rows stamped that day.
    pub rows: u64,
    /// Σ est_before − est_after; `None` when the day has no rows. No row, no saving claim: an
    /// empty day is a gap on the chart, never a zero.
    pub saved: Option<i64>,
    /// The same per plugin, only the plugins with rows that day. An `expand` row nets negative,
    /// as on the Plugins page.
    pub plugins: BTreeMap<String, i64>,
}

/// Days the Δtok trend covers: two weeks of day bars stay readable at a panel's width and keep
/// the 2 s frame small.
pub const SAVINGS_DAYS: usize = 14;

/// The Overview page (T15.3): the usage totals plus what the tab draws from them —
/// context-token-turns and the per-turn series behind the sparkline. The totals stay
/// flat under the `usage` key, so the `/ws` frame keeps the shape P19 pinned and the
/// SPA reads on untouched.
#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct Overview {
    #[serde(flatten)]
    pub totals: Stats,
    /// Σ over sessions of Σ over turns `ctx × turns-after`, where `ctx` is the turn's
    /// input-side tokens (`input + cache_create + cache_read`) and `turns-after` counts
    /// the session's later turns — the usage-row mirror of `stats`' `tokens × remain`.
    /// Output is left out on purpose: it re-enters as a later turn's input or cache,
    /// so counting it again would count it twice.
    pub ctt: i64,
    /// Per-turn `ctx`, sessions oldest-first and request order within a session, kept
    /// to the last [`OVERVIEW_TURNS`]. A `usage` row carries no timestamp, so across
    /// sessions this is session order, not wall-clock order.
    pub turns: Vec<i64>,
    /// Persistent operator warnings for the whole disabled period (e.g. proxy plain
    /// mode). Empty when none; omitted from the wire when empty so the P19 shape stays.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alerts: Vec<String>,
    /// The Δtok trend Overview and Stats draw: [`SAVINGS_DAYS`] days ending today, oldest
    /// first, from [`savings_trend`]. Empty while the first read runs or when the store will
    /// not read.
    pub savings: Vec<SavingsDay>,
}

/// Points of the Overview sparkline: wider than any terminal the TUI draws on, and
/// small enough that the `/ws` frame stays cheap at one snapshot per 2 s tick.
pub const OVERVIEW_TURNS: usize = 120;

/// Rows the Calls page rides (T15.5): the same budget as [`OVERVIEW_TURNS`] — more
/// rows than a terminal shows at once, and the `/ws` frame stays cheap at one
/// snapshot per 2 s tick.
pub const CALLS_ROWS: usize = 120;

/// Sessions the Sessions page rides (T25.1): the same budget as [`CALLS_ROWS`]. The
/// page is read on every 2 s tick, so it aggregates the newest sessions, not every
/// session the store ever recorded.
pub const SESSIONS_ROWS: usize = 120;

/// A plugin's page: its manifest, the static copy it contributes through
/// `Plugin::dashboard_page`, and the stats widget when it saves tokens.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PluginPage {
    pub id: &'static str,
    pub enabled: bool,
    pub surfaces: Vec<&'static str>,
    pub title: String,
    pub summary: String,
    pub saves_tokens: bool,
    pub fields: Vec<(String, String)>,
    pub stats: Option<Stats>,
}

/// One `rtok agents list` row: the same host variant `agents::list` prints.
#[derive(Debug, Serialize)]
pub struct AgentListRow {
    pub host: &'static str,
    pub kind: &'static str,
    pub name: &'static str,
    pub present: bool,
    pub app: Option<String>,
    pub version: Option<String>,
    pub config: Vec<String>,
    pub modules: Vec<crate::agents::ModuleRow>,
    /// T278: the `mcp` module per surface — `surface`, `entry`, `plugin`.
    pub mcp: Vec<crate::agents::mcp::McpRow>,
    pub plugins: Vec<crate::agents::PluginRow>,
    /// T382: the installed rtok plugin's version; absent when none is installed or no version
    /// is recorded anywhere.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_version: Option<String>,
    /// T382: where that plugin came from (`github`, `local` or `marketplace`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_source: Option<String>,
}

/// Skills page (T63.1, D23): one row per skill the host lists.
#[derive(Debug, Default, Clone, Serialize, JsonSchema)]
pub struct SkillsPage {
    /// Totals: listed, desc bytes ≈ tokens/req (chars/4, research.md §10.2), resident, input share.
    pub header: String,
    pub rows: Vec<SkillPageRow>,
}

#[derive(Debug, Default, Clone, Serialize, JsonSchema)]
pub struct SkillPageRow {
    pub name: String,
    pub source: String,
    pub desc_chars: usize,
    pub body_bytes: u64,
    pub invocations: u64,
    pub resident: u64,
    pub last_invoked: String,
    pub never: bool,
}

/// `rtok otel status` as data: endpoint, watermarks, pending rows, last exporter line.
#[derive(Debug, Serialize)]
pub struct OtelStatus {
    pub endpoint: Option<String>,
    pub calls_mark: i64,
    pub calls_pending: i64,
    pub logs_mark: i64,
    pub logs_pending: i64,
    pub sessions_mark: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<OtelLastLog>,
}

#[derive(Debug, Serialize)]
pub struct OtelLastLog {
    pub level: String,
    pub name: String,
    pub message: String,
}

/// Memory plugin status for `rtok memory status` and `--json` (T69.4).
#[derive(Debug, Serialize)]
pub struct MemoryStatus {
    pub since: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub notes: MemoryNotesTotals,
    pub recall: MemoryRecallTotals,
    pub calls: MemoryMcpCalls,
    pub by_project: Vec<MemoryProjectBlock>,
}

#[derive(Debug, Serialize)]
pub struct MemoryNotesTotals {
    pub live: u64,
    pub pinned: u64,
    pub retired: u64,
    pub body_bytes: i64,
}

#[derive(Debug, Serialize)]
pub struct MemoryRecallTotals {
    pub recalls: u64,
    pub stood_for_bytes: i64,
    pub injected_bytes: i64,
}

#[derive(Debug, Serialize)]
pub struct MemoryMcpCalls {
    pub mem_search: u64,
    pub mem_get: u64,
}

#[derive(Debug, Serialize)]
pub struct MemoryProjectBlock {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub kinds: Vec<MemoryKindAgg>,
}

#[derive(Debug, Serialize)]
pub struct MemoryKindAgg {
    pub kind: String,
    pub live: u64,
    pub pinned: u64,
    pub retired: u64,
    pub body_bytes: i64,
    pub oldest_ts: i64,
    pub newest_ts: i64,
}

/// One `rtok memory status` snapshot — the same type `--json` prints (T69.4).
pub fn memory_status(
    cfg: &Config,
    project: Option<&str>,
    since: Option<&str>,
) -> Result<MemoryStatus> {
    let since_label = since.unwrap_or(&cfg.stats.since);
    let span = match since {
        Some(flag) => stats::parse_since(flag)?,
        None => stats::parse_since_from(&cfg.stats.since, "stats.since")?,
    };
    let since_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
        - i64::try_from(span.as_secs()).unwrap_or(i64::MAX);
    let store = Store::open(&cfg.core.db_path)?;
    let aggs = store.memory_note_aggs(project)?;
    let mut notes = MemoryNotesTotals {
        live: 0,
        pinned: 0,
        retired: 0,
        body_bytes: 0,
    };
    let mut by_project: BTreeMap<Option<String>, Vec<MemoryKindAgg>> = BTreeMap::new();
    for row in aggs {
        notes.live += row.live;
        notes.pinned += row.pinned;
        notes.retired += row.retired;
        notes.body_bytes += row.body_bytes;
        by_project
            .entry(row.project.clone())
            .or_default()
            .push(MemoryKindAgg {
                kind: row.kind,
                live: row.live,
                pinned: row.pinned,
                retired: row.retired,
                body_bytes: row.body_bytes,
                oldest_ts: row.oldest_ts,
                newest_ts: row.newest_ts,
            });
    }
    let (recalls, stood_for_bytes, injected_bytes) = store.memory_recall_totals(since_unix)?;
    let (mem_search, mem_get) = store.memory_mcp_calls(since_unix)?;
    Ok(MemoryStatus {
        since: since_label.to_string(),
        project: project.map(str::to_string),
        notes,
        recall: MemoryRecallTotals {
            recalls,
            stood_for_bytes,
            injected_bytes,
        },
        calls: MemoryMcpCalls {
            mem_search,
            mem_get,
        },
        by_project: by_project
            .into_iter()
            .map(|(project, kinds)| MemoryProjectBlock { project, kinds })
            .collect(),
    })
}

/// The pages the model offers, each as `(page, snapshot key)` — the wire key that
/// carries the page's numbers; `type` is the wire envelope, not a page. D23: a page
/// that exists on one surface and not the other is a defect, and
/// `tests/surface_parity.rs` holds every surface to this list.
pub fn pages() -> &'static [(&'static str, &'static str)] {
    &[
        ("overview", "usage"),
        ("plugins", "plugins"),
        ("calls", "calls"),
        ("sessions", "sessions"),
        ("doctor", "doctor"),
        ("logs", "logs"),
        ("skills", "skills"),
        ("stats", "stats"),
        ("graph", "graph"),
        ("hosts", "hosts"),
        ("config", "config"),
        ("services", "services"),
        ("worktrees", "worktrees"),
        ("usage", "agent_usage"),
        ("tasks", "tasks"),
    ]
}

/// Open the store at `core.db_path` and read one snapshot. A store that will not open
/// is not fatal for an operator surface: the pages render with zeros and [`Snapshot::error`]
/// carries why (T60.6).
pub fn snapshot(cfg: &Config) -> Snapshot {
    let opened = Store::open(&cfg.core.db_path);
    let store_err = opened.as_ref().err().map(|e| e.to_string());
    let store = opened.ok();
    let mut snap = Model::new(cfg, store.as_ref()).snapshot();
    if let Some(msg) = store_err {
        snap.error = Some(msg);
    } else if snap.doctor.is_none() {
        snap.error = Some("doctor probe failed".into());
    }
    snap
}

/// The Sessions page (T25.1, D27): one row per session, newest first — the same rows
/// the snapshot's `sessions` key carries and `rtok agents sessions` (T25.2) renders.
/// `since` is a `started_at` floor in unix seconds; `0` asks for every session.
/// The store is required, like `plugin_stats`: a page whose whole content is the
/// store's rows reports an unreadable store rather than rendering an empty page.
pub fn sessions(cfg: &Config, since: i64) -> Result<Vec<SessionTotals>> {
    let store = Store::open(&cfg.core.db_path)?;
    Ok(Model::new(cfg, Some(&store)).sessions(since))
}

/// The `rtok stats` page (T15.11): the transcript report — `sessions` here counts transcript
/// files, the report's own definition — with the store's per-API `usage` attached. The store
/// stays optional, as it always was: a store that will not open costs the report its `api`
/// table and nothing else. The store's own definition of a session (`sessions` rows joined to
/// `usage`, D27) is the Sessions page T25.1 adds; both definitions live here, one per page,
/// rather than one number quietly serving two questions.
pub fn stats_report(cfg: &Config) -> Result<stats::Report> {
    let since = stats::parse_since_from(&cfg.stats.since, "stats.since")?;
    let mut report = stats::collect(
        &cfg.stats.transcripts_dir,
        since,
        &cfg.stats.plugin,
        stats::Replay::from_cfg(cfg),
    )?;
    if let Ok(store) = Store::open(&cfg.core.db_path) {
        let _ = stats::attach_api(&mut report, &store);
        let _ = stats::attach_lanes(&mut report, &store);
        let _ = stats::attach_bash_cmd(&mut report, &store);
        let _ = stats::attach_checkpoint_notes(&mut report, &store);
        if cfg.stats.price {
            let _ = stats::attach_costs(&mut report, &store, &cfg.stats.prices);
        }
    }
    stats::attach_codex(&mut report, &cfg.stats.codex_dir, since);
    Ok(report)
}

/// The `rtok stats --plugin <id>` page: one catalogue plugin's `Measurement` rows as data.
/// The store is required — this is the one `stats` view that reports an unreadable store.
pub fn plugin_stats(cfg: &Config, plugin: &str) -> Result<Value> {
    let store = Store::open(&cfg.core.db_path)?;
    let rows = store.list_measurements(plugin)?;
    // `archive_hits` still isolates the expand-rate metric, but `rows` no longer drops
    // expand rows from the listing (T207): they are a cost, not a saving, so they count
    // negative (`ReportSavings::saved`'s contract) rather than being hidden — dropping
    // them here made this page's row count disagree with the Plugins page's.
    let archive_hits = rows.iter().filter(|r| r.kind == "expand").count();
    let rows: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "kind": r.kind,
                "before": r.before_bytes,
                "after": r.after_bytes,
                "est_before": r.est_before,
                "est_after": r.est_after,
                "ref_id": r.ref_id,
            })
        })
        .collect();
    let mut out = json!({
        "plugin": plugin,
        "archive_hits": archive_hits,
        "rows": rows,
    });
    if plugin == "archive" {
        // T5.4 honesty metric: how often a live-zone pointer had to be expanded.
        let (decisions, expanded) = store.archive_decision_counts()?;
        let rate = if decisions > 0 {
            expanded as f64 / decisions as f64
        } else {
            0.0
        };
        out["decisions"] = json!(decisions);
        out["expanded"] = json!(expanded);
        out["expand_rate"] = json!(rate);
    }
    Ok(out)
}

/// The `rtok stats --cache` page: per-session prompt-cache health from the proxy's
/// `usage` rows.
pub fn cache_health(cfg: &Config) -> Result<Vec<cache::SessionHealth>> {
    let store = Store::open(&cfg.core.db_path)?;
    cache::report(&store)
}

mod report;
pub use report::*;

/// The `rtok doctor` page (T15.11): hooks, MCP servers, proxy chains. Every probe runs on
/// this call — the CLI and a cold snapshot miss pay once; each probe is bounded by its
/// `[doctor]` timeout. Surfaces re-read through [`doctor_for_snapshot`] so a 2 s tick
/// cannot re-spawn every MCP server (T15.6 hot path).
pub fn doctor(cfg: &Config) -> Result<doctor::Report> {
    doctor::page(cfg)
}

/// One `agents list` row for a host variant — shared by the full list and the
/// `info`-filtered form.
fn agent_row(
    a: &dyn crate::agents::Agent,
    v: &crate::agents::Variant,
    cfg: &Config,
) -> AgentListRow {
    let present = crate::agents::present(v);
    let app = crate::agents::app_path(v).map(|p| p.display().to_string());
    let version = app.as_ref().map(|_| crate::agents::app_version(v));
    let config = a
        .files(cfg, v.kind)
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    let (modules, mcp, plugins) = if present {
        (
            crate::agents::module_rows(a, v.kind, cfg),
            crate::agents::mcp::rows(a, cfg, v.kind),
            crate::agents::plugin_rows(a, v.kind, cfg),
        )
    } else {
        (Vec::new(), Vec::new(), Vec::new())
    };
    let status = present
        .then(|| crate::agents::plugin_status(a, v.kind, cfg))
        .flatten();
    AgentListRow {
        host: a.id(),
        kind: v.kind.as_str(),
        name: v.name,
        present,
        app,
        version,
        config,
        modules,
        mcp,
        plugins,
        plugin_version: status.as_ref().and_then(|s| s.version.clone()),
        plugin_source: status.and_then(|s| s.source),
    }
}

/// `rtok agents list` as data — one row per known host variant.
pub fn agents_list(cfg: &Config) -> Vec<AgentListRow> {
    let mut out = Vec::new();
    for id in crate::agents::HOSTS {
        let Some(a) = crate::agents::host(id) else {
            continue;
        };
        for v in a.variants() {
            out.push(agent_row(a, v, cfg));
        }
    }
    out
}

/// `rtok agents info <host>` as data — the same variant blocks [`agents_list`] prints,
/// filtered to `ids` (already-resolved host ids).
pub fn agents_listed(cfg: &Config, ids: &[&str]) -> Vec<AgentListRow> {
    crate::agents::visit_hosts(ids, |a, v| agent_row(a, v, cfg))
}

/// `rtok otel status` as data — the same watermarks the table prints.
pub fn otel_status(cfg: &Config) -> Result<OtelStatus> {
    let store = Store::open(&cfg.core.db_path)?;
    let (calls_pending, logs_pending) = store.otel_pending()?;
    let last = store.last_log("otel")?.map(|l| OtelLastLog {
        level: l.level,
        name: l.name,
        message: l.message,
    });
    Ok(OtelStatus {
        endpoint: cfg.otel.resolve().map(|ep| ep.url),
        calls_mark: store.otel_mark("calls")?,
        calls_pending,
        logs_mark: store.otel_mark("logs")?,
        logs_pending,
        sessions_mark: store.otel_mark("sessions")?,
        last,
    })
}

/// How long a snapshot may reuse the last doctor report. Shorter than a sitting at the
/// Doctor tab feels stale; far longer than the 2 s tick, so MCP spawns are not the tick.
const DOCTOR_SNAPSHOT_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// How long a snapshot may reuse the last worktrees read (T232): the walk takes tens
/// of seconds per pass in a checkout with a built `target/`, so reusing
/// `DOCTOR_SNAPSHOT_TTL` would re-walk almost continuously in an open tui/web. The junk
/// section of the Hosts page (T330.2) walks agent folders and reuses it for the same reason.
const WORKTREES_TTL: Duration = Duration::from_secs(300);

/// Snapshot-only doctor: same [`doctor`] probes, cached briefly so `rtok tui` / `rtok web`
/// ticks do not spawn MCP servers every two seconds. `rtok doctor` still goes through
/// [`doctor`] uncached. Tests bypass the cache so Check pins stay byte-identical to a
/// direct probe.
fn doctor_for_snapshot(cfg: &Config) -> Option<doctor::Report> {
    if cfg!(test) {
        return doctor(cfg).ok();
    }
    use std::sync::OnceLock;

    struct Entry {
        at: Instant,
        key: String,
        report: Option<doctor::Report>,
    }
    static CACHE: OnceLock<Mutex<Option<Entry>>> = OnceLock::new();
    let key = format!(
        "{}{}{}",
        cfg.doctor.settings_path.display(),
        cfg.doctor.claude_json.display(),
        cfg.doctor.mcp_json.display()
    );
    let lock = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = lock.lock()
        && let Some(e) = guard.as_ref()
        && e.key == key
        && e.at.elapsed() < DOCTOR_SNAPSHOT_TTL
    {
        return e.report.clone();
    }
    let report = doctor(cfg).ok();
    if let Ok(mut guard) = lock.lock() {
        *guard = Some(Entry {
            at: Instant::now(),
            key,
            report: report.clone(),
        });
    }
    report
}

/// T63.1: T61.3 listing × T61.1 resident. One accessor, two UIs (D23). Does not walk skill files.
pub fn skills_from(
    listing: Option<&doctor::SkillsAudit>,
    stats: Option<&BTreeMap<String, stats::SkillRow>>,
    usage_input: u64,
    stats_scanned: bool,
) -> SkillsPage {
    let mut rows: Vec<SkillPageRow> = listing
        .map(|a| a.rows.as_slice())
        .unwrap_or(&[])
        .iter()
        .map(|r| {
            let st = stats.and_then(|m| m.get(&r.name));
            let invocations = r.invocations.or_else(|| st.map(|s| s.count)).unwrap_or(0);
            let never = r
                .invocations
                .map(|n| n == 0)
                .unwrap_or(stats_scanned && invocations == 0);
            SkillPageRow {
                name: r.name.clone(),
                source: r.source.clone(),
                desc_chars: r.desc_chars,
                body_bytes: r.body_bytes,
                invocations,
                resident: st.map(|s| s.resident).unwrap_or(0),
                last_invoked: if never { "never".into() } else { "—".into() },
                never,
            }
        })
        .collect();
    rows.sort_by(|a, b| b.resident.cmp(&a.resident).then(a.name.cmp(&b.name)));
    let desc_bytes = listing.map(|a| a.desc_bytes).unwrap_or(0);
    let resident_bytes: u64 = rows.iter().map(|r| r.resident).sum();
    let desc_tokens = desc_bytes / 4;
    let share = if usage_input == 0 {
        0.0
    } else {
        100.0 * (resident_bytes / 4) as f64 / usage_input as f64
    };
    SkillsPage {
        header: format!(
            "{} skills · {} desc bytes ≈ {} tok/req · {} resident · {:.1}% of input",
            rows.len(),
            desc_bytes,
            desc_tokens,
            resident_bytes,
            share
        ),
        rows,
    }
}

/// T63.1 / T227: one transcript scan feeds both the Skills page (T61.3 listing joined
/// to T61.1 resident/invocations) and the Stats page ([`stats_page_text`]) — cached
/// briefly so `rtok tui` / `rtok web` ticks do not re-parse every transcript every two
/// seconds, the same shape as [`doctor_for_snapshot`]'s cache. `rtok stats` itself
/// still goes through [`stats_report`] uncached.
#[allow(clippy::type_complexity)]
fn stats_skills(
    cfg: &Config,
) -> (
    Option<BTreeMap<String, stats::SkillRow>>,
    u64,
    bool,
    Option<String>,
) {
    if cfg!(test) {
        return (None, 0, false, None);
    }
    use std::sync::OnceLock;
    struct Entry {
        at: Instant,
        key: String,
        skills: Option<BTreeMap<String, stats::SkillRow>>,
        usage_input: u64,
        scanned: bool,
        text: Option<String>,
    }
    static CACHE: OnceLock<Mutex<Option<Entry>>> = OnceLock::new();
    let key = format!("{}{}", cfg.stats.transcripts_dir.display(), cfg.stats.since);
    let lock = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = lock.lock()
        && let Some(e) = guard.as_ref()
        && e.key == key
        && e.at.elapsed() < DOCTOR_SNAPSHOT_TTL
    {
        return (e.skills.clone(), e.usage_input, e.scanned, e.text.clone());
    }
    // T227: cost costs no network (`[stats.prices]` is a local table), so the page
    // always carries it — `rtok stats` still needs `--price` to print it.
    let mut priced = cfg.clone();
    priced.stats.price = true;
    let (skills, usage_input, scanned, text) = match stats_report(&priced) {
        Ok(report) => {
            let text = Some(stats_page_text(cfg, &report));
            (report.skills, report.usage_input, true, text)
        }
        Err(_) => (None, 0, false, None),
    };
    if let Ok(mut guard) = lock.lock() {
        *guard = Some(Entry {
            at: Instant::now(),
            key,
            skills: skills.clone(),
            usage_input,
            scanned,
            text: text.clone(),
        });
    }
    (skills, usage_input, scanned, text)
}

/// The Stats page (T227): [`stats_skills`]'s one scan rendered as `rtok stats
/// --price`'s table (D24), plus `rtok stats --cache`'s table appended below — D27's
/// one page for two commands, neither re-aggregated.
fn stats_page_text(cfg: &Config, report: &stats::Report) -> String {
    let mut out = report.to_table();
    if let Ok(health) = cache_health(cfg) {
        out.push_str("\ncache health\n");
        out.push_str(&cache::table(&health));
    }
    out
}

/// Dead-symbol lines shown on the Graph page before it says "and N more" instead of
/// flooding the page (T230). A cheap per-tick bound, not a token budget — `graph dead
/// --json` (uncapped) still has the rest.
#[cfg(feature = "graph")]
const GRAPH_DEAD_CAP: usize = 200;

/// The Graph page (T230): `graph status::collect`/`format_table` (same text `rtok
/// graph status` prints) plus `graph::dead_candidates` (the same rows `rtok graph
/// dead --json` prints, minus the `index_for` walk `--json` runs first), capped for
/// display. Opens its own store handle like [`doctor`] — a few count queries plus a
/// scan of the store's dead candidates, not a re-index. With linked projects (T329.21) the
/// pending and dead rows of the whole scope are listed, each headed `[project] `; the TUI shows
/// this text verbatim (D27), so it gets the same prefix. `None` when the `graph` feature is off
/// (build-min) or either read failed.
#[cfg(feature = "graph")]
fn graph_page_text(cfg: &Config) -> Option<String> {
    use crate::plugins::graph::{dead_candidates, scope, status};

    let rt = crate::plugin::Runtime::open(cfg.clone(), "web-graph").ok()?;
    let ctx = crate::plugin::Ctx::new(&rt);
    let root = std::env::current_dir().ok()?;
    let mut health = status::collect(&ctx, &root).ok()?;
    // T329.21: with linked projects each row names its project; a scope of one keeps the old
    // text, so a lone project's page has no badge to repeat.
    let members = scope::resolve(&rt.store, None, &root).unwrap_or_default();
    let linked = members.len() > 1;
    if linked {
        health.pending = scope::pending_page_paths(&ctx, &members);
    }
    let mut out = status::format_table(&health);
    out.push_str("\ndead symbols\n");
    // T230: reads the store as it stands, no `index_for` walk — cheap per tick.
    let found = if linked {
        scope::dead_page_rows(&ctx, &members)
            .map(|(rows, notes)| (rows.into_iter().map(|(p, r)| (Some(p), r)).collect(), notes))
    } else {
        dead_candidates(&ctx, &root).map(|rows| {
            (
                rows.into_iter().map(|r| (None, r)).collect::<Vec<_>>(),
                String::new(),
            )
        })
    };
    match found {
        Ok((rows, notes)) => {
            if rows.is_empty() {
                out.push_str(" none\n");
            }
            let total = rows.len();
            for (project, r) in rows.iter().take(GRAPH_DEAD_CAP) {
                let tag = project
                    .as_ref()
                    .map_or_else(String::new, |p| format!("[{p}] "));
                out.push_str(&format!(
                    " {tag}{}:{} {} {}\n",
                    r.path, r.line, r.kind, r.name
                ));
            }
            if total > GRAPH_DEAD_CAP {
                out.push_str(&format!(
                    " … {} more, capped at {GRAPH_DEAD_CAP} — `rtok graph dead --json` has the rest\n",
                    total - GRAPH_DEAD_CAP
                ));
            }
            for note in notes.lines() {
                out.push_str(&format!(" {note}\n"));
            }
        }
        Err(e) => out.push_str(&format!(" dead scan failed: {e}\n")),
    }
    Some(out)
}

/// What `/ws` carries per project; with `graph` off the registry is never built.
#[cfg(feature = "graph")]
pub use crate::plugins::graph::projects::ProjectRow;
#[cfg(not(feature = "graph"))]
#[derive(Debug, Serialize, JsonSchema)]
pub enum ProjectRow {}

/// The graph page's drill-down request and answer (T329.14); with `graph` off no request parses.
#[cfg(feature = "graph")]
pub use crate::plugins::graph::drill::{DrillGraph, DrillRequest};
#[cfg(not(feature = "graph"))]
#[derive(Debug, Serialize, JsonSchema)]
pub enum DrillGraph {}
#[cfg(not(feature = "graph"))]
#[derive(Debug, serde::Deserialize, Serialize, JsonSchema)]
pub enum DrillRequest {}

/// The graph page's Compare mode request and answer (T329.35); with `graph` off no request parses.
#[cfg(feature = "graph")]
pub use crate::plugins::graph::diff::{DiffReport, DiffRequest};
#[cfg(not(feature = "graph"))]
#[derive(Debug, Serialize, JsonSchema)]
pub enum DiffReport {}
#[cfg(not(feature = "graph"))]
#[derive(Debug, serde::Deserialize, Serialize, JsonSchema)]
pub enum DiffRequest {}

/// The graph page's Export menu and read-only import view (T329.40); with `graph` off no request
/// parses.
#[cfg(feature = "graph")]
pub use crate::plugins::graph::export::{
    Export as GraphExport, ExportFile, ExportRequest, ImportRequest,
};
#[cfg(not(feature = "graph"))]
#[derive(Debug, Serialize, JsonSchema)]
pub enum GraphExport {}
#[cfg(not(feature = "graph"))]
#[derive(Debug, Serialize, JsonSchema)]
pub enum ExportFile {}
#[cfg(not(feature = "graph"))]
#[derive(Debug, serde::Deserialize, Serialize, JsonSchema)]
pub enum ExportRequest {}
#[cfg(not(feature = "graph"))]
#[derive(Debug, serde::Deserialize, Serialize, JsonSchema)]
pub enum ImportRequest {}

#[cfg(feature = "graph")]
fn project_rows(cfg: &Config) -> Option<Vec<ProjectRow>> {
    let rt = crate::plugin::Runtime::open(cfg.clone(), "web-projects").ok()?;
    crate::plugins::graph::projects::rows(&rt).ok()
}

#[cfg(not(feature = "graph"))]
fn project_rows(_cfg: &Config) -> Option<Vec<ProjectRow>> {
    None
}

#[cfg(not(feature = "graph"))]
fn graph_page_text(_cfg: &Config) -> Option<String> {
    None
}

/// A page read too slow for a 2 s tick (T231, T232): the tick renders the last known
/// value while at most one background thread refreshes it once `ttl` has passed — a
/// cold or stale entry never blocks the tick. `None` before the first read ever lands.
struct Background<T> {
    cache: Mutex<Option<(Instant, T)>>,
    refreshing: AtomicBool,
}

impl<T: Clone + Send + 'static> Background<T> {
    fn forget(&self) {
        if let Ok(mut guard) = self.cache.lock() {
            *guard = None;
        }
    }

    const fn new() -> Self {
        Self {
            cache: Mutex::new(None),
            refreshing: AtomicBool::new(false),
        }
    }

    /// The last known value if it is younger than `ttl`; otherwise starts at most one
    /// background `read` (skipped while one is already in flight) and, either way,
    /// returns immediately with the last known value, or `None` before any read has
    /// landed.
    fn get(&'static self, ttl: Duration, read: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        let last_known = self.cache.lock().ok().and_then(|g| {
            g.as_ref()
                .map(|(at, value)| (value.clone(), at.elapsed() < ttl))
        });
        if !matches!(last_known, Some((_, true))) && !self.refreshing.swap(true, Ordering::SeqCst) {
            std::thread::spawn(move || {
                let value = read();
                if let Ok(mut guard) = self.cache.lock() {
                    *guard = Some((Instant::now(), value));
                }
                self.refreshing.store(false, Ordering::SeqCst);
            });
        }
        last_known.map(|(value, _)| value)
    }
}

/// The Hosts page (T231): [`crate::agents::list`]'s blocks, verbatim — the same
/// per-variant kind/version/config/module text `rtok agents list` prints, built from
/// the same probe `agents_list`'s JSON form calls (D27, no duplicated logic).
/// `agents list` spawns one `--version` per host variant (T168) — too slow for a 2 s
/// snapshot tick — so this reuses [`Background`]: a cold or stale entry never blocks
/// the tick, and the tick renders the last known text, or "probing hosts…" before the
/// first probe lands. The junk section below it has its own cache, [`WORKTREES_TTL`].
fn hosts_page_text(cfg: &Config) -> String {
    static HOSTS: Background<String> = Background::new();
    let hosts_cfg = cfg.clone();
    let Some(hosts) = HOSTS.get(DOCTOR_SNAPSHOT_TTL, move || crate::agents::list(&hosts_cfg))
    else {
        return "probing hosts…\n".to_string();
    };
    let junk = junk_page(cfg).map_or_else(|| "measuring folders…\n".to_string(), |j| j.0);
    format!("{hosts}\njunk\n{junk}")
}

static JUNK: Background<(String, JunkCard)> = Background::new();

/// Drops the cached junk report: after a clear the sizes on the page would otherwise stay as
/// they were for up to [`WORKTREES_TTL`].
pub fn forget_junk() {
    JUNK.forget();
}

/// The junk section of the Hosts page as text (tui) and as the card (web), from one report.
/// T330.1: the list rides this page (D27). It walks every installed host's folders (up to
/// `AGENT_SCAN_LIMIT` each), so it has its own slow cache: on the 30 s host-probe TTL an open
/// tui or web would re-walk the disk almost nonstop (the T232 worktrees case).
fn junk_page(cfg: &Config) -> Option<(String, JunkCard)> {
    let junk_cfg = cfg.clone();
    JUNK.get(WORKTREES_TTL, move || {
        let report = crate::agents::junk::report(&junk_cfg);
        let text =
            crate::agents::junk::to_list(&report, &crate::agents::junk_items::View::new(&junk_cfg));
        (text, JunkCard::from(&report))
    })
}

/// The Config page (T228): [`config_entries`]'s rows, the same ones `config
/// show`/`config get` already build (D27). `cfg.home` keeps a `--config`-rooted
/// snapshot reading that home, never the real one. Read-only: unlike `config show`, a
/// snapshot tick never runs `ensure_user_file` — a page view must not create files.
fn config_page_text(cfg: &Config) -> Option<String> {
    let fig = layers::figment(&cfg.home, None, None);
    let mut out = String::new();
    for (key, value, source) in layers::entries(&fig) {
        out.push_str(&format!("{key} = {value} ({source})\n"));
    }
    Some(out)
}

/// The Services page (T229): [`demon::rows`]'s per-service state — the same rows
/// `demon status` already builds (D27) — plus [`otel_status`]'s exporter health, so
/// both commands can join `COMMAND_PAGES`. Plain text, not [`demon::table`]: that
/// colours the state word for a terminal, which the web `BodyText` cannot render.
/// No `last_error` column: nothing records one per service today, so the row points
/// at the service's own log file instead of fabricating a message. `None` on a
/// failed tick, like [`config_page_text`].
fn services_page_text(cfg: &Config) -> Option<String> {
    let rows = demon::rows(cfg, &[]).ok()?;
    let mut out = String::new();
    for r in &rows {
        out.push_str(&format!(
            "{}  {}  pid={}  uptime={}  log={}\n",
            r.service,
            if r.running { "running" } else { "stopped" },
            r.child.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
            r.uptime_secs
                .map(|s| format!("{s}s"))
                .unwrap_or_else(|| "-".into()),
            r.log.display(),
        ));
    }
    if let Ok(otel) = otel_status(cfg) {
        out.push_str(&format!(
            "otel endpoint={} calls_mark={} calls_pending={} logs_mark={} \
             logs_pending={} sessions_mark={}\n",
            otel.endpoint.as_deref().unwrap_or("-"),
            otel.calls_mark,
            otel.calls_pending,
            otel.logs_mark,
            otel.logs_pending,
            otel.sessions_mark,
        ));
        if let Some(last) = &otel.last {
            out.push_str(&format!(
                "last flush: [{}] {} {}\n",
                last.level, last.name, last.message
            ));
        }
    }
    Some(out)
}

/// The Worktrees page (T232): [`crate::worktree::list::rows`]'s rows rendered
/// through its own [`crate::worktree::list::to_table`] — the exact `worktree list`
/// table (path, branch, owner, state, age, `target/` size), so `worktree list` can
/// join `COMMAND_PAGES`; `gc`/`clean` stay CLI-only verdicts, not model data.
/// `usage` walks every worktree's file tree — real cost in a checkout with a built
/// `target/`, tens of seconds in this one — far too slow for a 2 s tick, so this
/// reuses [`Background`] with its own [`WORKTREES_TTL`] (longer than
/// [`DOCTOR_SNAPSHOT_TTL`] — see that constant's doc comment): a cold or stale entry
/// never blocks the tick, a background thread refreshes it, and the tick renders the
/// last known text, or "reading worktrees…" before the first read lands. Outside a
/// git repository `rows` fails; the page says so rather than rendering "did not
/// answer" for what is an ordinary case. `None` only when the current directory
/// could not be read.
fn worktrees_page_text() -> Option<String> {
    static WORKTREES: Background<Option<String>> = Background::new();
    WORKTREES
        .get(WORKTREES_TTL, move || {
            std::env::current_dir()
                .ok()
                .map(|cwd| match crate::worktree::list::rows(&cwd) {
                    Ok(rows) => {
                        crate::worktree::list::to_table(&rows, std::time::SystemTime::now())
                    }
                    Err(_) => "not a git repository\n".to_string(),
                })
        })
        .unwrap_or_else(|| Some("reading worktrees…\n".to_string()))
}

/// How long a snapshot may reuse the last usage read (T358.5). Reading every agent's session
/// logs scales with the history on disk, and the numbers only move when a turn ends.
const USAGE_TTL: Duration = Duration::from_secs(120);

/// The Usage page (T358.5): [`usage::report`] over `[agents.usage]`, exactly what `rtok
/// agents usage` prints. `--source logs` and `both` parse every session file in the window
/// — far too slow for a 2 s tick — so this reuses [`Background`] with its own [`USAGE_TTL`]:
/// a cold or stale entry never blocks the tick, which renders the last known page, or
/// "reading usage…" before the first read lands.
fn usage_page(cfg: &Config) -> UsagePage {
    static USAGE: Background<UsagePage> = Background::new();
    let cfg = cfg.clone();
    USAGE
        .get(USAGE_TTL, move || read_usage_page(&cfg))
        .unwrap_or_else(|| UsagePage {
            text: "reading usage…\n".into(),
            report: None,
        })
}

/// The read behind [`usage_page`], synchronous. A store that will not open or a bad
/// `[agents.usage]` value is the page's text, not a failed snapshot.
fn read_usage_page(cfg: &Config) -> UsagePage {
    let report = Store::open(&cfg.core.db_path)
        .map_err(anyhow::Error::from)
        .and_then(|store| usage::report(cfg, &store, crate::log::now() as i64));
    match report {
        Ok(report) => UsagePage {
            text: report.to_text(),
            report: Some(report),
        },
        Err(e) => UsagePage {
            text: format!("usage did not answer this tick: {e:#}\n"),
            report: None,
        },
    }
}

/// One row of the `rtok config show` page: an effective key, its value, and which layer
/// (`default|user|project|env|flag`) set it.
#[derive(Debug, Serialize)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
    pub source: String,
}

/// The `rtok config show` page (T15.11): every effective key with its origin. Built from
/// the layered figment rather than an extracted `Config`, so `show` still works on a file
/// that would not extract — wrong types are `validate`'s to report, not `show`'s.
/// `config_file` is the `--config` override; `None` resolves through `RTOK_CONFIG` /
/// `<home>/config.toml`.
pub fn config_entries(home: &Path, config_file: Option<&Path>) -> Result<Vec<ConfigEntry>> {
    Config::ensure_user_file(home, config_file)?;
    let fig = layers::figment(home, config_file, None);
    Ok(layers::entries(&fig)
        .into_iter()
        .map(|(key, value, source)| ConfigEntry { key, value, source })
        .collect())
}

/// Reads the pages. Borrows the store so tests can pass an in-memory one.
pub struct Model<'a> {
    cfg: &'a Config,
    store: Option<&'a Store>,
}

impl<'a> Model<'a> {
    pub fn new(cfg: &'a Config, store: Option<&'a Store>) -> Self {
        Self { cfg, store }
    }

    pub fn snapshot(&self) -> Snapshot {
        let calls = self.calls();
        let ref_ids = self
            .store
            .and_then(|s| {
                s.archive_ref_ids(&calls.iter().map(|c| c.id).collect::<Vec<_>>())
                    .ok()
            })
            .unwrap_or_default();
        let doctor = doctor_for_snapshot(self.cfg);
        let (st, usage, scanned, stats_text) = stats_skills(self.cfg);
        let skills = skills_from(
            doctor.as_ref().and_then(|d| d.skills.as_ref()),
            st.as_ref(),
            usage,
            scanned,
        );
        Snapshot {
            kind: "snapshot",
            usage: self.overview(),
            plugins: self.plugins(),
            calls,
            sessions: self.sessions(0),
            // The one doctor query (D27): the snapshot carries what `rtok doctor`
            // renders, so neither surface grows a probe of its own. Cached briefly —
            // a failed or in-flight tick is `None`, never a failed snapshot.
            doctor,
            logs: self.log_lines(None),
            error: None,
            skills,
            ref_ids,
            // T227: the same scan `stats_skills` already ran for the Skills page.
            stats: stats_text,
            // T230: reads the store on this tick — see `graph_page_text`.
            graph: graph_page_text(self.cfg),
            // T329.12: the registry, read fresh each tick so a second tab sees a selection.
            projects: project_rows(self.cfg),
            // T231: cached in the background — see `hosts_page_text`.
            hosts: hosts_page_text(self.cfg),
            junk: junk_page(self.cfg).map(|j| j.1),
            // T228: reads the layered figment fresh each tick — see `config_page_text`.
            config: config_page_text(self.cfg),
            // T229: reuses `demon::rows` and `otel_status` — see `services_page_text`.
            services: services_page_text(self.cfg),
            // T232: cached briefly — see `worktrees_page_text`.
            worktrees: worktrees_page_text(),
            // T358.5: logs are read off the tick — see `usage_page`.
            agent_usage: usage_page(self.cfg),
            // T441.13.7: read off the tick — see `tasks::page`.
            tasks: tasks::page(self.cfg),
        }
    }

    /// Sessions page (T25.1): the newest [`SESSIONS_ROWS`] sessions, newest first, through
    /// the store's one `session_totals` query — the model does not keep a second reader
    /// (D27), and it does not decide what "live" means: `ended_at` is in the row and
    /// the renderers filter. No store (or one that will not read): an empty page,
    /// like Overview's zeros.
    pub fn sessions(&self, since: i64) -> Vec<SessionTotals> {
        self.store
            .and_then(|s| s.recent_session_totals(since, SESSIONS_ROWS as i64).ok())
            .unwrap_or_default()
    }

    /// Calls page (T15.5): the last [`CALLS_ROWS`] ledger rows, newest first, through
    /// the store's one `recent_calls` read — the model keeps no second reader (D27).
    /// No store, or one that will not read: an empty page, like Overview's zeros.
    /// When the proxy is in plain mode, in-memory live passthrough rows are prepended
    /// (negative ids; kind `live_passthrough`) so TUI/web still show traffic flowing.
    pub fn calls(&self) -> Vec<CallRow> {
        let mut rows = self
            .store
            .and_then(|s| s.recent_calls(CALLS_ROWS as i64).ok())
            .unwrap_or_default();
        let live = live_calls(self.cfg);
        if !live.is_empty() {
            let mut merged: Vec<CallRow> = live
                .into_iter()
                .enumerate()
                .map(|(i, c)| live_as_call_row(-(i as i32 + 1), c))
                .collect();
            merged.append(&mut rows);
            merged.truncate(CALLS_ROWS);
            return merged;
        }
        rows
    }

    /// Overview: provider usage totals across every API, plus the CTT and per-turn
    /// series the tab draws (T15.3), from `usage_by_api` and `usage_ctt`.
    /// No store, or one that will not read: zeros, like before.
    pub fn overview(&self) -> Overview {
        let mut out = Overview {
            alerts: crate::proxy::live::alerts(self.cfg),
            ..Overview::default()
        };
        let Some(store) = self.store else {
            return out;
        };
        if let Ok(rows) = store.usage_by_api() {
            for r in rows {
                out.totals.input += r.input;
                out.totals.output += r.output;
                out.totals.cache_create += r.cache_create;
                out.totals.cache_read += r.cache_read;
            }
        }
        if let Ok((ctt, turns)) = store.usage_ctt(OVERVIEW_TURNS as i64) {
            out.ctt = ctt;
            out.turns = turns;
        }
        // A bad zone is the Usage page's error to report; the trend falls back to the system's.
        let tz = usage::zone(&self.cfg.agents.usage.tz).unwrap_or_else(|_| TimeZone::system());
        out.savings = savings_trend(store, crate::log::now() as i64, &tz).unwrap_or_default();
        out
    }

    /// `rtok plugins`: the Plugins page. A missing database is an empty store, same as the
    /// command used to open one itself.
    pub fn plugin_pages(cfg: &Config) -> Vec<PluginPage> {
        let store = Store::open(&cfg.core.db_path).ok();
        // Both borrows are parameters so the store's short lifetime can shorten `cfg`'s.
        fn pages(cfg: &Config, store: Option<&Store>) -> Vec<PluginPage> {
            Model::new(cfg, store).plugins()
        }
        pages(cfg, store.as_ref())
    }

    /// Plugins: the catalogue, each with its page and — when it saves tokens — its stats.
    pub fn plugins(&self) -> Vec<PluginPage> {
        // T207: one grouped read for the whole page instead of a `list_measurements`
        // per catalogue plugin — the N+1 `report_savings` had too, and the same read.
        let totals = self.plugin_stat_totals();
        Registry::new(self.cfg)
            .pages()
            .into_iter()
            .map(|(m, enabled, mut page)| {
                if m.id == "memory"
                    && let Some(store) = self.store
                    && let Ok(aggs) = store.memory_note_aggs(None)
                {
                    let live: u64 = aggs.iter().map(|r| r.live).sum();
                    let pinned: u64 = aggs.iter().map(|r| r.pinned).sum();
                    let retired: u64 = aggs.iter().map(|r| r.retired).sum();
                    page.fields.push(("notes live".into(), live.to_string()));
                    page.fields
                        .push(("notes pinned".into(), pinned.to_string()));
                    page.fields
                        .push(("notes retired".into(), retired.to_string()));
                }
                // Checkpoints are written by `inject`; without it there is nothing to count.
                #[cfg(feature = "inject")]
                if m.id == "memory"
                    && let Some(store) = self.store
                    && let Ok(rows) =
                        store.kv_prefix(&crate::plugin::plugin_state_key("memory", ""))
                {
                    let (typed, skipped) = crate::plugins::checkpoint::prompt_counts(&rows);
                    page.fields
                        .push(("checkpoint prompts typed".into(), typed.to_string()));
                    page.fields.push((
                        "checkpoint host records skipped".into(),
                        skipped.to_string(),
                    ));
                }
                // After the live numbers: the TUI shows these pairs on one line (T419), and
                // the settings there are the part a narrow terminal may cut.
                page.fields.extend(config_fields(m.id, self.cfg));
                PluginPage {
                    id: m.id,
                    enabled,
                    surfaces: m.surfaces.iter().map(|s| s.as_str()).collect(),
                    title: page.title,
                    summary: page.summary,
                    saves_tokens: page.saves_tokens,
                    fields: page.fields,
                    stats: page
                        .saves_tokens
                        .then(|| totals.get(m.id).copied().unwrap_or_default()),
                }
            })
            .collect()
    }

    /// `rows`/est_before/est_after per plugin, every `Measurement` kind summed in — an
    /// `expand` row counts negative here too (`ReportSavings::saved`'s contract), the
    /// same [`Store::measurement_totals`] read `report_savings` builds its rows from, so
    /// the Plugins page and `rtok report` cannot disagree about one plugin's rows again
    /// (T207).
    fn plugin_stat_totals(&self) -> BTreeMap<String, Stats> {
        let mut by = BTreeMap::new();
        let Some(store) = self.store else {
            return by;
        };
        let Ok(totals) = store.measurement_totals() else {
            return by;
        };
        for t in totals {
            fold_total(&mut by, t);
        }
        by
    }

    /// Logs page (T15.11): the last `n` log lines, newest first — `[log] lines` when
    /// `None`. The selection is the model's; the numbering and colour are the CLI's.
    pub fn log_lines(&self, n: Option<usize>) -> Vec<String> {
        crate::log::tail(self.cfg, n)
    }

    /// Demon page (T15.11): one row per supervised service, state asked of the kernel
    /// rather than read out of the state file.
    pub fn demon(&self, named: &[Service]) -> Result<Vec<demon::Row>> {
        demon::rows(self.cfg, named)
    }
}

/// One measurement group into its plugin's [`Stats`]: the Plugins page's totals and the Δtok
/// trend's days sum through this one fold, so a day cannot count a plugin differently.
fn fold_total(by: &mut BTreeMap<String, Stats>, t: MeasurementTotal) {
    let s = by.entry(t.plugin).or_default();
    s.rows += t.rows as u64;
    s.est_before += t.est_before;
    s.est_after += t.est_after;
}

/// The Δtok trend (T414.13): [`SAVINGS_DAYS`] days ending with `now`'s day in `tz`, oldest
/// first, from [`Store::measurement_totals_since`] folded per day and plugin. A row stamped
/// after today (a skewed clock) has no day here and is left out rather than moved.
pub fn savings_trend(store: &Store, now: i64, tz: &TimeZone) -> Result<Vec<SavingsDay>> {
    let today = Timestamp::from_second(now)?.to_zoned(tz.clone()).date();
    let first = today.checked_sub(Span::new().days(SAVINGS_DAYS as i64 - 1))?;
    let mut days: Vec<(Date, BTreeMap<String, Stats>)> =
        std::iter::successors(Some(first), |d| d.tomorrow().ok())
            .take(SAVINGS_DAYS)
            .map(|d| (d, BTreeMap::new()))
            .collect();
    for (ts, t) in store.measurement_totals_since(usage::start_of(first, tz)?)? {
        let day = Timestamp::from_second(ts)?.to_zoned(tz.clone()).date();
        if let Some((_, by)) = days.iter_mut().find(|(d, _)| *d == day) {
            fold_total(by, t);
        }
    }
    Ok(days
        .into_iter()
        .map(|(day, by)| {
            let rows = by.values().map(|s| s.rows).sum();
            let plugins: BTreeMap<String, i64> = by
                .into_iter()
                .map(|(p, s)| (p, s.est_before - s.est_after))
                .collect();
            SavingsDay {
                day: day.to_string(),
                rows,
                saved: (rows > 0).then(|| plugins.values().sum()),
                plugins,
            }
        })
        .collect())
}

fn live_calls(cfg: &Config) -> Vec<crate::proxy::LiveCall> {
    if cfg.proxy.enabled && cfg.core.enabled {
        return Vec::new();
    }
    // Same-process ring first (tests, in-process proxy). Only ask a running
    // proxy over HTTP when this process has nothing — otherwise an empty or
    // foreign `/live` on the default port would hide real local rows.
    let local = crate::proxy::live::snapshot();
    if !local.is_empty() {
        return local;
    }
    fetch_live(cfg).unwrap_or_default()
}

fn fetch_live(cfg: &Config) -> Option<Vec<crate::proxy::LiveCall>> {
    let url = format!("http://{}:{}", cfg.proxy.bind, cfg.proxy.port);
    let timeout = std::time::Duration::from_millis(cfg.doctor.probe_timeout_ms.max(100));
    let body = crate::doctor::http_get(&url, "/live", timeout)?;
    serde_json::from_str(&body).ok()
}

fn live_as_call_row(id: i32, c: crate::proxy::LiveCall) -> CallRow {
    CallRow {
        id,
        ts: c.ts,
        session: "(live)".into(),
        surface: "proxy".into(),
        kind: "live_passthrough".into(),
        plugin: None,
        name: Some(c.path),
        parent_id: None,
        ms: Some(c.ms),
        ok: if c.status < 400 { 1 } else { 0 },
        error: None,
        host: None,
        provider: c.provider,
        model: c.model,
        api: None,
        input: Some(c.request_bytes as i64),
        cache_create: None,
        cache_read: None,
        output: Some(c.response_bytes as i64),
    }
}

/// The Calls page size column: bytes for in-memory plain-proxy rows, tokens when a
/// `usage` row is linked (`api` set), otherwise `-`.
pub fn call_size_label(c: &CallRow) -> String {
    if c.kind == "live_passthrough" {
        return match (c.input, c.output) {
            (None, None) => "-".into(),
            _ => format!("{} B", c.input.unwrap_or(0) + c.output.unwrap_or(0)),
        };
    }
    if c.api.is_some() && c.input.is_some() {
        return format!("{} tok", call_linked_tokens(c));
    }
    "-".into()
}

/// Sum of the four counters on a usage-linked call row.
pub fn call_linked_tokens(c: &CallRow) -> i64 {
    c.input.unwrap_or(0)
        + c.cache_create.unwrap_or(0)
        + c.cache_read.unwrap_or(0)
        + c.output.unwrap_or(0)
}

/// Session drill-down (T60.3, D23): the snapshot's `SessionTotals` row plus the
/// snapshot's calls filtered by that id. Both surfaces render this pair; neither
/// grows a second session type or a second query (D27).
/// Archive payload for both surfaces (T60.4, D23): [`crate::expand::fetch`] then
/// [`crate::expand::render_lines`] with `[expand] max_lines`. A live-zone pointer
/// freezes exactly as `rtok expand` does — one function, no second path. `grep`
/// is `--grep` parity for the TUI `/` filter and the web filter box.
pub fn expand_payload(cfg: &Config, id: &str, grep: Option<&str>) -> Option<String> {
    let cx = crate::plugin::Runtime::open(cfg.clone(), "expand").ok()?;
    let bytes = crate::expand::fetch(&cx, id).ok()??;
    let text = String::from_utf8_lossy(&bytes);
    crate::expand::render_lines(&text, id, None, grep, 0, cfg.expand.max_lines).ok()
}

pub fn session_detail<'a>(
    snapshot: &'a Snapshot,
    id: &str,
) -> Option<(&'a SessionTotals, Vec<&'a CallRow>)> {
    let session = snapshot.sessions.iter().find(|s| s.id == id)?;
    Some((
        session,
        snapshot.calls.iter().filter(|c| c.session == id).collect(),
    ))
}

fn config_fields(id: &str, cfg: &Config) -> Vec<(String, String)> {
    let p = &cfg.plugins;
    match id {
        "cmd" => vec![
            kv("rewrite", p.cmd.rewrite),
            kv("trailer_min_lines", p.cmd.trailer_min_lines),
        ],
        "read" => vec![
            kv("default_mode", &p.read.default_mode),
            kv("max_chars", p.read.max_chars),
        ],
        "archive" => vec![
            kv("keep_turns", p.archive.keep_turns),
            kv("min_tokens", p.archive.min_tokens),
        ],
        "inject" => vec![kv("budget_tokens", p.inject.budget_tokens)],
        "guard" => vec![kv("window_turns", p.guard.window_turns)],
        "memory" => vec![
            kv("recall_titles", p.memory.recall_titles),
            kv("recall_tokens", p.memory.recall_tokens),
            kv("prompt_recall", p.memory.prompt_recall),
            kv("sync_tokens", p.memory.sync_tokens),
        ],
        "graph" => vec![kv("max_tokens", p.graph.max_tokens)],
        "docs" => vec![
            kv("query_limit", p.docs.query_limit),
            kv("snippet_chars", p.docs.snippet_chars),
            kv("max_tokens", p.docs.max_tokens),
        ],
        "toon" => vec![kv("min_rows", p.toon.min_rows)],
        "proxy" => vec![
            kv("enabled", cfg.proxy.enabled),
            kv("mode", &cfg.proxy.mode),
            kv("port", cfg.proxy.port),
        ],
        "measure" => vec![kv("since", &cfg.stats.since)],
        _ => vec![],
    }
}

fn kv(k: &str, v: impl ToString) -> (String, String) {
    (k.into(), v.to_string())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
