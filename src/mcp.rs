// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok mcp` — rmcp JSON-RPC over stdio (plan T4.1); `http` serves the same `Server` over
//! Streamable HTTP (T401).
//!
//! A one-shot `tools/list` (the Check) is accepted without `initialize`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::{BufRead, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, JsonObject, ListToolsResult, ProtocolVersion,
    ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
};
use serde_json::{Value, json};

#[cfg(feature = "cmd")]
pub mod wrap;

pub mod ping;

pub mod http;

mod agents;
mod messages;
mod tasks;
mod worktrees;

use crate::agents::link;
use crate::config::Config;
use crate::plugin::{Runtime, ToolDef};
use crate::plugins::Registry;
use crate::tokens::Class;

fn expand_def() -> ToolDef {
    ToolDef {
        name: "expand",
        description: "Return archived payload by id; optional lines a-b, regex grep (hits as N:line), context N.",
        input_schema: json!({"type":"object","properties":{"id":{"type":"string"},"lines":{"type":"string"},"grep":{"type":"string"},"context":{"type":"integer"}},"required":["id"]}),
    }
}

fn ping_def() -> ToolDef {
    ToolDef {
        name: "ping",
        description: "Alive check. Returns MCP <agent> жив for the host display name.",
        input_schema: json!({"type":"object","properties":{"agent":{"type":"string"}},"required":["agent"]}),
    }
}

fn whoami_def() -> ToolDef {
    ToolDef {
        name: "whoami",
        description: "This session's rtok agent: id, short id, host, host session, cwd, how it was linked, claimed worktrees. An error when no agent is linked yet.",
        input_schema: json!({"type":"object","properties":{}}),
    }
}

/// Serve MCP on stdin/stdout until EOF.
#[cfg_attr(not(feature = "graph"), allow(unused_variables))]
pub fn run(cfg: &Config) -> Result<()> {
    crate::agents::ensure_hook_client_link_here();
    let server = Server::new(cfg)?;
    // Background, own connection: housekeeping must neither delay `initialize` nor die on a
    // contended store (T75, T352).
    crate::store::spawn_retention(cfg, "mcp");
    crate::otel::export::spawn_ticker(cfg);
    // P8d watcher (T8.16): a thread inside this process, never a second writer.
    // Any value but `off` arms the notify backend.
    let watch_root: Option<std::path::PathBuf> =
        if server.cx.config.plugins.graph.watch.as_str() != "off" {
            std::env::current_dir().ok()
        } else {
            None
        };
    let stop = AtomicBool::new(false);
    std::thread::scope(|s| {
        // T329.5: the project's scope is watched, not just its root.
        #[cfg(feature = "graph")]
        if let Some(root) = &watch_root {
            s.spawn(|| crate::plugins::graph::watch::run_scope(&server.cx, root, &stop));
        }
        // T329.17: the health check of the project's scope; the only code that re-probes.
        #[cfg(feature = "graph")]
        if let Ok(root) = std::env::current_dir() {
            let (cx, stop) = (&server.cx, &stop);
            s.spawn(move || crate::plugins::graph::health::run(cx, Some(&root), stop));
        }
        // T329.8: register, link and index what the project's manifests reference, off the
        // request path so `initialize` and the first tool call are not delayed. Detached with its
        // own connection: EOF must not wait for up to `max_auto_projects` indexes, and a cut-off
        // index is only pending files the next run picks up.
        #[cfg(feature = "graph")]
        if let Ok(root) = std::env::current_dir() {
            let cfg = cfg.clone();
            std::thread::spawn(move || {
                let Ok(cx) = crate::plugin::Runtime::open(cfg, "graph-refs") else {
                    return;
                };
                let followed = crate::plugins::graph::follow::refresh(&cx, &root);
                crate::plugins::graph::follow::index_new(&cx, &followed);
            });
        }
        let res: Result<()> = (|| {
            let mut stdin = std::io::stdin().lock();
            let mut stdout = std::io::stdout();
            let mut buf = Vec::new();
            while let Some(line) = next_line(&mut stdin, &mut buf, MAX_LINE)? {
                if line.trim().is_empty() {
                    continue;
                }
                if let Some(out) = server.handle_line(&line) {
                    writeln!(stdout, "{out}")?;
                    stdout.flush()?;
                }
            }
            Ok(())
        })();
        stop.store(true, Ordering::Relaxed);
        crate::otel::export::flush_blocking(&server.cx);
        #[cfg(feature = "graph")]
        crate::plugins::graph::lsp::shutdown();
        res
    })
}

/// Guards a one-shot `--call` the way `run`'s stdin-EOF path guards a served session: drops
/// the cached LSP child (T142) on every exit — success, `Err`, or an early `?` — since `call`
/// has no end-of-loop point of its own to shut it down at.
#[cfg(feature = "graph")]
struct LspGuard;

#[cfg(feature = "graph")]
impl Drop for LspGuard {
    fn drop(&mut self) {
        crate::plugins::graph::lsp::shutdown();
    }
}

/// One-shot `tools/call` for hosts that cannot speak MCP (`rtok mcp --call`, T70.3).
pub fn call(cfg: &Config, name: &str, args: &Value) -> Result<String> {
    #[cfg(feature = "graph")]
    let _lsp_guard = LspGuard;
    let server = Server::new(cfg)?;
    if !server.allows(name) {
        return Err(unknown_tool(name));
    }
    let found = server.listed.iter().find(|t| t.def.name == name);
    let plugin = found.map(|t| t.plugin).unwrap_or("archive");
    let args = prepare_args(name, args);
    // Same required-field gate as `call_tool` (T213): a missing argument must not reach
    // `invoke` and become a handler-level default.
    let (text, ok) =
        if let Some(msg) = found.and_then(|t| missing_required(&t.def.input_schema, &args)) {
            (format!("invalid params: {msg}"), false)
        } else {
            server.invoke_text(name, &args)
        };
    let _ = record(&server.cx, plugin, name, &args, &text);
    if ok { Ok(text) } else { bail!("{text}") }
}

/// Runs `invoke` and maps its `Result` to `(text, ok)` — the one place that happens, so
/// `tools/call` (`call_tool`) and the one-shot `--call` (`call` above) read a tool failure
/// identically instead of keeping two copies of the same match arms in sync by hand.
/// `anyhow::Error`'s `Display` (`{e}`, not the chained `{e:#}`) is always the bare message
/// with no prefix of its own (T172: a failure must not read `Error: Error: …`), so nothing
/// here needs to guard against doubling one up.
fn invoke_text(cx: &Runtime, name: &str, args: &Value) -> (String, bool) {
    match invoke(cx, name, args) {
        Ok(t) => (t, true),
        Err(e) => (e.to_string(), false),
    }
}

/// Longest request line kept in memory. Tool arguments are notes and paths, far below this.
const MAX_LINE: u64 = 8 << 20;

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

/// T263: the first `file://` root of a `roots/list` answer becomes the cwd, so every tool's
/// `current_dir()` is the project even when the host launched us in `/` (Claude.app).
/// Anything else keeps the launch cwd. T351: every `file://` directory root, the first
/// included, is also an allowed root of the path guard.
fn apply_roots_response(result: &Value) {
    let paths: Vec<std::path::PathBuf> = result["roots"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["uri"].as_str())
        .filter(|uri| uri.starts_with("file://"))
        .filter_map(|uri| url::Url::parse(uri).ok())
        .filter_map(|url| url.to_file_path().ok())
        .collect();
    if let Some(first) = paths.first()
        && first.is_dir()
    {
        let _ = std::env::set_current_dir(first);
    }
    #[cfg(feature = "read")]
    crate::plugins::read::roots::set_client_roots(
        paths.into_iter().filter(|p| p.is_dir()).collect(),
    );
}

/// Protocol dialects `rtok mcp` has been built and tested against, oldest first. Per the
/// MCP lifecycle spec's version negotiation
/// (https://modelcontextprotocol.io/specification — "Initialization"): a server answers
/// `initialize` with the client's requested `protocolVersion` when it supports that
/// version, else the latest version it does support. `initialize` used to ignore the
/// request entirely and answer with `ProtocolVersion::default()` — whatever `rmcp` itself
/// considers current — so a dependency bump could silently change the advertised dialect
/// with no test to catch it (T213); the last entry here is that pinned fallback instead.
const SUPPORTED_PROTOCOL_VERSIONS: &[ProtocolVersion] = &[
    ProtocolVersion::V_2024_11_05,
    ProtocolVersion::V_2025_03_26,
    ProtocolVersion::V_2025_06_18,
];

/// The `initialize` result, the one place it is built: stdio (`handle_value`) and HTTP
/// (`http::Http::get_info`) must announce the same name, version and capabilities.
fn server_info(version: ProtocolVersion) -> ServerConfig {
    // Default `Implementation` still comes from rmcp's build env (`name: "rmcp"`).
    // 3.x types are non_exhaustive; construct via the public builders.
    ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
        .with_server_info(Implementation::new("rtok", env!("CARGO_PKG_VERSION")))
        .with_protocol_version(version)
}

fn negotiate_protocol_version(requested: &Value) -> ProtocolVersion {
    if let Ok(v) = serde_json::from_value::<ProtocolVersion>(requested.clone())
        && SUPPORTED_PROTOCOL_VERSIONS.contains(&v)
    {
        return v;
    }
    SUPPORTED_PROTOCOL_VERSIONS
        .last()
        .cloned()
        .unwrap_or(ProtocolVersion::LATEST)
}

/// A missing required argument used to become a handler-level default instead of a visible
/// error — `mem_save` without `body` stored an empty note, a missing `expand` `id` came
/// back as `"unknown archive id: "`, `handoff` silently ran with a default budget, turning
/// a client's bug into corrupt or misleading data (T213). JSON-RPC 2.0 has a dedicated code
/// for this, `-32602` "Invalid params" (https://www.jsonrpc.org/specification#error_object);
/// `rtok mcp` reports it the way it already reports `unknown tool` / `unknown archive id` —
/// an `isError` `CallToolResult` carrying the message, not a protocol-level error object —
/// so every client sees it whether or not it inspects JSON-RPC error codes. The `required`
/// list lives once, on each tool's own `input_schema` (`mcp_tools()`); this reads that
/// instead of hand-maintaining a second, driftable copy per tool.
fn missing_required(schema: &Value, args: &Value) -> Option<String> {
    let required = schema.get("required")?.as_array()?;
    let props = schema.get("properties");
    for field in required.iter().filter_map(Value::as_str) {
        let kind = props
            .and_then(|p| p.get(field))
            .and_then(|p| p.get("type"))
            .and_then(Value::as_str);
        let present = match args.get(field) {
            None | Some(Value::Null) => false,
            Some(v) => match kind {
                Some("string") => v.as_str().is_some_and(|s| !s.is_empty()),
                Some("integer") => v.is_i64() || v.is_u64(),
                Some("boolean") => v.is_boolean(),
                _ => true,
            },
        };
        if !present {
            // Name what the call did send: a misspelt key (`query` for `pattern`) is then
            // visible in the error instead of a guess (T353).
            let got = args.as_object().map_or_else(String::new, |o| {
                o.keys().map(String::as_str).collect::<Vec<_>>().join(", ")
            });
            let got = if got.is_empty() { "no arguments" } else { &got };
            return Some(format!("missing `{field}` (got: {got})"));
        }
    }
    None
}

/// The arguments a tool call is validated and run with — the one place they are normalised
/// (`call_tool` and the one-shot `call` both go through it): `null` becomes `{}`, and
/// `search`'s `query` is accepted as `pattern` — the key agents most often send it under
/// (T353). The advertised schema keeps `pattern` only; an alias is tolerated, not promoted.
fn prepare_args(name: &str, args: &Value) -> Value {
    let mut args = if args.is_null() {
        json!({})
    } else {
        args.clone()
    };
    if name == "search"
        && args.get("pattern").is_none_or(Value::is_null)
        && let Some(obj) = args.as_object_mut()
        && let Some(q) = obj.remove("query")
    {
        obj.insert("pattern".into(), q);
    }
    if name == "docs_query"
        && args.get("query").is_none_or(Value::is_null)
        && let Some(obj) = args.as_object_mut()
    {
        let alias = obj.remove("question").or_else(|| obj.remove("userQuery"));
        if let Some(q) = alias {
            obj.insert("query".into(), q);
        }
    }
    args
}

/// The one place `"unknown tool: <name>"` is worded — a name `invoke` never heard of and a
/// name the allow-list dropped (T192) must read identically, so both paths call this instead
/// of formatting the string a second time.
fn unknown_tool(name: &str) -> anyhow::Error {
    anyhow::anyhow!("unknown tool: {name}")
}

/// The next request line, `None` at EOF. `lines()` ended the server on one non-UTF-8 byte (its
/// `Err` went up through `?`) and buffered a line of any length first. Now bad bytes become
/// U+FFFD and fail JSON parsing like any junk line, and a line over `max` answers `-32700`
/// (the rest of it is drained unbuffered, and the truncated `"{"` stub fails parsing downstream
/// instead of the loop skipping it as empty).
fn next_line(r: &mut impl BufRead, buf: &mut Vec<u8>, max: u64) -> std::io::Result<Option<String>> {
    buf.clear();
    if (&mut *r).take(max + 1).read_until(b'\n', buf)? == 0 {
        return Ok(None);
    }
    if buf.len() as u64 > max {
        if buf.last() != Some(&b'\n') {
            r.skip_until(b'\n')?;
        }
        return Ok(Some("{".to_owned()));
    }
    Ok(Some(String::from_utf8_lossy(buf).into_owned()))
}

struct Listed {
    plugin: &'static str,
    def: ToolDef,
}

struct Server {
    cx: Runtime,
    listed: Vec<Listed>,
    /// T263: `initialize` declared `capabilities.roots`.
    roots_capable: AtomicBool,
    /// T283.1: the agent this process serves, cached once a stable rule links it (`Env`, `Own`;
    /// see `link::Rule::is_stable`). Hooks may register the row after the host started this
    /// process, so an unlinked or cwd-linked state is re-evaluated on every call.
    agent: Mutex<Option<(String, link::Rule)>>,
    /// Unix seconds this process started: a cwd candidate must have been seen since.
    started: i64,
}

impl Server {
    fn new(cfg: &Config) -> Result<Self> {
        // One session per process: `read_cache` rows are keyed by session and never expire, so
        // the literal "mcp" made every `rtok mcp` process answer `unchanged since <sha>` for a
        // file only another conversation had read. The surface stays "mcp" (see `record`).
        let cx = Runtime::open(cfg.clone(), format!("mcp-{}", std::process::id()))?;
        // T351: sibling worktrees and client roots pass the path guard in this process only.
        #[cfg(feature = "read")]
        crate::plugins::read::roots::enable();
        let mut listed = vec![
            Listed {
                plugin: "archive",
                def: expand_def(),
            },
            // Stays listed when `[mcp] tools` is narrowed, same as `expand`: `rtok mcp ping`
            // has to reach this tool or the liveness check can only fail.
            Listed {
                plugin: "mcp",
                def: ping_def(),
            },
            // The agent link is the process's own state, so like `ping` it is not a plugin tool.
            Listed {
                plugin: "mcp",
                def: whoami_def(),
            },
            // T285: the worktree tools act for the linked agent, so like `whoami` they are
            // this process's own, not a plugin's; unlike it, `[mcp] tools` can narrow them.
            Listed {
                plugin: "mcp",
                def: worktrees::add_def(),
            },
            Listed {
                plugin: "mcp",
                def: worktrees::list_def(),
            },
            Listed {
                plugin: "mcp",
                def: worktrees::remove_def(),
            },
            Listed {
                plugin: "mcp",
                def: worktrees::adopt_def(),
            },
            Listed {
                plugin: "mcp",
                def: messages::send_def(),
            },
            Listed {
                plugin: "mcp",
                def: messages::inbox_def(),
            },
            Listed {
                plugin: "mcp",
                def: agents::list_def(),
            },
            Listed {
                plugin: "mcp",
                def: agents::show_def(),
            },
            Listed {
                plugin: "mcp",
                def: agents::status_def(),
            },
        ];
        // T441.6: the project's tasks, same functions as `rtok task …`.
        listed.extend(
            tasks::defs()
                .into_iter()
                .map(|def| Listed { plugin: "mcp", def }),
        );
        let builtin: Vec<&str> = crate::plugins::all()
            .iter()
            .map(|p| p.manifest().id)
            .collect();
        for p in Registry::new(cfg).enabled() {
            let id = p.manifest().id;
            // Out-of-tree (WASM) plugins have no `tools/call` arm in `invoke` yet, so listing
            // their tools only bought callers an `unknown tool` error.
            if !builtin.contains(&id) {
                continue;
            }
            for def in p.mcp_tools() {
                if listed.iter().any(|t| t.def.name == def.name) {
                    continue;
                }
                listed.push(Listed { plugin: id, def });
            }
        }
        // T410: with worktrees off the tools are not offered at all, rather than offered and
        // refused, so a model never plans around them.
        if !cfg.worktree.enabled {
            listed.retain(|t| !t.def.name.starts_with("worktree_"));
        }
        if !cfg.mcp.tools.is_empty() {
            // `expand` stays listed whatever the allow-list says: D4 losslessness.
            listed.retain(|t| {
                t.def.name == "expand"
                    || t.def.name == "ping"
                    || t.def.name == "whoami"
                    || cfg.mcp.tools.iter().any(|n| n.as_str() == t.def.name)
            });
        }
        Ok(Self {
            cx,
            listed,
            roots_capable: AtomicBool::new(false),
            agent: Mutex::new(None),
            started: crate::log::now() as i64,
        })
    }

    /// The agent this process serves: the cached link, else one attempt at the rules of
    /// [`link::resolve`]. A store error or a disabled registry reads as not linked, never as a
    /// failed request (fail open).
    fn link(&self) -> link::Link {
        if let Some((id, rule)) = self.agent.lock().ok().and_then(|g| g.clone()) {
            return link::Link::Linked { id, rule };
        }
        let cx = &self.cx;
        let (Some(host_id), true) = (cx.host_id(), cx.config.agents.enabled) else {
            return link::Link::None;
        };
        let cwd = std::env::current_dir().ok();
        let who = link::Caller {
            host: &cx.config.hook.host,
            host_id,
            cwd: cwd.as_deref().and_then(std::path::Path::to_str),
            own_session: &cx.session,
            idle: &cx.config.agents.idle,
            since: self.started,
            ancestors: &rtok_sys::ancestors(std::process::id() as i32, link::ANCESTORS),
        };
        let found = link::resolve(&cx.store, &who, |k| std::env::var(k).ok()).unwrap_or_else(|e| {
            cx.log("warn", "mcp", "link", &format!("agent link failed: {e:#}"));
            link::Link::None
        });
        // A cwd link is a guess from who else is in the directory; a second session that starts
        // there later makes it ambiguous, so only a stable rule is remembered.
        if let link::Link::Linked { id, rule } = &found
            && rule.is_stable()
            && let Ok(mut g) = self.agent.lock()
        {
            *g = Some((id.clone(), *rule));
        }
        found
    }

    /// `invoke` plus the tools that need this process's own state.
    fn invoke_text(&self, name: &str, args: &Value) -> (String, bool) {
        let own = match name {
            "whoami" => self.whoami(),
            "worktree_add" => self
                .agent()
                .and_then(|(agent, _)| worktrees::add(&self.cx, &agent, args)),
            "worktree_remove" => self
                .agent()
                .and_then(|(agent, _)| worktrees::remove(&self.cx, &agent, args)),
            "worktree_adopt" => self
                .agent()
                .and_then(|(agent, _)| worktrees::adopt(&self.cx, &agent, args)),
            "agent_send" => self
                .agent()
                .and_then(|(agent, _)| messages::send(&self.cx, &agent, args)),
            "agent_inbox" => self
                .agent()
                .and_then(|(agent, _)| messages::inbox(&self.cx, &agent, args)),
            "agents_list" => agents::list(&self.cx, args),
            "agent_show" => agents::show(&self.cx, args),
            "agent_status_set" => self
                .agent()
                .and_then(|(agent, _)| agents::set_status(&self.cx, &agent, args)),
            "worktree_list" => worktrees::list(&self.cx),
            n if n.starts_with("task_") => tasks::call(&self.cx, n, args),
            _ => return invoke_text(&self.cx, name, args),
        };
        match own {
            Ok(t) => (t, true),
            Err(e) => (e.to_string(), false),
        }
    }

    /// The agent this session is linked to, or the reason it is not.
    fn agent(&self) -> Result<(crate::store::AgentDetail, link::Rule)> {
        let (id, rule) = match self.link() {
            link::Link::Linked { id, rule } => (id, rule),
            link::Link::Ambiguous(ids) => {
                let short: Vec<String> = ids
                    .iter()
                    .map(|i| i.chars().take(8).collect::<String>())
                    .collect();
                bail!(
                    "ambiguous: agents {} share this cwd; use RTOK_AGENT_ID",
                    short.join(", ")
                );
            }
            link::Link::Outside => bail!(
                "not linked to an agent session: this rtok mcp process is under no live {} \
                 session and in none's cwd, so one app likely shares it across sessions \
                 (Claude desktop's claude_desktop_config.json entry); use `rtok agents` and \
                 `rtok worktree` from the agent's shell",
                self.cx.config.hook.host
            ),
            link::Link::None => bail!("not linked to an agent session"),
        };
        let Some(d) = self.cx.store.agent_detail(&id)? else {
            bail!("not linked to an agent session");
        };
        Ok((d, rule))
    }

    fn whoami(&self) -> Result<String> {
        let (d, rule) = self.agent()?;
        let id = &d.id;
        let worktrees: Vec<String> = self
            .cx
            .store
            .open_worktree_claims()?
            .into_iter()
            .filter(|(_, agent)| agent == id)
            .map(|(path, _)| path)
            .collect();
        Ok(json!({
            "id": d.id,
            "short": d.short,
            "host": d.host,
            "host_session": d.host_session_id,
            "cwd": d.cwd,
            "rule": rule.as_str(),
            "worktrees": worktrees,
        })
        .to_string())
    }

    fn tools(&self) -> Vec<Tool> {
        self.listed.iter().map(|t| to_tool(&t.def)).collect()
    }

    fn allows(&self, name: &str) -> bool {
        self.listed.iter().any(|t| t.def.name == name)
    }

    /// One line in, at most one line out. A JSON-RPC batch (top-level array) answers with an
    /// array of the responses its members produced; an empty batch is `-32600` per JSON-RPC 2.0.
    fn handle_line(&self, line: &str) -> Option<String> {
        if line.trim().is_empty() {
            return None;
        }
        let req: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return Some(rpc_error(Value::Null, -32700, "parse error").to_string()),
        };
        if let Some(items) = req.as_array() {
            if items.is_empty() {
                return Some(rpc_error(Value::Null, -32600, "empty batch").to_string());
            }
            let out: Vec<Value> = items.iter().filter_map(|v| self.handle_value(v)).collect();
            return (!out.is_empty()).then(|| Value::Array(out).to_string());
        }
        self.handle_value(&req).map(|v| v.to_string())
    }

    fn handle_value(&self, req: &Value) -> Option<Value> {
        let obj = match req.as_object() {
            Some(o) => o,
            None => return Some(rpc_error(Value::Null, -32600, "invalid request")),
        };
        let method = obj.get("method").and_then(|m| m.as_str()).unwrap_or("");
        if obj.get("id").is_none() || method.starts_with("notifications/") {
            // T263: the one server-to-client request: ask for roots once the client can answer.
            if self.roots_capable.load(Ordering::Relaxed)
                && matches!(
                    method,
                    "notifications/initialized" | "notifications/roots/list_changed"
                )
            {
                return Some(json!({"jsonrpc":"2.0","id":"rtok-roots","method":"roots/list"}));
            }
            return None;
        }
        let id = obj["id"].clone();
        if method.is_empty() {
            // T263: the answer to our `roots/list` (result or error) is a response, not a request.
            if id == "rtok-roots" {
                if let Some(result) = obj.get("result") {
                    apply_roots_response(result);
                }
                return None;
            }
            return Some(rpc_error(id, -32600, "invalid request"));
        }
        let result = match method {
            "initialize" => {
                if !req["params"]["capabilities"]["roots"].is_null() {
                    self.roots_capable.store(true, Ordering::Relaxed);
                }
                // T283.1: a first attempt now; `whoami` retries while hooks have not registered.
                let _ = self.link();
                let version = negotiate_protocol_version(&req["params"]["protocolVersion"]);
                serde_json::to_value(server_info(version)).unwrap_or(json!({}))
            }
            "ping" => json!({}),
            "tools/list" => serde_json::to_value(ListToolsResult::with_all_items(self.tools()))
                .unwrap_or(json!({"tools": []})),
            "tools/call" => {
                let name = req["params"]["name"].as_str().unwrap_or("");
                let args = req["params"]["arguments"].clone();
                serde_json::to_value(self.call_tool(name, &args)).unwrap_or(json!({}))
            }
            _ => {
                // JSON-RPC 2.0's own wording for -32601, not the raw method name (T213):
                // https://www.jsonrpc.org/specification#error_object. The method still
                // reaches the client, in `data`, for anyone who wants it.
                return Some(json!({
                    "jsonrpc":"2.0",
                    "id":id,
                    "error":{"code":-32601,"message":"Method not found","data":{"method":method}},
                }));
            }
        };
        Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }

    fn call_tool(&self, name: &str, args: &Value) -> CallToolResult {
        let found = self.listed.iter().find(|t| t.def.name == name);
        let plugin = found.map(|t| t.plugin).unwrap_or("archive");
        let args = prepare_args(name, args);
        // A failure is an `isError` result with the same message text, not a success block
        // the model has to recognise by wording. A name the allow-list dropped never reaches
        // `invoke` — it must not run a tool the config says is off — but it fails with the
        // exact text `invoke`'s own unknown-name arm would give (`unknown_tool`, T192). A
        // request missing one of the tool's own `required` fields never reaches `invoke`
        // either, so no handler can turn it into a silent default or a store write (T213).
        let (text, ok) = if !self.allows(name) {
            (unknown_tool(name).to_string(), false)
        } else if let Some(msg) = found.and_then(|t| missing_required(&t.def.input_schema, &args)) {
            (format!("invalid params: {msg}"), false)
        } else {
            self.invoke_text(name, &args)
        };
        let _ = record(&self.cx, plugin, name, &args, &text);
        if !ok {
            self.cx
                .log("error", "mcp", name, &format!("tool failed: {text}"));
        }
        let content = vec![ContentBlock::text(text)];
        if ok {
            CallToolResult::success(content)
        } else {
            CallToolResult::error(content)
        }
    }
}

/// Tools that change nothing a user or another agent would see (T528). They carry MCP
/// `readOnlyHint`, so Codex's `default_tools_approval_mode = "writes"` runs them without
/// asking and still asks for every other tool. `agent_inbox` counts: it marks only this
/// session's own messages read. Only built-in tools are listed, so no plugin can claim a name.
const READ_ONLY_TOOLS: &[&str] = &[
    "expand",
    "ping",
    "whoami",
    "worktree_list",
    "agent_inbox",
    "agents_list",
    "agent_show",
    "task_list",
    "task_get",
    "task_next",
    "task_ready",
    "read",
    "search",
    "tree",
    "mem_search",
    "mem_pack",
    "mem_get",
    "handoff",
    "symbol",
    "callers",
    "impact",
    "outline",
    "explore",
    "graph_diff",
    "graph_export",
    "docs_resolve",
    "docs_query",
    "docs_get",
];

fn to_tool(def: &ToolDef) -> Tool {
    let schema = def
        .input_schema
        .as_object()
        .cloned()
        .unwrap_or_else(JsonObject::new);
    let tool = Tool::new(def.name, def.description, Arc::new(schema));
    if READ_ONLY_TOOLS.contains(&def.name) {
        tool.annotate(ToolAnnotations::new().read_only(true))
    } else {
        tool
    }
}

fn invoke(cx: &Runtime, name: &str, args: &Value) -> Result<String> {
    match name {
        "ping" => Ok(ping::ping_text(args["agent"].as_str().unwrap_or(""))),
        "expand" => expand_text(cx, args),
        #[cfg(feature = "memory")]
        "mem_save" => mem_save(cx, args),
        #[cfg(feature = "memory")]
        "mem_search" => mem_search(cx, args),
        #[cfg(feature = "docs")]
        "docs_resolve" => docs_resolve(cx, args),
        #[cfg(feature = "docs")]
        "docs_query" => docs_query(cx, args),
        #[cfg(feature = "docs")]
        "docs_get" => docs_get(cx, args),
        #[cfg(feature = "memory")]
        "mem_pack" => mem_pack(cx, args),
        #[cfg(feature = "memory")]
        "mem_get" => mem_get(cx, args),
        #[cfg(feature = "memory")]
        "mem_update" => mem_update(cx, args),
        #[cfg(feature = "memory")]
        "handoff" => handoff(cx, args),
        #[cfg(feature = "read")]
        "read" => read_file(cx, args),
        #[cfg(feature = "read")]
        "search" => search_files(cx, args),
        #[cfg(feature = "read")]
        "tree" => tree_files(cx, args),
        #[cfg(feature = "graph")]
        "symbol" | "callers" | "impact" | "outline" | "explore" | "graph_diff" | "graph_export" => {
            // T329.6: the root a graph call answers for is the project in use.
            if cx.config.plugins.graph.auto_add_projects
                && let Ok(root) = std::env::current_dir()
            {
                let _ = cx
                    .store
                    .auto_add_project(&root, crate::store::Origin::Mcp, None);
            }
            // T329.15: the page streams these events from the store; a failed write is ignored.
            let mut call = crate::plugins::graph::events::Call::start(cx, name, args);
            let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
            let answer =
                crate::plugins::graph::scope::resolve(&cx.store, args["project"].as_str(), &cwd)
                    .and_then(|scope| {
                        call.progress(scope.len());
                        // The diff reads the store's scans, which a plugin `Ctx` does not carry.
                        if name == "graph_diff" {
                            return crate::plugins::graph::diff::call(cx, args, &scope);
                        }
                        if name == "graph_export" {
                            return crate::plugins::graph::export::call(cx, args, &scope);
                        }
                        crate::plugins::graph::call(
                            &crate::plugin::Ctx::new(cx),
                            name,
                            args,
                            &scope,
                        )
                    });
            call.end(&answer);
            let answer = answer?;
            // T329.17: a project of the scope that is down is said in the answer being read.
            let notice = crate::plugins::graph::health::notice(cx, args["project"].as_str(), &cwd);
            Ok(match notice {
                Some(n) => n + &answer,
                None => answer,
            })
        }
        _ => Err(unknown_tool(name)),
    }
}

fn expand_text(cx: &Runtime, args: &Value) -> Result<String> {
    let id = args["id"].as_str().unwrap_or("");
    if let Some(spec) = args["lines"].as_str() {
        crate::expand::parse_range(spec, usize::MAX)?;
    }
    let Some(bytes) = crate::expand::fetch(cx, id)? else {
        bail!(crate::expand::unknown_id_message(id));
    };
    let text = String::from_utf8_lossy(&bytes);
    let context = args["context"].as_u64().map_or(0, |n| n as usize);
    let body = slice(
        &text,
        args["lines"].as_str(),
        args["grep"].as_str(),
        context,
    )?;
    Ok(cap_result(
        &body,
        id,
        cx.config.mcp.max_result_chars as usize,
    ))
}

fn slice(text: &str, lines: Option<&str>, grep: Option<&str>, context: usize) -> Result<String> {
    Ok(crate::expand::filter_lines(text, lines, grep, context)?.join("\n"))
}

fn cap_result(text: &str, id: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    crate::expand::cut(text, &format!("\n… expand({id}) …\n"), max)
}

#[cfg(feature = "memory")]
fn mem_save(cx: &Runtime, args: &Value) -> Result<String> {
    let kind = args["kind"].as_str().unwrap_or("note");
    let title = args["title"].as_str().unwrap_or("");
    let body = args["body"].as_str().unwrap_or("");
    let project = args["project"].as_str();
    let (id, updated) = crate::plugins::memory::mem_save(cx, kind, title, body, project)?;
    Ok(json!({"id": id, "updated": updated}).to_string())
}

#[cfg(feature = "docs")]
fn docs_resolve(cx: &Runtime, args: &Value) -> Result<String> {
    let name = args["name"].as_str().unwrap_or("");
    crate::plugins::docs::docs_resolve(cx, name)
}

#[cfg(feature = "docs")]
fn docs_query(cx: &Runtime, args: &Value) -> Result<String> {
    let name = args["name"].as_str().unwrap_or("");
    let version = args["version"].as_str();
    let query = args["query"].as_str().unwrap_or("");
    crate::plugins::docs::docs_query(cx, name, version, query)
}

#[cfg(feature = "docs")]
fn docs_get(cx: &Runtime, args: &Value) -> Result<String> {
    let id = args["id"]
        .as_i64()
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(|| anyhow::anyhow!("invalid doc id: {}", args["id"]))?;
    crate::plugins::docs::docs_get(cx, id)
}

#[cfg(feature = "memory")]
fn mem_pack(cx: &Runtime, args: &Value) -> Result<String> {
    let query = args["query"].as_str().unwrap_or("");
    let limit = args["limit"].as_u64().map_or(8, |n| n.clamp(1, 20)) as u32;
    let max_tokens = args["max_tokens"]
        .as_u64()
        .map_or(400, |n| n.clamp(1, 2000)) as u32;
    crate::plugins::memory::mem_pack(cx, query, limit, max_tokens)
}

#[cfg(feature = "memory")]
fn mem_search(cx: &Runtime, args: &Value) -> Result<String> {
    let query = args["query"].as_str().unwrap_or("");
    // `plugins.memory.search_limit` is the ceiling, not just the default: the caller's
    // `limit` used to size the response unbounded.
    let max = u64::from(cx.config.plugins.memory.search_limit);
    let limit = args["limit"].as_u64().map_or(max, |n| n.min(max)) as u32;
    let hits = crate::plugins::memory::mem_search(cx, query, limit)?;
    Ok(json!(
        hits.iter()
            .map(|h| json!({"id": h.id, "title": h.title, "snippet": h.snippet}))
            .collect::<Vec<_>>()
    )
    .to_string())
}

/// A JSON unsigned integer as `u32`, saturating: `as u32` would wrap `2^32 + 1` to `1`.
#[cfg(feature = "read")]
fn saturating_u32(v: &Value) -> Option<u32> {
    v.as_u64().map(|n| u32::try_from(n).unwrap_or(u32::MAX))
}

#[cfg(feature = "read")]
fn search_files(cx: &Runtime, args: &Value) -> Result<String> {
    let pattern = args["pattern"].as_str().unwrap_or("");
    let path = args["path"].as_str().unwrap_or(".");
    let max = saturating_u32(&args["max"]);
    crate::plugins::read::search::search(&crate::plugin::Ctx::new(cx), pattern, path, max)
}

#[cfg(feature = "read")]
fn tree_files(cx: &Runtime, args: &Value) -> Result<String> {
    let path = args["path"].as_str().unwrap_or(".");
    let depth = saturating_u32(&args["depth"]);
    crate::plugins::read::search::tree(&crate::plugin::Ctx::new(cx), path, depth)
}

#[cfg(feature = "read")]
fn read_file(cx: &Runtime, args: &Value) -> Result<String> {
    let path = args["path"].as_str().unwrap_or("");
    let mode = args["mode"].as_str().unwrap_or("");
    let range = args["range"].as_str();
    crate::plugins::read::read(&crate::plugin::Ctx::new(cx), path, mode, range)
}

#[cfg(feature = "memory")]
fn mem_get(cx: &Runtime, args: &Value) -> Result<String> {
    if let Some(n) = args.get("obs").and_then(Value::as_i64) {
        let id = i32::try_from(n).map_err(|_| anyhow::anyhow!("invalid obs id: {n}"))?;
        return crate::plugins::memory::obs_get(cx, id)?
            .ok_or_else(|| anyhow::anyhow!("unknown observation id: {id}"));
    }
    let id = args["id"]
        .as_i64()
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(|| anyhow::anyhow!("invalid note id: {}", args["id"]))?;
    crate::plugins::memory::mem_get(cx, id)?.ok_or_else(|| anyhow::anyhow!("unknown note id: {id}"))
}

#[cfg(feature = "memory")]
fn mem_update(cx: &Runtime, args: &Value) -> Result<String> {
    let id = args["id"]
        .as_i64()
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(|| anyhow::anyhow!("invalid note id: {}", args["id"]))?;
    let retire = args["retire"].as_bool().unwrap_or(false);
    let superseded_by = args["superseded_by"]
        .as_i64()
        .and_then(|n| i32::try_from(n).ok());
    let pinned = args["pinned"].as_bool();
    crate::plugins::memory::mem_update(cx, id, retire, superseded_by, pinned)
}

fn record(cx: &Runtime, plugin: &str, name: &str, args: &Value, result: &str) -> Result<()> {
    let args_s = args.to_string();
    let before = i64::from(cx.estimate(&args_s, Class::Json));
    let after = i64::from(cx.estimate(result, Class::Json));
    let host = cx.store.host_id(&cx.config.hook.host)?.or(Some(6));
    cx.store
        .upsert_session(&cx.session, host, None, None, Some("mcp"))?;
    let call_id = cx.store.insert_call(
        &cx.session,
        "mcp",
        "mcp_call",
        host,
        None,
        None,
        Some(plugin),
        Some(name),
    )?;
    let cap = cx.config.core.call_io_inline_bytes as usize;
    cx.store.insert_call_io(
        call_id,
        Some(args_s.as_bytes()),
        Some(result.as_bytes()),
        cap,
        Some(&cx.config.core.archive_dir),
    )?;
    cx.store
        .insert_tokens(call_id, None, "before", "estimate", before)?;
    cx.store
        .insert_tokens(call_id, None, "after", "estimate", after)?;
    cx.store
        .insert_tokens(call_id, Some(plugin), "mcp", "estimate", after)?;
    Ok(())
}

#[cfg(feature = "memory")]
fn handoff(cx: &Runtime, args: &Value) -> Result<String> {
    // `missing_required` already rejected an absent value; saturate instead of `as u32`
    // wrapping a huge budget into a tiny one (T213).
    let budget = args["budget_tokens"]
        .as_u64()
        .map_or(800, |n| u32::try_from(n).unwrap_or(u32::MAX));
    Ok(crate::plugins::memory::handoff::handoff(
        &crate::plugin::Ctx::new(cx),
        budget,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::testutil::config as tmp;
    use crate::tokens::Class;
    use rstest::rstest;
    use std::fs;

    /// A long line answers `-32700` downstream (the next request still parses) and a
    /// non-UTF-8 byte no longer ends the read loop.
    #[test]
    fn next_line_skips_long_lines_and_survives_bad_utf8() {
        let mut r: &[u8] = b"0123456789\n{\"id\":1}\n\xff\n1234\n";
        let mut buf = Vec::new();
        let mut got = Vec::new();
        while let Some(l) = next_line(&mut r, &mut buf, 5).unwrap() {
            got.push(l);
        }
        assert_eq!(got, ["{", "{", "\u{FFFD}\n", "1234\n"]);
        let mut r: &[u8] = b"0123456789\n{\"id\":1}\n";
        assert_eq!(next_line(&mut r, &mut buf, 5).unwrap().unwrap(), "{");
        assert_eq!(
            next_line(&mut r, &mut buf, 64).unwrap().unwrap(),
            "{\"id\":1}\n"
        );
    }

    /// T528: tools that change a worktree, a task, a note or another agent's inbox; Codex asks
    /// before each under `"writes"`.
    const WRITING_TOOLS: &[&str] = &[
        "worktree_add",
        "worktree_remove",
        "worktree_adopt",
        "agent_send",
        "agent_status_set",
        "task_create",
        "task_status",
        "task_claim",
        "task_release",
        "task_dep",
        "task_priority",
        "task_conflict",
        "task_parallel",
        "mem_save",
        "mem_update",
    ];

    /// T528: every built-in tool, enabled or not, is sorted into one list on purpose, so a new
    /// tool can neither skip Codex's approval nor lose it by accident.
    #[test]
    fn every_tool_is_read_only_or_writing_and_only_read_only_ones_say_so() {
        let (cfg, dir) = tmp("mcp-read-only");
        let server = Server::new(&cfg).unwrap();
        let defs: Vec<ToolDef> = server
            .listed
            .iter()
            .map(|t| t.def.clone())
            .chain(crate::plugins::all().iter().flat_map(|p| p.mcp_tools()))
            .collect();
        for def in &defs {
            let read_only = READ_ONLY_TOOLS.contains(&def.name);
            assert!(
                read_only != WRITING_TOOLS.contains(&def.name),
                "{} must be in exactly one list",
                def.name
            );
            let hint = to_tool(def).annotations.and_then(|a| a.read_only_hint);
            assert_eq!(hint, read_only.then_some(true), "{}", def.name);
        }
        for name in READ_ONLY_TOOLS.iter().chain(WRITING_TOOLS) {
            assert!(defs.iter().any(|d| d.name == *name), "{name} is no tool");
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn malformed_line_answers_parse_error() {
        let (cfg, dir) = tmp("mcp-bad");
        let server = Server::new(&cfg).unwrap();
        let v: Value = serde_json::from_str(&server.handle_line("{bad").unwrap()).unwrap();
        assert_eq!(v["error"]["code"], -32700);
        assert_eq!(v["id"], Value::Null);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn non_object_line_answers_invalid_request() {
        let (cfg, dir) = tmp("mcp-nonobj");
        let server = Server::new(&cfg).unwrap();
        let v: Value = serde_json::from_str(&server.handle_line("123").unwrap()).unwrap();
        assert_eq!(v["error"]["code"], -32600);
        assert_eq!(v["id"], Value::Null);
        assert!(server.handle_line("").is_none());
        assert!(
            server
                .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/x"}"#)
                .is_none()
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn slice_bare_lines_matches_cli_parse_range() {
        let text = (1..=12)
            .map(|n| format!("L{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(slice(&text, Some("10"), None, 0).unwrap(), "L10\nL11\nL12");
        assert_eq!(slice(&text, Some("10-10"), None, 0).unwrap(), "L10");
        assert!(slice(&text, Some("wat"), None, 0).is_err());
    }

    /// T67.2: the MCP `expand` accepts `context` like the CLI `--context`, windows
    /// merged with `--`, absolute numbers, under the same `max_result_chars` cap.
    #[test]
    fn expand_text_context_returns_merged_windows() {
        let (cfg, dir) = tmp("mcp-ctx");
        let cx = crate::plugin::Runtime::open(cfg, "mcp-ctx").unwrap();
        let body = "a\nHIT\nb\nc\nd\ne\nHIT\nf\n";
        let id = cx
            .store
            .put_archive("mcp", body.as_bytes(), &cx.config.core.archive_dir)
            .unwrap();
        let args = serde_json::json!({"id": id, "grep": "HIT", "context": 1});
        let out = expand_text(&cx, &args).unwrap();
        assert_eq!(out, "1:a\n2:HIT\n3:b\n--\n6:e\n7:HIT\n8:f");
        // Without `context` the hits stay bare, as before.
        let args = serde_json::json!({"id": id, "grep": "HIT"});
        assert_eq!(expand_text(&cx, &args).unwrap(), "2:HIT\n7:HIT");
        let _ = fs::remove_dir_all(dir);
    }

    #[rstest]
    #[case(500)]
    #[case(200)]
    fn expand_honours_max_result_chars(#[case] max_chars: u32) {
        let (mut cfg, dir) = tmp("mcp-cap");
        cfg.mcp.max_result_chars = max_chars;
        let cx = crate::plugin::Runtime::open(cfg, "mcp-cap").unwrap();
        let blob = "x".repeat(100 * 1024);
        let id = cx
            .store
            .put_archive("mcp", blob.as_bytes(), &cx.config.core.archive_dir)
            .unwrap();
        let args = serde_json::json!({"id": id});
        let out = expand_text(&cx, &args).unwrap();
        assert!(out.contains("expand("), "{out}");
        assert!(
            out.chars().count() <= max_chars as usize,
            "{}/{}",
            out.chars().count(),
            max_chars
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn descriptions_at_most_60_tokens() {
        let (cfg, dir) = tmp("desc");
        let server = Server::new(&cfg).unwrap();
        let max = cfg.mcp.max_description_tokens;
        for t in server.tools() {
            let d = t.description.as_deref().unwrap_or("");
            let n = crate::tokens::estimate(d, Class::Prose, &cfg.estimator);
            assert!(n <= max, "{} is {n} tokens (max {max}): {d}", t.name);
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn tools_call_writes_calls_io_and_three_token_rows() {
        let (cfg, dir) = tmp("call");
        let server = Server::new(&cfg).unwrap();
        let id = server
            .cx
            .store
            .put_archive("mcp", b"payload", &cfg.core.archive_dir)
            .unwrap();
        let line = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"expand","arguments":{{"id":"{id}"}}}}}}"#
        );
        let out = server.handle_line(&line).expect("response");
        assert!(out.contains("payload"), "{out}");
        assert_eq!(server.cx.store.count_kind("mcp_call").unwrap(), 1);
        assert_eq!(server.cx.store.count_call_io().unwrap(), 1);
        assert_eq!(server.cx.store.count_tokens().unwrap(), 3);
        let _ = fs::remove_dir_all(dir);
    }

    /// A failed call is an `isError` result carrying the message, never a success block.
    #[test]
    fn failed_call_sets_is_error() {
        let (cfg, dir) = tmp("iserr");
        let server = Server::new(&cfg).unwrap();
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"nope","arguments":{}}}"#;
        let v: Value = serde_json::from_str(&server.handle_line(line).unwrap()).unwrap();
        assert_eq!(v["result"]["isError"], true, "{v}");
        assert_eq!(v["result"]["content"][0]["text"], "unknown tool: nope");
        let line = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"expand","arguments":{"id":"x","lines":"wat"}}}"#;
        let v: Value = serde_json::from_str(&server.handle_line(line).unwrap()).unwrap();
        assert_eq!(v["result"]["isError"], true, "{v}");
        let _ = fs::remove_dir_all(dir);
    }

    /// T172: the `read` root guard (`src/plugins/read/mod.rs`'s `resolve_with`) answers
    /// `isError: true` with the refusal text, never a plain-text success block the model
    /// could mistake for file content.
    #[test]
    fn read_outside_cwd_sets_is_error() {
        let (cfg, dir) = tmp("outside-cwd");
        let server = Server::new(&cfg).unwrap();
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"read","arguments":{"path":"/etc/hosts"}}}"#;
        let v: Value = serde_json::from_str(&server.handle_line(line).unwrap()).unwrap();
        assert_eq!(v["result"]["isError"], true, "{v}");
        assert!(
            v["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("path outside cwd"),
            "{v}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// T172: a range the model quotes verbatim (`"1-1"`, copied from earlier output) reads
    /// the same file the bare form does instead of failing with `invalid line range`.
    #[test]
    fn read_accepts_a_quoted_range() {
        let (cfg, dir) = tmp("quoted-range");
        let server = Server::new(&cfg).unwrap();
        let bare = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"read","arguments":{"path":"Cargo.toml","range":"1-1"}}}"#;
        let quoted = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read","arguments":{"path":"Cargo.toml","range":"\"1-1\""}}}"#;
        let v_bare: Value = serde_json::from_str(&server.handle_line(bare).unwrap()).unwrap();
        let v_quoted: Value = serde_json::from_str(&server.handle_line(quoted).unwrap()).unwrap();
        assert_eq!(v_bare["result"]["isError"], false, "{v_bare}");
        assert_eq!(v_quoted["result"]["isError"], false, "{v_quoted}");
        assert_eq!(
            v_bare["result"]["content"][0]["text"],
            v_quoted["result"]["content"][0]["text"]
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// T172: `invoke_text` never doubles an `Error:` prefix onto a failure's text — there is
    /// exactly one site (`invoke_text`) that turns a tool `Err` into content text, and it
    /// never adds a prefix of its own, across the tool surfaces most likely to fail (an
    /// unknown name, the read root guard, and a malformed range — the audit's "timeout"
    /// case is the same code path once the error reaches `invoke_text`).
    #[test]
    fn failed_call_text_never_doubles_the_error_prefix() {
        let (cfg, dir) = tmp("no-double-prefix");
        let server = Server::new(&cfg).unwrap();
        let lines = [
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"nope","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read","arguments":{"path":"/etc/hosts"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read","arguments":{"path":"Cargo.toml","range":"975-1015"}}}"#,
        ];
        for line in lines {
            let v: Value = serde_json::from_str(&server.handle_line(line).unwrap()).unwrap();
            assert_eq!(v["result"]["isError"], true, "{v}");
            let text = v["result"]["content"][0]["text"].as_str().unwrap();
            assert!(!text.contains("Error: Error:"), "{text}");
            assert!(text.matches("Error:").count() <= 1, "{text}");
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn ping_replies_with_the_display_name_and_writes_no_measurement() {
        let (cfg, dir) = tmp("ping");
        let server = Server::new(&cfg).unwrap();
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"ping","arguments":{"agent":"Claude Code"}}}"#;
        let v: Value = serde_json::from_str(&server.handle_line(line).unwrap()).unwrap();
        assert_eq!(v["result"]["isError"], false, "{v}");
        assert_eq!(v["result"]["content"][0]["text"], "MCP Claude Code жив");
        assert_eq!(server.cx.store.count_kind("mcp_call").unwrap(), 1);
        assert_eq!(server.cx.store.count_call_io().unwrap(), 1);
        assert_eq!(server.cx.store.count_tokens().unwrap(), 3);
        assert_eq!(server.cx.store.count_measurements().unwrap(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    /// T192: `cfg.mcp.tools` allow-list filters listing and calls; `expand`, `ping` and `whoami` stay
    /// listed unconditionally (`expand` is D4 losslessness, `ping` is the liveness check).
    #[test]
    fn tools_allow_list_filters_listing_and_calls() {
        let (mut cfg, dir) = tmp("allow");
        cfg.mcp.tools = vec!["read".to_string()];
        let server = Server::new(&cfg).unwrap();
        let mut names: Vec<String> = server.tools().iter().map(|t| t.name.to_string()).collect();
        names.sort();
        assert_eq!(names, ["expand", "ping", "read", "whoami"]);
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search","arguments":{"pattern":"x"}}}"#;
        let v: Value = serde_json::from_str(&server.handle_line(line).unwrap()).unwrap();
        assert_eq!(v["result"]["isError"], true, "{v}");
        assert_eq!(v["result"]["content"][0]["text"], "unknown tool: search");
        let line = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"expand","arguments":{"id":"x"}}}"#;
        let v: Value = serde_json::from_str(&server.handle_line(line).unwrap()).unwrap();
        assert_eq!(v["result"]["isError"], true, "{v}");
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(text, crate::expand::unknown_id_message("x"));
        assert!(text.starts_with("unknown archive id: x ("), "{text}");
        // The one-shot `call()` path (used by hosts that cannot speak MCP) rejects a
        // filtered name with the same text, never running it.
        let err = call(&cfg, "search", &json!({"pattern": "x"}))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "unknown tool: search");
        let _ = fs::remove_dir_all(dir);
    }

    /// `max` / `depth` arrive as JSON `u64`; `as u32` wrapped `2^32 + 1` to `1`, so asking
    /// for more rows returned a single one. They saturate like `handoff`'s budget (T213).
    #[cfg(feature = "read")]
    #[test]
    fn search_and_tree_saturate_huge_limits_instead_of_wrapping() {
        let (mut cfg, dir) = tmp("mcp-u32-wrap");
        let root = dir.join("src-tree");
        fs::create_dir_all(root.join("a").join("b")).unwrap();
        fs::write(root.join("hits.txt"), "needle\nneedle\nneedle\n").unwrap();
        fs::write(root.join("a").join("b").join("deep.txt"), "x").unwrap();
        cfg.plugins.read.allow_paths = vec![root.clone()];
        let wraps_to_one = u64::from(u32::MAX) + 2;
        let path = root.to_string_lossy();
        let out = call(
            &cfg,
            "search",
            &json!({"pattern": "needle", "path": path, "max": wraps_to_one}),
        )
        .unwrap();
        assert_eq!(out.lines().count(), 3, "{out}");
        let out = call(&cfg, "tree", &json!({"path": path, "depth": wraps_to_one})).unwrap();
        assert!(out.contains("deep.txt"), "{out}");
        let _ = fs::remove_dir_all(dir);
    }

    /// T353: agents send `query` to `search`; it runs as `pattern`. Any other missing key
    /// names what the call did send, so a misspelt argument is visible in the error.
    #[cfg(feature = "read")]
    #[test]
    fn search_accepts_query_and_missing_keys_name_what_was_sent() {
        let (mut cfg, dir) = tmp("mcp-query-alias");
        let root = dir.join("src-tree");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("hits.txt"), "needle\n").unwrap();
        cfg.plugins.read.allow_paths = vec![root.clone()];
        let path = root.to_string_lossy();
        let by_pattern = call(&cfg, "search", &json!({"pattern": "needle", "path": path})).unwrap();
        let by_query = call(&cfg, "search", &json!({"query": "needle", "path": path})).unwrap();
        assert_eq!(by_pattern, by_query);
        assert!(by_query.contains("hits.txt"), "{by_query}");
        // An explicit `pattern` wins over the alias.
        let both = json!({"pattern": "needle", "query": "zzz", "path": path});
        assert_eq!(call(&cfg, "search", &both).unwrap(), by_pattern);
        let err = call(&cfg, "search", &json!({"path": path, "range": "1-80"})).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid params: missing `pattern` (got: path, range)"
        );
        let err = call(&cfg, "search", &json!({})).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid params: missing `pattern` (got: no arguments)"
        );
        // No alias elsewhere: `read` does not take `query`.
        let err = call(&cfg, "read", &json!({"query": "x"})).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid params: missing `path` (got: query)"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn docs_query_prepare_args_renames_question_and_user_query() {
        let a = prepare_args(
            "docs_query",
            &json!({"name": "serde", "question": "Deserialize"}),
        );
        assert_eq!(a["query"], "Deserialize");
        assert!(a.get("question").is_none());
        let a = prepare_args(
            "docs_query",
            &json!({"name": "serde", "userQuery": "Deserialize"}),
        );
        assert_eq!(a["query"], "Deserialize");
        assert!(a.get("userQuery").is_none());
        let a = prepare_args(
            "docs_query",
            &json!({"name": "serde", "query": "a", "question": "b"}),
        );
        assert_eq!(a["query"], "a", "explicit query wins");
    }

    #[test]
    fn one_shot_call_uses_the_same_invoke_as_tools_call() {
        let (cfg, dir) = tmp("oneshot");
        let err = call(&cfg, "nope", &json!({})).unwrap_err().to_string();
        assert_eq!(err, "unknown tool: nope");
        let server = Server::new(&cfg).unwrap();
        let id = server
            .cx
            .store
            .put_archive("mcp", b"payload", &cfg.core.archive_dir)
            .unwrap();
        let text = call(&cfg, "expand", &json!({"id": id})).unwrap();
        assert_eq!(text, "payload");
        let _ = fs::remove_dir_all(dir);
    }

    /// A JSON-RPC batch answers as one array; notifications inside it produce nothing.
    #[test]
    fn batch_answers_with_an_array() {
        let (cfg, dir) = tmp("batch");
        let server = Server::new(&cfg).unwrap();
        let line = r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/initialized"},{"jsonrpc":"2.0","id":2,"method":"nope"}]"#;
        let v: Value = serde_json::from_str(&server.handle_line(line).unwrap()).unwrap();
        let arr = v.as_array().expect("array");
        assert_eq!(arr.len(), 2, "{v}");
        assert_eq!(arr[0]["id"], 1);
        assert_eq!(arr[1]["error"]["code"], -32601);
        assert_eq!(arr[1]["error"]["message"], "Method not found", "{v}");
        assert_eq!(arr[1]["error"]["data"]["method"], "nope", "{v}");
        assert!(server.handle_line("[]").unwrap().contains("-32600"));
        assert!(
            server
                .handle_line(r#"[{"jsonrpc":"2.0","method":"notifications/x"}]"#)
                .is_none()
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// Each `rtok mcp` process is its own session; the surface tag stays "mcp".
    #[test]
    fn session_is_per_process() {
        let (cfg, dir) = tmp("sess");
        let server = Server::new(&cfg).unwrap();
        assert_eq!(server.cx.session, format!("mcp-{}", std::process::id()));
        let _ = fs::remove_dir_all(dir);
    }

    #[rstest]
    fn retention_runs_on_mcp_session_start() {
        let (mut cfg, dir) = tmp("mcp-retain");
        cfg.core.retain_calls_days = 1;
        {
            let store = Store::open(&cfg.core.db_path).unwrap();
            store
                .upsert_session("sess", Some(1), None, None, Some("mcp"))
                .unwrap();
            let call = store
                .insert_call("sess", "mcp", "mcp_call", Some(1), None, None, None, None)
                .unwrap();
            let body = vec![b'z'; 70 * 1024];
            store
                .insert_call_io(
                    call,
                    Some(&body),
                    None,
                    64 * 1024,
                    Some(&cfg.core.archive_dir),
                )
                .unwrap();
            store.set_call_ts(call, 0).unwrap();
        }
        let server = Server::new(&cfg).unwrap();
        server
            .cx
            .store
            .run_retention(cfg.core.retain_calls_days, cfg.core.retain_hook_bodies_days)
            .unwrap();
        assert_eq!(server.cx.store.count_calls().unwrap(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    /// T213: `initialize` echoes back a `protocolVersion` this server supports, and falls
    /// back to the pinned latest-supported version (never `ProtocolVersion::default()`,
    /// which drifts with the linked `rmcp`) when the client asks for one it doesn't know,
    /// or asks for none at all. Spec: https://modelcontextprotocol.io/specification —
    /// "Initialization".
    #[test]
    fn initialize_negotiates_protocol_version() {
        let (cfg, dir) = tmp("mcp-negotiate");
        let server = Server::new(&cfg).unwrap();
        let req = |params: Value| {
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":params}).to_string()
        };
        let client = json!({"capabilities": {}, "clientInfo": {"name":"t","version":"1"}});
        let mut supported = client.clone();
        supported["protocolVersion"] = json!("2024-11-05");
        let v: Value = serde_json::from_str(&server.handle_line(&req(supported)).unwrap()).unwrap();
        assert_eq!(v["result"]["protocolVersion"], "2024-11-05", "{v}");
        let mut unknown = client.clone();
        unknown["protocolVersion"] = json!("1999-01-01");
        let v: Value = serde_json::from_str(&server.handle_line(&req(unknown)).unwrap()).unwrap();
        assert_eq!(v["result"]["protocolVersion"], "2025-06-18", "{v}");
        let v: Value = serde_json::from_str(&server.handle_line(&req(client)).unwrap()).unwrap();
        assert_eq!(v["result"]["protocolVersion"], "2025-06-18", "{v}");
        let _ = fs::remove_dir_all(dir);
    }

    /// Check (T213): `mem_save` with only `{"title":"t"}` is rejected before it ever
    /// touches the store — `body` is `required` on the schema, but the handler used to
    /// `unwrap_or("")` a missing one, silently saving a broken note.
    #[cfg(feature = "memory")]
    #[test]
    fn mem_save_missing_body_rejected_before_store_write() {
        let (cfg, dir) = tmp("mcp-mem-save-missing");
        let server = Server::new(&cfg).unwrap();
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mem_save","arguments":{"title":"t"}}}"#;
        let v: Value = serde_json::from_str(&server.handle_line(line).unwrap()).unwrap();
        assert_eq!(v["result"]["isError"], true, "{v}");
        assert_eq!(
            v["result"]["content"][0]["text"], "invalid params: missing `body` (got: title)",
            "{v}"
        );
        assert!(
            server.cx.store.note_bodies().unwrap().is_empty(),
            "notes table must stay empty"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// T283.1: `initialize` then `whoami` through the server's own JSON-RPC path, as a host
    /// would; `(isError, text)` of the answer.
    fn whoami_over_rpc(server: &Server) -> (bool, String) {
        let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{}}}"#;
        assert!(server.handle_line(init).is_some());
        let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"whoami","arguments":{}}}"#;
        let v: Value = serde_json::from_str(&server.handle_line(call).unwrap()).unwrap();
        (
            v["result"]["isError"] == true,
            v["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        )
    }

    fn cwd_string() -> String {
        std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn whoami_names_the_agent_a_hook_registered_for_this_cwd() {
        let (mut cfg, dir) = tmp("whoami-cwd");
        cfg.hook.host = "claude".into();
        let server = Server::new(&cfg).unwrap();
        let host = server.cx.host_id().unwrap();
        let id = server
            .cx
            .store
            .register_agent(host, "sess-a", None, Some(&cwd_string()), None)
            .unwrap();
        let (is_err, text) = whoami_over_rpc(&server);
        assert!(!is_err, "{text}");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["id"], id);
        assert_eq!(v["short"], id[..8]);
        assert_eq!(v["host"], "claude");
        assert_eq!(v["rule"], "cwd");
        assert_eq!(v["worktrees"], json!([]));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn whoami_retries_until_the_hooks_have_registered() {
        let (mut cfg, dir) = tmp("whoami-late");
        cfg.hook.host = "claude".into();
        let server = Server::new(&cfg).unwrap();
        let (is_err, text) = whoami_over_rpc(&server);
        assert!(is_err && text == "not linked to an agent session", "{text}");
        let host = server.cx.host_id().unwrap();
        server
            .cx
            .store
            .register_agent(host, "sess-a", None, Some(&cwd_string()), None)
            .unwrap();
        let call = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"whoami","arguments":{}}}"#;
        let v: Value = serde_json::from_str(&server.handle_line(call).unwrap()).unwrap();
        assert_ne!(v["result"]["isError"], true, "{v}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn whoami_refuses_to_pick_between_sessions_in_one_cwd() {
        let (mut cfg, dir) = tmp("whoami-ambiguous");
        cfg.hook.host = "claude".into();
        let server = Server::new(&cfg).unwrap();
        let host = server.cx.host_id().unwrap();
        for s in ["sess-a", "sess-b"] {
            server
                .cx
                .store
                .register_agent(host, s, None, Some(&cwd_string()), None)
                .unwrap();
        }
        let (is_err, text) = whoami_over_rpc(&server);
        assert!(is_err && text.starts_with("ambiguous: agents "), "{text}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn whoami_ignores_a_session_that_ended_before_this_process_started() {
        let (mut cfg, dir) = tmp("whoami-stale");
        cfg.hook.host = "claude".into();
        let mut server = Server::new(&cfg).unwrap();
        server.started = crate::log::now() as i64 - 100;
        let host = server.cx.host_id().unwrap();
        // Session A in this cwd ended minutes ago, still inside `[agents] idle`; B's hook has
        // not written its row yet, so A must not be taken for B.
        let a = server
            .cx
            .store
            .register_agent(host, "sess-a", None, Some(&cwd_string()), None)
            .unwrap();
        server
            .cx
            .store
            .set_agent_last_seen(&a, crate::log::now() as i64 - 500)
            .unwrap();
        let (is_err, text) = whoami_over_rpc(&server);
        assert!(is_err && text == "not linked to an agent session", "{text}");
        let _ = fs::remove_dir_all(dir);
    }

    // Only where `rtok_sys::ancestors` can read this process's parents: without its own chain
    // the server cannot tell it sits outside a session (Windows reads none).
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn whoami_says_when_no_live_session_is_above_this_process_or_in_its_cwd() {
        let (mut cfg, dir) = tmp("whoami-outside");
        cfg.hook.host = "claude".into();
        let server = Server::new(&cfg).unwrap();
        let host = server.cx.host_id().unwrap();
        let other = server
            .cx
            .store
            .register_agent(host, "sess-a", None, Some("/elsewhere-t455"), None)
            .unwrap();
        // Above any real pid limit, so no process of the test run can be in this chain.
        server
            .cx
            .store
            .set_agent_ancestors(&other, &[2_000_000_001, 2_000_000_002])
            .unwrap();
        let (is_err, text) = whoami_over_rpc(&server);
        assert!(
            is_err
                && text.starts_with(
                    "not linked to an agent session: this rtok mcp process is under no live \
                     claude session"
                ),
            "{text}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_cwd_link_is_reevaluated_when_a_second_session_appears() {
        let (mut cfg, dir) = tmp("whoami-reeval");
        cfg.hook.host = "claude".into();
        let server = Server::new(&cfg).unwrap();
        let host = server.cx.host_id().unwrap();
        server
            .cx
            .store
            .register_agent(host, "sess-a", None, Some(&cwd_string()), None)
            .unwrap();
        let (is_err, text) = whoami_over_rpc(&server);
        assert!(!is_err, "{text}");
        server
            .cx
            .store
            .register_agent(host, "sess-b", None, Some(&cwd_string()), None)
            .unwrap();
        let call = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"whoami","arguments":{}}}"#;
        let v: Value = serde_json::from_str(&server.handle_line(call).unwrap()).unwrap();
        assert_eq!(v["result"]["isError"], true, "{v}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_host_without_hooks_registers_itself_through_mcp_alone() {
        let (mut cfg, dir) = tmp("whoami-hookless");
        cfg.hook.host = "zed".into();
        let server = Server::new(&cfg).unwrap();
        let (is_err, text) = whoami_over_rpc(&server);
        assert!(!is_err, "{text}");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["rule"], "own");
        assert_eq!(v["host_session"], format!("mcp-{}", std::process::id()));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_disabled_registry_links_nothing() {
        let (mut cfg, dir) = tmp("whoami-off");
        cfg.hook.host = "zed".into();
        cfg.agents.enabled = false;
        let server = Server::new(&cfg).unwrap();
        let (is_err, text) = whoami_over_rpc(&server);
        assert!(is_err && text == "not linked to an agent session", "{text}");
        let _ = fs::remove_dir_all(dir);
    }
}
