// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! SQL the typed DSL cannot express. Each item names the construct Diesel 2.3 lacks.

use diesel::alias;
use diesel::prelude::*;
use diesel::query_builder::{AstPass, Query, QueryFragment, QueryId};
use diesel::sql_types::{BigInt, Binary, Double, Integer, Nullable, Text};
use diesel::sqlite::Sqlite;

use super::schema::{
    archive, archive_decisions, archive_summaries, call_io, calls, extractor, logs, measurements,
    read_cache, symbol_stale, symbols, tokens, usage,
};

/// `ON CONFLICT DO UPDATE … WHERE` — Diesel 2.3's `DoUpdate` has no WHERE clause, so an
/// unchanged embedding would be rewritten and `embedded_at` would move.
#[derive(QueryId)]
pub(crate) struct UpsertNoteEmbedding {
    pub note_id: i32,
    pub model: String,
    pub dims: i32,
    pub text_hash: String,
    pub vector: Vec<u8>,
}

impl RunQueryDsl<SqliteConnection> for UpsertNoteEmbedding {}

impl QueryFragment<Sqlite> for UpsertNoteEmbedding {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql(
            "INSERT INTO note_embeddings (note_id, model, dims, text_hash, vector) VALUES (",
        );
        out.push_bind_param::<Integer, _>(&self.note_id)?;
        out.push_sql(", ");
        out.push_bind_param::<Text, _>(&self.model)?;
        out.push_sql(", ");
        out.push_bind_param::<Integer, _>(&self.dims)?;
        out.push_sql(", ");
        out.push_bind_param::<Text, _>(&self.text_hash)?;
        out.push_sql(", ");
        out.push_bind_param::<Binary, _>(&self.vector)?;
        out.push_sql(
            ") ON CONFLICT(note_id) DO UPDATE SET \
               model = excluded.model, \
               dims = excluded.dims, \
               text_hash = excluded.text_hash, \
               embedded_at = unixepoch(), \
               vector = excluded.vector \
             WHERE excluded.text_hash != note_embeddings.text_hash \
                OR excluded.model != note_embeddings.model \
                OR excluded.dims != note_embeddings.dims",
        );
        Ok(())
    }
}

// `COALESCE` for an upsert `DO UPDATE SET` (T163.5): not one of Diesel's built-ins.
#[diesel::declare_sql_function]
extern "SQL" {
    fn coalesce<T: diesel::sql_types::SqlType + diesel::sql_types::SingleValue>(
        x: diesel::sql_types::Nullable<T>,
        y: diesel::sql_types::Nullable<T>,
    ) -> diesel::sql_types::Nullable<T>;
}

// SQLite `length()`: character count of a TEXT value, not byte count.
diesel::define_sql_function!(fn length(x: Text) -> BigInt);

// `SUM` as `Nullable<BigInt>`: Diesel's `sum()` widens to `Nullable<Numeric>`.
diesel::define_sql_function! {
    #[aggregate]
    #[sql_name = "SUM"]
    fn sum_bigint(x: BigInt) -> Nullable<BigInt>;
}

/// `substr(text, start, length)` — not one of Diesel's built-ins (T163.6).
#[diesel::declare_sql_function]
extern "SQL" {
    fn substr(
        x: diesel::sql_types::Text,
        start: diesel::sql_types::Integer,
        length: diesel::sql_types::Integer,
    ) -> diesel::sql_types::Text;
}

// SQLite `unixepoch()` — not a Diesel built-in.
diesel::define_sql_function!(fn unixepoch() -> Nullable<BigInt>);

/// One `PRAGMA` as a `QueryFragment`. `HAS_STATIC_QUERY_ID = false` because
/// `busy_timeout`'s text includes the millisecond value.
struct PragmaStmt {
    sql: String,
}

impl QueryId for PragmaStmt {
    type QueryId = ();
    const HAS_STATIC_QUERY_ID: bool = false;
}

impl QueryFragment<Sqlite> for PragmaStmt {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql(&self.sql);
        Ok(())
    }
}

impl RunQueryDsl<SqliteConnection> for PragmaStmt {}

fn exec_pragma(conn: &mut SqliteConnection, sql: impl Into<String>) -> QueryResult<()> {
    PragmaStmt { sql: sql.into() }.execute(conn).map(|_| ())
}

/// Milliseconds are a duration we computed, written as digits.
pub(crate) fn busy_timeout(conn: &mut SqliteConnection, ms: u128) -> QueryResult<()> {
    exec_pragma(conn, format!("PRAGMA busy_timeout = {ms}"))
}

pub(crate) fn pragma_journal_wal(conn: &mut SqliteConnection) -> QueryResult<()> {
    exec_pragma(conn, "PRAGMA journal_mode = WAL")
}

pub(crate) fn pragma_synchronous_normal(conn: &mut SqliteConnection) -> QueryResult<()> {
    exec_pragma(conn, "PRAGMA synchronous = NORMAL")
}

pub(crate) fn pragma_foreign_keys_on(conn: &mut SqliteConnection) -> QueryResult<()> {
    exec_pragma(conn, "PRAGMA foreign_keys = ON")
}

/// T352: only effective before the first table exists — the caller sets it on a brand-new file.
pub(crate) fn pragma_auto_vacuum_incremental(conn: &mut SqliteConnection) -> QueryResult<()> {
    exec_pragma(conn, "PRAGMA auto_vacuum = INCREMENTAL")
}

/// T646: `sqlite_stat1` rows for `symbols`. With none, the planner served `ORDER BY path` from
/// `symbols_root_path (root=?)` — every row of the root — instead of seeking
/// `symbols_root_name`. The sampled form (SQLite's `analysis_limit`) keeps an index pass on a
/// large store cheap and still tells the planner that `root` alone is not selective.
/// No DSL form for either statement.
pub(crate) fn analyze_symbols(conn: &mut SqliteConnection) -> QueryResult<()> {
    exec_pragma(conn, "PRAGMA analysis_limit = 1000")?;
    exec_pragma(conn, "ANALYZE symbols")
}

/// A row of `PRAGMA incremental_vacuum`: one per freed page, no columns.
#[derive(QueryableByName)]
struct FreedPage {}

/// T352: hand every free page back to the filesystem; a no-op when `auto_vacuum` is 0.
pub(crate) fn pragma_incremental_vacuum(conn: &mut SqliteConnection) -> QueryResult<()> {
    // One page is freed per result row, so the statement must be stepped to the end (`execute`
    // steps once). No DSL form for a PRAGMA.
    diesel::sql_query("PRAGMA incremental_vacuum")
        .load::<FreedPage>(conn)
        .map(|_| ())
}

/// T352: `VACUUM` rebuilds the file, which is the only way to change `auto_vacuum` on a store
/// that already has tables. No DSL form.
pub(crate) fn vacuum(conn: &mut SqliteConnection) -> QueryResult<()> {
    exec_pragma(conn, "VACUUM")
}

/// T968: `PRAGMA wal_checkpoint(TRUNCATE)` — copies the WAL into the database and cuts the WAL
/// file to zero bytes. It answers one row `(busy, log frames, checkpointed frames)`, so it
/// is a query, not a statement. A reader holding the WAL makes it report busy, which is
/// not an error: the next prune retries. No DSL form for a PRAGMA.
#[derive(QueryId)]
pub(crate) struct WalCheckpointTruncate;

impl QueryFragment<Sqlite> for WalCheckpointTruncate {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql("PRAGMA wal_checkpoint(TRUNCATE)");
        Ok(())
    }
}

impl Query for WalCheckpointTruncate {
    type SqlType = (Integer, Integer, Integer);
}

impl RunQueryDsl<SqliteConnection> for WalCheckpointTruncate {}

/// `PRAGMA auto_vacuum` — a read, one column, no DSL form (0 none, 1 full, 2 incremental).
#[derive(QueryId)]
pub(crate) struct AutoVacuumMode;

impl QueryFragment<Sqlite> for AutoVacuumMode {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql("PRAGMA auto_vacuum");
        Ok(())
    }
}

impl Query for AutoVacuumMode {
    type SqlType = Integer;
}

impl RunQueryDsl<SqliteConnection> for AutoVacuumMode {}

#[cfg(any(test, feature = "test-util"))]
pub(crate) fn pragma_query_only_on(conn: &mut SqliteConnection) -> QueryResult<()> {
    exec_pragma(conn, "PRAGMA query_only = ON")
}

/// `PRAGMA journal_mode` — a read, one column, no DSL form.
#[cfg(test)]
#[derive(QueryId)]
pub(crate) struct JournalMode;

#[cfg(test)]
impl QueryFragment<Sqlite> for JournalMode {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql("PRAGMA journal_mode");
        Ok(())
    }
}

#[cfg(test)]
impl Query for JournalMode {
    type SqlType = Text;
}

#[cfg(test)]
impl RunQueryDsl<SqliteConnection> for JournalMode {}

/// `BEGIN IMMEDIATE` — Diesel 2.3 has `immediate_transaction` but no statement form for a
/// second connection that holds the write lock across a sleep (concurrency test).
#[cfg(test)]
#[derive(QueryId)]
pub(crate) struct BeginImmediate;

#[cfg(test)]
impl QueryFragment<Sqlite> for BeginImmediate {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql("BEGIN IMMEDIATE");
        Ok(())
    }
}

#[cfg(test)]
impl RunQueryDsl<SqliteConnection> for BeginImmediate {}

/// `COMMIT` — pair for [`BeginImmediate`] when the lock is released by hand.
#[cfg(test)]
#[derive(QueryId)]
pub(crate) struct Commit;

#[cfg(test)]
impl QueryFragment<Sqlite> for Commit {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql("COMMIT");
        Ok(())
    }
}

#[cfg(test)]
impl RunQueryDsl<SqliteConnection> for Commit {}

/// `EXPLAIN QUERY PLAN` — no form in Diesel 2.3's typed DSL. Restates `archive_in_session`'s
/// correlated subquery so the plan can assert `measurements_session_ts`.
#[cfg(test)]
#[derive(QueryId)]
pub(crate) struct ExplainArchiveInSessionPlan;

#[cfg(test)]
impl QueryFragment<Sqlite> for ExplainArchiveInSessionPlan {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql(
            "EXPLAIN QUERY PLAN SELECT archive.id, \
             (SELECT COUNT(*) FROM measurements \
              WHERE measurements.session = archive.session AND measurements.ts > archive.ts) \
             FROM archive \
             WHERE archive.id = 'x' AND archive.session = 's' AND archive.agent_id IS NULL",
        );
        Ok(())
    }
}

#[cfg(test)]
impl Query for ExplainArchiveInSessionPlan {
    type SqlType = (Integer, Integer, Integer, Text);
}

#[cfg(test)]
impl RunQueryDsl<SqliteConnection> for ExplainArchiveInSessionPlan {}

/// `sqlite_master` catalog — not a `table!` Diesel can model.
#[cfg(test)]
#[derive(QueryId)]
pub(crate) struct SqliteMasterSnapshot;

#[cfg(test)]
impl QueryFragment<Sqlite> for SqliteMasterSnapshot {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql(
            "SELECT type AS kind, name, tbl_name, sql FROM sqlite_master \
             WHERE type IN ('table', 'index', 'trigger')",
        );
        Ok(())
    }
}

#[cfg(test)]
impl Query for SqliteMasterSnapshot {
    type SqlType = (Text, Text, Text, Nullable<Text>);
}

#[cfg(test)]
impl RunQueryDsl<SqliteConnection> for SqliteMasterSnapshot {}

/// `PRAGMA table_xinfo` — no DSL form; table name is a schema identifier, not a bind.
/// `HAS_STATIC_QUERY_ID = false`: the SQL text changes with `table`, so Diesel must not
/// reuse a prepared statement from another table.
#[cfg(test)]
pub(crate) struct PragmaTableXinfo {
    pub table: String,
}

#[cfg(test)]
impl QueryId for PragmaTableXinfo {
    type QueryId = ();
    const HAS_STATIC_QUERY_ID: bool = false;
}

#[cfg(test)]
impl QueryFragment<Sqlite> for PragmaTableXinfo {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql("SELECT name, type AS ty, \"notnull\", pk FROM pragma_table_xinfo('");
        out.push_sql(&self.table);
        out.push_sql("')");
        Ok(())
    }
}

#[cfg(test)]
impl Query for PragmaTableXinfo {
    type SqlType = (Text, Text, Integer, Integer);
}

#[cfg(test)]
impl RunQueryDsl<SqliteConnection> for PragmaTableXinfo {}

/// Fixture DDL for schema-drift mutation tests — one statement per execute (prepared
/// statements do not run a semicolon-separated batch). Not a static query id: each
/// fixture string is a different statement.
#[cfg(test)]
pub(crate) struct FixtureSql {
    pub sql: &'static str,
}

#[cfg(test)]
impl QueryId for FixtureSql {
    type QueryId = ();
    const HAS_STATIC_QUERY_ID: bool = false;
}

#[cfg(test)]
impl QueryFragment<Sqlite> for FixtureSql {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql(self.sql);
        Ok(())
    }
}

#[cfg(test)]
impl RunQueryDsl<SqliteConnection> for FixtureSql {}

/// `sqlite_master` catalog count — no `table!` for SQLite's schema tables.
#[cfg(test)]
#[derive(QueryId)]
pub(crate) struct CountCoreV2Tables;

#[cfg(test)]
impl QueryFragment<Sqlite> for CountCoreV2Tables {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN \
             ('hosts','providers','models','sessions','calls','call_io','tokens','logs')",
        );
        Ok(())
    }
}

#[cfg(test)]
impl Query for CountCoreV2Tables {
    type SqlType = BigInt;
}

#[cfg(test)]
impl RunQueryDsl<SqliteConnection> for CountCoreV2Tables {}

/// Multi-row fixture seed with fixed ids and duplicate topic keys on a pre-0020
/// database. One statement (a prepared execute does not run a semicolon batch); not a
/// typed insert of the live `notes` shape.
#[cfg(test)]
#[derive(QueryId)]
pub(crate) struct SeedPre0020DuplicateNotes;

#[cfg(test)]
impl QueryFragment<Sqlite> for SeedPre0020DuplicateNotes {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql(
            "INSERT INTO notes (id, ts, project, kind, title, body) VALUES \
             (1, 1, NULL, 'note', 'dup', 'stale'), \
             (2, 2, NULL, 'note', 'dup', 'fresh'), \
             (3, 1, 'rtok', 'note', 'dup', 'stale'), \
             (4, 2, 'rtok', 'note', 'dup', 'fresh')",
        );
        Ok(())
    }
}

#[cfg(test)]
impl RunQueryDsl<SqliteConnection> for SeedPre0020DuplicateNotes {}

/// Multi-row fixture seed on a pre-0021 `measurements` table (`once_key` does not exist
/// yet; Diesel's `table!` already lists it). One statement (a prepared execute does not
/// run a semicolon batch).
#[cfg(test)]
#[derive(QueryId)]
pub(crate) struct SeedPre0021Measurements;

#[cfg(test)]
impl QueryFragment<Sqlite> for SeedPre0021Measurements {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql(
            "INSERT INTO measurements (ts, session, plugin, kind, before_bytes, after_bytes, \
             est_before, est_after) VALUES (1, 's', 'read', 'delta', 9, 1, 3, 1), \
             (1, 's', 'read', 'delta', 9, 1, 3, 1)",
        );
        Ok(())
    }
}

#[cfg(test)]
impl RunQueryDsl<SqliteConnection> for SeedPre0021Measurements {}

/// Expression conflict target `COALESCE(project, '')` — Diesel's `on_conflict` names columns only.
#[derive(QueryId)]
pub(crate) struct UpsertNote {
    pub project: Option<String>,
    pub kind: String,
    pub title: String,
    pub body: String,
}

impl QueryFragment<Sqlite> for UpsertNote {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql("INSERT INTO notes (project, kind, title, body) VALUES (");
        out.push_bind_param::<Nullable<Text>, _>(&self.project)?;
        out.push_sql(", ");
        out.push_bind_param::<Text, _>(&self.kind)?;
        out.push_sql(", ");
        out.push_bind_param::<Text, _>(&self.title)?;
        out.push_sql(", ");
        out.push_bind_param::<Text, _>(&self.body)?;
        out.push_sql(
            ") ON CONFLICT (COALESCE(project, ''), kind, title) DO UPDATE SET \
                 body = excluded.body, \
                 ts = unixepoch(), \
                 retired = NULL, \
                 superseded_by = NULL \
             RETURNING id",
        );
        Ok(())
    }
}

impl Query for UpsertNote {
    type SqlType = Integer;
}

impl RunQueryDsl<SqliteConnection> for UpsertNote {}

/// FTS5 `MATCH` and `bm25()` — no form in Diesel 2.3's typed DSL.
#[derive(QueryId)]
pub(crate) struct SearchNotes {
    pub query: String,
    pub limit: i32,
}

impl QueryFragment<Sqlite> for SearchNotes {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql(
            "SELECT n.id, n.title, substr(n.body, 1, 120), n.pinned, n.uses, n.ts, n.last_used, \
             n.project \
             FROM notes_fts f JOIN notes n ON n.id = f.rowid \
             WHERE notes_fts MATCH ",
        );
        out.push_bind_param::<Text, _>(&self.query)?;
        out.push_sql(" AND n.retired IS NULL ORDER BY bm25(notes_fts) LIMIT ");
        out.push_bind_param::<Integer, _>(&self.limit)?;
        Ok(())
    }
}

impl Query for SearchNotes {
    /// `(id, title, snippet, pinned, uses, ts, last_used, project)`: the columns `rank_blend`
    /// and the project boost read.
    type SqlType = (
        Integer,
        Text,
        Text,
        Integer,
        Integer,
        BigInt,
        Nullable<BigInt>,
        Nullable<Text>,
    );
}

impl RunQueryDsl<SqliteConnection> for SearchNotes {}

/// FTS5 `MATCH` / `bm25()` over cached rustdoc items (T455). Same quoting path as notes.
#[derive(QueryId)]
pub(crate) struct SearchDocItems {
    pub query: String,
    pub crate_name: String,
    pub version: String,
    pub snippet: i32,
    pub limit: i32,
}

impl QueryFragment<Sqlite> for SearchDocItems {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql("SELECT i.id, i.path, i.kind, substr(i.docs, 1, ");
        out.push_bind_param::<Integer, _>(&self.snippet)?;
        out.push_sql(
            ") FROM doc_items_fts f JOIN doc_items i ON i.id = f.rowid \
             WHERE doc_items_fts MATCH ",
        );
        out.push_bind_param::<Text, _>(&self.query)?;
        out.push_sql(" AND i.crate_name = ");
        out.push_bind_param::<Text, _>(&self.crate_name)?;
        out.push_sql(" AND i.version = ");
        out.push_bind_param::<Text, _>(&self.version)?;
        out.push_sql(" ORDER BY bm25(doc_items_fts) LIMIT ");
        out.push_bind_param::<Integer, _>(&self.limit)?;
        Ok(())
    }
}

impl Query for SearchDocItems {
    type SqlType = (Integer, Text, Text, Text);
}

impl RunQueryDsl<SqliteConnection> for SearchDocItems {}

/// FTS5 `MATCH` and `bm25()` over `symbols_fts` — no form in Diesel 2.3's typed DSL (T474).
#[derive(QueryId)]
pub(crate) struct SearchSymbols {
    pub root: String,
    pub query: String,
    pub limit: i32,
}

impl QueryFragment<Sqlite> for SearchSymbols {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql(
            "SELECT s.path, s.name, s.kind, s.line, s.signature, s.doc, bm25(symbols_fts) \
             FROM symbols_fts JOIN symbols s ON s.id = symbols_fts.rowid \
             WHERE symbols_fts MATCH ",
        );
        out.push_bind_param::<Text, _>(&self.query)?;
        out.push_sql(" AND s.root = ");
        out.push_bind_param::<Text, _>(&self.root)?;
        out.push_sql(" AND s.is_def = 1 LIMIT ");
        out.push_bind_param::<Integer, _>(&self.limit)?;
        Ok(())
    }
}

impl Query for SearchSymbols {
    type SqlType = (Text, Text, Text, Integer, Text, Text, Double);
}

impl RunQueryDsl<SqliteConnection> for SearchSymbols {}

/// FTS5 over `observations`. Optional project: `NULL` matches every project.
#[derive(QueryId)]
pub(crate) struct SearchObservations {
    pub query: String,
    pub project: Option<String>,
    pub limit: i32,
}

impl QueryFragment<Sqlite> for SearchObservations {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql(
            "SELECT o.id, o.title, o.session_id, substr(o.narrative, 1, 120) \
             FROM observations_fts f JOIN observations o ON o.id = f.rowid \
             WHERE observations_fts MATCH ",
        );
        out.push_bind_param::<Text, _>(&self.query)?;
        // Unbound rows (NULL project) recall in every project; a bound row only in its own.
        out.push_sql(" AND (");
        out.push_bind_param::<Nullable<Text>, _>(&self.project)?;
        out.push_sql(" IS NULL OR o.project IS NULL OR o.project = ");
        out.push_bind_param::<Nullable<Text>, _>(&self.project)?;
        out.push_sql(") ORDER BY bm25(observations_fts) LIMIT ");
        out.push_bind_param::<Integer, _>(&self.limit)?;
        Ok(())
    }
}

impl Query for SearchObservations {
    type SqlType = (Integer, Text, Text, Text);
}

impl RunQueryDsl<SqliteConnection> for SearchObservations {}

/// A duplicate observation in the last few seconds, by `(session_id, dedup)`.
#[derive(QueryId)]
pub(crate) struct RecentObservationDup {
    pub session_id: String,
    pub dedup: String,
    pub since: i64,
}

impl QueryFragment<Sqlite> for RecentObservationDup {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql("SELECT id FROM observations WHERE session_id = ");
        out.push_bind_param::<Text, _>(&self.session_id)?;
        out.push_sql(" AND dedup = ");
        out.push_bind_param::<Text, _>(&self.dedup)?;
        out.push_sql(" AND ts >= ");
        out.push_bind_param::<BigInt, _>(&self.since)?;
        out.push_sql(" LIMIT 1");
        Ok(())
    }
}

impl Query for RecentObservationDup {
    type SqlType = Integer;
}

impl RunQueryDsl<SqliteConnection> for RecentObservationDup {}

/// `COUNT() OVER` and `ROW_NUMBER() OVER` — no window functions in Diesel 2.3's typed DSL.
#[derive(QueryId)]
pub(crate) struct UsageCtt;

impl QueryFragment<Sqlite> for UsageCtt {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql(
            "SELECT COALESCE(SUM(ctx * (total - rn)), 0) FROM (
                SELECT input + cache_create + cache_read AS ctx,
                       COUNT(*) OVER (PARTITION BY session) AS total,
                       ROW_NUMBER() OVER (PARTITION BY session ORDER BY ts, id) AS rn
                FROM usage)",
        );
        Ok(())
    }
}

impl Query for UsageCtt {
    type SqlType = BigInt;
}

impl RunQueryDsl<SqliteConnection> for UsageCtt {}

/// `JOIN` of a grouped `MIN` subquery — Diesel 2.3 cannot type this shape.
#[derive(QueryId)]
pub(crate) struct UsageCttTail {
    pub turns: i64,
}

impl QueryFragment<Sqlite> for UsageCttTail {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql(
            "SELECT u.input + u.cache_create + u.cache_read
             FROM usage u
             JOIN (SELECT session, MIN(ts) AS first_ts, MIN(id) AS first_id
                   FROM usage GROUP BY session) f ON f.session = u.session
             ORDER BY f.first_ts DESC, f.first_id DESC, u.ts DESC, u.id DESC
             LIMIT ",
        );
        out.push_bind_param::<BigInt, _>(&self.turns)?;
        Ok(())
    }
}

impl Query for UsageCttTail {
    type SqlType = BigInt;
}

impl RunQueryDsl<SqliteConnection> for UsageCttTail {}

/// Four CTEs, `UNION ALL`, and per-group `MAX(id)` subqueries — no form in Diesel 2.3.
#[derive(QueryId)]
pub(crate) struct RecentSessionTotals {
    pub since: i64,
    pub limit: i64,
}

impl QueryFragment<Sqlite> for RecentSessionTotals {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql(
            "WITH tot AS (
                 SELECT session AS sid, SUM(input) AS input, SUM(cache_create) AS cache_create,
                        SUM(cache_read) AS cache_read, SUM(output) AS output
                 FROM usage GROUP BY session
             ),
             last_u AS (
                 SELECT session AS sid, api, model FROM usage
                 WHERE id IN (SELECT MAX(id) FROM usage GROUP BY session)
             ),
             act AS (
                 SELECT sid, MAX(ts) AS ts FROM (
                     SELECT session AS sid, ts FROM usage
                     UNION ALL
                     SELECT session_id AS sid, ts FROM calls
                 ) GROUP BY sid
             ),
             prov AS (
                 SELECT c.session_id AS sid, p.slug AS provider
                 FROM calls c JOIN providers p ON p.id = c.provider_id
                 WHERE c.id IN (SELECT MAX(id) FROM calls
                                WHERE provider_id IS NOT NULL GROUP BY session_id)
             )
             SELECT s.id, h.slug, s.project,
                    prov.provider, last_u.api, last_u.model,
                    COALESCE(tot.input, 0),
                    COALESCE(tot.cache_create, 0),
                    COALESCE(tot.cache_read, 0),
                    COALESCE(tot.output, 0),
                    s.started_at,
                    COALESCE(act.ts, s.started_at),
                    s.ended_at
             FROM sessions s
             LEFT JOIN hosts h ON h.id = s.host_id
             LEFT JOIN tot ON tot.sid = s.id
             LEFT JOIN last_u ON last_u.sid = s.id
             LEFT JOIN act ON act.sid = s.id
             LEFT JOIN prov ON prov.sid = s.id
             WHERE s.started_at >= ",
        );
        out.push_bind_param::<BigInt, _>(&self.since)?;
        out.push_sql(" ORDER BY s.started_at DESC, s.id LIMIT ");
        out.push_bind_param::<BigInt, _>(&self.limit)?;
        Ok(())
    }
}

impl Query for RecentSessionTotals {
    type SqlType = (
        Text,
        Nullable<Text>,
        Nullable<Text>,
        Nullable<Text>,
        Nullable<Text>,
        Nullable<Text>,
        BigInt,
        BigInt,
        BigInt,
        BigInt,
        BigInt,
        BigInt,
        Nullable<BigInt>,
    );
}

impl RunQueryDsl<SqliteConnection> for RecentSessionTotals {}

/// Correlated `MAX(id)` subquery in a `LEFT JOIN` — Diesel 2.3 has no typed form.
#[derive(QueryId)]
pub(crate) struct RecentCalls {
    pub limit: i64,
}

impl QueryFragment<Sqlite> for RecentCalls {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql(
            "SELECT c.id, c.ts, c.session_id, c.surface,
                    c.kind, c.plugin, c.name,
                    c.parent_id, c.ms, c.ok, c.error,
                    h.slug, p.slug, m.slug,
                    u.api, u.input, u.cache_create,
                    u.cache_read, u.output
             FROM calls c
             LEFT JOIN hosts h ON h.id = c.host_id
             LEFT JOIN providers p ON p.id = c.provider_id
             LEFT JOIN models m ON m.id = c.model_id
             LEFT JOIN usage u ON u.call_id = c.id
                  AND u.id = (SELECT MAX(id) FROM usage WHERE call_id = c.id)
             ORDER BY c.id DESC
             LIMIT ",
        );
        out.push_bind_param::<BigInt, _>(&self.limit)?;
        Ok(())
    }
}

impl Query for RecentCalls {
    type SqlType = (
        Integer,
        BigInt,
        Text,
        Text,
        Text,
        Nullable<Text>,
        Nullable<Text>,
        Nullable<Integer>,
        Nullable<Double>,
        Integer,
        Nullable<Text>,
        Nullable<Text>,
        Nullable<Text>,
        Nullable<Text>,
        Nullable<Text>,
        Nullable<BigInt>,
        Nullable<BigInt>,
        Nullable<BigInt>,
        Nullable<BigInt>,
    );
}

impl RunQueryDsl<SqliteConnection> for RecentCalls {}

/// `UNION` of two archive columns plus three `NOT EXISTS` — no typed form that stays one
/// statement. `cutoff` is bound once per comparison.
#[derive(QueryId)]
pub(crate) struct DoomedArchives {
    pub cutoff: i64,
}

impl QueryFragment<Sqlite> for DoomedArchives {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql(
            "SELECT DISTINCT a.id, a.path FROM archive a \
             WHERE a.id IN ( \
               SELECT request_archive FROM call_io \
               WHERE call_id IN (SELECT id FROM calls WHERE ts < ",
        );
        out.push_bind_param::<BigInt, _>(&self.cutoff)?;
        out.push_sql(
            ") AND request_archive IS NOT NULL \
               UNION \
               SELECT response_archive FROM call_io \
               WHERE call_id IN (SELECT id FROM calls WHERE ts < ",
        );
        out.push_bind_param::<BigInt, _>(&self.cutoff)?;
        out.push_sql(
            ") AND response_archive IS NOT NULL \
             ) \
             AND NOT EXISTS ( \
               SELECT 1 FROM call_io c \
               WHERE c.call_id NOT IN (SELECT id FROM calls WHERE ts < ",
        );
        out.push_bind_param::<BigInt, _>(&self.cutoff)?;
        out.push_sql(
            ") \
               AND (c.request_archive = a.id OR c.response_archive = a.id) \
             ) \
             AND NOT EXISTS ( \
               SELECT 1 FROM archive_decisions d WHERE d.archive_id = a.id \
             ) \
             AND NOT EXISTS ( \
               SELECT 1 FROM read_cache r WHERE r.archive_id = a.id \
             )",
        );
        Ok(())
    }
}

impl Query for DoomedArchives {
    type SqlType = (Text, Text);
}

impl RunQueryDsl<SqliteConnection> for DoomedArchives {}

pub(crate) fn purge_related(conn: &mut SqliteConnection, cutoff: i64) -> QueryResult<()> {
    diesel::delete(logs::table)
        .filter(
            logs::ts.lt(cutoff).or(logs::call_id.eq_any(
                calls::table
                    .filter(calls::ts.lt(cutoff))
                    .select(calls::id.nullable()),
            )),
        )
        .execute(conn)?;
    diesel::delete(tokens::table)
        .filter(tokens::ts.lt(cutoff).or(
            tokens::call_id.eq_any(calls::table.filter(calls::ts.lt(cutoff)).select(calls::id)),
        ))
        .execute(conn)?;
    diesel::delete(call_io::table)
        .filter(
            call_io::call_id.eq_any(calls::table.filter(calls::ts.lt(cutoff)).select(calls::id)),
        )
        .execute(conn)?;
    diesel::update(usage::table)
        .filter(
            usage::call_id.eq_any(
                calls::table
                    .filter(calls::ts.lt(cutoff))
                    .select(calls::id.nullable()),
            ),
        )
        .set(usage::call_id.eq(None::<i32>))
        .execute(conn)?;
    diesel::update(measurements::table)
        .filter(
            measurements::call_id.eq_any(
                calls::table
                    .filter(calls::ts.lt(cutoff))
                    .select(calls::id.nullable()),
            ),
        )
        .set(measurements::call_id.eq(None::<i32>))
        .execute(conn)?;
    let old = alias!(calls as old);
    diesel::update(calls::table)
        .filter(
            calls::parent_id.eq_any(
                old.filter(old.field(calls::ts).lt(cutoff))
                    .select(old.field(calls::id).nullable()),
            ),
        )
        .set(calls::parent_id.eq(None::<i32>))
        .execute(conn)?;
    Ok(())
}

/// T352: hook stdin bodies are only read back by short session windows and the OTel export, so
/// they are cleared after `cutoff` while the `calls` row, byte counts, shas and archive columns
/// stay. Clears at most `batch` rows that still hold a body, so one write transaction stays
/// short; returns how many it cleared.
pub(crate) fn clear_hook_bodies(
    conn: &mut SqliteConnection,
    cutoff: i64,
    batch: i64,
) -> QueryResult<usize> {
    let old_hooks = calls::table
        .filter(calls::kind.eq("hook"))
        .filter(calls::ts.lt(cutoff))
        .select(calls::id);
    // An alias, as in `purge_related`: Diesel rejects a subselect of the table being updated.
    let io = alias!(call_io as pending_io);
    let pending = io
        .filter(io.field(call_io::call_id).eq_any(old_hooks))
        .filter(
            io.field(call_io::request_json)
                .is_not_null()
                .or(io.field(call_io::response_json).is_not_null())
                .or(io.field(call_io::request_raw).is_not_null())
                .or(io.field(call_io::response_raw).is_not_null()),
        )
        .select(io.field(call_io::call_id))
        .limit(batch);
    diesel::update(call_io::table)
        .filter(call_io::call_id.eq_any(pending))
        .set((
            call_io::request_json.eq(None::<String>),
            call_io::response_json.eq(None::<String>),
            call_io::request_raw.eq(None::<Vec<u8>>),
            call_io::response_raw.eq(None::<Vec<u8>>),
            // T433: the session fields belong to the body; the row is swept once unreferenced.
            call_io::hook_session_id.eq(None::<i32>),
        ))
        .execute(conn)
}

/// T352: every root the graph index holds rows for.
pub(crate) fn symbol_roots(conn: &mut SqliteConnection) -> QueryResult<Vec<String>> {
    let mut roots: Vec<String> = symbols::table.select(symbols::root).distinct().load(conn)?;
    roots.extend(
        symbol_stale::table
            .select(symbol_stale::root)
            .distinct()
            .load::<String>(conn)?,
    );
    roots.extend(
        extractor::table
            .select(extractor::root)
            .distinct()
            .load::<String>(conn)?,
    );
    roots.sort();
    roots.dedup();
    Ok(roots)
}

pub(crate) fn delete_symbol_root(conn: &mut SqliteConnection, root: &str) -> QueryResult<()> {
    diesel::delete(symbols::table.filter(symbols::root.eq(root))).execute(conn)?;
    diesel::delete(symbol_stale::table.filter(symbol_stale::root.eq(root))).execute(conn)?;
    diesel::delete(extractor::table.filter(extractor::root.eq(root))).execute(conn)?;
    Ok(())
}

pub(crate) fn delete_old_calls(conn: &mut SqliteConnection, cutoff: i64) -> QueryResult<usize> {
    diesel::delete(calls::table.filter(calls::ts.lt(cutoff))).execute(conn)
}

/// T968: a prune can delete an archive body that a surviving `call_io` row still names; the
/// foreign key refuses that delete, and the row keeps its sha and byte count without the body.
pub(crate) fn detach_call_io_archive(conn: &mut SqliteConnection, id: &str) -> QueryResult<()> {
    diesel::update(call_io::table.filter(call_io::request_archive.eq(id)))
        .set(call_io::request_archive.eq(None::<String>))
        .execute(conn)?;
    diesel::update(call_io::table.filter(call_io::response_archive.eq(id)))
        .set(call_io::response_archive.eq(None::<String>))
        .execute(conn)?;
    Ok(())
}

pub(crate) fn purge_archive(conn: &mut SqliteConnection, id: &str) -> QueryResult<()> {
    diesel::delete(archive_decisions::table.filter(archive_decisions::archive_id.eq(id)))
        .execute(conn)?;
    diesel::delete(archive_summaries::table.filter(archive_summaries::archive_id.eq(id)))
        .execute(conn)?;
    diesel::update(read_cache::table.filter(read_cache::archive_id.eq(id)))
        .set(read_cache::archive_id.eq(None::<String>))
        .execute(conn)?;
    diesel::delete(archive::table.filter(archive::id.eq(id))).execute(conn)?;
    Ok(())
}
