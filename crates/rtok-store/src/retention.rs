// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Store methods: retention and vacuum.

use super::*;

impl Store {
    /// Drop `calls` older than `days` with the rows that only describe them (`logs`, `tokens`,
    /// `call_io`). Ledger rows that point at a dropped call — `usage`, `measurements`, a newer
    /// child call — are kept and detached: a saving is not deleted with its call, and without the
    /// detach `foreign_keys = ON` refused the delete after the first three had already committed.
    /// One transaction and one cutoff, so it is all of it or none of it.
    /// The archive paths `run_retention` would delete for `core.retain_calls_days`, still on
    /// disk — read-only, nothing is removed (T182 `agents junk clear` dry run).
    pub fn archives_pending_retention(&self, retain_calls_days: u32) -> Result<Vec<PathBuf>> {
        let days = i64::from(retain_calls_days);
        if days <= 0 {
            return Ok(Vec::new());
        }
        let now = i64::try_from(unix_now()).unwrap_or(i64::MAX);
        let cutoff = now.saturating_sub(days.saturating_mul(86_400));
        let mut conn = self.lock()?;
        Ok(doomed_archives(&mut conn, cutoff)?
            .into_iter()
            .map(|a| PathBuf::from(a.path))
            .collect())
    }

    pub fn purge_calls_older_than(&self, days: i64) -> Result<usize> {
        if days <= 0 {
            return Ok(0);
        }
        let now = i64::try_from(unix_now()).unwrap_or(i64::MAX);
        let cutoff = now.saturating_sub(days.saturating_mul(86_400));
        let mut conn = self.lock()?;
        // T75: every surface opens this one file, and a purge starting while another
        // process held the write lock came back "database is locked" — the deferred
        // read-then-write transaction could lose instantly (a snapshot upgrade skips
        // the busy handler) or after the steady 1 s, and `mcp`/`proxy` died on it at
        // session start. Same contract as `migrate`: take the writer lock up front
        // under the maintenance window, and restore the hook's 1 s bound after,
        // whatever happened inside.
        set_busy(&mut conn, std::time::Duration::from_secs(30))?;
        let purged = conn.exclusive_transaction::<_, anyhow::Error, _>(|c| {
            let doomed = doomed_archives(c, cutoff)?;
            sql_ext::purge_related(c, cutoff)?;
            for arch in &doomed {
                sql_ext::purge_archive(c, &arch.id)?;
            }
            let n = sql_ext::delete_old_calls(c, cutoff)?;
            Ok((
                n,
                doomed
                    .into_iter()
                    .map(|a| PathBuf::from(a.path))
                    .collect::<Vec<_>>(),
            ))
        });
        set_busy(&mut conn, self.wait.busy)?;
        let paths = purged?;
        for path in paths.1 {
            let _ = std::fs::remove_file(path);
        }
        Ok(paths.0)
    }

    /// Apply `core.retain_calls_days` (0 = keep forever) and `core.retain_hook_bodies_days`
    /// (0 = keep hook bodies as long as their `calls` row), drop symbol rows of roots that no
    /// longer exist, then return the freed pages. Proxy and MCP call this once at session
    /// start; the count is the `calls` rows purged.
    pub fn run_retention(
        &self,
        retain_calls_days: u32,
        retain_hook_bodies_days: u32,
    ) -> Result<usize> {
        let purged = self.purge_calls_older_than(i64::from(retain_calls_days))?;
        self.clear_hook_bodies_older_than(i64::from(retain_hook_bodies_days))?;
        // T433: after both, since either can leave a session-fields row unreferenced.
        self.maintenance(|c| Ok(hook_fields::drop_orphans(c)?))?;
        self.drop_dead_symbol_roots(std::env::home_dir().as_deref())?;
        self.maintenance(|c| sql_ext::pragma_incremental_vacuum(c).map_err(Into::into))?;
        Ok(purged)
    }

    /// T352: run `f` under the maintenance busy window (see `purge_calls_older_than`).
    pub(crate) fn maintenance<T>(
        &self,
        f: impl FnOnce(&mut SqliteConnection) -> Result<T>,
    ) -> Result<T> {
        let mut conn = self.lock()?;
        set_busy(&mut conn, std::time::Duration::from_secs(30))?;
        let out = f(&mut conn);
        set_busy(&mut conn, self.wait.busy)?;
        out
    }

    /// T352: clear the stdin bodies of hook `call_io` rows older than `days` (0 = never).
    /// The `calls` row, byte counts, shas and archive columns stay; readers already treat a
    /// missing body as empty. Returns the number of rows cleared.
    pub fn clear_hook_bodies_older_than(&self, days: i64) -> Result<usize> {
        self.clear_hook_bodies_in_batches(days, HOOK_BODY_BATCH)
    }

    /// [`Store::clear_hook_bodies_older_than`] with an explicit batch size: one short write
    /// transaction per `batch` rows, the connection lock released in between, so a first run
    /// over a large store never holds the SQLite write lock for the whole clear.
    pub(crate) fn clear_hook_bodies_in_batches(&self, days: i64, batch: i64) -> Result<usize> {
        if days <= 0 {
            return Ok(0);
        }
        let now = i64::try_from(unix_now()).unwrap_or(i64::MAX);
        let cutoff = now.saturating_sub(days.saturating_mul(86_400));
        let mut total = 0;
        loop {
            let n = self.maintenance(|c| Ok(sql_ext::clear_hook_bodies(c, cutoff, batch)?))?;
            if n == 0 {
                return Ok(total);
            }
            total += n;
        }
    }

    /// T352: retention on its own connection, then the one-time conversion of a pre-T352 store
    /// to `auto_vacuum = INCREMENTAL`. Retention goes first so the `VACUUM` rewrites the
    /// already-smaller file. Both are housekeeping with a next-start retry (T75): an error is
    /// not fatal. A failed `VACUUM` (busy, `SQLITE_FULL`) rolls back and leaves the file
    /// intact; a successful one briefly blocks other writers, once per store. Warnings come
    /// back to the caller; this function prints nothing.
    pub fn housekeeping(job: &RetentionJob) -> Vec<StoreWarning> {
        let warn = |what: &'static str, e: &StoreError| StoreWarning {
            what,
            message: format!("{what} skipped until next start: {e:#}"),
        };
        let store = match Store::open(&job.db_path) {
            Ok(s) => s,
            Err(e) => return vec![warn("retention", &e)],
        };
        if let Err(e) = store.run_retention(job.retain_calls_days, job.retain_hook_bodies_days) {
            return vec![warn("retention", &e)];
        }
        // T968: after the age retention, so the limits only act on what it left.
        if let Err(e) = store.prune(&job.db_path, &job.prune, false) {
            return vec![warn("prune", &e)];
        }
        match store.convert_to_incremental_vacuum() {
            Ok(_) => Vec::new(),
            Err(e) => vec![warn("vacuum", &e)],
        }
    }

    /// T352/T356: drop the graph index of every root that is no longer a directory (a removed
    /// worktree or clone) or that may never be a root (`/`, `home`). The empty root is a
    /// placeholder, not a path, and stays.
    pub fn drop_dead_symbol_roots(&self, home: Option<&Path>) -> Result<usize> {
        self.maintenance(|c| {
            let gone: Vec<String> = sql_ext::symbol_roots(c)?
                .into_iter()
                .filter(|r| {
                    let p = Path::new(r);
                    !r.is_empty() && (!p.is_dir() || is_unwalkable_root(p, home))
                })
                .collect();
            for root in &gone {
                sql_ext::delete_symbol_root(c, root)?;
            }
            Ok(gone.len())
        })
    }

    /// T352: make an existing store shrink on delete. A store created before T352 has
    /// `auto_vacuum = 0`; the mode only changes through a `VACUUM`, which rewrites the file
    /// (needs free disk the size of the database). Returns whether it converted anything.
    pub fn convert_to_incremental_vacuum(&self) -> Result<bool> {
        self.maintenance(|c| {
            if sql_ext::AutoVacuumMode.get_result::<i32>(c)? == 2 {
                return Ok(false);
            }
            sql_ext::pragma_auto_vacuum_incremental(c)?;
            sql_ext::vacuum(c)?;
            Ok(true)
        })
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn set_query_only(&self) -> Result<()> {
        let mut conn = self.lock()?;
        sql_ext::pragma_query_only_on(&mut conn)?;
        Ok(())
    }
}
