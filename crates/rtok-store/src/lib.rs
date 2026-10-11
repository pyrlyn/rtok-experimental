// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! One SQLite file (plan T0.3, T13.1, decision D8): WAL mode, FTS5, migrations keyed by filename.
//!
//! Paths and settings come in as arguments. This crate prints nothing: a failed housekeeping
//! step comes back as [`StoreWarning`].

#![forbid(unsafe_code)]

/// `bail!` that returns [`StoreError`], not `anyhow::Error`. `anyhow::bail!` returns
/// `Err(anyhow::Error)` and will not coerce into this crate's `Result`.
macro_rules! bail {
    ($($tt:tt)*) => {
        return ::core::result::Result::Err($crate::StoreError::from(::anyhow::anyhow!($($tt)*)))
    };
}
pub(crate) use bail;

mod error;
pub use error::{Result, StoreError};
mod paths;
pub use paths::{
    canon, is_unwalkable_root, path_ends_with, path_starts_with, same_path, strip_prefix,
};
pub mod sanitize;
mod task_id;
pub use task_id::{MAX_DEPTH, MAX_PREFIX, TaskId, check_prefix};

// Agent registry (T282, D34): one row per host session rtok sees, one per sub-agent.
mod agents;
pub use agents::{AgentDetail, AgentRow, idle_secs};
// Graph call events the web page streams (T329.15).
mod graph_events;
pub use graph_events::{EventPhase, GraphEvent, MeasurementSample};
// Messages between agents and the user (T287).
mod messages;
pub use messages::{Message, short_agent_id};
pub mod embed;
pub use embed::EmbedSettings;
// T433: hook session fields saved once, spliced back into the stdin on read.
mod hook_fields;
mod host_usage;
pub use host_usage::read_host_messages;
#[cfg(any(test, feature = "test-util"))]
pub use host_usage::seed_host_messages;
mod migrations;
pub mod models;
pub mod otel;
pub mod schema;
// Statements the typed DSL cannot express (window functions). T163.3 and T163.9 add to this module.
mod sql_ext;
// T163: shared Diesel extension for SQL the DSL cannot express (recursive CTEs, FTS5).
// Symbol index (graph plugin) — SQLite only (D18 loser deleted; P39: Ladybug/Grafeo removed).
mod symbols;
// T329.14: whole-root scans the graph page's drill-down builds its nodes and edges from.
mod symbol_graph;
pub use symbol_graph::{DefRow, RefGroup};
// T329.1: the graph project registry.
mod docs;
mod note_files;
pub use docs::{DocHit, DocItem};
mod observations;
mod project_links;
mod projects;
pub use project_links::{Link, LinkKind};
pub use projects::{Origin, Project, Resolved, canon_root};
// T285: which agent a worktree is bound to (the git lock stays the source of truth).
mod task_claims;
mod task_counters;
mod worktree_claims;
mod worktree_pending;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use anyhow::Context;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Integer, Nullable, Text};
use diesel::sqlite::SqliteConnection;
use serde::Serialize;
use sha2::{Digest, Sha256};

use rtok_plugin_sdk::Measurement;
// The two row shapes a plugin sees are the contract's (D25); the diesel rows below feed them.
pub use rtok_plugin_sdk::{ArchiveDecision, NoteHit};

// `models` (the schema::models table) is not imported bare: it collides with this file's
// own `pub mod models` of Diesel row structs, so upsert_model qualifies it as `schema::models`.
use schema::{
    archive, archive_decisions, call_io, calls, hook_sessions, hosts, kv, logs, measurements,
    note_versions, notes, providers, read_cache, sessions, tokens, usage,
};

pub(crate) use sql_ext::{coalesce, length, substr, sum_bigint, unixepoch};

// `usage_by_model_tier` groups by a column of each table.
diesel::allow_columns_to_appear_in_same_group_by_clause!(
    usage::model,
    calls::kind,
    calls::service_tier
);

/// Pause between `open` attempts while another connection holds the lock.
const OPEN_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(100);

/// `checkpoint:*` and `session:*` are rewritten often and are not user notes (T472).
fn keeps_note_versions(kind: &str) -> bool {
    !kind.starts_with("checkpoint:") && !kind.starts_with("session:")
}

/// Insert the previous title and body. `version = COALESCE(MAX(version), 0) + 1`.
fn record_note_version(
    conn: &mut SqliteConnection,
    note_id: i32,
    title: &str,
    body: &str,
) -> Result<()> {
    use diesel::dsl::max;
    let next = note_versions::table
        .filter(note_versions::note_id.eq(note_id))
        .select(max(note_versions::version))
        .first::<Option<i32>>(conn)?
        .unwrap_or(0)
        + 1;
    diesel::insert_into(note_versions::table)
        .values((
            note_versions::note_id.eq(note_id),
            note_versions::title.eq(title),
            note_versions::body.eq(body),
            note_versions::version.eq(next),
        ))
        .execute(conn)?;
    Ok(())
}

pub struct Store {
    conn: Mutex<SqliteConnection>,
    wait: LockWait,
    /// `[core] store_raw` (T431): save request bodies verbatim instead of cleaned.
    store_raw: std::sync::atomic::AtomicBool,
}

/// How long one connection waits on another process's lock (T178).
#[derive(Debug, Clone, Copy)]
pub struct LockWait {
    /// `busy_timeout` for every statement.
    pub busy: std::time::Duration,
    /// Fresh connections tried when the WAL switch returns "database is locked".
    pub attempts: u32,
    /// `busy_timeout` while migrations run.
    pub migrate: std::time::Duration,
}

impl LockWait {
    /// Every surface but the hook: 1 s per statement, 10 connects, 30 s for a migration run.
    pub const STEADY: Self = Self {
        busy: std::time::Duration::from_secs(1),
        attempts: 10,
        migrate: std::time::Duration::from_secs(30),
    };
}

/// A statement gave up on another process's lock after its `busy_timeout`.
pub fn is_locked(e: &dyn std::error::Error) -> bool {
    let mut cur = Some(e);
    while let Some(err) = cur {
        if format!("{err:#}").contains("database is locked") {
            return true;
        }
        cur = err.source();
    }
    false
}

/// Unix seconds, or `0` when the clock is before the epoch. Same value `rtok_log::now` returns,
/// so a retention cutoff does not depend on the log crate.
pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `calls.kind` of a Batch-lane request. The host's `lane::KIND_BATCH` is this string.
pub const BATCH_CALL_KIND: &str = "api_request:batch";

/// `calls.service_tier` of a request a provider served on its Flex tier (T385.12.2); the price
/// key suffix `@flex` is this name.
pub const FLEX_SERVICE_TIER: &str = "flex";

/// What [`Store::housekeeping`] skipped. The caller prints and logs `message`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreWarning {
    /// `retention` or `vacuum`.
    pub what: &'static str,
    /// `{what} skipped until next start: {error}` — the text a surface used to print.
    pub message: String,
}

/// Paths and day bounds for [`Store::housekeeping`]. The store does not read config.
#[derive(Debug, Clone)]
pub struct RetentionJob {
    pub db_path: PathBuf,
    pub retain_calls_days: u32,
    pub retain_hook_bodies_days: u32,
}

type MigrateHook = fn(&str, &mut dyn FnMut() -> Result<usize>) -> Result<usize>;

static LONG_MIGRATE: std::sync::OnceLock<MigrateHook> = std::sync::OnceLock::new();

/// Spinner for a migration whose wait is at least a second. The hook binary sets this;
/// unset, the migration just runs. The store itself prints nothing.
pub fn on_long_migrate(hook: MigrateHook) {
    let _ = LONG_MIGRATE.set(hook);
}

/// Which write lock [`hold_write_lock`] takes. Tests use it instead of opening Diesel themselves.
#[derive(Debug, Clone, Copy)]
pub enum HeldWrite {
    /// `BEGIN IMMEDIATE`.
    Immediate,
    /// `BEGIN EXCLUSIVE`.
    Exclusive,
}

/// Hold a write lock on `db` for `hold`, after the same PRAGMAs a second connection uses.
/// Returns once the lock is taken.
pub fn hold_write_lock(
    db: &Path,
    mode: HeldWrite,
    hold: std::time::Duration,
) -> std::thread::JoinHandle<()> {
    use diesel::connection::SimpleConnection;
    let (held, held_ack) = std::sync::mpsc::channel();
    let url = db.to_str().expect("db path is utf-8").to_string();
    let sql = match mode {
        HeldWrite::Immediate => "BEGIN IMMEDIATE;",
        HeldWrite::Exclusive => "BEGIN EXCLUSIVE;",
    };
    let holder = std::thread::spawn(move || {
        let mut conn = SqliteConnection::establish(&url).expect("open store");
        conn.batch_execute("PRAGMA busy_timeout = 1000; PRAGMA journal_mode = WAL;")
            .expect("pragma");
        conn.batch_execute(sql).expect("begin");
        held.send(()).expect("held");
        std::thread::sleep(hold);
        conn.batch_execute("COMMIT;").expect("commit");
    });
    held_ack.recv().expect("lock taken");
    holder
}

fn set_busy(conn: &mut SqliteConnection, busy: std::time::Duration) -> Result<()> {
    // PRAGMA rejects a bound parameter; `sql_ext` writes the digits.
    sql_ext::busy_timeout(conn, busy.as_millis())?;
    Ok(())
}

/// One row of [`Store::sessions_by_cwd`].
#[derive(Debug, Clone)]
pub struct SessionSeen {
    pub id: String,
    pub host: Option<String>,
    pub cwd: String,
    /// Newest `calls.ts` of the session, else `started_at`.
    pub last_seen: i64,
    pub ended_at: Option<i64>,
}

/// Turn arbitrary user text into an FTS5 MATCH phrase query: every blank-separated token is
/// quoted, so `*`, `(`, `-`, `AND` and `"` are searched for as characters instead of being
/// read as FTS5 syntax. `None` when there is no token left to search for.
pub(crate) fn fts_phrase_query(query: &str) -> Option<String> {
    let quoted: Vec<String> = query
        .split_whitespace()
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect();
    (!quoted.is_empty()).then(|| quoted.join(" "))
}

/// The four required `notes` columns as one `.values(...)` tuple — shared by
/// [`Store::insert_note`] and [`Store::insert_note_if_absent`] (T209), which differ only
/// in the `INSERT` variant and what an ignored conflict means for the caller.
#[allow(clippy::type_complexity)]
fn note_values<'a>(
    project: Option<&'a str>,
    kind: &'a str,
    title: &'a str,
    body: &'a str,
) -> (
    diesel::dsl::Eq<notes::project, Option<&'a str>>,
    diesel::dsl::Eq<notes::kind, &'a str>,
    diesel::dsl::Eq<notes::title, &'a str>,
    diesel::dsl::Eq<notes::body, &'a str>,
) {
    (
        notes::project.eq(project),
        notes::kind.eq(kind),
        notes::title.eq(title),
        notes::body.eq(body),
    )
}

#[cfg(any(test, feature = "test-util"))]
thread_local! {
    /// Test-only tally of fresh SQLite connections on this thread (T203): PreCompact,
    /// SessionEnd and SessionStart used to open a second or third `Store` beside the one the
    /// hook `Runtime` already holds; the count lets a test assert one open per hook run.
    /// `test-util` publishes it to the rtok test suite; `cfg(test)` alone is invisible
    /// to a crate that depends on this one.
    pub static OPEN_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Rows per write transaction when clearing hook bodies (T352).
const HOOK_BODY_BATCH: i64 = 5_000;

fn age_days(now: i64, ts: i64) -> f64 {
    (now.saturating_sub(ts) as f64 / 86_400.0).max(0.0)
}

fn days_since(now: i64, last_used: Option<i64>) -> Option<f64> {
    last_used.map(|t| age_days(now, t))
}

/// Retention used only as a rank (T454). Adapted from agentmemory
/// `src/functions/retention.ts` `computeRetention`: a pin is 1, otherwise
/// `min(1, salience * exp(-0.01 * ageDays) + 0.3 / daysSinceUsed)`.
/// Never deletes a row.
pub fn retention_score(
    pinned: bool,
    uses: i32,
    age_days: f64,
    days_since_used: Option<f64>,
) -> f64 {
    if pinned {
        return 1.0;
    }
    let salience = 0.5 + (f64::from(uses) * 0.02).min(0.2);
    let temporal = (-0.01 * age_days).exp();
    let boost = match days_since_used {
        Some(d) if d > 0.0 => 0.3 / d,
        _ => 0.0,
    };
    (salience * temporal + boost).min(1.0)
}

mod archive_rows;
mod cache;
mod call_log;
mod handle;
mod measure_rows;
mod note_store;
mod retention;
mod session_rows;

#[derive(QueryableByName)]
struct ArchPath {
    #[diesel(sql_type = Text)]
    id: String,
    #[diesel(sql_type = Text)]
    path: String,
}

/// Archive rows whose only `call_io` references are all older than `cutoff` and that carry no
/// decision or read-cache row — shared by [`Store::purge_calls_older_than`] (which deletes them)
/// and [`Store::archives_pending_retention`] (which only previews the same set, T182).
fn doomed_archives(c: &mut SqliteConnection, cutoff: i64) -> Result<Vec<ArchPath>> {
    let rows: Vec<(String, String)> = sql_ext::DoomedArchives { cutoff }.load(c)?;
    Ok(rows
        .into_iter()
        .map(|(id, path)| ArchPath { id, path })
        .collect())
}

/// `(inline json, archive sha, byte count, content sha, archive file path, file created by
/// this call, raw bytes)`. The archive fields are `Some`/`true` together only when the body
/// was spilled to disk — see [`Store::spill`] (T208). `raw` is `Some` only for an inline body
/// that is not valid UTF-8 (T211) — see [`inline_body`].
type Spill = (
    Option<String>,
    Option<String>,
    i64,
    Option<String>,
    Option<PathBuf>,
    bool,
    Option<Vec<u8>>,
);

/// `(call_io.request_json, hook_sessions.fields)` — [`Store::recent_hook_inputs`] (T433).
type HookInputRow = (Option<String>, Option<String>);

/// A missing body stays `""` (see [`Store::recent_hook_inputs`]); a split one is made whole.
fn rebuild_hook_inputs(rows: Vec<HookInputRow>) -> Vec<String> {
    rows.into_iter()
        .map(|(body, fields)| {
            body.map(|b| hook_fields::rebuild(b, fields.as_deref()))
                .unwrap_or_default()
        })
        .collect()
}

/// `(host slug, project, cwd)` — [`Store::session_row`].
pub type SessionRow = (Option<String>, Option<String>, Option<String>);

/// Text stored inline for display, the sha256 of `body`'s raw bytes, and — only when `body`
/// is not valid UTF-8 — the exact bytes to keep alongside the lossy text (T211). Valid UTF-8
/// already round-trips losslessly through the `TEXT` column (its bytes are `body`'s bytes),
/// so the `BLOB` column is written only for the lossy case, avoiding doubled storage for the
/// common path. The sha is always over `body` itself, so it matches the old behavior for
/// valid UTF-8 and now verifies the true wire bytes for invalid UTF-8 too.
fn inline_body(body: &[u8]) -> (String, String, Option<Vec<u8>>) {
    let sha = hex_sha256(body);
    match std::str::from_utf8(body) {
        Ok(s) => (s.to_owned(), sha, None),
        Err(_) => (
            String::from_utf8_lossy(body).into_owned(),
            sha,
            Some(body.to_vec()),
        ),
    }
}

pub fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Write `body` to `dir/<sha>`, content-addressed so the same body written twice is the same
/// file (T208: `insert_call_io`'s two spills can share a body with an earlier archive row).
/// The bool says whether this call created the file (`false` when it already held this exact
/// content) — only a file this call created is safe to remove if the caller's insert fails.
fn write_archive_file(dir: &Path, sha: &str, body: &[u8]) -> Result<(PathBuf, bool)> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(sha);
    let created = !path.exists();
    // T324: `expand` reads this file lock-free, and another hook process may write the same
    // body at once, so write a sibling temp file and rename it over the target (atomic on one
    // filesystem). `fs::write` truncates in place and let a reader see an empty or short file.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{sha}.tmp-{}-{n}", std::process::id()));
    if let Err(e) = std::fs::write(&tmp, body).and_then(|()| std::fs::rename(&tmp, &path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok((path, created))
}

/// The `archive` row behind a file [`write_archive_file`] already wrote. `on_conflict …
/// do_nothing`: the same body archived twice (T5.3 repeat requests) is one row.
fn insert_archive_row_conn(
    conn: &mut SqliteConnection,
    sha: &str,
    session: &str,
    bytes: i64,
    path: &Path,
    agent_id: Option<&str>,
) -> Result<()> {
    diesel::insert_into(archive::table)
        .values((
            archive::id.eq(sha),
            archive::session.eq(session),
            archive::bytes.eq(bytes),
            archive::path.eq(path.to_string_lossy().as_ref()),
            archive::sha256.eq(sha),
            archive::agent_id.eq(agent_id),
        ))
        .on_conflict(archive::id)
        .do_nothing()
        .execute(conn)?;
    Ok(())
}

/// The `archive_decisions.expanded_ts` update behind [`Store::mark_expanded`] and
/// [`Store::mark_expanded_recorded`].
fn mark_expanded_conn(conn: &mut SqliteConnection, archive_id: &str) -> Result<usize> {
    // `unixepoch()` has no typed-DSL form; bind Rust's now.
    let now = i64::try_from(unix_now()).unwrap_or(i64::MAX);
    Ok(diesel::update(
        archive_decisions::table
            .filter(archive_decisions::archive_id.eq(archive_id))
            .filter(archive_decisions::expanded_ts.is_null()),
    )
    .set(archive_decisions::expanded_ts.eq(now))
    .execute(conn)?)
}

/// The `measurements` insert behind [`Store::insert_measurement`] and
/// [`Store::mark_expanded_recorded`].
fn insert_measurement_conn(
    conn: &mut SqliteConnection,
    session: &str,
    m: &Measurement,
    once: Option<&str>,
) -> Result<()> {
    let once_key = once.map(|o| {
        let r = m.ref_id.as_deref().unwrap_or("");
        format!("{o}|{}|{}|{r}", m.plugin, m.kind)
    });
    let before_bytes = i64::try_from(m.before_bytes).context("measurement before_bytes")?;
    let after_bytes = i64::try_from(m.after_bytes).context("measurement after_bytes")?;
    let est_before = i32::try_from(m.est_before).context("measurement est_before")?;
    let est_after = i32::try_from(m.est_after).context("measurement est_after")?;
    diesel::insert_into(measurements::table)
        .values((
            measurements::session.eq(session),
            measurements::plugin.eq(m.plugin),
            measurements::kind.eq(m.kind),
            measurements::before_bytes.eq(before_bytes),
            measurements::after_bytes.eq(after_bytes),
            measurements::est_before.eq(est_before),
            measurements::est_after.eq(est_after),
            measurements::ref_id.eq(m.ref_id.as_deref()),
            measurements::call_id.eq(m.call_id),
            measurements::once_key.eq(once_key),
        ))
        .on_conflict(measurements::once_key)
        .do_nothing()
        .execute(conn)?;
    Ok(())
}

/// Portable JSONL metadata for [`Store::insert_portable_note_if_absent`] (T294).
pub struct PortableNote<'a> {
    pub project: Option<&'a str>,
    pub kind: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub id: Option<i32>,
    pub ts: Option<i64>,
    pub retired: Option<i64>,
    pub superseded_by: Option<i32>,
    pub pinned: Option<i32>,
}

/// One portable `memory export` row (T294). Field order matches [`Store::list_export_notes`].
#[derive(Debug, Clone, Queryable)]
pub struct ExportNote {
    pub id: i32,
    pub ts: i64,
    pub project: Option<String>,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub retired: Option<i64>,
    pub superseded_by: Option<i32>,
    pub pinned: i32,
}

/// One `notes` row's lifecycle-relevant fields (T69.1): revise needs `kind`/`project`,
/// `mem_get` prefixes retired rows, recall orders by `pinned`. Field order matches
/// [`Store::note_row`]'s select.
#[derive(Debug, Clone, Queryable)]
pub struct NoteRow {
    pub id: i32,
    pub project: Option<String>,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub retired: Option<i64>,
    pub superseded_by: Option<i32>,
    pub pinned: i32,
}

impl NoteRow {
    pub fn is_pinned(&self) -> bool {
        self.pinned != 0
    }
}

/// One `(project, kind)` row from [`Store::memory_note_aggs`].
#[derive(Debug, Clone)]
pub struct MemoryNoteKindAgg {
    pub project: Option<String>,
    pub kind: String,
    pub live: u64,
    pub pinned: u64,
    pub retired: u64,
    pub body_bytes: i64,
    pub oldest_ts: i64,
    pub newest_ts: i64,
}

/// One `measurements` row for `stats --plugin`.
#[derive(Debug, Queryable)]
pub struct MeasRow {
    pub kind: String,
    pub before_bytes: i64,
    pub after_bytes: i64,
    pub est_before: i32,
    pub est_after: i32,
    pub ref_id: Option<String>,
}

/// Each session's host slug, `None` when it has no host row.
fn host_by_session(conn: &mut SqliteConnection) -> Result<HashMap<String, Option<String>>> {
    Ok(sessions::table
        .left_join(hosts::table)
        .select((sessions::id, hosts::slug.nullable()))
        .load::<(String, Option<String>)>(conn)?
        .into_iter()
        .collect())
}

/// One `(plugin, kind)` group from [`Store::measurement_totals`] (T207), or one
/// `(plugin, kind, ts)` group from [`Store::measurement_totals_since`].
#[derive(Debug, Clone)]
pub struct MeasurementTotal {
    pub plugin: String,
    pub kind: String,
    pub rows: i64,
    pub est_before: i64,
    pub est_after: i64,
}

/// What both measurement aggregates select: plugin, kind, `COUNT(*)` and the two sums.
type MeasurementTotalRow = (String, String, i64, Option<i64>, Option<i64>);

impl MeasurementTotal {
    fn from_row((plugin, kind, rows, est_before, est_after): MeasurementTotalRow) -> Self {
        Self {
            plugin,
            kind,
            rows,
            est_before: est_before.unwrap_or(0),
            est_after: est_after.unwrap_or(0),
        }
    }
}

/// Aggregated usage totals grouped by API (T11.6).
#[derive(Debug, Clone)]
pub struct ApiUsage {
    pub api: String,
    pub input: i64,
    pub cache_create: i64,
    pub cache_read: i64,
    pub output: i64,
}

/// Aggregated usage totals grouped by `calls.kind` and `calls.service_tier` (T385.6, T385.12.2).
#[derive(Debug, Clone)]
pub struct LaneUsage {
    pub kind: String,
    /// The tier the provider reported; `None` when the response named none.
    pub tier: Option<String>,
    pub input: i64,
    pub cache_create: i64,
    pub cache_read: i64,
    pub output: i64,
}

/// Aggregated usage totals grouped by model (`rtok stats --price`, T49.1).
/// A `NULL` model (older rows) reads back as `"unknown"`.
#[derive(Debug, Clone)]
pub struct ModelUsage {
    pub model: String,
    pub input: i64,
    pub cache_create: i64,
    pub cache_read: i64,
    pub output: i64,
}

/// [`ModelUsage`] of one `calls.kind` (the proxy lane), T385.10.
#[derive(Debug, Clone)]
pub struct LaneModelUsage {
    pub kind: Option<String>,
    pub usage: ModelUsage,
}

/// One [`Store::usage_slices`] row: the usage of one session on one model at `ts` (unix
/// seconds, UTC).
#[derive(Debug, Clone)]
pub struct UsageSlice {
    pub host: Option<String>,
    pub api: String,
    pub session: String,
    pub model: Option<String>,
    pub ts: i64,
    pub input: i64,
    pub cache_create: i64,
    pub cache_read: i64,
    pub output: i64,
}

/// One `usage` row (proxy ground truth, T5.1).
#[derive(Debug)]
pub struct UsageRow {
    pub session: String,
    pub model: Option<String>,
    pub api: String,
    pub input: i64,
    pub cache_create: i64,
    pub cache_read: i64,
    pub output: i64,
    pub call_id: Option<i64>,
}

/// One session's totals ([`Store::session_totals`], T25.1) — the rendering input of the
/// Sessions page and `rtok agent sessions` (T25.2). Everything below is one query's
/// output, so no renderer can re-derive a number differently (D27): tokens are whole-
/// session sums of `usage`, `last_activity` is the MAX ts over the session's `usage`
/// and `calls` rows (falling back to `started_at` when there are none), and `ended_at`
/// `None` means live.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema, Queryable, QueryableByName,
)]
pub struct SessionTotals {
    #[diesel(sql_type = Text)]
    pub id: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub host: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub project: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub provider: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub api: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub model: Option<String>,
    #[diesel(sql_type = BigInt)]
    pub input: i64,
    #[diesel(sql_type = BigInt)]
    pub cache_create: i64,
    #[diesel(sql_type = BigInt)]
    pub cache_read: i64,
    #[diesel(sql_type = BigInt)]
    pub output: i64,
    #[diesel(sql_type = BigInt)]
    pub started_at: i64,
    #[diesel(sql_type = BigInt)]
    pub last_activity: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    pub ended_at: Option<i64>,
}

/// One `calls` row as the Calls page serves it ([`Store::recent_calls`], T15.5): the
/// ledger's own columns plus the slugs its ids point at and — when the call is one
/// that recorded usage — the newest `usage` row linked to it. One query's output, so
/// no renderer can re-derive a field differently (D27); `api` `None` means no usage
/// row is linked (a hook, MCP call or plugin run carries none).
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema, Queryable, QueryableByName)]
pub struct CallRow {
    #[diesel(sql_type = Integer)]
    pub id: i32,
    #[diesel(sql_type = BigInt)]
    pub ts: i64,
    #[diesel(sql_type = Text)]
    pub session: String,
    #[diesel(sql_type = Text)]
    pub surface: String,
    #[diesel(sql_type = Text)]
    pub kind: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub plugin: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub name: Option<String>,
    #[diesel(sql_type = Nullable<Integer>)]
    pub parent_id: Option<i32>,
    #[diesel(sql_type = Nullable<Double>)]
    pub ms: Option<f64>,
    #[diesel(sql_type = Integer)]
    pub ok: i32,
    #[diesel(sql_type = Nullable<Text>)]
    pub error: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub host: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub provider: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub model: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub api: Option<String>,
    #[diesel(sql_type = Nullable<BigInt>)]
    pub input: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    pub cache_create: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    pub cache_read: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    pub output: Option<i64>,
}

impl Store {
    /// `(call plugin, surface, kind, tool, ok, token plugin, phase, source, tokens)` of every
    /// `mcp_call`, ordered by tool, phase, token plugin. The HTTP MCP test reads this instead
    /// of opening Diesel itself.
    #[allow(clippy::type_complexity)]
    pub fn mcp_call_token_rows(
        &self,
    ) -> Result<
        Vec<(
            Option<String>,
            String,
            String,
            Option<String>,
            i32,
            Option<String>,
            String,
            String,
            i64,
        )>,
    > {
        calls::table
            .inner_join(tokens::table)
            .filter(calls::kind.eq("mcp_call"))
            .select((
                calls::plugin,
                calls::surface,
                calls::kind,
                calls::name,
                calls::ok,
                tokens::plugin,
                tokens::phase,
                tokens::source,
                tokens::n_tokens,
            ))
            .order((calls::name, tokens::phase, tokens::plugin))
            .load(&mut *self.lock()?)
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod support;
#[cfg(test)]
mod tests_archive;
#[cfg(test)]
mod tests_schema;
#[cfg(test)]
mod tests_store;
