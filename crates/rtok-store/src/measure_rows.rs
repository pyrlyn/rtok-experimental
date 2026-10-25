// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Store methods: measurements.

use super::*;

impl Store {
    /// One `measurements` row. Prefer `Runtime::record`, which supplies the session.
    pub fn insert_measurement(&self, session: &str, m: &Measurement) -> Result<()> {
        self.insert_measurement_once(session, m, None)
    }

    /// [`Store::insert_measurement`] for one delivery of a call: a row whose `once` key (the
    /// call, e.g. `PreToolUse:<tool_use_id>`) plus plugin, kind and ref is already stored is
    /// dropped, so a call delivered twice counts once (T245).
    pub fn insert_measurement_once(
        &self,
        session: &str,
        m: &Measurement,
        once: Option<&str>,
    ) -> Result<()> {
        let mut conn = self.lock()?;
        insert_measurement_conn(&mut conn, session, m, once)
    }

    /// [`Store::insert_measurement_once`] for several rows under one write lock and one commit.
    /// All or none: the rows came from one dispatch and read as one event.
    pub fn insert_measurements_once(
        &self,
        session: &str,
        ms: &[Measurement],
        once: Option<&str>,
    ) -> Result<()> {
        let mut conn = self.lock()?;
        conn.immediate_transaction(|conn| -> Result<()> {
            for m in ms {
                insert_measurement_conn(conn, session, m, once)?;
            }
            Ok(())
        })
    }

    /// Count `measurements` for one plugin. Used by `examples/hello_plugin.rs`.
    pub fn measurement_count(&self, plugin: &str) -> Result<i64> {
        let mut conn = self.lock()?;
        Ok(measurements::table
            .filter(measurements::plugin.eq(plugin))
            .count()
            .get_result(&mut *conn)?)
    }

    /// Measurement rows for `rtok stats --plugin <id>` (T3.6).
    pub fn list_measurements(&self, plugin: &str) -> Result<Vec<MeasRow>> {
        let mut conn = self.lock()?;
        measurements::table
            .filter(measurements::plugin.eq(plugin))
            .order(measurements::id.asc())
            .select((
                measurements::kind,
                measurements::before_bytes,
                measurements::after_bytes,
                measurements::est_before,
                measurements::est_after,
                measurements::ref_id,
            ))
            .load::<MeasRow>(&mut *conn)
            .map_err(Into::into)
    }

    /// Rows and est_before/est_after summed per `(plugin, kind)`, across every plugin the
    /// ledger has ever seen — not just the catalogue (T207). The one aggregate
    /// `report_window`, `report_savings`, `otel_saved_totals` and both `plugin_stats`
    /// read instead of each hand-rolling its own sum (or, for `otel_saved_totals`,
    /// dropping to raw SQL); callers decide what an `expand` group means (a cost, not a
    /// saving — `ReportSavings::saved`'s contract), this is just the read.
    pub fn measurement_totals(&self) -> Result<Vec<MeasurementTotal>> {
        use diesel::dsl::{count_star, sum};
        let mut conn = self.lock()?;
        let rows = measurements::table
            .group_by((measurements::plugin, measurements::kind))
            .select((
                measurements::plugin,
                measurements::kind,
                count_star(),
                sum(measurements::est_before),
                sum(measurements::est_after),
            ))
            .order((measurements::plugin, measurements::kind))
            .load::<MeasurementTotalRow>(&mut *conn)?;
        Ok(rows.into_iter().map(MeasurementTotal::from_row).collect())
    }

    /// [`Self::measurement_totals`] over the rows stamped at or after `since`, one group per
    /// `(plugin, kind, ts)`: the same aggregate with the row's second as its grain, which the
    /// caller folds into day buckets in a time zone (T414.13). Diesel 2.3 cannot `GROUP BY` a
    /// computed `ts / N` bucket (the gap [`Self::usage_slices`] documents), and a day edge
    /// moves with the zone's offset anyway.
    pub fn measurement_totals_since(&self, since: i64) -> Result<Vec<(i64, MeasurementTotal)>> {
        use diesel::dsl::{count_star, sum};
        let mut conn = self.lock()?;
        let rows = measurements::table
            .filter(measurements::ts.ge(since))
            .group_by((measurements::plugin, measurements::kind, measurements::ts))
            .select((
                measurements::ts,
                (
                    measurements::plugin,
                    measurements::kind,
                    count_star(),
                    sum(measurements::est_before),
                    sum(measurements::est_after),
                ),
            ))
            .order(measurements::ts)
            .load::<(i64, MeasurementTotalRow)>(&mut *conn)?;
        Ok(rows
            .into_iter()
            .map(|(ts, row)| (ts, MeasurementTotal::from_row(row)))
            .collect())
    }

    /// The ledger's `est_before - est_after` per host over the rows stamped in
    /// `[since, until)`, with `None` for a session that has no host row (T358.6). An `expand`
    /// row already reads negative (retrieval costs tokens), so the sum is the net saving the
    /// report shows. [`Self::measurement_totals`] cannot serve this: it groups by `(plugin,
    /// kind)` with no window and no session. Sessions are grouped here and mapped to hosts in
    /// Rust for the same Diesel join-group gap as [`Self::usage_slices`].
    pub fn measurement_saved_by_host(
        &self,
        since: i64,
        until: i64,
    ) -> Result<Vec<(Option<String>, i64)>> {
        use diesel::dsl::sum;
        let mut conn = self.lock()?;
        let rows: Vec<(String, Option<i64>, Option<i64>)> = measurements::table
            .filter(measurements::ts.ge(since).and(measurements::ts.lt(until)))
            .group_by(measurements::session)
            .select((
                measurements::session,
                sum(measurements::est_before),
                sum(measurements::est_after),
            ))
            .load(&mut *conn)?;
        let host_of = host_by_session(&mut conn)?;
        let mut by_host: BTreeMap<Option<String>, i64> = BTreeMap::new();
        for (session, before, after) in rows {
            let host = host_of.get(&session).cloned().flatten();
            *by_host.entry(host).or_default() += before.unwrap_or(0) - after.unwrap_or(0);
        }
        Ok(by_host.into_iter().collect())
    }

    /// [`Self::measurement_totals`] over the rows stamped at or after `since`, split by the
    /// project name of the session that wrote them (`None`: a session with no project). Rows
    /// are summed per `(plugin, kind)` as there; an `expand` group reads negative, a cost and
    /// not a saving, as in [`Self::measurement_saved_by_host`]. Sessions are grouped here and
    /// mapped to projects in Rust for the same Diesel join-group gap.
    pub fn measurement_saved_by_project(
        &self,
        since: i64,
    ) -> Result<Vec<(Option<String>, MeasurementTotal)>> {
        use diesel::dsl::{count_star, sum};
        let mut conn = self.lock()?;
        let rows: Vec<(String, MeasurementTotalRow)> = measurements::table
            .filter(measurements::ts.ge(since))
            .group_by((
                measurements::session,
                measurements::plugin,
                measurements::kind,
            ))
            .select((
                measurements::session,
                (
                    measurements::plugin,
                    measurements::kind,
                    count_star(),
                    sum(measurements::est_before),
                    sum(measurements::est_after),
                ),
            ))
            .load(&mut *conn)?;
        let project_of = project_by_session(&mut conn)?;
        let mut by: BTreeMap<(Option<String>, String, String), MeasurementTotal> = BTreeMap::new();
        for (session, row) in rows {
            let project = project_of.get(&session).cloned().flatten();
            let t = MeasurementTotal::from_row(row);
            let sum = by
                .entry((project, t.plugin.clone(), t.kind.clone()))
                .or_insert_with(|| MeasurementTotal {
                    rows: 0,
                    est_before: 0,
                    est_after: 0,
                    ..t.clone()
                });
            sum.rows += t.rows;
            sum.est_before += t.est_before;
            sum.est_after += t.est_after;
        }
        Ok(by.into_iter().map(|((p, ..), t)| (p, t)).collect())
    }

    /// Test helper: stamp every `measurements` row of `session` at `ts`.
    #[cfg(any(test, feature = "test-util"))]
    pub fn set_measurement_ts(&self, session: &str, ts: i64) -> Result<()> {
        let mut conn = self.lock()?;
        diesel::update(measurements::table.filter(measurements::session.eq(session)))
            .set(measurements::ts.eq(ts))
            .execute(&mut *conn)?;
        Ok(())
    }

    /// Per `(project, kind)` note counts for `memory status` (T69.4). Excludes
    /// `checkpoint:*` and `session:*` housekeeping kinds, same as `list_notes` /
    /// `list_note_titles` (T304): `session:<id>` is unique per session, so leaving it in
    /// would grow one row per historical session forever.
    pub fn memory_note_aggs(&self, project: Option<&str>) -> Result<Vec<MemoryNoteKindAgg>> {
        use diesel::dsl::{case_when, max, min};
        type Row = (
            Option<String>,
            String,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
        );
        let mut conn = self.lock()?;
        // `None` = no project filter; `Some(p)` = equality. Two typed arms — boxed queries
        // cannot `group_by` this select shape.
        macro_rules! load_aggs {
            ($q:expr) => {
                $q.group_by((notes::project, notes::kind))
                    .select((
                        notes::project,
                        notes::kind,
                        sum_bigint(
                            case_when::<_, _, BigInt>(notes::retired.is_null(), 1i64)
                                .otherwise(0i64),
                        ),
                        sum_bigint(
                            case_when::<_, _, BigInt>(
                                notes::retired.is_null().and(notes::pinned.ne(0)),
                                1i64,
                            )
                            .otherwise(0i64),
                        ),
                        sum_bigint(
                            case_when::<_, _, BigInt>(notes::retired.is_not_null(), 1i64)
                                .otherwise(0i64),
                        ),
                        sum_bigint(length(notes::body)),
                        min(notes::ts),
                        max(notes::ts),
                    ))
                    .order((notes::project, notes::kind))
                    .load(&mut *conn)?
            };
        }
        let base = notes::table
            .filter(notes::kind.not_like("checkpoint:%"))
            .filter(notes::kind.not_like("session:%"));
        let rows: Vec<Row> = match project {
            Some(p) => load_aggs!(base.filter(notes::project.eq(p))),
            None => load_aggs!(base),
        };
        Ok(rows
            .into_iter()
            .map(
                |(project, kind, live, pinned, retired, body_bytes, oldest_ts, newest_ts)| {
                    MemoryNoteKindAgg {
                        project,
                        kind,
                        live: u64::try_from(live.unwrap_or(0)).unwrap_or(0),
                        pinned: u64::try_from(pinned.unwrap_or(0)).unwrap_or(0),
                        retired: u64::try_from(retired.unwrap_or(0)).unwrap_or(0),
                        body_bytes: body_bytes.unwrap_or(0),
                        oldest_ts: oldest_ts.unwrap_or(0),
                        newest_ts: newest_ts.unwrap_or(0),
                    }
                },
            )
            .collect())
    }

    /// SessionStart recall measurements in a time window (T69.4).
    pub fn memory_recall_totals(&self, since_unix: i64) -> Result<(u64, i64, i64)> {
        use diesel::dsl::count_star;
        let mut conn = self.lock()?;
        let (recalls, stood_for_bytes, injected_bytes): (i64, Option<i64>, Option<i64>) =
            measurements::table
                .filter(measurements::plugin.eq("memory"))
                .filter(measurements::kind.eq("recall"))
                .filter(measurements::ts.ge(since_unix))
                .select((
                    count_star(),
                    sum_bigint(measurements::before_bytes),
                    sum_bigint(measurements::after_bytes),
                ))
                .first(&mut *conn)?;
        Ok((
            u64::try_from(recalls).unwrap_or(0),
            stood_for_bytes.unwrap_or(0),
            injected_bytes.unwrap_or(0),
        ))
    }

    /// MCP `mem_search` / `mem_get` calls in a time window (T69.4).
    pub fn memory_mcp_calls(&self, since_unix: i64) -> Result<(u64, u64)> {
        use diesel::dsl::case_when;
        let mut conn = self.lock()?;
        let (mem_search, mem_get): (Option<i64>, Option<i64>) = calls::table
            .filter(calls::plugin.eq("memory"))
            .filter(calls::surface.eq("mcp"))
            .filter(calls::kind.eq("mcp_call"))
            .filter(calls::ts.ge(since_unix))
            .select((
                sum_bigint(
                    case_when::<_, _, BigInt>(calls::name.eq("mem_search"), 1i64).otherwise(0i64),
                ),
                sum_bigint(
                    case_when::<_, _, BigInt>(calls::name.eq("mem_get"), 1i64).otherwise(0i64),
                ),
            ))
            .first(&mut *conn)?;
        Ok((
            u64::try_from(mem_search.unwrap_or(0)).unwrap_or(0),
            u64::try_from(mem_get.unwrap_or(0)).unwrap_or(0),
        ))
    }

    /// Last `ref_id` on a measurement row for one session (T69.5 prompt_recall dedup).
    pub fn last_measurement_ref(
        &self,
        session: &str,
        plugin: &str,
        kind: &str,
    ) -> Result<Option<String>> {
        let mut conn = self.lock()?;
        let ref_id: Option<Option<String>> = measurements::table
            .filter(measurements::session.eq(session))
            .filter(measurements::plugin.eq(plugin))
            .filter(measurements::kind.eq(kind))
            .order(measurements::id.desc())
            .select(measurements::ref_id)
            .first(&mut *conn)
            .optional()?;
        Ok(ref_id.flatten())
    }
}
