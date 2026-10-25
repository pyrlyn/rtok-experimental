//! What a plugin may ask the host to do.
//!
//! A plugin never sees the host's database. It sees [`Host`] — estimate, record, log, read my
//! own configuration — plus one narrow trait per area of stored state it actually uses:
//! [`Archive`], [`Notes`], [`ReadCache`], [`Ledger`], [`Symbols`]. That is the whole reason
//! this crate can be three dependencies deep while `rtok` carries SQLite, tree-sitter and a
//! web server.
//!
//! Nothing here is a general database API. A method exists because a plugin calls it; the
//! host is free to store the state any way it likes, and two of these traits (`Symbols`,
//! `ReadCache`) are caches whose contents may vanish between calls.
//!
//! Every method takes `&self` and may be called from any thread. Errors are returned, never
//! panicked: a plugin that cannot read the store still has to fail open.

use std::collections::HashSet;

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::Measurement;

/// What kind of text is being measured. The host keeps a chars-per-token rate per class, so
/// the estimate stays within about ±15 % without a tokenizer and without a network call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Source code, shell output, diffs.
    Code,
    /// Natural-language prose, markdown.
    Prose,
    /// JSON / structured data (punctuation-heavy → fewer chars per token).
    Json,
    /// CJK scripts: roughly one token per character.
    Cjk,
}

/// One full-text search hit from [`Notes::search_notes`].
#[derive(Clone, Debug)]
pub struct NoteHit {
    /// Note id; pass it to [`Notes::get_note_body`] for the whole note.
    pub id: i32,
    /// The note's title.
    pub title: String,
    /// A short excerpt around the match — capped by the host, never the whole body.
    pub snippet: String,
}

/// One mechanical observation to store (T454). One value so the host method
/// stays under the argument cap.
pub struct NewObservation<'a> {
    /// Host session that produced it.
    pub session_id: &'a str,
    /// Project key, when the session has one.
    pub project: Option<&'a str>,
    /// `file_read`, `command_run`, `error`, …
    pub obs_type: &'a str,
    /// Tool name, capped.
    pub title: &'a str,
    /// Scrubbed narrative, capped.
    pub narrative: &'a str,
    /// sha256 of session, tool and narrative.
    pub dedup: &'a str,
    /// Existing root-relative files the call named.
    pub files: &'a [String],
}

/// One mechanical observation (T454): a scrubbed tool-call summary, not the raw output.
#[derive(Clone, Debug)]
pub struct ObsHit {
    /// Observation id; pass it to [`Notes::observation_narrative`].
    pub id: i32,
    /// Tool name, capped.
    pub title: String,
    /// Host session that produced it.
    pub session_id: String,
    /// A short excerpt of the narrative.
    pub snippet: String,
}

/// The frozen decision for one archived tool result, from [`Archive::archive_decision`].
///
/// A tool result is shortened once. Every later turn replays the same pointer text, because
/// context that changes byte-for-byte between turns costs a cache miss on every one of them.
#[derive(Clone, Debug)]
pub struct ArchiveDecision {
    /// Handle for [`Archive::get_archive`] and for `rtok expand <id>`.
    pub archive_id: String,
    /// The exact replacement text shown in place of the result.
    pub pointer: String,
    /// Whether the user has already expanded this one.
    pub expanded: bool,
}

/// A same-session archive row whose sha256 matched a later payload (T65.1).
#[derive(Clone, Debug)]
pub struct ArchiveHit {
    /// Handle for [`Archive::get_archive`] and for `rtok expand <id>`.
    pub id: String,
    /// Measurement rows in this session after the original archive; 0 if none yet.
    pub turns: u64,
}

/// The host, as a plugin sees it.
///
/// This is the part every plugin gets: estimate what text will cost, record what you saved,
/// say what happened, and read your own configuration section. Everything else is a
/// capability trait.
pub trait Host: Send + Sync {
    /// The host session this dispatch belongs to. Every row a plugin writes is attributed
    /// to it; a plugin rarely needs it directly.
    fn session(&self) -> &str;

    /// Working directory of this dispatch, when the surface knows it.
    /// Hook events carry one; MCP and the CLI typically do not.
    fn cwd(&self) -> Option<&str> {
        None
    }

    /// Estimated token count for `text`. No tokenizer, no network — safe on the hot path.
    fn estimate(&self, text: &str, class: Class) -> u32;

    /// Persist a [`Measurement`]. This is the only path a saving has into the host's
    /// numbers: what is not recorded here did not happen.
    fn record(&self, m: &Measurement) -> Result<()>;

    /// Keep one small value about the plugin's own work under `key` in `plugin`'s namespace;
    /// a repeat key replaces it. For signals that are not savings (T419: checkpoint prompt
    /// counts) and so must never reach [`Host::record`]'s numbers. Default `Ok(())`: a host
    /// without a store drops the value and the plugin still runs.
    fn plugin_state_set(&self, _plugin: &str, _key: &str, _value: &str) -> Result<()> {
        Ok(())
    }

    /// The value [`Host::plugin_state_set`] kept under `key`, if any. Default `None`.
    fn plugin_state_get(&self, _plugin: &str, _key: &str) -> Result<Option<String>> {
        Ok(None)
    }

    /// Open a `calls` row for one unit of work on `surface`, returning its id.
    fn record_call(&self, surface: &str, kind: &str, name: Option<&str>) -> Result<i32>;

    /// Open a `plugin_run` row for `plugin`, nested under the current call when there is one.
    fn record_plugin_run(&self, surface: &str, plugin: &str) -> Result<i32>;

    /// Attribute `tokens` to a call — the before/after phases of a plugin's own work.
    fn record_tokens(
        &self,
        call_id: i32,
        plugin: Option<&str>,
        phase: &str,
        source: &str,
        tokens: i64,
    ) -> Result<()>;

    /// Append a log line. Never fails: a plugin cannot be made to care where logs go.
    fn log(&self, level: &str, source: &str, name: &str, message: &str);

    /// One section of the host's configuration as JSON, by dotted path — `"plugins.read"`,
    /// `"proxy"`, `"estimator"` — or [`Value::Null`] when the host has no such section.
    ///
    /// A plugin reads its own settings through [`Ctx::plugin_config`]; this is the way out
    /// for the few that legitimately need a host-wide value, such as the proxy's mode.
    fn config_json(&self, path: &str) -> Value;

    /// The `calls` row this dispatch runs under, when the surface has one.
    fn call_id(&self) -> Option<i32> {
        None
    }

    /// Watcher debounce window of `root`: relative paths not yet re-indexed (T68.3). One set per
    /// root since T329.5, when a scope's projects are watched side by side.
    fn graph_watch_pending(&self, root: &str) -> Vec<String> {
        let _ = root;
        Vec::new()
    }

    /// Publish `root`'s in-flight pending set for staleness banners (T68.3).
    fn publish_graph_watch_pending(&self, root: &str, paths: &[String]) {
        let _ = (root, paths);
    }
}

/// Everything a plugin may touch, in one object-safe bound. [`Ctx`] derefs to it, so a
/// plugin calls `cx.estimate(..)`, `cx.put_archive(..)`, `cx.symbol_defs(..)` and the rest
/// without naming a single trait.
pub trait Capabilities: Host + Archive + Notes + ReadCache + Ledger + Symbols {}

impl<T> Capabilities for T where T: Host + Archive + Notes + ReadCache + Ledger + Symbols + ?Sized {}

/// What a plugin is handed on every event: the host, and nothing else.
///
/// `Ctx` is a borrow, not a state — the host builds one per dispatch. It derefs to
/// [`Capabilities`], so every host method is reachable directly on it.
pub struct Ctx<'a> {
    host: &'a dyn Capabilities,
    agent_id: Option<&'a str>,
}

impl<'a> Ctx<'a> {
    /// Wrap a host for one dispatch.
    pub fn new(host: &'a dyn Capabilities) -> Self {
        Self {
            host,
            agent_id: None,
        }
    }

    /// T129: wrap a host for one dispatch inside a sub-agent's context window. A window is
    /// `(session_id, agent_id)`: hooks fired in a sub-agent carry `agent_id` on the payload,
    /// and "already seen" state must not cross windows.
    pub fn with_agent(host: &'a dyn Capabilities, agent_id: Option<&'a str>) -> Self {
        Self { host, agent_id }
    }

    /// The sub-agent this dispatch belongs to, or `None` for the parent window.
    pub fn agent_id(&self) -> Option<&str> {
        self.agent_id
    }

    /// This plugin's own `[plugins.<id>]` section, deserialized into `T`.
    ///
    /// Configuration is data, not a schema the host has to know: a plugin declares whatever
    /// struct it wants and the host hands over the matching section. A missing or unparsable
    /// section yields `T::default()` — a plugin that cannot read its own settings still runs.
    pub fn plugin_config<T: DeserializeOwned + Default>(&self, id: &str) -> T {
        section(self.host, &format!("plugins.{id}"))
    }

    /// A host-wide section by dotted path, deserialized into `T`; `T::default()` when the
    /// host has no such section. Reach for [`Ctx::plugin_config`] first.
    pub fn config<T: DeserializeOwned + Default>(&self, path: &str) -> T {
        section(self.host, path)
    }
}

/// One configuration section, or `T::default()` when the host has none under that path.
fn section<T: DeserializeOwned + Default>(host: &dyn Host, path: &str) -> T {
    serde_json::from_value(host.config_json(path)).unwrap_or_default()
}

impl<'a> std::ops::Deref for Ctx<'a> {
    type Target = dyn Capabilities + 'a;

    fn deref(&self) -> &(dyn Capabilities + 'a) {
        self.host
    }
}

/// Content-addressed blob storage — the other half of "lossless by default".
///
/// A plugin that shortens something puts the original here first and quotes the returned id,
/// so `rtok expand <id>` can always give it back.
pub trait Archive {
    /// Store `body` and return its handle. Storing the same bytes twice returns the same id.
    fn put_archive(&self, body: &[u8]) -> Result<String>;

    /// [`Archive::put_archive`], tagged with the context window that wrote it (T127): a
    /// sub-agent's `agent_id`, or `None` for the main window — see
    /// [`Archive::archive_in_session`]. Default drops the context and delegates, so a host
    /// that has not opted into context scoping needs no change.
    fn put_archive_for(&self, body: &[u8], _context: Option<&str>) -> Result<String> {
        self.put_archive(body)
    }

    /// The bytes behind a handle, or `None` if the host no longer has them.
    fn get_archive(&self, id: &str) -> Result<Option<Vec<u8>>>;

    /// The payload's size in bytes, or `None` if the host no longer has it. Metadata only:
    /// hot paths (the guard deny) must not read a possibly-megabyte body. The default fails
    /// open (`None`), so hosts that only keep bodies in memory need no change.
    fn archive_size(&self, _id: &str) -> Result<Option<u64>> {
        Ok(None)
    }

    /// The decision already made for `tool_use_id`, if this result was shortened before.
    fn archive_decision(&self, tool_use_id: &str) -> Result<Option<ArchiveDecision>>;

    /// Persist the decision for `tool_use_id`. First writer wins: a pointer that is already
    /// recorded stays exactly as it is, because the model pays a cache miss for every byte
    /// of context that moves between turns.
    fn put_archive_decision(
        &self,
        tool_use_id: &str,
        archive_id: &str,
        pointer: &str,
    ) -> Result<()>;

    /// Mark an archived blob as expanded by the user; returns how many rows changed.
    fn mark_expanded(&self, archive_id: &str) -> Result<usize>;

    /// The archive row for `sha256` written in this session under `context` — a sub-agent's
    /// `agent_id`, or `None` for the main window (T127). A body one context window archived
    /// is a hit only for that same window: a different one never saw those bytes, so a
    /// pointer to them would send the caller to `expand` a body it does not have. Default
    /// `Ok(None)` so a host that cannot look it up fails open (the caller prints the body).
    fn archive_in_session(
        &self,
        _sha256: &str,
        _context: Option<&str>,
    ) -> Result<Option<ArchiveHit>> {
        Ok(None)
    }

    /// The compressor's summary of `archive_id` for compressor `version` (T385.9.1), if one was
    /// stored. Default `Ok(None)`: a host without a summary cache just shows the extractive view.
    fn archive_summary(&self, _archive_id: &str, _version: &str) -> Result<Option<String>> {
        Ok(None)
    }

    /// This session's archived tool results still in the live window, newest first
    /// (T58.2): `(archive_id, tool, bytes)`. Lets `checkpoint::attach_ids` read through the
    /// host's own connection instead of opening a second `Store` (T203). Default
    /// `Ok(Vec::new())` fails open for a host that does not track a live window.
    fn session_live_archives(&self, _session: &str) -> Result<Vec<(String, String, i64)>> {
        Ok(Vec::new())
    }
}

/// Durable notes the host can search — what a plugin remembers between sessions.
pub trait Notes {
    /// Upsert on `(project, kind, title)` and return the note id (T69.5 `remember:` path).
    fn upsert_note(
        &self,
        project: Option<&str>,
        kind: &str,
        title: &str,
        body: &str,
    ) -> Result<i32>;

    /// Save a note and return its id. `project` scopes it; `None` means "not project-bound".
    ///
    /// Same one-row-per-`(project, kind, title)` contract as [`Notes::upsert_note`] (T209:
    /// the `notes_topic` index enforces it in the store either way) — a repeat key updates
    /// the existing row (newest body wins) instead of erroring. Implementations delegate
    /// to `upsert_note`; call that directly when the caller wants to be explicit about
    /// updating.
    fn insert_note(
        &self,
        project: Option<&str>,
        kind: &str,
        title: &str,
        body: &str,
    ) -> Result<i32>;

    /// The body of the most recent note of `kind`.
    fn latest_note(&self, kind: &str) -> Result<Option<String>>;

    /// Newest note body whose kind starts with `kind_prefix` for `project`.
    fn latest_note_for_project(
        &self,
        project: Option<&str>,
        kind_prefix: &str,
    ) -> Result<Option<String>>;

    /// Newest `session:*` note body for `project` (`None` = unbound), id order. Lets
    /// `checkpoint::offer_session` read through the host's own connection instead of
    /// opening a second `Store` (T203). Default `Ok(None)` fails open for a host with no
    /// session notes.
    fn latest_session_note(&self, _project: Option<&str>) -> Result<Option<String>> {
        Ok(None)
    }

    /// The `limit` most recent `(id, title)` pairs, newest first.
    fn list_note_titles(&self, project: Option<&str>, limit: u32) -> Result<Vec<(i32, String)>>;

    /// One note's body by id.
    fn get_note_body(&self, id: i32) -> Result<Option<String>>;

    /// Full-text search over notes. An empty query returns no hits rather than everything.
    fn search_notes(&self, query: &str, limit: u32) -> Result<Vec<NoteHit>>;

    /// FTS fused with vectors already stored (RRF). Does not embed missing rows.
    ///
    /// Default is [`Notes::search_notes`]: a host with no vector table stays on the text list.
    fn search_notes_hybrid_stored(&self, query: &str, limit: u32) -> Result<Vec<NoteHit>> {
        self.search_notes(query, limit)
    }

    /// Replace the files note `id` is linked to (T374); paths are relative to the project root.
    /// Default `Ok(())` fails open for a host that keeps no links.
    fn set_note_files(&self, _id: i32, _paths: &[String]) -> Result<()> {
        Ok(())
    }

    /// Live notes of `project` (and unbound ones; `None` = any) linked to any of `paths` (T374), the
    /// one linked to most of them first. Root-relative paths repeat across projects, so the
    /// project scope keeps another repository's `src/main.rs` note out.
    /// Default `Ok(Vec::new())` fails open for a host that keeps no links.
    fn notes_for_files(
        &self,
        _project: Option<&str>,
        _paths: &[String],
        _limit: u32,
    ) -> Result<Vec<NoteHit>> {
        Ok(Vec::new())
    }

    /// Count one read of note `id` (`uses`, `last_used`). Default no-op.
    fn touch_note(&self, _id: i32) -> Result<()> {
        Ok(())
    }

    /// Insert a mechanical observation. `Ok(None)` means a recent duplicate was skipped.
    /// Default `Ok(None)` for a host that does not store observations.
    fn insert_observation(&self, _obs: &NewObservation<'_>) -> Result<Option<i32>> {
        Ok(None)
    }

    /// FTS over observation narratives. An empty query returns no hits.
    fn search_observations(
        &self,
        _project: Option<&str>,
        _query: &str,
        _limit: u32,
    ) -> Result<Vec<ObsHit>> {
        Ok(Vec::new())
    }

    /// Observations linked to any of `paths`, newest first.
    fn observations_for_files(
        &self,
        _project: Option<&str>,
        _paths: &[String],
        _limit: u32,
    ) -> Result<Vec<ObsHit>> {
        Ok(Vec::new())
    }

    /// Newest observations in one session.
    fn recent_observations(&self, _session_id: &str, _limit: u32) -> Result<Vec<ObsHit>> {
        Ok(Vec::new())
    }

    /// The scrubbed narrative of observation `id`.
    fn observation_narrative(&self, _id: i32) -> Result<Option<String>> {
        Ok(None)
    }
}

/// Per-session memory of what has already been read, so the same file is not sent twice.
///
/// This is a cache: entries may be evicted, and every method is scoped to the current
/// session by the host.
pub trait ReadCache {
    /// Remember that `path` was read, by content hash, optionally with the archived body.
    fn put_read_cache(&self, path: &str, sha256: &str, archive_id: Option<&str>) -> Result<()>;

    /// The `(archive_id, timestamp)` remembered for `path`, if any.
    fn get_read_cache(&self, path: &str) -> Result<Option<(Option<String>, i64)>>;

    /// Forget `path` and anything stored under it — call this when the file changes.
    fn clear_read_cache(&self, path: &str) -> Result<()>;

    /// The keys of everything remembered this session (T374), so a caller can tell which files
    /// were read. A key is a path or a path followed by `\t` and a mode; a host that cannot list
    /// them returns none.
    fn read_cache_keys(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// A compaction dropped what the context window `context` (a sub-agent's `agent_id`, or
    /// `None` for the main one) had been shown (T929): forget the session's re-read cache and
    /// stop answering with pointers to bodies that window held. The bodies stay retrievable.
    /// Default: nothing to forget.
    fn forget_seen(&self, _context: Option<&str>) -> Result<()> {
        Ok(())
    }

    /// True when more than one context window may share this session, i.e. a sub-agent has
    /// started (T929). A surface that cannot tell which window is asking (MCP) then answers
    /// with the content, never a pointer to what another window was shown. Default: no.
    fn shared_session(&self) -> bool {
        false
    }
}

/// A read-only look back at this session's own traffic, for plugins that need a window
/// rather than a single event.
pub trait Ledger {
    /// The raw JSON of the last `limit` hook inputs in this session, newest first.
    fn recent_hook_inputs(&self, limit: i64) -> Result<Vec<String>>;

    /// The raw JSON of the last `limit` hook inputs whose `hook_event_name` is `event`,
    /// newest first (T202). A plugin that only cares about e.g. `PostToolUse` rows should
    /// call this instead of `recent_hook_inputs` plus its own filter loop, so a host backed
    /// by a real store can push the filter into the query instead of fetching and parsing
    /// every other event type first.
    ///
    /// Default: filters an unfiltered `limit`-sized window in memory. Correct but not the
    /// optimization — a host with a queryable store should override it.
    fn recent_hook_inputs_for_event(&self, event: &str, limit: i64) -> Result<Vec<String>> {
        Ok(self
            .recent_hook_inputs(limit)?
            .into_iter()
            .filter(|body| {
                serde_json::from_str::<Value>(body)
                    .ok()
                    .and_then(|v| v.get("hook_event_name")?.as_str().map(str::to_owned))
                    .as_deref()
                    == Some(event)
            })
            .collect())
    }

    /// How many calls this session has made since the unix timestamp `ts`.
    fn calls_since(&self, ts: i64) -> Result<i64>;

    /// The newest `ref_id` on a measurement row for this session (T69.5 dedup).
    fn last_measurement_ref(&self, plugin: &str, kind: &str) -> Result<Option<String>>;
}

/// One indexed symbol.
///
/// The first six fields are the graph edge. The rest is the byte span and the text
/// the definition search index stores.
#[derive(Debug, Clone)]
pub struct SymbolRow {
    /// Identifier as the tags query captured it.
    pub name: String,
    /// Tags syntax type (`function`, `struct`, `import`, …).
    pub kind: String,
    /// 1-based line of the hit.
    pub line: i32,
    /// True when the hit is a definition.
    pub is_def: bool,
    /// 1-based last line of the tagged node.
    pub end_line: i32,
    /// Enclosing definition, or the full import specifier on an import row.
    pub scope: String,
    /// First byte of the hit in the file.
    pub start_byte: i64,
    /// One past the last byte of the hit.
    pub end_byte: i64,
    /// Sha256 of `file[start_byte..end_byte]`. Empty when the span was not captured.
    pub content_hash: String,
    /// The definition's source line. Empty on a reference.
    pub signature: String,
    /// Contiguous `///` or `//!` lines directly above a definition, at most 512 bytes.
    pub doc: String,
}

impl SymbolRow {
    /// A graph edge with no byte span. Tests and hand-built rows use this.
    pub fn new(
        name: impl Into<String>,
        kind: impl Into<String>,
        line: i32,
        is_def: bool,
        end_line: i32,
        scope: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            kind: kind.into(),
            line,
            is_def,
            end_line,
            scope: scope.into(),
            start_byte: 0,
            end_byte: 0,
            content_hash: String::new(),
            signature: String::new(),
            doc: String::new(),
        }
    }
}

/// Byte range of one indexed definition, and the hashes that say the file still matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolSpan {
    /// First byte of the definition in the file.
    pub start_byte: u64,
    /// One past the last byte of the definition.
    pub end_byte: u64,
    /// Sha256 of `file[start_byte..end_byte]`.
    pub content_hash: String,
    /// Sha256 of the whole file at index time.
    pub file_sha: String,
}

/// Symbol rows for one indexed file.
pub type SymbolFileRows = Vec<SymbolRow>;
/// Batched cold-index writes: `(path, sha, stat, rows)` per file.
pub type SymbolFileBatch = Vec<(String, String, (i64, i64), SymbolFileRows)>;

/// The symbol index behind `symbol` / `callers` / `impact`: definitions and references
/// extracted from source, keyed by repository root and relative path.
///
/// This is a cache the host rebuilds from the files themselves, so a stale or empty index is
/// a correctness-neutral miss, never an error to propagate.
pub trait Symbols {
    /// How many symbols are indexed under `root`. Zero means "not indexed yet".
    fn symbol_count(&self, root: &str) -> Result<i64>;

    /// The recorded `(sha, mtime, size)` for one file, if it has been indexed.
    fn symbol_stat(&self, root: &str, path: &str) -> Result<Option<(String, i64, i64)>>;

    /// Every indexed file's `(sha, mtime, size)` under `root` (T35.3).
    fn symbol_stats(
        &self,
        root: &str,
    ) -> Result<std::collections::HashMap<String, (String, i64, i64)>>;

    /// Update a file's mtime and size without re-parsing it — the content is unchanged.
    fn touch_symbols(&self, root: &str, path: &str, mtime: i64, size: i64) -> Result<()>;

    /// Replace every symbol of one file in a single transaction. Returns how many were written.
    fn replace_symbols(
        &self,
        root: &str,
        path: &str,
        file_sha: &str,
        stat: (i64, i64),
        rows: &[SymbolRow],
    ) -> Result<usize>;

    /// Replace many files in one transaction (T35.3 cold index).
    fn replace_symbol_files(&self, root: &str, files: &SymbolFileBatch) -> Result<usize>;

    /// Drop every indexed file under `root` that is not in `keep`; returns how many went.
    fn delete_symbols_missing(&self, root: &str, keep: &HashSet<String>) -> Result<usize>;

    /// Drop what was indexed for one absolute path — the file changed under the index.
    fn mark_symbols_stale(&self, abs_path: &str) -> Result<()>;

    /// Distinct indexed files under `root` (T68.3 `graph status`).
    fn symbol_file_count(&self, root: &str) -> Result<i64> {
        let _ = root;
        Ok(0)
    }

    /// Pending re-index paths for `root` (T68.3).
    fn symbol_pending(&self, root: &str, root_path: &std::path::Path) -> Result<Vec<String>> {
        let _ = (root, root_path);
        Ok(Vec::new())
    }

    /// Unix seconds of the last successful index for `root` (T68.3).
    fn symbol_indexed_at(&self, root: &str) -> Result<Option<i64>> {
        let _ = root;
        Ok(None)
    }

    /// Record the last successful index time for `root` (T68.3).
    fn touch_symbol_indexed_at(&self, root: &str, ts: i64) -> Result<()> {
        let _ = (root, ts);
        Ok(())
    }

    /// Extractor fingerprint stored for `root`, if any (T35.5).
    fn extractor_fingerprint(&self, root: &str) -> Result<Option<String>> {
        let _ = root;
        Ok(None)
    }

    /// Record the extractor fingerprint for `root` after a cold index (T35.5).
    fn set_extractor_fingerprint(&self, root: &str, fp: &str) -> Result<()> {
        let _ = (root, fp);
        Ok(())
    }

    /// Definitions of `name`: `(path, kind, line, end_line)`.
    fn symbol_defs(&self, root: &str, name: &str) -> Result<Vec<(String, String, i32, i32)>>;

    /// References to `name` grouped by location: `(path, kind, count, line)`.
    fn symbol_ref_groups(&self, root: &str, name: &str) -> Result<Vec<(String, String, i64, i32)>>;

    /// Reference kinds of `name` per calling definition: `(path, scope, kind)`. The default
    /// is empty, which callers read as "kind unknown".
    fn symbol_ref_kinds(&self, root: &str, name: &str) -> Result<Vec<(String, String, String)>> {
        let _ = (root, name);
        Ok(Vec::new())
    }

    /// Callees per definition of `name`: `(def_path, def_line, callee, first_line)` (T68.2).
    fn symbol_callees(&self, root: &str, name: &str) -> Result<Vec<(String, i32, String, i32)>> {
        let _ = (root, name);
        Ok(Vec::new())
    }

    /// What `name` reaches within `depth` hops: `(depth, path, name)`.
    fn symbol_impact(
        &self,
        root: &str,
        name: &str,
        depth: u32,
    ) -> Result<Vec<(u32, String, String)>>;

    /// Definitions with no same-name reference under `root`: `(path, name, kind, line)`
    /// (T52.4). Name-based, like `callers`: a shared name keeps every same-named
    /// definition live. Callers filter pub, trait impls, tests and macros from these.
    fn symbol_dead_candidates(&self, root: &str) -> Result<Vec<(String, String, String, i32)>> {
        let _ = root;
        Ok(Vec::new())
    }

    /// Which of `names` are referenced (non-import rows) under `root` (T329.5).
    fn symbol_referenced_names(&self, root: &str, names: &[String]) -> Result<HashSet<String>> {
        let _ = (root, names);
        Ok(HashSet::new())
    }

    /// Distinct definition names starting with `prefix`, best `limit` by reference
    /// count, ties by name (T68.1 `explore` prefix resolution).
    fn symbol_name_prefix(&self, root: &str, prefix: &str, limit: i64) -> Result<Vec<String>> {
        let _ = (root, prefix, limit);
        Ok(Vec::new())
    }

    /// Call chains `from → … → to` walked in the caller direction within `depth`
    /// hops, shortest first (T68.1 `explore`; T68.4 `impact --to` reuses it).
    fn symbol_paths(&self, root: &str, from: &str, to: &str, depth: u32) -> Result<Vec<String>> {
        let _ = (root, from, to, depth);
        Ok(Vec::new())
    }

    /// T130.3: definitions in `path` as `(name, kind, line, end_line)`; empty without an index.
    fn symbol_file_defs(&self, root: &str, path: &str) -> Result<Vec<(String, String, i32, i32)>> {
        let _ = (root, path);
        Ok(Vec::new())
    }

    /// T68.6: import rows of `path` as `(name, line)`.
    fn symbol_imports(&self, root: &str, path: &str) -> Result<Vec<(String, i32)>> {
        let _ = (root, path);
        Ok(Vec::new())
    }

    /// T368: `(reference file, definition file)` imported by a referencing file.
    /// Empty when `name` has fewer than two definitions.
    fn symbol_imported_defs(&self, root: &str, name: &str) -> Result<Vec<(String, String)>> {
        let _ = (root, name);
        Ok(Vec::new())
    }

    /// T368: `(df, n_files)`. `(0, 0)` when the host has no index.
    fn symbol_name_freq(&self, root: &str, name: &str) -> Result<(i64, i64)> {
        let _ = (root, name);
        Ok((0, 0))
    }

    /// T368: recompute per-name document frequency after an index pass.
    fn rebuild_symbol_idf(&self, root: &str) -> Result<()> {
        let _ = root;
        Ok(())
    }

    /// T68.6: files that import `module` as `(path, line)`.
    fn symbol_importers(&self, root: &str, module: &str) -> Result<Vec<(String, i32)>> {
        let _ = (root, module);
        Ok(Vec::new())
    }

    /// T68.6: definitions in files that import `name` (the extra impact hop).
    fn symbol_import_follow(&self, root: &str, name: &str) -> Result<Vec<(String, String)>> {
        let _ = (root, name);
        Ok(Vec::new())
    }

    /// T52.3: names ranked by reference count with one def site:
    /// `(name, refs, path, line)`, `ORDER BY refs DESC, name ASC`.
    fn symbol_top_refs(&self, root: &str, limit: i64) -> Result<Vec<(String, i64, String, i32)>> {
        let _ = (root, limit);
        Ok(Vec::new())
    }

    /// T370: one row per `(name, path, is_def)` with how many index rows it has, imports left
    /// out. The graph plugin builds its file graph from this one scan.
    fn symbol_file_scan(&self, root: &str) -> Result<Vec<(String, String, bool, i64)>> {
        let _ = root;
        Ok(Vec::new())
    }

    /// T370: the file-graph document the plugin stored for `root`, if any.
    fn file_rank_get(&self, root: &str) -> Result<Option<String>> {
        let _ = root;
        Ok(None)
    }

    /// T370: replace the file-graph document of `root`.
    fn file_rank_put(&self, root: &str, graph: &str) -> Result<()> {
        let _ = (root, graph);
        Ok(())
    }

    /// Byte span of one definition (T474). `None` when that row is not indexed.
    fn symbol_span(
        &self,
        root: &str,
        path: &str,
        name: &str,
        kind: &str,
        line: i32,
    ) -> Result<Option<SymbolSpan>> {
        let _ = (root, path, name, kind, line);
        Ok(None)
    }

    /// Definitions whose name, signature, or doc matches `query` (T474).
    ///
    /// A hit whose name contains a query token ranks above a signature hit, which
    /// ranks above a doc hit. At most `limit` rows. Empty when nothing matches.
    fn symbol_fts(
        &self,
        root: &str,
        query: &str,
        limit: i64,
    ) -> Result<Vec<(String, String, String, i32)>> {
        let _ = (root, query, limit);
        Ok(Vec::new())
    }
}

// The tests for what a host owes a plugin live beside `testing::MemoryHost`, the host this
// crate ships for exactly that purpose (`src/testing.rs`).
