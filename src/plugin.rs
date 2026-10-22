// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The plugin contract (plan §1, T0.4). Every token-reduction method implements [`Plugin`].
//!
//! The contract itself lives in the published `rtok-plugin-sdk` crate (D25) and is re-exported
//! here, so `rtok::plugin::*` keeps resolving and an out-of-tree plugin and an in-tree one
//! implement the same types. What stays here is the host side: [`Runtime`], which owns the config
//! and the store, and the [`Plugin`] trait until T23.3 puts the host behind capability traits.
//!
//! Rules (AGENTS.md): fail open, lossless by default, and a saving that is not a
//! [`Measurement`] row does not exist. Default method bodies do nothing, so a plugin
//! implements only the surfaces it declares in its [`Manifest`].

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::config::Config;
use crate::store::Store;
use crate::tokens;

pub use rtok_plugin_sdk::{
    Archive, ArchiveDecision, ArchiveHit, Capabilities, Class, Ctx, DashboardPage, Host, Injection,
    Ledger, Manifest, Measurement, NewObservation, NoteHit, Notes, ObsHit, Plugin, PostToolUse,
    PreCompact, PreToolDecision, PreToolUse, PromptSubmit, ReadCache, SessionStart, SubagentStart,
    Surface, Symbols, ToolDef, ToolResultRef, ToolResults, WireRequest,
};

/// The longest prefix of `text` that estimates to at most `budget` tokens.
///
/// One estimate of the whole text gives the chars-per-token rate; the estimator rounds
/// up, so `chars * budget / est` never keeps too much. The loop after it only absorbs
/// float rounding and runs a couple of times at most — never once per character.
pub fn fit_budget(cx: &Ctx, text: &str, class: Class, budget: u32) -> String {
    let est = cx.estimate(text, class);
    if est <= budget {
        return text.to_string();
    }
    let chars = text.chars().count() as u64;
    let keep = (chars * u64::from(budget) / u64::from(est)) as usize;
    let mut out: String = text.chars().take(keep).collect();
    while !out.is_empty() && cx.estimate(&out, class) > budget {
        out.pop();
    }
    out
}

/// T65.1: same-session content-hash hit. Looks up before the caller archives.
/// Empty or shorter-than-the-pointer bodies stay as they are (fail open / no inflation).
///
/// T127: `context` scopes the hit to the caller's window — a sub-agent's `agent_id`, or
/// `None` for the main window. A surface with no way to know its context (today: the MCP
/// `read` tool) passes `None`, which keeps this exactly as it behaved before T127.
pub fn identical_result(
    host: &dyn Capabilities,
    plugin: &'static str,
    body: &[u8],
    context: Option<&str>,
) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    let sha = crate::store::hex_sha256(body);
    let hit = host.archive_in_session(&sha, context).ok().flatten()?;
    let n = hit.turns.max(1);
    let id = hit.id;
    let short = crate::names::short_id(&id);
    let msg = format!(
        "[rtok {short} · identical to a result {n} turns ago · expand: rtok expand {short}]"
    );
    if body.len() <= msg.len() {
        return None;
    }
    let text = std::str::from_utf8(body).unwrap_or("");
    let _ = host.record(&Measurement {
        plugin,
        kind: "dedup",
        before_bytes: body.len() as u64,
        after_bytes: msg.len() as u64,
        est_before: host.estimate(text, Class::Code),
        est_after: host.estimate(&msg, Class::Code),
        ref_id: Some(id.clone()),
        call_id: host.call_id(),
    });
    Some(msg)
}

/// Everything a plugin may touch: config, the store, and the session id.
/// The archive store is added in T3.1.
pub struct Runtime {
    /// Merged configuration for this run.
    pub config: Config,
    /// The one SQLite file (D8).
    pub store: Store,
    /// Host session id; every measurement is attributed to it.
    pub session: String,
    /// The `calls` row this dispatch runs under (the API request in the proxy), when the
    /// surface has one. `record_call` / `record_plugin_run` nest their rows under it.
    pub call_id: Option<i32>,
    /// The tool call this dispatch serves (`<event>:<tool_use_id>`), when the surface knows
    /// it: a host that delivers one call twice then records it once (T245).
    pub once: Option<String>,
    /// Resolved once from `[hook] host` (same shape as `proxy::ProxyState::new`); an unknown
    /// slug falls back to `other` (6) rather than leaving the session row unattributed.
    host_id: Option<i32>,
    /// Host process cwd, when the surface knows it (the hook surface sets this from the
    /// event's own `cwd`, T25.0). `project` is derived from it at write time.
    pub cwd: Option<String>,
    /// Files the graph watcher has queued but not yet re-indexed (T68.3), per watched root.
    pub graph_watch_pending: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    /// `Some` while [`Runtime::defer_measurements`] is on: `record` queues here and
    /// [`Runtime::flush_measurements`] writes the queue in one transaction.
    deferred: Mutex<Option<Vec<Measurement>>>,
    /// HTTP grants expansion only after this runtime has archived the bytes itself. The
    /// store's first-writer session cannot represent repeated content across sessions.
    archive_scope: Option<Mutex<BTreeSet<String>>>,
    /// The dir [`Runtime::in_memory`] made for this runtime's files; it goes with the runtime.
    scratch: Option<ScratchDir>,
}

/// Removes its directory when dropped. Tests make tens of thousands of in-memory runtimes, and
/// their dirs left behind made every path lookup in `$TMPDIR` slow under load (T643, T644).
struct ScratchDir(std::path::PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Runtime {
    /// Open the store at `config.core.db_path`.
    pub fn open(config: Config, session: impl Into<String>) -> Result<Self> {
        Self::open_with(config, session, crate::store::LockWait::STEADY)
    }

    /// [`Runtime::open`] with the store's own bound on waiting for other processes' locks.
    pub fn open_with(
        config: Config,
        session: impl Into<String>,
        wait: crate::store::LockWait,
    ) -> Result<Self> {
        let store = Store::open_with(&config.core.db_path, wait)?;
        Self::with_store(config, store, session)
    }

    /// Default config with every path under a fresh temp dir + in-memory store, for tests and
    /// examples.
    pub fn in_memory(session: impl Into<String>) -> Result<Self> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("rtok-mem-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let scratch = ScratchDir(dir.clone());
        let mut config = Config::default();
        config.rebase_paths(&dir);
        config.log.path = dir.join("rtok.log");
        let mut rt = Self::with_store(config, Store::open_in_memory()?, session)?;
        rt.scratch = Some(scratch);
        Ok(rt)
    }

    fn with_store(config: Config, store: Store, session: impl Into<String>) -> Result<Self> {
        store.set_store_raw(config.core.store_raw);
        let host_id = store.host_id(&config.hook.host)?.or(Some(6));
        Ok(Self {
            config,
            store,
            session: session.into(),
            call_id: None,
            once: None,
            host_id,
            cwd: None,
            graph_watch_pending: Arc::new(Mutex::new(HashMap::new())),
            deferred: Mutex::new(None),
            archive_scope: None,
            scratch: None,
        })
    }

    /// Start an empty archive capability set for a remote surface. Local surfaces keep the
    /// global archive lookup; restarting a remote server grants no previous capabilities.
    pub(crate) fn restrict_archives(&mut self) {
        self.archive_scope = Some(Mutex::new(BTreeSet::new()));
    }

    /// At most two granted IDs, enough to resolve a prefix or reject an ambiguity without
    /// consulting the shared archive. The ordered set bounds misses to a tree lookup.
    pub(crate) fn scoped_archive_ids(&self, prefix: &str) -> Option<Vec<String>> {
        self.archive_scope.as_ref().map(|scope| {
            scope
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .range(prefix.to_owned()..)
                .take_while(|id| id.starts_with(prefix))
                .take(2)
                .cloned()
                .collect()
        })
    }

    /// Estimated token count for `text` (uncalibrated chars-per-token heuristic, no tokenizer, no network).
    pub fn estimate(&self, text: &str, class: Class) -> u32 {
        tokens::estimate(text, class, &self.config.estimator)
    }

    /// Resolved `[hook] host` id (T282: the hooks dispatcher's own agent-registry writes need
    /// it, the same value `insert_call` already attributes `sessions`/`calls` rows to).
    pub(crate) fn host_id(&self) -> Option<i32> {
        self.host_id
    }

    /// Persist a measurement for this session (the only path for savings into the DB).
    pub fn record(&self, m: &Measurement) -> Result<()> {
        if let Some(queue) = self
            .deferred
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            queue.push(m.clone());
            return Ok(());
        }
        self.store
            .insert_measurement_once(&self.session, m, self.once.as_deref())
            .map_err(Into::into)
    }

    /// Queue every later [`Runtime::record`] instead of writing it. Each write is a lock
    /// acquisition and a WAL commit, and the hook's 10 ms budget (D1) feels every one of them
    /// when other sessions write too; SessionStart records three measurements back to back.
    pub fn defer_measurements(&self) {
        *self
            .deferred
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Vec::new());
    }

    /// Write what [`Runtime::defer_measurements`] queued, in order, in one transaction, and
    /// go back to writing each `record` at once. Never fails the caller: a lost row is a
    /// missing statistic, the same as a failed single `record`.
    pub fn flush_measurements(&self) {
        let queued = self
            .deferred
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .unwrap_or_default();
        if queued.is_empty() {
            return;
        }
        let _ = self
            .store
            .insert_measurements_once(&self.session, &queued, self.once.as_deref());
    }

    pub fn record_call(&self, surface: &str, kind: &str, name: Option<&str>) -> Result<i32> {
        self.insert_call(surface, kind, None, name)
    }

    /// A `plugin_run` row for `plugin`, nested under [`Runtime::call_id`] when set.
    pub fn record_plugin_run(&self, surface: &str, plugin: &str) -> Result<i32> {
        self.insert_call(surface, "plugin_run", Some(plugin), None)
    }

    fn insert_call(
        &self,
        surface: &str,
        kind: &str,
        plugin: Option<&str>,
        name: Option<&str>,
    ) -> Result<i32> {
        let project = project_of(self.cwd.as_deref());
        self.store.upsert_session(
            &self.session,
            self.host_id,
            project.as_deref(),
            self.cwd.as_deref(),
            None,
        )?;
        let id = self.store.insert_call(
            &self.session,
            surface,
            kind,
            self.host_id,
            None,
            None,
            plugin,
            name,
        )?;
        if let Some(parent) = self.call_id {
            self.store.set_call_parent(id, parent)?;
        }
        Ok(id)
    }

    pub fn record_tokens(
        &self,
        call_id: i32,
        plugin: Option<&str>,
        phase: &str,
        source: &str,
        tokens: i64,
    ) -> Result<()> {
        self.store
            .insert_tokens(call_id, plugin, phase, source, tokens)
            .map_err(Into::into)
    }

    /// Never returns `Err` to a plugin (fail open): [`crate::log::record`] is the funnel —
    /// the file line and the `logs` row, each of which swallows its own errors (D1).
    pub fn log(&self, level: &str, source: &str, name: &str, message: &str) {
        crate::log::record(
            &self.config,
            &self.store,
            Some(&self.session),
            None,
            level,
            source,
            name,
            message,
        );
    }
}

/// The project `cwd` belongs to (T25.0): T133's resolver, so a linked worktree's session is
/// attributed to its main repository — no subprocess, as the fail-open hook path needs.
fn project_of(cwd: Option<&str>) -> Option<String> {
    crate::project::project_name(std::path::Path::new(cwd?))
}

/// The `kv` key behind [`Host::plugin_state_set`] (T419). The plugin id leads, so one
/// prefix read gives the stats pages a plugin's whole state.
pub fn plugin_state_key(plugin: &str, key: &str) -> String {
    format!("plugin:{plugin}:{key}")
}

/// The host side of the contract (D25). `Runtime` *is* the host: every capability trait is
/// implemented here by delegating to the one store, and the session and the archive
/// directory come from the context rather than from the plugin's arguments.
///
/// A plugin sees only these traits, which is what lets `rtok-plugin-sdk` stay three
/// dependencies deep while this crate carries SQLite and tree-sitter.
impl Host for Runtime {
    fn session(&self) -> &str {
        &self.session
    }

    fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    fn estimate(&self, text: &str, class: Class) -> u32 {
        Runtime::estimate(self, text, class)
    }

    fn record(&self, m: &Measurement) -> Result<()> {
        Runtime::record(self, m)
    }

    fn plugin_state_set(&self, plugin: &str, key: &str, value: &str) -> Result<()> {
        self.store
            .kv_set(&plugin_state_key(plugin, key), value)
            .map_err(Into::into)
    }

    fn plugin_state_get(&self, plugin: &str, key: &str) -> Result<Option<String>> {
        self.store
            .kv_get(&plugin_state_key(plugin, key))
            .map_err(Into::into)
    }

    fn record_call(&self, surface: &str, kind: &str, name: Option<&str>) -> Result<i32> {
        Runtime::record_call(self, surface, kind, name)
    }

    fn record_plugin_run(&self, surface: &str, plugin: &str) -> Result<i32> {
        Runtime::record_plugin_run(self, surface, plugin)
    }

    fn record_tokens(
        &self,
        call_id: i32,
        plugin: Option<&str>,
        phase: &str,
        source: &str,
        tokens: i64,
    ) -> Result<()> {
        Runtime::record_tokens(self, call_id, plugin, phase, source, tokens)
    }

    fn log(&self, level: &str, source: &str, name: &str, message: &str) {
        Runtime::log(self, level, source, name, message);
    }

    /// One configuration section by dotted path. An unknown path is `Null`, which
    /// deserializes to the plugin's own defaults rather than to an error.
    fn config_json(&self, path: &str) -> serde_json::Value {
        let mut node = match serde_json::to_value(&self.config) {
            Ok(v) => v,
            Err(_) => return serde_json::Value::Null,
        };
        for key in path.split('.') {
            node = match node.get_mut(key) {
                Some(v) => v.take(),
                None => return serde_json::Value::Null,
            };
        }
        node
    }

    fn call_id(&self) -> Option<i32> {
        self.call_id
    }

    fn graph_watch_pending(&self, root: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .graph_watch_pending
            .lock()
            .map(|p| p.get(root).into_iter().flatten().cloned().collect())
            .unwrap_or_default();
        out.sort();
        out
    }

    fn publish_graph_watch_pending(&self, root: &str, paths: &[String]) {
        if let Ok(mut guard) = self.graph_watch_pending.lock() {
            if paths.is_empty() {
                guard.remove(root);
            } else {
                guard.insert(root.to_string(), paths.iter().cloned().collect());
            }
        }
    }
}

impl Archive for Runtime {
    fn put_archive(&self, body: &[u8]) -> Result<String> {
        self.put_archive_for(body, None)
    }

    fn get_archive(&self, id: &str) -> Result<Option<Vec<u8>>> {
        self.store
            .get_archive(id, Some(&self.config.core.archive_dir))
            .map_err(Into::into)
    }

    fn archive_decision(&self, tool_use_id: &str) -> Result<Option<ArchiveDecision>> {
        self.store
            .archive_decision(&self.session, tool_use_id)
            .map_err(Into::into)
    }

    fn put_archive_decision(
        &self,
        tool_use_id: &str,
        archive_id: &str,
        pointer: &str,
    ) -> Result<()> {
        self.store
            .put_archive_decision(tool_use_id, archive_id, &self.session, pointer)
            .map_err(Into::into)
    }

    fn mark_expanded(&self, archive_id: &str) -> Result<usize> {
        self.store.mark_expanded(archive_id).map_err(Into::into)
    }

    fn archive_size(&self, id: &str) -> Result<Option<u64>> {
        self.store
            .archive_size(id, Some(&self.config.core.archive_dir))
            .map_err(Into::into)
    }

    fn archive_in_session(
        &self,
        sha256: &str,
        context: Option<&str>,
    ) -> Result<Option<ArchiveHit>> {
        // Errors fail open: the caller prints the body instead of a pointer.
        match self
            .store
            .archive_in_session(&self.session, sha256, context)
        {
            Ok(Some((id, turns))) => Ok(Some(ArchiveHit { id, turns })),
            _ => Ok(None),
        }
    }

    fn archive_summary(&self, archive_id: &str, version: &str) -> Result<Option<String>> {
        self.store
            .archive_summary(archive_id, version)
            .map_err(Into::into)
    }

    fn session_live_archives(&self, session: &str) -> Result<Vec<(String, String, i64)>> {
        self.store
            .session_live_archives(session)
            .map_err(Into::into)
    }

    fn put_archive_for(&self, body: &[u8], context: Option<&str>) -> Result<String> {
        let id = self.store.put_archive_for(
            &self.session,
            body,
            &self.config.core.archive_dir,
            context,
        )?;
        if let Some(scope) = &self.archive_scope {
            scope
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(id.clone());
        }
        Ok(id)
    }
}

impl Notes for Runtime {
    fn upsert_note(
        &self,
        project: Option<&str>,
        kind: &str,
        title: &str,
        body: &str,
    ) -> Result<i32> {
        let (id, _) = self.store.upsert_note(project, kind, title, body)?;
        let embed = crate::store::embed_settings(&self.config.plugins.memory.embed);
        self.store.upsert_note_embedding(id, title, body, &embed)?;
        Ok(id)
    }

    fn insert_note(
        &self,
        project: Option<&str>,
        kind: &str,
        title: &str,
        body: &str,
    ) -> Result<i32> {
        // T209: `notes_topic` is one row per (project, kind, title), so plugins must not
        // see a hard error on a repeat key — delegate to the same upsert `upsert_note`
        // uses (newest body wins), per the SDK's `Notes::insert_note` contract.
        self.upsert_note(project, kind, title, body)
    }

    fn latest_note(&self, kind: &str) -> Result<Option<String>> {
        self.store.latest_note(kind).map_err(Into::into)
    }

    fn latest_note_for_project(
        &self,
        project: Option<&str>,
        kind_prefix: &str,
    ) -> Result<Option<String>> {
        self.store
            .latest_note_for_project(project, kind_prefix)
            .map_err(Into::into)
    }

    fn latest_session_note(&self, project: Option<&str>) -> Result<Option<String>> {
        self.store.latest_session_note(project).map_err(Into::into)
    }

    fn list_note_titles(&self, project: Option<&str>, limit: u32) -> Result<Vec<(i32, String)>> {
        self.store
            .list_note_titles(project, limit)
            .map_err(Into::into)
    }

    fn get_note_body(&self, id: i32) -> Result<Option<String>> {
        self.store.get_note_body(id).map_err(Into::into)
    }

    fn search_notes(&self, query: &str, limit: u32) -> Result<Vec<NoteHit>> {
        self.store.search_notes(query, limit).map_err(Into::into)
    }

    fn search_notes_hybrid_stored(&self, query: &str, limit: u32) -> Result<Vec<NoteHit>> {
        let embed = crate::store::embed_settings(&self.config.plugins.memory.embed);
        self.store
            .search_notes_hybrid_stored(query, limit, &embed)
            .map_err(Into::into)
    }

    fn set_note_files(&self, id: i32, paths: &[String]) -> Result<()> {
        self.store.set_note_files(id, paths).map_err(Into::into)
    }

    fn notes_for_files(
        &self,
        project: Option<&str>,
        paths: &[String],
        limit: u32,
    ) -> Result<Vec<NoteHit>> {
        self.store
            .notes_for_files(project, paths, limit)
            .map_err(Into::into)
    }

    fn touch_note(&self, id: i32) -> Result<()> {
        self.store.touch_note(id).map_err(Into::into)
    }

    fn insert_observation(&self, obs: &NewObservation<'_>) -> Result<Option<i32>> {
        self.store
            .insert_observation_once(obs, self.once.as_deref())
            .map_err(Into::into)
    }

    fn search_observations(
        &self,
        project: Option<&str>,
        query: &str,
        limit: u32,
    ) -> Result<Vec<ObsHit>> {
        self.store
            .search_observations(project, query, limit)
            .map_err(Into::into)
    }

    fn observations_for_files(
        &self,
        project: Option<&str>,
        paths: &[String],
        limit: u32,
    ) -> Result<Vec<ObsHit>> {
        self.store
            .observations_for_files(project, paths, limit)
            .map_err(Into::into)
    }

    fn recent_observations(&self, session_id: &str, limit: u32) -> Result<Vec<ObsHit>> {
        self.store
            .recent_observations(session_id, limit)
            .map_err(Into::into)
    }

    fn observation_narrative(&self, id: i32) -> Result<Option<String>> {
        self.store.observation_narrative(id).map_err(Into::into)
    }
}

impl ReadCache for Runtime {
    fn put_read_cache(&self, path: &str, sha256: &str, archive_id: Option<&str>) -> Result<()> {
        self.store
            .put_read_cache(&self.session, path, sha256, archive_id)
            .map_err(Into::into)
    }

    fn get_read_cache(&self, path: &str) -> Result<Option<(Option<String>, i64)>> {
        self.store
            .get_read_cache(&self.session, path)
            .map_err(Into::into)
    }

    fn clear_read_cache(&self, path: &str) -> Result<()> {
        self.store
            .clear_read_cache(&self.session, path)
            .map_err(Into::into)
    }

    fn read_cache_keys(&self) -> Result<Vec<String>> {
        self.store
            .read_cache_keys(&self.session)
            .map_err(Into::into)
    }
}

impl Ledger for Runtime {
    fn last_measurement_ref(&self, plugin: &str, kind: &str) -> Result<Option<String>> {
        self.store
            .last_measurement_ref(&self.session, plugin, kind)
            .map_err(Into::into)
    }

    fn recent_hook_inputs(&self, limit: i64) -> Result<Vec<String>> {
        self.store
            .recent_hook_inputs(&self.session, limit)
            .map_err(Into::into)
    }

    fn recent_hook_inputs_for_event(&self, event: &str, limit: i64) -> Result<Vec<String>> {
        self.store
            .recent_hook_inputs_for_event(&self.session, event, limit)
            .map_err(Into::into)
    }

    fn calls_since(&self, ts: i64) -> Result<i64> {
        self.store
            .calls_since(&self.session, ts)
            .map_err(Into::into)
    }
}

impl Symbols for Runtime {
    fn symbol_count(&self, root: &str) -> Result<i64> {
        self.store.symbol_count(root).map_err(Into::into)
    }

    fn symbol_stat(&self, root: &str, path: &str) -> Result<Option<(String, i64, i64)>> {
        self.store.symbol_stat(root, path).map_err(Into::into)
    }

    fn symbol_stats(
        &self,
        root: &str,
    ) -> Result<std::collections::HashMap<String, (String, i64, i64)>> {
        self.store.symbol_stats(root).map_err(Into::into)
    }

    fn touch_symbols(&self, root: &str, path: &str, mtime: i64, size: i64) -> Result<()> {
        self.store
            .touch_symbols(root, path, mtime, size)
            .map_err(Into::into)
    }

    fn replace_symbols(
        &self,
        root: &str,
        path: &str,
        file_sha: &str,
        stat: (i64, i64),
        rows: &[rtok_plugin_sdk::SymbolRow],
    ) -> Result<usize> {
        self.store
            .replace_symbols(root, path, file_sha, stat, rows)
            .map_err(Into::into)
    }

    fn replace_symbol_files(
        &self,
        root: &str,
        files: &rtok_plugin_sdk::SymbolFileBatch,
    ) -> Result<usize> {
        self.store
            .replace_symbol_files(root, files)
            .map_err(Into::into)
    }

    fn delete_symbols_missing(&self, root: &str, keep: &HashSet<String>) -> Result<usize> {
        self.store
            .delete_symbols_missing(root, keep)
            .map_err(Into::into)
    }

    fn mark_symbols_stale(&self, abs_path: &str) -> Result<()> {
        self.store.mark_symbols_stale(abs_path).map_err(Into::into)
    }

    fn extractor_fingerprint(&self, root: &str) -> Result<Option<String>> {
        self.store.extractor_fingerprint(root).map_err(Into::into)
    }

    fn set_extractor_fingerprint(&self, root: &str, fp: &str) -> Result<()> {
        self.store
            .set_extractor_fingerprint(root, fp)
            .map_err(Into::into)
    }

    fn symbol_defs(&self, root: &str, name: &str) -> Result<Vec<(String, String, i32, i32)>> {
        self.store.symbol_defs(root, name).map_err(Into::into)
    }

    fn symbol_file_defs(&self, root: &str, path: &str) -> Result<Vec<(String, String, i32, i32)>> {
        self.store.symbol_file_defs(root, path).map_err(Into::into)
    }

    fn symbol_ref_groups(&self, root: &str, name: &str) -> Result<Vec<(String, String, i64, i32)>> {
        self.store.symbol_ref_groups(root, name).map_err(Into::into)
    }

    fn symbol_callees(&self, root: &str, name: &str) -> Result<Vec<(String, i32, String, i32)>> {
        self.store.symbol_callees(root, name).map_err(Into::into)
    }

    fn symbol_impact(
        &self,
        root: &str,
        name: &str,
        depth: u32,
    ) -> Result<Vec<(u32, String, String)>> {
        self.store
            .symbol_impact(root, name, depth)
            .map_err(Into::into)
    }

    fn symbol_dead_candidates(&self, root: &str) -> Result<Vec<(String, String, String, i32)>> {
        self.store.symbol_dead_candidates(root).map_err(Into::into)
    }

    fn symbol_referenced_names(&self, root: &str, names: &[String]) -> Result<HashSet<String>> {
        self.store
            .symbol_referenced_names(root, names)
            .map_err(Into::into)
    }

    fn symbol_name_prefix(&self, root: &str, prefix: &str, limit: i64) -> Result<Vec<String>> {
        self.store
            .symbol_name_prefix(root, prefix, limit)
            .map_err(Into::into)
    }

    fn symbol_paths(&self, root: &str, from: &str, to: &str, depth: u32) -> Result<Vec<String>> {
        self.store
            .symbol_paths(root, from, to, depth)
            .map_err(Into::into)
    }

    fn symbol_file_count(&self, root: &str) -> Result<i64> {
        self.store.symbol_file_count(root).map_err(Into::into)
    }

    fn symbol_pending(&self, root: &str, root_path: &std::path::Path) -> Result<Vec<String>> {
        self.store
            .symbol_pending(root, root_path)
            .map_err(Into::into)
    }

    fn symbol_indexed_at(&self, root: &str) -> Result<Option<i64>> {
        self.store.symbol_indexed_at(root).map_err(Into::into)
    }

    fn touch_symbol_indexed_at(&self, root: &str, ts: i64) -> Result<()> {
        self.store
            .touch_symbol_indexed_at(root, ts)
            .map_err(Into::into)
    }

    fn symbol_imports(&self, root: &str, path: &str) -> Result<Vec<(String, i32)>> {
        self.store.symbol_imports(root, path).map_err(Into::into)
    }

    fn symbol_imported_defs(&self, root: &str, name: &str) -> Result<Vec<(String, String)>> {
        self.store
            .symbol_imported_defs(root, name)
            .map_err(Into::into)
    }

    fn symbol_name_freq(&self, root: &str, name: &str) -> Result<(i64, i64)> {
        self.store.symbol_name_freq(root, name).map_err(Into::into)
    }

    fn rebuild_symbol_idf(&self, root: &str) -> Result<()> {
        self.store.rebuild_symbol_idf(root).map_err(Into::into)
    }

    fn symbol_importers(&self, root: &str, module: &str) -> Result<Vec<(String, i32)>> {
        self.store
            .symbol_importers(root, module)
            .map_err(Into::into)
    }

    fn symbol_import_follow(&self, root: &str, name: &str) -> Result<Vec<(String, String)>> {
        self.store
            .symbol_import_follow(root, name)
            .map_err(Into::into)
    }

    fn symbol_top_refs(&self, root: &str, limit: i64) -> Result<Vec<(String, i64, String, i32)>> {
        self.store.symbol_top_refs(root, limit).map_err(Into::into)
    }

    fn symbol_file_scan(&self, root: &str) -> Result<Vec<(String, String, bool, i64)>> {
        self.store.symbol_file_scan(root).map_err(Into::into)
    }

    fn file_rank_get(&self, root: &str) -> Result<Option<String>> {
        self.store.file_rank_get(root).map_err(Into::into)
    }

    fn file_rank_put(&self, root: &str, graph: &str) -> Result<()> {
        self.store.file_rank_put(root, graph).map_err(Into::into)
    }

    fn symbol_span(
        &self,
        root: &str,
        path: &str,
        name: &str,
        kind: &str,
        line: i32,
    ) -> Result<Option<rtok_plugin_sdk::SymbolSpan>> {
        self.store
            .symbol_span(root, path, name, kind, line)
            .map_err(Into::into)
    }

    fn symbol_fts(
        &self,
        root: &str,
        query: &str,
        limit: i64,
    ) -> Result<Vec<(String, String, String, i32)>> {
        self.store
            .symbol_fts(root, query, limit)
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One contract, not a copy: `rtok::plugin::*` re-exports the published crate (D25), so
    /// these assignments only compile while both paths name the same type.
    #[test]
    fn re_exports_are_the_sdk_types() {
        let m = rtok_plugin_sdk::Manifest {
            id: "ext",
            surfaces: &[Surface::Mcp],
            default_on: true,
        };
        let m: Manifest = m;
        assert_eq!(m.id, "ext");
        let page: DashboardPage = rtok_plugin_sdk::DashboardPage::new("Ext", "external.", false);
        assert_eq!(page.title, "Ext");
    }

    /// D25's whole claim in one assignment: an out-of-tree plugin implements
    /// `rtok_plugin_sdk::Plugin` and the host accepts it, because there is only one trait.
    #[test]
    fn the_trait_is_the_published_one() {
        struct Ext;
        impl rtok_plugin_sdk::Plugin for Ext {
            fn manifest(&self) -> Manifest {
                Manifest {
                    id: "ext",
                    surfaces: &[Surface::Mcp],
                    default_on: false,
                }
            }
            fn dashboard_page(&self) -> DashboardPage {
                DashboardPage::new("Ext", "out of tree.", false)
            }
        }
        let p: &dyn Plugin = &Ext;
        assert_eq!(p.manifest().id, "ext");
    }

    /// T154: a hook run from a linked worktree attributes its session to the main
    /// repository, and keeps the worktree as its `cwd` for the ownership ledger.
    #[test]
    fn a_session_in_a_linked_worktree_is_attributed_to_the_main_repository() {
        let dir = crate::testutil::tmp_dir("t154-project");
        let (_main, wt) = crate::testutil::worktree_layout(&dir);
        let mut cx = Runtime::in_memory("t154").unwrap();
        cx.cwd = Some(wt.to_string_lossy().into_owned());
        cx.record_call("hook", "hook", Some("SessionStart"))
            .unwrap();
        let (_, project, cwd) = cx.store.session_row("t154").unwrap().unwrap();
        assert_eq!(project.as_deref(), Some("repo"));
        assert_eq!(cwd, cx.cwd);
        let seen = cx.store.sessions_by_cwd().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            (seen[0].id.as_str(), &seen[0].cwd),
            ("t154", cwd.as_ref().unwrap())
        );
        assert!(seen[0].last_seen > 0 && seen[0].ended_at.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T644: every call made a dir and none was removed, so 24,000 piled up in `$TMPDIR`.
    #[test]
    fn a_dropped_in_memory_runtime_leaves_no_directory() {
        let cx = Runtime::in_memory("t644").unwrap();
        let dir = cx.config.log.path.parent().unwrap().to_path_buf();
        assert!(dir.is_dir());
        drop(cx);
        assert!(!dir.exists(), "{}", dir.display());
    }

    #[test]
    fn log_survives_db_failure() {
        let mut cx = Runtime::in_memory("s").unwrap();
        cx.config.log.path =
            std::env::temp_dir().join(format!("rtok-log-db-fail-{}.log", std::process::id()));
        cx.store.set_query_only().unwrap();
        cx.log("error", "plugin", "read", "boom");
        assert!(cx.config.log.path.exists(), "file line written, row not");
        let _ = std::fs::remove_file(&cx.config.log.path);
    }

    /// T127: `identical_result` must not hand a sub-agent a pointer to bytes only a
    /// different context in the same session archived — that context never saw the body,
    /// so `expand` would answer for something it never received. The writer's own context
    /// still dedups its own repeat.
    #[test]
    fn identical_result_is_scoped_to_the_writer_context() {
        let cx = Runtime::in_memory("t127").unwrap();
        // Long enough to stay past the pointer message's own length (it embeds two
        // sha256 ids), so a same-context hit actually returns a pointer.
        let body = "repeated tool output ".repeat(50);
        let body = body.as_bytes();
        cx.put_archive_for(body, Some("agent-a")).unwrap();
        assert!(
            identical_result(&cx, "cmd", body, Some("agent-b")).is_none(),
            "a different sub-agent never saw agent-a's body"
        );
        assert!(
            identical_result(&cx, "cmd", body, None).is_none(),
            "the main window never saw agent-a's body either"
        );
        assert!(
            identical_result(&cx, "cmd", body, Some("agent-a")).is_some(),
            "agent-a's own repeat still dedups"
        );
    }
}
