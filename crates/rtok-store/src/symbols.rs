// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T8.10: the `graph` plugin's symbol index over SQLite (the only backend after P39).

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::Result;
use diesel::alias;
use diesel::dsl::{count, count_star, exists, min, not};
use diesel::expression_methods::AggregateExpressionMethods;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use super::Store;
use super::schema::{extractor, file_rank, symbol_stale, symbols};
use rtok_plugin_sdk::{SymbolRow, SymbolSpan};

/// The span [`Store::symbol_span`] returns (T474).
pub type Span = SymbolSpan;

/// Columns bound per inserted symbol row. SQLite's default variable limit is 999.
const INSERT_CHUNK: usize = 999 / 16;

#[derive(Insertable)]
#[diesel(table_name = symbols)]
struct NewSymbol<'a> {
    root: &'a str,
    path: &'a str,
    name: &'a str,
    kind: &'a str,
    line: i32,
    is_def: i32,
    file_sha: &'a str,
    mtime: i64,
    size: i64,
    end_line: i32,
    scope: &'a str,
    start_byte: i64,
    end_byte: i64,
    content_hash: &'a str,
    signature: &'a str,
    doc: &'a str,
}

fn fts_tokens(query: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| !t.is_empty())
        .filter(|t| seen.insert(t.to_ascii_lowercase()))
        .map(str::to_string)
        .collect()
}

fn fts_or(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn fts_has(hay: &str, token: &str) -> bool {
    hay.to_ascii_lowercase()
        .contains(&token.to_ascii_lowercase())
}

fn fts_hits(hay: &str, tokens: &[&str]) -> usize {
    tokens.iter().filter(|t| fts_has(hay, t)).count()
}

/// 0 name, 1 signature, 2 doc.
fn fts_tier(name: &str, signature: &str, _doc: &str, tokens: &[&str]) -> u8 {
    if fts_hits(name, tokens) > 0 {
        0
    } else if fts_hits(signature, tokens) > 0 {
        1
    } else {
        2
    }
}

fn fts_coverage(name: &str, signature: &str, doc: &str, tokens: &[&str]) -> usize {
    tokens
        .iter()
        .filter(|t| fts_has(name, t) || fts_has(signature, t) || fts_has(doc, t))
        .count()
}

/// SQLite's default `SQLITE_MAX_VARIABLE_NUMBER` is 999; a BFS level's frontier is chunked
/// below that so a wide fan-out never blows the bind limit in one `eq_any`.
const NAME_CHUNK: usize = 500;

/// One copied symbol row, without `id` (SQLite assigns it) and without `root`
/// (the destination root is written on insert).
type CopiedSymbol = (
    String,
    String,
    String,
    i32,
    i32,
    String,
    i64,
    i64,
    i32,
    String,
);

/// One level of `symbol_impact`'s walk: references of any name in `frontier`, as
/// `(matched name, path, enclosing scope)` — the direct-caller edge (T163.1 BFS, replaces
/// the old `WITH RECURSIVE` base/step: `s.name = w.scope`). The matched name comes back
/// with each row so a caller batching several frontier tips in one query can route a result
/// to the specific chain(s) it extends.
fn impact_refs(
    conn: &mut SqliteConnection,
    root: &str,
    frontier: &[String],
) -> QueryResult<Vec<(String, String, String)>> {
    let mut out = Vec::new();
    for chunk in frontier.chunks(NAME_CHUNK) {
        out.extend(
            symbols::table
                .filter(symbols::root.eq(root))
                .filter(symbols::name.eq_any(chunk))
                .filter(symbols::is_def.eq(0))
                .filter(symbols::name.ne(""))
                .filter(symbols::kind.ne("import"))
                .select((symbols::name, symbols::path, symbols::scope))
                .load::<(String, String, String)>(conn)?,
        );
    }
    Ok(out)
}

/// One level of `symbol_impact`'s walk: definitions in files that import any name in
/// `frontier`, as `(matched name, path, def name)` — the import-follow edge (T163.1 BFS,
/// replaces the old `WITH RECURSIVE` import branch). See [`impact_refs`] for why the
/// matched name comes back with each row.
fn impact_import_follow(
    conn: &mut SqliteConnection,
    root: &str,
    frontier: &[String],
) -> QueryResult<Vec<(String, String, String)>> {
    let (i, d) = alias!(symbols as i, symbols as d);
    let mut out = Vec::new();
    for chunk in frontier.chunks(NAME_CHUNK) {
        out.extend(
            i.inner_join(
                d.on(d
                    .field(symbols::root)
                    .eq(i.field(symbols::root))
                    .and(d.field(symbols::path).eq(i.field(symbols::path)))
                    .and(d.field(symbols::is_def).eq(1))
                    .and(d.field(symbols::name).ne(""))),
            )
            .filter(i.field(symbols::root).eq(root))
            .filter(i.field(symbols::name).eq_any(chunk))
            .filter(i.field(symbols::kind).eq("import"))
            .filter(i.field(symbols::is_def).eq(0))
            .select((
                i.field(symbols::name),
                d.field(symbols::path),
                d.field(symbols::name),
            ))
            .load::<(String, String, String)>(conn)?,
        );
    }
    Ok(out)
}

/// Groups one BFS level's flat query rows `(matched tip, value)` by that tip, deduping a
/// repeated `value` under the same tip — shared by `symbol_impact`'s two edges and
/// `symbol_paths`'s one so each builds a `tip -> Vec<value>` map of candidates to hand back
/// to the partial chain(s) that produced the tip.
fn group_by_tip<T: PartialEq>(rows: Vec<(String, T)>, by_tip: &mut HashMap<String, Vec<T>>) {
    for (tip, value) in rows {
        let values = by_tip.entry(tip).or_default();
        if !values.contains(&value) {
            values.push(value);
        }
    }
}

/// One step of `symbol_paths`'s walk: for every name in `tips`, the reference rows that
/// could extend a chain through it — `(referenced name, enclosing scope)` (T163.1 BFS,
/// replaces the old `WITH RECURSIVE` step: `s.name = w.tip`).
fn path_next_hops(
    conn: &mut SqliteConnection,
    root: &str,
    tips: &[String],
) -> QueryResult<Vec<(String, String)>> {
    let mut out = Vec::new();
    for chunk in tips.chunks(NAME_CHUNK) {
        out.extend(
            symbols::table
                .filter(symbols::root.eq(root))
                .filter(symbols::name.eq_any(chunk))
                .filter(symbols::is_def.eq(0))
                .filter(symbols::scope.ne(""))
                .filter(symbols::kind.ne("import"))
                .select((symbols::name, symbols::scope))
                .load::<(String, String)>(conn)?,
        );
    }
    Ok(out)
}

/// Every row of one file under one root, through the `(root, path)` index.
fn delete_file(conn: &mut SqliteConnection, root: &str, path: &str) -> QueryResult<usize> {
    diesel::delete(symbols::table.filter(symbols::root.eq(root).and(symbols::path.eq(path))))
        .execute(conn)
}

fn note_stale(conn: &mut SqliteConnection, root: &str, path: &str) -> QueryResult<()> {
    diesel::insert_or_ignore_into(symbol_stale::table)
        .values((symbol_stale::root.eq(root), symbol_stale::path.eq(path)))
        .execute(conn)?;
    Ok(())
}

/// One file's rows, inside the caller's transaction: drop what the file had, insert the
/// new tags in chunks, and return how many landed. A tagless file still gets a row so the
/// `file_sha` stands and the next run skips it on the stat alone.
fn replace_one(
    conn: &mut SqliteConnection,
    root: &str,
    path: &str,
    file_sha: &str,
    stat: (i64, i64),
    rows: &[SymbolRow],
) -> QueryResult<usize> {
    diesel::delete(symbols::table.filter(symbols::root.eq(root).and(symbols::path.eq(path))))
        .execute(conn)?;
    if rows.is_empty() {
        diesel::insert_into(symbols::table)
            .values((
                symbols::root.eq(root),
                symbols::path.eq(path),
                symbols::name.eq(""),
                symbols::kind.eq(""),
                symbols::line.eq(0),
                symbols::is_def.eq(0),
                symbols::file_sha.eq(file_sha),
                symbols::mtime.eq(stat.0),
                symbols::size.eq(stat.1),
            ))
            .execute(conn)?;
        return Ok(0);
    }
    for chunk in rows.chunks(INSERT_CHUNK) {
        let values: Vec<NewSymbol> = chunk
            .iter()
            .map(|row| NewSymbol {
                root,
                path,
                name: &row.name,
                kind: &row.kind,
                line: row.line,
                is_def: i32::from(row.is_def),
                file_sha,
                mtime: stat.0,
                size: stat.1,
                end_line: row.end_line,
                scope: &row.scope,
                start_byte: row.start_byte,
                end_byte: row.end_byte,
                content_hash: &row.content_hash,
                signature: &row.signature,
                doc: &row.doc,
            })
            .collect();
        diesel::insert_into(symbols::table)
            .values(&values)
            .execute(conn)?;
    }
    clear_stale(conn, root, path)?;
    Ok(rows.len())
}

fn clear_stale(conn: &mut SqliteConnection, root: &str, path: &str) -> QueryResult<()> {
    diesel::delete(
        symbol_stale::table.filter(symbol_stale::root.eq(root).and(symbol_stale::path.eq(path))),
    )
    .execute(conn)?;
    Ok(())
}

impl Store {
    /// Rows indexed under one repo root (T8.3). Every symbol call is scoped to a root, so
    /// two repos in the one store (D8) never evict or answer for each other.
    pub fn symbol_count(&self, root: &str) -> Result<i64> {
        let mut conn = self.lock()?;
        Ok(symbols::table
            .filter(symbols::root.eq(root))
            .count()
            .get_result(&mut *conn)?)
    }

    /// Distinct indexed file paths under `root` (T68.3 `graph status`).
    pub fn symbol_file_count(&self, root: &str) -> Result<i64> {
        let mut conn = self.lock()?;
        Ok(symbols::table
            .filter(symbols::root.eq(root))
            .select(symbols::path)
            .distinct()
            .count()
            .get_result(&mut *conn)?)
    }

    /// Paths still carrying the T8.3 stale mark (hook delete, not yet re-indexed).
    pub fn symbol_stale_paths(&self, root: &str) -> Result<Vec<String>> {
        let mut conn = self.lock()?;
        Ok(symbol_stale::table
            .filter(symbol_stale::root.eq(root))
            .select(symbol_stale::path)
            .order(symbol_stale::path.asc())
            .load(&mut *conn)?)
    }

    /// Pending files: hook-staled rows plus indexed paths whose stat no longer matches disk.
    pub fn symbol_pending(&self, root: &str, root_path: &std::path::Path) -> Result<Vec<String>> {
        let mut pending: HashSet<String> = self.symbol_stale_paths(root)?.into_iter().collect();
        for (path, (_, mtime, size)) in self.symbol_stats(root)? {
            let abs = root_path.join(&path);
            let stat = abs
                .metadata()
                .map(|md| {
                    let mtime = md
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos() as i64)
                        .unwrap_or(0);
                    (mtime, md.len() as i64)
                })
                .unwrap_or((0, 0));
            if stat != (mtime, size) && stat != (0, 0) {
                pending.insert(path);
            }
        }
        let mut out: Vec<String> = pending.into_iter().collect();
        out.sort();
        Ok(out)
    }

    /// What the index knows about one file: `(sha256, mtime_nanos, size)` (T8.4). A caller
    /// whose stat matches is skipped without the file being opened.
    pub fn symbol_stat(&self, root: &str, path: &str) -> Result<Option<(String, i64, i64)>> {
        let mut conn = self.lock()?;
        Ok(symbols::table
            .filter(symbols::root.eq(root).and(symbols::path.eq(path)))
            .select((symbols::file_sha, symbols::mtime, symbols::size))
            .first::<(String, i64, i64)>(&mut *conn)
            .optional()?)
    }

    pub fn symbol_stats(&self, root: &str) -> Result<HashMap<String, (String, i64, i64)>> {
        let mut conn = self.lock()?;
        let rows: Vec<(String, String, i64, i64)> = symbols::table
            .filter(symbols::root.eq(root))
            .select((
                symbols::path,
                symbols::file_sha,
                symbols::mtime,
                symbols::size,
            ))
            .distinct()
            .load(&mut *conn)?;
        Ok(rows
            .into_iter()
            .map(|(p, s, m, z)| (p, (s, m, z)))
            .collect())
    }

    /// Copy one root's symbol rows onto another, skipping paths `to` already has and paths in
    /// a directory of `skip_under` (relative, no trailing slash).
    /// `id` is omitted so SQLite assigns it. The extractor fingerprint is copied when
    /// `to` has none, so the next index does not treat the root as a new extractor and
    /// delete the rows. One `BEGIN IMMEDIATE`.
    pub fn copy_symbol_rows(&self, from: &str, to: &str, skip_under: &[&str]) -> Result<usize> {
        if from == to {
            return Ok(0);
        }
        let mut conn = self.lock()?;
        Ok(
            conn.immediate_transaction::<usize, diesel::result::Error, _>(|conn| {
                let existing: Vec<String> = symbols::table
                    .filter(symbols::root.eq(to))
                    .select(symbols::path)
                    .distinct()
                    .load(conn)?;
                let have: HashSet<String> = existing.into_iter().collect();
                let rows: Vec<CopiedSymbol> = symbols::table
                    .filter(symbols::root.eq(from))
                    .select((
                        symbols::path,
                        symbols::name,
                        symbols::kind,
                        symbols::line,
                        symbols::is_def,
                        symbols::file_sha,
                        symbols::mtime,
                        symbols::size,
                        symbols::end_line,
                        symbols::scope,
                    ))
                    .load(conn)?;
                let rows: Vec<_> = rows
                    .into_iter()
                    .filter(|row| {
                        let skipped = skip_under.iter().any(|dir| {
                            let rest = row.0.strip_prefix(dir);
                            rest.is_some_and(|r| r.starts_with('/'))
                        });
                        !skipped && !have.contains(&row.0)
                    })
                    .collect();
                let inserted = rows.len();
                for chunk in rows.chunks(INSERT_CHUNK) {
                    let values: Vec<_> = chunk
                        .iter()
                        .map(
                            |(
                                path,
                                name,
                                kind,
                                line,
                                is_def,
                                sha,
                                mtime,
                                size,
                                end_line,
                                scope,
                            )| {
                                (
                                    symbols::root.eq(to),
                                    symbols::path.eq(path),
                                    symbols::name.eq(name),
                                    symbols::kind.eq(kind),
                                    symbols::line.eq(line),
                                    symbols::is_def.eq(is_def),
                                    symbols::file_sha.eq(sha),
                                    symbols::mtime.eq(mtime),
                                    symbols::size.eq(size),
                                    symbols::end_line.eq(end_line),
                                    symbols::scope.eq(scope),
                                )
                            },
                        )
                        .collect();
                    diesel::insert_into(symbols::table)
                        .values(&values)
                        .execute(conn)?;
                }
                let dest_fp: Option<String> = extractor::table
                    .filter(extractor::root.eq(to))
                    .select(extractor::fingerprint)
                    .first(conn)
                    .optional()?;
                if dest_fp.is_none() {
                    let src_fp: Option<String> = extractor::table
                        .filter(extractor::root.eq(from))
                        .select(extractor::fingerprint)
                        .first(conn)
                        .optional()?;
                    if let Some(fp) = src_fp {
                        diesel::insert_into(extractor::table)
                            .values((extractor::root.eq(to), extractor::fingerprint.eq(fp)))
                            .execute(conn)?;
                    }
                }
                Ok(inserted)
            })?,
        )
    }

    /// Record a new stat for a file whose content hashed the same (T8.4): the rows stand,
    /// only the freshness key moves, so the next run skips it on the stat alone.
    pub fn touch_symbols(&self, root: &str, path: &str, mtime: i64, size: i64) -> Result<()> {
        let mut conn = self.lock()?;
        diesel::update(symbols::table.filter(symbols::root.eq(root).and(symbols::path.eq(path))))
            .set((symbols::mtime.eq(mtime), symbols::size.eq(size)))
            .execute(&mut *conn)?;
        Ok(())
    }

    pub fn replace_symbol_files(
        &self,
        root: &str,
        files: &rtok_plugin_sdk::SymbolFileBatch,
    ) -> Result<usize> {
        if files.is_empty() {
            return Ok(0);
        }
        let mut conn = self.lock()?;
        // T352: every write here is `immediate_transaction` (BEGIN IMMEDIATE): a deferred one that
        // has to upgrade after another connection committed (the session-start housekeeping
        // thread) gets SQLITE_BUSY at once in WAL, skipping the busy handler.
        Ok(
            conn.immediate_transaction::<usize, diesel::result::Error, _>(|conn| {
                let mut inserted = 0usize;
                for (path, file_sha, stat, rows) in files {
                    inserted += replace_one(conn, root, path, file_sha, *stat, rows)?;
                }
                Ok(inserted)
            })?,
        )
    }

    pub fn replace_symbols(
        &self,
        root: &str,
        path: &str,
        file_sha: &str,
        stat: (i64, i64),
        rows: &[SymbolRow],
    ) -> Result<usize> {
        let mut conn = self.lock()?;
        // One transaction per file: thousands of autocommit inserts dominated index time.
        // PERF(T35.3) where: the row loop below, per file of a cold index (this repo: 18 093
        // rows over 127 files). What: multi-row INSERTs chunked under SQLite's variable limit,
        // one transaction per batch of files. Why: ~140 single-row INSERTs per file. Not yet
        // measured apart from the parse — measure before changing.
        Ok(
            conn.immediate_transaction::<usize, diesel::result::Error, _>(|conn| {
                replace_one(conn, root, path, file_sha, stat, rows)
            })?,
        )
    }

    pub fn delete_symbols_missing(&self, root: &str, keep: &HashSet<String>) -> Result<usize> {
        let mut conn = self.lock()?;
        if keep.is_empty() {
            return Ok(
                diesel::delete(symbols::table.filter(symbols::root.eq(root)))
                    .execute(&mut *conn)?,
            );
        }
        let have: Vec<String> = symbols::table
            .filter(symbols::root.eq(root))
            .select(symbols::path)
            .distinct()
            .load(&mut *conn)?;
        let missing: Vec<&str> = have
            .iter()
            .filter(|p| !keep.contains(p.as_str()))
            .map(String::as_str)
            .collect();
        if missing.is_empty() {
            return Ok(0);
        }
        Ok(diesel::delete(
            symbols::table.filter(symbols::root.eq(root).and(symbols::path.eq_any(&missing))),
        )
        .execute(&mut *conn)?)
    }

    /// Drop rows for one canonical absolute file path. No indexing on the hook path.
    /// A same-named file in another repo survives: the row must match on `(root, path)`.
    /// The caller does not know the root, so every `/` split of the path is tried through
    /// [`Store::mark_symbols_stale_in`] — a dozen indexed point deletes, where
    /// `WHERE ? = root || '/' || path` scanned the whole table on each `Edit`/`Write`.
    pub fn mark_symbols_stale(&self, abs_path: &str) -> Result<()> {
        let mut conn = self.lock()?;
        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
            for (i, _) in abs_path.match_indices('/') {
                let root = &abs_path[..i];
                let rel = &abs_path[i + 1..];
                if delete_file(conn, root, rel)? > 0 {
                    note_stale(conn, root, rel)?;
                }
            }
            Ok(())
        })?;
        Ok(())
    }

    /// Drop the rows of `rel_path` under `root`: one indexed delete for a caller that knows
    /// the root.
    pub fn mark_symbols_stale_in(&self, root: &str, rel_path: &str) -> Result<()> {
        let mut conn = self.lock()?;
        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
            if delete_file(conn, root, rel_path)? > 0 {
                note_stale(conn, root, rel_path)?;
            }
            Ok(())
        })?;
        Ok(())
    }

    pub fn extractor_fingerprint(&self, root: &str) -> Result<Option<String>> {
        let mut conn = self.lock()?;
        Ok(extractor::table
            .filter(extractor::root.eq(root))
            .select(extractor::fingerprint)
            .first(&mut *conn)
            .optional()?)
    }

    pub fn set_extractor_fingerprint(&self, root: &str, fp: &str) -> Result<()> {
        let mut conn = self.lock()?;
        diesel::insert_into(extractor::table)
            .values((extractor::root.eq(root), extractor::fingerprint.eq(fp)))
            .on_conflict(extractor::root)
            .do_update()
            .set(extractor::fingerprint.eq(fp))
            .execute(&mut *conn)?;
        Ok(())
    }

    pub fn symbol_indexed_at(&self, root: &str) -> Result<Option<i64>> {
        let mut conn = self.lock()?;
        let indexed_at: Option<Option<i64>> = extractor::table
            .filter(extractor::root.eq(root))
            .select(extractor::indexed_at)
            .first(&mut *conn)
            .optional()?;
        Ok(indexed_at.flatten())
    }

    pub fn touch_symbol_indexed_at(&self, root: &str, ts: i64) -> Result<()> {
        let mut conn = self.lock()?;
        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
            let existing_fingerprint: Option<String> = extractor::table
                .filter(extractor::root.eq(root))
                .select(extractor::fingerprint)
                .first(conn)
                .optional()?;
            diesel::insert_into(extractor::table)
                .values((
                    extractor::root.eq(root),
                    extractor::fingerprint.eq(existing_fingerprint.unwrap_or_default()),
                    extractor::indexed_at.eq(ts),
                ))
                .on_conflict(extractor::root)
                .do_update()
                .set(extractor::indexed_at.eq(ts))
                .execute(conn)?;
            Ok(())
        })?;
        Ok(())
    }

    /// T130.3: definitions in one file as `(name, kind, line, end_line)`, ordered by line.
    pub fn symbol_file_defs(
        &self,
        root: &str,
        path: &str,
    ) -> Result<Vec<(String, String, i32, i32)>> {
        let mut conn = self.lock()?;
        Ok(symbols::table
            .filter(
                symbols::root
                    .eq(root)
                    .and(symbols::path.eq(path))
                    .and(symbols::is_def.eq(1)),
            )
            .order(symbols::line.asc())
            .select((
                symbols::name,
                symbols::kind,
                symbols::line,
                symbols::end_line,
            ))
            .load(&mut *conn)?)
    }

    /// Definitions of `name` as `(path, kind, line)`, ordered by path then line (T8.2 `symbol`).
    pub fn symbol_defs(&self, root: &str, name: &str) -> Result<Vec<(String, String, i32, i32)>> {
        let mut conn = self.lock()?;
        Ok(symbols::table
            .filter(
                symbols::root
                    .eq(root)
                    .and(symbols::name.eq(name))
                    .and(symbols::is_def.eq(1)),
            )
            .order((symbols::path.asc(), symbols::line.asc()))
            .select((
                symbols::path,
                symbols::kind,
                symbols::line,
                symbols::end_line,
            ))
            .load(&mut *conn)?)
    }

    /// Byte span of one definition (T474).
    pub fn symbol_span(
        &self,
        root: &str,
        path: &str,
        name: &str,
        kind: &str,
        line: i32,
    ) -> Result<Option<Span>> {
        let mut conn = self.lock()?;
        let row: Option<(i64, i64, String, String)> = symbols::table
            .filter(symbols::root.eq(root))
            .filter(symbols::path.eq(path))
            .filter(symbols::name.eq(name))
            .filter(symbols::kind.eq(kind))
            .filter(symbols::line.eq(line))
            .filter(symbols::is_def.eq(1))
            .select((
                symbols::start_byte,
                symbols::end_byte,
                symbols::content_hash,
                symbols::file_sha,
            ))
            .first(&mut *conn)
            .optional()?;
        Ok(
            row.map(|(start_byte, end_byte, content_hash, file_sha)| Span {
                start_byte: u64::try_from(start_byte).unwrap_or(0),
                end_byte: u64::try_from(end_byte).unwrap_or(0),
                content_hash,
                file_sha,
            }),
        )
    }

    /// Definitions matching `query` in name, signature, or doc (T474).
    ///
    /// A name that contains a query token outranks a signature hit, which outranks a doc hit.
    pub fn symbol_fts(
        &self,
        root: &str,
        query: &str,
        limit: i64,
    ) -> Result<Vec<(String, String, String, i32)>> {
        let limit = usize::try_from(limit).unwrap_or(0);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let tokens = fts_tokens(query);
        let match_query = fts_or(&tokens);
        if match_query.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = self.lock()?;
        let mut hits = super::sql_ext::SearchSymbols {
            root: root.to_string(),
            query: match_query,
            limit: 200,
        }
        .load::<(String, String, String, i32, String, String, f64)>(&mut *conn)?;
        let tokens: Vec<&str> = tokens.iter().map(String::as_str).collect();
        hits.sort_by(|a, b| {
            fts_tier(&a.1, &a.4, &a.5, &tokens)
                .cmp(&fts_tier(&b.1, &b.4, &b.5, &tokens))
                .then(
                    fts_coverage(&b.1, &b.4, &b.5, &tokens)
                        .cmp(&fts_coverage(&a.1, &a.4, &a.5, &tokens)),
                )
                .then(a.6.partial_cmp(&b.6).unwrap_or(std::cmp::Ordering::Equal))
                .then(a.0.cmp(&b.0))
                .then(a.3.cmp(&b.3))
                .then(a.1.cmp(&b.1))
        });
        hits.truncate(limit);
        Ok(hits
            .into_iter()
            .map(|(path, name, kind, line, _, _, _)| (path, name, kind, line))
            .collect())
    }

    /// Reference sites of `name` as `(path, line)`, ordered by path then line (T8.2 `callers`).
    pub fn symbol_refs(&self, root: &str, name: &str) -> Result<Vec<(String, i32)>> {
        let mut conn = self.lock()?;
        Ok(symbols::table
            .filter(
                symbols::root
                    .eq(root)
                    .and(symbols::name.eq(name))
                    .and(symbols::is_def.eq(0))
                    .and(symbols::kind.ne("import")),
            )
            .order((symbols::path.asc(), symbols::line.asc()))
            .select((symbols::path, symbols::line))
            .load(&mut *conn)?)
    }

    /// Callees of each definition of `name`: `(path, line, callee, first_ref_line)` (T68.2).
    /// A reference row counts when it shares the definition's path and its `scope` is the
    /// definition's name; results are ordered by definition site then first reference line.
    // Self-join `symbols` against itself, grouped by `(d.path, d.line, r.name)`: Diesel's
    // `alias!` self-join fields (`AliasedField`) have no `IsContainedInGroupBy` bridge (only
    // `ValidGrouping<()>`, i.e. no `GROUP BY` at all), so the `GROUP BY` itself can't be
    // expressed through the typed DSL — the join and filter can. Load the ungrouped rows with
    // the typed DSL and do the `GROUP BY MIN(r.line)` / `ORDER BY` in Rust instead (T163.1).
    pub fn symbol_callees(
        &self,
        root: &str,
        name: &str,
    ) -> Result<Vec<(String, i32, String, i32)>> {
        let (d, r) = alias!(symbols as d, symbols as r);
        let mut conn = self.lock()?;
        let rows: Vec<(String, i32, String, i32)> = d
            .inner_join(
                r.on(r
                    .field(symbols::root)
                    .eq(d.field(symbols::root))
                    .and(r.field(symbols::path).eq(d.field(symbols::path)))
                    .and(r.field(symbols::is_def).eq(0))
                    .and(r.field(symbols::scope).eq(d.field(symbols::name)))),
            )
            .filter(
                d.field(symbols::root)
                    .eq(root)
                    .and(d.field(symbols::is_def).eq(1))
                    .and(d.field(symbols::name).eq(name))
                    .and(r.field(symbols::name).ne("")),
            )
            .select((
                d.field(symbols::path),
                d.field(symbols::line),
                r.field(symbols::name),
                r.field(symbols::line),
            ))
            .load(&mut *conn)?;

        // GROUP BY (path, line, callee), keeping MIN(r.line) as first_line.
        let mut groups: HashMap<(String, i32, String), i32> = HashMap::new();
        for (path, line, callee, r_line) in rows {
            groups
                .entry((path, line, callee))
                .and_modify(|first| *first = (*first).min(r_line))
                .or_insert(r_line);
        }
        let mut out: Vec<(String, i32, String, i32)> = groups
            .into_iter()
            .map(|((path, line, callee), first_line)| (path, line, callee, first_line))
            .collect();
        // ORDER BY d.path, d.line, first_line, r.name
        out.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.cmp(&b.1))
                .then(a.3.cmp(&b.3))
                .then(a.2.cmp(&b.2))
        });
        Ok(out)
    }

    /// Reference sites of `name` collapsed to one row per calling definition (T8.5):
    /// `(path, scope, count, first line)`, `scope` empty at file level. The grouping is the
    /// call edge — the same rows ungrouped are `symbol_refs`.
    pub fn symbol_ref_groups(
        &self,
        root: &str,
        name: &str,
    ) -> Result<Vec<(String, String, i64, i32)>> {
        let mut conn = self.lock()?;
        let rows: Vec<(String, String, i64, Option<i32>)> = symbols::table
            .filter(
                symbols::root
                    .eq(root)
                    .and(symbols::name.eq(name))
                    .and(symbols::is_def.eq(0))
                    .and(symbols::kind.ne("import")),
            )
            .group_by((symbols::path, symbols::scope))
            .order((symbols::path.asc(), symbols::scope.asc()))
            .select((
                symbols::path,
                symbols::scope,
                count_star(),
                min(symbols::line),
            ))
            .load(&mut *conn)?;
        Ok(rows
            .into_iter()
            .map(|(p, s, n, l)| (p, s, n, l.unwrap_or(0)))
            .collect())
    }

    /// Reference kinds of `name` per calling definition: distinct `(path, scope, kind)`
    /// (`call`, `type`, `class`, `implementation`), imports excluded. `rtok review` uses it
    /// to keep a call from linking to a type and a type use from linking to a function.
    pub fn symbol_ref_kinds(
        &self,
        root: &str,
        name: &str,
    ) -> Result<Vec<(String, String, String)>> {
        let mut conn = self.lock()?;
        Ok(symbols::table
            .filter(
                symbols::root
                    .eq(root)
                    .and(symbols::name.eq(name))
                    .and(symbols::is_def.eq(0))
                    .and(symbols::kind.ne("import")),
            )
            .select((symbols::path, symbols::scope, symbols::kind))
            .distinct()
            .order((
                symbols::path.asc(),
                symbols::scope.asc(),
                symbols::kind.asc(),
            ))
            .load(&mut *conn)?)
    }

    pub fn has_symbol_def(&self, root: &str, name: &str) -> Result<bool> {
        Ok(!self.symbol_defs(root, name)?.is_empty())
    }

    pub fn symbol_ref_count(&self, root: &str, name: &str) -> Result<i64> {
        Ok(self.symbol_refs(root, name)?.len() as i64)
    }

    /// T52.3: names under `root` ranked by reference count, with one def site
    /// `(path, line)`. `ORDER BY refs DESC, name ASC` (byte-stable). The def
    /// site is first by `path ASC, line ASC`. Import rows are not refs.
    pub fn symbol_top_refs(
        &self,
        root: &str,
        limit: i64,
    ) -> Result<Vec<(String, i64, String, i32)>> {
        let (d, r, e) = alias!(symbols as d, symbols as r, symbols as e);
        let mut conn = self.lock()?;
        let refs_count = || {
            r.filter(r.field(symbols::root).eq(d.field(symbols::root)))
                .filter(r.field(symbols::name).eq(d.field(symbols::name)))
                .filter(r.field(symbols::is_def).eq(0))
                .filter(r.field(symbols::kind).ne("import"))
                .count()
                .single_value()
        };
        let earlier_def_exists = exists(
            e.filter(e.field(symbols::root).eq(d.field(symbols::root)))
                .filter(e.field(symbols::name).eq(d.field(symbols::name)))
                .filter(e.field(symbols::is_def).eq(1))
                .filter(
                    e.field(symbols::path).lt(d.field(symbols::path)).or(e
                        .field(symbols::path)
                        .eq(d.field(symbols::path))
                        .and(e.field(symbols::line).lt(d.field(symbols::line)))),
                ),
        );
        let rows: Vec<(String, Option<i64>, String, i32)> = d
            .filter(d.field(symbols::root).eq(root))
            .filter(d.field(symbols::is_def).eq(1))
            .filter(d.field(symbols::name).ne(""))
            .filter(not(earlier_def_exists))
            .select((
                d.field(symbols::name),
                refs_count(),
                d.field(symbols::path),
                d.field(symbols::line),
            ))
            .order((refs_count().desc(), d.field(symbols::name).asc()))
            .limit(limit)
            .load(&mut *conn)?;
        Ok(rows
            .into_iter()
            .map(|(name, refs, path, line)| (name, refs.unwrap_or(0), path, line))
            .collect())
    }

    /// T370: one row per `(name, path, is_def)` with its row count, imports and nameless rows
    /// left out. The file graph is built from this single scan, in memory.
    pub fn symbol_file_scan(&self, root: &str) -> Result<Vec<(String, String, bool, i64)>> {
        let mut conn = self.lock()?;
        let rows: Vec<(String, String, i32, i64)> = symbols::table
            .filter(
                symbols::root
                    .eq(root)
                    .and(symbols::name.ne(""))
                    .and(symbols::kind.ne("import")),
            )
            .group_by((symbols::name, symbols::path, symbols::is_def))
            .select((symbols::name, symbols::path, symbols::is_def, count_star()))
            .order((
                symbols::name.asc(),
                symbols::path.asc(),
                symbols::is_def.asc(),
            ))
            .load(&mut *conn)?;
        Ok(rows
            .into_iter()
            .map(|(name, path, is_def, n)| (name, path, is_def != 0, n))
            .collect())
    }

    /// T370: the stored file-graph document of `root`, if an index run wrote one.
    pub fn file_rank_get(&self, root: &str) -> Result<Option<String>> {
        let mut conn = self.lock()?;
        Ok(file_rank::table
            .find(root)
            .select(file_rank::graph)
            .first(&mut *conn)
            .optional()?)
    }

    pub fn file_rank_put(&self, root: &str, graph: &str) -> Result<()> {
        let mut conn = self.lock()?;
        diesel::insert_into(file_rank::table)
            .values((file_rank::root.eq(root), file_rank::graph.eq(graph)))
            .on_conflict(file_rank::root)
            .do_update()
            .set(file_rank::graph.eq(graph))
            .execute(&mut *conn)?;
        Ok(())
    }

    /// T52.4: definitions with no same-name reference row under `root`,
    /// as `(path, name, kind, line)`. Name-based, like `callers`: a shared
    /// name keeps every same-named definition live. Callers filter pub,
    /// trait impls, tests and macros from this candidate set.
    pub fn symbol_dead_candidates(&self, root: &str) -> Result<Vec<(String, String, String, i32)>> {
        let (d, r) = alias!(symbols as d, symbols as r);
        let mut conn = self.lock()?;
        let has_ref = exists(
            r.filter(r.field(symbols::root).eq(d.field(symbols::root)))
                .filter(r.field(symbols::name).eq(d.field(symbols::name)))
                .filter(r.field(symbols::is_def).eq(0))
                .filter(r.field(symbols::kind).ne("import")),
        );
        Ok(d.filter(d.field(symbols::root).eq(root))
            .filter(d.field(symbols::is_def).eq(1))
            .filter(d.field(symbols::name).ne(""))
            .filter(not(has_ref))
            .order((d.field(symbols::path).asc(), d.field(symbols::line).asc()))
            .select((
                d.field(symbols::path),
                d.field(symbols::name),
                d.field(symbols::kind),
                d.field(symbols::line),
            ))
            .load(&mut *conn)?)
    }

    /// T329.5: which of `names` have a non-import reference row under `root`. The scoped `dead`
    /// asks it of the other projects of a scope, so a definition only a linked project calls
    /// stays live; the same rule as the `has_ref` test of [`Self::symbol_dead_candidates`].
    pub fn symbol_referenced_names(&self, root: &str, names: &[String]) -> Result<HashSet<String>> {
        let mut conn = self.lock()?;
        let mut out = HashSet::new();
        for chunk in names.chunks(NAME_CHUNK) {
            out.extend(
                symbols::table
                    .filter(symbols::root.eq(root))
                    .filter(symbols::name.eq_any(chunk))
                    .filter(symbols::is_def.eq(0))
                    .filter(symbols::kind.ne("import"))
                    .select(symbols::name)
                    .distinct()
                    .load::<String>(&mut *conn)?,
            );
        }
        Ok(out)
    }

    /// Callers of `name` out to `depth`, each `(path, scope)` at its first depth (T8.13).
    ///
    /// Level-by-level BFS (T163.1) over the same two edges the old `WITH RECURSIVE` walk
    /// used: a direct reference (`impact_refs`, edge `s.name = frontier`) and an
    /// import-follow hop (`impact_import_follow`, edge `i.name = frontier`). Like
    /// `symbol_paths`, each partial chain keeps its own `seen` set (the old CTE's per-row
    /// `seen`, seeded with the depth-1 row's own output — not `name` itself, matching the
    /// old seed exactly) rather than a global visited set: a name already on *this* chain's
    /// history is never re-emitted for it, but an unrelated chain that never saw that name
    /// still can, and does, produce its own row. Both edges are tried from every chain's
    /// tip at every level, exactly like the CTE's two recursive branches. `results` is keyed
    /// by `(path, scope)` and filled in strictly increasing depth order, so the first write
    /// per key is its minimum depth — the old `GROUP BY path, scope` + `MIN(depth)`.
    pub fn symbol_impact(
        &self,
        root: &str,
        name: &str,
        depth: u32,
    ) -> Result<Vec<(u32, String, String)>> {
        struct Chain {
            tip: String,
            seen: HashSet<String>,
        }

        let max_depth = depth.clamp(1, 4);
        let mut conn = self.lock()?;
        let mut results: HashMap<(String, String), u32> = HashMap::new();
        let mut current = vec![Chain {
            tip: name.to_string(),
            seen: HashSet::new(),
        }];
        let mut level: u32 = 1;
        while !current.is_empty() && level <= max_depth {
            let tips: Vec<String> = {
                let set: HashSet<&str> = current.iter().map(|c| c.tip.as_str()).collect();
                set.into_iter().map(str::to_string).collect()
            };
            let mut by_tip: HashMap<String, Vec<(String, String)>> = HashMap::new();
            group_by_tip(
                impact_refs(&mut conn, root, &tips)?
                    .into_iter()
                    .map(|(tip, path, scope)| (tip, (path, scope)))
                    .collect(),
                &mut by_tip,
            );
            group_by_tip(
                impact_import_follow(&mut conn, root, &tips)?
                    .into_iter()
                    .map(|(tip, path, def_name)| (tip, (path, def_name)))
                    .collect(),
                &mut by_tip,
            );

            let mut next = Vec::new();
            for c in &current {
                let Some(edges) = by_tip.get(&c.tip) else {
                    continue;
                };
                for (path, out_name) in edges {
                    if c.seen.contains(out_name) {
                        continue;
                    }
                    results
                        .entry((path.clone(), out_name.clone()))
                        .or_insert(level);
                    // The old CTE only recurses through a row whose `scope` isn't empty
                    // (`impact_refs` never returns one anyway: `name.ne("")` on the frontier
                    // side blocks it), but `impact_import_follow` has no such filter on the
                    // import row's own name, so an empty `out_name` reaching here (from a
                    // top-level reference with no enclosing scope) must stop here rather
                    // than become the next tip -- else it would query `impact_import_follow`
                    // with an empty frontier name and could follow an import row the CTE
                    // never would (PR #206 review).
                    if out_name.is_empty() {
                        continue;
                    }
                    let mut seen = c.seen.clone();
                    seen.insert(out_name.clone());
                    next.push(Chain {
                        tip: out_name.clone(),
                        seen,
                    });
                }
            }
            current = next;
            level += 1;
        }
        let mut out: Vec<(u32, String, String)> = results
            .into_iter()
            .map(|((path, scope), depth)| (depth, path, scope))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        Ok(out)
    }

    /// T68.1: distinct definition names starting with `prefix`, best `limit` by
    /// reference count (ties by name, byte-stable) — `explore`'s fallback when a
    /// query token is not an exact definition name.
    pub fn symbol_name_prefix(&self, root: &str, prefix: &str, limit: i64) -> Result<Vec<String>> {
        // Only the correlated subquery needs a second `symbols` occurrence (`r`); the outer
        // query groups by the real `symbols::name` column, which keeps `GROUP BY` on a plain
        // (non-aliased) column — see `symbol_callees` for why an aliased self-join can't.
        let r = alias!(symbols as r);
        let mut conn = self.lock()?;
        let escaped = prefix
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let like = format!("{escaped}%");
        let refs_count = || {
            r.filter(r.field(symbols::root).eq(root))
                .filter(r.field(symbols::name).eq(symbols::name))
                .filter(r.field(symbols::is_def).eq(0))
                .filter(r.field(symbols::kind).ne("import"))
                .count()
                .single_value()
        };
        let rows: Vec<(String, Option<i64>)> = symbols::table
            .filter(symbols::root.eq(root))
            .filter(symbols::is_def.eq(1))
            .filter(symbols::name.ne(""))
            .filter(symbols::name.like(&like).escape('\\'))
            .group_by(symbols::name)
            .select((symbols::name, refs_count()))
            .order((refs_count().desc(), symbols::name.asc()))
            .limit(limit)
            .load(&mut *conn)?;
        Ok(rows.into_iter().map(|(name, _)| name).collect())
    }

    /// T68.1: call chains `from → … → to` walked in the caller direction (the
    /// `impact` edges: each step is a definition that references the previous
    /// one). Simple paths only — a name already in the chain is never revisited
    /// and a branch stops growing once it reaches `to` — so cycles terminate and
    /// the shortest forms come first. `explore` prints these between the symbols
    /// a question resolved to; T68.4 reuses the query for `impact --to`.
    pub fn symbol_paths(
        &self,
        root: &str,
        from: &str,
        to: &str,
        depth: u32,
    ) -> Result<Vec<String>> {
        // Simple-path enumeration (T163.1 BFS), one level of `path_next_hops` per step: unlike
        // `symbol_impact`, every distinct chain string matters here, not just the shortest
        // reach of a name, so each partial path keeps its own `seen` set of every name already
        // on it (the old CTE's per-row `seen` chain) rather than a global visited set. A path
        // whose tip already equals `to` stops extending (mirrors the old `WHERE w.tip != ?`)
        // but was already recorded as a result at the depth it reached `to`.
        struct Partial {
            chain: String,
            tip: String,
            seen: HashSet<String>,
        }

        let max_depth = depth.clamp(1, 4);
        let mut conn = self.lock()?;
        let mut results: Vec<(u32, String)> = Vec::new();
        if from == to {
            results.push((1, from.to_string()));
        }
        let mut current = vec![Partial {
            chain: from.to_string(),
            tip: from.to_string(),
            seen: HashSet::from([from.to_string()]),
        }];
        let mut level: u32 = 1;
        while level < max_depth {
            let active: Vec<&Partial> = current.iter().filter(|p| p.tip != to).collect();
            if active.is_empty() {
                break;
            }
            let tips: Vec<String> = {
                let set: HashSet<&str> = active.iter().map(|p| p.tip.as_str()).collect();
                set.into_iter().map(str::to_string).collect()
            };
            let mut by_tip: HashMap<String, Vec<String>> = HashMap::new();
            group_by_tip(path_next_hops(&mut conn, root, &tips)?, &mut by_tip);

            let mut next = Vec::new();
            for p in active {
                let Some(scopes) = by_tip.get(&p.tip) else {
                    continue;
                };
                for scope in scopes {
                    if p.seen.contains(scope) {
                        continue;
                    }
                    let chain = format!("{} → {}", p.chain, scope);
                    if scope == to {
                        results.push((level + 1, chain.clone()));
                    }
                    let mut seen = p.seen.clone();
                    seen.insert(scope.clone());
                    next.push(Partial {
                        chain,
                        tip: scope.clone(),
                        seen,
                    });
                }
            }
            current = next;
            level += 1;
        }

        results.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        Ok(results.into_iter().map(|(_, chain)| chain).collect())
    }

    /// T68.6: import rows of `path` as `(name, line)`, first-seen order.
    pub fn symbol_imports(&self, root: &str, path: &str) -> Result<Vec<(String, i32)>> {
        let mut conn = self.lock()?;
        Ok(symbols::table
            .filter(
                symbols::root
                    .eq(root)
                    .and(symbols::path.eq(path))
                    .and(symbols::kind.eq("import"))
                    .and(symbols::is_def.eq(0)),
            )
            .order(symbols::line.asc())
            .select((symbols::name, symbols::line))
            .load(&mut *conn)?)
    }

    /// T68.6: files that import `module` as `(path, line)`.
    pub fn symbol_importers(&self, root: &str, module: &str) -> Result<Vec<(String, i32)>> {
        let mut conn = self.lock()?;
        Ok(symbols::table
            .filter(
                symbols::root
                    .eq(root)
                    .and(symbols::name.eq(module))
                    .and(symbols::kind.eq("import"))
                    .and(symbols::is_def.eq(0)),
            )
            .order((symbols::path.asc(), symbols::line.asc()))
            .select((symbols::path, symbols::line))
            .load(&mut *conn)?)
    }

    /// T68.6: definitions in files that import `name` — the extra impact hop.
    pub fn symbol_import_follow(&self, root: &str, name: &str) -> Result<Vec<(String, String)>> {
        let (i, d) = alias!(symbols as i, symbols as d);
        let mut conn = self.lock()?;
        Ok(i.inner_join(
            d.on(d
                .field(symbols::root)
                .eq(i.field(symbols::root))
                .and(d.field(symbols::path).eq(i.field(symbols::path)))
                .and(d.field(symbols::is_def).eq(1))
                .and(d.field(symbols::name).ne(""))),
        )
        .filter(
            i.field(symbols::root)
                .eq(root)
                .and(i.field(symbols::name).eq(name))
                .and(i.field(symbols::kind).eq("import"))
                .and(i.field(symbols::is_def).eq(0)),
        )
        .order((d.field(symbols::path).asc(), d.field(symbols::line).asc()))
        .select((d.field(symbols::path), d.field(symbols::name)))
        .load(&mut *conn)?)
    }

    /// T368: `(reference file, definition file)`. Not a join: a specifier and a repo path share no column.
    pub fn symbol_imported_defs(&self, root: &str, name: &str) -> Result<Vec<(String, String)>> {
        let defs: Vec<String> = self
            .symbol_defs(root, name)?
            .into_iter()
            .map(|(path, ..)| path)
            .collect();
        if defs.len() < 2 {
            return Ok(Vec::new());
        }
        let mut conn = self.lock()?;
        let ref_files: Vec<String> = symbols::table
            .filter(
                symbols::root
                    .eq(root)
                    .and(symbols::name.eq(name))
                    .and(symbols::is_def.eq(0))
                    .and(symbols::kind.ne("import")),
            )
            .select(symbols::path)
            .distinct()
            .load(&mut *conn)?;
        if ref_files.is_empty() {
            return Ok(Vec::new());
        }
        let mut imports: Vec<(String, String)> = Vec::new();
        for chunk in ref_files.chunks(NAME_CHUNK) {
            imports.extend(
                symbols::table
                    .filter(symbols::root.eq(root))
                    .filter(symbols::kind.eq("import"))
                    .filter(symbols::is_def.eq(0))
                    .filter(symbols::path.eq_any(chunk))
                    .select((symbols::path, symbols::scope))
                    .load::<(String, String)>(&mut *conn)?,
            );
        }
        drop(conn);
        // T646: split each definition path and each specifier once, not once per pair.
        let keys: Vec<(&String, Vec<String>)> = defs.iter().map(|d| (d, file_key(d))).collect();
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for (ref_file, spec) in imports {
            let Some(imp) = import_path_segs(&spec) else {
                continue;
            };
            for (def, key) in &keys {
                if key_prefixes_import(key, &imp) && seen.insert((ref_file.clone(), (*def).clone()))
                {
                    out.push((ref_file.clone(), (*def).clone()));
                }
            }
        }
        out.sort();
        Ok(out)
    }

    /// T368: `(df, n_files)` from the cache written at the end of an index pass.
    /// `(0, 0)` when that pass has not run, so ranking does not rescan `symbols`.
    pub fn symbol_name_freq(&self, root: &str, name: &str) -> Result<(i64, i64)> {
        Ok(self.cached_name_freq(root, name)?.unwrap_or((0, 0)))
    }

    /// T368: one `GROUP BY name` of referencing files, stored so ranking does not
    /// rescan `symbols` on every `callers` / `impact`.
    pub fn rebuild_symbol_idf(&self, root: &str) -> Result<()> {
        let mut conn = self.lock()?;
        let rows: Vec<(String, i64)> = symbols::table
            .filter(symbols::root.eq(root))
            .filter(symbols::is_def.eq(0))
            .filter(symbols::kind.ne("import"))
            .filter(symbols::name.ne(""))
            .group_by(symbols::name)
            .select((symbols::name, count(symbols::path).aggregate_distinct()))
            .load(&mut *conn)?;
        let n = distinct_files(&mut conn, root)?;
        crate::sql_ext::analyze_symbols(&mut conn)?;
        drop(conn);
        let mut df = BTreeMap::new();
        for (name, n_files) in rows {
            df.insert(name, n_files);
        }
        let body = serde_json::to_string(&IdfCache { n, df })?;
        self.kv_set(&idf_key(root), &body)
    }

    fn cached_name_freq(&self, root: &str, name: &str) -> Result<Option<(i64, i64)>> {
        let key = idf_key(root);
        let Some(body) = self.kv_get(&key)? else {
            return Ok(None);
        };
        // T646: ranking asks once per visited name, and parsing the whole map each time was
        // most of a `graph review`. The body is still read, so another process's rebuild is seen.
        let mut memo = self
            .idf_memo
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if memo.as_ref().is_none_or(|m| m.key != key || m.body != body) {
            let cache = serde_json::from_str(&body).unwrap_or(IdfCache {
                n: 0,
                df: BTreeMap::new(),
            });
            *memo = Some(IdfMemo { key, body, cache });
        }
        let cache = &memo.as_ref().expect("set above").cache;
        Ok(Some((cache.df.get(name).copied().unwrap_or(0), cache.n)))
    }
}

/// Per-root document frequency written at the end of an index pass (T368).
#[derive(serde::Serialize, serde::Deserialize)]
struct IdfCache {
    n: i64,
    df: BTreeMap<String, i64>,
}

/// One parsed [`IdfCache`] with the kv row it came from.
pub(crate) struct IdfMemo {
    key: String,
    body: String,
    cache: IdfCache,
}

fn idf_key(root: &str) -> String {
    format!("graph-idf:{root}")
}

/// Indexed files, not symbol rows. `.distinct().count()` drops the DISTINCT
/// (Diesel replaces the select), so this is `COUNT(DISTINCT path)`.
fn distinct_files(conn: &mut SqliteConnection, root: &str) -> Result<i64> {
    Ok(symbols::table
        .filter(symbols::root.eq(root))
        .select(count(symbols::path).aggregate_distinct())
        .get_result(conn)?)
}

pub(crate) fn import_segs(spec: &str) -> Vec<String> {
    spec.split(['/', '\\', ':', '.'])
        .filter(|s| !s.is_empty())
        .filter(|s| !matches!(*s, "crate" | "self" | "super" | "package"))
        .map(|s| s.replace('-', "_"))
        .collect()
}

pub(crate) fn file_key(file: &str) -> Vec<String> {
    let mut parts: Vec<&str> = file.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    if let Some(last) = parts.last_mut()
        && let Some((stem, _)) = last.rsplit_once('.')
    {
        *last = stem;
    }
    if matches!(parts.last().copied(), Some("mod" | "lib" | "index")) {
        parts.pop();
    }
    parts
        .into_iter()
        .filter(|p| *p != "src" && *p != "crates")
        .map(|s| s.replace('-', "_"))
        .collect()
}

/// A bare identifier is not a path. Hyphens match underscores; the file's segments must prefix the import.
#[cfg(test)]
fn import_matches_file(spec: &str, file: &str) -> bool {
    import_path_segs(spec).is_some_and(|imp| key_prefixes_import(&file_key(file), &imp))
}

/// `None` for a bare identifier or an empty path.
fn import_path_segs(spec: &str) -> Option<Vec<String>> {
    if !spec.contains([':', '/', '.', '\\']) {
        return None;
    }
    Some(import_segs(spec)).filter(|imp| !imp.is_empty())
}

fn key_prefixes_import(file: &[String], imp: &[String]) -> bool {
    !file.is_empty() && file.len() <= imp.len() && imp[..file.len()] == file[..]
}

#[cfg(test)]
#[path = "symbols_tests.rs"]
mod tests;
