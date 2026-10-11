// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Claude Code hook surface: `rtok hook <event>`.
//!
//! - [`types`] — stdin/stdout JSON contract (plan T0.6)
//! - dispatcher — plan T2.1

#![allow(clippy::print_stdout, clippy::print_stderr)]

mod push;
pub mod resident;
pub mod types;

use crate::config::Config;
use crate::plugin::{Ctx, Injection, PreToolDecision, Runtime, SessionStart};
use crate::plugins::Registry;
use crate::tokens::Class;
use serde_json::Value;
use std::io::{Read, Write};
use std::panic::{self, AssertUnwindSafe};
use std::time::Instant;
use types::{HookInput, HookOutput, HookSpecificOutput};

/// T178: the hook's budget is 10 ms, so it waits 5 ms on another process's lock — one connect,
/// migrations included — and then fails open. Every statement used to wait 1 s.
const LOCK_WAIT: crate::store::LockWait = crate::store::LockWait {
    busy: std::time::Duration::from_millis(5),
    attempts: 1,
    migrate: std::time::Duration::from_millis(5),
};

// T309: each lock wait stays within half the 10 ms hook budget (D1). `tests/latency.rs` proves
// the hook gives up instead of waiting for the holder; the ms bound lives here, where it is exact.
const _: () = assert!(LOCK_WAIT.busy.as_millis() <= 5 && LOCK_WAIT.migrate.as_millis() <= 5);

/// Fail-open hook entry: always writes JSON and does not return `Err`.
/// With `[hook] fail_open = false` (debugging only) errors surface as a panic
/// instead of `{}` — the default `true` keeps the fail-open rule (D1).
///
/// T201: stdin used to be an unbounded `read_to_end`, so a multi-MB payload paid a JSON
/// parse plus downstream hashing that grew linearly with its size — the ≤ 10 ms budget (D1)
/// broke deterministically per MB. `core.hook_max_input_bytes` bounds the read via `Take`
/// (one byte over the cap, so a body exactly at the limit is not mistaken for oversized);
/// a body over it is discarded before it ever reaches the JSON parser and the hook fails
/// open to `{}` unmodified, with one stderr line.
pub fn run(event: &str, mut stdin: impl Read, mut stdout: impl Write, cfg: &Config) {
    let max = u64::from(cfg.core.hook_max_input_bytes);
    if cfg.hook.fail_open {
        let mut buf = Vec::new();
        if let Err(e) = stdin.by_ref().take(max + 1).read_to_end(&mut buf) {
            let msg = format!("stdin read failed: {e}; failing open");
            eprintln!("rtok: hook {event} {msg}");
            crate::log::append(cfg, "error", "hook", event, &msg);
            let _ = stdout.write_all(b"{}");
            return;
        }
        if buf.len() as u64 > max {
            let msg = format!(
                "stdin over core.hook_max_input_bytes ({max} bytes, got {}); failing open",
                buf.len()
            );
            eprintln!("rtok: hook {event} {msg}");
            crate::log::append(cfg, "error", "hook", event, &msg);
            let _ = stdout.write_all(b"{}");
            return;
        }
        let out = panic::catch_unwind(AssertUnwindSafe(|| dispatch_owned(&buf, event, cfg)))
            .unwrap_or_else(|payload| {
                let msg = format!("panicked: {}; failing open", panic_message(&payload));
                eprintln!("rtok: hook {event} {msg}");
                crate::log::append(cfg, "error", "hook", event, &msg);
                b"{}".to_vec()
            });
        let _ = stdout.write_all(&out);
    } else {
        let mut buf = Vec::new();
        stdin
            .by_ref()
            .take(max + 1)
            .read_to_end(&mut buf)
            .expect("rtok hook: stdin unreadable (fail_open = false)");
        assert!(
            buf.len() as u64 <= max,
            "rtok hook: stdin over core.hook_max_input_bytes ({max} bytes, debugging only)"
        );
        let out = dispatch_owned_strict(&buf, event, cfg).expect("rtok hook");
        let _ = stdout.write_all(&out);
    }
}

/// Session id: stdin first, then `$<core.session_env>`, else `"unknown"`.
/// An empty `session_env` key disables the env fallback.
fn resolve_session(
    stdin_session: &str,
    session_env_key: &str,
    env: impl Fn(&str) -> Option<String>,
) -> String {
    if !stdin_session.is_empty() {
        return stdin_session.to_string();
    }
    if !session_env_key.is_empty()
        && let Some(v) = env(session_env_key)
    {
        let v = v.trim().to_string();
        if !v.is_empty() {
            return v;
        }
    }
    "unknown".into()
}

/// T282: the sub-agent key `register_agent`/`end_agent` upsert against — the host's own
/// `agent_id` (`HookInput`, `research.md` §17.2), or `None` for the main window. An empty
/// string round-trips from a host that sends the field but leaves it blank, so it is treated
/// the same as absent.
fn agent_parent_key(input: &HookInput) -> Option<&str> {
    input.agent_id.as_deref().filter(|s| !s.is_empty())
}

/// T282 (D34): "Bash: cargo nextest run", "Edit: src/worktree/add.rs" — the tool name plus
/// the first 60 chars of its main argument, truncated again to 120 total. Never file
/// contents or prompt text: `tool_input` is scanned only for a short handful of well-known
/// argument keys, never serialized whole.
fn agent_activity(input: &HookInput) -> Option<String> {
    let tool = input.tool_name.as_deref()?;
    let arg = input.tool_input.as_ref().and_then(tool_main_argument);
    let activity = match arg {
        Some(a) => format!("{tool}: {}", first_chars(&a, 60)),
        None => tool.to_string(),
    };
    Some(first_chars(&activity, 120))
}

/// The one field, among a tool's several, that names what it acts on — never the body of a
/// `Write`/`Edit` or the text of a prompt.
fn tool_main_argument(input: &Value) -> Option<String> {
    ["command", "file_path", "path", "pattern", "query", "url"]
        .iter()
        .find_map(|key| input.get(key).and_then(Value::as_str))
        .map(str::to_string)
}

fn first_chars(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    s.chars().take(n).collect()
}

/// T283 (D34): the line SessionStart/SubagentStart offer so an agent learns its own rtok id.
/// Fixed wording, byte-stable but for the id; fed into the same budgeted `inject::apply`
/// path as every other offering (`inject_event`) rather than a parallel one. `None` when
/// `agent` is `None` — `[agents] enabled = false` or the host id could not be resolved
/// (`dispatch` already folds both into that one `Option`).
/// T442: `task <id> <title>` for the claim this agent holds in the checkout `cx.cwd` names.
/// The newest row wins. A title's first line only, so the offering stays one line.
fn claim_injection(cx: &Runtime, agent: Option<&str>) -> Option<Injection> {
    let agent = agent?;
    let cwd = cx.cwd.as_deref()?;
    let project = crate::project::project_key(std::path::Path::new(cwd))?;
    let row = cx.store.latest_task_claim(&project, agent).ok()??;
    let title = row.title.split(['\n', '\r']).next().unwrap_or("").trim();
    let text = format!("task {} {title}", row.task_id);
    Some(Injection {
        plugin: "task_claim",
        text: text.trim_end().to_string(),
        // Under the agent id (10) and checkpoint recall (9), so those stay when the budget
        // is tight. Stable for a given claim row.
        priority: 8,
    })
}

fn agent_id_injection(agent: Option<&str>) -> Option<Injection> {
    let id = agent?;
    let short = first_chars(id, 8);
    Some(Injection {
        plugin: "agent_id",
        text: format!(
            "rtok agent id: {short} (full: {id}). Use it with rtok's agent_* and worktree_* MCP tools; agent_inbox reads messages sent to you."
        ),
        // Highest offered priority (`Inject`'s own compact/startup recall tops out at 9): a
        // few words wide, so it never meaningfully competes with a real offering for budget,
        // and an agent that cannot see its own id cannot use `agent_*`/`worktree_*` at all.
        priority: 10,
    })
}

/// T283: Claude Code's `SessionStart` hook may run with `CLAUDE_ENV_FILE` set to a path it
/// then runs as a script preamble before every Bash command of the session (confirmed
/// 2026-09-27 against <https://code.claude.com/docs/en/hooks-guide> "Reload environment when
/// directory or files change" — `SessionStart`/`CwdChanged` "write to `CLAUDE_ENV_FILE`,
/// which Claude Code runs as a script preamble before each Bash command" — and the reference
/// entry it links to, <https://code.claude.com/docs/en/hooks#persist-environment-variables>).
/// Appending `export RTOK_AGENT_ID=<uuid>` there lets `rtok` invoked from the agent's own
/// Bash tool resolve its caller without the T281 host rule. Idempotent (a resumed session can
/// run `SessionStart` more than once) and fail-open: no env var, no file, or a write error
/// each leave the hook's exit-0 contract untouched.
fn write_agent_env_file(agent: Option<&str>) {
    let Some(id) = agent else { return };
    let Some(path) = std::env::var_os("CLAUDE_ENV_FILE").filter(|p| !p.is_empty()) else {
        return;
    };
    let line = format!("export RTOK_AGENT_ID={id}");
    if let Ok(existing) = std::fs::read_to_string(&path)
        && existing.lines().any(|l| l == line)
    {
        return;
    }
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let _ = writeln!(f, "{line}");
}

/// `Some(message)` when `[hook] max_ms` is non-zero and the event ran over budget.
/// `max_ms = 0` disables the budget. Pure so the slow path stays one `eprintln!`
/// plus one `warn` log row (see `note_slow`, T170).
fn slow_note(ms: f64, max_ms: u64, event: &str) -> Option<String> {
    if max_ms > 0 && ms > max_ms as f64 {
        Some(format!(
            "hook {event} slow: {ms:.1} ms over max_ms {max_ms} ms"
        ))
    } else {
        None
    }
}

/// T170: an over-budget event is logged at `warn` through the log funnel, not only printed
/// to stderr — hook stderr never reaches the operator, so `eprintln!` alone left `rtok.log`
/// and the `logs` table empty. `Runtime::log` fails open, so a dead store still exits 0 (D1).
fn note_slow(cx: &Runtime, event: &str, ms: f64) {
    if let Some(note) = slow_note(ms, cx.config.hook.max_ms, event) {
        eprintln!("rtok: {note}");
        cx.log("warn", "hook", event, &note);
    }
}

/// T204: a panicking plugin used to be indistinguishable from one returning `None` — the
/// `catch_unwind` `Err` was dropped with `.ok()`/`let _`, so `rtok doctor` / `rtok logs` never
/// saw it. One funnel for the four per-plugin loops below: extract the payload and log it at
/// `error` before the caller drops the plugin's output and moves on (fail open, D1). Runs only
/// on the panic path, so the budget stays untouched the rest of the time.
fn panic_message(err: &Box<dyn std::any::Any + Send>) -> String {
    err.downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| err.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".into())
}

fn log_panic(cx: &Runtime, plugin: &str, event: &str, err: Box<dyn std::any::Any + Send>) {
    let payload = panic_message(&err);
    // `source = "plugin"`, `name = <plugin id>` matches the funnel's existing convention
    // (see `insert_log`'s callers) — a `rtok logs`/`doctor` reader can filter on the plugin.
    cx.log(
        "error",
        "plugin",
        plugin,
        &format!("{event} panicked: {payload}"),
    );
}

fn dispatch_owned(stdin: &[u8], event: &str, cfg: &Config) -> Vec<u8> {
    match panic::catch_unwind(AssertUnwindSafe(|| {
        dispatch_owned_strict(stdin, event, cfg)
    })) {
        Ok(Ok(out)) => out,
        _ => b"{}".to_vec(),
    }
}

fn dispatch_owned_strict(stdin: &[u8], event: &str, cfg: &Config) -> Result<Vec<u8>, String> {
    let mut input: HookInput =
        serde_json::from_slice(stdin).map_err(|e| format!("hook {event}: bad stdin: {e}"))?;
    // Grok Build sends its own envelope to every hook it runs, including the Claude and Cursor
    // hooks it imports, so its reserved `GROK_HOOK_EVENT` wins over `--host` (plan T98).
    let grok = cfg.hook.host == "grok" || std::env::var_os("GROK_HOOK_EVENT").is_some();
    let copilot = !grok && cfg.hook.host == "copilot";
    let cursor = !grok && cfg.hook.host == "cursor";
    let gemini = !grok && cfg.hook.host == "gemini";
    let codewhale = !grok && cfg.hook.host == "codewhale";
    let cline = !grok && cfg.hook.host == "cline";
    let codebuddy = !grok && cfg.hook.host == "codebuddy";
    if grok {
        input.adapt_grok(event);
    } else if cursor {
        input.adapt_cursor(event);
    } else if copilot {
        input.adapt_copilot(event);
    } else if gemini {
        input.adapt_gemini(event);
    } else if codewhale {
        input.adapt_codewhale(event);
    } else if cline {
        input.adapt_cline(event);
    } else if cfg.hook.host == "devin" {
        input.adapt_devin(event, std::env::var("DEVIN_PROJECT_DIR").ok());
    } else if cfg.hook.host == "commandcode" {
        input.adapt_commandcode(event, std::env::var("COMMANDCODE_PROJECT_DIR").ok());
    } else if input.hook_event_name.is_empty() {
        input.hook_event_name = event.to_string();
    }
    // The call row keeps what plugins read back later (the spawn-brief ledger, the read
    // window): an adapted host's input in Claude's field names, not its raw camelCase (T262.5).
    // Claude's own stdin is already that shape and stays byte-identical.
    let adapted =
        grok || copilot || cursor || gemini || codewhale || cline || cfg.hook.host == "devin";
    let stored = adapted.then(|| serde_json::to_vec(&input).ok()).flatten();
    let stdin = stored.as_deref().unwrap_or(stdin);
    let session = resolve_session(&input.session_id, &cfg.core.session_env, |k| {
        std::env::var(k).ok()
    });
    let wait = if std::env::var_os(DEFERRED_ENV).is_some() {
        crate::store::LockWait::STEADY
    } else {
        LOCK_WAIT
    };
    let mut cx = match Runtime::open_with(cfg.clone(), session, wait) {
        Ok(cx) => cx,
        Err(e) => {
            if crate::store::is_locked(e.as_ref()) {
                defer_session_end(cfg, &input.hook_event_name, stdin);
            }
            return Err(format!("hook {event}: store open: {e}"));
        }
    };
    // SessionStart carries `cwd` like every other event, so the session row is attributed
    // from the first hook of the run rather than whichever call happens to arrive first.
    cx.cwd = input.cwd.clone();
    // Plugin hooks plus leftover settings-file hooks deliver one call twice (T245).
    let id = input.tool_use_id.as_deref().filter(|id| !id.is_empty());
    cx.once = id.map(|id| format!("{}:{id}", input.hook_event_name));
    let out = dispatch(stdin, &input, &cx);
    if copilot {
        let parsed: HookOutput = serde_json::from_slice(&out).unwrap_or_default();
        return Ok(copilot_output(&parsed));
    }
    if cursor {
        let parsed: HookOutput = serde_json::from_slice(&out).unwrap_or_default();
        return Ok(cursor_output(&parsed));
    }
    if gemini {
        let parsed: HookOutput = serde_json::from_slice(&out).unwrap_or_default();
        return Ok(gemini_output(&parsed, &input.hook_event_name));
    }
    if codewhale {
        let parsed: HookOutput = serde_json::from_slice(&out).unwrap_or_default();
        return Ok(codewhale_output(&parsed, input.prompt.as_deref()));
    }
    if cline {
        let parsed: HookOutput = serde_json::from_slice(&out).unwrap_or_default();
        return Ok(cline_output(&parsed));
    }
    if codebuddy {
        let parsed: HookOutput = serde_json::from_slice(&out).unwrap_or_default();
        return Ok(codebuddy_output(&parsed));
    }
    Ok(out)
}

/// CodeBuddy Code (https://www.codebuddy.ai/docs/cli/hooks, checked 2026-10-11) sends Claude's
/// stdin and reads Claude's output envelope, except that a PreToolUse rewrite travels as
/// `hookSpecificOutput.modifiedInput` where Claude says `updatedInput`. Decisions, context and
/// `{}` pass through untouched.
pub fn codebuddy_output(out: &HookOutput) -> Vec<u8> {
    let Ok(mut v) = serde_json::to_value(out) else {
        return b"{}".to_vec();
    };
    if let Some(h) = v
        .get_mut("hookSpecificOutput")
        .and_then(serde_json::Value::as_object_mut)
        && let Some(input) = h.remove("updatedInput")
    {
        h.insert("modifiedInput".into(), input);
    }
    serde_json::to_vec(&v).unwrap_or_else(|_| b"{}".to_vec())
}

/// Gemini CLI (https://geminicli.com/docs/hooks/reference/, fetched 2026-09-22) reads a
/// top-level `{decision: "deny", reason}` to block a call (nothing nests under
/// `permissionDecision`) and `{hookSpecificOutput: {tool_input}}` to rewrite one; after a tool
/// it reads `{hookSpecificOutput: {additionalContext}}`, the key Claude uses too. `{}` stays.
pub fn gemini_output(out: &HookOutput, event: &str) -> Vec<u8> {
    let empty = || b"{}".to_vec();
    let Some(h) = &out.hook_specific_output else {
        return empty();
    };
    if event == "PreToolUse" {
        if h.permission_decision.as_deref() == Some("deny") {
            let mut o = serde_json::Map::new();
            o.insert("decision".into(), "deny".into());
            if let Some(r) = &h.permission_decision_reason {
                o.insert("reason".into(), r.as_str().into());
            }
            return serde_json::to_vec(&serde_json::Value::Object(o)).unwrap_or_else(|_| empty());
        }
        if let Some(input) = &h.updated_input {
            let v = serde_json::json!({"hookSpecificOutput": {"tool_input": input}});
            return serde_json::to_vec(&v).unwrap_or_else(|_| empty());
        }
        return empty();
    }
    if let Some(ctx) = &h.additional_context {
        let v = serde_json::json!({"hookSpecificOutput": {"additionalContext": ctx}});
        return serde_json::to_vec(&v).unwrap_or_else(|_| empty());
    }
    empty()
}

/// CodeWhale's `message_submit` reads `{"text": "..."}` on stdout to replace the prompt
/// wholesale (https://github.com/Hmbown/Codewhale/blob/main/docs/HOOKS.md#message_submit,
/// fetched 2026-09-24) — there is no separate "add context" field like Claude's
/// `additionalContext`, so a `UserPromptSubmit` plugin's context is folded into a full
/// replacement text instead. `{}` (empty stdout) leaves the prompt unchanged, same as every
/// other host with nothing to add.
pub fn codewhale_output(out: &HookOutput, prompt: Option<&str>) -> Vec<u8> {
    let empty = || b"{}".to_vec();
    let Some(ctx) = out
        .hook_specific_output
        .as_ref()
        .and_then(|h| h.additional_context.as_deref())
    else {
        return empty();
    };
    let text = match prompt {
        Some(p) if !p.is_empty() => format!("{p}\n\n[hook context] {ctx}"),
        _ => ctx.to_string(),
    };
    serde_json::to_vec(&serde_json::json!({"text": text})).unwrap_or_else(|_| empty())
}

/// GitHub Copilot CLI reads a flat object: `{permissionDecision, permissionDecisionReason,
/// modifiedArgs}` on preToolUse, `{additionalContext}` after a tool; nothing nests under
/// `hookSpecificOutput`. A Claude `decision: block` becomes `deny`. `{}` stays `{}`.
pub fn copilot_output(out: &HookOutput) -> Vec<u8> {
    let mut o = serde_json::Map::new();
    let mut put = |k: &str, v: serde_json::Value| {
        o.insert(k.to_string(), v);
    };
    if let Some(h) = &out.hook_specific_output {
        if let Some(d) = &h.permission_decision {
            put("permissionDecision", d.as_str().into());
        }
        if let Some(r) = &h.permission_decision_reason {
            put("permissionDecisionReason", r.as_str().into());
        }
        if let Some(u) = &h.updated_input {
            put("modifiedArgs", u.clone());
        }
        if let Some(c) = &h.additional_context {
            put("additionalContext", c.as_str().into());
        }
    }
    if out.decision.as_deref() == Some("block") && !o.contains_key("permissionDecision") {
        o.insert("permissionDecision".into(), "deny".into());
        if let Some(r) = &out.reason {
            o.insert("permissionDecisionReason".into(), r.as_str().into());
        }
    }
    serde_json::to_vec(&serde_json::Value::Object(o)).unwrap_or_else(|_| b"{}".to_vec())
}

/// Cline file hooks read a flat object: `overrideInput` replaces the tool input (PreToolUse
/// only), `context` is injected into the next turn, `cancel: true` + `errorMessage` blocks.
/// `{}` means do nothing — and stays `{}` here. A Claude `decision: block` becomes `cancel`.
pub fn cline_output(out: &HookOutput) -> Vec<u8> {
    let mut o = serde_json::Map::new();
    let deny = out.decision.as_deref() == Some("block")
        || out
            .hook_specific_output
            .as_ref()
            .and_then(|h| h.permission_decision.as_deref())
            == Some("deny");
    if let Some(h) = &out.hook_specific_output {
        if let Some(u) = &h.updated_input
            && let Some(cmd) = u.get("command").and_then(|c| c.as_str())
        {
            o.insert(
                "overrideInput".into(),
                serde_json::json!({"commands": [cmd]}),
            );
        }
        if let Some(c) = &h.additional_context {
            o.insert("context".into(), c.as_str().into());
        }
        if deny && h.permission_decision_reason.is_some() {
            o.insert(
                "errorMessage".into(),
                h.permission_decision_reason
                    .as_deref()
                    .unwrap_or_default()
                    .into(),
            );
        }
    }
    if deny {
        o.insert("cancel".into(), true.into());
        if !o.contains_key("errorMessage")
            && let Some(r) = &out.reason
        {
            o.insert("errorMessage".into(), r.as_str().into());
        }
    }
    serde_json::to_vec(&serde_json::Value::Object(o)).unwrap_or_else(|_| b"{}".to_vec())
}

pub fn dispatch(stdin: &[u8], input: &HookInput, cx: &Runtime) -> Vec<u8> {
    let start = Instant::now();
    let registry = Registry::new(&cx.config);
    // T282 (D34): every event registers or touches this session's rtok agent id — one
    // indexed upsert (two when `agent_id` names a sub-agent, so its `parent_id` is set from
    // the row's first insert). Never lets a store error reach the fail-open hook (D1); skipped
    // entirely when the host id could not be resolved (`agents.host_id` is `NOT NULL` — SQLite
    // treats NULL as distinct in a UNIQUE index, so a NULL host_id would insert a fresh row on
    // every event instead of upserting). `SessionEnd` below reuses this same id rather than
    // upserting a second time.
    let agent = cx
        .config
        .agents
        .enabled
        .then(|| cx.host_id())
        .flatten()
        .and_then(|host_id| {
            let id = cx
                .store
                .register_agent(
                    host_id,
                    &cx.session,
                    agent_parent_key(input),
                    cx.cwd.as_deref(),
                    agent_activity(input).as_deref(),
                )
                .ok()?;
            // T283.3: what lets an `rtok mcp` process find this row by its host ancestor.
            if let Some(pid) = cx
                .config
                .hook_client_pid
                .and_then(|p| i32::try_from(p).ok())
            {
                let chain = rtok_sys::ancestors(pid, crate::agents::link::ANCESTORS);
                let _ = cx.store.set_agent_ancestors(&id, &chain);
            }
            Some(id)
        });
    let parent = match cx.record_call("hook", "hook", Some(&input.hook_event_name)) {
        Ok(id) => Some(id),
        // T178: another process held the writer lock past `LOCK_WAIT`. Every later write would
        // wait again, so pass the input through unchanged and record nothing.
        Err(e) if crate::store::is_locked(e.as_ref()) => {
            let msg = format!("skipped: store locked: {e:#}");
            eprintln!("rtok: hook {} {msg}", input.hook_event_name);
            crate::log::append(&cx.config, "error", "hook", &input.hook_event_name, &msg);
            defer_session_end(&cx.config, &input.hook_event_name, stdin);
            return b"{}".to_vec();
        }
        Err(e) => {
            crate::log::append(
                &cx.config,
                "error",
                "hook",
                &input.hook_event_name,
                &format!("record_call failed: {e:#}"),
            );
            None
        }
    };
    let out = match input.hook_event_name.as_str() {
        "PreToolUse" => pre_tool(input, cx, &registry),
        "PostToolUse" => post_tool(input, cx, &registry, agent.as_deref()),
        "AfterMCPExecution" => after_mcp(input, cx),
        // T295: Claude Code's PostCompact has no decision control and rejects any
        // `hookSpecificOutput` (code.claude.com/docs/en/hooks, checked 2026-09-27); its
        // checkpoint arrives on SessionStart source=compact. `compact_summary` is Claude's own
        // input field: Codex (`turn_id`) and Devin's `PostCompaction` still inject below.
        "PostCompact"
            if cx.config.hook.host == "claude" && input.extra.contains_key("compact_summary") =>
        {
            HookOutput::default()
        }
        "SessionStart" => {
            write_agent_env_file(agent.as_deref());
            // T329.6: one upsert beside `register_agent`'s; a locked store only skips it.
            if cx.config.plugins.graph.auto_add_projects
                && let Some(cwd) = cx.cwd.as_deref()
            {
                let _ = cx.store.auto_add_project(
                    std::path::Path::new(cwd),
                    crate::store::Origin::Session,
                    None,
                );
            }
            // T289.5: a worktree a post-create script left without an agent goes to the first
            // session that starts inside it.
            if let (Some(id), Some(cwd)) = (agent.as_deref(), cx.cwd.as_deref()) {
                let _ = crate::worktree::claim::complete_pending(
                    &cx.store,
                    std::path::Path::new(cwd),
                    id,
                );
            }
            // T428: memory recall, the handoff and `inject` each record a measurement; one
            // commit for the three instead of one lock wait each.
            cx.defer_measurements();
            let out = inject_event(input, cx, &registry, agent.as_deref());
            cx.flush_measurements();
            out
        }
        "UserPromptSubmit" | "PostCompact" | "SubagentStart" => {
            inject_event(input, cx, &registry, agent.as_deref())
        }
        "PreCompact" => {
            if let Some(ev) = input.pre_compact() {
                for p in registry.enabled() {
                    if let Err(e) =
                        panic::catch_unwind(AssertUnwindSafe(|| p.pre_compact(&ev, &Ctx::new(cx))))
                    {
                        log_panic(cx, p.manifest().id, "PreCompact", e);
                    }
                }
            }
            // T454: titles and up to two observations, so the host's compaction sees them.
            // No notes and no observations in this session stays `{}`.
            #[cfg(feature = "memory")]
            let extra = crate::plugins::memory::compact_context(&Ctx::new(cx));
            #[cfg(not(feature = "memory"))]
            let extra: Option<Injection> = None;
            if let Some(inj) = extra {
                HookOutput {
                    hook_specific_output: Some(HookSpecificOutput {
                        hook_event_name: "PreCompact".into(),
                        additional_context: Some(inj.text),
                        ..HookSpecificOutput::default()
                    }),
                    ..HookOutput::default()
                }
            } else {
                HookOutput::default()
            }
        }
        "SessionEnd" => {
            #[cfg(feature = "inject")]
            {
                let path = input.transcript_path.as_deref().unwrap_or("");
                let _ = panic::catch_unwind(AssertUnwindSafe(|| {
                    let _ = crate::plugins::checkpoint::save_session(path, &Ctx::new(cx));
                }));
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            // T282: end this event's own agent row (the main window, or the sub-agent named
            // by its `agent_id`) — reuse the id `register_agent` above already resolved,
            // rather than upserting the row a second time.
            if let Some(id) = &agent {
                let _ = cx.store.end_agent(id, now);
            }
            if cx
                .store
                .end_session(&cx.session, now)
                .is_err_and(|e| crate::store::is_locked(&e))
            {
                defer_session_end(&cx.config, "SessionEnd", stdin);
            }
            HookOutput::default()
        }
        _ => HookOutput::default(),
    };
    let bytes = serde_json::to_vec(&out).unwrap_or_else(|_| b"{}".to_vec());
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    note_slow(cx, &input.hook_event_name, ms);
    if let Some(id) = parent {
        let _ = cx.store.set_call_ms(id, ms);
        let cap = cx.config.core.call_io_inline_bytes as usize;
        let _ = cx.store.insert_hook_call_io(id, stdin, Some(&bytes), cap);
    }
    if matches!(input.hook_event_name.as_str(), "Stop" | "SessionEnd") {
        crate::otel::export::spawn_child(cx);
    }
    bytes
}

/// Set on the child [`defer_session_end`] spawns: wait like any command, never defer again.
const DEFERRED_ENV: &str = "RTOK_HOOK_DEFERRED";

/// T83.15: `SessionEnd`'s write is the one nothing repeats — lost to another writer's lock
/// (the flush child `Stop` spawned, still writing its watermarks) the session never gets
/// `ended_at` and its OTel root span never ships. Rather than wait past the hook's 10 ms, hand
/// the event to a detached `rtok hook SessionEnd` that waits `LockWait::STEADY` — the same
/// hand-off `Stop` uses for the flush. `stdin` is Claude-shaped (adapted hosts included), so
/// the child needs no `--host`. Any other event, or the deferred child itself: nothing.
fn defer_session_end(cfg: &Config, event: &str, stdin: &[u8]) {
    if event != "SessionEnd" || std::env::var_os(DEFERRED_ENV).is_some() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["hook", "SessionEnd"])
        .env(DEFERRED_ENV, "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if !cfg.home.as_os_str().is_empty() {
        cmd.env("RTOK_HOME", &cfg.home);
    }
    // As in `otel::export::spawn_child`: the child must not hold the agent's pipes open.
    rtok_sys::stop_inheriting_own_stdio();
    if let Ok(mut child) = cmd.spawn()
        && let Some(mut pipe) = child.stdin.take()
    {
        // A SessionEnd payload is a few hundred bytes, well under a pipe buffer: no block.
        let _ = pipe.write_all(stdin);
    }
}

/// Cursor's `afterMCPExecution` docs (https://cursor.com/docs/agent/hooks, fetched 2026-09-24)
/// list no output shape for this event — it is audit-only. `postToolUse`'s
/// `updated_mcp_tool_output` is the documented replacement key (T70.4), and `post_tool`'s
/// `cursor_mcp_output` already fills it via `mcp::wrap::shorten_result` (per-block, `isError`
/// skip, `Measurement` row — D21: one call path). So `afterMCPExecution` stays byte-passthrough
/// rather than re-shortening the same result a second time on a second, divergent path (T190).
fn after_mcp(_input: &HookInput, _cx: &Runtime) -> HookOutput {
    HookOutput::default()
}

fn pre_tool(input: &HookInput, cx: &Runtime, registry: &Registry) -> HookOutput {
    let Some(ev) = input.pre_tool() else {
        return HookOutput::default();
    };
    let mut rewrite: Option<PreToolDecision> = None;
    for p in registry.enabled() {
        let got = match panic::catch_unwind(AssertUnwindSafe(|| {
            p.pre_tool(&ev, &Ctx::with_agent(cx, input.agent_id.as_deref()))
        })) {
            Ok(v) => v,
            Err(e) => {
                log_panic(cx, p.manifest().id, "PreToolUse", e);
                None
            }
        };
        match got {
            Some(PreToolDecision::Deny { reason }) => {
                return HookOutput {
                    hook_specific_output: Some(HookSpecificOutput {
                        hook_event_name: "PreToolUse".into(),
                        permission_decision: Some("deny".into()),
                        permission_decision_reason: Some(reason),
                        ..HookSpecificOutput::default()
                    }),
                    ..HookOutput::default()
                };
            }
            Some(r @ PreToolDecision::Rewrite { .. }) => rewrite = Some(r),
            None => {}
        }
    }
    if let Some(PreToolDecision::Rewrite { input, reason }) = rewrite {
        return HookOutput {
            hook_specific_output: Some(HookSpecificOutput {
                hook_event_name: "PreToolUse".into(),
                updated_input: Some(input),
                permission_decision_reason: Some(reason),
                ..HookSpecificOutput::default()
            }),
            ..HookOutput::default()
        };
    }
    HookOutput::default()
}

fn post_tool(
    input: &HookInput,
    cx: &Runtime,
    registry: &Registry,
    agent: Option<&str>,
) -> HookOutput {
    let Some(ev) = input.post_tool() else {
        return HookOutput::default();
    };
    let mut parts = Vec::new();
    for p in registry.enabled() {
        match panic::catch_unwind(AssertUnwindSafe(|| {
            p.post_tool(&ev, &Ctx::with_agent(cx, input.agent_id.as_deref()))
        })) {
            Ok(Some(s)) => parts.push(s),
            Ok(None) => {}
            Err(e) => log_panic(cx, p.manifest().id, "PostToolUse", e),
        }
    }
    // T288: pushed messages lead, so `cap_budget` keeps them over later plugin context.
    let push = push::pending(cx, agent);
    if let Some(p) = &push {
        parts.insert(0, p.text.clone());
    }
    let text = cap_budget(cx, &parts.join("\n"));
    if let Some(p) = &push {
        p.settle(cx, &text);
    }
    let updated = cursor_mcp_output(input, cx);
    if text.is_empty() && updated.is_none() {
        return HookOutput::default();
    }
    HookOutput {
        hook_specific_output: Some(HookSpecificOutput {
            hook_event_name: "PostToolUse".into(),
            additional_context: (!text.is_empty()).then_some(text),
            updated_mcp_tool_output: updated,
            ..HookSpecificOutput::default()
        }),
        ..HookOutput::default()
    }
}

/// Cursor reads a flat object: `{updated_mcp_tool_output}` after an MCP tool,
/// `{additional_context}` on `sessionStart`/`postToolUse`. Anything else (a guard deny, a
/// shell hook with no replacement) keeps the Claude `hookSpecificOutput` shape.
pub fn cursor_output(out: &HookOutput) -> Vec<u8> {
    let nested = || serde_json::to_vec(out).unwrap_or_else(|_| b"{}".to_vec());
    let Some(h) = &out.hook_specific_output else {
        return nested();
    };
    let mut o = serde_json::Map::new();
    if let Some(updated) = &h.updated_mcp_tool_output {
        o.insert("updated_mcp_tool_output".into(), updated.clone());
    }
    if let Some(c) = &h.additional_context {
        o.insert("additional_context".into(), c.as_str().into());
    }
    if o.is_empty() {
        return nested();
    }
    serde_json::to_vec(&serde_json::Value::Object(o)).unwrap_or_else(|_| b"{}".to_vec())
}

fn is_rtok_mcp(input: &HookInput) -> bool {
    if input
        .extra
        .get("mcp_server_name")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|s| s.eq_ignore_ascii_case("rtok"))
    {
        return true;
    }
    let raw = input.tool_name.as_deref().unwrap_or("");
    raw.strip_prefix("MCP:").unwrap_or(raw) == "expand"
}

fn mcp_result(response: &serde_json::Value) -> Option<serde_json::Value> {
    if response
        .get("content")
        .and_then(serde_json::Value::as_array)
        .is_some()
    {
        return Some(response.clone());
    }
    if response
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(serde_json::Value::as_array)
        .is_some()
    {
        return Some(response["result"].clone());
    }
    None
}

fn cursor_mcp_output(input: &HookInput, cx: &Runtime) -> Option<serde_json::Value> {
    if cx.config.hook.host != "cursor" || is_rtok_mcp(input) {
        return None;
    }
    // `shorten_result` rewrites it in place; without the `cmd` plugin nothing does.
    #[cfg_attr(not(feature = "cmd"), allow(unused_mut))]
    let mut result = mcp_result(input.tool_response.as_ref()?)?;
    #[cfg(feature = "cmd")]
    {
        let settings = crate::plugins::cmd::rules::Settings::from_config(&cx.config);
        let server = input
            .extra
            .get("mcp_server_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("mcp");
        let raw = input.tool_name.as_deref().unwrap_or("tool");
        let tool = raw.strip_prefix("MCP:").unwrap_or(raw);
        if crate::mcp::wrap::shorten_result(
            cx,
            &settings,
            server,
            tool,
            &mut result,
            "archive",
            "mcp",
        ) {
            return Some(result);
        }
    }
    #[cfg(not(feature = "cmd"))]
    let _ = result;
    None
}

fn inject_event(
    input: &HookInput,
    cx: &Runtime,
    registry: &Registry,
    agent: Option<&str>,
) -> HookOutput {
    let mut inj = Vec::new();
    for p in registry.enabled() {
        let one = match panic::catch_unwind(AssertUnwindSafe(|| {
            if let Some(ev) = input.session_start() {
                p.session_start(&ev, &Ctx::new(cx))
            } else if let Some(ev) = input.prompt_submit() {
                p.prompt_submit(&ev, &Ctx::new(cx))
            } else if input.hook_event_name == "PostCompact" {
                p.session_start(&SessionStart { source: "compact" }, &Ctx::new(cx))
            } else if let Some(ev) = input.subagent_start() {
                // The parent's own ledger, not the new subagent's (T130): it has none yet.
                p.subagent_start(&ev, &Ctx::new(cx))
            } else {
                None
            }
        })) {
            Ok(v) => v,
            Err(e) => {
                log_panic(cx, p.manifest().id, &input.hook_event_name, e);
                None
            }
        };
        if let Some(i) = one {
            inj.push(i);
        }
    }
    // T283: the agent's own id, SessionStart/SubagentStart only — never UserPromptSubmit or
    // a post-compaction re-inject, which already knows it.
    if matches!(
        input.hook_event_name.as_str(),
        "SessionStart" | "SubagentStart"
    ) && let Some(i) = agent_id_injection(agent)
    {
        inj.push(i);
    }
    // T442: the task this agent claimed, one line, SessionStart and PostCompact only. Fail
    // open: a store error or a cwd that is not a checkout adds nothing. No measurement of
    // its own; it rides the same budget as every other offering.
    if matches!(
        input.hook_event_name.as_str(),
        "SessionStart" | "PostCompact"
    ) && let Some(i) = claim_injection(cx, agent)
    {
        inj.push(i);
    }
    // T288: the agent's undelivered messages, on UserPromptSubmit only (PostToolUse pushes
    // through `post_tool`); an empty inbox offers nothing.
    let push = (input.hook_event_name == "UserPromptSubmit")
        .then(|| push::pending(cx, agent))
        .flatten();
    if let Some(p) = &push {
        inj.push(p.injection());
    }
    // No offerings → no Measurement noise (D3): UserPromptSubmit usually has none.
    if inj.is_empty() {
        return HookOutput::default();
    }
    #[cfg(feature = "inject")]
    let text = crate::plugins::inject::apply(&Ctx::new(cx), inj);
    #[cfg(not(feature = "inject"))]
    let text = {
        let _ = inj;
        String::new()
    };
    if let Some(p) = &push {
        p.settle(cx, &text);
    }
    if text.is_empty() {
        return HookOutput::default();
    }
    HookOutput {
        hook_specific_output: Some(HookSpecificOutput {
            hook_event_name: input.hook_event_name.clone(),
            additional_context: Some(text),
            ..HookSpecificOutput::default()
        }),
        ..HookOutput::default()
    }
}

/// Fit PostToolUse context to `plugins.inject.budget_tokens` (D5). Whatever does not fit is
/// archived once and named by a `dropped:post_tool:<est> · expand: rtok expand <id>` marker
/// (T189). The marker always survives: kept lines are given back until it fits, so a cut
/// never loses the rest without an id.
fn cap_budget(cx: &Runtime, text: &str) -> String {
    let budget = cx.config.plugins.inject.budget_tokens;
    if cx.estimate(text, Class::Prose) <= budget {
        return text.to_string();
    }
    let lines: Vec<&str> = text.lines().collect();
    let mut keep = 0;
    while keep < lines.len() && cx.estimate(&lines[..=keep].join("\n"), Class::Prose) <= budget {
        keep += 1;
    }
    if keep == lines.len() {
        return lines.join("\n");
    }
    // The id is 64 hex digits whatever the bytes, so a placeholder of the same length
    // prices the marker before anything is archived.
    let marker_for = |dropped: &str, id: &str| {
        let est = cx.estimate(dropped, Class::Prose);
        if id.is_empty() {
            format!("dropped:post_tool:{est}")
        } else {
            format!("dropped:post_tool:{est} · expand: rtok expand {id}")
        }
    };
    let placeholder = "0".repeat(64);
    while keep > 0 {
        let dropped = lines[keep..].join("\n");
        let with = format!(
            "{}\n{}",
            lines[..keep].join("\n"),
            marker_for(&dropped, &placeholder)
        );
        if cx.estimate(&with, Class::Prose) <= budget {
            break;
        }
        keep -= 1;
    }
    let dropped = lines[keep..].join("\n");
    let id = rtok_plugin_sdk::Archive::put_archive(cx, dropped.as_bytes()).unwrap_or_default();
    let marker = marker_for(&dropped, &id);
    if keep > 0 {
        return format!("{}\n{marker}", lines[..keep].join("\n"));
    }
    // Estimates round up per part, so a prefix that fits the room left after `\n{marker}`
    // keeps the whole line under budget. The archive holds the whole line, prefix included.
    let room = budget.saturating_sub(cx.estimate(&format!("\n{marker}"), Class::Prose));
    let prefix = crate::plugin::fit_budget(&Ctx::new(cx), lines[0], Class::Prose, room);
    if !prefix.is_empty() {
        return format!("{prefix}\n{marker}");
    }
    if cx.estimate(&marker, Class::Prose) <= budget {
        marker
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::{DashboardPage, Manifest, Plugin, PostToolUse, Surface};
    use types::{post_out, pre_out};

    /// Shared by every per-host `_output` test below: hook stdout bytes back to `Value`.
    fn json(bytes: Vec<u8>) -> serde_json::Value {
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn copilot_output_shapes_pre_post_block_and_empty() {
        assert_eq!(
            json(copilot_output(&HookOutput::default())),
            serde_json::json!({})
        );

        let pre = pre_out(
            Some("allow"),
            Some("rtok"),
            Some(serde_json::json!({"command": "rtok cmd -- git status"})),
        );
        assert_eq!(
            json(copilot_output(&pre)),
            serde_json::json!({
                "permissionDecision": "allow",
                "permissionDecisionReason": "rtok",
                "modifiedArgs": {"command": "rtok cmd -- git status"}
            })
        );

        let post = post_out("ctx");
        assert_eq!(
            json(copilot_output(&post)),
            serde_json::json!({"additionalContext": "ctx"})
        );

        let block = HookOutput {
            decision: Some("block".into()),
            reason: Some("guard".into()),
            ..Default::default()
        };
        assert_eq!(
            json(copilot_output(&block)),
            serde_json::json!({"permissionDecision": "deny", "permissionDecisionReason": "guard"})
        );
    }
    #[test]
    fn gemini_output_shapes_deny_rewrite_context_and_empty() {
        assert_eq!(
            json(gemini_output(&HookOutput::default(), "PreToolUse")),
            serde_json::json!({})
        );

        let deny = pre_out(Some("deny"), Some("dup"), None);
        assert_eq!(
            json(gemini_output(&deny, "PreToolUse")),
            serde_json::json!({"decision": "deny", "reason": "dup"})
        );

        let rewrite = pre_out(
            None,
            None,
            Some(serde_json::json!({"command": "rtok cmd -- git status"})),
        );
        assert_eq!(
            json(gemini_output(&rewrite, "PreToolUse")),
            serde_json::json!({"hookSpecificOutput": {"tool_input": {"command": "rtok cmd -- git status"}}})
        );

        let post = post_out("ctx");
        assert_eq!(
            json(gemini_output(&post, "PostToolUse")),
            serde_json::json!({"hookSpecificOutput": {"additionalContext": "ctx"}})
        );
    }

    #[test]
    fn codewhale_output_folds_context_into_a_full_text_replacement_and_empty_stays_empty() {
        assert_eq!(
            json(codewhale_output(&HookOutput::default(), Some("hi"))),
            serde_json::json!({})
        );
        let post = post_out("ctx");
        assert_eq!(
            json(codewhale_output(&post, Some("hi"))),
            serde_json::json!({"text": "hi\n\n[hook context] ctx"})
        );
        // No original prompt (adapter found none): the context stands alone.
        assert_eq!(
            json(codewhale_output(&post, None)),
            serde_json::json!({"text": "ctx"})
        );
    }

    use crate::plugin::Runtime;

    #[test]
    fn fixture_pre_tool_is_valid_json() {
        let raw = include_str!("../../tests/fixtures/hooks/pre_tool_bash.json");
        let cx = Runtime::in_memory("b1e2c3d4-0000-4000-8000-000000000001").unwrap();
        let input: HookInput = serde_json::from_str(raw).unwrap();
        let out = dispatch(raw.as_bytes(), &input, &cx);
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert!(v.is_object());
        assert!(cx.store.count_kind("hook").unwrap() >= 1);
    }

    #[test]
    fn oversized_hook_call_io_does_not_archive() {
        let raw = include_str!("../../tests/fixtures/hooks/pre_tool_bash.json");
        let mut v: serde_json::Value = serde_json::from_str(raw).unwrap();
        v["pad"] = serde_json::Value::String("x".repeat(70_000));
        let stdin = serde_json::to_vec(&v).unwrap();
        assert!(stdin.len() > 65_536);
        let input: HookInput = serde_json::from_slice(&stdin).unwrap();
        let cx = Runtime::in_memory("b1e2c3d4-0000-4000-8000-000000000002").unwrap();
        let _ = dispatch(&stdin, &input, &cx);
        let ids = cx.store.call_ids_of_kind("hook").unwrap();
        assert!(!ids.is_empty());
        let (req, res) = cx.store.call_io_archives(ids[0]).unwrap();
        assert!(req.is_none(), "{req:?}");
        assert!(res.is_none(), "{res:?}");
        // T201: never archived (`archive_dir = None` on every hook call) must mean never
        // hashed either — `spill` skips the sha256 pass rather than computing one nothing
        // can ever expand.
        let (req_sha, _res_sha) = cx.store.call_io_shas(ids[0]).unwrap();
        assert!(req_sha.is_none(), "{req_sha:?}");
    }

    #[test]
    fn malformed_stdin_is_empty_object() {
        let cfg = Config::default();
        let mut out = Vec::new();
        run("PreToolUse", b"not-json".as_slice(), &mut out, &cfg);
        assert_eq!(out, b"{}");
    }

    #[test]
    fn cline_output_shapes_override_context_block_and_empty() {
        let json = |b: Vec<u8>| serde_json::from_slice::<serde_json::Value>(&b).unwrap();
        assert_eq!(
            json(cline_output(&HookOutput::default())),
            serde_json::json!({})
        );
        let pre = HookOutput {
            hook_specific_output: Some(HookSpecificOutput {
                hook_event_name: "PreToolUse".into(),
                updated_input: Some(serde_json::json!({"command": "rtok run -- 'git status'"})),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            json(cline_output(&pre)),
            serde_json::json!({
                "overrideInput": {"commands": ["rtok run -- 'git status'"]}
            })
        );
        let post = HookOutput {
            hook_specific_output: Some(HookSpecificOutput {
                hook_event_name: "PostToolUse".into(),
                additional_context: Some("ctx".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            json(cline_output(&post)),
            serde_json::json!({"context": "ctx"})
        );
        let block = HookOutput {
            decision: Some("block".into()),
            reason: Some("guard".into()),
            ..Default::default()
        };
        assert_eq!(
            json(cline_output(&block)),
            serde_json::json!({"cancel": true, "errorMessage": "guard"})
        );
    }

    #[test]
    fn codebuddy_output_renames_updated_input_and_passes_the_rest() {
        let json = |b: Vec<u8>| serde_json::from_slice::<serde_json::Value>(&b).unwrap();
        assert_eq!(
            json(codebuddy_output(&HookOutput::default())),
            serde_json::json!({})
        );
        let rewrite = pre_out(
            None,
            None,
            Some(serde_json::json!({"command": "rtok run -- ls"})),
        );
        let v = json(codebuddy_output(&rewrite));
        assert_eq!(
            v,
            serde_json::json!({"hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "modifiedInput": {"command": "rtok run -- ls"}
            }})
        );
        assert!(v["hookSpecificOutput"].get("updatedInput").is_none());
        let deny = pre_out(Some("deny"), Some("guard"), None);
        assert_eq!(
            json(codebuddy_output(&deny)),
            serde_json::to_value(&deny).unwrap()
        );
        let post = post_out("ctx");
        assert_eq!(
            json(codebuddy_output(&post)),
            serde_json::to_value(&post).unwrap()
        );
    }

    fn cline_cfg(dir: &std::path::Path) -> Config {
        let mut cfg = Config::default();
        cfg.hook.host = "cline".into();
        cfg.core.db_path = dir.join("rtok.db");
        cfg.core.archive_dir = dir.join("archive");
        cfg
    }

    #[test]
    fn codebuddy_pre_tool_use_rewrites_into_modified_input() {
        let dir = unique_dir("rtok-hook-codebuddy");
        let mut cfg = cline_cfg(&dir);
        cfg.hook.host = "codebuddy".into();
        let stdin = serde_json::to_vec(&serde_json::json!({
            "session_id": "cb-1",
            "cwd": "/tmp",
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "git status"}
        }))
        .unwrap();
        let out = dispatch_owned_strict(&stdin, "PreToolUse", &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let h = &v["hookSpecificOutput"];
        assert!(
            h["modifiedInput"]["command"]
                .as_str()
                .unwrap()
                .starts_with("rtok run"),
            "{v}"
        );
        assert!(h.get("updatedInput").is_none(), "{v}");
    }

    #[test]
    fn cline_pre_tool_use_rewrites_single_command_and_fails_open() {
        let dir = unique_dir("rtok-hook-cline");
        let cfg = cline_cfg(&dir);
        let stdin = serde_json::to_vec(&serde_json::json!({
            "hookName": "tool_call",
            "taskId": "cline-1",
            "workspaceRoots": ["/tmp"],
            "tool_call": {"id": "tc-1", "name": "run_commands", "input": {"commands": ["git status"]}}
        }))
        .unwrap();
        let out = dispatch_owned_strict(&stdin, "PreToolUse", &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let cmd = v["overrideInput"]["commands"][0].as_str().unwrap();
        assert!(cmd.starts_with("rtok run"), "{v}");

        for bad in [b"not-json".as_slice(), b"".as_slice()] {
            let mut out = Vec::new();
            run("PreToolUse", bad, &mut out, &cfg);
            assert_eq!(out, b"{}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T45.4: `core.session_env` resolves the session when stdin has none.
    #[test]
    fn resolve_session_prefers_stdin_then_env() {
        let env = |_: &str| Some("env-sess".to_string());
        assert_eq!(resolve_session("stdin-sess", "ANY_KEY", env), "stdin-sess");
        assert_eq!(resolve_session("", "ANY_KEY", env), "env-sess");
        assert_eq!(
            resolve_session("", "ANY_KEY", |_| Some("  padded  ".to_string())),
            "padded"
        );
        assert_eq!(resolve_session("", "", env), "unknown");
        assert_eq!(resolve_session("", "ANY_KEY", |_| None), "unknown");
        assert_eq!(
            resolve_session("", "ANY_KEY", |_| Some("   ".to_string())),
            "unknown"
        );
    }

    /// T45.4: `[hook] max_ms` fires only when non-zero and exceeded.
    #[test]
    fn slow_note_fires_only_over_budget() {
        assert!(slow_note(12.0, 10, "PreToolUse").is_some());
        assert_eq!(slow_note(9.9, 10, "PreToolUse"), None);
        assert_eq!(slow_note(500.0, 0, "PreToolUse"), None);
    }

    /// T170: an over-budget hook run lands one `warn` row in the log store, not
    /// just a stderr line; a run inside the budget adds no row. Deterministic:
    /// drives `note_slow` directly with fixed millisecond values rather than
    /// timing a real hook run.
    #[test]
    fn slow_hook_run_records_one_warn_row() {
        let mut cx = Runtime::in_memory("b1e2c3d4-0000-4000-8000-0000000000b1").unwrap();
        cx.config.hook.max_ms = 10;
        note_slow(&cx, "PreToolUse", 50.0);
        let rows = cx.store.logs_after(0, 10).unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].level, "warn");
        assert!(rows[0].message.contains("slow"), "{}", rows[0].message);

        note_slow(&cx, "PreToolUse", 5.0);
        let rows = cx.store.logs_after(0, 10).unwrap();
        assert_eq!(rows.len(), 1, "under-budget run added a row: {rows:?}");
    }

    /// T45.4: `fail_open = false` surfaces a bad payload instead of `{}`.
    #[test]
    #[should_panic(expected = "bad stdin")]
    fn strict_path_panics_on_malformed_stdin() {
        let cfg = Config {
            hook: crate::config::Hook {
                fail_open: false,
                ..Default::default()
            },
            ..Config::default()
        };
        let mut out = Vec::new();
        run("PreToolUse", b"not-json".as_slice(), &mut out, &cfg);
    }

    /// T45.4: the strict path runs a valid event (temp store, default env key unset).
    #[test]
    fn strict_path_runs_valid_input() {
        let dir = std::env::temp_dir().join(format!("rtok-hooks-t454-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = crate::testutil::config_in(&dir);
        cfg.hook.fail_open = false;
        // Hermetic regardless of ambient env: the fixture carries its own
        // session id, and the fallback key is a probe nothing else sets.
        cfg.core.session_env = "RTOK_T454_PROBE_SESSION".into();
        // The fixture carries its own session id, so the `session_env`
        // fallback is not exercised here; the probe key only proves the
        // lookup misses hermetically. No env mutation needed.
        let raw = include_str!("../../tests/fixtures/hooks/pre_tool_bash.json");
        let mut out = Vec::new();
        run("PreToolUse", raw.as_bytes(), &mut out, &cfg);
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert!(v.is_object());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T25.0 Check: a hook run leaves a `sessions` row with non-NULL `host_id` and
    /// `project` — resolved from `[hook] host` and the event's own `cwd`, not left `None`.
    #[test]
    fn hook_run_attributes_the_session() {
        let dir = std::env::temp_dir().join(format!("rtok-hooks-t25-{}", std::process::id()));
        let repo = dir.join("myproj");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let raw = include_str!("../../tests/fixtures/hooks/pre_tool_bash.json");
        let mut v: serde_json::Value = serde_json::from_str(raw).unwrap();
        v["cwd"] = serde_json::Value::String(repo.display().to_string());
        v["session_id"] = serde_json::Value::String("b1e2c3d4-0000-4000-8000-0000000000aa".into());
        let stdin = serde_json::to_vec(&v).unwrap();
        let input: HookInput = serde_json::from_slice(&stdin).unwrap();
        let mut cx = Runtime::in_memory(input.session_id.clone()).unwrap();
        cx.cwd = input.cwd.clone();
        let _ = dispatch(&stdin, &input, &cx);
        let (slug, project, cwd) = cx.store.session_row(&cx.session).unwrap().unwrap();
        assert_eq!(
            slug.as_deref(),
            Some("claude"),
            "default [hook] host resolves"
        );
        assert_eq!(project.as_deref(), Some("myproj"), "git root basename");
        assert_eq!(cwd.as_deref(), Some(repo.display().to_string()).as_deref());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T25.0 Check: `pi` records as `pi`, not `other` — 0010.sql seeds the slug
    /// `rtok agents install pi` installs but 0002.sql's original list never had.
    #[test]
    fn pi_host_resolves_to_pi_not_other() {
        let dir = std::env::temp_dir().join(format!("rtok-hooks-t25-pi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = crate::testutil::config_in(&dir);
        cfg.hook.host = "pi".into();
        let raw = include_str!("../../tests/fixtures/hooks/pre_tool_bash.json");
        let input: HookInput = serde_json::from_str(raw).unwrap();
        let cx = Runtime::open(cfg, input.session_id.clone()).unwrap();
        let _ = dispatch(raw.as_bytes(), &input, &cx);
        let (slug, ..) = cx.store.session_row(&cx.session).unwrap().unwrap();
        assert_eq!(slug.as_deref(), Some("pi"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A fresh, empty scratch dir named `<prefix>-<pid>-<nanos>`, unique per call.
    fn unique_dir(prefix: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `out` carries one `dropped:post_tool:` marker whose `rtok expand <sha256>` id fetches
    /// back exactly `want`.
    fn assert_marker_expands_to(cx: &Runtime, out: &str, want: &str) {
        let marker = out
            .lines()
            .find(|l| l.starts_with("dropped:post_tool:"))
            .unwrap_or_else(|| panic!("marker missing: {out}"));
        assert!(marker.contains(" · expand: rtok expand "), "{marker}");
        let id = marker.rsplit("rtok expand ").next().unwrap();
        assert!(
            id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit()),
            "{marker}"
        );
        let got = crate::expand::fetch(cx, id).unwrap().unwrap();
        assert_eq!(got, want.as_bytes(), "{marker}");
    }

    #[test]
    fn cap_budget_marks_drop_when_first_line_exceeds() {
        let mut cx = Runtime::in_memory("cap-first").unwrap();
        cx.config.plugins.inject.budget_tokens = 40;
        let huge = "word ".repeat(400);
        let want = huge.trim_end().to_string();
        let out = cap_budget(&cx, &want);
        assert!(!out.is_empty(), "must not swallow the whole payload");
        assert_marker_expands_to(&cx, &out, &want);
        assert!(cx.estimate(&out, Class::Prose) <= 40, "{out}");
    }

    /// Lines that fit alone but leave no room for the id-carrying marker give lines
    /// back: the marker is never dropped, and kept lines + `expand <id>` rebuild the input.
    #[test]
    fn cap_budget_gives_back_lines_so_the_marker_fits() {
        let mut cx = Runtime::in_memory("cap-giveback").unwrap();
        cx.config.plugins.inject.budget_tokens = 40;
        let text = (0..200)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = cap_budget(&cx, &text);
        assert!(cx.estimate(&out, Class::Prose) <= 40, "{out}");
        let (kept, marker) = out.rsplit_once('\n').expect("kept lines + marker");
        assert!(marker.starts_with("dropped:post_tool:"), "{out}");
        let id = marker.rsplit("rtok expand ").next().unwrap();
        assert_eq!(id.len(), 64, "{marker}");
        let rest = crate::expand::fetch(&cx, id).unwrap().unwrap();
        assert_eq!(
            format!("{kept}\n{}", String::from_utf8(rest).unwrap()),
            text
        );
    }

    #[test]
    fn cap_budget_keeps_fitting_lines_and_names_the_rest() {
        let mut cx = Runtime::in_memory("cap-rest").unwrap();
        cx.config.plugins.inject.budget_tokens = 30;
        let small = "ok";
        let huge = "word ".repeat(400);
        let want = huge.trim_end().to_string();
        let text = format!("{small}\n{want}");
        let out = cap_budget(&cx, &text);
        assert!(out.starts_with("ok\n"), "{out}");
        assert_marker_expands_to(&cx, &out, &want);
        assert!(cx.estimate(&out, Class::Prose) <= 30, "{out}");
    }

    #[test]
    fn cursor_session_start_injects_flat_and_stable() {
        let dir = std::env::temp_dir().join(format!("rtok-cursor-ss-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = crate::testutil::config_in(&dir);
        cfg.hook.host = "cursor".into();
        cfg.plugins.inject.modes = vec!["nudges".into()];
        let raw = serde_json::json!({
            "hook_event_name": "sessionStart",
            "conversation_id": "sess-cur",
            "cwd": dir.display().to_string(),
            "source": "startup"
        });
        let stdin = serde_json::to_vec(&raw).unwrap();
        let mut out1 = Vec::new();
        run("SessionStart", stdin.as_slice(), &mut out1, &cfg);
        let mut out2 = Vec::new();
        run("SessionStart", stdin.as_slice(), &mut out2, &cfg);
        assert_eq!(out1, out2, "byte-stable");
        let v: serde_json::Value = serde_json::from_slice(&out1).unwrap();
        assert!(v.get("hookSpecificOutput").is_none(), "{v}");
        assert!(v.get("additional_context").is_some(), "{v}");
        let mut claude = cfg.clone();
        claude.hook.host = "claude".into();
        claude.plugins.inject.modes = vec!["nudges".into()];
        let mut claude_out = Vec::new();
        run(
            "SessionStart",
            include_str!("../../tests/fixtures/hooks/session_start.json").as_bytes(),
            &mut claude_out,
            &claude,
        );
        let cv: serde_json::Value = serde_json::from_slice(&claude_out).unwrap();
        let cursor_ctx = v["additional_context"].as_str().unwrap_or("");
        let claude_ctx = cv["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap_or("");
        // T283: both now lead with `rtok agent id: ...` — a different id per host/session
        // by design (two different registered agents), so the flat-shape claim is about
        // everything after that first line.
        assert!(cursor_ctx.starts_with("rtok agent id: "), "{cursor_ctx}");
        assert!(claude_ctx.starts_with("rtok agent id: "), "{claude_ctx}");
        fn after_first_line(s: &str) -> &str {
            s.split_once('\n').map_or("", |(_, rest)| rest)
        }
        assert_eq!(after_first_line(cursor_ctx), after_first_line(claude_ctx));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T295: Claude Code's PostCompact prints `{}` (it rejects `hookSpecificOutput`), while
    /// SessionStart source=compact carries the checkpoint. The same PostCompact without
    /// `compact_summary` (Codex's shape) still injects it.
    #[test]
    fn claude_post_compact_prints_empty_and_session_start_carries_the_checkpoint() {
        let dir = std::env::temp_dir().join(format!("rtok-t295-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = Config::default();
        cfg.core.db_path = dir.join("rtok.db");
        cfg.core.archive_dir = dir.join("archive");
        let post: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/hooks/post_compact.json"))
                .unwrap();
        let session = post["session_id"].clone();
        let pre = serde_json::json!({
            "hook_event_name": "PreCompact", "session_id": session,
            "cwd": post["cwd"], "transcript_path": "", "trigger": "auto"
        });
        let mut out = Vec::new();
        run("PreCompact", pre.to_string().as_bytes(), &mut out, &cfg);
        let context = |event: &str, input: &serde_json::Value| {
            let mut out = Vec::new();
            run(event, input.to_string().as_bytes(), &mut out, &cfg);
            out
        };
        assert_eq!(
            String::from_utf8_lossy(&context("PostCompact", &post)),
            "{}"
        );

        let start = serde_json::json!({
            "hook_event_name": "SessionStart", "session_id": session,
            "cwd": post["cwd"], "source": "compact"
        });
        let v: serde_json::Value =
            serde_json::from_slice(&context("SessionStart", &start)).unwrap();
        assert_eq!(
            v["hookSpecificOutput"]["hookEventName"], "SessionStart",
            "{v}"
        );
        let text = v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap_or("");
        assert!(text.contains("checkpoint"), "{v}");

        let mut codex = post.clone();
        codex.as_object_mut().unwrap().remove("compact_summary");
        let v: serde_json::Value = serde_json::from_slice(&context("PostCompact", &codex)).unwrap();
        let text = v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap_or("");
        assert!(text.contains("checkpoint"), "{v}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn cursor_cfg(dir: &std::path::Path) -> Config {
        let mut c = crate::testutil::config_in(dir);
        c.hook.host = "cursor".into();
        c
    }

    /// T98: a Grok PreToolUse on `run_terminal_command` is rewritten and answered in Claude's
    /// shape, which Grok reads unchanged.
    #[cfg(feature = "cmd")]
    #[test]
    fn grok_pre_tool_use_rewrites_the_terminal_command() {
        let dir = std::env::temp_dir().join(format!("rtok-hook-grok-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut cfg = cursor_cfg(&dir);
        cfg.hook.host = "grok".into();
        let stdin = serde_json::to_vec(&serde_json::json!({
            "hookEventName": "pre_tool_use",
            "hook_event_name": "PreToolUse",
            "sessionId": "grok-1",
            "cwd": dir.to_string_lossy(),
            "toolName": "run_terminal_command",
            "toolInput": {"command": "git status"}
        }))
        .unwrap();
        let out = dispatch_owned_strict(&stdin, "PreToolUse", &cfg).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let hso = &v["hookSpecificOutput"];
        assert_eq!(hso["hookEventName"], "PreToolUse", "{v}");
        let cmd = hso["updatedInput"]["command"].as_str().unwrap_or("");
        assert!(cmd.contains("git status") && cmd != "git status", "{v}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T390: `sessionEnd` (stdin names it in camelCase) ends the agent row. T390.1: the
    /// `beforeSubmitPrompt` hook an upgraded T390 install still has registered stays harmless,
    /// still answering with valid JSON.
    #[test]
    fn cursor_session_end_hook_ends_the_agent_and_a_stale_prompt_hook_fails_open() {
        let dir = unique_dir("rtok-hook-t390-cursor");
        let mut cfg = cursor_cfg(&dir);
        cfg.plugins.inject.modes = vec!["nudges".into()];
        let run_event = |event: &str, stdin: serde_json::Value| {
            dispatch_owned_strict(&serde_json::to_vec(&stdin).unwrap(), event, &cfg).unwrap()
        };
        let prompt = run_event(
            "UserPromptSubmit",
            serde_json::json!({
                "hook_event_name": "beforeSubmitPrompt",
                "conversation_id": "sess-cursor-390",
                "cwd": dir.to_string_lossy(),
                "prompt": "fix the build",
                "attachments": [],
            }),
        );
        assert!(json(prompt).is_object());

        let host_id = Runtime::open(cfg.clone(), "irrelevant")
            .unwrap()
            .host_id()
            .unwrap();
        let store = crate::store::Store::open(&cfg.core.db_path).unwrap();
        let id = store
            .register_agent(host_id, "sess-cursor-390", None, None, None)
            .unwrap();
        assert_eq!(store.agent_row(&id).unwrap().unwrap().ended_at, None);

        let end = run_event(
            "SessionEnd",
            serde_json::json!({
                "hook_event_name": "sessionEnd",
                "conversation_id": "sess-cursor-390",
                "session_id": "sess-cursor-390",
                "reason": "completed",
                "duration_ms": 45000,
            }),
        );
        assert_eq!(json(end), serde_json::json!({}));
        let row = store.agent_row(&id).unwrap().unwrap();
        assert!(row.ended_at.is_some(), "sessionEnd must end the agent row");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn mcp_stdin(server: &str, tool: &str, text: &str) -> Vec<u8> {
        let result = serde_json::json!({"content":[{"type":"text","text": text}]});
        serde_json::to_vec(&serde_json::json!({
            "hook_event_name": "postToolUse",
            "tool_name": tool,
            "tool_input": {},
            "tool_output": result.to_string(),
            "conversation_id": "s-mcp",
            "mcp_server_name": server
        }))
        .unwrap()
    }

    #[test]
    fn cursor_output_emits_snake_case_mcp_replacement() {
        assert_eq!(
            json(cursor_output(&HookOutput::default())),
            serde_json::json!({})
        );
        let out = HookOutput {
            hook_specific_output: Some(HookSpecificOutput {
                hook_event_name: "PostToolUse".into(),
                updated_mcp_tool_output: Some(serde_json::json!({"content":[]})),
                additional_context: Some("note".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let v = json(cursor_output(&out));
        assert_eq!(
            v["updated_mcp_tool_output"]["content"],
            serde_json::json!([])
        );
        assert_eq!(v["additional_context"], "note");
        assert!(v.get("hookSpecificOutput").is_none());
    }

    #[test]
    fn cursor_mcp_post_tool_use_shortens_only_foreign_long_results() {
        let dir = unique_dir("rtok-hook-mcp");
        let cfg = cursor_cfg(&dir);
        let long: String = (1..=200).map(|i| format!("line {i}\n")).collect();
        let out = dispatch_owned_strict(
            &mcp_stdin("linear", "MCP:list_issues", &long),
            "PostToolUse",
            &cfg,
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let printed = v["updated_mcp_tool_output"]["content"][0]["text"]
            .as_str()
            .unwrap_or("");
        assert!(printed.contains("expand: rtok expand "), "{v}");
        assert!(printed.len() < long.len(), "{printed}");
        let store = crate::store::Store::open(&cfg.core.db_path).unwrap();
        let rows = store.list_measurements("archive").unwrap();
        assert_eq!(
            rows.iter().filter(|r| r.kind == "mcp").count(),
            1,
            "{rows:?}"
        );

        let small = dispatch_owned_strict(
            &mcp_stdin("linear", "MCP:list_issues", "ok\n"),
            "PostToolUse",
            &cfg,
        )
        .unwrap();
        assert_eq!(small, b"{}");

        let own =
            dispatch_owned_strict(&mcp_stdin("rtok", "MCP:search", &long), "PostToolUse", &cfg)
                .unwrap();
        assert_eq!(own, b"{}", "rtok MCP results must not be rewritten");
        assert_eq!(
            store
                .list_measurements("archive")
                .unwrap()
                .iter()
                .filter(|r| r.kind == "mcp")
                .count(),
            1
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn after_mcp_stdin(server: &str, tool: &str, content: &serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "hook_event_name": "afterMCPExecution",
            "tool_name": tool,
            "conversation_id": "s-after-mcp",
            "mcp_server_name": server,
            "result_json": content.to_string()
        }))
        .unwrap()
    }

    /// T190: Cursor's `afterMCPExecution` docs (https://cursor.com/docs/agent/hooks, fetched
    /// 2026-09-24) list no output shape for this event — it is audit-only. `postToolUse`
    /// already shortens the same MCP result via `updated_mcp_tool_output`
    /// (`cursor_mcp_post_tool_use_shortens_only_foreign_long_results` above), so
    /// `AfterMCPExecution` must stay a pure byte-passthrough: no second shortening, no block
    /// duplication, and no `Measurement` row of its own — for an oversized two-block result
    /// and for an `isError` result alike.
    #[test]
    fn cursor_after_mcp_execution_is_byte_passthrough() {
        let dir = unique_dir("rtok-hook-after-mcp");
        let cfg = cursor_cfg(&dir);

        let long: String = (1..=200).map(|i| format!("line {i}\n")).collect();
        let two_blocks = serde_json::json!({
            "content": [
                {"type": "text", "text": long},
                {"type": "text", "text": "second block\n"}
            ]
        });
        let out = dispatch_owned_strict(
            &after_mcp_stdin("linear", "list_issues", &two_blocks),
            "AfterMCPExecution",
            &cfg,
        )
        .unwrap();
        assert_eq!(
            out, b"{}",
            "AfterMCPExecution has no documented output shape; postToolUse shortens instead"
        );
        assert_eq!(
            two_blocks["content"].as_array().unwrap().len(),
            2,
            "the original blocks must not have been touched"
        );

        let err = serde_json::json!({
            "isError": true,
            "content": [{"type": "text", "text": long}]
        });
        let out_err = dispatch_owned_strict(
            &after_mcp_stdin("linear", "list_issues", &err),
            "AfterMCPExecution",
            &cfg,
        )
        .unwrap();
        assert_eq!(out_err, b"{}");

        let store = crate::store::Store::open(&cfg.core.db_path).unwrap();
        assert_eq!(
            store
                .list_measurements("archive")
                .unwrap()
                .iter()
                .filter(|r| r.kind == "mcp")
                .count(),
            0,
            "AfterMCPExecution alone must not record a Measurement"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    struct PanicsOnPostTool;
    impl Plugin for PanicsOnPostTool {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "t204-panics",
                surfaces: &[Surface::Hook],
                default_on: true,
            }
        }
        fn dashboard_page(&self) -> DashboardPage {
            DashboardPage::new("t204-panics", "T204 test fixture.", true)
        }
        fn post_tool(&self, _ev: &PostToolUse, _cx: &Ctx) -> Option<String> {
            panic!("boom");
        }
    }

    struct SurvivesPostTool;
    impl Plugin for SurvivesPostTool {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "t204-survives",
                surfaces: &[Surface::Hook],
                default_on: true,
            }
        }
        fn dashboard_page(&self) -> DashboardPage {
            DashboardPage::new("t204-survives", "T204 test fixture.", true)
        }
        fn post_tool(&self, _ev: &PostToolUse, _cx: &Ctx) -> Option<String> {
            Some("good context".into())
        }
    }

    /// T204: a plugin that panics in `post_tool` must not take the rest of the dispatch down
    /// with it — the surviving plugin's context still reaches stdout — and its panic must land
    /// as one `level = "error"` row in the `logs` table naming the plugin, not vanish silently.
    #[test]
    fn a_panicking_plugin_is_logged_and_the_rest_survives() {
        let cx = Runtime::in_memory("t204-panic").unwrap();
        let registry = Registry::from_plugins(
            vec![Box::new(PanicsOnPostTool), Box::new(SurvivesPostTool)],
            &cx.config,
        );
        let input: HookInput =
            serde_json::from_str(include_str!("../../tests/fixtures/hooks/post_tool.json"))
                .unwrap();

        let out = post_tool(&input, &cx, &registry, None);

        let ctx = out
            .hook_specific_output
            .as_ref()
            .and_then(|h| h.additional_context.as_deref())
            .unwrap_or("");
        assert_eq!(ctx, "good context", "the surviving plugin's output {out:?}");
        assert!(
            !ctx.contains("boom"),
            "the panic payload must not leak into stdout"
        );

        assert_eq!(
            cx.store.count_logs("error", "t204-panics").unwrap(),
            1,
            "the panic must be logged exactly once, naming the plugin"
        );
        assert_eq!(
            cx.store.count_logs("error", "t204-survives").unwrap(),
            0,
            "the plugin that did not panic must not be logged"
        );
    }

    // ── T282 (D34): the rtok agent registry ────────────────────────────────────

    /// `pre_tool_bash.json` with `session_id` overridden, parsed both ways (the raw `Value`
    /// for further mutation, and the typed `HookInput`) plus a fresh in-memory `Runtime` for
    /// that same session — the shape every T282 dispatch-level test below starts from.
    fn agent_fixture(session_id: &str) -> (serde_json::Value, Vec<u8>, HookInput, Runtime) {
        let raw = include_str!("../../tests/fixtures/hooks/pre_tool_bash.json");
        let mut v: serde_json::Value = serde_json::from_str(raw).unwrap();
        v["session_id"] = serde_json::Value::String(session_id.into());
        let stdin = serde_json::to_vec(&v).unwrap();
        let input: HookInput = serde_json::from_slice(&stdin).unwrap();
        let cx = Runtime::in_memory(input.session_id.clone()).unwrap();
        (v, stdin, input, cx)
    }

    #[test]
    fn dispatch_registers_and_touches_the_agent_with_truncated_activity() {
        let (mut v, _, _, mut cx) = agent_fixture("agent-sess-1");
        v["tool_input"]["command"] = serde_json::Value::String(
            "cargo nextest run --no-fail-fast --release --workspace --all-targets --verbose".into(),
        );
        let stdin = serde_json::to_vec(&v).unwrap();
        let input: HookInput = serde_json::from_slice(&stdin).unwrap();
        cx.cwd = input.cwd.clone();
        let _ = dispatch(&stdin, &input, &cx);

        let id = cx
            .store
            .register_agent(cx.host_id().unwrap(), &cx.session, None, None, None)
            .unwrap();
        let row = cx.store.agent_row(&id).unwrap().unwrap();
        assert_eq!(row.host_session_id, "agent-sess-1");
        assert_eq!(row.parent_id, None);
        assert_eq!(
            row.activity.as_deref(),
            Some("Bash: cargo nextest run --no-fail-fast --release --workspace --all"),
            "tool name plus the first 60 chars of its main argument"
        );
    }

    /// T283.3: a hook that knows its client's pid records the ancestors above it on the agent
    /// row; a hook without one (an old client) leaves the row without any.
    #[test]
    fn a_known_client_pid_records_its_ancestors_on_the_agent_row() {
        let me = std::process::id();
        let chain = rtok_sys::ancestors(me as i32, crate::agents::link::ANCESTORS);
        let ancestors_after = |pid: Option<u32>, session: &str| {
            let (_, stdin, input, mut cx) = agent_fixture(session);
            cx.config.hook_client_pid = pid;
            let _ = dispatch(&stdin, &input, &cx);
            let id = cx
                .store
                .register_agent(cx.host_id().unwrap(), &cx.session, None, None, None)
                .unwrap();
            cx.store.agent_row(&id).unwrap().unwrap().ancestors
        };
        assert_eq!(ancestors_after(Some(me), "anc-sess-1"), chain);
        assert!(ancestors_after(None, "anc-sess-2").is_empty());
    }

    #[test]
    fn a_sub_agent_hook_event_registers_a_child_row_under_its_parent() {
        let (v, stdin, input, cx) = agent_fixture("agent-sess-2");
        let _ = dispatch(&stdin, &input, &cx); // the main window's own row

        let mut child = v.clone();
        child["agent_id"] = serde_json::Value::String("sub-1".into());
        let child_stdin = serde_json::to_vec(&child).unwrap();
        let child_input: HookInput = serde_json::from_slice(&child_stdin).unwrap();
        let _ = dispatch(&child_stdin, &child_input, &cx);

        let host_id = cx.host_id().unwrap();
        let parent_id = cx
            .store
            .register_agent(host_id, &cx.session, None, None, None)
            .unwrap();
        let child_id = cx
            .store
            .register_agent(host_id, &cx.session, Some("sub-1"), None, None)
            .unwrap();
        assert_ne!(parent_id, child_id);
        let child_row = cx.store.agent_row(&child_id).unwrap().unwrap();
        assert_eq!(child_row.parent_id.as_deref(), Some(parent_id.as_str()));
        assert_eq!(child_row.host_session_id, "agent-sess-2");
    }

    #[test]
    fn session_end_ends_the_agent_row() {
        let (v, stdin, input, cx) = agent_fixture("agent-sess-3");
        let _ = dispatch(&stdin, &input, &cx);
        let id = cx
            .store
            .register_agent(cx.host_id().unwrap(), &cx.session, None, None, None)
            .unwrap();
        assert_eq!(cx.store.agent_row(&id).unwrap().unwrap().ended_at, None);

        let mut end = v.clone();
        end["hook_event_name"] = serde_json::Value::String("SessionEnd".into());
        let end_stdin = serde_json::to_vec(&end).unwrap();
        let end_input: HookInput = serde_json::from_slice(&end_stdin).unwrap();
        let _ = dispatch(&end_stdin, &end_input, &cx);
        assert!(cx.store.agent_row(&id).unwrap().unwrap().ended_at.is_some());
    }

    #[test]
    fn agents_enabled_false_skips_registration() {
        let (_, stdin, input, mut cx) = agent_fixture("agent-sess-off");
        cx.config.agents.enabled = false;
        let _ = dispatch(&stdin, &input, &cx);
        assert!(
            cx.store.live_agents("1d").unwrap().is_empty(),
            "[agents] enabled = false must leave the agents table untouched"
        );
    }

    /// research.md §26: Cursor sends `conversation_id`, Cline sends `taskId` — neither is
    /// `session_id`, so this proves each host's adapter feeds its own field into the same
    /// `agents.host_session_id` the Claude-shaped hosts write directly.
    #[test]
    fn cursor_and_cline_register_under_their_own_session_id_field() {
        let dir = unique_dir("rtok-hook-t282-cursor");
        let cfg = cursor_cfg(&dir);
        let stdin = serde_json::to_vec(&serde_json::json!({
            "hook_event_name": "beforeShellExecution",
            "conversation_id": "sess-cursor-282",
            "cwd": dir.to_string_lossy(),
            "command": "ls -la",
        }))
        .unwrap();
        dispatch_owned_strict(&stdin, "PreToolUse", &cfg).unwrap();
        let store = crate::store::Store::open(&cfg.core.db_path).unwrap();
        // Same resolution `dispatch` itself used (`Runtime::with_store`'s `[hook] host` lookup,
        // falling back to `other`) — never a second copy of that fallback here.
        let host_id = Runtime::open(cfg.clone(), "irrelevant")
            .unwrap()
            .host_id()
            .unwrap();
        let id = store
            .register_agent(host_id, "sess-cursor-282", None, None, None)
            .unwrap();
        let row = store.agent_row(&id).unwrap().unwrap();
        assert_eq!(row.host_session_id, "sess-cursor-282");
        assert_eq!(row.activity.as_deref(), Some("Bash: ls -la"));
        let _ = std::fs::remove_dir_all(&dir);

        let dir = unique_dir("rtok-hook-t282-cline");
        let cfg = cline_cfg(&dir);
        let stdin = serde_json::to_vec(&serde_json::json!({
            "hookName": "tool_call",
            "taskId": "cline-sess-282",
            "workspaceRoots": ["/tmp"],
            "tool_call": {"id": "tc-1", "name": "run_commands", "input": {"commands": ["git status"]}}
        }))
        .unwrap();
        dispatch_owned_strict(&stdin, "PreToolUse", &cfg).unwrap();
        let store = crate::store::Store::open(&cfg.core.db_path).unwrap();
        let host_id = Runtime::open(cfg.clone(), "irrelevant")
            .unwrap()
            .host_id()
            .unwrap();
        let id = store
            .register_agent(host_id, "cline-sess-282", None, None, None)
            .unwrap();
        let row = store.agent_row(&id).unwrap().unwrap();
        assert_eq!(row.host_session_id, "cline-sess-282");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── T283 (D34): an agent learns its own rtok agent id ──────────────────────

    /// A bare `SessionStart` (`source: "startup"`) for `session_id`, plus a fresh in-memory
    /// `Runtime` for the same session — every T283 test below starts from this.
    fn session_start_fixture(session_id: &str) -> (Vec<u8>, HookInput, Runtime) {
        let raw = serde_json::json!({
            "hook_event_name": "SessionStart",
            "session_id": session_id,
            "cwd": "/repo",
            "source": "startup",
        });
        let stdin = serde_json::to_vec(&raw).unwrap();
        let input: HookInput = serde_json::from_slice(&stdin).unwrap();
        let cx = Runtime::in_memory(session_id.to_string()).unwrap();
        (stdin, input, cx)
    }

    fn additional_context(out: &[u8]) -> String {
        let v: serde_json::Value = serde_json::from_slice(out).unwrap();
        v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    /// The fixed wording `agent_id_injection` builds — kept here too so the test pins the
    /// spec's exact sentence, not just whatever the function happens to emit.
    fn expected_agent_line(id: &str) -> String {
        format!(
            "rtok agent id: {} (full: {id}). Use it with rtok's agent_* and worktree_* MCP tools; agent_inbox reads messages sent to you.",
            &id[..8]
        )
    }

    /// T293: an empty store still names the project on SessionStart. The fixture cwd
    /// (`/repo`) is not a checkout, so recall falls back to this process's project.
    fn expected_memory_line() -> String {
        let project = std::env::current_dir()
            .ok()
            .as_deref()
            .and_then(crate::project::project_name);
        format!(
            "memory project {}: no notes",
            project.as_deref().unwrap_or("none")
        )
    }

    fn expected_session_context(id: &str) -> String {
        format!("{}\n{}", expected_agent_line(id), expected_memory_line())
    }

    #[test]
    fn session_start_injects_the_fixed_wording_line_with_the_registered_id() {
        let (stdin, input, cx) = session_start_fixture("t283-sess-1");
        let out = dispatch(&stdin, &input, &cx);
        let id = cx
            .store
            .register_agent(cx.host_id().unwrap(), &cx.session, None, None, None)
            .unwrap();
        assert_eq!(additional_context(&out), expected_session_context(&id));

        // Same session again: `register_agent` upserts the same row, so the id — and the
        // whole line — stays byte-identical.
        let out2 = dispatch(&stdin, &input, &cx);
        assert_eq!(out, out2, "byte-stable across two runs of the same session");
    }

    /// T442: SessionStart names the claim in one line and records no measurement of its own.
    #[test]
    fn session_start_names_the_claimed_task() {
        let dir = std::env::temp_dir().join(format!("rtok-claim-hook-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(
            dir.join(".git/config"),
            "[remote \"origin\"]\n\turl = https://github.com/me/claim-hook.git\n",
        )
        .unwrap();
        let (stdin, input, mut cx) = session_start_fixture("t442-claim");
        cx.cwd = Some(dir.display().to_string());
        let host = cx.host_id().unwrap();
        let agent = cx
            .store
            .register_agent(host, &cx.session, None, cx.cwd.as_deref(), None)
            .unwrap();
        let key = crate::project::project_key(&dir).unwrap();
        cx.store
            .upsert_task_claim(&key, "R4", &agent, "Ship it\nmore")
            .unwrap();
        let out = dispatch(&stdin, &input, &cx);
        assert!(
            additional_context(&out).contains("task R4 Ship it"),
            "{}",
            additional_context(&out)
        );
        assert!(!additional_context(&out).contains("more"));
        assert!(cx.store.list_measurements("task_claim").unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T428: the recall and `inject` measurements reach the ledger in the order the plugins
    /// recorded them, though SessionStart now writes them in one commit, and the queue is
    /// closed afterwards so a later `record` is written at once.
    #[test]
    fn session_start_records_its_measurements_after_the_dispatch() {
        let (stdin, input, cx) = session_start_fixture("t418-sess");
        let _ = dispatch(&stdin, &input, &cx);
        assert_eq!(cx.store.list_measurements("memory").unwrap().len(), 1);
        assert_eq!(cx.store.list_measurements("inject").unwrap().len(), 1);
        let m = rtok_plugin_sdk::Measurement {
            plugin: "memory",
            kind: "after",
            before_bytes: 0,
            after_bytes: 0,
            est_before: 0,
            est_after: 0,
            ref_id: None,
            call_id: None,
        };
        cx.record(&m).unwrap();
        assert_eq!(cx.store.list_measurements("memory").unwrap().len(), 2);
    }

    #[test]
    fn deferred_measurements_wait_for_the_flush_and_keep_their_order() {
        let cx = Runtime::in_memory("t418-defer").unwrap();
        let row = |kind| rtok_plugin_sdk::Measurement {
            plugin: "memory",
            kind,
            before_bytes: 0,
            after_bytes: 0,
            est_before: 0,
            est_after: 0,
            ref_id: None,
            call_id: None,
        };
        cx.defer_measurements();
        cx.record(&row("first")).unwrap();
        cx.record(&row("second")).unwrap();
        assert!(cx.store.list_measurements("memory").unwrap().is_empty());
        cx.flush_measurements();
        let kinds: Vec<_> = cx
            .store
            .list_measurements("memory")
            .unwrap()
            .into_iter()
            .map(|m| m.kind)
            .collect();
        assert_eq!(kinds, ["first", "second"]);
    }

    #[test]
    fn session_start_line_is_fixed_wording_that_differs_only_by_the_id() {
        let (stdin_a, input_a, cx_a) = session_start_fixture("t283-sess-a");
        let (stdin_b, input_b, cx_b) = session_start_fixture("t283-sess-b");
        let ctx_a = additional_context(&dispatch(&stdin_a, &input_a, &cx_a));
        let ctx_b = additional_context(&dispatch(&stdin_b, &input_b, &cx_b));
        assert_ne!(ctx_a, ctx_b, "two sessions get two different ids");
        let id_a = cx_a
            .store
            .register_agent(cx_a.host_id().unwrap(), &cx_a.session, None, None, None)
            .unwrap();
        let id_b = cx_b
            .store
            .register_agent(cx_b.host_id().unwrap(), &cx_b.session, None, None, None)
            .unwrap();
        assert_eq!(ctx_a, expected_session_context(&id_a));
        assert_eq!(ctx_b, expected_session_context(&id_b));
    }

    #[test]
    fn session_start_has_no_line_when_agents_disabled() {
        let (stdin, input, mut cx) = session_start_fixture("t283-sess-off");
        cx.config.agents.enabled = false;
        let out = dispatch(&stdin, &input, &cx);
        let ctx = additional_context(&out);
        assert!(
            !ctx.contains("rtok agent id"),
            "agents off must not inject an id: {ctx}"
        );
        assert_eq!(ctx, expected_memory_line());
    }

    /// `SubagentStart` gets the sub-agent's *own* id, not its parent's (`agent_parent_key`
    /// is the resolving key `dispatch` already upserts against — no second lookup here).
    #[test]
    fn subagent_start_injects_the_sub_agent_s_own_id_not_the_parent_s() {
        let (v, stdin, input, cx) = agent_fixture("t283-parent-sess");
        let _ = dispatch(&stdin, &input, &cx); // registers the parent row

        let mut sub = v.clone();
        sub["hook_event_name"] = serde_json::Value::String("SubagentStart".into());
        sub["agent_id"] = serde_json::Value::String("sub-283".into());
        sub["agent_type"] = serde_json::Value::String("general-purpose".into());
        let sub_stdin = serde_json::to_vec(&sub).unwrap();
        let sub_input: HookInput = serde_json::from_slice(&sub_stdin).unwrap();
        let ctx = additional_context(&dispatch(&sub_stdin, &sub_input, &cx));

        let host_id = cx.host_id().unwrap();
        let parent_id = cx
            .store
            .register_agent(host_id, &cx.session, None, None, None)
            .unwrap();
        let sub_id = cx
            .store
            .register_agent(host_id, &cx.session, Some("sub-283"), None, None)
            .unwrap();
        assert_eq!(ctx, expected_agent_line(&sub_id));
        assert_ne!(ctx, expected_agent_line(&parent_id));
    }

    // ── T288: undelivered messages ride the next prompt / tool result ───────────

    const PROMPT: &str = include_str!("../../tests/fixtures/hooks/user_prompt_submit.json");
    const POST_TOOL: &str = include_str!("../../tests/fixtures/hooks/post_tool.json");

    /// `fixture`'s event for a fresh session, the runtime, and that session's agent id (the
    /// same row `dispatch` upserts).
    fn push_fixture(fixture: &str, session: &str) -> (Vec<u8>, HookInput, Runtime, String) {
        let mut v: serde_json::Value = serde_json::from_str(fixture).unwrap();
        v["session_id"] = session.into();
        let stdin = serde_json::to_vec(&v).unwrap();
        let input: HookInput = serde_json::from_slice(&stdin).unwrap();
        let cx = Runtime::in_memory(session.to_string()).unwrap();
        let id = cx
            .store
            .register_agent(cx.host_id().unwrap(), &cx.session, None, None, None)
            .unwrap();
        (stdin, input, cx, id)
    }

    fn frames(cx: &Runtime, to: &str) -> Vec<String> {
        let rows = cx.store.undelivered(to).unwrap();
        rows.iter()
            .map(crate::render::agent_message_frame)
            .collect()
    }

    #[test]
    fn one_message_is_pushed_framed_on_prompt_and_tool_result() {
        for fixture in [PROMPT, POST_TOOL] {
            let (stdin, input, cx, id) = push_fixture(fixture, "t288-one");
            cx.store.send_message(None, &id, "hello").unwrap();
            let frame = frames(&cx, &id).remove(0);
            let ctx = additional_context(&dispatch(&stdin, &input, &cx));
            assert!(ctx.starts_with(frame.trim_end()), "{ctx}");
            assert!(cx.store.undelivered(&id).unwrap().is_empty());
        }
    }

    #[test]
    fn an_over_budget_batch_pushes_what_fits_and_counts_the_rest_then_drains() {
        let (stdin, input, mut cx, id) = push_fixture(PROMPT, "t288-batch");
        for body in ["one", "two", "six"] {
            cx.store.send_message(None, &id, body).unwrap();
        }
        let f = frames(&cx, &id);
        let more = |n| format!("… and {n} more: call agent_inbox (or run rtok agents inbox)");
        cx.config.agents.push_bytes = (f[0].len() + more(3).len() + 1) as u32;
        let ctx = || additional_context(&dispatch(&stdin, &input, &cx));
        assert_eq!(ctx(), format!("{}{}", f[0], more(2)));
        assert_eq!(ctx(), format!("{}{}", f[1], more(1)));
        assert_eq!(ctx(), f[2].trim_end());
        assert_eq!(ctx(), "", "each message is pushed once");
    }

    #[test]
    fn a_message_over_push_bytes_is_announced_once_by_the_more_line() {
        let (stdin, input, mut cx, id) = push_fixture(PROMPT, "t288-big");
        cx.store.send_message(None, &id, "hello").unwrap();
        cx.config.agents.push_bytes = 64;
        let ctx = additional_context(&dispatch(&stdin, &input, &cx));
        assert_eq!(
            ctx,
            "… and 1 more: call agent_inbox (or run rtok agents inbox)"
        );
        assert_eq!(dispatch(&stdin, &input, &cx), b"{}");
        assert_eq!(cx.store.inbox(&id, true, false).unwrap().len(), 1);
    }

    #[test]
    fn an_empty_inbox_prints_nothing_and_records_no_measurement() {
        for fixture in [PROMPT, POST_TOOL] {
            let (stdin, input, cx, _) = push_fixture(fixture, "t288-empty");
            assert_eq!(dispatch(&stdin, &input, &cx), b"{}");
            assert_eq!(cx.store.measurement_count("inject").unwrap(), 0);
        }
    }

    #[test]
    fn a_pushed_message_is_delivered_once_and_stays_unread() {
        let (stdin, input, cx, id) = push_fixture(PROMPT, "t288-once");
        cx.store.send_message(None, &id, "hello").unwrap();
        assert!(!additional_context(&dispatch(&stdin, &input, &cx)).is_empty());
        let post: HookInput = serde_json::from_str(POST_TOOL).unwrap();
        let post = HookInput {
            session_id: cx.session.clone(),
            ..post
        };
        assert_eq!(dispatch(POST_TOOL.as_bytes(), &post, &cx), b"{}");
        assert_eq!(dispatch(&stdin, &input, &cx), b"{}");
        let unread = cx.store.inbox(&id, true, false).unwrap();
        assert_eq!(unread.len(), 1, "delivered, not read");
        assert!(unread[0].delivered_at.is_some() && unread[0].read_at.is_none());
    }

    #[test]
    fn no_push_when_agents_are_disabled() {
        for fixture in [PROMPT, POST_TOOL] {
            let (stdin, input, mut cx, id) = push_fixture(fixture, "t288-off");
            cx.store.send_message(None, &id, "hello").unwrap();
            cx.config.agents.enabled = false;
            assert_eq!(dispatch(&stdin, &input, &cx), b"{}");
            assert_eq!(cx.store.undelivered(&id).unwrap().len(), 1);
        }
    }
}
