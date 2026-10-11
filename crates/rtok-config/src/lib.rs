// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `~/.rtok/config.toml` — every setting rtok has (plan T0.2, T12.1, decision D12).
//!
//! `default.toml` is the reference file: it is embedded with `include_str!` and written
//! on a fresh install so its comments survive. Assignments in it are comments, so a key the
//! user never sets keeps following [`Config::default()`] when that default changes. A test
//! asserts the file still parses to exactly that, and that uncommenting the documented
//! assignments does too.
//!
//! Every section is `#[serde(default, deny_unknown_fields)]`: a partial file keeps the
//! defaults, and a typo is an error rather than a silently ignored key.
//! Layering (default < user file < project file < env < flags) is [`layers`] (T12.2).
//!
//! This crate prints nothing and writes no log. Load warnings sit on [`Config::notes`];
//! the surface prints and logs them.

#![forbid(unsafe_code)]

macro_rules! bail {
    ($($tt:tt)*) => {
        return ::core::result::Result::Err($crate::ConfigError::from(::anyhow::anyhow!($($tt)*)))
    };
}

mod error;
pub use error::{ConfigError, Result};

mod estimator;
pub use estimator::Estimator;

mod since;
pub use since::{parse_since, parse_since_from};

pub mod layers;
pub mod validate;

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

// T178: one resolver for rtok and the std-only `rtok-hook` client, so both find the same home.
use rtok_hook::{expand_with, home_dir_from, user_home_from};
use serde::{Deserialize, Serialize};

/// Reference file `rtok config init` writes. Assignments are comments so the file
/// documents every key without pinning the default from the day it was written.
pub const DEFAULT_TOML: &str = include_str!("../default.toml");

/// Write a config file with the installers' atomic swap: temp file, then rename. With a plain
/// `fs::write`, a kill or a full disk halfway through left `config.toml` truncated, and every
/// later rtok command, hooks included, failed to parse it.
pub fn write_file(path: &Path, body: &str) -> Result<()> {
    rtok_agent_sdk::write(&rtok_agent_sdk::Apply::default(), path, body, "config")
        .map_err(Into::into)
}

/// Host ids `rtok agents` installs. Config validation checks `[agents.junk] extra`
/// against this list; `agents` re-exports it.
pub const HOSTS: &[&str] = &[
    "claude",
    "cursor",
    "codex",
    "opencode",
    "kilo",
    "pi",
    "omp",
    "zcode",
    "kimi",
    "grok",
    "vscode",
    "copilot",
    "commandcode",
    "aider",
    "windsurf",
    "zed",
    "cline",
    "gemini",
    "codewhale",
    "mimo",
    "antigravity",
    "devin",
    "roo",
    "qwen",
    "codebuddy",
];

/// Adapters `[tasks] adapter` accepts. `tasks::run::ADAPTERS` re-exports this.
pub const TASK_ADAPTERS: [&str; 3] = ["disk", "github", "gitlab"];

/// Plugin catalogue: `(id, default_on)`. The registry's manifests must match this list
/// (asserted by a test in `plugins`), and [`Plugins`] has one field per id.
pub const CATALOGUE: [(&str, bool); 13] = [
    ("measure", true),
    ("cmd", true),
    ("read", true),
    ("json_tree", false),
    ("archive", true),
    ("proxy", true),
    ("inject", true),
    ("guard", true),
    ("memory", true),
    ("graph", true),
    ("toon", true),
    ("compress", true),
    ("docs", false),
];

/// Shorthand for the section attributes every table repeats.
macro_rules! section {
    ($(#[$m:meta])* $name:ident { $($(#[$fm:meta])* $field:ident : $ty:ty = $default:expr),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        pub struct $name {
            $($(#[$fm])* pub $field: $ty,)*
        }

        impl Default for $name {
            fn default() -> Self {
                Self { $($field: $default,)* }
            }
        }
    };
}

fn s(v: &str) -> String {
    v.to_string()
}

fn p(v: &str) -> PathBuf {
    PathBuf::from(v)
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(ToString::to_string).collect()
}

// ── core ────────────────────────────────────────────────────────────────────

section! {
    /// `[core]` — paths shared by every surface. Logging lives in `[log]` (D26).
    Core {
        /// Kill-switch for business logic. When false the process stays up; the HTTP
        /// proxy (and any other long-running surface that would apply plugins/bookkeeping)
        /// runs as a plain forwarder until the process exits. Disabling never stops the listener.
        enabled: bool = true,
        db_path: PathBuf = p("~/.rtok/rtok.db"),
        archive_dir: PathBuf = p("~/.rtok/archive"),
        /// Removed in T24.5: it is now `log.level`. Accepted from an old file with a
        /// warning, then dropped.
        #[serde(skip_serializing_if = "Option::is_none")]
        log_level: Option<String> = None,
        /// Removed in T24.5: it is now `log.path`. Accepted from an old file with a
        /// warning, then dropped.
        #[serde(skip_serializing_if = "Option::is_none")]
        log_file: Option<PathBuf> = None,
        session_env: String = s("CLAUDE_SESSION_ID"),
        call_io_inline_bytes: u32 = 65536,
        /// T201: `rtok hook <event>`'s stdin is bounded here before it is even parsed — a
        /// body over this many bytes fails the hook open to `{}` (unmodified, one stderr
        /// line) instead of paying a JSON-parse-plus-hashing cost that grows with the
        /// payload (D1: ≤ 10 ms). 8 MiB default.
        hook_max_input_bytes: u32 = rtok_hook::HOOK_MAX_INPUT_BYTES,
        retain_calls_days: u32 = 30,
        /// T352: hook stdin bodies in `call_io` are cleared after this many days; the call row,
        /// sizes and shas stay. 0 keeps bodies as long as `calls`.
        retain_hook_bodies_days: u32 = 3,
        /// T431: request bodies are saved without terminal escapes, control and zero-width
        /// characters, harness wrapper blocks and trailing whitespace; `true` saves them verbatim.
        store_raw: bool = false,
        /// Removed in T24.5: it is now `log.to_db`. Accepted from an old file with a
        /// warning, then dropped.
        #[serde(skip_serializing_if = "Option::is_none")]
        log_to_db: Option<bool> = None,
        /// Removed in T12.1: it is now `plugins.inject.budget_tokens`. Accepted from an old
        /// file with a warning, then dropped.
        #[serde(skip_serializing_if = "Option::is_none")]
        inject_budget_tokens: Option<u32> = None,
    }
}

// ── surfaces ────────────────────────────────────────────────────────────────

section! {
    /// `[log]` — rtok's own log (P24, D26). One rotating text file, plus the `logs` rows
    /// `rtok otel` exports. Legacy `[core] log_file` / `log_level` / `log_to_db` migrate here.
    Log {
        path: PathBuf = p("~/.rtok/logs/rtok.log"),
        max_bytes: u64 = 1_048_576,
        files: u32 = 5,
        lines: usize = 200,
        level: String = s("info"),
        to_db: bool = true,
        tspin: String = s("auto"),
    }
}

section! {
    /// `[hook]` — `rtok hook <event>`.
    Hook {
        host: String = s("claude"),
        max_ms: u64 = 10,
        fail_open: bool = true,
    }
}

section! {
    /// `[agents]` — the rtok agent registry (T282, D34): one row per host session, resolved
    /// by any unique id prefix of 4+ hex chars. `idle` bounds `live()` (`store::live_agents`,
    /// parsed by `humantime::parse_duration`); `enabled` gates registration only — the
    /// hook itself and `[core] enabled` are unaffected (and, off, no message is pushed).
    Agents {
        enabled: bool = true,
        idle: String = s("30m"),
        /// T288: bytes of framed messages one `UserPromptSubmit`/`PostToolUse` pushes into
        /// the agent's context; the rest become one "and N more" line.
        push_bytes: u32 = 1024,
        usage: AgentsUsage = AgentsUsage::default(),
        junk: AgentsJunk = AgentsJunk::default(),
    }
}

section! {
    /// `[agents.junk]` — `rtok agents junk list|clear` (T330.5.1): the age floors of the
    /// junk kinds, paths never touched, and the paths the user vouches for as junk (the third
    /// kind of D36 evidence beside a §22 row and a `CACHEDIR.TAG`). `~` in a path or glob is the
    /// user's home.
    AgentsJunk {
        /// `logs` entries modified within this many days stay.
        keep_logs_days: u32 = 30,
        /// `temp` entries touched within this many hours stay.
        temp_min_age_hours: u32 = 24,
        /// `sessions` (only with `--kind sessions`) older than this many days are junk; time
        /// is the only criterion (D36, T330 "Old sessions: time only"). `--session-days` is
        /// the one-run override.
        stale_session_days: u32 = 30,
        /// A worktree `rtok worktree gc` would remove is the `stale-worktrees` kind once it
        /// has been idle this many days (gc's `--idle`, T330.5.3).
        stale_worktree_days: u32 = 14,
        /// A crash dump in an `extra` crash folder older than this many days is `safe`; a
        /// younger one is `review`.
        crash_dump_min_age_days: u32 = 7,
        /// Globs of paths never touched, nor any folder that holds one.
        exclude: Vec<String> = Vec::new(),
        extra: Vec<JunkExtra> = Vec::new(),
    }
}

section! {
    /// One `[agents.junk] extra` entry: `path` is junk of `kind` for `host` (a host id or `rtok`).
    JunkExtra {
        host: String = String::new(),
        kind: String = String::new(),
        path: String = String::new(),
    }
}

/// The kinds an `[agents.junk] extra` entry may name: folders whose content ages out.
pub const JUNK_EXTRA_KINDS: [&str; 4] = ["cache", "temp", "logs", "crash-dumps"];

section! {
    /// `[agents.usage]` — `rtok agents usage` (T358): tokens and estimated cost per agent, day
    /// and month. `source` is `logs` (the agents' own session files, read from
    /// `[stats] transcripts_dir`, `codex_dir` and `[agents.usage.dirs]`), `rtok` (the store) or `both`. `hosts` empty = every
    /// host; `since` / `until` are a date (`2026-09-01`, a whole day in `tz`) or, for `since`, a
    /// duration (`30d`), empty = unbounded; `period` is `monthly` or `daily`; `by` groups
    /// the middle table by `agent` or `model`; `tz` is an IANA zone, empty = the system zone.
    AgentsUsage {
        source: String = s("logs"),
        hosts: Vec<String> = Vec::new(),
        since: String = String::new(),
        until: String = String::new(),
        period: String = s("monthly"),
        by: String = s("agent"),
        tz: String = String::new(),
        dirs: UsageDirs = UsageDirs::default(),
    }
}

section! {
    /// `[agents.usage.dirs]` — where `rtok agents usage` reads each host's own records (T358.3):
    /// a list per host, every entry a directory. Claude Code and Codex keep reading `[stats]
    /// transcripts_dir` and `codex_dir`. A default the file leaves untouched yields to the host's
    /// own relocation variable (`XDG_DATA_HOME`, `COPILOT_HOME`, `GEMINI_CLI_HOME`,
    /// `PI_CODING_AGENT_SESSION_DIR`, `PI_CODING_AGENT_DIR`, `KIMI_CODE_HOME`, `GROK_HOME`).
    UsageDirs {
        opencode: Vec<PathBuf> = vec![p("~/.local/share/opencode")],
        kilo: Vec<PathBuf> = vec![p("~/.local/share/kilo")],
        copilot: Vec<PathBuf> = vec![p("~/.copilot/session-state")],
        gemini: Vec<PathBuf> = vec![p("~/.gemini/tmp")],
        droid: Vec<PathBuf> = vec![p("~/.factory/sessions")],
        pi: Vec<PathBuf> = vec![p("~/.pi/agent/sessions")],
        kimi: Vec<PathBuf> = vec![p("~/.kimi-code/sessions")],
        grok: Vec<PathBuf> = vec![p("~/.grok/sessions")],
        zcode: Vec<PathBuf> = vec![p("~/.zcode")],
        antigravity: Vec<PathBuf> = vec![p("~/.gemini/antigravity")],
    }
}

impl UsageDirs {
    /// Point each default the config file left untouched at the host's own relocation variable,
    /// so a person who moved `~/.copilot` is read there without repeating it here. A relative
    /// value is ignored, as the XDG spec says, and a key the file set always wins.
    fn follow_env(&mut self, get: impl Fn(&str) -> Option<std::ffi::OsString>) {
        let base = UsageDirs::default();
        let abs = |key: &str| get(key).map(PathBuf::from).filter(|v| v.is_absolute());
        let moved = |list: &mut Vec<PathBuf>, was: &[PathBuf], to: Option<PathBuf>| {
            if let (true, Some(to)) = (list == was, to) {
                *list = vec![to];
            }
        };
        let xdg = abs("XDG_DATA_HOME");
        moved(
            &mut self.opencode,
            &base.opencode,
            xdg.as_ref().map(|d| d.join("opencode")),
        );
        moved(&mut self.kilo, &base.kilo, xdg.map(|d| d.join("kilo")));
        moved(
            &mut self.copilot,
            &base.copilot,
            abs("COPILOT_HOME").map(|d| d.join("session-state")),
        );
        moved(
            &mut self.gemini,
            &base.gemini,
            abs("GEMINI_CLI_HOME").map(|d| d.join(".gemini").join("tmp")),
        );
        // pi: the sessions variable names the folder itself and outranks the agent-dir one.
        moved(
            &mut self.pi,
            &base.pi,
            abs("PI_CODING_AGENT_SESSION_DIR")
                .or_else(|| abs("PI_CODING_AGENT_DIR").map(|d| d.join("sessions"))),
        );
        moved(
            &mut self.kimi,
            &base.kimi,
            abs("KIMI_CODE_HOME").map(|d| d.join("sessions")),
        );
        moved(
            &mut self.grok,
            &base.grok,
            abs("GROK_HOME").map(|d| d.join("sessions")),
        );
    }
}

section! {
    /// `[mcp]` — `rtok mcp`.
    Mcp {
        tools: Vec<String> = Vec::new(),
        max_description_tokens: u32 = 60,
        /// Above this, MCP tool results use head/tail + archive id.
        max_result_chars: u32 = 20000,
        /// `rtok mcp --http` without an address binds here. Loopback, so only a tunnel the
        /// user starts puts the server on the internet.
        http: String = s("127.0.0.1:8791"),
        /// The HTTP server's allow-list, used instead of `tools`: what an internet caller may
        /// run is chosen apart from what a local host may, and starts read-only.
        http_tools: Vec<String> = strs(&["read", "search", "tree"]),
        /// Bearer token every HTTP request must carry. Empty: the HTTP server refuses to start.
        token: String = String::new(),
        /// The public HTTPS URL a tunnel serves the HTTP server at. Its host passes the Host
        /// check and its origin the Origin check; anything else is refused.
        public_url: String = String::new(),
    }
}

section! {
    /// `[proxy.tools_rewrite]` — opt-in `tools[]` description rewrite (T59.5). Off: bytes
    /// identical. Empty `allow` keeps every tool not in `deny`. `input_schema` is never touched.
    ToolsRewrite {
        enabled: bool = false,
        max_description_tokens: u32 = 60,
        allow: Vec<String> = Vec::new(),
        deny: Vec<String> = Vec::new(),
    }
}

section! {
    /// `[proxy.lanes]` — request lanes (T385.1): each request is tagged agent, bulk, batch,
    /// files, embeddings, meta or internal in the ledger (`calls.kind`). Off: every request is
    /// an untagged `api_request` and the `x-rtok-lane` header and `/lane/<name>/` prefix are
    /// forwarded as the client sent them. The `agent` lane has no table: the global switches
    /// decide for it exactly as before lanes; `batch` and `files` have none either, they are
    /// always passed through.
    Lanes {
        enabled: bool = true,
        bulk: LanePolicy = LanePolicy::default(),
        embeddings: LanePolicy = LanePolicy::default(),
        meta: LanePolicy = LanePolicy::default(),
        internal: LanePolicy = LanePolicy::default(),
    }
}

section! {
    /// `[proxy.lanes.<lane>]` — what the proxy may change on one non-agent lane (T385.2).
    /// Every switch narrows its global counterpart: a rewrite runs only when both the global
    /// switch and the lane switch are on. All off by default, so a lane's bytes are forwarded
    /// as the client sent them until the operator opts that lane in.
    LanePolicy {
        /// `proxy.mode = "compress"` rewrites (archive, compress, terminal-noise strip).
        compress: bool = false,
        /// The `toon` filter inside that pass; needs `compress`.
        toon: bool = false,
        /// `proxy.tools_rewrite`.
        tools_rewrite: bool = false,
        /// `proxy.context_management`.
        context_management: bool = false,
        /// `plugins.proxy.semantic_cache`, lookup and store.
        semantic_cache: bool = false,
        /// OpenAI `service_tier = "flex"` on this lane's chat and responses calls; see
        /// `[proxy.flex]`. Never opened on the `agent` lane.
        flex: bool = false,
        /// Read timeout for this lane in seconds; 0 = `proxy.timeout_s`.
        timeout_s: u64 = 0,
        /// Base URL for every request on this lane, whatever its wire (T385.7); empty = the
        /// wire's own `proxy.upstream` / `openai_upstream` / `gemini_upstream`.
        upstream: String = String::new(),
        /// Requests on this lane upstream at once (T385.7); 0 = no cap. Other lanes, the
        /// `agent` lane above all, never wait for this one.
        max_in_flight: u32 = 0,
        /// With `max_in_flight` set: requests that may wait for a slot. One more is answered
        /// `429` with `Retry-After` and never reaches upstream.
        max_queued: u32 = 8,
    }
}

section! {
    /// `[proxy.batch]` — provider Batch observe (T385.4). The Batch calls themselves are
    /// tagged by `[proxy.lanes]`; this table only decides whether result files are read.
    BatchPolicy {
        /// Parse a fetched Batch results file (Anthropic `/results`, OpenAI file content) into
        /// one `usage` row per result line. Observation only: the body is forwarded untouched.
        parse_results: bool = false,
    }
}

section! {
    /// `[proxy.flex]` — how the Flex tier is set and what happens when it has no capacity
    /// (T385.5). Whether a lane gets Flex at all is `[proxy.lanes.<lane>] flex`.
    FlexPolicy {
        /// Overwrite a `service_tier` the client sent. Off: a client value is never changed.
        force: bool = false,
        /// On `429` from a request rtok set to Flex: `none` hands the 429 to the client,
        /// `backoff` retries on Flex with doubling delays, `default` retries once on `auto`.
        on_429: String = s("none"),
        /// `backoff` only: retries before the 429 goes to the client; at most 5.
        retries: u32 = 3,
        /// `backoff` only: delay before the first retry, doubled each time, capped at 30 s.
        backoff_ms: u64 = 1000,
    }
}

section! {
    /// `[proxy.routing]` — model routing (D9, I-101), opt-in. Only rtok's own `internal` calls
    /// and the `bulk` lane can be routed; the `agent` lane never is, and `batch` and `files`
    /// are never rewritten. A rule moves a call to another model only when `enabled`, the
    /// request's model is one of the rule's `from` entries and the rule names the quality
    /// bench behind the swap in `gate`. No rule ships by default.
    RoutingPolicy {
        enabled: bool = false,
        bulk: RouteRule = RouteRule::default(),
        internal: RouteRule = RouteRule::default(),
    }
}

section! {
    /// `[proxy.routing.<lane>]` — one lane's routing rule (T385.10).
    RouteRule {
        /// Request models this rule applies to: an exact id, or an id ending in `*` for a
        /// prefix (`gpt-5*`). Empty = the rule is off, so no lane is ever routed wholesale.
        from: Vec<String> = Vec::new(),
        /// The model that serves a matching call (a cheaper one). Empty = the rule is off.
        to: String = String::new(),
        /// The provider slug recorded for a routed call, when `to` lives at another provider
        /// than the wire's own (pair it with `[proxy.lanes.<lane>] upstream`); empty = the
        /// wire's provider.
        provider: String = String::new(),
        /// Where the quality bench for this `from` → `to` swap is kept (a path, URL or run
        /// id). rtok cannot judge answer quality, so the rule stays off while this is empty.
        gate: String = String::new(),
    }
}

section! {
    /// `[proxy]` — the proxy server itself; the usage-capture plugin is `[plugins.proxy]`.
    Proxy {
        /// When false the HTTP listener stays up but every request is byte-forwarded with
        /// no compress / bookkeeping / request shaping. Only killing the process stops HTTP.
        enabled: bool = true,
        bind: String = s("127.0.0.1"),
        port: u16 = 8790,
        mode: String = s("passthrough"),
        upstream: String = s("https://api.anthropic.com"),
        openai_upstream: String = s("https://api.openai.com"),
        gemini_upstream: String = s("https://generativelanguage.googleapis.com"),
        timeout_s: u64 = 600,
        include_usage: bool = true,
        /// Opt-in Anthropic server-side context editing (T51.2): add
        /// `context_management.edits: [{type: "clear_tool_uses_20250919"}]` plus the
        /// `context-management-2025-06-27` beta header on `/v1/messages` requests that
        /// do not already carry the field. Other wires are unaffected.
        context_management: bool = false,
        dry_run: bool = false,
        tools_rewrite: ToolsRewrite = ToolsRewrite::default(),
        lanes: Lanes = Lanes::default(),
        batch: BatchPolicy = BatchPolicy::default(),
        flex: FlexPolicy = FlexPolicy::default(),
        routing: RoutingPolicy = RoutingPolicy::default(),
    }
}

section! {
    /// `[web]` — `rtok web` (P19). React SPA + WebSocket API, the same data as `rtok tui`.
    Web {
        host: String = s("127.0.0.1"),
        port: u16 = 3333,
    }
}

section! {
    /// `[tui]` — `rtok tui` (P15). The terminal rendering of the one operator model (D23).
    /// `tab` names the opening tab (`""` = first page); `tick_secs` is the model
    /// re-read cadence (2 = the web socket's tick in `src/web/mod.rs`, so both
    /// surfaces go stale at the same rate).
    Tui {
        tab: String = String::new(),
        tick_secs: u64 = 2,
    }
}

section! {
    /// `[ui]` — the look of rtok's own lines for a person at a terminal: status, success,
    /// warning and error lines, summaries, `--help`. `emoji` prefixes them with one emoji,
    /// `color` paints them. Neither ever reaches a pipe, a hook, `--json`, or the output an
    /// agent reads: emoji need a terminal and colour follows `NO_COLOR` / `CLICOLOR_FORCE` /
    /// `TERM=dumb` too (`ui::style`).
    Ui {
        emoji: bool = true,
        color: bool = true,
    }
}

section! {
    /// `[demon]` — `rtok demon` (P20, D22). Supervises the long-running surfaces.
    Demon {
        services: Vec<String> = strs(&["proxy"]),
        state_dir: PathBuf = p("~/.rtok/demon"),
        backoff_ms: u64 = 1000,
        max_backoff_ms: u64 = 30000,
        healthy_ms: u64 = 10000,
        poll_ms: u64 = 200,
    }
}

section! {
    /// `[stats]` — `rtok stats`.
    Stats {
        since: String = s("30d"),
        format: String = s("table"),
        plugin: String = String::new(),
        transcripts_dir: PathBuf = p("~/.claude/projects"),
        /// Codex CLI session logs, read as one more `api` row (T49.2).
        codex_dir: PathBuf = p("~/.codex/sessions"),
        calibrate_samples: u32 = 30,
        baseline: String = String::new(),
        /// Show per-model USD costs from `prices` (`rtok stats --price`, T49.1).
        price: bool = false,
        /// USD per MTok per model id (`rtok stats --price`, T49.1). A `usage` row
        /// whose model has no entry here is listed with `-`, never priced by guess.
        prices: BTreeMap<String, ModelPrice> = default_stats_prices(),
    }
}

section! {
    /// One `[stats.prices."<model>"]` row — USD per MTok (T49.1). `cache_write` is
    /// the 5-minute cache-creation price; providers without a separate write price
    /// repeat `input`.
    ModelPrice {
        input: f64 = 0.0,
        cache_write: f64 = 0.0,
        cache_read: f64 = 0.0,
        output: f64 = 0.0,
    }
}

/// The shipped `[stats.prices]` rows (T49.1, T389). Sources: Anthropic
/// `claude-sonnet-5` / `claude-haiku-4-5` fetched 2026-09-17 and `claude-fable-5-1` /
/// `claude-opus-5-5` fetched 2026-10-06 and `claude-sonnet-5-5` re-checked 2026-10-08 (its
/// cache read is 0.05x input, not 0.1x), all from
/// https://platform.claude.com/docs/en/about-claude/pricing (input / 5m write /
/// read / output per MTok); OpenAI `gpt-5` / `gpt-5-mini` from
/// https://platform.openai.com/docs/pricing (short-context input / cached input /
/// output; no separate write price, so `cache_write = input`).
fn default_stats_prices() -> BTreeMap<String, ModelPrice> {
    [
        (
            "claude-sonnet-5",
            ModelPrice {
                input: 2.0,
                cache_write: 2.5,
                cache_read: 0.2,
                output: 10.0,
            },
        ),
        (
            "claude-fable-5-1",
            ModelPrice {
                input: 10.0,
                cache_write: 12.5,
                cache_read: 0.25,
                output: 50.0,
            },
        ),
        (
            "claude-opus-5-5",
            ModelPrice {
                input: 4.0,
                cache_write: 5.0,
                cache_read: 0.2,
                output: 20.0,
            },
        ),
        (
            "claude-sonnet-5-5",
            ModelPrice {
                input: 2.0,
                cache_write: 2.5,
                cache_read: 0.1,
                output: 10.0,
            },
        ),
        (
            "claude-haiku-4-5",
            ModelPrice {
                input: 1.0,
                cache_write: 1.25,
                cache_read: 0.1,
                output: 5.0,
            },
        ),
        (
            "gpt-5",
            ModelPrice {
                input: 1.25,
                cache_write: 1.25,
                cache_read: 0.125,
                output: 10.0,
            },
        ),
        (
            "gpt-5-mini",
            ModelPrice {
                input: 0.25,
                cache_write: 0.25,
                cache_read: 0.025,
                output: 2.0,
            },
        ),
    ]
    .into_iter()
    .map(|(k, v)| (s(k), v))
    .chain(tier_prices())
    .collect()
}

/// The shipped Batch and Flex rows (T385.12.1), keyed `<model>@batch` / `<model>@flex`; the
/// suffix is what `rtok stats --price` appends to the model of usage that ran on that tier.
/// Fetched 2026-10-08. Anthropic: https://platform.claude.com/docs/en/about-claude/pricing
/// (Batch is 50 % off input and output, and the cache multipliers stack on top, so the write
/// and read columns are half the standard ones; Anthropic has no Flex tier). OpenAI:
/// https://developers.openai.com/api/docs/pricing (Batch and Flex list the same rates; cached
/// input, no separate write price).
fn tier_prices() -> impl Iterator<Item = (String, ModelPrice)> {
    let row = |input, cache_write, cache_read, output| ModelPrice {
        input,
        cache_write,
        cache_read,
        output,
    };
    let anthropic = [
        ("claude-sonnet-5@batch", row(1.0, 1.25, 0.1, 5.0)),
        ("claude-sonnet-5-5@batch", row(1.0, 1.25, 0.05, 5.0)),
        ("claude-opus-5-5@batch", row(2.0, 2.5, 0.1, 10.0)),
        ("claude-fable-5-1@batch", row(5.0, 6.25, 0.125, 25.0)),
        ("claude-haiku-4-5@batch", row(0.5, 0.625, 0.05, 2.5)),
    ];
    let openai = [
        ("gpt-5@batch", row(0.625, 0.625, 0.0625, 5.0)),
        ("gpt-5@flex", row(0.625, 0.625, 0.0625, 5.0)),
        ("gpt-5-mini@batch", row(0.125, 0.125, 0.0125, 1.0)),
        ("gpt-5-mini@flex", row(0.125, 0.125, 0.0125, 1.0)),
    ];
    anthropic.into_iter().chain(openai).map(|(k, v)| (s(k), v))
}

section! {
    /// `[report]` — `rtok report` (P22, D24). `format` grows `html` (T22.2) and `pdf`
    /// (T22.3); `ai` and `budget_tokens` landed with T22.4, `charts` with its task.
    Report {
        format: String = s("md"),
        out: PathBuf = PathBuf::new(),
        since: String = s("30d"),
        ai: bool = false,
        budget_tokens: u32 = 8000,
    }
}

section! {
    /// `[bench]` — `rtok bench`.
    Bench {
        tasks: PathBuf = p("bench/tasks.toml"),
        runs: u32 = 3,
        dry_run: bool = false,
        timeout_s: u64 = 900,
        /// `""` = T9.1 six tasks; `"graph"` = T68.9 with/without MCP.
        suite: String = String::new(),
        /// `[bench.configs]` — free-form `name = settings file`, so not a fixed struct.
        configs: BTreeMap<String, PathBuf> = [
            (s("a"), p("bench/configs/legacy.json")),
            (s("b"), p("bench/configs/rtok.json")),
        ].into_iter().collect(),
    }
}

section! {
    /// `[doctor]` — `rtok doctor`.
    Doctor {
        settings_path: PathBuf = p("~/.claude/settings.json"),
        claude_json: PathBuf = p("~/.claude.json"),
        mcp_json: PathBuf = p(".mcp.json"),
        probe_timeout_ms: u64 = 500,
        mcp_timeout_ms: u64 = 15000,
        instruction_warn_tokens: u32 = 1000,
        instructions: bool = false,
        /// Threshold (in MCP description tokens) for suggesting `[proxy.tools_rewrite]` when it applies (T59.5).
        tools_rewrite_min_desc_tokens: u32 = 2000,
    }
}

section! {
    /// `[setup]` — `rtok agents install <host>`.
    Setup {
        dry_run: bool = false,
        yes: bool = false,
        backup: bool = true,
        backup_files: u32 = 5,
        hook_timeout_s: u64 = 5,
        modes: Vec<String> = Vec::new(),
        mcp: bool = true,
        proxy: bool = false,
        /// `agents update --force` (T279 PR 3): reinstall the host plugin even when the
        /// version decision would otherwise skip it.
        force: bool = false,
        /// `agents update --source github|local|marketplace` (T279 PR 3): override the
        /// install source the version decision compares against, instead of the receipt's
        /// recorded one. Validated by `agents::plugin_version::Source`'s `FromStr`, not here,
        /// so this section stays free of that module's types.
        source: Option<String> = None,
        claude: SetupClaude = SetupClaude::default(),
        cursor: SetupCursor = SetupCursor::default(),
        codex: SetupCodex = SetupCodex::default(),
        opencode: SetupOpenCode = SetupOpenCode::default(),
        kilo: SetupKilo = SetupKilo::default(),
        pi: SetupPi = SetupPi::default(),
        omp: SetupOmp = SetupOmp::default(),
        zcode: SetupZcode = SetupZcode::default(),
        kimi: SetupKimi = SetupKimi::default(),
        grok: SetupGrok = SetupGrok::default(),
        vscode: SetupVscode = SetupVscode::default(),
        copilot: SetupCopilot = SetupCopilot::default(),
        commandcode: SetupCommandCode = SetupCommandCode::default(),
        aider: SetupAider = SetupAider::default(),
        windsurf: SetupWindsurf = SetupWindsurf::default(),
        zed: SetupZed = SetupZed::default(),
        cline: SetupCline = SetupCline::default(),
        gemini: SetupGemini = SetupGemini::default(),
        codewhale: SetupCodewhale = SetupCodewhale::default(),
        mimo: SetupMimo = SetupMimo::default(),
        antigravity: SetupAntigravity = SetupAntigravity::default(),
        devin: SetupDevin = SetupDevin::default(),
        roo: SetupRoo = SetupRoo::default(),
        qwen: SetupQwen = SetupQwen::default(),
        codebuddy: SetupCodebuddy = SetupCodebuddy::default(),
    }
}

section! {
    /// `[setup.claude]`
    SetupClaude { settings_path: PathBuf = p("~/.claude/settings.json") }
}

section! {
    /// `[setup.cursor]`
    SetupCursor { hooks_path: PathBuf = p("~/.cursor/hooks.json") }
}

section! {
    /// `[setup.codex]`
    SetupCodex { config_path: PathBuf = p("~/.codex/config.toml") }
}

section! {
    /// `[setup.opencode]`
    SetupOpenCode { config_path: PathBuf = p("~/.config/opencode/opencode.json") }
}

section! {
    /// `[setup.kilo]` — `kilo.json`, merged by Kilo with a user's `kilo.jsonc` (T97).
    SetupKilo { config_path: PathBuf = p("~/.config/kilo/kilo.json") }
}

section! {
    /// `[setup.cline]` — hooks dir serves CLI + extension (D21 singleton, T96);
    /// MCP is per surface (CLI path below + VS Code extension globalStorage).
    SetupCline {
        hooks_path: PathBuf = p("~/Documents/Cline/Hooks"),
        mcp_path: PathBuf = p("~/.cline/data/settings/cline_mcp_settings.json"),
    }
}

section! {
    /// `[setup.pi]`
    SetupPi {
        extensions_path: PathBuf = p("~/.pi/agent/extensions"),
        /// Register the measured MCP tools through `pi.registerTool` (T70.3).
        /// Off: those descriptions do not ride every pi request.
        tools: bool = false,
    }
}

section! {
    /// `[setup.omp]` — oh my pi: the linked pi extension and native `mcp.json` (T92). A named
    /// omp profile is a path override.
    SetupOmp {
        extensions_path: PathBuf = p("~/.omp/agent/extensions"),
        mcp_path: PathBuf = p("~/.omp/agent/mcp.json"),
    }
}

section! {
    /// `[setup.zcode]`
    SetupZcode { config_path: PathBuf = p("~/.zcode/cli/config.json") }
}

section! {
    /// `[setup.kimi]` — `mcp.json` is read beside `config_path`.
    SetupKimi { config_path: PathBuf = p("~/.kimi-code/config.toml") }
}

section! {
    /// `[setup.grok]` — Grok Build's `config.toml`; `plugins/` sits beside it (`GROK_HOME`
    /// moves the whole home, so point `config_path` there too).
    SetupGrok { config_path: PathBuf = p("~/.grok/config.toml") }
}

section! {
    /// `[setup.copilot]` — `mcp-config.json` and `hooks/rtok.json` live under `dir`.
    SetupCopilot { dir: PathBuf = p("~/.copilot") }
}

section! {
    /// `[setup.commandcode]` — hooks merge into `settings.json`, MCP into `mcp.json`.
    SetupCommandCode { dir: PathBuf = p("~/.commandcode") }
}

section! {
    /// `[setup.vscode]` — profile `mcp.json` under Code / Code - Insiders user dirs (T48.8).
    SetupVscode {
        code_user_dir: PathBuf = PathBuf::new(),
        insiders_user_dir: PathBuf = PathBuf::new(),
    }
}

section! {
    /// `[setup.aider]` — `.aider.conf.yml` carries `openai-api-base` (T48.7).
    SetupAider { config_path: PathBuf = p("~/.aider.conf.yml") }
}

section! {
    /// `[setup.windsurf]` — Cascade's `mcp_config.json` (T48.5).
    SetupWindsurf { config_path: PathBuf = p("~/.codeium/windsurf/mcp_config.json") }
}

section! {
    /// `[setup.zed]` — Zed's `settings.json` carries `context_servers` (T48.6).
    SetupZed { config_path: PathBuf = p("~/.config/zed/settings.json") }
}

section! {
    /// `[setup.gemini]` — Gemini CLI's `settings.json` (`hooks`, `mcpServers`) lives under
    /// `dir` (T118.2).
    SetupGemini { dir: PathBuf = p("~/.gemini") }
}

section! {
    /// `[setup.codewhale]` — CodeWhale's `config.toml` (`[[hooks.hooks]]`) and sibling
    /// `mcp.json` live under `dir` (T185, `$CODEWHALE_HOME`).
    SetupCodewhale { dir: PathBuf = p("~/.codewhale") }
}

section! {
    /// `[setup.mimo]` — MiMo Code's `mimocode.json` (`mcp`), the OpenCode-fork config file
    /// (T186, `MIMOCODE_HOME`/`MIMOCODE_CONFIG` move it).
    SetupMimo { config_path: PathBuf = p("~/.config/mimocode/mimocode.json") }
}

section! {
    /// `[setup.devin]` — Devin CLI and Desktop. Hooks live in `config.json`; MCP in the
    /// sibling `mcp_config.json`. On Windows the installer redirects the shipped default
    /// to `%APPDATA%\devin\config.json` (T89).
    SetupDevin { config_path: PathBuf = p("~/.config/devin/config.json") }
}

section! {
    /// `[setup.roo]` — Roo Code's global `mcp_settings.json` (T413.1). Empty
    /// `mcp_path` means the VS Code user dir plus
    /// `globalStorage/rooveterinaryinc.roo-cline/settings/mcp_settings.json`.
    SetupRoo { mcp_path: PathBuf = PathBuf::new() }
}

section! {
    /// `[setup.qwen]` — Qwen Code's `settings.json` (`hooks`, `mcpServers`) lives under
    /// `dir` (T413.2). `QWEN_HOME` moves that directory; this key is the override.
    SetupQwen { dir: PathBuf = p("~/.qwen") }
}

section! {
    /// `[setup.codebuddy]` — CodeBuddy Code's config directory (`settings.json` `hooks`, the
    /// MCP file, `plugins/`; `CODEBUDDY_CONFIG_DIR` moves it) and WorkBuddy Desktop's, which
    /// embeds the same CLI (T560.2).
    SetupCodebuddy {
        dir: PathBuf = p("~/.codebuddy"),
        workbuddy_dir: PathBuf = p("~/.workbuddy"),
    }
}

section! {
    /// `[setup.antigravity]` — Antigravity 2.0 / IDE plugin root (`plugins_path`, rtok links
    /// `plugins/antigravity` there) and the CLI's staged-plugin root (`cli_plugins_path`,
    /// written only by `agy plugin install`; read to detect it) (T91.1).
    SetupAntigravity {
        plugins_path: PathBuf = p("~/.gemini/config/plugins"),
        cli_plugins_path: PathBuf = p("~/.gemini/antigravity-cli/plugins"),
    }
}

section! {
    /// `[expand]` — `rtok expand <id>`.
    Expand {
        /// 0 = unlimited; caps stdout line count from `rtok expand`.
        max_lines: u32 = 0,
        /// Ceiling on the live-zone re-read rate (T22.5): above it, `rtok report`
        /// finds the compression lossier in practice than it looks.
        max_rate: f64 = 0.05,
    }
}

section! {
    /// `[filter]` — `rtok filter --stdin` (T10.2).
    Filter { cmd: String = String::new() }
}

section! {
    /// `[worktree]` — `rtok worktree …`, MCP `worktree_*` and Claude's worktree hooks (T158).
    Worktree {
        /// Off: the commands refuse, MCP lists no `worktree_*` tool and the host hooks do what
        /// the host does without rtok.
        enabled: bool = true,
        /// Where worktrees are created, as `<root>/<repo>-<task>`.
        root: PathBuf = p("~/.rtok/worktrees"),
    }
}

section! {
    /// `[tasks]` — task adapters (T441): where `rtok task …` and MCP `task_*` store tasks.
    /// Usually set per project in `.rtok.toml`.
    Tasks {
        /// `disk`, `github` or `gitlab`.
        adapter: String = s("disk"),
        /// Task id prefix (`R` → `R12`). Empty: the project name's first letter.
        prefix: String = String::new(),
        disk: TasksDisk = TasksDisk::default(),
        github: TasksGithub = TasksGithub::default(),
        gitlab: TasksGitlab = TasksGitlab::default(),
        export: TasksExport = TasksExport::default(),
    }
}

section! {
    /// `[tasks.export]` — where `rtok task export` mirrors the disk tasks (T441.13.5).
    TasksExport {
        github: TasksExportGithub = TasksExportGithub::default(),
        linear: TasksExportLinear = TasksExportLinear::default(),
    }
}

section! {
    /// `[tasks.export.linear]` — the Linear team whose issues mirror the tasks (T441.13.6).
    TasksExportLinear {
        /// The team's short key, e.g. `ENG`. Empty: `rtok task export linear` refuses to run.
        team: String = String::new(),
    }
}

section! {
    /// `[tasks.export.github]` — the repository whose issues mirror the tasks.
    TasksExportGithub {
        /// `owner/name`. Empty: `[tasks.github] repo`, else the `origin` remote.
        repo: String = String::new(),
    }
}

section! {
    /// `[tasks.disk]` — one Markdown file per task.
    TasksDisk {
        /// Relative to the project root; done tasks move to `<dir>/done`.
        dir: PathBuf = p("tasks"),
    }
}

section! {
    /// `[tasks.github]` — GitHub Issues, sub-issues and a Projects v2 Status field.
    TasksGithub {
        /// `owner/name`. Empty: the `origin` remote.
        repo: String = String::new(),
        /// Projects v2 number under the repo owner (user or organization): each issue joins it and its
        /// `Status` single-select follows the task (open → Todo, in-progress → In Progress, done and
        /// closed → Done). Needs the `project` token scope; any failure only warns. 0 = issues only.
        project: u32 = 0,
    }
}

section! {
    /// `[tasks.gitlab]` — GitLab Issues with `status::` labels; a subtask's issue links to
    /// its parent's.
    TasksGitlab {
        /// https base URL, for self-hosted instances.
        url: String = s("https://gitlab.com"),
        /// `group/name` path or numeric id. Empty: the `origin` remote.
        project: String = String::new(),
    }
}

section! {
    /// `[otel]` — OpenTelemetry export (D19, P16). Off until `endpoint` resolves.
    Otel {
        endpoint: String = String::new(),
        headers: String = String::new(),
        service_name: String = s("rtok"),
        content: bool = true,
        content_bytes: u32 = 65536,
        flush_secs: u32 = 5,
    }
}

/// Where a flush posts: `[otel] endpoint`, else `OTEL_EXPORTER_OTLP_ENDPOINT`; same for headers.
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    /// Base URL without a trailing slash; `/v1/traces` etc. are appended.
    pub url: String,
    pub headers: Vec<(String, String)>,
}

impl Otel {
    /// `None` when neither the key nor the env var names an endpoint — export is off.
    pub fn resolve(&self) -> Option<Endpoint> {
        self.resolve_with(|k| std::env::var(k).ok())
    }

    pub fn resolve_with(&self, env: impl Fn(&str) -> Option<String>) -> Option<Endpoint> {
        let pick = |key: &str, var: &str| -> String {
            if key.trim().is_empty() {
                env(var).unwrap_or_default()
            } else {
                key.to_string()
            }
        };
        let url = pick(&self.endpoint, "OTEL_EXPORTER_OTLP_ENDPOINT");
        let url = url.trim().trim_end_matches('/').to_string();
        if url.is_empty() {
            return None;
        }
        let headers = pick(&self.headers, "OTEL_EXPORTER_OTLP_HEADERS")
            .split(',')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .filter(|(k, _)| !k.is_empty())
            .collect();
        Some(Endpoint { url, headers })
    }
}

// ── plugins ─────────────────────────────────────────────────────────────────

section! {
    /// `[plugins.*]` — one field per catalogue id, in `CATALOGUE` order.
    Plugins {
        measure: Measure = Measure::default(),
        cmd: Cmd = Cmd::default(),
        read: Read = Read::default(),
        json_tree: JsonTree = JsonTree::default(),
        archive: Archive = Archive::default(),
        proxy: ProxyPlugin = ProxyPlugin::default(),
        inject: Inject = Inject::default(),
        guard: Guard = Guard::default(),
        memory: Memory = Memory::default(),
        graph: Graph = Graph::default(),
        toon: Toon = Toon::default(),
        compress: Compress = Compress::default(),
        docs: Docs = Docs::default(),
        wasm: Wasm = Wasm::default(),
    }
}

section! {
    /// `[plugins.measure]`
    Measure { enabled: bool = true }
}

section! {
    /// `[plugins.cmd]`
    Cmd {
        enabled: bool = true,
        rewrite: bool = true,
        shell: String = String::new(),
        rules: PathBuf = p("~/.rtok/rules.toml"),
        /// Drop-in dir: every `*.toml` merges after `rules` in name order (T50.2).
        rules_dir: PathBuf = p("~/.rtok/rules.d"),
        trailer_min_lines: u32 = 40,
        fail_tail_lines: u32 = 80,
        never_wrap: Vec<String> = strs(&["rtok", "sudo"]),
        interactive_stems: Vec<String> = strs(&["python", "node", "psql", "sqlite3", "irb", "bash", "sh", "zsh", "docker", "kubectl"]),
    }
}

section! {
    /// `[plugins.read]`
    Read {
        enabled: bool = true,
        default_mode: String = s("full"),
        max_chars: u32 = 20000,
        native_max_bytes: u64 = 32768,
        /// T383: a native `Read` with `limit` 1..=this passes the read-advice hook.
        range_max_lines: u32 = 300,
        advice: bool = true,
        allow_paths: Vec<PathBuf> = Vec::new(),
        search_max: u32 = 50,
        /// Max bytes `search` will read from one file (T55.5). Larger files are skipped.
        search_max_bytes: u64 = 1_048_576,
        tree_depth: u32 = 2,
        /// T58.1: re-read of a changed file returns a unified diff vs the last archive.
        delta: bool = true,
        /// Serve the full file when the diff is not below this fraction of the file.
        delta_max_ratio: f32 = 0.6,
        /// Deprecated compatibility key; grammars are Cargo features now, so this is ignored.
        languages: Vec<String> = Vec::new(),
    }
}

section! {
    /// `[plugins.archive]`
    Archive {
        enabled: bool = true,
        keep_turns: u32 = 4,
        min_tokens: u32 = 1500,
        head_lines: u32 = 8,
        tail_lines: u32 = 4,
        /// Opt-in L0/L1/L2 tiered loading (P33). Default off — v0.1 archive+inject unchanged.
        /// Behaviour spec: OpenViking (AGPL-3.0); rtok does not vendor, link, or subprocess it
        /// (D6). Gates native implementation in T33.2.
        tiers: bool = false,
        /// Opt-in live-zone blob shrinking (T51.1): nested JSON dumps and `data:` blobs
        /// in user content blocks (never tool results, system, tools, or the last two
        /// turns). Default off until a bench shows cost per passed task does not rise.
        live_blobs: bool = false,
        /// Opt-in skill body archiving in the live zone (T61.2).
        skills: bool = false,
    }
}

section! {
    /// `[plugins.proxy.semantic_cache]` — opt-in response cache (P31). Off until Gate P31.
    SemanticCache {
        enabled: bool = false,
        threshold: f32 = 0.99,
        ttl_s: u64 = 300,
        max_messages: u32 = 1,
        require_empty_tools: bool = true,
        embed_backend: String = s("hash"),
        cache_by_model: bool = true,
        cache_by_provider: bool = true,
    }
}

section! {
    /// `[plugins.proxy]` — the usage-capture plugin, not the `[proxy]` server.
    ProxyPlugin {
        enabled: bool = true,
        semantic_cache: SemanticCache = SemanticCache::default(),
    }
}

section! {
    /// `[plugins.inject]`
    Inject {
        enabled: bool = true,
        budget_tokens: u32 = 800,
        modes_dir: PathBuf = p("~/.rtok/modes"),
        modes: Vec<String> = Vec::new(),
    }
}

section! {
    /// `[plugins.guard]`
    Guard {
        enabled: bool = true,
        window_turns: u32 = 8,
        /// Opt-in (T50.4): deny native `Grep`/`Glob` in PreToolUse and point at
        /// MCP `search`/`tree`. Off by default; also stays silent while the
        /// `read` plugin is disabled (no `search`/`tree` to point at).
        deny_grep_glob: bool = false,
        /// Opt-in (T369): a native `Grep` for one identifier (`foo`, `fn foo`, `class Foo`) is
        /// denied with that symbol's definitions from the index when it has one to five, so
        /// the model skips a second round trip. Index lookup only: no walk, no indexing.
        grep_symbol: bool = false,
        /// Opt-in (T62.1): on `PreToolUse(Skill)` a `SKILL.md` over `skill_max_bytes`
        /// is archived and denied with its markdown map plus the `expand` pointer,
        /// unless its frontmatter carries a key the host applies on invocation.
        skills: bool = false,
        skill_max_bytes: u32 = 8192,
    }
}

section! {
    /// `[plugins.memory.embed]` — optional vector search beside FTS5 (P29; off by default).
    MemoryEmbed {
        enabled: bool = false,
        provider: String = s("local"),
        model: String = s("all-MiniLM-L6-v2"),
        dimensions: u32 = 384,
        hybrid: bool = true,
    }
}

section! {
    /// `[plugins.memory]`
    Memory {
        enabled: bool = true,
        recall_titles: u32 = 5,
        recall_tokens: u32 = 200,
        prompt_recall: u32 = 5,
        checkpoint_tokens: u32 = 400,
        search_limit: u32 = 5,
        sync_tokens: u32 = 300,
        /// SessionStart `source = startup` restores the newest `session:*` note (T71.2).
        startup_recall: bool = true,
        /// Sub-agent handoff MCP tool (T59.6).
        handoff: bool = true,
        /// `SubagentStart` pointer digest appended to a freshly spawned subagent's context
        /// (T130).
        spawn_brief: bool = true,
        /// Token budget for the spawn brief (T130).
        spawn_brief_tokens: u32 = 300,
        embed: MemoryEmbed = MemoryEmbed::default(),
    }
}

section! {
    /// `[plugins.docs]`
    Docs {
        enabled: bool = false,
        query_limit: u32 = 5,
        snippet_chars: u32 = 400,
        max_tokens: u32 = 800,
    }
}

section! {
    /// `[plugins.graph]`
    Graph {
        enabled: bool = true,
        max_tokens: u32 = 2000,
        /// T377: `impact` token budget; a longer answer is grouped by file and cut with a count of
        /// what was left. 0 = no budget.
        impact_tokens: u32 = 1500,
        map_tokens: u32 = 0,
        /// T370: how the SessionStart map orders files: `refs` counts references per name,
        /// `pagerank` ranks files by personalized PageRank over the stored file graph.
        map_rank: String = s("refs"),
        body_lines: u32 = 40,
        auto_index: bool = true,
        auto_add_projects: bool = true,
        /// T329.9: `tags` (default), `lsp` (language server, tags when it cannot answer), `auto`
        /// (server first, tags second, text search when no grammar parses the project, per project
        /// and language) or `text` (T329.10: plain in-process text search only).
        backend: String = s("tags"),
        /// T329.9: how long one language-server wait (an initializing or still-indexing server)
        /// may take before the request falls back to tags.
        lsp_timeout_ms: u32 = 40_000,
        /// T329.9: `backend` for one language, keyed by language name (`go = "tags"`).
        backend_by_language: std::collections::HashMap<String, String> =
            std::collections::HashMap::new(),
        watch: String = s("off"),
        /// T329.8: follow the references a project's manifests make to other directories on this
        /// machine, register them and link them into the graph scope.
        auto_link_references: bool = true,
        /// T329.8: how many reference levels are followed from the project; 0 follows none.
        reference_depth: u32 = 3,
        /// T329.8: the most projects references may add to the registry.
        max_auto_projects: u32 = 20,
        /// T329.17: raise an alert when a project in the scope goes missing, unreachable, loses its
        /// working backend or has a manifest reference that points nowhere.
        alerts: bool = true,
        /// T329.17: seconds between the health checks of a process that hosts graph (`rtok mcp`,
        /// `rtok web`); 0 turns the check off.
        health_check_interval_s: u32 = 60,
        /// T329.34: seconds a node stays warm on the web graph page's live canvas after a call.
        live_heat_window_s: u32 = 300,
        /// T329.34: most call events one second of the live panel lists; counters still count
        /// every call.
        live_max_events_per_s: u32 = 50,
        /// T329.34: finished calls the live panel keeps in its feed.
        live_feed_rows: u32 = 200,
        exclude: Vec<String> = vec![],
        include: Vec<String> = vec![],
        extensions: std::collections::HashMap<String, String> = std::collections::HashMap::new(),
    }
}

section! {
    /// `[plugins.json_tree]`
    JsonTree { enabled: bool = false }
}

section! {
    /// `[plugins.toon]`
    Toon {
        enabled: bool = true,
        min_rows: u32 = 5,
    }
}

section! {
    /// `[plugins.compress]`
    Compress {
        enabled: bool = true,
    }
}

section! {
    /// `[plugins.wasm]` — out-of-tree `.wasm` plugin host (P32). Not a catalogue id;
    /// loading is implemented in T32.2 behind Cargo feature `wasm-host`.
    Wasm {
        enabled: bool = false,
        dir: PathBuf = p("~/.rtok/plugins"),
    }
}

/// One warning from a load: a legacy key, a malformed `.env`, or a file the loader skipped.
/// `message` is the stderr text after `rtok: `; `log` is the line written to the log file.
#[derive(Clone, Debug)]
pub struct LoadNote {
    pub event: &'static str,
    pub message: String,
    pub log: String,
}

/// Warnings from the load that produced this [`Config`]. Equality ignores them, the same way
/// it ignores which file was read. The surface prints `message` and logs `log`.
#[derive(Clone, Debug, Default)]
pub struct LoadNotes(pub Vec<LoadNote>);

impl PartialEq for LoadNotes {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// Path of the user file [`layers::load`] read. Equality ignores it.
#[derive(Clone, Debug, Default)]
pub struct LoadedFrom(pub Option<PathBuf>);

impl PartialEq for LoadedFrom {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

// ── the whole file ──────────────────────────────────────────────────────────

/// Every setting rtok has. Sections mirror the tables in `config/default.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub core: Core,
    pub estimator: Estimator,
    pub log: Log,
    pub hook: Hook,
    pub agents: Agents,
    pub mcp: Mcp,
    pub proxy: Proxy,
    pub web: Web,
    pub tui: Tui,
    pub ui: Ui,
    /// Renamed in T21.3: `[dashboard]` is now `[web]`. Accepted from an old file with a
    /// warning, then dropped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dashboard: Option<Web>,
    pub demon: Demon,
    pub stats: Stats,
    pub report: Report,
    pub bench: Bench,
    pub doctor: Doctor,
    pub setup: Setup,
    pub expand: Expand,
    pub filter: Filter,
    pub worktree: Worktree,
    pub tasks: Tasks,
    pub otel: Otel,
    pub plugins: Plugins,
    /// Directory the config was loaded from; not part of the file.
    #[serde(skip)]
    pub home: PathBuf,
    /// User file this process loaded. Not a setting: equality ignores it, so a loaded
    /// config still matches the same settings built in a test.
    #[serde(skip)]
    pub loaded_from: LoadedFrom,
    /// Warnings from this load. Not a setting; equality ignores them.
    #[serde(skip)]
    pub notes: LoadNotes,
    /// Override for the plugin install receipt path (T279, `agents::plugin_version`):
    /// `$XDG_STATE_HOME/rtok/plugins.json` and OS equivalents by default. Not part of the
    /// file — tests set it directly so a receipt round-trip never touches the real state
    /// directory (D29).
    #[serde(skip)]
    pub plugin_receipt_path: Option<PathBuf>,
    /// T283.3: the pid of the process that runs this hook call (the `rtok-hook` client, or
    /// `rtok hook` itself), so the agent row can record its ancestors. Not part of the file:
    /// each call sets it, and none (a test, an in-process run) records nothing.
    #[serde(skip)]
    pub hook_client_pid: Option<u32>,
}

impl Config {
    /// `$RTOK_HOME` or `$HOME/.rtok`; always absolute (T184).
    ///
    /// Unlike [`env_user_home`], this falls all the way to `std::env::home_dir()` (Unix:
    /// `getpwuid_r` when `HOME` is unset too; not deprecated on the pinned 1.98.1) before
    /// giving up on a user home — scoped to *this* one lookup so it does not also widen every
    /// other `~/x` config default's fallback (see [`env_user_home`]'s doc).
    pub fn home_dir() -> PathBuf {
        let user_home = env_user_home().or_else(std::env::home_dir);
        home_dir_absolute(
            std::env::var_os("RTOK_HOME"),
            user_home,
            || std::env::current_dir().ok(),
            || std::env::temp_dir().join(".rtok"),
        )
    }

    /// `<home>/config.toml`.
    pub fn path_for(home: &Path) -> PathBuf {
        home.join("config.toml")
    }

    /// User config file: `--config`, else `RTOK_CONFIG`, else [`path_for`].
    pub fn user_path(home: &Path, config_file: Option<&Path>) -> PathBuf {
        config_file
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("RTOK_CONFIG").map(PathBuf::from))
            .unwrap_or_else(|| Self::path_for(home))
    }

    pub fn load() -> Result<Self> {
        Self::load_with(None, None)
    }

    /// Read `<home>/config.toml`, creating it from the reference file when absent, then apply
    /// the full layering (user < project < env). Pins the user file to `home` so `RTOK_CONFIG`
    /// cannot leak into tests. No flags — see [`layers::load`] for that.
    pub fn load_from(home: &Path) -> Result<Self> {
        if !Self::path_for(home).exists() {
            Self::init(home, false)?;
        }
        layers::load(home, Some(&Self::path_for(home)), None)
    }

    /// CLI entry: optional `--config` / `RTOK_CONFIG`, plus the `flag` Dict from clap `Some`s.
    pub fn load_with(
        config_file: Option<&Path>,
        flags: Option<figment::value::Dict>,
    ) -> Result<Self> {
        let home = Self::home_dir();
        Self::ensure_user_file(&home, config_file)?;
        layers::load(&home, config_file, flags)
    }

    /// Warn and use defaults. Hooks (T12.3) never fail on a bad file.
    pub fn load_lenient(config_file: Option<&Path>, flags: Option<figment::value::Dict>) -> Self {
        match Self::load_with(config_file, flags) {
            Ok(c) => c,
            Err(e) => {
                let home = Self::home_dir();
                let mut c = Self::default();
                c.finish(&home);
                c.notes.0.push(LoadNote {
                    event: "load",
                    message: format!("config ignored ({e}); using defaults"),
                    log: format!("ignored ({e:#}); using defaults"),
                });
                layers::notify(&c);
                c
            }
        }
    }

    /// Create `<home>/config.toml` from the reference when neither `--config` nor `RTOK_CONFIG`
    /// names a file and the user file is missing.
    pub fn ensure_user_file(home: &Path, config_file: Option<&Path>) -> Result<()> {
        if config_file.is_some() || std::env::var_os("RTOK_CONFIG").is_some() {
            return Ok(());
        }
        if !Self::path_for(home).exists() {
            Self::init(home, false)?;
        }
        Ok(())
    }

    /// Write the reference file, comments included. Refuses to clobber an existing file
    /// unless `force`.
    pub fn init(home: &Path, force: bool) -> Result<PathBuf> {
        Self::init_maybe(home, None, force, false).map(|(p, _)| p)
    }

    /// [`Config::init`] with a preview: `dry_run` renders the `git diff` it would write and
    /// leaves the disk alone. The diff is empty when the file already is the reference file.
    pub fn init_maybe(
        home: &Path,
        config_file: Option<&Path>,
        force: bool,
        dry_run: bool,
    ) -> Result<(PathBuf, String)> {
        let path = Self::user_path(home, config_file);
        if path.exists() && !force {
            bail!("{} exists; pass --force to overwrite", path.display());
        }
        let before = std::fs::read_to_string(&path).unwrap_or_default();
        let diff = validate::file_diff(&path, &before, DEFAULT_TOML);
        if !dry_run {
            write_file(&path, DEFAULT_TOML)?;
        }
        Ok((path, diff))
    }

    /// Migrate legacy keys and expand `~` in paths. Called after every parse.
    fn finish(&mut self, home: &Path) {
        apply_legacy_fold(self);
        let mut notes = Vec::new();
        if let Some(budget) = self.core.inject_budget_tokens.take()
            && self.plugins.inject.budget_tokens == budget
        {
            notes.push(format!(
                "core.inject_budget_tokens is now plugins.inject.budget_tokens (using {budget})"
            ));
        }
        if let Some(web) = self.dashboard.take()
            && self.web == web
        {
            notes.push("[dashboard] is now [web] (using it)".to_string());
        }
        // T24.5 / D26: `[core] log_*` → `[log]`. Taken once so they are not re-read.
        if let Some(path) = self.core.log_file.take()
            && self.log.path == path
        {
            notes.push(format!(
                "core.log_file is now log.path (using {})",
                path.display()
            ));
        }
        if let Some(level) = self.core.log_level.take()
            && self.log.level == level
        {
            notes.push(format!("core.log_level is now log.level (using {level})"));
        }
        if let Some(to_db) = self.core.log_to_db.take()
            && self.log.to_db == to_db
        {
            notes.push(format!("core.log_to_db is now log.to_db (using {to_db})"));
        }
        self.home = home.to_path_buf();
        self.agents.usage.dirs.follow_env(|k| std::env::var_os(k));
        let user_home = env_user_home();
        self.expand_paths_with(home, user_home.as_deref());
        for note in notes {
            self.notes.0.push(LoadNote {
                event: "legacy",
                message: note.clone(),
                log: note,
            });
        }
    }

    /// Resolve every `~` path under `dir`, `~/.rtok/x` and `~/x` alike. For a config that never
    /// passes [`Config::finish`] (tests, `Runtime::in_memory`): a literal `~/.rtok/archive`
    /// resolves against the cwd and grew a `./~` directory in every checkout (T169).
    pub fn rebase_paths(&mut self, dir: &Path) {
        self.expand_paths_with(dir, Some(dir));
    }

    fn expand_paths_with(&mut self, rtok_home: &Path, user_home: Option<&Path>) {
        for (_, path) in self.path_fields_mut() {
            *path = expand_with(path, rtok_home, user_home);
        }
    }

    /// Every path field with its dotted `rtok config set` key — the one list `~` expansion
    /// walks and [`crate::testutil::config_file_in`] writes out (T254).
    pub fn path_fields_mut(&mut self) -> Vec<(Cow<'static, str>, &mut PathBuf)> {
        // The key is spelled from the field path itself, so the two cannot drift apart.
        macro_rules! keyed {
            ($s:ident; $($head:ident $(. $tail:ident)*),+ $(,)?) => {
                vec![$((
                    Cow::Borrowed(concat!(stringify!($head) $(, ".", stringify!($tail))*)),
                    &mut $s.$head $(. $tail)*,
                )),+]
            };
        }
        let mut out = keyed!(self;
            core.db_path,
            core.archive_dir,
            log.path,
            demon.state_dir,
            stats.transcripts_dir,
            stats.codex_dir,
            report.out,
            bench.tasks,
            doctor.settings_path,
            doctor.claude_json,
            doctor.mcp_json,
            setup.claude.settings_path,
            setup.cursor.hooks_path,
            setup.codex.config_path,
            setup.opencode.config_path,
            setup.kilo.config_path,
            setup.pi.extensions_path,
            setup.omp.extensions_path,
            setup.omp.mcp_path,
            setup.zcode.config_path,
            setup.kimi.config_path,
            setup.grok.config_path,
            setup.copilot.dir,
            setup.commandcode.dir,
            setup.vscode.code_user_dir,
            setup.vscode.insiders_user_dir,
            setup.aider.config_path,
            setup.windsurf.config_path,
            setup.zed.config_path,
            setup.cline.hooks_path,
            setup.cline.mcp_path,
            setup.gemini.dir,
            setup.qwen.dir,
            setup.codebuddy.dir,
            setup.codebuddy.workbuddy_dir,
            setup.codewhale.dir,
            setup.mimo.config_path,
            setup.antigravity.plugins_path,
            setup.antigravity.cli_plugins_path,
            setup.devin.config_path,
            plugins.cmd.rules,
            plugins.cmd.rules_dir,
            plugins.inject.modes_dir,
            plugins.wasm.dir,
            worktree.root,
        );
        out.extend(
            self.bench
                .configs
                .iter_mut()
                .map(|(k, v)| (format!("bench.configs.{k}").into(), v)),
        );
        out.extend(
            self.plugins
                .read
                .allow_paths
                .iter_mut()
                .enumerate()
                .map(|(i, p)| (format!("plugins.read.allow_paths[{i}]").into(), p)),
        );
        let dirs = &mut self.agents.usage.dirs;
        for (host, list) in [
            ("opencode", &mut dirs.opencode),
            ("kilo", &mut dirs.kilo),
            ("copilot", &mut dirs.copilot),
            ("gemini", &mut dirs.gemini),
            ("droid", &mut dirs.droid),
            ("pi", &mut dirs.pi),
            ("kimi", &mut dirs.kimi),
            ("grok", &mut dirs.grok),
            ("zcode", &mut dirs.zcode),
            ("antigravity", &mut dirs.antigravity),
        ] {
            out.extend(
                list.iter_mut()
                    .enumerate()
                    .map(|(i, p)| (format!("agents.usage.dirs.{host}[{i}]").into(), p)),
            );
        }
        out
    }

    /// `[plugins.<id>] enabled`. `default_on` is the answer for an id that is not in the
    /// catalogue — an external plugin registered through `Registry::from_plugins`.
    pub fn plugin_enabled(&self, id: &str, default_on: bool) -> bool {
        let p = &self.plugins;
        match id {
            "measure" => p.measure.enabled,
            "cmd" => p.cmd.enabled,
            "read" => p.read.enabled,
            "json_tree" => p.json_tree.enabled,
            "archive" => p.archive.enabled,
            "proxy" => p.proxy.enabled,
            "inject" => p.inject.enabled,
            "guard" => p.guard.enabled,
            "memory" => p.memory.enabled,
            "graph" => p.graph.enabled,
            "toon" => p.toon.enabled,
            "compress" => p.compress.enabled,
            "docs" => p.docs.enabled,
            _ => default_on,
        }
    }

    /// The writer mirror of [`Config::plugin_enabled`]: set `[plugins.<id>] enabled` on
    /// the in-memory copy after the file was written through `validate::set` (the TUI's
    /// plugin toggle, T15.4). An id outside the catalogue is nothing to mirror here —
    /// `validate::set` is what refuses it against the schema.
    pub fn set_plugin_enabled(&mut self, id: &str, on: bool) {
        let p = &mut self.plugins;
        match id {
            "measure" => p.measure.enabled = on,
            "cmd" => p.cmd.enabled = on,
            "read" => p.read.enabled = on,
            "json_tree" => p.json_tree.enabled = on,
            "archive" => p.archive.enabled = on,
            "proxy" => p.proxy.enabled = on,
            "inject" => p.inject.enabled = on,
            "guard" => p.guard.enabled = on,
            "memory" => p.memory.enabled = on,
            "graph" => p.graph.enabled = on,
            "toon" => p.toon.enabled = on,
            "compress" => p.compress.enabled = on,
            "docs" => p.docs.enabled = on,
            _ => {}
        }
    }
}

/// Fold legacy file keys into their replacements only while each target is still at
/// [`Config::default()`]. Env, flags, and an explicit new key win (T36.8).
pub(crate) fn apply_legacy_fold(cfg: &mut Config) {
    let defaults = Config::default();
    if let Some(budget) = cfg.core.inject_budget_tokens
        && cfg.plugins.inject.budget_tokens == defaults.plugins.inject.budget_tokens
    {
        cfg.plugins.inject.budget_tokens = budget;
    }
    if let Some(dash) = cfg.dashboard.as_ref()
        && cfg.web.host == defaults.web.host
        && dash.host != defaults.web.host
    {
        cfg.web.host = dash.host.clone();
    }
    if let Some(dash) = cfg.dashboard.as_ref()
        && cfg.web.port == defaults.web.port
        && dash.port != defaults.web.port
    {
        cfg.web.port = dash.port;
    }
    if let Some(path) = cfg.core.log_file.as_ref()
        && cfg.log.path == defaults.log.path
    {
        cfg.log.path = path.clone();
    }
    if let Some(level) = cfg.core.log_level.as_ref()
        && cfg.log.level == defaults.log.level
    {
        cfg.log.level = level.clone();
    }
    if let Some(to_db) = cfg.core.log_to_db
        && cfg.log.to_db == defaults.log.to_db
    {
        cfg.log.to_db = to_db;
    }
}

/// User home for `~` expansion and `$HOME/.rtok`.
///
/// Prefer `HOME` (Unix and Git Bash). On native Windows PowerShell `HOME` is
/// often unset — fall back to `USERPROFILE` so `rtok agents install` finds
/// `~/.claude` / `~/.cursor` instead of skipping with "not found".
///
/// Deliberately does *not* fall further to `std::env::home_dir()`: every other `~/x` config
/// default (`stats.codex_dir` and friends) is expanded against whatever this returns, and the
/// trycmd fixtures rely on a cleared `HOME` making those resolve to nothing rather than to the
/// real machine's passwd entry (T184) — `getpwuid_r` does not read `HOME` and would defeat that
/// isolation. [`Config::home_dir`] adds that one extra fallback itself, scoped to locating
/// rtok's own home.
pub fn env_user_home() -> Option<PathBuf> {
    user_home_from(std::env::var_os("HOME"), std::env::var_os("USERPROFILE"))
}

/// [`home_dir_from`], made absolute (T184).
///
/// A relative result out of `home_dir_from` is explicit input — a caller set `RTOK_HOME` or
/// `HOME`/`USERPROFILE` to a relative value (trycmd's fixtures rely on exactly this:
/// `RTOK_HOME = "target/tmp/…"` is meant to land under the crate root they run from) — so it is
/// resolved against `cwd()`, matching ordinary shell path semantics.
///
/// With no explicit input at all (`rtok_home` unset/empty and `user_home` unset) there is
/// nothing to resolve relative to except the caller's cwd, which is exactly the bug this closes:
/// a hook run from an arbitrary project directory must not create `.rtok/` in it (T169). That
/// case uses `fallback()` (the OS temp dir in production) instead, keeping hooks fail-open
/// without ever touching the cwd.
fn home_dir_absolute(
    rtok_home: Option<OsString>,
    user_home: Option<PathBuf>,
    cwd: impl FnOnce() -> Option<PathBuf>,
    fallback: impl FnOnce() -> PathBuf,
) -> PathBuf {
    let explicit = rtok_home.as_ref().is_some_and(|h| !h.is_empty()) || user_home.is_some();
    let home = home_dir_from(rtok_home, user_home);
    if home.is_absolute() {
        return home;
    }
    if !explicit {
        return fallback();
    }
    cwd().map(|c| c.join(&home)).unwrap_or(home)
}

/// [`expand_with`] against the process's own user home.
#[cfg(test)]
fn expand(path: &Path, home: &Path) -> PathBuf {
    expand_with(path, home, env_user_home().as_deref())
}

#[cfg(test)]
mod tests {
    #[test]
    fn usage_dirs_follow_a_hosts_relocation_variable_until_the_file_names_one() {
        // Absolute on Windows needs a drive; the strings below compare with `/` separators.
        let root = if cfg!(windows) { "C:" } else { "" };
        let env = |k: &str| match k {
            "XDG_DATA_HOME" => Some(format!("{root}/data").into()),
            "COPILOT_HOME" => Some(format!("{root}/cop").into()),
            "GEMINI_CLI_HOME" => Some("relative/is/ignored".into()),
            "PI_CODING_AGENT_DIR" => Some(format!("{root}/pi").into()),
            "KIMI_CODE_HOME" => Some(format!("{root}/kimi").into()),
            _ => None,
        };
        let mut d = super::UsageDirs {
            kilo: vec!["/mine".into()],
            ..Default::default()
        };
        d.follow_env(env);
        let one = |v: &Vec<std::path::PathBuf>| {
            v.iter()
                .map(|p| p.display().to_string().replace('\\', "/"))
                .collect::<Vec<_>>()
        };
        assert_eq!(one(&d.opencode), [format!("{root}/data/opencode")]);
        assert_eq!(one(&d.copilot), [format!("{root}/cop/session-state")]);
        assert_eq!(one(&d.kilo), ["/mine"]);
        assert_eq!(one(&d.gemini), ["~/.gemini/tmp"]);
        assert_eq!(one(&d.droid), ["~/.factory/sessions"]);
        assert_eq!(one(&d.pi), [format!("{root}/pi/sessions")]);
        assert_eq!(one(&d.kimi), [format!("{root}/kimi/sessions")]);
        assert_eq!(one(&d.grok), ["~/.grok/sessions"]);
        // The narrower pi variable names the folder itself and wins over the agent dir.
        let narrow = |k: &str| match k {
            "PI_CODING_AGENT_SESSION_DIR" => Some(format!("{root}/s").into()),
            "PI_CODING_AGENT_DIR" => Some(format!("{root}/pi").into()),
            _ => None,
        };
        let mut d = super::UsageDirs::default();
        d.follow_env(narrow);
        assert_eq!(one(&d.pi), [format!("{root}/s")]);
    }

    use super::*;
    use figment::Figment;
    use figment::providers::{Format, Toml};
    use figment::value::Value;
    use rstest::rstest;

    /// Parse a TOML string into a `Config` the same way the layered loader does (T12.2: figment's
    /// Toml provider, not the toml crate).
    fn parse(s: &str) -> Result<Config> {
        Figment::from(Toml::string(s)).extract().map_err(Into::into)
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rtok-cfg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Every string leaf of `cfg` that still starts with `~`, as `key = value`. Walks the
    /// serialized config, not a hand list, so a path key `finish` forgets fails here (T169).
    fn tilde_leaves(cfg: &Config) -> Vec<String> {
        fn walk(v: &Value, key: &str, out: &mut Vec<String>) {
            match v {
                Value::String(_, s) if s.starts_with('~') => out.push(format!("{key} = {s}")),
                Value::Dict(_, d) => d
                    .iter()
                    .for_each(|(k, v)| walk(v, &format!("{key}.{k}"), out)),
                Value::Array(_, a) => a.iter().for_each(|v| walk(v, key, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        walk(&Value::serialize(cfg).unwrap(), "", &mut out);
        out
    }

    fn assert_paths_expanded(cfg: &Config) {
        assert_eq!(tilde_leaves(cfg), Vec::<String>::new(), "unexpanded paths");
    }

    /// The Check for T12.1: the reference file is the defaults, exactly.
    #[test]
    fn otel_is_off_until_an_endpoint_resolves() {
        let o = Otel::default();
        assert_eq!(o.resolve_with(|_| None), None);
        let env = |k: &str| match k {
            "OTEL_EXPORTER_OTLP_ENDPOINT" => Some("http://localhost:4318/".to_string()),
            "OTEL_EXPORTER_OTLP_HEADERS" => Some("a=1, b=x=y".to_string()),
            _ => None,
        };
        let e = o.resolve_with(env).unwrap();
        assert_eq!(e.url, "http://localhost:4318");
        assert_eq!(
            e.headers,
            vec![("a".into(), "1".into()), ("b".into(), "x=y".into())]
        );
        let o = Otel {
            endpoint: "https://otel.example/".into(),
            headers: "signoz-ingestion-key=k".into(),
            ..Otel::default()
        };
        let e = o.resolve_with(env).unwrap();
        assert_eq!(e.url, "https://otel.example");
        assert_eq!(e.headers, vec![("signoz-ingestion-key".into(), "k".into())]);
    }

    /// Commented defaults still parse as [`Config::default`], an empty file does the same,
    /// and uncommenting the documented assignments must too — otherwise a comment can lie
    /// about the value a missing key will take.
    #[test]
    fn default_toml_is_the_defaults() {
        for (n, line) in DEFAULT_TOML.lines().enumerate() {
            let trimmed = line.trim();
            assert!(
                trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('['),
                "line {} pins a default: {line}",
                n + 1
            );
        }
        let parsed: Config = parse(DEFAULT_TOML).expect("default.toml parses");
        assert_eq!(
            parsed,
            Config::default(),
            "config/default.toml drifted from Config::default()"
        );
        assert_eq!(
            parse("").unwrap(),
            parsed,
            "an empty file must behave as the reference file"
        );
        assert!(
            super::validate::issues_in(std::path::Path::new("config.toml"), DEFAULT_TOML)
                .is_empty(),
            "rtok config validate must accept the reference file"
        );
        let live = uncomment_documented(DEFAULT_TOML);
        let documented = parse(&live).expect("uncommented reference parses");
        assert_eq!(
            documented, parsed,
            "a commented default does not match Config::default()"
        );
        let notes = super::validate::pinned_notes_in(std::path::Path::new("config.toml"), &live);
        assert!(
            notes.is_empty(),
            "the documented defaults are the current defaults: {notes:?}"
        );
    }

    /// Drop the `# ` that [`DEFAULT_TOML`] puts on assignments and on map tables whose
    /// presence would replace a non-empty default. Prose comments stay comments.
    fn uncomment_documented(src: &str) -> String {
        let mut out = String::new();
        for line in src.lines() {
            if let Some(rest) = line.strip_prefix("# ") {
                let code = rest.split('#').next().unwrap_or("").trim();
                let assignment = code.split_once('=').is_some_and(|(key, _)| {
                    let key = key.trim();
                    !key.is_empty()
                        && key.chars().all(|c| {
                            c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '"' | '.')
                        })
                });
                let map_table =
                    code.starts_with("[bench.configs]") || code.starts_with("[stats.prices.");
                if assignment || map_table {
                    out.push_str(rest);
                    out.push('\n');
                    continue;
                }
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    #[test]
    fn every_catalogue_id_is_answered() {
        let cfg = Config::default();
        for (id, on) in CATALOGUE {
            assert_eq!(cfg.plugin_enabled(id, !on), on, "{id}");
        }
        // An id outside the catalogue falls back to the manifest's default_on.
        assert!(cfg.plugin_enabled("external", true));
        assert!(!cfg.plugin_enabled("external", false));
    }

    /// The writer mirror round-trips through the reader for every catalogue id (T15.4).
    #[test]
    fn set_plugin_enabled_flips_every_catalogue_id() {
        let mut cfg = Config::default();
        for (id, on) in CATALOGUE {
            cfg.set_plugin_enabled(id, !on);
            assert_eq!(cfg.plugin_enabled(id, on), !on, "{id}");
        }
        // An id outside the catalogue is nothing to set; the reader's fallback stands.
        cfg.set_plugin_enabled("external", false);
        assert!(cfg.plugin_enabled("external", true));
    }

    #[test]
    fn creates_reference_file_and_budget_is_800() {
        let home = tmp("create");
        let cfg = Config::load_from(&home).unwrap();
        let written = std::fs::read_to_string(Config::path_for(&home)).unwrap();
        assert_eq!(
            written, DEFAULT_TOML,
            "init must write the reference verbatim"
        );
        assert_eq!(cfg.plugins.inject.budget_tokens, 800);
        assert_eq!(cfg.core.db_path, home.join("rtok.db"));
        assert_eq!(cfg.core.archive_dir, home.join("archive"));
        assert!(cfg.plugin_enabled("toon", true));
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn init_refuses_to_clobber_without_force() {
        let home = tmp("force");
        Config::init(&home, false).unwrap();
        std::fs::write(Config::path_for(&home), "[core]\n").unwrap();
        assert!(Config::init(&home, false).is_err());
        Config::init(&home, true).unwrap();
        assert_eq!(
            std::fs::read_to_string(Config::path_for(&home)).unwrap(),
            DEFAULT_TOML
        );
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn partial_file_keeps_defaults() {
        let cfg: Config =
            parse("[plugins.cmd]\nrewrite = false\n[estimator]\ncode = 4.0\n").unwrap();
        assert_eq!(cfg.plugins.inject.budget_tokens, 800);
        assert_eq!(cfg.estimator.code, 4.0);
        assert_eq!(cfg.estimator.prose, 4.2);
        assert!(!cfg.plugins.cmd.rewrite);
        assert!(cfg.plugins.cmd.enabled);
    }

    #[test]
    fn tools_rewrite_defaults_overlay_and_unknown_key() {
        let d = ToolsRewrite::default();
        assert!(!d.enabled);
        assert_eq!(d.max_description_tokens, 60);
        assert!(d.allow.is_empty());
        assert!(d.deny.is_empty());
        let cfg: Config = parse(
            "[proxy.tools_rewrite]
enabled = true
max_description_tokens = 40
deny = [\"Bash\"]
",
        )
        .unwrap();
        assert!(cfg.proxy.tools_rewrite.enabled);
        assert_eq!(cfg.proxy.tools_rewrite.max_description_tokens, 40);
        assert_eq!(cfg.proxy.tools_rewrite.deny, ["Bash"]);
        let err = parse("[proxy.tools_rewrite]\nbogus = true\n").unwrap_err();
        assert!(err.to_string().contains("bogus"), "{err}");
    }

    #[test]
    fn semantic_cache_defaults_overlay_and_unknown_key() {
        let d = SemanticCache::default();
        assert!(!d.enabled);
        assert_eq!(d.threshold, 0.99);
        assert_eq!(d.ttl_s, 300);
        assert_eq!(d.max_messages, 1);
        assert!(d.require_empty_tools);
        assert_eq!(d.embed_backend, "hash");
        assert!(d.cache_by_model);
        assert!(d.cache_by_provider);

        let cfg: Config = parse(
            "[plugins.proxy.semantic_cache]
enabled = true
threshold = 0.95
",
        )
        .unwrap();
        assert!(cfg.plugins.proxy.semantic_cache.enabled);
        assert_eq!(cfg.plugins.proxy.semantic_cache.threshold, 0.95);

        let err = parse(
            "[plugins.proxy.semantic_cache]
bogus = true
",
        )
        .unwrap_err();
        assert!(err.to_string().contains("bogus"), "{err}");
    }

    #[test]
    fn read_languages_is_accepted_only_for_compatibility() {
        let cfg = parse("[plugins.read]\nlanguages = [\"rust\", \"ts\"]\n")
            .expect("legacy languages key should still parse");
        assert_eq!(cfg.plugins.read.languages, ["rust", "ts"]);

        let err = parse("[plugins.read]\nlangauges = [\"rust\"]\n").unwrap_err();
        assert!(err.to_string().contains("langauges"), "{err}");
    }

    #[test]
    fn unknown_key_is_an_error() {
        let err = parse("[proxy]\nprot = 1\n").unwrap_err();
        assert!(err.to_string().contains("prot"), "{err}");
        assert!(parse("[nope]\nx = 1\n").is_err());
        assert!(parse("[plugins.compress]\nunknown = true\n").is_err());
        let err = parse("[plugins.memory.embed]\nprot = 1\n").unwrap_err();
        assert!(err.to_string().contains("prot"), "{err}");
        assert!(parse("[plugins.archive]\nno_such = 1\n").is_err());
    }

    #[test]
    fn compress_defaults_on_and_overlays_turn_off() {
        use super::layers;

        assert!(Config::default().plugins.compress.enabled);

        let cfg: Config = parse("[plugins.compress]\nenabled = false\n").unwrap();
        assert!(!cfg.plugins.compress.enabled);

        let home = tmp("compress-env");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".env"), "RTOK_PLUGINS_COMPRESS_ENABLED=false\n").unwrap();
        let cfg = layers::load(&home, None, None).unwrap();
        assert!(!cfg.plugins.compress.enabled);
        let row = layers::entries(&layers::figment(&home, None, None))
            .into_iter()
            .find(|(k, _, _)| k == "plugins.compress.enabled")
            .expect("leaf key listed");
        assert_eq!(row.1, "false");
        assert_eq!(row.2, "dotenv");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn memory_embed_defaults_off() {
        let cfg = Config::default();
        assert!(!cfg.plugins.memory.embed.enabled);
        assert_eq!(cfg.plugins.memory.embed.provider, "local");
        assert_eq!(cfg.plugins.memory.embed.model, "all-MiniLM-L6-v2");
        assert_eq!(cfg.plugins.memory.embed.dimensions, 384);
        assert!(cfg.plugins.memory.embed.hybrid);
    }

    #[test]
    fn memory_embed_enabled_from_file() {
        let cfg: Config = parse("[plugins.memory.embed]\nenabled = true\n").unwrap();
        assert!(cfg.plugins.memory.embed.enabled);
    }

    #[test]
    fn graph_backend_defaults_to_tags() {
        assert_eq!(Config::default().plugins.graph.backend, "tags");
        assert_eq!(Config::default().plugins.graph.map_tokens, 0);
    }

    #[test]
    fn graph_backend_overlay_from_toml() {
        let cfg = parse("[plugins.graph]\nbackend = \"lsp\"\n").unwrap();
        assert_eq!(cfg.plugins.graph.backend, "lsp");
    }

    /// T329.9: `auto` and the per-language table parse, the timeout defaults to 40 s, `text`
    /// (T329.10) is accepted, and a backend the graph does not know is refused by validation.
    #[test]
    fn graph_auto_backend_keys() {
        let g = Config::default().plugins.graph;
        assert_eq!((g.lsp_timeout_ms, g.backend_by_language.len()), (40_000, 0));
        let toml = "[plugins.graph]\nbackend = \"auto\"\nlsp_timeout_ms = 500\n\
                    [plugins.graph.backend_by_language]\ngo = \"tags\"\njava = \"text\"\n";
        let cfg = parse(toml).unwrap();
        assert_eq!(cfg.plugins.graph.backend, "auto");
        assert_eq!(cfg.plugins.graph.lsp_timeout_ms, 500);
        assert_eq!(cfg.plugins.graph.backend_by_language["go"], "tags");
        let dir = std::env::temp_dir().join(format!("rtok-auto-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.toml");
        std::fs::write(&path, toml).unwrap();
        assert!(validate::issues(&path).unwrap().is_empty());
        std::fs::write(&path, "[plugins.graph]\nbackend = \"text\"\n").unwrap();
        assert!(validate::issues(&path).unwrap().is_empty());
        for bad in [
            "[plugins.graph]\nbackend = \"grep\"\n",
            "[plugins.graph.backend_by_language]\ngo = \"rg\"\n",
        ] {
            std::fs::write(&path, bad).unwrap();
            let errs = validate::issues(&path).unwrap();
            assert!(
                errs.iter().any(|e| e.contains("plugins.graph.backend")),
                "{bad}: {errs:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn graph_backend_unknown_key_is_an_error() {
        assert!(parse("[plugins.graph]\nback_end = \"lsp\"\n").is_err());
    }

    /// T33.1: `plugins.archive.tiers` defaults off, overlays, and maps to `RTOK_PLUGINS_ARCHIVE_TIERS`.
    #[test]
    fn archive_tiers_defaults_and_overlays() {
        assert!(!Config::default().plugins.archive.tiers);
        let cfg: Config = parse("[plugins.archive]\ntiers = true\n").unwrap();
        assert!(cfg.plugins.archive.tiers);
        assert!(layers::leaf_keys().contains(&"plugins.archive.tiers".to_string()));
    }

    #[test]
    fn default_expands_every_pathbuf() {
        let home = Path::new("/tmp/rtok-tilde-default");
        let mut cfg = Config::default();
        cfg.finish(home);
        assert_paths_expanded(&cfg);
    }

    #[rstest]
    #[case::report_out("[report]\nout = \"~/rtok-report.md\"\n")]
    #[case::bench_tasks("[bench]\ntasks = \"~/bench/tasks.toml\"\n")]
    #[case::bench_configs("[bench.configs]\ncustom = \"~/bench/rtok.json\"\n")]
    #[case::allow_paths("[plugins.read]\nallow_paths = [\"~/src\"]\n")]
    #[case::wasm_dir("[plugins.wasm]\ndir = \"~/plugins\"\n")]
    #[case::doctor_mcp_json("[doctor]\nmcp_json = \"~/.mcp.json\"\n")]
    #[case::worktree_root("[worktree]\nroot = \"~/worktrees\"\n")]
    fn tilde_expands_for_every_path_key(#[case] toml: &str) {
        let home = Path::new("/tmp/rtok-tilde-keys");
        let mut cfg: Config = parse(toml).unwrap();
        cfg.finish(home);
        assert_paths_expanded(&cfg);
    }

    #[test]
    fn user_home_prefers_home_then_userprofile() {
        use std::ffi::OsString;
        assert_eq!(
            user_home_from(
                Some(OsString::from("/Users/me")),
                Some(OsString::from(r"C:\Users\me"))
            ),
            Some(PathBuf::from("/Users/me"))
        );
        assert_eq!(
            user_home_from(None, Some(OsString::from(r"C:\Users\Example"))),
            Some(PathBuf::from(r"C:\Users\Example"))
        );
        // Empty HOME must fall through to USERPROFILE (native Windows).
        assert_eq!(
            user_home_from(
                Some(OsString::from("")),
                Some(OsString::from(r"C:\Users\Example"))
            ),
            Some(PathBuf::from(r"C:\Users\Example"))
        );
        assert_eq!(user_home_from(None, None), None);
        assert_eq!(
            user_home_from(Some(OsString::from("")), Some(OsString::from(""))),
            None
        );
    }

    /// T169: a literal `~` in `RTOK_HOME` is expanded, never kept as a relative path.
    #[rstest]
    #[case::unset(None, "/Users/me/.rtok")]
    #[case::absolute(Some("/srv/rtok"), "/srv/rtok")]
    #[case::rtok_tilde(Some("~/.rtok"), "/Users/me/.rtok")]
    #[case::other_tilde(Some("~/state/rtok"), "/Users/me/state/rtok")]
    fn rtok_home_expands_a_literal_tilde(#[case] env: Option<&str>, #[case] want: &str) {
        let got = home_dir_from(env.map(Into::into), Some(PathBuf::from("/Users/me")));
        assert_eq!(got, PathBuf::from(want));
    }

    /// T184: `Config::home_dir` never hands back a path relative to an unknown cwd.
    #[test]
    fn home_dir_absolute_never_relative() {
        // Nothing resolves at all: fail open to the fallback, never the cwd.
        assert_eq!(
            home_dir_absolute(
                None,
                None,
                || Some(PathBuf::from("/cwd")),
                || { PathBuf::from("/tmp/rtok-fallback/.rtok") }
            ),
            PathBuf::from("/tmp/rtok-fallback/.rtok")
        );
        // An explicit relative RTOK_HOME (trycmd's `RTOK_HOME = "target/tmp/…"`) resolves
        // against the cwd, not the fallback.
        assert_eq!(
            home_dir_absolute(
                Some(OsString::from("target/tmp/case")),
                None,
                || Some(PathBuf::from("/repo")),
                || PathBuf::from("/tmp/rtok-fallback/.rtok"),
            ),
            PathBuf::from("/repo/target/tmp/case")
        );
        // An already-absolute result is returned as-is; cwd/fallback are not consulted.
        // `temp_dir()` is absolute on every platform (`/srv/rtok` is not on Windows).
        let abs = std::env::temp_dir().join("srv-rtok");
        assert_eq!(
            home_dir_absolute(
                Some(abs.clone().into_os_string()),
                None,
                || panic!("cwd should not be read"),
                || panic!("fallback should not run"),
            ),
            abs
        );
        // A relative HOME (no RTOK_HOME) is explicit input too: absolutized, not defaulted.
        assert_eq!(
            home_dir_absolute(
                None,
                Some(PathBuf::from("rel-home")),
                || Some(PathBuf::from("/repo")),
                || PathBuf::from("/tmp/rtok-fallback/.rtok"),
            ),
            PathBuf::from("/repo/rel-home/.rtok")
        );
        // An empty RTOK_HOME counts as unset, not as explicit input.
        assert_eq!(
            home_dir_absolute(
                Some(OsString::new()),
                None,
                || Some(PathBuf::from("/cwd")),
                || { PathBuf::from("/tmp/rtok-fallback/.rtok") }
            ),
            PathBuf::from("/tmp/rtok-fallback/.rtok")
        );
    }

    #[test]
    fn expand_covers_bare_tilde_and_rtok_home_dir() {
        let home = Path::new("/tmp/rtok-home");
        assert_eq!(expand(Path::new("~/.rtok"), home), home);
        assert_eq!(expand(Path::new("~/.rtok/"), home), home);
        assert_eq!(expand(Path::new("~/.rtok/db"), home), home.join("db"));
        if let Some(h) = env_user_home() {
            assert_eq!(expand(Path::new("~"), home), h);
            assert_eq!(
                expand(Path::new("~/.claude/settings.json"), home),
                h.join(".claude/settings.json")
            );
        }
    }

    #[test]
    fn expand_with_userprofile_resolves_cursor_and_claude_paths() {
        // Regression: without this, Windows `rtok agents install cursor|claude`
        // printed "not found, not installed" because `~/...` stayed literal.
        let rtok = Path::new("/tmp/rtok-home");
        let profile = Path::new(r"C:\Users\Example");
        assert_eq!(
            expand_with(Path::new("~/.cursor/hooks.json"), rtok, Some(profile)),
            profile.join(".cursor/hooks.json")
        );
        assert_eq!(
            expand_with(Path::new("~/.cursor/mcp.json"), rtok, Some(profile)),
            profile.join(".cursor/mcp.json")
        );
        assert_eq!(
            expand_with(Path::new("~/.claude/settings.json"), rtok, Some(profile)),
            profile.join(".claude/settings.json")
        );
        assert_eq!(
            expand_with(Path::new("~/.claude.json"), rtok, Some(profile)),
            profile.join(".claude.json")
        );
        // No user home → leave `~/...` literal (same as pre-fix Windows).
        assert_eq!(
            expand_with(Path::new("~/.cursor/hooks.json"), rtok, None),
            PathBuf::from("~/.cursor/hooks.json")
        );
        // PowerShell-style backslash tilde paths must expand too.
        assert_eq!(
            expand_with(Path::new("~\\.cursor\\hooks.json"), rtok, Some(profile)),
            profile.join(".cursor").join("hooks.json")
        );
        assert_eq!(
            expand_with(Path::new("~\\.rtok\\db"), rtok, Some(profile)),
            rtok.join("db")
        );
        assert_eq!(
            expand_with(Path::new("~/.rtok\\db"), rtok, Some(profile)),
            rtok.join("db")
        );
    }

    #[test]
    fn legacy_budget_key_migrates() {
        let mut cfg: Config = parse("[core]\ninject_budget_tokens = 250\n").unwrap();
        cfg.finish(Path::new("/tmp/rtok-legacy"));
        assert_eq!(cfg.plugins.inject.budget_tokens, 250);
        assert_eq!(cfg.core.inject_budget_tokens, None);
    }

    /// T24.5: an old `[core] log_*` file folds into `[log]` once and clears the legacy keys.
    #[test]
    fn legacy_core_log_keys_migrate_into_log() {
        let mut cfg: Config = parse(
            "[core]\nlog_file = \"/tmp/old.log\"\nlog_level = \"debug\"\nlog_to_db = false\n",
        )
        .unwrap();
        cfg.finish(Path::new("/tmp/rtok-legacy-log"));
        assert_eq!(cfg.log.path, PathBuf::from("/tmp/old.log"));
        assert_eq!(cfg.log.level, "debug");
        assert!(!cfg.log.to_db);
        assert_eq!(cfg.core.log_file, None);
        assert_eq!(cfg.core.log_level, None);
        assert_eq!(cfg.core.log_to_db, None);
    }

    /// T24.5: `config validate` rejects legacy keys — they are absent from the reference schema.
    #[test]
    fn validate_rejects_legacy_core_log_keys() {
        let dir = std::env::temp_dir().join(format!("rtok-val-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.toml");
        std::fs::write(
            &path,
            "[core]\nlog_file = \"/tmp/x.log\"\nlog_level = \"debug\"\nlog_to_db = false\n",
        )
        .unwrap();
        let errs = validate::issues(&path).unwrap();
        let joined = errs.join("\n");
        assert!(
            joined.contains("log_file")
                || joined.contains("log_level")
                || joined.contains("log_to_db"),
            "expected unknown-key errors, got {errs:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn wasm_disabled_by_default() {
        assert!(!Config::default().plugins.wasm.enabled);
    }

    #[test]
    fn wasm_overlay_enables() {
        let cfg: Config = parse("[plugins.wasm]\nenabled = true\n").unwrap();
        assert!(cfg.plugins.wasm.enabled);
    }

    #[test]
    fn wasm_unknown_key_is_denied() {
        let err = parse("[plugins.wasm]\nfuel_per_call = 1\n").unwrap_err();
        assert!(err.to_string().contains("fuel_per_call"), "{err}");
    }
}
