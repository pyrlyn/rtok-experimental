// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T282 (D34): the rtok agent id. The host's own session id collides across hosts and is
//! missing on several (`research.md` §26), so rtok issues its own random UUIDv4 per host session,
//! shown as its first 8 hex chars and resolved from any unique prefix of 4+ hex chars.
//! `parent_key` is `""` for the main window or the host's own sub-agent `agent_id`
//! (`HookInput::agent_id`, `research.md` §17.2); `parent_id` is the resolved rtok id of a
//! sub-agent's parent row.

use std::time::Duration;

use crate::Result;
use diesel::prelude::*;
use serde::Serialize;

use super::Store;
use super::schema::{agents, hosts};
use super::{coalesce, substr, unixepoch};

/// One `agents` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRow {
    pub id: String,
    pub host_id: i32,
    pub host_session_id: String,
    pub parent_key: String,
    pub parent_id: Option<String>,
    pub cwd: Option<String>,
    pub started_at: i64,
    pub last_seen: i64,
    pub ended_at: Option<i64>,
    pub activity: Option<String>,
    /// T283.3: the hook process's ancestors, nearest first (empty when never recorded).
    pub ancestors: Vec<i32>,
}

type AgentTuple = (
    String,
    i32,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
);

fn row_from(t: AgentTuple) -> AgentRow {
    AgentRow {
        id: t.0,
        host_id: t.1,
        host_session_id: t.2,
        parent_key: t.3,
        parent_id: t.4,
        cwd: t.5,
        started_at: t.6,
        last_seen: t.7,
        ended_at: t.8,
        activity: t.9,
        ancestors: t
            .10
            .iter()
            .flat_map(|a| a.split_whitespace())
            .filter_map(|p| p.parse().ok())
            .collect(),
    }
}

/// Column list `live_agents`/`agent_row` select — kept in one place so the tuple order in
/// [`AgentTuple`] and [`row_from`] only has to match here.
fn agent_cols() -> (
    agents::id,
    agents::host_id,
    agents::host_session_id,
    agents::parent_key,
    agents::parent_id,
    agents::cwd,
    agents::started_at,
    agents::last_seen,
    agents::ended_at,
    agents::activity,
    agents::ancestors,
) {
    (
        agents::id,
        agents::host_id,
        agents::host_session_id,
        agents::parent_key,
        agents::parent_id,
        agents::cwd,
        agents::started_at,
        agents::last_seen,
        agents::ended_at,
        agents::activity,
        agents::ancestors,
    )
}

/// One agent joined to its host's slug — [`Store::agent_detail`], the shape `rtok agents
/// whoami` (T283) prints and serializes for `--json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentDetail {
    pub id: String,
    pub short: String,
    pub host: String,
    pub host_session_id: String,
    pub parent_id: Option<String>,
    pub cwd: Option<String>,
    pub started_at: i64,
    pub last_seen: i64,
    pub ended_at: Option<i64>,
    pub activity: Option<String>,
    /// What the agent says it is busy with (`rtok agents status`, T284).
    pub status_text: Option<String>,
}

type AgentDetailTuple = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
);

fn agent_detail_from(t: AgentDetailTuple) -> AgentDetail {
    let short = t.0.chars().take(8).collect();
    AgentDetail {
        id: t.0,
        short,
        host: t.1,
        host_session_id: t.2,
        parent_id: t.3,
        cwd: t.4,
        started_at: t.5,
        last_seen: t.6,
        ended_at: t.7,
        activity: t.8,
        status_text: t.9,
    }
}

/// `agents ⋈ hosts` rows in the [`AgentDetailTuple`] order — the one select every
/// [`AgentDetail`] reader shares.
macro_rules! agent_details {
    () => {
        agents::table.inner_join(hosts::table).select((
            agents::id,
            hosts::slug,
            agents::host_session_id,
            agents::parent_id,
            agents::cwd,
            agents::started_at,
            agents::last_seen,
            agents::ended_at,
            agents::activity,
            agents::status_text,
        ))
    };
}

/// humantime window (`30m`, `0s`, `24h`, `7d`) shared by `[agents] idle` and the worktree
/// `--idle` / `--stale-lock` flags: one grammar, not the `--since` days grammar. `source`
/// names the flag or key in the error, so a bad `--idle` is never blamed on `--since`.
pub fn parse_idle_from(idle: &str, source: &str) -> anyhow::Result<Duration> {
    humantime::parse_duration(idle).map_err(|e| anyhow::anyhow!("bad {source} {idle:?}: {e}"))
}

/// `[agents] idle` (e.g. `"30m"`) in whole seconds, the one parser of that key.
pub fn idle_secs(idle: &str) -> Result<i64> {
    parse_idle_from(idle, "[agents] idle")?
        .as_secs()
        .try_into()
        .map_err(|_| anyhow::anyhow!("[agents] idle {idle:?} is out of range"))
        .map_err(Into::into)
}

fn main_agent_id(
    conn: &mut SqliteConnection,
    host_id: i32,
    host_session: &str,
) -> Result<Option<String>> {
    Ok(agents::table
        .filter(agents::host_id.eq(host_id))
        .filter(agents::host_session_id.eq(host_session))
        .filter(agents::parent_key.eq(""))
        .select(agents::id)
        .first(conn)
        .optional()?)
}

impl Store {
    /// True when a sub-agent has registered in this host session (T929): more than one context
    /// window shares the session, so a surface that cannot tell them apart must not answer one
    /// with a pointer to what another was shown.
    pub fn session_has_subagent(&self, session: &str) -> Result<bool> {
        let mut conn = self.lock()?;
        Ok(diesel::select(diesel::dsl::exists(
            agents::table
                .filter(agents::host_session_id.eq(session))
                .filter(agents::parent_key.ne("")),
        ))
        .get_result(&mut *conn)?)
    }

    /// Ensure the row for this host session (or, when `parent_key` names one, its sub-agent)
    /// exists and is fresh; returns its rtok id (existing on repeat, else a freshly minted
    /// UUIDv4). One indexed upsert on the hot path (`parent_key: None`); a sub-agent first
    /// resolves its parent's id with one extra indexed read, so `parent_id` is set from the
    /// row's very first insert when the main row is already there. When it is not (the
    /// sub-agent's event came first), the main row's registration adopts every unparented
    /// sub-agent of its session, and a sub-agent's own repeat fills a missing parent (T613).
    /// `activity` follows [`Store::touch_agent`]'s rule: `None`
    /// leaves whatever is already stored untouched (`register_agent` never blanks a hook
    /// event's activity when a later event, e.g. `SessionEnd`, only needs the id back).
    /// A repeat also clears `ended_at`, so a resumed session is live again (T324).
    pub fn register_agent(
        &self,
        host_id: i32,
        host_session: &str,
        parent_key: Option<&str>,
        cwd: Option<&str>,
        activity: Option<&str>,
    ) -> Result<String> {
        let mut conn = self.lock()?;
        let key = parent_key.unwrap_or("");
        // One write transaction, so a sub-agent and its main row registered by two hook
        // processes at once cannot interleave into an unparented child (T613).
        conn.immediate_transaction(|conn| {
            let parent_id = match key.is_empty() {
                true => None,
                false => main_agent_id(conn, host_id, host_session)?,
            };
            let id = uuid::Uuid::new_v4().to_string();
            let row_id: String = diesel::insert_into(agents::table)
                .values((
                    agents::id.eq(&id),
                    agents::host_id.eq(host_id),
                    agents::host_session_id.eq(host_session),
                    agents::parent_key.eq(key),
                    agents::parent_id.eq(parent_id.as_deref()),
                    agents::cwd.eq(cwd),
                    agents::activity.eq(activity),
                ))
                .on_conflict((agents::host_id, agents::host_session_id, agents::parent_key))
                .do_update()
                .set((
                    agents::last_seen.eq(unixepoch().assume_not_null()),
                    // A registering event is proof of life: a resumed session revives its row.
                    agents::ended_at.eq(None::<i64>),
                    // Repairs a row an older build left unparented; a parent already set stays.
                    agents::parent_id.eq(coalesce(
                        agents::parent_id,
                        diesel::upsert::excluded(agents::parent_id),
                    )),
                    agents::cwd.eq(coalesce(diesel::upsert::excluded(agents::cwd), agents::cwd)),
                    agents::activity.eq(coalesce(
                        diesel::upsert::excluded(agents::activity),
                        agents::activity,
                    )),
                ))
                .returning(agents::id)
                .get_result(conn)?;
            if key.is_empty() {
                // The main row's own event adopts the session's unparented sub-agents (rows an
                // older build wrote before this main row existed); a no-op once none are left.
                diesel::update(
                    agents::table
                        .filter(agents::host_id.eq(host_id))
                        .filter(agents::host_session_id.eq(host_session))
                        .filter(agents::parent_key.ne(""))
                        .filter(agents::parent_id.is_null()),
                )
                .set(agents::parent_id.eq(&row_id))
                .execute(conn)?;
            }
            Ok(row_id)
        })
    }

    /// The main (non-sub-agent) row of one host session, without touching it — T473's lookup
    /// for a caller that knows the host session id but must not create a row for it.
    pub fn main_agent(&self, host_id: i32, host_session: &str) -> Result<Option<String>> {
        let mut conn = self.lock()?;
        main_agent_id(&mut conn, host_id, host_session)
    }

    /// Set `last_seen` to now and `activity` to exactly what is passed (unlike
    /// [`Store::register_agent`], `None` here clears it) — for a caller that already holds
    /// the id and skips the host/session lookup (T283's MCP surface).
    pub fn touch_agent(&self, id: &str, activity: Option<&str>) -> Result<()> {
        let mut conn = self.lock()?;
        diesel::update(agents::table.filter(agents::id.eq(id)))
            .set((
                agents::last_seen.eq(unixepoch().assume_not_null()),
                agents::activity.eq(activity),
            ))
            .execute(&mut *conn)?;
        Ok(())
    }

    /// Stamp `ended_at`. `ts` comes from the caller (matches `end_session`'s convention) so a
    /// test can drive the clock and the hook path can share one `SystemTime::now()` read.
    pub fn end_agent(&self, id: &str, ts: i64) -> Result<()> {
        let mut conn = self.lock()?;
        diesel::update(agents::table.filter(agents::id.eq(id)))
            .set(agents::ended_at.eq(ts))
            .execute(&mut *conn)?;
        Ok(())
    }

    /// The one id whose text starts with `prefix`. `Err("unknown")` for zero matches,
    /// `Err("ambiguous: <ids>")` for more than one — never guesses. D34: at least 4 chars.
    pub fn resolve_agent(&self, prefix: &str) -> Result<String> {
        if prefix.chars().count() < 4 {
            bail!("an agent id prefix needs at least 4 characters");
        }
        let mut conn = self.lock()?;
        let len = i32::try_from(prefix.len()).unwrap_or(i32::MAX);
        let matches: Vec<String> = agents::table
            .filter(substr(agents::id, 1, len).eq(prefix))
            .select(agents::id)
            .load(&mut *conn)?;
        match matches.len() {
            0 => bail!("unknown"),
            1 => Ok(matches.into_iter().next().expect("len == 1")),
            _ => bail!("ambiguous: {}", matches.join(", ")),
        }
    }

    /// One agent by its exact id, host slug instead of `hosts.id` — the shape `rtok agents
    /// whoami` (T283) prints. The caller resolves a prefix through [`Store::resolve_agent`]
    /// first, so this is always looked up by the canonical id it returned.
    pub fn agent_detail(&self, id: &str) -> Result<Option<AgentDetail>> {
        let mut conn = self.lock()?;
        agent_details!()
            .filter(agents::id.eq(id))
            .first::<AgentDetailTuple>(&mut *conn)
            .optional()
            .map(|o| o.map(agent_detail_from))
            .map_err(Into::into)
    }

    /// Every agent (main rows and sub-agents) of these host sessions, oldest first — the
    /// rows `rtok agents sessions` (T284) nests under the session they belong to.
    pub fn agents_of_sessions(&self, sessions: &[String]) -> Result<Vec<AgentDetail>> {
        let mut conn = self.lock()?;
        Ok(agent_details!()
            .filter(agents::host_session_id.eq_any(sessions))
            .order((agents::started_at, agents::id))
            .load::<AgentDetailTuple>(&mut *conn)?
            .into_iter()
            .map(agent_detail_from)
            .collect())
    }

    /// The sub-agents whose `parent_id` is `id`, oldest first (`rtok agents show`, T284).
    pub fn agent_children(&self, id: &str) -> Result<Vec<AgentDetail>> {
        let mut conn = self.lock()?;
        Ok(agent_details!()
            .filter(agents::parent_id.eq(id))
            .order((agents::started_at, agents::id))
            .load::<AgentDetailTuple>(&mut *conn)?
            .into_iter()
            .map(agent_detail_from)
            .collect())
    }

    /// Set (or, with `None`, clear) the agent's own status text (T284); `false` when no
    /// row has this id.
    pub fn set_agent_status(&self, id: &str, text: Option<&str>) -> Result<bool> {
        let mut conn = self.lock()?;
        let n = diesel::update(agents::table.filter(agents::id.eq(id)))
            .set(agents::status_text.eq(text))
            .execute(&mut *conn)?;
        Ok(n > 0)
    }

    /// Every agent with no `ended_at` and a `last_seen` within `idle` (`[agents] idle`'s raw
    /// string, e.g. `"30m"`, parsed by `humantime::parse_duration`) of now. The cutoff is
    /// computed in SQL — `unixepoch() - secs`, not `SystemTime::now()` — so it shares
    /// SQLite's own clock with the `last_seen` values it is compared against.
    pub fn live_agents(&self, idle: &str) -> Result<Vec<AgentRow>> {
        let secs = idle_secs(idle)?;
        let mut conn = self.lock()?;
        Ok(agents::table
            .filter(agents::ended_at.is_null())
            .filter(agents::last_seen.ge(unixepoch().assume_not_null() - secs))
            .select(agent_cols())
            .load::<AgentTuple>(&mut *conn)?
            .into_iter()
            .map(row_from)
            .collect())
    }

    /// Every agent with an `ended_at` (T949): `worktree gc` stops honouring the locks they
    /// left, since nothing unlocks a worktree when its session ends.
    pub fn ended_agent_ids(&self) -> Result<Vec<String>> {
        let mut conn = self.lock()?;
        Ok(agents::table
            .filter(agents::ended_at.is_not_null())
            .select(agents::id)
            .load(&mut *conn)?)
    }

    /// T283.3: record the pids above the hook process that registered `id` (nearest first).
    /// Written only when it differs, so a hook repeating the same chain adds no write; an empty
    /// chain says nothing and keeps what is stored.
    pub fn set_agent_ancestors(&self, id: &str, chain: &[i32]) -> Result<()> {
        if chain.is_empty() {
            return Ok(());
        }
        let text = chain
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        let mut conn = self.lock()?;
        diesel::update(
            agents::table
                .filter(agents::id.eq(id))
                .filter(agents::ancestors.is_null().or(agents::ancestors.ne(&text))),
        )
        .set(agents::ancestors.eq(&text))
        .execute(&mut *conn)?;
        Ok(())
    }

    /// When `id` was last seen, or `None` when the store has no such agent. A missing row is
    /// what makes a claim stale (T442): the agent is not live on this machine.
    pub fn agent_last_seen(&self, id: &str) -> Result<Option<i64>> {
        let mut conn = self.lock()?;
        agents::table
            .filter(agents::id.eq(id))
            .select(agents::last_seen)
            .first::<i64>(&mut *conn)
            .optional()
            .map_err(Into::into)
    }

    /// Test-only: place `last_seen` at an exact time, so a fixture can say "seen before the
    /// MCP process started" without sleeping.
    #[cfg(any(test, feature = "test-util"))]
    pub fn set_agent_last_seen(&self, id: &str, ts: i64) -> Result<()> {
        let mut conn = self.lock()?;
        diesel::update(agents::table.filter(agents::id.eq(id)))
            .set(agents::last_seen.eq(ts))
            .execute(&mut *conn)?;
        Ok(())
    }

    /// Test/debug: one row by its exact id. `pub(crate)` so `hooks::mod`'s dispatch-level
    /// fixture tests can assert on what a full hook run wrote (T282).
    #[cfg(any(test, feature = "test-util"))]
    pub fn agent_row(&self, id: &str) -> Result<Option<AgentRow>> {
        let mut conn = self.lock()?;
        agents::table
            .filter(agents::id.eq(id))
            .select(agent_cols())
            .first::<AgentTuple>(&mut *conn)
            .optional()
            .map(|o| o.map(row_from))
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_is_idempotent_and_returns_the_same_id() {
        let store = Store::open_in_memory().unwrap();
        let claude = store.host_id("claude").unwrap().unwrap();
        let a = store
            .register_agent(claude, "sess-1", None, Some("/repo"), Some("Bash: ls"))
            .unwrap();
        let b = store
            .register_agent(claude, "sess-1", None, Some("/repo"), None)
            .unwrap();
        assert_eq!(a, b);
        let row = store.agent_row(&a).unwrap().unwrap();
        assert_eq!(row.host_session_id, "sess-1");
        assert_eq!(row.parent_key, "");
        assert_eq!(row.parent_id, None);
        // `register`'s second call passed no activity: the first call's sticks (D34: register
        // never blanks it — that is `touch_agent`'s job).
        assert_eq!(row.activity.as_deref(), Some("Bash: ls"));
    }

    /// T929: the MCP `read` tool asks whether another context window may share the session.
    #[test]
    fn session_has_subagent_is_true_only_once_a_child_row_exists() {
        let store = Store::open_in_memory().unwrap();
        let claude = store.host_id("claude").unwrap().unwrap();
        store
            .register_agent(claude, "sess-3", None, None, None)
            .unwrap();
        assert!(!store.session_has_subagent("sess-3").unwrap());
        store
            .register_agent(claude, "sess-3", Some("child"), None, None)
            .unwrap();
        assert!(store.session_has_subagent("sess-3").unwrap());
        assert!(!store.session_has_subagent("sess-4").unwrap());
    }

    #[test]
    fn a_sub_agent_gets_its_own_row_with_the_parent_s_id() {
        let store = Store::open_in_memory().unwrap();
        let claude = store.host_id("claude").unwrap().unwrap();
        let parent = store
            .register_agent(claude, "sess-2", None, None, None)
            .unwrap();
        let child = store
            .register_agent(claude, "sess-2", Some("child-1"), None, None)
            .unwrap();
        assert_ne!(parent, child);
        let child_row = store.agent_row(&child).unwrap().unwrap();
        assert_eq!(child_row.parent_key, "child-1");
        assert_eq!(child_row.parent_id.as_deref(), Some(parent.as_str()));
        // A second sub-agent under the same session is a third, distinct row.
        let child2 = store
            .register_agent(claude, "sess-2", Some("child-2"), None, None)
            .unwrap();
        assert_ne!(child2, child);
        // Repeating the first sub-agent's key upserts the same child row, not a fourth.
        let child_again = store
            .register_agent(claude, "sess-2", Some("child-1"), None, None)
            .unwrap();
        assert_eq!(child_again, child);
    }

    #[test]
    fn touch_sets_activity_and_end_stamps_ended_at() {
        let store = Store::open_in_memory().unwrap();
        let claude = store.host_id("claude").unwrap().unwrap();
        let id = store
            .register_agent(claude, "sess-3", None, None, None)
            .unwrap();
        store.touch_agent(&id, Some("Edit: src/a.rs")).unwrap();
        let row = store.agent_row(&id).unwrap().unwrap();
        assert_eq!(row.activity.as_deref(), Some("Edit: src/a.rs"));
        assert_eq!(row.ended_at, None);
        store.end_agent(&id, 1_800_000_000).unwrap();
        let row = store.agent_row(&id).unwrap().unwrap();
        assert_eq!(row.ended_at, Some(1_800_000_000));
    }

    #[test]
    fn agent_detail_joins_the_host_slug_and_short_id() {
        let store = Store::open_in_memory().unwrap();
        let claude = store.host_id("claude").unwrap().unwrap();
        let id = store
            .register_agent(claude, "sess-detail", None, Some("/repo"), Some("Bash: ls"))
            .unwrap();
        let detail = store.agent_detail(&id).unwrap().unwrap();
        assert_eq!(detail.id, id);
        assert_eq!(detail.short, id[..8]);
        assert_eq!(detail.host, "claude");
        assert_eq!(detail.host_session_id, "sess-detail");
        assert_eq!(detail.cwd.as_deref(), Some("/repo"));
        assert_eq!(detail.activity.as_deref(), Some("Bash: ls"));
        assert_eq!(detail.ended_at, None);
        assert!(
            store
                .agent_detail("ffffffff-0000-7000-8000-000000000000")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn resolve_prefix_exact_ambiguous_and_unknown() {
        let store = Store::open_in_memory().unwrap();
        let claude = store.host_id("claude").unwrap().unwrap();
        let a = store
            .register_agent(claude, "sess-a", None, None, None)
            .unwrap();
        // A hand-picked second id sharing `a`'s first 8 chars, so that prefix is ambiguous.
        let shared = format!("{}-0000-4000-8000-000000000000", &a[..8]);
        diesel::insert_into(agents::table)
            .values((
                agents::id.eq(&shared),
                agents::host_id.eq(claude),
                agents::host_session_id.eq("sess-b"),
                agents::parent_key.eq(""),
            ))
            .execute(&mut *store.lock().unwrap())
            .unwrap();

        assert_eq!(store.resolve_agent(&a).unwrap(), a); // full id: always unique
        let ambiguous = store.resolve_agent(&a[..8]).unwrap_err().to_string();
        assert!(ambiguous.starts_with("ambiguous: "), "{ambiguous}");
        assert!(ambiguous.contains(&a) && ambiguous.contains(&shared));
        let unknown = store.resolve_agent("ffffffff").unwrap_err().to_string();
        assert_eq!(unknown, "unknown");
        let short = store.resolve_agent(&a[..3]).unwrap_err().to_string();
        assert!(short.contains("at least 4"), "{short}");
    }

    #[test]
    fn live_excludes_ended_and_stale_rows() {
        let store = Store::open_in_memory().unwrap();
        let claude = store.host_id("claude").unwrap().unwrap();
        let live = store
            .register_agent(claude, "sess-live", None, None, None)
            .unwrap();
        let ended = store
            .register_agent(claude, "sess-ended", None, None, None)
            .unwrap();
        store.end_agent(&ended, 0).unwrap();
        let stale = store
            .register_agent(claude, "sess-stale", None, None, None)
            .unwrap();
        diesel::update(agents::table.filter(agents::id.eq(&stale)))
            .set(agents::last_seen.eq(unixepoch().assume_not_null() - 10_000))
            .execute(&mut *store.lock().unwrap())
            .unwrap();

        let ids: Vec<String> = store
            .live_agents("60s")
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert!(ids.contains(&live));
        assert!(!ids.contains(&ended), "ended rows are never live");
        assert!(
            !ids.contains(&stale),
            "a last_seen older than idle is never live"
        );
    }

    #[test]
    fn ancestors_are_stored_in_order_and_an_empty_chain_keeps_what_is_there() {
        let store = Store::open_in_memory().unwrap();
        let claude = store.host_id("claude").unwrap().unwrap();
        let id = store
            .register_agent(claude, "sess-anc", None, None, None)
            .unwrap();
        assert!(store.agent_row(&id).unwrap().unwrap().ancestors.is_empty());
        store.set_agent_ancestors(&id, &[300, 200, 100]).unwrap();
        store.set_agent_ancestors(&id, &[300, 200, 100]).unwrap();
        store.set_agent_ancestors(&id, &[]).unwrap();
        let row = store.agent_row(&id).unwrap().unwrap();
        assert_eq!(row.ancestors, [300, 200, 100]);
        // A later hook under a restarted host replaces the chain.
        store.set_agent_ancestors(&id, &[400, 200]).unwrap();
        assert_eq!(store.agent_row(&id).unwrap().unwrap().ancestors, [400, 200]);
    }

    /// `--idle` / `--stale-lock` share `[agents] idle`'s humantime grammar (`30m`, `0s`,
    /// `24h`, `7d`) rather than `--since`'s bare-days one.
    #[test]
    fn parse_idle_from_accepts_the_agents_idle_grammar() {
        assert_eq!(
            parse_idle_from("30m", "--idle").unwrap(),
            Duration::from_secs(30 * 60)
        );
        assert_eq!(parse_idle_from("0s", "--idle").unwrap(), Duration::ZERO);
        assert_eq!(
            parse_idle_from("24h", "--idle").unwrap(),
            Duration::from_secs(24 * 3600)
        );
        assert_eq!(
            parse_idle_from("7d", "--stale-lock").unwrap(),
            Duration::from_secs(7 * 86400)
        );
        assert_eq!(
            parse_idle_from("1m", "--idle").unwrap(),
            Duration::from_secs(60)
        );
        assert_eq!(
            parse_idle_from("1s", "--stale-lock").unwrap(),
            Duration::from_secs(1)
        );
        assert_eq!(
            parse_idle_from("15m", "--idle").unwrap(),
            Duration::from_secs(15 * 60)
        );
        assert_eq!(
            parse_idle_from("10m", "--idle").unwrap(),
            Duration::from_secs(10 * 60)
        );
    }

    /// A bad unit names the flag that was passed, never `--since`.
    #[test]
    fn parse_idle_from_names_the_passed_flag() {
        for flag in ["--idle", "--stale-lock", "[agents] idle"] {
            let err = parse_idle_from("5x", flag).unwrap_err().to_string();
            assert!(err.contains(flag), "{err}");
            assert!(!err.contains("--since"), "{err}");
        }
    }

    #[test]
    fn live_agents_parses_the_configured_idle_window() {
        let store = Store::open_in_memory().unwrap();
        let claude = store.host_id("claude").unwrap().unwrap();
        let id = store
            .register_agent(claude, "sess-idle", None, None, None)
            .unwrap();
        let ids: Vec<String> = store
            .live_agents("30m")
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert!(
            ids.contains(&id),
            "the default [agents] idle (\"30m\") must parse and match"
        );
        assert!(
            store.live_agents("not-a-duration").is_err(),
            "a bad [agents] idle must error, not panic"
        );
    }

    /// T324: a resumed session registers the same row again; it is alive again, not stuck
    /// behind the `ended_at` stamp of its earlier run.
    #[test]
    fn registering_an_ended_agent_again_makes_it_live() {
        let store = Store::open_in_memory().unwrap();
        let claude = store.host_id("claude").unwrap().unwrap();
        let id = store
            .register_agent(claude, "sess-resume", None, None, None)
            .unwrap();
        store.end_agent(&id, 1_800_000_000).unwrap();
        assert!(store.live_agents("30m").unwrap().iter().all(|r| r.id != id));
        let again = store
            .register_agent(claude, "sess-resume", None, None, None)
            .unwrap();
        assert_eq!(again, id);
        assert!(store.live_agents("30m").unwrap().iter().any(|r| r.id == id));
    }

    /// T613: whichever of a session's main and sub-agent rows arrives first, the sub-agent
    /// ends up under the one main row, ids stay stable, and nothing is registered twice.
    #[test]
    fn a_sub_agent_registered_before_its_main_is_attached_to_it() {
        let store = Store::open_in_memory().unwrap();
        let claude = store.host_id("claude").unwrap().unwrap();
        let child = store
            .register_agent(claude, "late-main", Some("sub-1"), None, Some("Bash: ls"))
            .unwrap();
        let main = store
            .register_agent(claude, "late-main", None, Some("/r"), None)
            .unwrap();
        let again = store
            .register_agent(claude, "late-main", Some("sub-1"), None, None)
            .unwrap();
        assert_eq!(
            store.agent_row(&child).unwrap().unwrap().parent_id,
            Some(main.clone()),
            "the main row's arrival adopted it"
        );
        assert_eq!(again, child, "same sub-agent id");
        assert_eq!(
            store.main_agent(claude, "late-main").unwrap(),
            Some(main.clone())
        );
        let row = store.agent_row(&child).unwrap().unwrap();
        assert_eq!(row.parent_id.as_deref(), Some(main.as_str()));
        assert_eq!(row.activity.as_deref(), Some("Bash: ls"), "activity kept");
        let kids = store.agent_children(&main).unwrap();
        assert_eq!(
            kids.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(),
            [child.as_str()]
        );
        let rows = store
            .agents_of_sessions(&["late-main".to_string()])
            .unwrap();
        assert_eq!(rows.len(), 2, "one main row and one sub-agent row");
        assert_eq!(rows.iter().filter(|r| r.parent_id.is_none()).count(), 1);
        // Another session's sub-agent is not adopted.
        let other = store
            .register_agent(claude, "elsewhere", Some("sub-1"), None, None)
            .unwrap();
        assert_ne!(
            store
                .agent_row(&other)
                .unwrap()
                .unwrap()
                .parent_id
                .as_deref(),
            Some(main.as_str())
        );
    }

    /// T613: a row an older build left unparented is repaired in place, by the main row's
    /// next registration or by its own, and a parent already set is never replaced.
    #[test]
    fn existing_orphan_sub_agents_are_repaired_in_place() {
        let store = Store::open_in_memory().unwrap();
        let claude = store.host_id("claude").unwrap().unwrap();
        let orphan = |session: &str, key: &str| {
            let id = uuid::Uuid::new_v4().to_string();
            diesel::insert_into(agents::table)
                .values((
                    agents::id.eq(&id),
                    agents::host_id.eq(claude),
                    agents::host_session_id.eq(session),
                    agents::parent_key.eq(key),
                ))
                .execute(&mut *store.lock().unwrap())
                .unwrap();
            id
        };
        let a = orphan("legacy", "sub-a");
        let b = orphan("legacy", "sub-b");
        let main = store
            .register_agent(claude, "legacy", None, None, None)
            .unwrap();
        for id in [&a, &b] {
            let row = store.agent_row(id).unwrap().unwrap();
            assert_eq!(row.parent_id.as_deref(), Some(main.as_str()), "{id}");
        }

        let c = orphan("legacy-2", "sub-c");
        let main2 = store
            .register_agent(claude, "legacy-2", None, None, None)
            .unwrap();
        diesel::update(agents::table.filter(agents::id.eq(&c)))
            .set(agents::parent_id.eq(None::<String>))
            .execute(&mut *store.lock().unwrap())
            .unwrap();
        let again = store
            .register_agent(claude, "legacy-2", Some("sub-c"), None, None)
            .unwrap();
        assert_eq!(again, c);
        assert_eq!(
            store.agent_row(&c).unwrap().unwrap().parent_id.as_deref(),
            Some(main2.as_str())
        );
        // A parent that is already set stays, whoever registers.
        diesel::update(agents::table.filter(agents::id.eq(&c)))
            .set(agents::parent_id.eq(&a))
            .execute(&mut *store.lock().unwrap())
            .unwrap();
        store
            .register_agent(claude, "legacy-2", Some("sub-c"), None, None)
            .unwrap();
        store
            .register_agent(claude, "legacy-2", None, None, None)
            .unwrap();
        assert_eq!(
            store.agent_row(&c).unwrap().unwrap().parent_id.as_deref(),
            Some(a.as_str())
        );
    }
}
