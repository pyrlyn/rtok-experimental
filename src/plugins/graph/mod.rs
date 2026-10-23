// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `graph` — `symbol` / `callers` / `outline` over a symbol index rtok builds itself
//! with tree-sitter-tags, with capped output (plan P8).
//!
//! Spec: the catalogue in `plan.md` §1 names the tools this replaces; none is a
//! dependency (D6) — the behaviour is re-implemented here.
//!
//! T8.2: three MCP tools. Each call first runs the incremental index over the current
//! directory (unchanged files are skipped by sha256, so a call costs one directory walk),
//! then answers from the `symbols` table. Every response is capped at
//! `plugins.graph.max_tokens`: the head lines that fit, then `N more, expand <id>` with the
//! full text archived. The printed id is the 8-character prefix (T672, T907). A `cap` measurement records capped vs uncapped estimate only when
//! the answer was actually shortened; an unchanged answer writes no row (T181).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use rtok_sys::proc::ProgressRunner;
use serde_json::{Value, json};

use rtok_plugin_sdk::{
    Class, Ctx, DashboardPage, Injection, Manifest, Measurement, Plugin, PostToolUse, SessionStart,
    Surface, ToolDef,
};

pub mod blast;
pub mod capability;
pub mod cochange;
pub mod diff;
pub mod draw;
pub mod drill;
pub mod events;
pub mod export;
pub mod follow;
pub mod health;
pub mod index;
pub mod jobs;
pub mod lsp;
pub mod projects;
pub mod rank;
pub mod resolve;
pub mod review;
pub mod scope;
pub mod status;
pub mod tally;
pub mod text;
pub mod walk;
pub mod watch;

#[cfg(test)]
thread_local! {
    pub(crate) static SYMBOL_SRC_READS: AtomicUsize = const { AtomicUsize::new(0) };
}

pub struct GraphPlugin;

impl Plugin for GraphPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "graph",
            surfaces: &[Surface::Mcp, Surface::Hook],
            default_on: true,
        }
    }

    fn dashboard_page(&self) -> DashboardPage {
        DashboardPage::new(
            "Graph",
            "symbol / callers / impact from a tree-sitter-tags index.",
            true,
        )
    }

    fn post_tool(&self, ev: &PostToolUse, cx: &Ctx) -> Option<String> {
        if ev.tool_name != "Edit" && ev.tool_name != "Write" {
            return None;
        }
        if let Some(p) = ev.tool_input.get("file_path").and_then(|v| v.as_str()) {
            let _ = cx.mark_symbols_stale(&index::canon(Path::new(p)));
        }
        None
    }

    fn session_start(&self, ev: &SessionStart, cx: &Ctx) -> Option<Injection> {
        repo_map(ev, cx)
    }

    fn mcp_tools(&self) -> Vec<ToolDef> {
        vec![
            ToolDef {
                name: "symbol",
                description: "Definitions of a symbol — id, path:line kind, then the body — optional id or names; path or kind narrows a name.",
                input_schema: json!({"type":"object","properties":{"name":{"type":"string"},"names":{"type":"array","items":{"type":"string"}},"id":{"type":"string"},"path":{"type":"string"},"kind":{"type":"string"},"project":{"type":"string"}}}),
            },
            ToolDef {
                name: "callers",
                description: "Which definitions reference a symbol: path, caller, count. Optional path keeps one subtree. No name: unreferenced.",
                input_schema: json!({"type":"object","properties":{"name":{"type":"string"},"path":{"type":"string"},"kind":{"type":"string"},"project":{"type":"string"}}}),
            },
            ToolDef {
                name: "impact",
                description: "What breaks if a symbol changes: callers up to depth, grouped by file and cut at a token budget (all=true: every row). Optional to: chains reaching it. Empty name + path lists affected tests.",
                input_schema: json!({"type":"object","properties":{"name":{"type":"string"},"to":{"type":"string"},"depth":{"type":"integer"},"all":{"type":"boolean"},"path":{"type":"string"},"project":{"type":"string"}}}),
            },
            ToolDef {
                name: "outline",
                description: "Definitions in one file (read mode=map); kind filters.",
                input_schema: json!({"type":"object","properties":{"path":{"type":"string"},"kind":{"type":"string"},"project":{"type":"string"}},"required":["path"]}),
            },
            ToolDef {
                name: "explore",
                description: "Answers a code question: the query's symbols as definitions with bodies, call paths between them, impact counts. Optional path narrows.",
                input_schema: json!({"type":"object","properties":{"query":{"type":"string"},"path":{"type":"string"},"project":{"type":"string"}},"required":["query"]}),
            },
            ToolDef {
                name: "graph_diff",
                description: "Symbols changed between git refs, with callers.",
                input_schema: json!({"type":"object","properties":{"from":{"type":["string","array"],"description":"ref, default HEAD; project:ref for one project; a list for several"},"to":{"type":"string","description":"ref or working (default)"},"project":{"type":"string"}}}),
            },
            ToolDef {
                name: "graph_export",
                description: "The graph as redacted rtok.graph.v1 JSON.",
                input_schema: json!({"type":"object","properties":{"level":{"type":"string","description":"overview (default) or symbols"},"focus":{"type":"string","description":"symbol; with depth"},"depth":{"type":"integer"},"project":{"type":"string"}}}),
            },
        ]
    }
}

/// T52.1: optional narrow-down for `symbol` / `callers` / `impact` (`path`
/// substring, `kind` exact on `symbol`). No new tool: the filters answer
/// "callers of X inside path Y of kind Z" in the one call that transcripts now
/// spend a scoped-search chain on (610 of 615 `ctx_search` calls carry a path;
/// 252 search-to-search refinements in 33 sessions; research.md §2). `callers`
/// and `impact` honour `path` only. `impact` walks the full graph and filters
/// the reported lines, so depth still crosses files outside the filter.
pub struct Filter {
    pub path: String,
    pub kind: String,
    /// T377: `impact` prints every reached row instead of the budgeted, file-grouped answer.
    pub all: bool,
}

impl Filter {
    pub fn none() -> Self {
        Self {
            path: String::new(),
            kind: String::new(),
            all: false,
        }
    }

    pub(crate) fn path_ok(&self, path: &str) -> bool {
        self.path.is_empty() || path.contains(&self.path)
    }

    pub(crate) fn kind_ok(&self, kind: &str) -> bool {
        self.kind.is_empty() || kind == self.kind
    }

    /// ` in <path>` / ` of kind <kind>` suffix for the empty-answer lines; empty
    /// when no filter is set, so unfiltered answers stay byte-exact (T8.9).
    pub(crate) fn scope_note(&self) -> String {
        let mut s = String::new();
        if !self.path.is_empty() {
            s.push_str(&format!(" in {}", self.path));
        }
        if !self.kind.is_empty() {
            s.push_str(&format!(" of kind {}", self.kind));
        }
        s
    }
}

/// Walk or not (T8.15): `auto_index = true` is today's behaviour — every call
/// walks and the stat gate skips unchanged files. `false` indexes a root with no
/// rows once (`index::ensure`) and never walks again; re-indexing is
/// `rtok graph index` or the P8d watcher, and a hook-staled file reads as
/// missing until then.
pub fn index_for(cx: &Ctx, root: &Path) -> Result<index::Report> {
    if cx.plugin_config::<crate::config::Graph>("graph").auto_index {
        index::run(cx, root, false)
    } else {
        index::ensure(cx, root)
    }
}

/// Pending re-index paths: hook marks, stat drift and the watcher queue (T68.3).
pub(crate) fn pending_paths(cx: &Ctx, root: &Path) -> Result<Vec<String>> {
    let key = index::canon(root);
    let mut set: HashSet<String> = cx.symbol_pending(&key, root)?.into_iter().collect();
    for p in cx.graph_watch_pending(&key) {
        set.insert(p);
    }
    let mut out: Vec<String> = set.into_iter().collect();
    out.sort();
    Ok(out)
}

fn stale_banner(cx: &Ctx, root: &Path) -> Result<String> {
    let cfg = cx.plugin_config::<crate::config::Graph>("graph");
    let pending = pending_paths(cx, root)?;
    if pending.is_empty() {
        return Ok(String::new());
    }
    let watch_pending =
        !cx.graph_watch_pending(&index::canon(root)).is_empty() && cfg.watch != "off";
    if !cfg.auto_index || watch_pending {
        let show = pending.len().min(5);
        let listed = pending[..show].join(", ");
        let tail = if pending.len() > 5 { ", …" } else { "" };
        return Ok(format!(
            "stale: {} files pending ({}{})
",
            pending.len(),
            listed,
            tail
        ));
    }
    Ok(String::new())
}

pub(crate) fn with_stale(cx: &Ctx, root: &Path, text: String) -> Result<String> {
    let banner = stale_banner(cx, root)?;
    if banner.is_empty() {
        Ok(text)
    } else {
        Ok(format!("{banner}{text}"))
    }
}

/// T329.9: how one project is answered. `Tags` is the default and never looks at a server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Tags,
    /// Server first, tags when it cannot answer (T376); the answer carries no mode header.
    Lsp,
    /// Per project and language: server first, tags second, and the answer says which spoke.
    Auto,
    /// T329.10: plain text search, no index and no server (`text.rs`).
    Text,
}

/// The `backend` value that governs the project at `root`: its language's
/// `backend_by_language` entry, else `backend`. A capability record is made under this value.
pub(crate) fn backend_name(cx: &Ctx, root: &Path) -> String {
    let cfg = cx.plugin_config::<crate::config::Graph>("graph");
    // Only a config with overrides pays for the language lookup, so the default path stays stat-free.
    (!cfg.backend_by_language.is_empty())
        .then(|| lsp::language_of(root))
        .flatten()
        .and_then(|lang| cfg.backend_by_language.get(lang))
        .unwrap_or(&cfg.backend)
        .clone()
}

impl Mode {
    /// A value the config schema refuses reads as `Tags`, the safe answer.
    fn named(name: &str) -> Self {
        match name {
            "lsp" => Mode::Lsp,
            "auto" => Mode::Auto,
            "text" => Mode::Text,
            _ => Mode::Tags,
        }
    }
}

/// The mode of the project at `root`.
pub(crate) fn mode_of(cx: &Ctx, root: &Path) -> Mode {
    Mode::named(&backend_name(cx, root))
}

/// The text after the line that names the mode (`(lsp)`, `(text)`, `(tags)`, `(tags; lsp: <reason>)`).
pub(crate) fn without_mode_line(text: &str) -> &str {
    if text.starts_with("(lsp)\n") || text.starts_with("(text)\n") || text.starts_with("(tags") {
        text.split_once('\n').map_or(text, |(_, rest)| rest)
    } else {
        text
    }
}

/// What an LSP answer prints when the server found nothing; the answer is then checked
/// against the tags index instead of being trusted.
fn lsp_none_answer(text: &str) -> bool {
    let text = text.trim_start();
    text.is_empty()
        || [
            "no definition of",
            "no references to",
            "nothing reaches",
            "no symbols resolved",
        ]
        .iter()
        .any(|p| text.starts_with(p))
}

/// Not a saving, so before == after, as in `lsp::finish`: `before_bytes` is the milliseconds
/// spent. Fail open: a lost statistic must not turn a good answer into an error.
fn record_mode_row(cx: &Ctx, kind: &'static str, t0: std::time::Instant, out: &str) {
    let est = cx.estimate(out, Class::Code);
    let _ = cx.record(&Measurement {
        plugin: "graph",
        kind,
        before_bytes: t0.elapsed().as_millis() as u64,
        after_bytes: out.len() as u64,
        est_before: est,
        est_after: est,
        ref_id: None,
        call_id: cx.call_id(),
    });
}

fn tags_kind(tool: &str) -> &'static str {
    match tool {
        "symbol" => "tags.symbol",
        "callers" => "tags.callers",
        "outline" => "tags.outline",
        "explore" => "tags.explore",
        _ => "tags.impact",
    }
}

fn text_kind(tool: &str) -> &'static str {
    match tool {
        "symbol" => "text.symbol",
        "callers" => "text.callers",
        "outline" => "text.outline",
        "explore" => "text.explore",
        _ => "text.impact",
    }
}

/// T329.10: the text backend's door, in front of [`lsp_or_tags`]. When `text::applies`, `text`
/// `searched` answers headed `(text)` and records a `text.<tool>` row; otherwise `rest` answers as
/// before. A root the search cannot read is the one error the answer is allowed to be (plan T329
/// 6a, "when no mode works"), so it names itself instead of falling to an index that is not there.
fn text_or(
    cx: &Ctx,
    root: &Path,
    tool: &'static str,
    searched: impl FnOnce() -> Result<String>,
    rest: impl FnOnce() -> Result<String>,
) -> Result<String> {
    if !text::applies(cx, root) {
        return rest();
    }
    let t0 = std::time::Instant::now();
    let body = searched().map_err(|e| anyhow::anyhow!("no graph backend available: {e:#}"))?;
    let out = format!("(text)\n{}", cap(cx, body)?);
    record_mode_row(cx, text_kind(tool), t0, &out);
    Ok(out)
}

/// T376, T329.9, T329.11: the one door the five tools take. `Lsp` and `Auto` ask the project's
/// capability record (`capability`) whether the language server answers: it is checked on the
/// first request only, and a server that is missing, not ready or dead gives the tags answer
/// headed `(tags; lsp: <reason>)`, so the caller sees which backend spoke and why. So does one
/// that answers "nothing" for a name the tags index knows. `Auto` also heads an LSP answer
/// `(lsp)`, goes straight to tags (`(tags)`) for a language with no server, and records a
/// `tags.<tool>` row for every tags answer. `names` are the identifiers the tags index is asked
/// about; empty means any empty LSP answer falls back. No retry: a server that broke is not
/// asked again until the record is replaced.
fn lsp_or_tags(
    cx: &Ctx,
    root: &Path,
    tool: &'static str,
    names: &[&str],
    lsp: impl FnOnce() -> Result<String>,
    tags: impl FnOnce() -> Result<String>,
) -> Result<String> {
    door(cx, root, tool, names, lsp::probe, lsp, tags)
}

/// [`lsp_or_tags`] with the capability probe passed in, so a test can count it.
fn door(
    cx: &Ctx,
    root: &Path,
    tool: &'static str,
    names: &[&str],
    probe: impl FnOnce(&Path) -> capability::Probe,
    lsp: impl FnOnce() -> Result<String>,
    tags: impl FnOnce() -> Result<String>,
) -> Result<String> {
    let name = backend_name(cx, root);
    let mode = Mode::named(&name);
    // `Text` never gets here: `text_or` answers it first.
    if matches!(mode, Mode::Tags | Mode::Text) {
        return tags();
    }
    let auto = mode == Mode::Auto;
    let t0 = std::time::Instant::now();
    let counted = tally::mark();
    lsp::set_timeout_ms(u64::from(
        cx.plugin_config::<crate::config::Graph>("graph")
            .lsp_timeout_ms,
    ));
    let said = |text: String| {
        if auto { format!("(lsp)\n{text}") } else { text }
    };
    let reason: String = match capability::ask(cx, root, &name, probe, lsp) {
        capability::Asked::Tags(rec) if auto && !rec.server => {
            let out = format!("(tags)\n{}", tags()?);
            record_mode_row(cx, tags_kind(tool), t0, &out);
            return Ok(out);
        }
        capability::Asked::Tags(rec) => rec.reason.unwrap_or_default(),
        capability::Asked::Lsp(Ok(text)) if !lsp_none_answer(&text) => return Ok(said(text)),
        capability::Asked::Lsp(Ok(text)) => {
            if !names.is_empty() && !tags_know(cx, root, names)? {
                return Ok(said(text));
            }
            "empty answer".to_string()
        }
        capability::Asked::Lsp(Err(e)) => capability::reason_of(&e),
    };
    // The tags answer replaces the server's, so what the server counted before it failed or
    // came back empty is not part of the answer.
    tally::rewind(counted);
    // Every server error already starts with "lsp: "; the header says it once under `auto`.
    let shown = match (auto, reason.strip_prefix("lsp: ")) {
        (true, Some(rest)) => rest,
        _ => &reason,
    };
    let out = format!("(tags; lsp: {shown})\n{}", tags()?);
    record_mode_row(cx, "lsp_fallback", t0, &out);
    if auto {
        record_mode_row(cx, tags_kind(tool), t0, &out);
    }
    Ok(out)
}

fn tags_know(cx: &Ctx, root: &Path, names: &[&str]) -> Result<bool> {
    index_for(cx, root)?;
    let key = index::canon(root);
    for name in names {
        if !cx.symbol_defs(&key, name)?.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// MCP dispatch for the five tools (`mcp.rs` `invoke`). An `Err` becomes an `isError` result.
/// `scope` is the project scope of the call (T329.4.1, T329.4.2), resolved by the caller because
/// `Ctx` carries no project registry. T263: every tool but `outline` walks its scope's roots
/// (index or LSP server), so `scope::` checks each member.
pub fn call(cx: &Ctx, name: &str, args: &Value, scope: &[scope::Member]) -> Result<String> {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let own;
    let scope = if scope.is_empty() {
        own = [scope::Member {
            name: String::new(),
            root: root.clone(),
        }];
        &own[..]
    } else {
        scope
    };
    let arg = |k: &str| args[k].as_str().unwrap_or("");
    let filter = Filter {
        path: arg("path").to_string(),
        kind: arg("kind").to_string(),
        all: args["all"].as_bool().unwrap_or(false),
    };
    let text = match name {
        "symbol" => {
            let id = arg("id");
            if !id.is_empty() {
                scope::symbol_id(cx, scope, id)?
            } else {
                let names = symbol_arg_names(args);
                if names.iter().all(|n| n.is_empty()) {
                    anyhow::bail!("missing `name` or `id`");
                }
                scope::symbols(cx, scope, &names, &filter)?
            }
        }
        "callers" if arg("name").is_empty() => scope::dead_filtered(cx, scope, &filter)?,
        "callers" => scope::callers(cx, scope, arg("name"), &filter)?,
        "impact" => {
            let name = arg("name");
            if name.is_empty() {
                let depth = args["depth"].as_u64().unwrap_or(3) as u32;
                scope::affected_path(cx, scope, arg("path"), depth)?
            } else {
                scope::impact(
                    cx,
                    scope,
                    name,
                    args["depth"].as_u64().unwrap_or(2) as u32,
                    &filter,
                    args["to"].as_str(),
                )?
            }
        }
        "outline" => {
            let text = scope::outline(cx, scope, arg("path"))?;
            return Ok(outline_of_kind(&text, arg("path"), &filter.kind));
        }
        "explore" => scope::explore(cx, scope, arg("query"), &filter)?,
        _ => anyhow::bail!("unknown tool: {name}"),
    };
    Ok(format!("{}{text}", head_stale_prefix(cx, scope)))
}

/// T665: `outline` narrowed to one kind of definition (`fn`, `struct`, …) — the word the map
/// prints after any `pub…` marker. A whole-file answer (no `kind`) stays byte-exact.
fn outline_of_kind(text: &str, path: &str, kind: &str) -> String {
    if kind.is_empty() {
        return text.to_string();
    }
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| l.split_whitespace().find(|w| !w.starts_with("pub")) == Some(kind))
        .collect();
    if kept.is_empty() {
        return format!("no {kind} in {path}");
    }
    kept.join("\n")
}

/// `name`, or `names` when the caller asks for several. One name stays the old call.
fn symbol_arg_names(args: &Value) -> Vec<String> {
    if let Some(list) = args.get("names").and_then(Value::as_array) {
        let names: Vec<String> = list
            .iter()
            .filter_map(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if !names.is_empty() {
            return names;
        }
    }
    vec![
        args.get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    ]
}

#[derive(serde::Serialize, serde::Deserialize)]
struct IndexedHead {
    head: String,
}

fn head_key(root: &Path) -> String {
    format!("head:{}", index::canon(root))
}

/// Remember `git rev-parse HEAD` for this root. Git missing or a non-repo is a no-op:
/// indexing still succeeds.
pub(crate) fn remember_indexed_head(cx: &Ctx, root: &Path) {
    let Some(stdout) = git_stdout(root, &["rev-parse", "HEAD"]) else {
        return;
    };
    let Ok(text) = String::from_utf8(stdout) else {
        return;
    };
    let head = text.trim();
    if head.is_empty() {
        return;
    }
    let Ok(value) = serde_json::to_string(&IndexedHead {
        head: head.to_string(),
    }) else {
        return;
    };
    let _ = cx.plugin_state_set("graph", &head_key(root), &value);
}

/// `index <stored> head <live>` when the indexed commit is not HEAD. Empty when
/// there is no stored head, git failed, or the two shas match — the answer stays
/// byte-identical. Full `rev-parse` hex, no timestamp.
fn head_stale_prefix(cx: &Ctx, scope: &[scope::Member]) -> String {
    let mut out = String::new();
    for member in scope {
        let Ok(Some(raw)) = cx.plugin_state_get("graph", &head_key(&member.root)) else {
            continue;
        };
        let Ok(stored) = serde_json::from_str::<IndexedHead>(&raw) else {
            continue;
        };
        if stored.head.is_empty() {
            continue;
        }
        let Some(stdout) = git_stdout(&member.root, &["rev-parse", "HEAD"]) else {
            continue;
        };
        let Ok(live) = String::from_utf8(stdout) else {
            continue;
        };
        let live = live.trim();
        if live.is_empty() || stored.head == live {
            continue;
        }
        out.push_str(&format!("index {} head {live}\n", stored.head));
    }
    out
}

/// `symbol(name)`: `path:line kind` per definition, then that definition's source from
/// `line` to `end_line`, at most `plugins.graph.body_lines` lines each (T8.6). One call
/// answers "what is this and what does it do", which took a `symbol` plus a `read` at v0.1.
pub fn symbol(cx: &Ctx, root: &Path, name: &str) -> Result<String> {
    symbol_filtered(cx, root, name, &Filter::none())
}

/// Filtered `symbol`: a non-empty `filter` keeps only matching definitions (T52.1).
pub fn symbol_filtered(cx: &Ctx, root: &Path, name: &str, filter: &Filter) -> Result<String> {
    text_or(
        cx,
        root,
        "symbol",
        || text::symbol(cx, root, name, filter),
        || {
            lsp_or_tags(
                cx,
                root,
                "symbol",
                &[name],
                || lsp::symbol(cx, root, name, filter),
                || symbol_tags(cx, root, name, filter),
            )
        },
    )
}

fn symbol_tags(cx: &Ctx, root: &Path, name: &str, filter: &Filter) -> Result<String> {
    index_for(cx, root)?;
    let rows: Vec<_> = cx
        .symbol_defs(&index::canon(root), name)?
        .into_iter()
        .filter(|(path, kind, ..)| filter.path_ok(path) && filter.kind_ok(kind))
        .collect();
    if rows.is_empty() {
        return with_stale(
            cx,
            root,
            format!("no definition of {name}{}", filter.scope_note()),
        );
    }
    tally::hit(root, name, rows.iter().map(|r| r.0.as_str()));
    let key = index::canon(root);
    let callees = cx.symbol_callees(&key, name)?;
    with_stale(
        cx,
        root,
        cap(
            cx,
            defs_text(cx, root, name, &rows, &callees, &Tag::default()),
        )?,
    )
}

/// What a definition's `path:line kind` head carries in a multi-project answer: the
/// `[project] ` label in front and the ambiguity `?` behind (T329.4.1).
#[derive(Default)]
pub(crate) struct Tag<'a> {
    pub(crate) prefix: &'a str,
    pub(crate) suffix: &'a str,
}

/// `{path}:{line} {kind}` per definition, then that definition's source, at most
/// `plugins.graph.body_lines` lines each (T8.6). Shared by `symbol` and `explore`
/// (T68.1) so both print a definition the same way; reads each source file once.
fn calls_line(names: &[String], cap: usize) -> String {
    if names.is_empty() {
        return String::new();
    }
    let show = names.len().min(cap);
    let listed = names[..show].join(", ");
    let extra = names.len() - show;
    if extra > 0 {
        format!("calls: {listed} (+{extra})\n")
    } else {
        format!("calls: {listed}\n")
    }
}

fn ambiguous_banner(n: usize) -> String {
    format!("{n} names ambiguous (?): narrow with path or kind, or backend = \"lsp\"\n")
}

fn mark_ambiguous_lines(out: &str) -> String {
    if out.is_empty() {
        return String::new();
    }
    out.lines()
        .map(|line| format!("{line} ?"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn annotate_ambiguous(cx: &Ctx, root: &Path, name: &str, out: String) -> Result<String> {
    Ok(flag_ambiguous(
        cx.symbol_defs(&index::canon(root), name)?.len(),
        out,
    ))
}

/// `defs` is every definition of the name in the answer's scope, not only the shown ones.
fn flag_ambiguous(defs: usize, out: String) -> String {
    if defs > 1 {
        format!("{}{}", ambiguous_banner(1), mark_ambiguous_lines(&out))
    } else {
        out
    }
}

fn defs_text(
    cx: &Ctx,
    root: &Path,
    name: &str,
    rows: &[(String, String, i32, i32)],
    callees: &[(String, i32, String, i32)],
    tag: &Tag,
) -> String {
    let budget = cx.plugin_config::<crate::config::Graph>("graph").body_lines as usize;
    let cap = budget / 2;
    let key = index::canon(root);
    let mut by_def: HashMap<(String, i32), Vec<String>> = HashMap::new();
    for (path, line, callee, _) in callees {
        by_def
            .entry((path.clone(), *line))
            .or_default()
            .push(callee.clone());
    }
    let mut out = String::new();
    let mut sha_of: HashMap<String, Option<String>> = HashMap::new();
    let mut full_of: HashMap<String, String> = HashMap::new();
    let mut stale_said: HashSet<String> = HashSet::new();
    for (path, kind, line, end_line) in rows {
        let span = cx
            .symbol_span(&key, path, name, kind, *line)
            .ok()
            .flatten()
            .filter(|s| !s.content_hash.is_empty());
        let body = if let Some(span) = span {
            let current = sha_of.entry(path.clone()).or_insert_with(|| {
                symbol_src_reads_add(1);
                std::fs::read(root.join(path))
                    .ok()
                    .map(|b| crate::store::hex_sha256(&b))
            });
            if current.as_deref() != Some(span.file_sha.as_str()) {
                if stale_said.insert(path.clone()) {
                    out.push_str(&format!("stale {path}\n"));
                }
                String::new()
            } else {
                match read_span(&root.join(path), span.start_byte, span.end_byte) {
                    Ok(text) => {
                        let n = text.lines().count() as i32;
                        if n == 0 {
                            String::new()
                        } else {
                            body_lines(&text, 1, n, budget, &mut |bytes| cx.put_archive(bytes).ok())
                        }
                    }
                    Err(_) => {
                        if stale_said.insert(path.clone()) {
                            out.push_str(&format!("stale {path}\n"));
                        }
                        String::new()
                    }
                }
            }
        } else {
            let src = full_of.entry(path.clone()).or_insert_with(|| {
                symbol_src_reads_add(1);
                std::fs::read_to_string(root.join(path)).unwrap_or_default()
            });
            body_lines(src, *line, *end_line, budget, &mut |bytes| {
                cx.put_archive(bytes).ok()
            })
        };
        let head = def_head(path, name, kind, *line);
        out.push_str(&format!("{}{head}{}\n{body}", tag.prefix, tag.suffix));
        if let Some(names) = by_def.get(&(path.clone(), *line)) {
            out.push_str(&calls_line(names, cap));
        }
    }
    out
}

/// `{path}::{name}#{kind}@{line}` — the id printed on every definition head (T474).
pub(crate) fn def_id(path: &str, name: &str, kind: &str, line: i32) -> String {
    format!("{path}::{name}#{kind}@{line}")
}

/// The id, then the historical `{path}:{line} {kind}` head.
pub(crate) fn def_head(path: &str, name: &str, kind: &str, line: i32) -> String {
    format!("{} {path}:{line} {kind}", def_id(path, name, kind, line))
}

/// `path::name#kind@line` → `(path, name, kind, line)`.
pub(crate) fn parse_symbol_id(id: &str) -> Option<(String, String, String, i32)> {
    let (rest, line) = id.rsplit_once('@')?;
    let line = line.parse().ok()?;
    let (rest, kind) = rest.rsplit_once('#')?;
    let (path, name) = rest.split_once("::")?;
    if path.is_empty() || name.is_empty() || kind.is_empty() {
        return None;
    }
    Some((path.to_string(), name.to_string(), kind.to_string(), line))
}

/// One definition selected by id. `name` on the call is ignored by the caller.
pub fn symbol_by_id(cx: &Ctx, root: &Path, id: &str) -> Result<String> {
    let Some((path, name, kind, line)) = parse_symbol_id(id) else {
        return Ok(format!("no definition of {id}"));
    };
    index_for(cx, root)?;
    let key = index::canon(root);
    let rows: Vec<_> = cx
        .symbol_defs(&key, &name)?
        .into_iter()
        .filter(|(p, k, l, _)| p == &path && k == &kind && *l == line)
        .collect();
    if rows.is_empty() {
        return with_stale(cx, root, format!("no definition of {id}"));
    }
    tally::hit(root, &name, [path.as_str()]);
    let callees = cx.symbol_callees(&key, &name)?;
    with_stale(
        cx,
        root,
        cap(
            cx,
            defs_text(cx, root, &name, &rows, &callees, &Tag::default()),
        )?,
    )
}

fn read_span(path: &Path, start: u64, end: u64) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(start))?;
    let mut buf = vec![0u8; end.saturating_sub(start) as usize];
    f.read_exact(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// One definition as `symbol` prints it: the id and `{path}:{line} {kind}` head, then its source.
/// Also the text a symbol-shaped `Grep` is answered with (T369), so both read the same.
pub(crate) fn def_text(
    src: &str,
    name: &str,
    (path, kind, line, end_line): (&str, &str, i32, i32),
    budget: usize,
    archive: &mut dyn FnMut(&[u8]) -> Option<String>,
) -> String {
    format!(
        "{}\n{}",
        def_head(path, name, kind, line),
        body_lines(src, line, end_line, budget, archive)
    )
}

/// Byte offsets of source lines `[first, last)` (0-based, `last` exclusive).
fn line_byte_span(src: &str, first: usize, last: usize) -> (usize, usize) {
    let mut starts = vec![0usize];
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    let start = starts.get(first).copied().unwrap_or(src.len());
    let end = starts.get(last).copied().unwrap_or(src.len());
    (start.min(end), end)
}

/// Source of one definition, `line..=end_line`, at most `budget` lines.
/// Past the budget the uncut span is archived and the line ends `… N more lines, expand <id>`.
/// Shared with the LSP backend so both print a body the same way.
pub(crate) fn body_lines(
    src: &str,
    line: i32,
    end_line: i32,
    budget: usize,
    archive: &mut dyn FnMut(&[u8]) -> Option<String>,
) -> String {
    let first = line.max(1) as usize - 1;
    let last = end_line.max(line) as usize;
    let body: Vec<&str> = src.lines().skip(first).take(last - first).collect();
    let mut out = String::new();
    for l in body.iter().take(budget) {
        out.push_str(l);
        out.push('\n');
    }
    if body.len() > budget {
        let (start, end) = line_byte_span(src, first, last);
        let uncut = src.as_bytes().get(start..end).unwrap_or(b"");
        let extra = body.len() - budget;
        if let Some(id) = archive(uncut) {
            let shown = crate::names::short_id(&id);
            out.push_str(&format!("  … {extra} more lines, expand {shown}\n"));
        } else {
            out.push_str(&format!("  … {extra} more lines\n"));
        }
    }
    out
}

/// `callers(name)`: one line per calling definition, `path  scope xN (Lline)` (T8.5).
/// v0.1 printed every site with its source line; the edge is what the caller needs, and it
/// costs a fraction of the bytes.
pub fn callers(cx: &Ctx, root: &Path, name: &str) -> Result<String> {
    callers_filtered(cx, root, name, &Filter::none())
}

/// Filtered `callers`: a non-empty `filter.path` keeps one subtree (T52.1).
pub fn callers_filtered(cx: &Ctx, root: &Path, name: &str, filter: &Filter) -> Result<String> {
    text_or(
        cx,
        root,
        "callers",
        || text::callers(cx, root, name, filter),
        || {
            lsp_or_tags(
                cx,
                root,
                "callers",
                &[name],
                || lsp::callers(cx, root, name, filter),
                || callers_tags(cx, root, name, filter),
            )
        },
    )
}

fn callers_tags(cx: &Ctx, root: &Path, name: &str, filter: &Filter) -> Result<String> {
    index_for(cx, root)?;
    let key = index::canon(root);
    let ranked = resolve::rank_name(cx, &key, name)?;
    let rows: Vec<_> = cx
        .symbol_ref_groups(&key, name)?
        .into_iter()
        .filter(|(path, ..)| filter.path_ok(path))
        .filter(|(path, ..)| ranked.allows(path))
        .collect();
    if rows.is_empty() {
        return with_stale(
            cx,
            root,
            annotate_ambiguous(
                cx,
                root,
                name,
                format!("no references to {name}{}", filter.scope_note()),
            )?,
        );
    }
    tally::hit(root, name, rows.iter().map(|r| r.0.as_str()));
    let mut out = String::new();
    for (path, scope, n, line) in rows {
        let scope = if scope.is_empty() {
            String::new()
        } else {
            format!("  {scope}")
        };
        out.push_str(&format!("{path}{scope} ×{n} (L{line})\n"));
    }
    if ranked.others > 0 {
        out.push_str(&other_defs_line(name, ranked.others));
    }
    with_stale(cx, root, cap(cx, annotate_ambiguous(cx, root, name, out)?)?)
}

fn other_defs_line(name: &str, others: usize) -> String {
    format!("+{others} other definitions of {name}\n")
}

/// `impact(name, depth)`: breadth-first walk of the `scope` edges T8.5 stored — who calls
/// `name`, who calls them, and so on (T8.7). One `depth  path  scope` line per definition
/// reached. A definition is expanded once, so a call cycle terminates.
pub fn impact(cx: &Ctx, root: &Path, name: &str, depth: u32, to: Option<&str>) -> Result<String> {
    impact_filtered(cx, root, name, depth, &Filter::none(), to)
}

/// Filtered `impact`: a non-empty `filter.path` keeps the reported lines in one
/// subtree; the walk itself still crosses files outside it, so depth is not cut
/// short (T52.1).
pub fn impact_filtered(
    cx: &Ctx,
    root: &Path,
    name: &str,
    depth: u32,
    filter: &Filter,
    to: Option<&str>,
) -> Result<String> {
    text_or(
        cx,
        root,
        "impact",
        || text::impact(cx, root, name, depth, filter, to),
        || {
            lsp_or_tags(
                cx,
                root,
                "impact",
                &[name],
                || with_stale(cx, root, lsp::impact(cx, root, name, depth, filter, to)?),
                || impact_tags(cx, root, name, depth, filter, to),
            )
        },
    )
}

fn impact_tags(
    cx: &Ctx,
    root: &Path,
    name: &str,
    depth: u32,
    filter: &Filter,
    to: Option<&str>,
) -> Result<String> {
    index_for(cx, root)?;
    if let Some(target) = to.filter(|s| !s.is_empty()) {
        let chains = cx
            .symbol_paths(&index::canon(root), target, name, depth)?
            .into_iter()
            .map(|c| reverse_call_chain(&c))
            .collect::<Vec<_>>();
        if chains.is_empty() {
            return with_stale(
                cx,
                root,
                format!("no path from {name} to {target} within depth {depth}"),
            );
        }
        let body = chains.join(
            "
",
        ) + "
";
        tally::hit(root, name, std::iter::empty::<&str>());
        return with_stale(
            cx,
            root,
            cap(cx, annotate_ambiguous(cx, root, name, body)?)?,
        );
    }
    let key = index::canon(root);
    let ranked = resolve::rank_name(cx, &key, name)?;
    // `def_path` is only set when several files define the name. One definition is its file; an
    // unresolved name may itself be a path, so its history is asked as is.
    let file = ranked
        .def_path
        .clone()
        .or_else(|| match cx.symbol_defs(&key, name).ok()?.as_slice() {
            [(path, ..)] => Some(path.clone()),
            _ => None,
        })
        .unwrap_or_else(|| name.to_string());
    let mut rows = impact_bfs(cx, &key, name, depth)?;
    rows.retain(|(_, path, _)| filter.path_ok(path));
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    if rows.is_empty() {
        let text = annotate_ambiguous(
            cx,
            root,
            name,
            format!("nothing reaches {name}{}", filter.scope_note()),
        )?;
        return with_stale(cx, root, with_cochange(cx, root, &file, text));
    }
    tally::hit(root, name, rows.iter().map(|r| r.1.as_str()));
    let mut text = String::new();
    if ranked.others > 0 {
        // Cap keeps the head, so this line has to lead or a long walk hides it.
        text.push_str(&other_defs_line(name, ranked.others));
    }
    // The budget covers the finished answer, so what is added around the rows is taken off it.
    let mark = cx.symbol_defs(&key, name)?.len() > 1;
    let mut around = if mark {
        ambiguous_banner(1) + &mark_ambiguous_lines(&text)
    } else {
        text.clone()
    };
    around.extend(cochange::changes_with(cx, root, &file));
    let budget = blast::Budget {
        overhead: cx.estimate(&around, Class::Code),
        mark,
        ..blast::Budget::new(cx, filter.all)
    };
    text.push_str(&blast::render(cx, &rows, name, &budget, || {
        rank::ranks(cx, &key)
    })?);
    let text = cap(cx, annotate_ambiguous(cx, root, name, text)?)?;
    with_stale(cx, root, with_cochange(cx, root, &file, text))
}

/// T371: the files that change with `file` in git history, after the walk's lines. Added after
/// the cap, which keeps the head, so a long walk cannot hide it.
fn with_cochange(cx: &Ctx, root: &Path, file: &str, mut text: String) -> String {
    if let Some(line) = cochange::changes_with(cx, root, file) {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&line);
        text.push('\n');
    }
    text
}

/// `symbol_paths` walks callers; `impact --to` prints callee chains, so reverse the arrows.
fn reverse_call_chain(chain: &str) -> String {
    let parts: Vec<&str> = chain.split(" → ").collect();
    parts.into_iter().rev().collect::<Vec<_>>().join(" → ")
}

/// One `depth  path  scope` line per row, `(file)` when the row is file-level.
/// Shared by `impact` and `explore` (T68.1) so both print a walk the same way.
pub(crate) fn impact_lines_text(rows: &[(u32, String, String)]) -> String {
    let mut out = String::new();
    for (d, path, scope) in rows {
        if scope.is_empty() {
            out.push_str(&format!("{d}  {path}  (file)\n"));
        } else {
            out.push_str(&format!("{d}  {path}  {scope}\n"));
        }
    }
    out
}

/// T367: the graph root a CLI subcommand works on: `path`, else the cwd. A missing or non-directory
/// path is an error naming it, so a typo cannot report an empty index with exit 0. The T356 refusal
/// of `/` and `$HOME` stays in `index::run_with`.
pub fn cli_root(path: Option<PathBuf>) -> Result<PathBuf> {
    let root = match path {
        Some(p) => p,
        None => std::env::current_dir()?,
    };
    let meta = std::fs::metadata(&root).map_err(|e| anyhow::anyhow!("{}: {e}", root.display()))?;
    anyhow::ensure!(meta.is_dir(), "{}: not a directory", root.display());
    Ok(root)
}

/// T329.4.2: the root of a subcommand that works on one project: `--project` (id or directory,
/// as `projects` resolves it), else `cli_root(path)`.
pub fn cli_root_for(
    store: &crate::store::Store,
    path: Option<PathBuf>,
    project: Option<String>,
) -> Result<PathBuf> {
    match project {
        Some(target) => cli_root(Some(PathBuf::from(projects::resolve(store, &target)?.root))),
        None => cli_root(path),
    }
}

/// One `dead()` row (T52.4 / T230): an unreferenced private definition's location.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DeadRow {
    pub path: String,
    pub line: i32,
    pub kind: String,
    pub name: String,
}

/// `dead_rows()`: [`index_for`] (today's behaviour: freshen, then read) followed by
/// [`dead_candidates`]. The shared computation behind `dead()`'s text and `graph dead
/// --json` — CLI calls that are expected to walk the tree once for a current answer.
pub fn dead_rows(cx: &Ctx, root: &Path) -> Result<Vec<DeadRow>> {
    // T329.10: a text search has no reference edges, so "unreferenced" would be a guess.
    if text::applies(cx, root) {
        anyhow::bail!("dead: {}", text::UNAVAILABLE);
    }
    index_for(cx, root)?;
    dead_candidates(cx, root)
}

/// Unreferenced private definitions (T52.4), read from the store as it stands — no
/// [`index_for`] walk. Drops pub items, methods in trait impls/trait bodies, test
/// files and `#[test]` fns, `macro` definitions and `main`. The Graph page (T230)
/// calls this directly: a 2 s tick reads the store, it does not re-walk the tree.
pub fn dead_candidates(cx: &Ctx, root: &Path) -> Result<Vec<DeadRow>> {
    let key = index::canon(root);
    let mut files: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut rows = Vec::new();
    for (path, name, kind, line) in cx.symbol_dead_candidates(&key)? {
        if kind == "macro" || name == "main" || is_test_path(&path) {
            continue;
        }
        let src = files
            .entry(path.clone())
            .or_insert_with(|| {
                std::fs::read_to_string(root.join(&path))
                    .unwrap_or_default()
                    .lines()
                    .map(str::to_string)
                    .collect()
            })
            .clone();
        let def = src
            .get(line.max(1) as usize - 1)
            .map(String::as_str)
            .unwrap_or("");
        let trimmed = def.trim_start();
        if trimmed.starts_with("pub ") || trimmed.starts_with("pub(") || trimmed == "pub" {
            continue;
        }
        if has_test_attr(&src[..(line.max(1) as usize - 1).min(src.len())]) {
            continue;
        }
        #[cfg(feature = "lang-rust")]
        if path.ends_with(".rs") {
            let (ranges, types) = rust_impls(&src.join("\n"));
            // Named as an `impl` target (`impl S`, `impl T for S`): used.
            if types.iter().any(|t| t == &name) {
                continue;
            }
            if kind == "method"
                && ranges
                    .iter()
                    .any(|(s, e)| *s <= line as usize && line as usize <= *e)
            {
                continue;
            }
        }
        rows.push(DeadRow {
            path,
            line,
            kind,
            name,
        });
    }
    Ok(rows)
}

/// Whether the attribute/comment block right above a definition marks a test: `#[test]`,
/// `#[rstest]`, `#[tokio::test]`, `#[cfg(test)]`, `#[case(..)]`, wherever it sits in the stack.
fn has_test_attr(above: &[String]) -> bool {
    above
        .iter()
        .rev()
        .map(|l| l.trim_start())
        .take_while(|t| t.starts_with("#[") || t.starts_with("//"))
        .any(|t| {
            ["#[test", "#[cfg(test", "#[rstest", "#[case"]
                .iter()
                .any(|p| t.starts_with(p))
                || t.split(['(', ']'])
                    .next()
                    .is_some_and(|a| a.ends_with("::test"))
        })
}

/// `dead()`: [`dead_rows`] as `path:line kind name` lines (T52.4), capped for hook /
/// CLI text output. `graph dead --json` (T230) prints the same rows uncapped instead.
pub fn dead(cx: &Ctx, root: &Path) -> Result<String> {
    if text::applies(cx, root) {
        return Ok(format!("(text)\ndead: {}", text::UNAVAILABLE));
    }
    let rows = dead_rows(cx, root)?;
    if rows.is_empty() {
        return Ok(format!("no dead code in {}", root.display()));
    }
    let mut out = String::new();
    for r in &rows {
        out.push_str(&format!("{}:{} {} {}\n", r.path, r.line, r.kind, r.name));
    }
    cap(cx, out)
}

/// Test files by path: `tests/` dirs and test-named files. Test-only bodies
/// (`#[test]`) are filtered at the call site from the source lines.
fn is_test_path(path: &str) -> bool {
    path == "tests"
        || path.starts_with("tests/")
        || path.contains("/tests/")
        || path.contains("/__tests__/")
        || path.split('/').next_back().is_some_and(|f| {
            f.starts_with("test_")
                || f.starts_with("_test")
                || f.contains("_test.")
                || f.contains(".test.")
                || f.contains(".spec.")
        })
}

/// Fixtures, benchmarks and examples by directory: code that ships to no user, so a missing
/// test there is not a finding.
fn is_support_path(path: &str) -> bool {
    const DIRS: &[&str] = &[
        "fixtures", "fixture", "testdata", "bench", "benches", "examples",
    ];
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    parts.iter().any(|d| DIRS.contains(d))
}

/// 1-based line ranges of `impl X for Y` blocks and `trait` bodies in Rust source
/// (methods there are interface surface, not dead code), plus every `impl` target
/// type name (`impl S`, `impl T for S` — the tags query records no reference for
/// the type of a trait impl, so `S` would otherwise read as dead).
#[cfg(feature = "lang-rust")]
fn rust_impls(src: &str) -> (Vec<(usize, usize)>, Vec<String>) {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    if parser.set_language(&lang).is_err() {
        return (Vec::new(), Vec::new());
    }
    let Some(tree) = parser.parse(src, None) else {
        return (Vec::new(), Vec::new());
    };
    let mut out = (Vec::new(), Vec::new());
    collect_impls(tree.root_node(), src.as_bytes(), &mut out);
    out
}

#[cfg(feature = "lang-rust")]
fn collect_impls(
    node: tree_sitter::Node<'_>,
    src: &[u8],
    out: &mut (Vec<(usize, usize)>, Vec<String>),
) {
    if node.kind() == "impl_item" {
        if let Some(t) = node.child_by_field_name("type")
            && let Ok(name) = t.utf8_text(src)
        {
            // Generics read as `S<T>` and never equal a definition name.
            out.1
                .push(name.split('<').next().unwrap_or(name).trim().to_string());
        }
        if node.child_by_field_name("trait").is_some() {
            out.0
                .push((node.start_position().row + 1, node.end_position().row + 1));
        }
    }
    if node.kind() == "trait_item" {
        out.0
            .push((node.start_position().row + 1, node.end_position().row + 1));
    }
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();
    for child in children {
        collect_impls(child, src, out);
    }
}
const EMPTY_AFFECTED: &str = "no indexed test reaches the change; run the suite";

fn rel_of(root: &Path, path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = path.strip_prefix("./").unwrap_or(&path);
    match (
        fs_kit::path::canonicalize(root.join(path)),
        fs_kit::path::canonicalize(root),
    ) {
        (Ok(abs), Ok(root)) => abs
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string()),
        _ => path.to_string(),
    }
}

/// Tests that reach files changed in git (`git diff --name-only`). No Measurement.
pub fn affected(
    cx: &Ctx,
    root: &Path,
    since: Option<&str>,
    staged: bool,
    json: bool,
) -> Result<String> {
    affected_from_paths(cx, root, &git_changed_files(root, since, staged), 3, json)
}

pub(crate) fn affected_from_paths(
    cx: &Ctx,
    root: &Path,
    paths: &[String],
    depth: u32,
    json: bool,
) -> Result<String> {
    index_for(cx, root)?;
    let key = index::canon(root);
    let (mut hits, starts) = changed_starts(cx, root, &key, paths)?;
    for name in &starts {
        for (_, path, scope) in impact_bfs(cx, &key, name, depth)? {
            if is_test_path(&path) {
                hits.insert((path, via_of(name, scope)));
            }
        }
    }
    tally_tests(root, &hits);
    Ok(format_affected(&hits, json))
}

/// The affected test files a project lists; the symbol each is reached through names no file.
pub(super) fn tally_tests(root: &Path, hits: &Hits) {
    if !hits.is_empty() {
        tally::hit(root, "", hits.iter().map(|(file, _)| file.as_str()));
    }
}

/// What a project's changed paths start from: the test files they hit directly or by naming
/// convention, and the definitions the walk climbs up from (T329.5 shares this with the scope).
fn changed_starts(
    cx: &Ctx,
    root: &Path,
    key: &str,
    paths: &[String],
) -> Result<(Hits, HashSet<String>)> {
    // T372: indexed paths for name-convention test links (existing files only).
    let indexed: HashSet<String> = cx.symbol_stats(key)?.into_keys().collect();
    let mut hits = BTreeSet::new();
    let mut starts = HashSet::new();
    for raw in paths {
        let rel = rel_of(root, raw);
        for candidate in name_linked_tests(&rel) {
            if indexed.contains(&candidate) && is_test_path(&candidate) {
                hits.insert((candidate, "(by name)".to_string()));
            }
        }
        for name in defs_in_path(cx, root, key, &rel)? {
            if is_test_path(&rel) {
                hits.insert((rel.clone(), name.clone()));
            }
            starts.insert(name);
        }
    }
    Ok((hits, starts))
}

/// The symbol a test is reached through: the calling scope, else the changed name itself.
fn via_of(name: &str, scope: String) -> String {
    if scope.is_empty() {
        name.to_string()
    } else {
        scope
    }
}

/// T372: candidate test paths linked by naming convention to `rel` (same stem).
fn name_linked_tests(rel: &str) -> Vec<String> {
    let path = Path::new(rel);
    let Some(stem) = path.file_stem().and_then(OsStr::to_str) else {
        return Vec::new();
    };
    // `foo.test.ts` / `foo.spec.ts` → stem before the test suffix for reverse lookup is unused;
    // we only map source → test here.
    let parent = path
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .filter(|p| !p.is_empty() && p != ".")
        .unwrap_or_default();
    let join = |dir: &str, file: &str| -> String {
        if dir.is_empty() {
            file.to_string()
        } else {
            format!("{dir}/{file}")
        }
    };
    let ext = path
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut out = Vec::new();
    match ext.as_str() {
        "rs" => {
            out.push(format!("tests/{stem}.rs"));
            out.push(join(&parent, &format!("{stem}_test.rs")));
        }
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => {
            out.push(join(&parent, &format!("{stem}.test.{ext}")));
            out.push(join(&parent, &format!("{stem}.spec.{ext}")));
            out.push(join(&parent, &format!("__tests__/{stem}.{ext}")));
        }
        "py" => {
            out.push(join(&parent, &format!("test_{stem}.py")));
            out.push(join(&parent, &format!("{stem}_test.py")));
        }
        "go" => {
            out.push(join(&parent, &format!("{stem}_test.go")));
        }
        _ => {}
    }
    out
}

fn defs_in_path(cx: &Ctx, root: &Path, key: &str, rel: &str) -> Result<Vec<String>> {
    let abs = root.join(rel);
    let src = std::fs::read_to_string(&abs).unwrap_or_default();
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for hit in crate::plugins::read::outline::tags(&abs, &src)? {
        if hit.is_def
            && seen.insert(hit.name.clone())
            && cx
                .symbol_defs(key, &hit.name)?
                .iter()
                .any(|(p, ..)| p == rel)
        {
            names.push(hit.name);
        }
    }
    Ok(names)
}

/// Stdout of `git -C root <args>`. Failure text starts with `git diff failed`
/// so a review can tell a broken git from an empty diff.
pub(crate) fn git_stdout_result(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let argv = [OsStr::new("-C"), root.as_os_str()]
        .into_iter()
        .chain(args.iter().map(OsStr::new));
    let out = ProgressRunner::quiet("git", argv)
        .run()
        .map_err(|e| anyhow::anyhow!("git diff failed: {e}"))?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let err = err.trim();
    let detail = if err.is_empty() { "git failed" } else { err };
    anyhow::bail!("git diff failed: {detail}")
}

/// Stdout of `git -C root <args>`; `None` when git is missing, `root` is not a repo or git failed.
pub(crate) fn git_stdout(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    git_stdout_result(root, args).ok()
}

/// `git diff` argv: `diff`, then `extra`, then `--cached` and the ref. Shared by `affected`
/// (errors swallowed) and `review` (errors returned).
pub(crate) fn git_diff_args(since: Option<&str>, staged: bool, extra: &[&str]) -> Vec<String> {
    let mut args = Vec::with_capacity(extra.len() + 3);
    args.push("diff".to_string());
    args.extend(extra.iter().map(|s| (*s).to_string()));
    if staged {
        args.push("--cached".to_string());
    }
    if let Some(since) = since {
        args.push(since.to_string());
    }
    args
}

fn git_changed_files(root: &Path, since: Option<&str>, staged: bool) -> Vec<String> {
    let args = git_diff_args(since, staged, &["--name-only", "--relative", "-z"]);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    git_stdout(root, &refs)
        .unwrap_or_default()
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .filter_map(|s| String::from_utf8(s.to_vec()).ok())
        .collect()
}

fn test_command(path: &str, name: &str) -> Option<String> {
    match Path::new(path)
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or("")
    {
        "rs" => Some(format!("cargo test {name}")),
        "py" => Some(format!("pytest {path}::{name}")),
        "go" => Some(format!("go test -run {name}")),
        "ts" | "tsx" | "js" | "mjs" | "cjs" | "jsx" => Some(format!("vitest {path}")),
        _ => None,
    }
}

/// Affected tests as `(file, via symbol)`.
type Hits = BTreeSet<(String, String)>;

fn tests_json(hits: &Hits) -> Vec<Value> {
    hits.iter()
        .map(|(file, symbol)| {
            json!({
                "file": file,
                "symbol": symbol,
                "command": test_command(file, symbol),
            })
        })
        .collect()
}

fn format_affected(hits: &Hits, json: bool) -> String {
    if json {
        let tests = tests_json(hits);
        return if tests.is_empty() {
            json!({"tests": [], "message": EMPTY_AFFECTED}).to_string()
        } else {
            json!({"tests": tests}).to_string()
        };
    }
    if hits.is_empty() {
        return EMPTY_AFFECTED.to_string();
    }
    let mut out = String::new();
    for (file, symbol) in hits {
        out.push_str(&format!("{file} ← via {symbol}\n"));
        if let Some(cmd) = test_command(file, symbol) {
            out.push_str(&format!("{cmd}\n"));
        }
    }
    out
}

/// T8.7 BFS (T8.14 baseline). T68.5 walks it from each changed file's definitions.
/// `follow_imports` (default true, T68.6) takes one extra hop from an import row
/// to that file's definitions.
pub(crate) fn impact_bfs(
    cx: &Ctx,
    root: &str,
    name: &str,
    depth: u32,
) -> Result<Vec<(u32, String, String)>> {
    impact_bfs_follow(cx, root, name, depth, true)
}

pub(crate) fn impact_bfs_follow(
    cx: &Ctx,
    root: &str,
    name: &str,
    depth: u32,
    follow_imports: bool,
) -> Result<Vec<(u32, String, String)>> {
    Ok(impact_walk_roots(cx, &[root], name, depth, follow_imports)?
        .into_iter()
        .map(|(_, d, path, scope)| (d, path, scope))
        .collect())
}

/// The walk over the indexes of a project scope (T329.4.2): one frontier, asked of every root
/// at each level, so a function in C reaches its callers in B and then theirs in A. A row's first
/// field is the position of the root it was found in. Each root ranks an ambiguous name on its
/// own files.
pub(crate) fn impact_walk_roots(
    cx: &Ctx,
    roots: &[&str],
    name: &str,
    depth: u32,
    follow_imports: bool,
) -> Result<Vec<(usize, u32, String, String)>> {
    impact_walk_memo(
        cx,
        roots,
        name,
        depth,
        follow_imports,
        &mut WalkMemo::default(),
    )
}

/// Store answers for one `(root, name)`, kept across walks over an index that does not change
/// meanwhile. `review` walks hundreds of definitions whose callers overlap; asking again for
/// each walk was most of its time (T646).
#[derive(Default)]
pub(crate) struct WalkMemo {
    ranked: HashMap<RootName, resolve::Hit>,
    refs: HashMap<RootName, Vec<RefGroup>>,
    follows: HashMap<RootName, Vec<(String, String)>>,
}

type RootName = (String, String);
/// One `symbol_ref_groups` row: path, scope, count, first line.
type RefGroup = (String, String, i64, i32);

pub(crate) fn impact_walk_memo(
    cx: &Ctx,
    roots: &[&str],
    name: &str,
    depth: u32,
    follow_imports: bool,
    memo: &mut WalkMemo,
) -> Result<Vec<(usize, u32, String, String)>> {
    let mut seen: HashSet<String> = HashSet::from([name.to_string()]);
    let mut frontier = vec![name.to_string()];
    let mut out = Vec::new();
    for d in 1..=depth.clamp(1, 4) {
        let mut next = Vec::new();
        for from in &frontier {
            for (i, root) in roots.iter().enumerate() {
                let key = (root.to_string(), from.clone());
                // Resolved once per root and name: an ambiguous callee only follows the winning definition (T368).
                if !memo.ranked.contains_key(&key) {
                    memo.ranked
                        .insert(key.clone(), resolve::rank_name(cx, root, from)?);
                }
                if !memo.refs.contains_key(&key) {
                    memo.refs
                        .insert(key.clone(), cx.symbol_ref_groups(root, from)?);
                }
                let allow = &memo.ranked[&key];
                for (path, scope, ..) in &memo.refs[&key] {
                    if !allow.allows(path) {
                        continue;
                    }
                    if scope.is_empty() {
                        out.push((i, d, path.clone(), String::new()));
                    } else if seen.insert(scope.clone()) {
                        out.push((i, d, path.clone(), scope.clone()));
                        next.push(scope.clone());
                    }
                }
                if follow_imports {
                    if !memo.follows.contains_key(&key) {
                        memo.follows
                            .insert(key.clone(), cx.symbol_import_follow(root, from)?);
                    }
                    for (path, def) in &memo.follows[&key] {
                        if seen.insert(def.clone()) {
                            out.push((i, d, path.clone(), def.clone()));
                            next.push(def.clone());
                        }
                    }
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
fn symbol_src_reads_add(n: usize) {
    SYMBOL_SRC_READS.with(|c| c.fetch_add(n, Ordering::Relaxed));
}

#[cfg(not(test))]
fn symbol_src_reads_add(_n: usize) {}

/// `outline(path)`: the `read` plugin's `map` mode, capped like the other two.
pub fn outline(cx: &Ctx, path: &str) -> Result<String> {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    outline_in(cx, &root, path)
}

fn outline_in(cx: &Ctx, root: &Path, path: &str) -> Result<String> {
    // The path guard runs outside the wrapper: a refused path is an error to report, not
    // a server failure to paper over with the tags answer.
    let abs = if mode_of(cx, root) != Mode::Tags {
        let allow = &cx.plugin_config::<crate::config::Read>("read").allow_paths;
        Some(crate::plugins::read::resolve(root, Path::new(path), allow)?)
    } else {
        None
    };
    if let Some(abs) = abs.as_deref()
        && !abs.exists()
    {
        let hint = crate::plugins::read::roots::moved_hint(root, abs);
        if !hint.is_empty() {
            anyhow::bail!("No such file or directory: {path}{hint}");
        }
    }
    text_or(
        cx,
        root,
        "outline",
        || text::outline(cx, root, abs.as_deref().unwrap_or(Path::new(path))),
        || {
            lsp_or_tags(
                cx,
                root,
                "outline",
                &[],
                || {
                    let abs = abs.as_deref().unwrap_or(Path::new(path));
                    with_stale(cx, root, lsp::outline(cx, root, &abs.to_string_lossy())?)
                },
                || {
                    let text = crate::plugins::read::read_with(
                        cx,
                        &crate::fs::HostFs,
                        root,
                        path,
                        "map",
                        None,
                    )?;
                    tally::hit(root, "", [path]);
                    with_stale(cx, root, cap(cx, text)?)
                },
            )
        },
    )
}

// ---------- T68.1: explore ----------

/// Identifier tokens of a free-text question: alphanumeric/`_` runs, deduped,
/// first 8 — single letters are real identifiers (`b`, `c`, `x`), so nothing but
/// empty runs are dropped; the cap keeps the resolution and the pairwise path
/// walk below fast.
pub(super) fn explore_tokens(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| !t.is_empty())
        .filter(|t| seen.insert((*t).to_string()))
        .take(8)
        .map(str::to_string)
        .collect()
}

/// At most this many distinct names land in one answer: pairwise paths are
/// `O(names²)` walks and the answer is capped anyway.
const EXPLORE_MAX_NAMES: usize = 10;

/// The backend pieces `explore` assembles its answer from (T68.1). Both backends —
/// tree-sitter-tags (`TagsExplore`) and LSP (`lsp::LspExplore`) — answer the same
/// four queries, so one assembler produces one answer shape.
pub(crate) trait ExploreParts {
    /// Exact-then-prefix resolution of one query token to definition names (best 5).
    fn resolve(&mut self, token: &str) -> Result<Vec<String>>;
    /// `{path}:{line} {kind}` + body lines for every definition of `name`.
    fn defs(&mut self, name: &str) -> Result<String>;
    /// Call chains `a → … → b` in the caller direction, at most 3 hops.
    fn paths(&mut self, a: &str, b: &str) -> Result<Vec<String>>;
    /// What `impact(name, 1)` would print, and the row count behind it.
    fn impact1(&mut self, name: &str) -> Result<(String, usize)>;
    fn def_count(&mut self, name: &str) -> Result<usize>;
}

/// One assembled `explore` answer plus the bytes the separate calls it replaces
/// would have returned (`symbol` + `impact` per name) — the Measurement's `before`.
pub(crate) fn assemble_explore(
    query: &str,
    filter: &Filter,
    parts: &mut dyn ExploreParts,
) -> Result<(String, u64)> {
    let mut names: Vec<Named> = Vec::new();
    for token in explore_tokens(query) {
        for name in parts.resolve(&token)? {
            if !names.iter().any(|n| n.name == name) {
                names.push(Named {
                    name,
                    token: token.clone(),
                });
            }
        }
        if names.len() >= EXPLORE_MAX_NAMES {
            break;
        }
    }
    assemble_named(query, filter, parts, &names)
}

/// A name `explore` prints and the query token that resolved it (empty when the full-text
/// fallback found it, which no token asked for).
pub(crate) struct Named {
    pub name: String,
    pub token: String,
}

/// The answer `assemble_explore` prints once the names are known.
pub(crate) fn assemble_named(
    query: &str,
    filter: &Filter,
    parts: &mut dyn ExploreParts,
    names: &[Named],
) -> Result<(String, u64)> {
    if names.is_empty() {
        return Ok((
            format!("no symbols resolved for \"{query}\"{}", filter.scope_note()),
            0,
        ));
    }
    let mut counts = HashMap::new();
    for Named { name, .. } in names {
        counts.insert(name.clone(), parts.def_count(name)?);
    }
    let ambiguous = counts.values().filter(|c| **c > 1).count();
    let mut out = String::new();
    if ambiguous > 0 {
        out.push_str(&ambiguous_banner(ambiguous));
    }
    let mut before = 0u64;
    let mut impact = Vec::new();
    for Named { name, token } in names {
        let seen = tally::mark();
        let defs = parts.defs(name)?;
        tally::rename(seen, token);
        before += defs.len() as u64;
        out.push_str(&format!("= {name}\n{defs}"));
        // The impact line prints a count, not the callers' files, so those rows are not listed.
        let seen = tally::mark();
        let (text, n) = parts.impact1(name)?;
        tally::rewind(seen);
        before += text.len() as u64;
        let mark = if counts[name] > 1 { " ?" } else { "" };
        impact.push(format!("{name} ← {n}{mark}"));
    }
    out.push_str("paths:\n");
    let mut any = false;
    for a in names.iter().map(|n| &n.name) {
        for b in names.iter().map(|n| &n.name) {
            if a != b {
                for chain in parts.paths(a, b)? {
                    out.push_str(&chain);
                    out.push('\n');
                    any = true;
                }
            }
        }
    }
    if !any {
        out.push_str("none\n");
    }
    out.push_str("impact:\n");
    for line in impact {
        out.push_str(&line);
        out.push('\n');
    }
    Ok((out, before))
}

/// `explore(query, path?)` (T68.1): one call answers a code question the way
/// `symbol` + `impact` + caller-walking did in three to five. The query splits
/// into identifier tokens; each resolves exactly, else by prefix best-5 by
/// reference count. The answer prints every definition body once per file, the
/// call paths between the resolved symbols (`symbol_paths`, ≤ 3 hops) and one
/// impact depth-1 line per symbol, then goes through `cap` like the other tools.
pub fn explore(cx: &Ctx, root: &Path, query: &str, filter: &Filter) -> Result<String> {
    let tokens = explore_tokens(query);
    let names: Vec<&str> = tokens.iter().map(String::as_str).collect();
    text_or(
        cx,
        root,
        "explore",
        || {
            let mut parts = text::TextExplore::new(cx, root, filter);
            Ok(assemble_explore(query, filter, &mut parts)?.0)
        },
        || {
            lsp_or_tags(
                cx,
                root,
                "explore",
                &names,
                || lsp::explore(cx, root, query, filter),
                || explore_tags(cx, root, query, filter),
            )
        },
    )
}

fn explore_tags(cx: &Ctx, root: &Path, query: &str, filter: &Filter) -> Result<String> {
    index_for(cx, root)?;
    let mut parts = TagsExplore {
        cx,
        root,
        filter,
        key: index::canon(root),
        label: "",
    };
    let mut names: Vec<Named> = Vec::new();
    for token in explore_tokens(query) {
        for name in parts.resolve(&token)? {
            if !names.iter().any(|n| n.name == name) {
                names.push(Named {
                    name,
                    token: token.clone(),
                });
            }
        }
        if names.len() >= EXPLORE_MAX_NAMES {
            break;
        }
    }
    // No name resolved: search signature and doc too (T474). An FTS error keeps
    // the "no symbols resolved" line. A resolved name stays byte-stable aside from the id.
    if names.is_empty()
        && let Ok(hits) = cx.symbol_fts(&parts.key, query, EXPLORE_MAX_NAMES as i64)
    {
        for (_, name, _, _) in hits {
            if !names.iter().any(|n| n.name == name) {
                names.push(Named {
                    name,
                    token: String::new(),
                });
            }
            if names.len() >= EXPLORE_MAX_NAMES {
                break;
            }
        }
    }
    let (text, before) = assemble_named(query, filter, &mut parts, &names)?;
    with_stale(cx, root, cap_kind(cx, text, before, "explore")?)
}

/// The tree-sitter-tags backend's pieces: every query is answered from the indexed
/// rows; definition bodies are read from disk the way `symbol` reads them.
struct TagsExplore<'a> {
    cx: &'a Ctx<'a>,
    root: &'a Path,
    filter: &'a Filter,
    key: String,
    /// The `[project] ` label of a scoped answer (T329.4.2); empty for one project.
    label: &'a str,
}

impl ExploreParts for TagsExplore<'_> {
    fn resolve(&mut self, token: &str) -> Result<Vec<String>> {
        if !self.cx.symbol_defs(&self.key, token)?.is_empty() {
            return Ok(vec![token.to_string()]);
        }
        self.cx.symbol_name_prefix(&self.key, token, 5)
    }

    fn defs(&mut self, name: &str) -> Result<String> {
        let ranked = resolve::rank_name(self.cx, &self.key, name)?;
        let mut rows: Vec<_> = self
            .cx
            .symbol_defs(&self.key, name)?
            .into_iter()
            .filter(|(path, ..)| self.filter.path_ok(path))
            .collect();
        if let Some(path) = &ranked.def_path {
            let kept: Vec<_> = rows.iter().filter(|row| &row.0 == path).cloned().collect();
            if !kept.is_empty() {
                rows = kept;
            }
        }
        if rows.is_empty() {
            return Ok(format!(
                "no definition of {name}{}\n",
                self.filter.scope_note()
            ));
        }
        tally::hit(self.root, name, rows.iter().map(|r| r.0.as_str()));
        let callees = self.cx.symbol_callees(&self.key, name)?;
        let tag = Tag {
            prefix: self.label,
            suffix: "",
        };
        let mut text = defs_text(self.cx, self.root, name, &rows, &callees, &tag);
        if ranked.others > 0 {
            text.push_str(&other_defs_line(name, ranked.others));
        }
        Ok(text)
    }

    fn def_count(&mut self, name: &str) -> Result<usize> {
        Ok(self.cx.symbol_defs(&self.key, name)?.len())
    }

    fn paths(&mut self, a: &str, b: &str) -> Result<Vec<String>> {
        self.cx.symbol_paths(&self.key, a, b, 3)
    }

    fn impact1(&mut self, name: &str) -> Result<(String, usize)> {
        let ranked = resolve::rank_name(self.cx, &self.key, name)?;
        let mut rows = impact_bfs(self.cx, &self.key, name, 1)?;
        rows.retain(|(_, path, _)| self.filter.path_ok(path));
        rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        if rows.is_empty() {
            return Ok((
                format!("nothing reaches {name}{}", self.filter.scope_note()),
                0,
            ));
        }
        let n = rows.len();
        let mut text = impact_lines_text(&rows);
        if ranked.others > 0 {
            text.push_str(&other_defs_line(name, ranked.others));
        }
        Ok((text, n))
    }
}

/// T52.3: ranked repo map from existing `symbols` rows. `map_tokens = 0` is off;
/// a missing index does not walk the tree (hook path). `map_rank = "pagerank"` (T370) reads the
/// stored file graph instead and falls back to the reference counts when none is stored.
fn repo_map(ev: &SessionStart, cx: &Ctx) -> Option<Injection> {
    let cfg = cx.plugin_config::<crate::config::Graph>("graph");
    let cap = cfg.map_tokens;
    if cap == 0 {
        return None;
    }
    let cwd = cx
        .cwd()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())?;
    if cfg.map_rank == "pagerank" {
        let root = index::canon(&cwd);
        let seeds = if ev.source == "compact" {
            crate::plugins::checkpoint::last_paths(cx, &root)
        } else {
            Vec::new()
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as i64);
        if let Some(text) = rank::map(cx, &root, cap, &seeds, now) {
            return Some(Injection {
                plugin: "graph",
                text,
                priority: 1,
            });
        }
    }
    let rows = cx
        .symbol_top_refs(&index::canon(&cwd), i64::from(cap))
        .ok()?;
    if rows.is_empty() {
        return None;
    }
    let mut lines = Vec::with_capacity(rows.len() + 1);
    lines.push("repo map".into());
    for (name, refs, path, line) in &rows {
        lines.push(format!("{name} {path}:{line} {refs}"));
    }
    let mut text = lines.join("\n");
    while cx.estimate(&text, Class::Prose) > cap && lines.len() > 1 {
        lines.pop();
        text = lines.join("\n");
    }
    if lines.len() == 1 {
        return None;
    }
    Some(Injection {
        plugin: "graph",
        text,
        priority: 1,
    })
}

/// Cap at `plugins.graph.max_tokens`: whole head lines that fit, then `N more, expand <id>`.
/// Records one measurement only when something was shortened: truncated (`ref_id` set)
/// or smaller than what it stands for. An unchanged answer is not a saving and writes
/// no row (T181). `before_bytes` is the uncapped text for the four plain tools, and the
/// sum of the calls `explore` replaced for `kind = "explore"` (T68.1).
fn cap_kind(cx: &Ctx, text: String, before_bytes: u64, kind: &'static str) -> Result<String> {
    let max = cx.plugin_config::<crate::config::Graph>("graph").max_tokens;
    let est = cx.estimate(&text, Class::Code);
    let (out, ref_id) = if est <= max {
        (text, None)
    } else {
        let id = cx.put_archive(text.as_bytes())?;
        // The estimator is linear in chars, so the char budget scales the same way.
        // 120 leaves room for the trailer (`N more, expand ` plus the printed prefix).
        let text_chars = text.chars().count();
        let budget_chars = (text_chars * max as usize / est as usize).saturating_sub(120);
        let total = text.lines().count();
        let mut head = String::new();
        let mut shown = 0;
        for line in text.lines() {
            if shown > 0 && head.chars().count() + line.chars().count() + 1 > budget_chars {
                break;
            }
            head.push_str(line);
            head.push('\n');
            shown += 1;
        }
        (
            format!(
                "{head}{} more, expand {}",
                total - shown,
                crate::names::short_id(&id)
            ),
            Some(id),
        )
    };
    let after_bytes = out.len() as u64;
    if ref_id.is_some() || after_bytes < before_bytes {
        cx.record(&Measurement {
            plugin: "graph",
            kind,
            before_bytes,
            after_bytes,
            est_before: est,
            est_after: cx.estimate(&out, Class::Code),
            ref_id,
            call_id: cx.call_id(),
        })?;
    }
    Ok(out)
}

fn cap(cx: &Ctx, text: String) -> Result<String> {
    let before = text.len() as u64;
    cap_kind(cx, text, before, "cap")
}

#[cfg(test)]
mod tests {
    use super::index::tests::cx;
    use super::*;
    use rstest::rstest;
    use std::fs;

    fn crate_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A moved HEAD names both full shas when the index is not rebuilt (`auto_index`
    /// off). A directory git does not own gets no line. `outline` stays bare.
    #[test]
    fn stale_head_line_names_both_shas() {
        let (mut cx, dir) = cx("stale-head");
        fs::write(dir.join("a.rs"), "fn alpha() {}\n").unwrap();
        let scope = [scope::Member {
            name: String::new(),
            root: dir.clone(),
        }];
        let plain = call(&Ctx::new(&cx), "symbol", &json!({"name": "alpha"}), &scope).unwrap();
        assert!(
            !plain.lines().next().unwrap_or("").starts_with("index "),
            "{plain}"
        );
        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "t@example.com"]);
        git(&dir, &["config", "user.name", "t"]);
        git(&dir, &["add", "a.rs"]);
        git(&dir, &["commit", "-m", "a"]);
        index::run(&Ctx::new(&cx), &dir, false).unwrap();
        let old = String::from_utf8(git_stdout(&dir, &["rev-parse", "HEAD"]).unwrap()).unwrap();
        let old = old.trim().to_string();
        git(&dir, &["commit", "--allow-empty", "-m", "b"]);
        let new = String::from_utf8(git_stdout(&dir, &["rev-parse", "HEAD"]).unwrap()).unwrap();
        let new = new.trim().to_string();
        cx.config.plugins.graph.auto_index = false;
        let out = call(&Ctx::new(&cx), "symbol", &json!({"name": "alpha"}), &scope).unwrap();
        assert!(
            out.starts_with(&format!("index {old} head {new}\n")),
            "{out}"
        );
        assert!(out.contains("fn alpha()"), "{out}");
        let outline = call(&Ctx::new(&cx), "outline", &json!({"path": "a.rs"}), &scope).unwrap();
        assert!(!outline.starts_with("index "), "{outline}");
        let _ = fs::remove_dir_all(dir);
    }

    /// T8.6: one call gives the definition and its source. The `cap` body is read back
    /// verbatim from the file, so the test cannot pass on a stale or paraphrased index.
    #[test]
    fn symbol_returns_the_definition_body() {
        let (cx, dir) = cx("body");
        let out = symbol(&Ctx::new(&cx), &crate_root(), "cap").unwrap();
        let src = fs::read_to_string(crate_root().join("src/plugins/graph/mod.rs")).unwrap();
        let head = src
            .lines()
            .find(|l| l.starts_with("fn cap(cx: &Ctx"))
            .unwrap();
        assert!(out.contains(head), "{out}");
        assert!(
            out.lines().any(|l| l.contains("src/plugins/graph/mod.rs:")),
            "{out}"
        );
        // T181: an answer under the cap is unchanged, so it is not a saving and writes no row.
        assert_eq!(cx.store.measurement_count("graph").unwrap(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    /// The printed expand id is the 8-char prefix (T907). The archive row keeps the full id.
    fn printed_archive_id(cx: &crate::plugin::Runtime, shown: &str) -> String {
        assert_eq!(shown.len(), crate::names::MIN_PREFIX, "{shown}");
        let id = crate::expand::resolve_id(cx, shown).unwrap();
        assert!(id.starts_with(shown), "{id} {shown}");
        id
    }

    /// T8.6: bodies make `symbol` far larger than v0.1, so the cap and the archive matter
    /// more, not less. 500 one-line definitions of the same name must still fit the budget.
    #[test]
    fn five_hundred_definitions_are_capped_with_archive_id() {
        let (cx, dir) = cx("defcap");
        // One file, not 500: the index runs a transaction per file, and 500 of them put
        // ~13 s of fixture setup into every `just check` for nothing this test measures.
        let src = "fn dup() {\n    ();\n}\n".repeat(500);
        fs::write(dir.join("d.rs"), &src).unwrap();
        let out = symbol(&Ctx::new(&cx), &dir, "dup").unwrap();
        let trailer = out.lines().last().unwrap();
        assert!(trailer.contains(" more, expand "), "{trailer}");
        let id = printed_archive_id(&cx, trailer.rsplit(' ').next().unwrap());
        let full = String::from_utf8(cx.store.get_archive(&id, None).unwrap().unwrap()).unwrap();
        assert_eq!(
            full.lines().filter(|l| l.starts_with("fn dup()")).count(),
            500,
            "the archive holds every definition"
        );
        assert!(cx.estimate(&out, Class::Code) <= cx.config.plugins.graph.max_tokens);
        assert_eq!(cx.store.measurement_count("graph").unwrap(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    /// T36.16: one source file, many definitions — read it once, not once per row.
    #[rstest]
    fn symbol_reads_each_source_file_once() {
        SYMBOL_SRC_READS.with(|c| c.store(0, Ordering::Relaxed));
        let (cx, dir) = cx("symread");
        let src = "fn dup() {\n    ();\n}\n".repeat(500);
        fs::write(dir.join("d.rs"), &src).unwrap();
        symbol(&Ctx::new(&cx), &dir, "dup").unwrap();
        let reads = SYMBOL_SRC_READS.with(|c| c.load(Ordering::Relaxed));
        assert_eq!(reads, 1);
        let _ = fs::remove_dir_all(dir);
    }

    /// T36.16: cap scales in chars so CJK-heavy output cannot overshoot `max_tokens`.
    #[rstest]
    fn cjk_capped_output_respects_max_tokens() {
        let (cx, dir) = cx("cjkcap");
        let body = "// 漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字\n".repeat(3000);
        fs::write(dir.join("cjk.rs"), format!("fn cjk() {{\n{body}}}")).unwrap();
        let out = symbol(&Ctx::new(&cx), &dir, "cjk").unwrap();
        assert!(cx.estimate(&out, Class::Code) <= cx.config.plugins.graph.max_tokens);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn symbol_main_is_in_src_main_rs() {
        let (cx, dir) = cx("symbol");
        // The repo has many `main`s, listed in path order under the output cap; `src/` holds
        // both binaries' (T178 added `rtok-hook`).
        let src = Filter {
            path: "src/".into(),
            kind: String::new(),
            ..Filter::none()
        };
        let out = symbol_filtered(&Ctx::new(&cx), &crate_root(), "main", &src).unwrap();
        for bin in ["src/main.rs:", "src/bin/rtok-hook.rs:"] {
            assert!(out.lines().any(|l| l.starts_with(bin)), "{bin}\n{out}");
        }
        assert_eq!(
            symbol(&Ctx::new(&cx), &crate_root(), "no_such_fn").unwrap(),
            "no definition of no_such_fn"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// T8.5: the caller is a definition, not a line number. `Runtime::estimate` calls the free
    /// `tokens::estimate`, so `src/plugin.rs` must report `estimate` as the calling scope.
    #[test]
    fn callers_estimate_lists_src_plugin_rs() {
        let (cx, dir) = cx("callers");
        let out = callers(&Ctx::new(&cx), &crate_root(), "estimate").unwrap();
        assert!(
            out.lines()
                .any(|l| l.starts_with("src/plugin.rs  estimate \u{d7}")),
            "{out}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn five_hundred_hits_are_capped_with_archive_id() {
        let (cx, dir) = cx("cap");
        // 500 distinct callers, not 500 calls: grouping collapses the latter to one line.
        let mut src = String::from("fn zeta() {}\n");
        for i in 0..500 {
            src.push_str(&format!("fn c{i}() {{ zeta(); }}\n"));
        }
        fs::write(dir.join("zeta.rs"), &src).unwrap();
        let out = callers(&Ctx::new(&cx), &dir, "zeta").unwrap();
        let trailer = out.lines().last().unwrap();
        assert!(trailer.contains(" more, expand "), "{trailer}");
        let shown = trailer.rsplit(' ').next().unwrap();
        let id = printed_archive_id(&cx, shown);
        let full = cx
            .store
            .get_archive(&id, None)
            .unwrap()
            .expect("archived full text");
        assert_eq!(String::from_utf8(full).unwrap().lines().count(), 500);
        let max = cx.config.plugins.graph.max_tokens;
        let est = cx.estimate(&out, Class::Code);
        assert!(est <= max, "{est} > {max}");
        // T181: the one row records a real saving, not `before == after`.
        let rows = cx.store.list_measurements("graph").unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.kind, "cap");
        assert_eq!(row.after_bytes, out.len() as i64);
        assert!(row.after_bytes < row.before_bytes, "{row:?}");
        assert!(row.est_after < row.est_before, "{row:?}");
        assert_eq!(row.ref_id.as_deref(), Some(id.as_str()));
        let _ = fs::remove_dir_all(dir);
    }

    /// T8.7: `impact` walks the edges `callers` only reports one hop of. Depth bounds the
    /// walk, and `x`/`y` calling each other must not loop.
    #[test]
    fn impact_walks_the_call_chain_and_terminates() {
        let (cx, dir) = cx("impact");
        fs::write(
            dir.join("chain.rs"),
            "fn a() {\n    b();\n}\nfn b() {\n    c();\n}\nfn x() {\n    y();\n    c();\n}\nfn y() {\n    x();\n}\n",
        )
        .unwrap();
        let at = |out: &str, n: &str| {
            out.lines()
                .find(|l| l.ends_with(&format!("  {n}")))
                .map(|l| l[..1].to_string())
        };
        let two = impact(&Ctx::new(&cx), &dir, "c", 2, None).unwrap();
        assert_eq!(at(&two, "b").as_deref(), Some("1"), "{two}");
        assert_eq!(at(&two, "a").as_deref(), Some("2"), "{two}");
        let one = impact(&Ctx::new(&cx), &dir, "c", 1, None).unwrap();
        assert_eq!(at(&one, "b").as_deref(), Some("1"), "{one}");
        assert_eq!(at(&one, "a"), None, "depth 1 must stop at the callers");
        // x calls y, y calls x, both reach c: the walk visits each once and returns.
        let deep = impact(&Ctx::new(&cx), &dir, "c", 4, None).unwrap();
        assert_eq!(
            deep.lines().filter(|l| l.ends_with("  x")).count(),
            1,
            "{deep}"
        );
        assert_eq!(
            impact(&Ctx::new(&cx), &dir, "no_such_fn", 2, None).unwrap(),
            "nothing reaches no_such_fn"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn symbol_lists_callees_on_impact_fixture() {
        let (cx, dir) = cx("callees");
        fs::write(
            dir.join("chain.rs"),
            "fn a() {\n    b();\n}\nfn b() {\n    c();\n}\nfn c() {}\n",
        )
        .unwrap();
        let ctx = Ctx::new(&cx);
        assert!(symbol(&ctx, &dir, "a").unwrap().contains("calls: b"));
        assert!(symbol(&ctx, &dir, "b").unwrap().contains("calls: c"));
        assert!(!symbol(&ctx, &dir, "c").unwrap().contains("calls:"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn callers_and_impact_mark_ambiguous_names() {
        let (cx, dir) = cx("ambiguous");
        fs::write(dir.join("a.rs"), "fn new() {}\nfn alpha() { new(); }\n").unwrap();
        fs::write(dir.join("b.rs"), "fn new() {}\n").unwrap();
        let ctx = Ctx::new(&cx);
        assert!(!callers(&ctx, &dir, "alpha").unwrap().contains('?'));
        let new = callers(&ctx, &dir, "new").unwrap();
        assert!(new.starts_with("1 names ambiguous"));
        assert!(new.contains(" ?\n"));
        assert!(
            impact(&ctx, &dir, "new", 1, None)
                .unwrap()
                .starts_with("1 names ambiguous")
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// T368: an import picks one of two same-named definitions. The other
    /// definition's references collapse to one line.
    #[test]
    fn ambiguous_callers_follow_the_import() {
        let (cx, dir) = cx("rank-import");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/a.rs"), "pub fn parse() {}\n").unwrap();
        fs::write(
            dir.join("src/b.rs"),
            "pub fn parse() {}\nfn local() { parse(); }\n",
        )
        .unwrap();
        fs::write(
            dir.join("src/c.rs"),
            "use crate::a::parse;\nfn go() { parse(); }\n",
        )
        .unwrap();
        // Two references in a three-file tree are most of the index. The 5% cutoff
        // needs a wider tree so `parse` stays rankable.
        for i in 0..40 {
            fs::write(
                dir.join(format!("src/p{i}.rs")),
                format!("fn pad{i}() {{}}\n"),
            )
            .unwrap();
        }
        let ctx = Ctx::new(&cx);
        let out = callers(&ctx, &dir, "parse").unwrap();
        assert!(out.contains("src/c.rs"), "{out}");
        assert!(!out.lines().any(|l| l.contains("src/b.rs")), "{out}");
        assert!(out.contains("+1 other definitions of parse"), "{out}");
        let imp = impact(&ctx, &dir, "parse", 1, None).unwrap();
        assert!(imp.contains("src/c.rs"), "{imp}");
        assert!(!imp.lines().any(|l| l.contains("src/b.rs")), "{imp}");
        assert!(imp.contains("+1 other definitions of parse"), "{imp}");
        let explored = explore(&ctx, &dir, "parse", &Filter::none()).unwrap();
        assert!(
            explored.contains("+1 other definitions of parse"),
            "{explored}"
        );
        assert!(explored.contains("src/a.rs"), "{explored}");
        assert!(!explored.contains("src/b.rs:"), "{explored}");
        let _ = fs::remove_dir_all(dir);
    }

    /// T8.13: CTE / path query and the BFS return the same (depth, path, scope) set.
    #[test]
    fn impact_fanout_matches_bfs() {
        let (cx, dir) = cx("fanout");
        let mut src = String::from("fn sink() {}\n");
        for i in 0..10 {
            src.push_str(&format!("fn a{i}() {{ sink(); }}\n"));
            src.push_str(&format!("fn b{i}() {{ a{i}(); }}\n"));
            src.push_str(&format!("fn c{i}() {{ b{i}(); }}\n"));
            src.push_str(&format!("fn d{i}() {{ c{i}(); }}\n"));
        }
        fs::write(dir.join("fan.rs"), src).unwrap();
        index::run(&Ctx::new(&cx), &dir, false).unwrap();
        let key = index::canon(&dir);
        let mut cte: Vec<_> = cx.store.symbol_impact(&key, "sink", 4).unwrap();
        let mut bfs = impact_bfs(&Ctx::new(&cx), &key, "sink", 4).unwrap();
        cte.sort();
        bfs.sort();
        assert!(!cte.is_empty(), "fan-out-10 must reach sink");
        assert_eq!(cte, bfs, "query vs BFS");
        let _ = fs::remove_dir_all(dir);
    }

    /// T646: walks sharing one `WalkMemo` return what fresh walks return, and a name already
    /// visited by an earlier walk is answered from the memo, not the store.
    #[test]
    fn shared_walk_memo_keeps_rows_and_reuses_visited_names() {
        let (cx, dir) = cx("walk-memo");
        let mut src = String::from("fn sink() {}\n");
        for i in 0..4 {
            src.push_str(&format!("fn a{i}() {{ sink(); }}\n"));
            src.push_str(&format!("fn b{i}() {{ a{i}(); }}\n"));
            src.push_str(&format!("fn c{i}() {{ b{i}(); }}\n"));
        }
        fs::write(dir.join("fan.rs"), src).unwrap();
        let ctx = Ctx::new(&cx);
        index::run(&ctx, &dir, false).unwrap();
        let key = index::canon(&dir);
        let mut memo = WalkMemo::default();
        for name in ["sink", "a1", "b2", "sink"] {
            let shared = impact_walk_memo(&ctx, &[&key], name, 3, true, &mut memo).unwrap();
            let fresh = impact_walk_roots(&ctx, &[&key], name, 3, true).unwrap();
            assert_eq!(shared, fresh, "{name}");
        }
        let asked = memo.refs.len();
        assert!(asked > 1, "{asked}");
        impact_walk_memo(&ctx, &[&key], "a2", 3, true, &mut memo).unwrap();
        assert_eq!(memo.refs.len(), asked, "a2 was reached from sink");
        let _ = fs::remove_dir_all(dir);
    }

    /// T52.1: `path` keeps one subtree. One name defined in two files: the
    /// unfiltered answer lists both, the filtered one only the match, and a
    /// match-nothing filter names the scope in the empty answer.
    #[test]
    fn symbol_path_filter_keeps_one_file() {
        let (cx, dir) = cx("filter-path");
        fs::write(dir.join("a.rs"), "fn dup() {}\n").unwrap();
        fs::write(dir.join("b.rs"), "fn dup() {}\n").unwrap();
        let ctx = Ctx::new(&cx);
        let both = symbol_filtered(&ctx, &dir, "dup", &Filter::none()).unwrap();
        assert!(both.contains("a.rs") && both.contains("b.rs"), "{both}");
        let one = symbol_filtered(
            &ctx,
            &dir,
            "dup",
            &Filter {
                path: "b.rs".into(),
                kind: String::new(),
                ..Filter::none()
            },
        )
        .unwrap();
        assert!(one.contains("b.rs") && !one.contains("a.rs"), "{one}");
        assert_eq!(
            symbol_filtered(
                &ctx,
                &dir,
                "dup",
                &Filter {
                    path: "zzz".into(),
                    kind: String::new(),
                    ..Filter::none()
                },
            )
            .unwrap(),
            "no definition of dup in zzz"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// T329.36: one project, the tags index. The counts are the files and projects the answer
    /// lists, and a name with no listing adds nothing.
    #[test]
    fn single_project_counts_equal_the_listing() {
        let (cx, dir) = cx("t32936-single");
        fs::write(dir.join("a.rs"), "fn dup() {}\nfn go() { dup(); }\n").unwrap();
        fs::write(dir.join("b.rs"), "fn dup() {}\nfn also() { dup(); }\n").unwrap();
        let ctx = Ctx::new(&cx);
        let counted = |f: &dyn Fn() -> String| {
            tally::arm();
            let answer = f();
            (answer, tally::take())
        };
        let none = Filter::none();
        let (answer, got) = counted(&|| symbol_filtered(&ctx, &dir, "dup", &none).unwrap());
        assert_eq!(answer.matches("::dup#function@1").count(), 2, "{answer}");
        assert_eq!((got.symbols, got.files, got.projects), (1, 2, 1));
        let (answer, got) = counted(&|| callers_filtered(&ctx, &dir, "dup", &none).unwrap());
        assert_eq!(answer.matches("\u{d7}1").count(), 2, "{answer}");
        assert_eq!((got.symbols, got.files, got.projects), (1, 2, 1));
        let (_, got) = counted(&|| impact_filtered(&ctx, &dir, "dup", 1, &none, None).unwrap());
        assert_eq!((got.symbols, got.files, got.projects), (1, 2, 1));
        let (answer, got) = counted(&|| symbol_filtered(&ctx, &dir, "zzz", &none).unwrap());
        assert_eq!(answer, "no definition of zzz");
        assert_eq!((got.symbols, got.files, got.projects), (0, 0, 0));
        let _ = fs::remove_dir_all(dir);
    }

    /// T329.36: what the server counted before it failed is not part of the tags answer that
    /// replaces it.
    #[test]
    fn a_failed_server_leaves_no_count_behind() {
        let (mut c, dir) = crate::testutil::config("t32936-door");
        c.plugins.graph.backend = "lsp".into();
        let cx = crate::plugin::Runtime::open(c, "t32936-door").unwrap();
        let ctx = Ctx::new(&cx);
        tally::arm();
        let out = served(
            &ctx,
            &dir,
            "symbol",
            &["alpha"],
            || {
                tally::hit(&dir, "alpha", ["lost.rs"]);
                anyhow::bail!("lsp: server died")
            },
            || {
                tally::hit(&dir, "alpha", ["a.rs"]);
                Ok("tags answer".to_string())
            },
        )
        .unwrap();
        assert!(out.starts_with("(tags; lsp:"), "{out}");
        let got = tally::take();
        assert_eq!((got.symbols, got.files, got.projects), (1, 1, 1));
        let _ = fs::remove_dir_all(dir);
    }

    /// T52.1: `kind` keeps one definition kind. A struct and a function share a
    /// name (different namespaces, compiles); the filter keeps the asked kind.
    #[test]
    fn symbol_kind_filter_picks_struct_over_function() {
        let (cx, dir) = cx("filter-kind");
        fs::write(dir.join("k.rs"), "struct Shape;\nfn Shape() {}\n").unwrap();
        let ctx = Ctx::new(&cx);
        let out = symbol_filtered(
            &ctx,
            &dir,
            "Shape",
            &Filter {
                path: String::new(),
                kind: "struct".into(),
                ..Filter::none()
            },
        )
        .unwrap();
        assert!(out.contains("struct"), "{out}");
        assert!(!out.contains("fn Shape"), "{out}");
        let _ = fs::remove_dir_all(dir);
    }

    /// T52.1: `callers` with a path keeps the callers in that subtree only.
    #[test]
    fn callers_path_filter_keeps_one_subtree() {
        let (cx, dir) = cx("filter-callers");
        fs::write(
            dir.join("chain.rs"),
            "fn a() {\n    b();\n}\nfn b() {\n    c();\n}\nfn c() {}\n",
        )
        .unwrap();
        fs::write(dir.join("other.rs"), "fn d() {\n    c();\n    c();\n}\n").unwrap();
        let ctx = Ctx::new(&cx);
        let one = callers_filtered(
            &ctx,
            &dir,
            "c",
            &Filter {
                path: "other".into(),
                kind: String::new(),
                ..Filter::none()
            },
        )
        .unwrap();
        assert!(
            one.contains("other.rs") && !one.contains("chain.rs"),
            "{one}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// T52.1: `impact` with a path reports only the lines in that subtree.
    #[test]
    fn impact_path_filter_reports_matching_lines() {
        let (cx, dir) = cx("filter-impact");
        fs::write(
            dir.join("chain.rs"),
            "fn a() {\n    b();\n}\nfn b() {\n    c();\n}\nfn c() {}\n",
        )
        .unwrap();
        fs::write(dir.join("other.rs"), "fn d() {\n    c();\n    c();\n}\n").unwrap();
        let ctx = Ctx::new(&cx);
        let out = impact_filtered(
            &ctx,
            &dir,
            "c",
            2,
            &Filter {
                path: "other".into(),
                kind: String::new(),
                ..Filter::none()
            },
            None,
        )
        .unwrap();
        assert_eq!(out, "1  other.rs  d\n", "{out}");
        assert_eq!(
            impact_filtered(
                &ctx,
                &dir,
                "c",
                2,
                &Filter {
                    path: "zzz".into(),
                    kind: String::new(),
                    ..Filter::none()
                },
                None,
            )
            .unwrap(),
            "nothing reaches c in zzz"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// T8.15: with `auto_index = false` an edit is invisible until `rtok graph index`
    /// (which is `index::run`). The call itself opens no file.
    #[test]
    fn auto_index_false_is_stale_until_explicit_index() {
        let (mut cx, dir) = cx("noauto");
        cx.config.plugins.graph.auto_index = false;
        fs::write(dir.join("a.rs"), "fn alpha() {}\n").unwrap();
        let first = symbol(&Ctx::new(&cx), &dir, "alpha").unwrap();
        assert!(first.contains("a.rs:1"), "{first}");
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(dir.join("a.rs"), "// bump\nfn alpha() {}\n").unwrap();
        let stale = symbol(&Ctx::new(&cx), &dir, "alpha").unwrap();
        assert!(
            stale.starts_with("stale:") && stale.contains("a.rs"),
            "pending edit must carry a staleness banner: {stale}"
        );
        assert!(
            stale.contains("a.rs:1"),
            "must still report the old line: {stale}"
        );
        let r = index_for(&Ctx::new(&cx), &dir).unwrap();
        assert_eq!(r.read, 0, "the call must open no file");
        index::run(&Ctx::new(&cx), &dir, false).unwrap(); // what `rtok graph index` does
        let fresh = symbol(&Ctx::new(&cx), &dir, "alpha").unwrap();
        assert!(
            fresh.contains("a.rs:2"),
            "explicit index shows the new line: {fresh}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// T8.15: a root with no rows is still indexed once with `auto_index = false`.
    #[test]
    fn auto_index_false_empty_root_still_answers() {
        let (mut cx, dir) = cx("noauto-empty");
        cx.config.plugins.graph.auto_index = false;
        fs::write(dir.join("main.rs"), "fn main() {}\n").unwrap();
        let out = symbol(&Ctx::new(&cx), &dir, "main").unwrap();
        assert!(out.contains("main.rs:1"), "{out}");
        let _ = fs::remove_dir_all(dir);
    }

    /// Gate P8b: each description ≤ 60 tokens, the whole surface ≤ 170 (150 for the five tools of T68.1;
    /// `graph_diff`, T329.18, is the sixth at about 28, `graph_export`, T329.16, the seventh at about 10).
    #[test]
    fn graph_surface_is_seven_tools_under_budget() {
        let (cx, dir) = cx("surface");
        let tools = GraphPlugin.mcp_tools();
        assert_eq!(tools.len(), 7);
        let est = |d: &str| crate::tokens::estimate(d, Class::Prose, &cx.config.estimator);
        let n: u32 = tools.iter().map(|t| est(t.description)).sum();
        // stderr, not `println!`: the library denies print macros, and this gate is a count.
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr(),
            "graph surface: {} tools, {n} description tokens",
            tools.len()
        );
        assert!(n <= 170, "graph descriptions are {n} tokens");
        for t in &tools {
            assert!(
                est(t.description) <= 60,
                "{} description is {} tokens",
                t.name,
                est(t.description)
            );
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn outline_reuses_read_map() {
        let (cx, dir) = cx("outline");
        let out = outline(&Ctx::new(&cx), "src/main.rs").unwrap();
        assert!(out.contains("main"), "{out}");
        let _ = fs::remove_dir_all(dir);
    }

    /// T52.4: one truly-dead private fn is listed; pub API, trait-impl methods,
    /// `#[test]` fns, macro definitions and test-dir files are not.
    #[test]
    fn dead_lists_only_the_private_orphan() {
        let (cx, dir) = cx("dead");
        fs::write(
            dir.join("live.rs"),
            "pub fn caller() {\n    used();\n}\nfn used() {}\n",
        )
        .unwrap();
        fs::write(dir.join("dead.rs"), "fn orphan() {}\n").unwrap();
        fs::write(dir.join("api.rs"), "pub fn exported() {}\n").unwrap();
        fs::write(
            dir.join("traits.rs"),
            "struct S;\ntrait T {\n    fn m(&self);\n}\nimpl T for S {\n    fn m(&self) {}\n}\n",
        )
        .unwrap();
        fs::write(dir.join("mac.rs"), "macro_rules! gen {\n    () => {};\n}\n").unwrap();
        fs::write(dir.join("tested.rs"), "#[test]\nfn my_test() {}\n").unwrap();
        // Stacked attributes: the test marker is not the line right above the fn.
        fs::write(
            dir.join("param.rs"),
            "#[rstest]\n#[case(1)]\n#[case(2)]\n#[case(3)]\n#[case(4)]\nfn param_test(#[case] n: u8) {}\n\
             #[test]\n#[ignore]\n#[should_panic]\n#[cfg(unix)]\nfn stacked_test() {}\n\
             #[tokio::test]\nasync fn async_test() {}\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join("tests")).unwrap();
        fs::write(dir.join("tests/helper.rs"), "fn help_me() {}\n").unwrap();
        let out = dead(&Ctx::new(&cx), &dir).unwrap();
        assert!(out.contains("orphan"), "{out}");
        for kept in [
            "used",
            "caller",
            "exported",
            "m",
            "gen",
            "my_test",
            "param_test",
            "stacked_test",
            "async_test",
            "help_me",
            "T",
            "S",
        ] {
            assert!(
                !out.lines().any(|l| l.ends_with(&format!(" {kept}"))),
                "{kept} must not be listed as dead:\n{out}"
            );
        }
        let _ = fs::remove_dir_all(dir);
    }

    /// The guard fails before any language server is spawned, so no LSP binary is needed.
    #[test]
    fn lsp_outline_refuses_paths_outside_the_root() {
        let (mut c, dir) = crate::testutil::config("lsp-outside");
        c.plugins.graph.backend = "lsp".into();
        let cx = crate::plugin::Runtime::open(c, "lsp-outside").unwrap();
        for path in ["/etc/passwd", "../../../../../../etc/passwd"] {
            let err = outline(&Ctx::new(&cx), path).unwrap_err().to_string();
            assert!(err.contains("outside cwd"), "{path}: {err}");
        }
        let _ = fs::remove_dir_all(dir);
    }

    /// T376: a project with no language-server manifest makes `lsp::*` fail before any
    /// process is spawned (so the real PATH is never consulted); each of the five tools then
    /// answers from the tags index under the fallback prefix and records one `lsp_fallback`.
    #[test]
    fn lsp_backend_falls_back_to_tags_for_every_tool() {
        let (mut c, dir) = crate::testutil::config("t376-fallback");
        c.plugins.graph.backend = "lsp".into();
        let cx = crate::plugin::Runtime::open(c, "t376-fallback").unwrap();
        fs::write(
            dir.join("a.rs"),
            "pub fn alpha() {\n    beta();\n}\nfn beta() {}\n",
        )
        .unwrap();
        let ctx = Ctx::new(&cx);
        let f = Filter::none();
        let answers = [
            (
                "symbol",
                symbol_filtered(&ctx, &dir, "beta", &f).unwrap(),
                "beta",
            ),
            (
                "callers",
                callers_filtered(&ctx, &dir, "beta", &f).unwrap(),
                "a.rs",
            ),
            (
                "impact",
                impact_filtered(&ctx, &dir, "beta", 1, &f, None).unwrap(),
                "alpha",
            ),
            ("outline", outline_in(&ctx, &dir, "a.rs").unwrap(), "alpha"),
            (
                "explore",
                explore(&ctx, &dir, "beta", &f).unwrap(),
                "= beta",
            ),
        ];
        for (tool, out, needle) in &answers {
            assert!(
                out.starts_with("(tags; lsp: lsp: no Cargo.toml"),
                "{tool}: {out}"
            );
            assert!(out.contains(needle), "{tool}: {out}");
        }
        let rows = cx.store.list_measurements("graph").unwrap();
        let fallbacks = rows.iter().filter(|r| r.kind == "lsp_fallback").count();
        assert_eq!(fallbacks, 5, "{rows:?}");
        let _ = fs::remove_dir_all(dir);
    }

    /// The door with a probe that always finds the server, so the closures are the server.
    fn served(
        cx: &Ctx,
        root: &Path,
        tool: &'static str,
        names: &[&str],
        lsp: impl FnOnce() -> Result<String>,
        tags: impl FnOnce() -> Result<String>,
    ) -> Result<String> {
        door(cx, root, tool, names, |_| Ok(()), lsp, tags)
    }

    /// T376: an empty LSP answer falls back only for a name the tags index has, and the
    /// default backend never adds the prefix.
    #[test]
    fn lsp_empty_answer_falls_back_only_for_a_known_name() {
        let (mut c, dir) = crate::testutil::config("t376-empty");
        c.plugins.graph.backend = "lsp".into();
        let cx = crate::plugin::Runtime::open(c, "t376-empty").unwrap();
        fs::write(dir.join("a.rs"), "pub fn alpha() {}\n").unwrap();
        let ctx = Ctx::new(&cx);
        let tags = || Ok("tags answer".to_string());
        let known = served(
            &ctx,
            &dir,
            "symbol",
            &["alpha"],
            || Ok("no definition of alpha".into()),
            tags,
        )
        .unwrap();
        assert_eq!(known, "(tags; lsp: empty answer)\ntags answer");
        let unknown = served(
            &ctx,
            &dir,
            "symbol",
            &["zzz"],
            || Ok("no definition of zzz".into()),
            tags,
        )
        .unwrap();
        assert_eq!(unknown, "no definition of zzz");
        let real = served(
            &ctx,
            &dir,
            "symbol",
            &["alpha"],
            || Ok("a.rs:1 function".into()),
            tags,
        )
        .unwrap();
        assert_eq!(real, "a.rs:1 function");
        let (plain, dir2) = crate::testutil::runtime("t376-plain");
        let out = served(
            &Ctx::new(&plain),
            &dir2,
            "symbol",
            &["alpha"],
            || panic!("lsp"),
            tags,
        )
        .unwrap();
        assert_eq!(out, "tags answer");
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(dir2);
    }

    fn auto_runtime(tag: &str, by_language: &[(&str, &str)]) -> (crate::plugin::Runtime, PathBuf) {
        let (mut c, dir) = crate::testutil::config(tag);
        c.plugins.graph.backend = "auto".into();
        for (lang, mode) in by_language {
            c.plugins
                .graph
                .backend_by_language
                .insert((*lang).into(), (*mode).into());
        }
        (crate::plugin::Runtime::open(c, tag).unwrap(), dir)
    }

    fn kinds(cx: &crate::plugin::Runtime) -> Vec<String> {
        let mut kinds: Vec<String> = cx
            .store
            .list_measurements("graph")
            .unwrap()
            .into_iter()
            .map(|r| r.kind)
            .collect();
        kinds.sort();
        kinds
    }

    /// T329.9: a language with no server goes straight to tags, a server answer is headed
    /// `(lsp)`, and a server that is missing or died mid-session gives the tags answer with the
    /// reason. The fake closures stand in for the server, so none needs to be installed.
    #[test]
    fn auto_says_which_mode_answered_and_why_tags_did() {
        let (cx, dir) = auto_runtime("t3299-auto", &[]);
        let ctx = Ctx::new(&cx);
        let tags = || Ok("tags answer".to_string());
        // One project per question: a project's mode is checked once (T329.11).
        let ask = |n: usize, lsp: Result<String>| {
            let project = dir.join(format!("p{n}"));
            fs::create_dir_all(&project).unwrap();
            fs::write(project.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
            served(&ctx, &project, "symbol", &["alpha"], || lsp, tags)
        };
        // No marker file: nothing to ask, and the server closure is never run.
        let none = lsp_or_tags(&ctx, &dir, "symbol", &["alpha"], || panic!("lsp"), tags);
        assert_eq!(none.unwrap(), "(tags)\ntags answer");
        assert_eq!(
            ask(1, Ok("a.rs:1 function".into())).unwrap(),
            "(lsp)\na.rs:1 function"
        );
        assert_eq!(
            ask(2, Err(anyhow::anyhow!("lsp: rust-analyzer not on PATH"))).unwrap(),
            "(tags; lsp: rust-analyzer not on PATH)\ntags answer"
        );
        // A crash while a request is open: the notice names how the server ended.
        let died = ask(3, Err(anyhow::anyhow!("lsp: eof; exited signal: 9"))).unwrap();
        assert_eq!(died, "(tags; lsp: eof; exited signal: 9)\ntags answer");
        assert_eq!(
            kinds(&cx),
            [
                "lsp_fallback",
                "lsp_fallback",
                "tags.symbol",
                "tags.symbol",
                "tags.symbol"
            ]
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// T329.9: `backend_by_language` wins over `backend`, in both directions, and `lsp` keeps the
    /// T376 shape: no header on a server answer.
    #[test]
    fn a_language_override_picks_the_mode_of_its_projects() {
        let (cx, dir) = auto_runtime("t3299-by-lang", &[("rust", "tags")]);
        let ctx = Ctx::new(&cx);
        fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        assert_eq!(mode_of(&ctx, &dir), Mode::Tags);
        let out = served(
            &ctx,
            &dir,
            "symbol",
            &[],
            || panic!("lsp"),
            || Ok("tags answer".into()),
        );
        assert_eq!(out.unwrap(), "tags answer");
        assert!(kinds(&cx).is_empty());
        for (by, want) in [("lsp", Mode::Lsp), ("auto", Mode::Auto)] {
            let (cx, dir) = auto_runtime("t3299-by-lang-2", &[("rust", by)]);
            fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
            assert_eq!(mode_of(&Ctx::new(&cx), &dir), want);
            let _ = fs::remove_dir_all(dir);
        }
        let (mut c, dir2) = crate::testutil::config("t3299-lsp-plain");
        c.plugins.graph.backend = "lsp".into();
        let cx = crate::plugin::Runtime::open(c, "t3299-lsp-plain").unwrap();
        let ok = served(
            &Ctx::new(&cx),
            &dir2,
            "symbol",
            &[],
            || Ok("a.rs:1 function".into()),
            || panic!("tags"),
        );
        assert_eq!(ok.unwrap(), "a.rs:1 function");
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(dir2);
    }

    /// The language name comes from the marker file; a value the schema refuses reads as tags.
    #[test]
    fn mode_of_reads_the_language_and_distrusts_unknown_values() {
        let (cx, dir) = auto_runtime("t3299-mode", &[("go", "tags"), ("rust", "grep")]);
        let ctx = Ctx::new(&cx);
        assert_eq!(mode_of(&ctx, &dir), Mode::Auto);
        fs::write(dir.join("go.mod"), "module x\n").unwrap();
        assert_eq!(lsp::language_of(&dir), Some("go"));
        assert_eq!(mode_of(&ctx, &dir), Mode::Tags);
        fs::write(dir.join("Cargo.toml"), "").unwrap();
        assert_eq!(lsp::language_of(&dir), Some("rust"));
        assert_eq!(mode_of(&ctx, &dir), Mode::Tags);
        assert_eq!(without_mode_line("(tags; lsp: x)\nbody\n"), "body\n");
        assert_eq!(without_mode_line("(lsp)\nbody"), "body");
        assert_eq!(without_mode_line("(text)\nbody"), "body");
        assert_eq!(
            without_mode_line("(not a mode line)\n"),
            "(not a mode line)\n"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// T329.10: under `auto`, a project with no server and no file a grammar parses is answered by
    /// text search through every tool, headed `(text)`, and each answer leaves a `text.*` row. A
    /// project the tags index can parse keeps the `(tags)` answer, and `dead` makes no guess.
    #[test]
    fn auto_answers_a_language_with_no_grammar_from_text_search() {
        let (cx, dir) = auto_runtime("t32910-auto", &[]);
        fs::write(
            dir.join("app.rb"),
            "def greet(name)\n  puts name\nend\n\ndef main\n  greet('x')\nend\n",
        )
        .unwrap();
        let ctx = Ctx::new(&cx);
        let scope = [scope::Member {
            name: "web".into(),
            root: dir.clone(),
        }];
        let ask = |tool: &str, args: Value| call(&ctx, tool, &args, &scope).unwrap();
        let symbol = ask("symbol", json!({"name": "greet"}));
        assert!(
            symbol.starts_with("(text)\napp.rb:1 function\ndef greet(name)\n"),
            "{symbol}"
        );
        assert!(ask("callers", json!({"name": "greet"})).starts_with("(text)\napp.rb ×1 (L6)\n"));
        assert!(ask("impact", json!({"name": "greet"})).starts_with("(text)\n1  app.rb  ×1\n"));
        let explore = ask("explore", json!({"query": "greet"}));
        assert!(
            explore.starts_with("(text)\n= greet\napp.rb:1 function\n"),
            "{explore}"
        );
        let outline = ask("outline", json!({"path": "app.rb"}));
        assert!(
            outline.starts_with("(text)\n1 function greet\n5 function main\n"),
            "{outline}"
        );
        assert_eq!(
            kinds(&cx),
            [
                "text.callers",
                "text.explore",
                "text.impact",
                "text.outline",
                "text.symbol"
            ]
        );
        let dead = dead(&ctx, &dir).unwrap();
        assert!(dead.contains("dead: not available in text mode"), "{dead}");
        assert!(dead_rows(&ctx, &dir).is_err());
        let scoped = scope::dead(&ctx, &scope).unwrap();
        assert!(scoped.contains("not available in text mode"), "{scoped}");
        // Text search only wins where nothing parses the project.
        fs::write(dir.join("lib.rs"), "fn alpha() {}\n").unwrap();
        let tags = ask("symbol", json!({"name": "alpha"}));
        assert!(tags.starts_with("(tags)\n"), "{tags}");
        let _ = fs::remove_dir_all(dir);
    }

    /// T329.10: `backend = "text"` pins text search even where a grammar parses the project, and in a
    /// scope each project answers through its own mode, labelled.
    #[test]
    fn text_pin_and_a_scope_of_mixed_modes() {
        let (mut c, dir) = crate::testutil::config("t32910-mixed");
        c.plugins.graph.backend = "auto".into();
        c.plugins
            .graph
            .backend_by_language
            .insert("ruby".into(), "text".into());
        let cx = crate::plugin::Runtime::open(c, "t32910-mixed").unwrap();
        let (web, api) = (dir.join("web"), dir.join("api"));
        fs::create_dir_all(&web).unwrap();
        fs::create_dir_all(&api).unwrap();
        fs::write(web.join("Gemfile"), "").unwrap();
        fs::write(web.join("app.rb"), "def greet\nend\n").unwrap();
        fs::write(api.join("lib.rs"), "fn serve() {}\n").unwrap();
        assert_eq!(lsp::language_of(&web), Some("ruby"));
        let ctx = Ctx::new(&cx);
        assert_eq!(mode_of(&ctx, &web), Mode::Text);
        let scope = [
            scope::Member {
                name: "api".into(),
                root: api.clone(),
            },
            scope::Member {
                name: "web".into(),
                root: web.clone(),
            },
        ];
        let out = scope::symbol(&ctx, &scope, "greet", &Filter::none()).unwrap();
        assert!(
            out.starts_with("[web] (text)\napp.rb:1 function\n"),
            "{out}"
        );
        assert!(!out.contains("(tags)"), "{out}");
        let both = scope::symbol(&ctx, &scope, "serve", &Filter::none()).unwrap();
        assert!(both.starts_with("[api] (tags)\n"), "{both}");
        let dead = scope::dead(&ctx, &scope).unwrap();
        assert!(
            dead.contains("[web] dead: not available in text mode"),
            "{dead}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    // ---------- T68.1: explore ----------

    /// A fake backend pins the assembled answer's section format byte-exact without
    /// a store or a language server: `= name` + bodies, `paths:` (or `none`), then
    /// `impact:` counts — and `before` is exactly the bytes the replaced
    /// `symbol` + `impact` calls would have printed.
    struct FakeParts;

    impl ExploreParts for FakeParts {
        fn resolve(&mut self, token: &str) -> Result<Vec<String>> {
            Ok(match token {
                "aa" => vec!["aa".to_string()],
                "bb" => vec!["bb".to_string()],
                _ => Vec::new(),
            })
        }
        fn defs(&mut self, name: &str) -> Result<String> {
            Ok(format!("src/{name}.rs:1 function\nfn {name}() {{}}\n"))
        }
        fn paths(&mut self, a: &str, b: &str) -> Result<Vec<String>> {
            if a == "aa" && b == "bb" {
                return Ok(vec!["aa → cc → bb".to_string()]);
            }
            Ok(Vec::new())
        }
        fn impact1(&mut self, name: &str) -> Result<(String, usize)> {
            Ok((format!("nothing reaches {name}"), 0))
        }
        fn def_count(&mut self, _name: &str) -> Result<usize> {
            Ok(1)
        }
    }

    #[test]
    fn assemble_explore_formats_sections_and_counts_replaced_bytes() {
        let mut parts = FakeParts;
        let (out, before) = assemble_explore("aa bb zz", &Filter::none(), &mut parts).unwrap();
        assert_eq!(
            out,
            "= aa\nsrc/aa.rs:1 function\nfn aa() {}\n\
             = bb\nsrc/bb.rs:1 function\nfn bb() {}\n\
             paths:\naa → cc → bb\n\
             impact:\naa ← 0\nbb ← 0\n"
        );
        let replaced: u64 = [
            "src/aa.rs:1 function\nfn aa() {}\n",
            "src/bb.rs:1 function\nfn bb() {}\n",
            "nothing reaches aa",
            "nothing reaches bb",
        ]
        .iter()
        .map(|s| s.len() as u64)
        .sum();
        assert_eq!(before, replaced);
    }

    #[test]
    fn assemble_explore_prints_none_when_no_path_connects() {
        let (out, _) = assemble_explore("zz", &Filter::none(), &mut FakeParts).unwrap();
        assert_eq!(out, "no symbols resolved for \"zz\"");
        let (out, _) = assemble_explore("aa", &Filter::none(), &mut FakeParts).unwrap();
        assert!(out.contains("paths:\nnone\n"), "{out}");
        assert!(out.contains("impact:\naa ← 0\n"), "{out}");
    }

    #[test]
    fn explore_tokens_split_query_and_dedupe() {
        assert_eq!(
            explore_tokens("how do Foo_bar and foo-bar relate?"),
            vec![
                "how".to_string(),
                "do".to_string(),
                "Foo_bar".to_string(),
                "and".to_string(),
                "foo".to_string(),
                "bar".to_string(),
                "relate".to_string()
            ]
        );
    }

    /// T474: `truncated source lines` is not a symbol name. After the graph plugin is
    /// indexed, `explore` still includes `body_lines` from its name and doc comment.
    #[test]
    fn explore_truncated_source_lines_includes_body_lines() {
        let (cx, dir) = cx("explore-fts");
        let root = crate_root().join("src/plugins/graph");
        let out = explore(
            &Ctx::new(&cx),
            &root,
            "truncated source lines",
            &Filter::none(),
        )
        .unwrap();
        assert!(out.contains("body_lines"), "{out}");
        let _ = fs::remove_dir_all(dir);
    }

    /// T474: `id` selects one definition and ignores a different `name`.
    #[test]
    fn symbol_by_id_returns_one_definition() {
        let (cx, dir) = cx("symid");
        fs::write(dir.join("a.rs"), "fn dup() {}\n").unwrap();
        fs::write(dir.join("b.rs"), "fn dup() { let x = 1; }\n").unwrap();
        let ctx = Ctx::new(&cx);
        let out = symbol_by_id(&ctx, &dir, "b.rs::dup#function@1").unwrap();
        assert!(
            out.contains("b.rs::dup#function@1 b.rs:1 function"),
            "{out}"
        );
        assert!(out.contains("let x = 1"), "{out}");
        assert!(!out.contains("a.rs"), "{out}");
        assert_eq!(
            symbol_by_id(&ctx, &dir, "nope").unwrap(),
            "no definition of nope"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// T474: a body past the line budget archives the uncut span and points at it.
    #[test]
    fn long_body_archives_the_uncut_span() {
        let (cx, dir) = cx("longbody");
        let mut src = String::from("fn long() {\n");
        for i in 0..50 {
            src.push_str(&format!("    let v{i} = {i};\n"));
        }
        src.push_str("}\n");
        fs::write(dir.join("long.rs"), &src).unwrap();
        let ctx = Ctx::new(&cx);
        let out = symbol(&ctx, &dir, "long").unwrap();
        assert!(out.contains("expand "), "{out}");
        assert!(!out.contains("let v49"), "the head must stay cut: {out}");
        let shown = out
            .lines()
            .find_map(|l| l.split("expand ").nth(1))
            .unwrap()
            .trim();
        let id = printed_archive_id(&cx, shown);
        let archived = cx.store.get_archive(&id, None).unwrap().unwrap();
        let span = ctx
            .symbol_span(&index::canon(&dir), "long.rs", "long", "function", 1)
            .unwrap()
            .unwrap();
        let file = fs::read(dir.join("long.rs")).unwrap();
        let expect = &file[span.start_byte as usize..span.end_byte as usize];
        assert_eq!(archived, expect);
        assert!(String::from_utf8_lossy(&archived).contains("let v49"));
        let _ = fs::remove_dir_all(dir);
    }

    /// T474: a file whose sha no longer matches is not sliced.
    #[test]
    fn a_changed_file_prints_stale_and_is_not_sliced() {
        let (mut cx, dir) = cx("stale-span");
        fs::write(dir.join("a.rs"), "fn alpha() { let n = 1; }\n").unwrap();
        let first = symbol(&Ctx::new(&cx), &dir, "alpha").unwrap();
        assert!(first.contains("fn alpha()"), "{first}");
        cx.config.plugins.graph.auto_index = false;
        fs::write(
            dir.join("a.rs"),
            "/* shifted */\nfn alpha() { let n = 1; }\n",
        )
        .unwrap();
        let out = symbol(&Ctx::new(&cx), &dir, "alpha").unwrap();
        assert!(out.contains("stale a.rs\n"), "{out}");
        assert!(!out.contains("shifted"), "{out}");
        assert!(out.contains("a.rs:1 function"), "{out}");
        let _ = fs::remove_dir_all(dir);
    }

    /// End to end over the tags index: one question about `b` and `c` answers with
    /// both definition bodies, the only caller chain between them and both impact
    /// counts — byte for byte.
    #[test]
    fn explore_answers_a_two_symbol_question_byte_exact() {
        let (cx, dir) = cx("explore");
        fs::write(
            dir.join("chain.rs"),
            "fn a() {\n    b();\n}\nfn b() {\n    c();\n}\nfn c() {}\n",
        )
        .unwrap();
        fs::write(dir.join("other.rs"), "fn d() {\n    c();\n    c();\n}\n").unwrap();
        let out = explore(
            &Ctx::new(&cx),
            &dir,
            "how do b and c interact",
            &Filter::none(),
        )
        .unwrap();
        assert_eq!(
            out,
            "= b\nchain.rs::b#function@4 chain.rs:4 function\nfn b() {\n    c();\n}\ncalls: c\n\
             = c\nchain.rs::c#function@7 chain.rs:7 function\nfn c() {}\n\
             paths:\nc → b\n\
             impact:\nb ← 1\nc ← 2\n"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// Prefix resolution ranks by reference count: `alphabet` (referenced twice)
    /// lands before `alpha` (referenced once) for the token `alph`.
    #[test]
    fn explore_prefix_resolution_ranks_by_reference_count() {
        let (cx, dir) = cx("explore-prefix");
        fs::write(
            dir.join("p.rs"),
            "fn alpha() {}\nfn alphabet() {}\nfn user() {\n    alphabet();\n    alpha();\n    alphabet();\n}\n",
        )
        .unwrap();
        let out = explore(&Ctx::new(&cx), &dir, "alph", &Filter::none()).unwrap();
        let alphabet = out.find("= alphabet\n").expect("alphabet section");
        let alpha = out.find("= alpha\n").expect("alpha section");
        assert!(alphabet < alpha, "{out}");
        assert_eq!(
            explore(&Ctx::new(&cx), &dir, "zz zz", &Filter::none()).unwrap(),
            "no symbols resolved for \"zz zz\""
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// The `path` filter keeps the printed definitions in one subtree, same as
    /// `symbol`'s filter.
    #[test]
    fn explore_path_filter_keeps_one_subtree() {
        let (cx, dir) = cx("explore-path");
        fs::write(dir.join("a.rs"), "fn dup() {}\n").unwrap();
        fs::write(dir.join("b.rs"), "fn dup() {}\n").unwrap();
        let out = explore(
            &Ctx::new(&cx),
            &dir,
            "dup",
            &Filter {
                path: "b.rs".into(),
                kind: String::new(),
                ..Filter::none()
            },
        )
        .unwrap();
        assert!(out.contains("b.rs:1") && !out.contains("a.rs:1"), "{out}");
        let _ = fs::remove_dir_all(dir);
    }

    /// T68.4: `impact --to` prints call chains between two symbols on the fixture.
    #[test]
    fn impact_to_filters_call_chains() {
        let (cx, dir) = cx("impact-to");
        fs::write(
            dir.join("chain.rs"),
            "fn a() {
    b();
}
fn b() {
    c();
}
fn c() {}
",
        )
        .unwrap();
        let ctx = Ctx::new(&cx);
        assert_eq!(
            impact(&ctx, &dir, "a", 3, Some("c")).unwrap(),
            "a → b → c
"
        );
        assert_eq!(
            impact(&ctx, &dir, "a", 3, Some("b")).unwrap(),
            "a → b
"
        );
        assert_eq!(
            impact(&ctx, &dir, "b", 3, Some("a")).unwrap(),
            "no path from b to a within depth 3"
        );
        assert_eq!(
            impact(&ctx, &dir, "a", 1, Some("c")).unwrap(),
            "no path from a to c within depth 1"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// `symbol_paths` walks caller edges: chains read from the callee towards its
    /// callers, cycles terminate, and a name with no inbound chain answers empty.
    #[test]
    fn symbol_paths_walks_callers_and_terminates_on_cycles() {
        let (cx, dir) = cx("paths");
        fs::write(
            dir.join("chain.rs"),
            "fn a() {\n    b();\n}\nfn b() {\n    c();\n}\nfn c() {}\n",
        )
        .unwrap();
        fs::write(dir.join("other.rs"), "fn d() {\n    c();\n}\n").unwrap();
        index::run(&Ctx::new(&cx), &dir, false).unwrap();
        let key = index::canon(&dir);
        assert_eq!(
            cx.store.symbol_paths(&key, "c", "b", 3).unwrap(),
            vec!["c → b".to_string()]
        );
        assert!(cx.store.symbol_paths(&key, "b", "c", 3).unwrap().is_empty());
        assert_eq!(
            cx.store.symbol_paths(&key, "c", "a", 3).unwrap(),
            vec!["c → b → a".to_string()]
        );
        assert_eq!(
            cx.store.symbol_paths(&key, "c", "d", 3).unwrap(),
            vec!["c → d".to_string()]
        );
        fs::write(
            dir.join("cyc.rs"),
            "fn e() {\n    f();\n    g();\n}\nfn f() {\n    e();\n}\nfn g() {}\n",
        )
        .unwrap();
        index::run(&Ctx::new(&cx), &dir, false).unwrap();
        assert_eq!(
            cx.store.symbol_paths(&key, "e", "f", 3).unwrap(),
            vec!["e → f".to_string()]
        );
        assert!(
            cx.store.symbol_paths(&key, "f", "g", 3).unwrap().is_empty(),
            "e → f cycles; g is never a caller here"
        );
        let _ = fs::remove_dir_all(dir);
    }

    fn affected_fixture(tag: &str) -> (crate::plugin::Runtime, PathBuf) {
        let (cx, dir) = cx(tag);
        fs::create_dir_all(dir.join("tests")).unwrap();
        fs::write(
            dir.join("lib.rs"),
            "fn add() {
}
",
        )
        .unwrap();
        fs::write(
            dir.join("other.rs"),
            "fn other() {
}
",
        )
        .unwrap();
        fs::write(
            dir.join("tests/add.rs"),
            "fn test_add() {
    add();
}
",
        )
        .unwrap();
        fs::write(
            dir.join("tests/other.rs"),
            "fn test_other() {
    other();
}
",
        )
        .unwrap();
        (cx, dir)
    }

    /// T68.5: two tests, only the one that calls the changed symbol is listed.
    #[test]
    fn affected_two_tests_one_reaches_the_change() {
        let (cx, dir) = affected_fixture("affected");
        let ctx = Ctx::new(&cx);
        let out = affected_from_paths(&ctx, &dir, &["lib.rs".into()], 3, false).unwrap();
        assert!(out.contains("tests/add.rs ← via test_add"), "{out}");
        assert!(out.contains("cargo test test_add"), "{out}");
        assert!(!out.contains("test_other"), "{out}");
        assert_eq!(cx.store.measurement_count("graph").unwrap(), 0);
        let js = affected_from_paths(&ctx, &dir, &["lib.rs".into()], 3, true).unwrap();
        assert!(js.contains("tests/add.rs"), "{js}");
        assert_eq!(
            affected_from_paths(&ctx, &dir, &["nope.rs".into()], 3, false).unwrap(),
            EMPTY_AFFECTED
        );
        assert_eq!(test_command("t.py", "n").as_deref(), Some("pytest t.py::n"));
        assert_eq!(test_command("t.go", "N").as_deref(), Some("go test -run N"));
        assert_eq!(test_command("t.ts", "n").as_deref(), Some("vitest t.ts"));
        let _ = fs::remove_dir_all(dir);
    }

    /// T372: name-convention links list a test that never imports the source.
    #[test]
    fn affected_links_tests_by_naming_convention() {
        let (cx, dir) = cx("affected-by-name");
        fs::create_dir_all(dir.join("tests")).unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        // Source with a sibling name-linked test that does not call it.
        fs::write(dir.join("src/foo.rs"), "pub fn foo() {}\n").unwrap();
        fs::write(
            dir.join("tests/foo.rs"),
            "fn test_foo_name_only() {\n    let _ = 1;\n}\n",
        )
        .unwrap();
        // Negative: no matching test file for this source.
        fs::write(dir.join("src/lonely.rs"), "pub fn lonely() {}\n").unwrap();
        let ctx = Ctx::new(&cx);
        let out = affected_from_paths(&ctx, &dir, &["src/foo.rs".into()], 3, false).unwrap();
        assert!(
            out.contains("tests/foo.rs ← via (by name)"),
            "rust name link missing: {out}"
        );
        let out = affected_from_paths(&ctx, &dir, &["src/lonely.rs".into()], 3, false).unwrap();
        assert!(
            !out.contains("(by name)"),
            "lonely source must not invent a name link: {out}"
        );
        // Table-driven candidates for the four language conventions plus a negative.
        assert_eq!(
            name_linked_tests("src/foo.rs"),
            vec!["tests/foo.rs".to_string(), "src/foo_test.rs".to_string()]
        );
        assert_eq!(
            name_linked_tests("src/foo.ts"),
            vec![
                "src/foo.test.ts".to_string(),
                "src/foo.spec.ts".to_string(),
                "src/__tests__/foo.ts".to_string(),
            ]
        );
        assert_eq!(
            name_linked_tests("pkg/foo.py"),
            vec!["pkg/test_foo.py".to_string(), "pkg/foo_test.py".to_string()]
        );
        assert_eq!(
            name_linked_tests("pkg/foo.go"),
            vec!["pkg/foo_test.go".to_string()]
        );
        assert!(name_linked_tests("readme.md").is_empty());
        assert!(is_test_path("src/foo.test.ts"));
        assert!(is_test_path("src/foo.spec.ts"));
        assert!(is_test_path("src/__tests__/foo.ts"));
        assert!(is_test_path("src/test_foo.py"));
        assert!(is_test_path("src/foo_test.go"));
        let _ = fs::remove_dir_all(dir);
    }

    /// T68.5: changed files come from `git diff --name-only`, not a library.
    #[test]
    fn affected_reads_git_diff_name_only() {
        let (cx, dir) = affected_fixture("affected-git");
        let git = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .current_dir(&dir)
                    .env("GIT_AUTHOR_NAME", "t")
                    .env("GIT_AUTHOR_EMAIL", "t@t")
                    .env("GIT_COMMITTER_NAME", "t")
                    .env("GIT_COMMITTER_EMAIL", "t@t")
                    .args(args)
                    .status()
                    .unwrap()
                    .success(),
                "{args:?}"
            );
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);
        fs::write(
            dir.join("lib.rs"),
            "fn add() {
    let _ = 1;
}
",
        )
        .unwrap();
        let out = affected(&Ctx::new(&cx), &dir, None, false, false).unwrap();
        assert!(out.contains("tests/add.rs ← via test_add"), "{out}");
        assert!(!out.contains("test_other"), "{out}");
        let _ = fs::remove_dir_all(dir);
    }

    /// T68.6: a test that only `use`s the changed name is reached; imports are not refs.
    #[test]
    fn import_edge_reaches_import_only_test_and_is_not_a_reference() {
        let (cx, dir) = cx("import-edge");
        fs::create_dir_all(dir.join("tests")).unwrap();
        fs::write(dir.join("lib.rs"), "fn add() {}\n").unwrap();
        fs::write(
            dir.join("tests/only.rs"),
            "use crate::add;\nfn test_via_import() {}\n",
        )
        .unwrap();
        let ctx = Ctx::new(&cx);
        index::run(&ctx, &dir, false).unwrap();
        let key = index::canon(&dir);
        let names: Vec<_> = cx
            .store
            .symbol_imports(&key, "tests/only.rs")
            .unwrap()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert!(names.contains(&"add".to_string()), "{names:?}");
        let importers: Vec<_> = cx
            .store
            .symbol_importers(&key, "add")
            .unwrap()
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        assert_eq!(importers, vec!["tests/only.rs".to_string()]);
        assert!(
            cx.store.symbol_refs(&key, "add").unwrap().is_empty(),
            "imports must not count as references"
        );
        let out = affected_from_paths(&ctx, &dir, &["lib.rs".into()], 3, false).unwrap();
        assert!(out.contains("tests/only.rs ← via test_via_import"), "{out}");
        let mut cte = cx.store.symbol_impact(&key, "add", 1).unwrap();
        let mut bfs = impact_bfs(&ctx, &key, "add", 1).unwrap();
        cte.sort();
        bfs.sort();
        assert_eq!(cte, bfs, "query vs BFS with imports");
        assert!(
            impact_bfs_follow(&ctx, &key, "add", 1, false)
                .unwrap()
                .is_empty(),
            "follow_imports=false must not walk the import"
        );
        let _ = fs::remove_dir_all(dir);
    }

    fn seed_map(rt: &crate::plugin::Runtime, dir: &Path) {
        let root = index::canon(dir);
        rt.store
            .replace_symbols(
                &root,
                "a.rs",
                "s",
                (0, 0),
                &[
                    rtok_plugin_sdk::SymbolRow::new("hot", "function", 1, true, 1, ""),
                    rtok_plugin_sdk::SymbolRow::new("mid", "function", 2, true, 2, ""),
                    rtok_plugin_sdk::SymbolRow::new("cold", "function", 3, true, 3, ""),
                    rtok_plugin_sdk::SymbolRow::new("hot", "function", 10, false, 10, ""),
                    rtok_plugin_sdk::SymbolRow::new("hot", "function", 11, false, 11, ""),
                    rtok_plugin_sdk::SymbolRow::new("mid", "function", 12, false, 12, ""),
                ],
            )
            .unwrap();
    }

    fn start(rt: &crate::plugin::Runtime) -> Option<Injection> {
        GraphPlugin.session_start(&SessionStart { source: "startup" }, &Ctx::new(rt))
    }

    #[test]
    fn repo_map_off_by_default_and_empty_index() {
        let (mut rt, dir) = cx("map-off");
        rt.cwd = Some(dir.to_string_lossy().into_owned());
        seed_map(&rt, &dir);
        assert!(start(&rt).is_none(), "map_tokens=0 must not inject");
        rt.config.plugins.graph.map_tokens = 200;
        let (empty, empty_dir) = cx("map-empty");
        let mut empty = empty;
        empty.config.plugins.graph.map_tokens = 200;
        empty.cwd = Some(empty_dir.to_string_lossy().into_owned());
        assert!(start(&empty).is_none(), "empty index must not inject");
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(empty_dir);
    }

    /// T370: `map_rank = "pagerank"` lists files from the stored graph, falls back to the
    /// reference counts when none is stored, and is byte-stable.
    #[test]
    fn repo_map_pagerank_lists_files_and_falls_back_without_a_graph() {
        let (mut rt, dir) = cx("map-rank");
        rt.cwd = Some(dir.to_string_lossy().into_owned());
        rt.config.plugins.graph.map_tokens = 2000;
        rt.config.plugins.graph.map_rank = "pagerank".into();
        seed_map(&rt, &dir);
        let fallback = start(&rt).expect("refs fallback");
        assert!(fallback.text.contains("hot a.rs:1 2"), "{}", fallback.text);
        rank::refresh(&Ctx::new(&rt), &index::canon(&dir)).unwrap();
        let ranked = start(&rt).expect("pagerank map");
        assert_eq!(ranked.text, "repo map\na.rs: hot, mid");
        assert_eq!(start(&rt).unwrap(), ranked);
        assert_eq!(ranked.priority, 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn repo_map_ranked_byte_stable_and_trimmed() {
        let (mut rt, dir) = cx("map-on");
        rt.cwd = Some(dir.to_string_lossy().into_owned());
        rt.config.plugins.graph.map_tokens = 2000;
        rt.config.plugins.graph.map_rank = "refs".into();
        seed_map(&rt, &dir);
        let a = start(&rt).expect("map on");
        let b = start(&rt).expect("map on again");
        assert_eq!(a, b);
        assert_eq!(a.priority, 1);
        let lines: Vec<_> = a.text.lines().collect();
        assert_eq!(lines[0], "repo map");
        assert!(lines[1].starts_with("hot a.rs:1 2"), "{lines:?}");
        assert!(lines[2].starts_with("mid a.rs:2 1"), "{lines:?}");
        assert!(lines[3].starts_with("cold a.rs:3 0"), "{lines:?}");
        let full = rt.estimate(&a.text, Class::Prose);
        rt.config.plugins.graph.map_tokens = full.saturating_sub(1).max(1);
        let trimmed = start(&rt).expect("trimmed map");
        assert!(
            trimmed.text.lines().count() < a.text.lines().count(),
            "cap must drop a line\nfull={}\ntrim={}",
            a.text,
            trimmed.text
        );
        let _ = fs::remove_dir_all(dir);
    }
}
