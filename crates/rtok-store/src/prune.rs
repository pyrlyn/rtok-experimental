// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Store methods: the size and age limits of the database and the archive (T968).
//!
//! `run_retention` already drops what is older than `retain_calls_days`. This adds what it does
//! not: a cap on the archive, which kept every body a decision or the read cache still named,
//! and a cap on the database file, which grew with the calls inside the window.

use super::*;
use diesel::dsl::max;

/// A session with a call this recent can still ask for `expand <id>`. A day in a long
/// session is no gap; a week with no call is a session that was closed without a `SessionEnd`.
pub const OPEN_SESSION_SECS: i64 = 14 * 86_400;

/// Call retention is never shortened below this to meet `max_store_mb`: a store that is still
/// over its limit at the floor is reported, not emptied.
pub const RETAIN_CALLS_FLOOR_DAYS: u32 = 7;

/// Where the size loop starts when `retain_calls_days = 0` keeps calls forever.
const SIZE_LOOP_START_DAYS: u32 = 30;

/// The limits [`Store::prune`] applies; `0` turns a limit off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrunePolicy {
    pub retain_calls_days: u32,
    pub retain_archive_days: u32,
    pub max_store_bytes: u64,
    pub max_archive_bytes: u64,
}

/// What one [`Store::prune`] removed (or, for a dry run, would remove: calls are not
/// previewed, since the size loop depends on how far each purge shrinks the file).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneReport {
    pub archive_entries: usize,
    pub archive_bytes: u64,
    pub calls: usize,
    /// Database plus WAL bytes before and after.
    pub store_before: u64,
    pub store_after: u64,
}

/// Database plus WAL bytes on disk — what a long write has to get through.
pub fn store_file_bytes(db_path: &Path) -> u64 {
    let wal = {
        let mut s = db_path.as_os_str().to_os_string();
        s.push("-wal");
        PathBuf::from(s)
    };
    [db_path, wal.as_path()]
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum()
}

/// One archive row a prune can delete.
struct Doomed {
    id: String,
    path: String,
    bytes: i64,
}

impl Store {
    /// Apply `policy`: calls past `retain_calls_days`, archive bodies of idle sessions past
    /// `retain_archive_days` or beyond `max_archive_bytes` (oldest first), then call retention
    /// shortened toward [`RETAIN_CALLS_FLOOR_DAYS`] while the database stays over
    /// `max_store_bytes`. A body that a session with a recent call can still reach — its own,
    /// or one its decisions and read cache name — is never deleted. After any deletion the WAL is
    /// checkpointed and the file compacted. `dry_run` deletes nothing.
    pub fn prune(
        &self,
        db_path: &Path,
        policy: &PrunePolicy,
        dry_run: bool,
    ) -> Result<PruneReport> {
        let now = i64::try_from(unix_now()).unwrap_or(i64::MAX);
        let mut report = PruneReport {
            store_before: store_file_bytes(db_path),
            ..PruneReport::default()
        };
        let plan = self.archive_prune_plan(policy, now)?;
        report.archive_entries = plan.len();
        report.archive_bytes = plan
            .iter()
            .map(|d| u64::try_from(d.bytes).unwrap_or(0))
            .sum();
        if dry_run {
            report.store_after = report.store_before;
            return Ok(report);
        }
        self.drop_archives(&plan)?;
        report.calls = self.purge_calls_older_than(i64::from(policy.retain_calls_days))?;
        if report.archive_entries + report.calls > 0 {
            self.compact()?;
        }
        if policy.max_store_bytes > 0 {
            let mut days = match policy.retain_calls_days {
                0 => SIZE_LOOP_START_DAYS,
                d => d,
            };
            while store_file_bytes(db_path) > policy.max_store_bytes
                && days > RETAIN_CALLS_FLOOR_DAYS
            {
                days = (days / 2).max(RETAIN_CALLS_FLOOR_DAYS);
                report.calls += self.purge_calls_older_than(i64::from(days))?;
                self.compact()?;
            }
        }
        report.store_after = store_file_bytes(db_path);
        Ok(report)
    }

    /// Checkpoint the WAL and hand the free pages back. A pre-T352 store (`auto_vacuum = 0`)
    /// gets the full `VACUUM` that converts it; every other store only an incremental one.
    fn compact(&self) -> Result<()> {
        self.convert_to_incremental_vacuum()?;
        self.maintenance(|c| {
            sql_ext::pragma_incremental_vacuum(c)?;
            sql_ext::WalCheckpointTruncate.get_result::<(i32, i32, i32)>(c)?;
            Ok(())
        })
    }

    /// The archive rows to delete under `policy`, oldest first.
    fn archive_prune_plan(&self, policy: &PrunePolicy, now: i64) -> Result<Vec<Doomed>> {
        if policy.retain_archive_days == 0 && policy.max_archive_bytes == 0 {
            return Ok(Vec::new());
        }
        let mut conn = self.lock()?;
        let open = open_sessions(&mut conn, now.saturating_sub(OPEN_SESSION_SECS))?;
        let reachable = reachable_archives(&mut conn, &open)?;
        let rows: Vec<(String, String, String, i64, i64)> = archive::table
            .order((archive::ts.asc(), archive::id.asc()))
            .select((
                archive::id,
                archive::session,
                archive::path,
                archive::ts,
                archive::bytes,
            ))
            .load(&mut *conn)?;
        let mut total: i64 = rows.iter().map(|r| r.4).sum();
        let age_cutoff = (policy.retain_archive_days > 0).then(|| {
            now.saturating_sub(i64::from(policy.retain_archive_days).saturating_mul(86_400))
        });
        let cap = i64::try_from(policy.max_archive_bytes).unwrap_or(i64::MAX);
        let mut doomed = Vec::new();
        for (id, session, path, ts, bytes) in rows {
            if open.contains(&session) || reachable.contains(&id) {
                continue;
            }
            let too_old = age_cutoff.is_some_and(|c| ts < c);
            let over_cap = policy.max_archive_bytes > 0 && total > cap;
            if too_old || over_cap {
                total -= bytes;
                doomed.push(Doomed { id, path, bytes });
            }
        }
        Ok(doomed)
    }

    /// Delete the rows in one transaction, then the files: a crash between the two leaves
    /// an orphan file, never a row naming a missing one.
    fn drop_archives(&self, doomed: &[Doomed]) -> Result<()> {
        if doomed.is_empty() {
            return Ok(());
        }
        self.maintenance(|c| {
            c.immediate_transaction::<_, diesel::result::Error, _>(|c| {
                for d in doomed {
                    sql_ext::detach_call_io_archive(c, &d.id)?;
                    sql_ext::purge_archive(c, &d.id)?;
                }
                Ok(())
            })?;
            Ok(())
        })?;
        for d in doomed {
            let _ = std::fs::remove_file(&d.path);
        }
        Ok(())
    }
}

/// Sessions with a call since `live_cutoff` that were not ended, or were ended before that
/// call (a resumed session keeps its first `ended_at`).
fn open_sessions(conn: &mut SqliteConnection, live_cutoff: i64) -> Result<HashSet<String>> {
    let recent: Vec<(String, Option<i64>)> = calls::table
        .filter(calls::ts.ge(live_cutoff))
        .group_by(calls::session_id)
        .select((calls::session_id, max(calls::ts)))
        .load(conn)?;
    let ended: HashMap<String, i64> = sessions::table
        .filter(sessions::ended_at.is_not_null())
        .select((sessions::id, sessions::ended_at.assume_not_null()))
        .load::<(String, i64)>(conn)?
        .into_iter()
        .collect();
    Ok(recent
        .into_iter()
        .filter(|(id, last)| match (ended.get(id), last) {
            (Some(ended), Some(last)) => last > ended,
            _ => true,
        })
        .map(|(id, _)| id)
        .collect())
}

/// Archive ids the decisions and the read cache of an open session name: another session
/// owns the row, but this one can still ask for the body.
fn reachable_archives(
    conn: &mut SqliteConnection,
    open: &HashSet<String>,
) -> Result<HashSet<String>> {
    let open: Vec<&String> = open.iter().collect();
    let mut ids = HashSet::new();
    // Under SQLite's bound-parameter limit.
    for chunk in open.chunks(500) {
        ids.extend(
            archive_decisions::table
                .filter(archive_decisions::session.eq_any(chunk))
                .select(archive_decisions::archive_id)
                .load::<String>(conn)?,
        );
        ids.extend(
            read_cache::table
                .filter(read_cache::session.eq_any(chunk))
                .filter(read_cache::archive_id.is_not_null())
                .select(read_cache::archive_id.assume_not_null())
                .load::<String>(conn)?,
        );
    }
    Ok(ids)
}
