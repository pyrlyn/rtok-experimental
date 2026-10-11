// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Store methods: calls, tokens, usage, and logs.

use super::*;

impl Store {
    #[allow(clippy::too_many_arguments)]
    pub fn insert_call(
        &self,
        session_id: &str,
        surface: &str,
        kind: &str,
        host_id: Option<i32>,
        provider_id: Option<i32>,
        model_id: Option<i32>,
        plugin: Option<&str>,
        name: Option<&str>,
    ) -> Result<i32> {
        let mut conn = self.lock()?;
        Ok(diesel::insert_into(calls::table)
            .values((
                calls::session_id.eq(session_id),
                calls::surface.eq(surface),
                calls::kind.eq(kind),
                calls::host_id.eq(host_id),
                calls::provider_id.eq(provider_id),
                calls::model_id.eq(model_id),
                calls::plugin.eq(plugin),
                calls::name.eq(name),
            ))
            .returning(calls::id)
            .get_result(&mut *conn)?)
    }

    /// Nest a `plugin_run` row under the hook, MCP call or API request it ran in.
    pub fn set_call_parent(&self, id: i32, parent: i32) -> Result<()> {
        let mut conn = self.lock()?;
        diesel::update(calls::table.filter(calls::id.eq(id)))
            .set(calls::parent_id.eq(parent))
            .execute(&mut *conn)?;
        Ok(())
    }

    /// The tier the provider reported serving the request on (T385.12.2).
    pub fn set_call_service_tier(&self, id: i32, tier: &str) -> Result<()> {
        let mut conn = self.lock()?;
        diesel::update(calls::table.filter(calls::id.eq(id)))
            .set(calls::service_tier.eq(tier))
            .execute(&mut *conn)?;
        Ok(())
    }

    pub fn set_call_ms(&self, id: i32, ms: f64) -> Result<()> {
        let mut conn = self.lock()?;
        diesel::update(calls::table.filter(calls::id.eq(id)))
            .set(calls::ms.eq(ms))
            .execute(&mut *conn)?;
        Ok(())
    }

    pub fn count_kind(&self, kind: &str) -> Result<i64> {
        let mut conn = self.lock()?;
        Ok(calls::table
            .filter(calls::kind.eq(kind))
            .count()
            .get_result(&mut *conn)?)
    }

    pub fn call_ids_of_kind(&self, kind: &str) -> Result<Vec<i32>> {
        let mut conn = self.lock()?;
        Ok(calls::table
            .filter(calls::kind.eq(kind))
            .select(calls::id)
            .load(&mut *conn)?)
    }

    pub fn call_io_archives(&self, call_id: i32) -> Result<(Option<String>, Option<String>)> {
        let mut conn = self.lock()?;
        Ok(call_io::table
            .filter(call_io::call_id.eq(call_id))
            .select((call_io::request_archive, call_io::response_archive))
            .first::<(Option<String>, Option<String>)>(&mut *conn)
            .optional()?
            .unwrap_or((None, None)))
    }

    /// T201: the sha256 columns beside [`Self::call_io_archives`] — `NULL` for a body
    /// `spill` never archived (over `inline_cap` with `archive_dir = None`, the hook path),
    /// so a caller can tell "never hashed" from "hashed and inlined".
    #[cfg(any(test, feature = "test-util"))]
    pub fn call_io_shas(&self, call_id: i32) -> Result<(Option<String>, Option<String>)> {
        let mut conn = self.lock()?;
        Ok(call_io::table
            .filter(call_io::call_id.eq(call_id))
            .select((call_io::request_sha256, call_io::response_sha256))
            .first::<(Option<String>, Option<String>)>(&mut *conn)
            .optional()?
            .unwrap_or((None, None)))
    }

    /// Archive ids a Calls row can expand (T60.4): spilled `call_io` body first,
    /// else a `measurements.ref_id` on that call. One pair of queries for the
    /// page, so the snapshot does not N+1 on a tick.
    pub fn archive_ref_ids(
        &self,
        call_ids: &[i32],
    ) -> Result<std::collections::BTreeMap<i32, String>> {
        let mut out = std::collections::BTreeMap::new();
        if call_ids.is_empty() {
            return Ok(out);
        }
        let mut conn = self.lock()?;
        let io: Vec<(i32, Option<String>, Option<String>)> = call_io::table
            .filter(call_io::call_id.eq_any(call_ids.iter().copied()))
            .select((
                call_io::call_id,
                call_io::request_archive,
                call_io::response_archive,
            ))
            .load(&mut *conn)?;
        for (call_id, request_archive, response_archive) in io {
            if let Some(id) = response_archive.or(request_archive) {
                out.insert(call_id, id);
            }
        }
        let ms: Vec<(Option<i32>, Option<String>)> = measurements::table
            .filter(measurements::call_id.eq_any(call_ids.iter().copied()))
            .filter(measurements::ref_id.is_not_null())
            .select((measurements::call_id, measurements::ref_id))
            .load(&mut *conn)?;
        for (call_id, ref_id) in ms {
            if let (Some(call_id), Some(ref_id)) = (call_id, ref_id) {
                out.entry(call_id).or_insert(ref_id);
            }
        }
        Ok(out)
    }

    pub fn token_phases(&self, call_id: i32) -> Result<Vec<String>> {
        let mut conn = self.lock()?;
        Ok(tokens::table
            .filter(tokens::call_id.eq(call_id))
            .select(tokens::phase)
            .load(&mut *conn)?)
    }

    pub fn count_call_io(&self) -> Result<i64> {
        let mut conn = self.lock()?;
        Ok(call_io::table.count().get_result(&mut *conn)?)
    }

    pub fn count_tokens(&self) -> Result<i64> {
        let mut conn = self.lock()?;
        Ok(tokens::table.count().get_result(&mut *conn)?)
    }

    /// Whole-`measurements`-ledger row count (T207): `report_window`'s "measurements"
    /// figure, replacing a per-catalogue-plugin `list_measurements` loop that missed
    /// out-of-tree/WASM plugins and cost an N+1.
    pub fn count_measurements(&self) -> Result<i64> {
        let mut conn = self.lock()?;
        Ok(measurements::table.count().get_result(&mut *conn)?)
    }

    /// Whole-`usage`-ledger row count (T207): `report_window`'s "usage" figure,
    /// replacing a per-session `usage_rows` loop (an N+1 for no reason — the loop never
    /// used anything but the row count).
    pub fn count_usage(&self) -> Result<i64> {
        let mut conn = self.lock()?;
        Ok(usage::table.count().get_result(&mut *conn)?)
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn count_calls(&self) -> Result<i64> {
        let mut conn = self.lock()?;
        Ok(calls::table.count().get_result(&mut *conn)?)
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn set_call_ts(&self, call_id: i32, ts: i64) -> Result<()> {
        let mut conn = self.lock()?;
        diesel::update(calls::table.filter(calls::id.eq(call_id)))
            .set(calls::ts.eq(ts))
            .execute(&mut *conn)?;
        Ok(())
    }

    pub(crate) fn call_session(&self, call_id: i32) -> Result<String> {
        let mut conn = self.lock()?;
        calls::table
            .filter(calls::id.eq(call_id))
            .select(calls::session_id)
            .first(&mut *conn)
            .with_context(|| format!("call {call_id} has no session"))
            .map_err(Into::into)
    }

    /// `[core] store_raw` (T431): with `true`, [`Store::insert_call_io`] saves request bodies
    /// verbatim. Off by default, so a store opened without the config still cleans.
    pub fn set_store_raw(&self, raw: bool) {
        self.store_raw
            .store(raw, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn insert_tokens(
        &self,
        call_id: i32,
        plugin: Option<&str>,
        phase: &str,
        source: &str,
        n_tokens: i64,
    ) -> Result<()> {
        let mut conn = self.lock()?;
        diesel::insert_into(tokens::table)
            .values((
                tokens::call_id.eq(call_id),
                tokens::plugin.eq(plugin),
                tokens::phase.eq(phase),
                tokens::source.eq(source),
                tokens::n_tokens.eq(n_tokens),
            ))
            .execute(&mut *conn)?;
        Ok(())
    }

    /// Proxy ground truth (plan T5.1): one `usage` row per API request.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_usage(
        &self,
        session: &str,
        model: Option<&str>,
        api: &str,
        input: i64,
        cache_create: i64,
        cache_read: i64,
        output: i64,
        call_id: i32,
    ) -> Result<()> {
        let mut conn = self.lock()?;
        diesel::insert_into(usage::table)
            .values((
                usage::session.eq(session),
                usage::model.eq(model),
                usage::api.eq(api),
                usage::input.eq(input),
                usage::cache_create.eq(cache_create),
                usage::cache_read.eq(cache_read),
                usage::output.eq(output),
                usage::call_id.eq(call_id),
            ))
            .execute(&mut *conn)?;
        Ok(())
    }

    /// Many `usage` rows for one call in a single transaction — a Batch results file (T385.4)
    /// carries one per request. Each row is `(model, api, [input, cache_create, cache_read,
    /// output])`, the same four counters as [`Store::insert_usage`].
    pub fn insert_usage_rows(
        &self,
        session: &str,
        call_id: i32,
        rows: &[(Option<&str>, &str, [i64; 4])],
    ) -> Result<()> {
        let mut conn = self.lock()?;
        conn.transaction(|conn| -> Result<()> {
            // 8 binds per row, well under SQLite's 32766-variable cap per statement.
            for chunk in rows.chunks(500) {
                let values: Vec<_> = chunk
                    .iter()
                    .map(|(model, api, [input, cache_create, cache_read, output])| {
                        (
                            usage::session.eq(session),
                            usage::model.eq(*model),
                            usage::api.eq(*api),
                            usage::input.eq(*input),
                            usage::cache_create.eq(*cache_create),
                            usage::cache_read.eq(*cache_read),
                            usage::output.eq(*output),
                            usage::call_id.eq(call_id),
                        )
                    })
                    .collect();
                diesel::insert_into(usage::table)
                    .values(&values)
                    .execute(conn)?;
            }
            Ok(())
        })
    }

    /// Re-tag a call once its response showed what it was (T385.4: an OpenAI file download
    /// that held Batch results).
    pub fn set_call_kind(&self, id: i32, kind: &str) -> Result<()> {
        let mut conn = self.lock()?;
        diesel::update(calls::table.filter(calls::id.eq(id)))
            .set(calls::kind.eq(kind))
            .execute(&mut *conn)?;
        Ok(())
    }

    /// Test-only: one proxy `usage` turn — the session row, a bare `api_request` call
    /// and the `usage` row. Tests that need request bodies (cache-bust causes) still
    /// write their own `call_io`.
    #[cfg(any(test, feature = "test-util"))]
    pub fn insert_proxy_turn(
        &self,
        session: &str,
        input: i64,
        cache_create: i64,
        cache_read: i64,
        output: i64,
    ) -> Result<()> {
        self.upsert_session(session, None, None, None, Some("proxy"))?;
        let id = self.insert_call(
            session,
            "proxy",
            "api_request",
            None,
            None,
            None,
            None,
            Some("/v1/messages"),
        )?;
        self.insert_usage(
            session,
            Some("m"),
            "anthropic",
            input,
            cache_create,
            cache_read,
            output,
            id,
        )
    }

    /// Test-only: one `usage` row for `session` (attributed to `host`, a seeded slug) at a
    /// fixed `ts`, so day and month bucketing can be pinned. `legs` is input, cache write,
    /// cache read, output.
    #[cfg(any(test, feature = "test-util"))]
    pub fn insert_usage_at(
        &self,
        session: &str,
        host: Option<&str>,
        model: &str,
        ts: i64,
        legs: [i64; 4],
    ) -> Result<()> {
        let host_id = match host {
            Some(h) => self.host_id(h)?,
            None => None,
        };
        self.upsert_session(session, host_id, None, None, Some("proxy"))?;
        let call = self.insert_call(
            session,
            "proxy",
            "api_request",
            None,
            None,
            None,
            None,
            Some("/v1/messages"),
        )?;
        let [input, cache_create, cache_read, output] = legs;
        self.insert_usage(
            session,
            Some(model),
            "anthropic",
            input,
            cache_create,
            cache_read,
            output,
            call,
        )?;
        let mut conn = self.lock()?;
        diesel::update(usage::table.filter(usage::call_id.eq(call)))
            .set(usage::ts.eq(ts))
            .execute(&mut *conn)?;
        Ok(())
    }

    /// Provider counters for an api_request (plan T5.1): one `tokens` row,
    /// `phase = 'after'`, `source = 'provider'`, carrying the four counters. `total` comes
    /// from the wire ([`rtok_plugin_sdk`-side `Wire::provider_total`]); the counters are
    /// disjoint on every wire (OpenAI/Gemini `input` has the cached slice subtracted).
    pub fn insert_provider_tokens(
        &self,
        call_id: i32,
        total: i64,
        input: i64,
        cache_create: i64,
        cache_read: i64,
        output: i64,
    ) -> Result<()> {
        let mut conn = self.lock()?;
        diesel::insert_into(tokens::table)
            .values((
                tokens::call_id.eq(call_id),
                tokens::phase.eq("after"),
                tokens::source.eq("provider"),
                tokens::n_tokens.eq(total),
                tokens::input.eq(input),
                tokens::output.eq(output),
                tokens::cache_create.eq(cache_create),
                tokens::cache_read.eq(cache_read),
            ))
            .execute(&mut *conn)?;
        Ok(())
    }

    /// Sessions that have usage rows, oldest first (`rtok stats --cache`, T5.5).
    pub fn usage_sessions(&self) -> Result<Vec<String>> {
        use diesel::dsl::min;
        let mut conn = self.lock()?;
        usage::table
            .group_by(usage::session)
            .select(usage::session)
            .order((min(usage::ts), min(usage::id)))
            .load::<String>(&mut *conn)
            .map_err(Into::into)
    }

    /// Usage rows for one session, newest first (proxy Check, later `stats`).
    pub fn usage_rows(&self, session: &str) -> Result<Vec<UsageRow>> {
        let mut conn = self.lock()?;
        let rows = usage::table
            .filter(usage::session.eq(session))
            .order((usage::ts.desc(), usage::id.desc()))
            .select((
                usage::session,
                usage::model,
                usage::api,
                usage::input,
                usage::cache_create,
                usage::cache_read,
                usage::output,
                usage::call_id,
            ))
            .load::<(
                String,
                Option<String>,
                String,
                i64,
                i64,
                i64,
                i64,
                Option<i32>,
            )>(&mut *conn)?;
        Ok(rows
            .into_iter()
            .map(
                |(session, model, api, input, cache_create, cache_read, output, call_id)| {
                    UsageRow {
                        session,
                        model,
                        api,
                        input,
                        cache_create,
                        cache_read,
                        output,
                        call_id: call_id.map(i64::from),
                    }
                },
            )
            .collect())
    }

    /// The dashboard Overview's CTT and its last `turns` per-turn contexts, in the order of
    /// [`Self::usage_sessions`] then [`Self::usage_rows`] reversed (T15.3). Two reads: the
    /// Overview used to load every usage row, one query per session, on each 2 s tick.
    pub fn usage_ctt(&self, turns: i64) -> Result<(i64, Vec<i64>)> {
        let mut conn = self.lock()?;
        let ctt: i64 = sql_ext::UsageCtt.get_result(&mut *conn)?;
        let mut tail: Vec<i64> = sql_ext::UsageCttTail { turns }.load(&mut *conn)?;
        tail.reverse();
        Ok((ctt, tail))
    }

    pub fn usage_by_api(&self) -> Result<Vec<ApiUsage>> {
        let mut conn = self.lock()?;
        let rows = usage::table
            .group_by(usage::api)
            .select((
                usage::api,
                sum_bigint(usage::input),
                sum_bigint(usage::cache_create),
                sum_bigint(usage::cache_read),
                sum_bigint(usage::output),
            ))
            .order(usage::api)
            .load::<(String, Option<i64>, Option<i64>, Option<i64>, Option<i64>)>(&mut *conn)?;
        Ok(rows
            .into_iter()
            .map(|(api, input, cache_create, cache_read, output)| ApiUsage {
                api,
                input: input.unwrap_or(0),
                cache_create: cache_create.unwrap_or(0),
                cache_read: cache_read.unwrap_or(0),
                output: output.unwrap_or(0),
            })
            .collect())
    }

    /// Usage totals grouped by the `calls.kind` of the request that produced them, which is
    /// the proxy lane that handled it (T385.6), and by the service tier the provider reported
    /// (T385.12.2). Every proxy `usage` row carries its call, so the inner join drops nothing.
    pub fn usage_by_lane_tier(&self) -> Result<Vec<LaneUsage>> {
        type Row = (
            String,
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
        );
        let mut conn = self.lock()?;
        let rows: Vec<Row> = usage::table
            .inner_join(calls::table)
            .group_by((calls::kind, calls::service_tier))
            .select((
                calls::kind,
                calls::service_tier,
                sum_bigint(usage::input),
                sum_bigint(usage::cache_create),
                sum_bigint(usage::cache_read),
                sum_bigint(usage::output),
            ))
            .order((calls::kind, calls::service_tier))
            .load(&mut *conn)?;
        Ok(rows
            .into_iter()
            .map(
                |(kind, tier, input, cache_create, cache_read, output)| LaneUsage {
                    kind,
                    tier,
                    input: input.unwrap_or(0),
                    cache_create: cache_create.unwrap_or(0),
                    cache_read: cache_read.unwrap_or(0),
                    output: output.unwrap_or(0),
                },
            )
            .collect())
    }

    /// Usage totals grouped by model (`rtok stats --price`, T49.1). One statement,
    /// like [`Self::usage_by_api`]: `NULL` models already group into one bucket under
    /// plain `GROUP BY model` (grouping treats every `NULL` as equal), so the SQL side
    /// only needs the raw column. The old raw SQL grouped by `COALESCE(model, 'unknown')`,
    /// so a `NULL`-model group and a row whose model is literally `"unknown"` merged into
    /// one row — replicated here by folding the `NULL → "unknown"` rows into a
    /// `BTreeMap<String, ModelUsage>` keyed by the display name, which also gives the sort
    /// (Diesel cannot validate a `CASE` as the same grouped expression once it also appears
    /// inside `ORDER BY`/`SELECT` — a Diesel 2.3.13 `GROUP BY`-over-computed-expression gap,
    /// not a raw-SQL fallback).
    pub fn usage_by_model(&self) -> Result<Vec<ModelUsage>> {
        self.model_usage(false)
    }

    /// [`Self::usage_by_model`] with the usage of Batch-lane calls (T385.12.1) listed under
    /// `<model>@batch` and that of calls the provider served on Flex (T385.12.2) under
    /// `<model>@flex`, the keys of the matching `[stats.prices]` rows. A Batch call is never
    /// Flex, so the lane wins.
    pub fn usage_by_model_tier(&self) -> Result<Vec<ModelUsage>> {
        self.model_usage(true)
    }

    /// [`Self::usage_by_model_tier`] split by the `calls.kind` (the proxy lane) that produced
    /// the usage (T385.10), so `rtok stats --price` can cost each lane at the model that
    /// really served it. A usage row with no call reads back with `kind = None`.
    pub fn usage_by_lane_model_tier(&self) -> Result<Vec<LaneModelUsage>> {
        self.lane_model_usage(true)
    }

    fn model_usage(&self, by_tier: bool) -> Result<Vec<ModelUsage>> {
        let mut by_model: BTreeMap<String, ModelUsage> = BTreeMap::new();
        for row in self.lane_model_usage(by_tier)? {
            let u = row.usage;
            let entry = by_model.entry(u.model.clone()).or_insert(ModelUsage {
                model: u.model,
                input: 0,
                cache_create: 0,
                cache_read: 0,
                output: 0,
            });
            entry.input += u.input;
            entry.cache_create += u.cache_create;
            entry.cache_read += u.cache_read;
            entry.output += u.output;
        }
        Ok(by_model.into_values().collect())
    }

    fn lane_model_usage(&self, by_tier: bool) -> Result<Vec<LaneModelUsage>> {
        type Row = (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
        );
        let batch = BATCH_CALL_KIND;
        let mut conn = self.lock()?;
        let rows: Vec<Row> = usage::table
            .left_join(calls::table)
            .group_by((usage::model, calls::kind, calls::service_tier))
            .select((
                usage::model,
                calls::kind.nullable(),
                calls::service_tier.nullable(),
                sum_bigint(usage::input),
                sum_bigint(usage::cache_create),
                sum_bigint(usage::cache_read),
                sum_bigint(usage::output),
            ))
            .load(&mut *conn)?;
        let mut merged: BTreeMap<(Option<String>, String), LaneModelUsage> = BTreeMap::new();
        for (model, kind, tier, input, cache_create, cache_read, output) in rows {
            let mut model = model.unwrap_or_else(|| "unknown".to_string());
            if by_tier && kind.as_deref() == Some(batch) {
                model.push_str("@batch");
            } else if by_tier && tier.as_deref() == Some(FLEX_SERVICE_TIER) {
                model.push_str("@flex");
            }
            let entry = merged
                .entry((kind.clone(), model.clone()))
                .or_insert_with(|| LaneModelUsage {
                    kind,
                    usage: ModelUsage {
                        model,
                        input: 0,
                        cache_create: 0,
                        cache_read: 0,
                        output: 0,
                    },
                });
            entry.usage.input += input.unwrap_or(0);
            entry.usage.cache_create += cache_create.unwrap_or(0);
            entry.usage.cache_read += cache_read.unwrap_or(0);
            entry.usage.output += output.unwrap_or(0);
        }
        Ok(merged.into_values().collect())
    }

    /// Usage grouped by session, model and timestamp for `rtok agents usage` (T358.1), with
    /// `ts` in `[since, until)`. The caller cuts day and month boundaries in a time zone, so
    /// the store never sees one; Diesel 2.3 cannot `GROUP BY` a computed `ts / N` bucket (the
    /// gap [`Self::usage_by_model`] documents), so the grain is the request. The host comes from a second read of
    /// `sessions` because Diesel 2.3 cannot group a join's columns across tables (the same gap
    /// as [`Self::usage_by_model`]); a session with no host row (an older proxy-only session)
    /// reads back with `host = None`, which the caller labels by `api`.
    pub fn usage_slices(&self, since: i64, until: i64) -> Result<Vec<UsageSlice>> {
        type Row = (
            String,
            String,
            Option<String>,
            i64,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
        );
        let mut conn = self.lock()?;
        let rows: Vec<Row> = usage::table
            .filter(usage::ts.ge(since).and(usage::ts.lt(until)))
            .group_by((usage::api, usage::session, usage::model, usage::ts))
            .select((
                usage::api,
                usage::session,
                usage::model,
                usage::ts,
                sum_bigint(usage::input),
                sum_bigint(usage::cache_create),
                sum_bigint(usage::cache_read),
                sum_bigint(usage::output),
            ))
            .load(&mut *conn)?;
        let host_of = host_by_session(&mut conn)?;
        Ok(rows
            .into_iter()
            .map(
                |(api, session, model, ts, input, cache_create, cache_read, output)| UsageSlice {
                    host: host_of.get(&session).cloned().flatten(),
                    api,
                    session,
                    model,
                    ts,
                    input: input.unwrap_or(0),
                    cache_create: cache_create.unwrap_or(0),
                    cache_read: cache_read.unwrap_or(0),
                    output: output.unwrap_or(0),
                },
            )
            .collect())
    }

    /// One row per session the store knows (T25.1, D27): the single read behind the
    /// Sessions page and `rtok agent sessions`. One statement — `usage` (by `session`)
    /// and `calls` (by `session_id`) are pre-aggregated per session because joining
    /// both flat to `sessions` would fan every `usage` row out across every `calls`
    /// row and multiply the token sums. `since` windows on `sessions.started_at`
    /// (unix seconds; 0 = every session) while the totals stay whole-session — a
    /// session's cost is what it spent, not what it spent after an arbitrary line.
    /// Newest first; `ended_at IS NULL` is "live". A session with no `usage` rows yet
    /// still appears, zeroed, with `last_activity = started_at`.
    pub fn session_totals(&self, since: i64) -> Result<Vec<SessionTotals>> {
        self.recent_session_totals(since, -1)
    }

    /// [`Store::session_totals`] capped at the newest `limit` sessions in SQL, so the Sessions
    /// page does not load every session to show a screenful. A negative `limit` is no cap.
    pub fn recent_session_totals(&self, since: i64, limit: i64) -> Result<Vec<SessionTotals>> {
        let mut conn = self.lock()?;
        // tot / last_u / act / prov: four CTEs in `sql_ext::RecentSessionTotals`.
        sql_ext::RecentSessionTotals { since, limit }
            .load(&mut *conn)
            .map_err(Into::into)
    }

    /// The Calls page's one read (T15.5, D27): the newest `limit` `calls` rows, newest
    /// first, each with the slugs its ids point at and — when the call recorded one —
    /// its newest `usage` row linked (the same linkage [`Store::call_detail`] serves the
    /// otel span). One statement, so no renderer can re-derive a field differently.
    pub fn recent_calls(&self, limit: i64) -> Result<Vec<CallRow>> {
        let mut conn = self.lock()?;
        sql_ext::RecentCalls { limit }
            .load(&mut *conn)
            .map_err(Into::into)
    }

    /// `models.slug` recorded on a call — the proxy Check asserts it equals the request `model`.
    pub fn model_slug_of_call(&self, call_id: i32) -> Result<Option<String>> {
        let mut conn = self.lock()?;
        calls::table
            .inner_join(schema::models::table)
            .filter(calls::id.eq(call_id))
            .select(schema::models::slug)
            .first(&mut *conn)
            .optional()
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_log(
        &self,
        level: &str,
        source: &str,
        name: &str,
        message: &str,
        session: Option<&str>,
        call_id: Option<i32>,
        plugin: Option<&str>,
    ) -> Result<()> {
        let mut conn = self.lock()?;
        diesel::insert_into(logs::table)
            .values((
                logs::level.eq(level),
                logs::source.eq(source),
                logs::name.eq(name),
                logs::message.eq(message),
                logs::session.eq(session),
                logs::call_id.eq(call_id),
                logs::plugin.eq(plugin),
            ))
            .execute(&mut *conn)?;
        Ok(())
    }

    /// T204 test helper: how many `logs` rows match this `level`/`name` (the funnel's plugin-id
    /// column, see `insert_log`'s callers). Not for production code — a caller that needs this
    /// for real belongs on the `rtok logs`/`doctor` read path instead.
    #[cfg(any(test, feature = "test-util"))]
    pub fn count_logs(&self, level: &str, name: &str) -> Result<i64> {
        let mut conn = self.lock()?;
        logs::table
            .filter(logs::level.eq(level))
            .filter(logs::name.eq(name))
            .count()
            .get_result(&mut *conn)
            .map_err(Into::into)
    }
}
