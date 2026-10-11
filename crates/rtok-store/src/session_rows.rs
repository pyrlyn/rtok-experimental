// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Store methods: sessions and hosts.

use super::*;

impl Store {
    pub fn upsert_session(
        &self,
        id: &str,
        host_id: Option<i32>,
        project: Option<&str>,
        cwd: Option<&str>,
        source: Option<&str>,
    ) -> Result<()> {
        let mut conn = self.lock()?;
        // COALESCE keeps a non-NULL value: a later writer that does not know the
        // attribution (proxy/mcp pass None for project/cwd; Runtime::insert_call
        // passes None for source) must not wipe what an earlier hook already set.
        diesel::insert_into(sessions::table)
            .values((
                sessions::id.eq(id),
                sessions::host_id.eq(host_id),
                sessions::project.eq(project),
                sessions::cwd.eq(cwd),
                sessions::source.eq(source),
            ))
            .on_conflict(sessions::id)
            .do_update()
            .set((
                sessions::host_id.eq(coalesce(
                    diesel::upsert::excluded(sessions::host_id),
                    sessions::host_id,
                )),
                sessions::project.eq(coalesce(
                    diesel::upsert::excluded(sessions::project),
                    sessions::project,
                )),
                sessions::cwd.eq(coalesce(
                    diesel::upsert::excluded(sessions::cwd),
                    sessions::cwd,
                )),
                sessions::source.eq(coalesce(
                    diesel::upsert::excluded(sessions::source),
                    sessions::source,
                )),
            ))
            .execute(&mut *conn)?;
        Ok(())
    }

    /// Upsert provider + model; returns `(provider_id, model_id)`. Proxy ground truth:
    /// the request `model` must resolve to a `models` row (plan T5.1 Check).
    /// T208: `BEGIN IMMEDIATE` — the select-then-insert-then-select below would otherwise
    /// start as a read and race another writer's upgrade to the same rows ("database is
    /// locked" even inside `wait.busy`); an immediate transaction takes the write lock up
    /// front and serializes instead.
    pub fn upsert_model(&self, provider_slug: &str, model_slug: &str) -> Result<(i32, i32)> {
        let mut conn = self.lock()?;
        conn.immediate_transaction(|conn| -> Result<(i32, i32)> {
            diesel::insert_or_ignore_into(providers::table)
                .values((
                    providers::slug.eq(provider_slug),
                    providers::name.eq(provider_slug),
                ))
                .execute(&mut *conn)?;
            let provider_id: i32 = providers::table
                .filter(providers::slug.eq(provider_slug))
                .select(providers::id)
                .first(&mut *conn)
                .optional()?
                .context("provider")?;
            diesel::insert_or_ignore_into(schema::models::table)
                .values((
                    schema::models::provider_id.eq(provider_id),
                    schema::models::slug.eq(model_slug),
                ))
                .execute(&mut *conn)?;
            let model_id: i32 = schema::models::table
                .filter(schema::models::provider_id.eq(provider_id))
                .filter(schema::models::slug.eq(model_slug))
                .select(schema::models::id)
                .first(&mut *conn)
                .optional()?
                .context("model")?;
            Ok((provider_id, model_id))
        })
    }

    pub fn host_id(&self, slug: &str) -> Result<Option<i32>> {
        let mut conn = self.lock()?;
        Ok(hosts::table
            .filter(hosts::slug.eq(slug))
            .select(hosts::id)
            .first(&mut *conn)
            .optional()?)
    }

    /// One session's `(host_id slug, project, cwd)` — T25.0's Check reads the row a hook run
    /// left rather than re-deriving it from `upsert_session`'s arguments, and the live Graph
    /// calls feed (T329.34) names a session by its host when no agent row exists.
    pub fn session_row(&self, id: &str) -> Result<Option<SessionRow>> {
        let mut conn = self.lock()?;
        sessions::table
            .left_join(hosts::table)
            .filter(sessions::id.eq(id))
            .select((hosts::slug.nullable(), sessions::project, sessions::cwd))
            .first::<SessionRow>(&mut *conn)
            .optional()
            .map_err(Into::into)
    }

    /// Every session with a `cwd`, newest activity first (T154): the worktree ownership
    /// ledger is this projection of what the hooks already write — no second writer.
    /// `last_seen` is the newest `calls` row, or `started_at` before the first one lands.
    pub fn sessions_by_cwd(&self) -> Result<Vec<SessionSeen>> {
        let mut conn = self.lock()?;
        // Two builder queries joined in memory: newest call per session, then the sessions
        // with their host — the query builder has no COALESCE over a grouped join.
        let last_call: HashMap<String, i64> = calls::table
            .group_by(calls::session_id)
            .select((calls::session_id, diesel::dsl::max(calls::ts)))
            .load::<(String, Option<i64>)>(&mut *conn)?
            .into_iter()
            .filter_map(|(id, ts)| Some((id, ts?)))
            .collect();
        let mut rows: Vec<SessionSeen> = sessions::table
            .left_join(hosts::table)
            .filter(sessions::cwd.is_not_null())
            .select((
                sessions::id,
                hosts::slug.nullable(),
                sessions::cwd.assume_not_null(),
                sessions::started_at,
                sessions::ended_at,
            ))
            .load::<(String, Option<String>, String, i64, Option<i64>)>(&mut *conn)?
            .into_iter()
            .map(|(id, host, cwd, started_at, ended_at)| SessionSeen {
                last_seen: last_call.get(&id).copied().unwrap_or(started_at),
                id,
                host,
                cwd,
                ended_at,
            })
            .collect();
        rows.sort_by(|a, b| b.last_seen.cmp(&a.last_seen).then_with(|| a.id.cmp(&b.id)));
        Ok(rows)
    }
}
