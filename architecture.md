# rtok architecture

One static Rust binary. Every token-reduction method is a plugin behind one trait. Three
surfaces reach the plugins: Claude Code hooks, an MCP server, and an API proxy. One SQLite
file records what every plugin did, before and after, so a saving is a row or it does not
exist.

This document describes the shape; `plan.md` holds the decisions (D1–D14) and the tasks;
`research.md` holds the evidence. `roadmap.md` is the per-plugin build plan. `ideas.md` holds propositions not yet in `plan.md`.

## 1. Principles the code enforces

| Principle | Where it lives |
|-----------|----------------|
| Fail open: a hook exits 0 in ≤ 10 ms even on error, output `{}` | `hooks::types::HookOutput::default()` serialises to `{}`; the dispatcher (T2.1) wraps plugins in `catch_unwind` |
| Lossless by default: anything shortened is retrievable via `expand <id>` | `archive` table + `~/.rtok/archive/`; every capped output carries an id |
| A saving that is not a `Measurement` row does not exist | `plugin::Measurement` is the only type `Ctx::record` accepts; `measurements` table |
| Injected context is budgeted and byte-stable | single `inject` plugin; `core.inject_budget_tokens` |
| PostToolUse can only add context | `Plugin::post_tool` returns `Option<String>` (additionalContext), nothing else |
| v0.1: no daemon on the hook path, no subprocess plugins, no WASM (D1/D6); `rtok demon` supervises long-running surfaces only (D22); the one exception is the optional resident `rtok hook --serve`, which the `rtok-hook` client bypasses when it does not answer (D32) | plugins are in-tree modules behind Cargo features; WASM remains Later |
| Every plugin is written here from scratch; no third-party tool on any code path (D6) | `Manifest` has no adapter kind; T0.8 Check greps `src/plugins` for retired tool names |
| Every CLI flag is a config key; one precedence rule (D12, D14) | clap 4 derive; figment layers + provenance; toml_edit for `config set`; `tests/config_coverage.rs` walks the clap tree |

## 2. Layers

Production modules form a DAG. `tests/module_graph.rs` fails the build on a cycle
or on an edge that points up.

```
surfaces     cli  hooks  mcp  proxy  web  tui  demon
                │
                ▼
app          plugins  agents  measure  report  doctor
                │
                ▼
core         plugin (host Runtime)  config  store  log  tokens
                │
                ▼
foundation   fs  sanitize  proc  tls  names  lane  diff  since
             stdio  task_id  logfile  progress  bytes  project
                │
                ▼
             crates/rtok-plugin-sdk     (the published contract, D25)
```

A module may use another module of the same rank when that does not close a cycle.
Surfaces may call one another on the same condition (`cli` reaches `hooks`; `web`
reaches `demon`). Modules that sit beside this stack (`render`, `model`, `expand`,
`info`, `otel`, `worktree`, and the rest) must not name `cli`, `web`, or `tui`.
They also do not write the terminal: text goes to `stdio::stdout` / `stdio::stderr`
(`log` re-exports them), and `cli::run` prints it. `clippy::print_stdout` and
`print_stderr` are denied outside the surface modules.

`config`, `store`, `log`, and `tokens` reference only core and foundation.
`plugin` is the host runtime: it may use `project` and the other core modules,
and it does not use a surface or an app module. The operator model lives in
`src/model` (D23), split by page. `src/web` re-exports it for the dashboard; the
TUI and the reading commands render the same values and do not name `crate::web`
for them. The `rtok web` command is the one caller of the server.

## 3. Module map

| Path | Role | Plan task |
|------|------|-----------|
| `src/main.rs` | clap CLI; each subcommand is a thin call into the library | T0.1 |
| `src/lib.rs` | crate root; declares the modules below | — |
| `crates/rtok-config/` + `src/config/` | `Config::load()`, defaults, `[plugins.<id>]`, `CATALOGUE`. `src/config` re-exports the crate | T0.2 |
| `crates/rtok-config/src/layers.rs`, `validate.rs`, `default.toml` | figment providers (D14); `rtok config show/validate/set` (see `docs/config.md`). Load warnings stay on `Config::notes`; the CLI prints and logs them | P12 |
| `crates/rtok-store/` + `crates/rtok-store/migrations/` | Diesel models; `Store::open`; `insert_call`/`tokens`/`log`; `insert_measurement`. `src/store` re-exports the crate | T0.3, P13 |
| `crates/rtok-store/src/symbols.rs` | The `graph` symbol index over SQLite (Ladybug/Grafeo backends removed, P39) | T8.10; P39 |
| `src/testutil.rs`, `tests/common/` | test-only: a fresh temp dir and a `Config`/`Runtime` confined to it; the nearest-rank p95 the latency gates share; `agents::real_config` seeds the invoking user's own host configs into a throwaway home and answers `None` under `CI`, so the checks that read them are local-only and skip everywhere else | T34.2, T34.3, T78 |
| `src/plugin.rs` | host `Runtime`: opens the store, estimates tokens, records measurements. The contract types are in `crates/rtok-plugin-sdk` (§4) | T0.4, D25 |
| `src/plugins/mod.rs` | feature-gated module list, `all()`, `Registry` | T0.4 |
| `src/plugins/<id>/` | one plugin: `mod.rs` + `README.md` (what/why) + `AGENTS.md` (how to work on it) | per plugin |
| `src/tokens.rs` | `estimate(text, Class, &Estimator)`, `tokens_saved` | T0.5 |
| `src/hooks/types.rs` | `HookInput`, `HookOutput`, event views | T0.6 |
| `src/hooks/mod.rs` | dispatcher: merge plugin outputs, log `events`, fail open | T2.1 |
| `src/mcp.rs` | rmcp stdio server built from `Plugin::mcp_tools()` | T4.1 |
| `src/proxy/` | axum passthrough + `compress` mode via `Plugin::proxy_filter()` | T5.1 |
| `src/proxy/wire.rs`, `anthropic.rs`, `openai_chat.rs`, `openai_responses.rs` | `Wire` adapters: one per API format, exposing tool results and `usage` in one normalised shape (D11) | P11 |
| `src/tui/` | ratatui operator dashboard: `rtok tui` (D17, P15) | P15 |
| `src/otel/` | OTLP/HTTP JSON projection of the ledgers: `otlp.rs` encoder, `map.rs` GenAI semconv mapping, `export.rs` flush + watermarks, `metrics.rs` sums; `rtok otel flush | status` (D19) | P16 |
| `src/web/` | axum WebSocket + the embedded React SPA (`web/`, built to `web/dist`, served by `src/web/spa.rs`): `rtok web` (D20; `rtok dashboard` is the deprecated spelling). Serves the D23 operator model `rtok tui` also renders; the SPA is not linked into the hook binary and adds nothing to the hook path. `/ws` refuses a browser upgrade whose `Origin` host differs from `Host`, or whose `Host` is a DNS name other than `localhost` (DNS rebinding); header-less clients pass (T193). | P19 |
| `src/measure/` | JSONL ingest, `rtok stats`, baselines, cache report | P1 |
| `src/agents/` | agent hosts (`rtok agents install\|remove\|list`): one folder per host, each `<host>/mod.rs` implementing the `Agent` contract (variants, files, installed modules, apply) and `<host>/README.md` saying which rtok modules it takes and why the rest cannot be taken; a test keeps README and `support()` in step. Backups and `--dry-run` come from `rtok-agent-sdk`; a host that ships a plugin declares it as one `agents::plugin::HostPlugin` (source, destination, label, host name) instead of respelling the link cycle. | T2.3, P10, T44.2, T77 |
| `examples/hello_plugin.rs` | smallest complete plugin, run by CI | — |
| `tests/fixtures/hooks/*.json` | one real payload per hook event | T0.6 |

## 4. The plugin contract

The trait and the types a plugin names (`Plugin`, `Manifest`, `Ctx`, `Measurement`,
`Class`, the event views) live in `crates/rtok-plugin-sdk` (decision D25).
`src/plugin.rs` is the host `Runtime`: it opens the store, estimates tokens, and
records measurements. It is not the contract an out-of-tree plugin implements.

The sketch below is the shape of that contract.

```rust
pub trait Plugin: Send + Sync {
    fn manifest(&self) -> Manifest;                                   // id, surfaces, default_on
    fn pre_tool(&self, ev: &PreToolUse, cx: &Ctx) -> Option<PreToolDecision>;   // Deny | Rewrite
    fn post_tool(&self, ev: &PostToolUse, cx: &Ctx) -> Option<String>;          // additionalContext only
    fn session_start(&self, ev: &SessionStart, cx: &Ctx) -> Option<Injection>;
    fn prompt_submit(&self, ev: &PromptSubmit, cx: &Ctx) -> Option<Injection>;
    fn pre_compact(&self, ev: &PreCompact, cx: &Ctx);
    fn mcp_tools(&self) -> Vec<ToolDef>;
    fn proxy_filter(&self, req: &mut MessagesRequest, cx: &Ctx) -> Vec<Measurement>;
}
```

Every method has a no-op default, so a plugin implements only the surfaces its manifest
declares. Event types are borrowed views over `HookInput`, so no copying happens on the hook
path.

`Ctx` is the whole world a plugin sees:

```rust
pub struct Ctx { pub config: Config, pub store: Store, pub session: String, pub call_id: Option<i32> } // call_id: owning `calls` row (proxy api_request), parent of `record_plugin_run` rows
impl Ctx {
    fn estimate(&self, text: &str, class: Class) -> u32;   // tokens, ±15 %
    fn plugin_cfg(&self, id: &str) -> Option<&PluginCfg>;  // [plugins.<id>] table
    fn record(&self, m: &Measurement) -> Result<()>;       // the only way to claim a saving
}
```

The archive store (`~/.rtok/archive/`) joins `Ctx` in T3.1.

### Merge rules in the dispatcher (T2.1)

- PreToolUse: the first `Deny` wins; `Rewrite` is last-writer; anything else passes through.
- PostToolUse: `additionalContext` strings are concatenated, then capped by the inject budget.
- SessionStart / UserPromptSubmit: all `Injection`s go to `inject`, which sorts by priority
  and emits until the budget.
- Any panic or error inside a plugin → that plugin's output is dropped, the event is logged
  with the error, and the hook still exits 0 with whatever the other plugins produced.

## 5. One kind of plugin

Every plugin is native Rust written from scratch in this repo (decision D6). No plugin
spawns, links, imports, or reads the data of a third-party tool. The tools rtok replaces are
specs (`research.md`); their names appear in code only where rtok inspects them (`doctor`),
retires them (`setup --replace`) or benches against them.

Third parties extend rtok from outside: depend on the `rtok` library, implement `Plugin`,
and build a binary with `Registry::from_plugins(vec![Box::new(Mine)], &config)` (T0.8).
`docs/plugin-authoring.md` and `examples/` are the whole public surface; this repo ships no
third-party plugins.

## 6. Compile-time and run-time selection

- **Compile time**: one Cargo feature per plugin id, `default = all`. `plugins::all()` pushes
  each plugin under `#[cfg(feature = "<id>")]`. `cargo build --no-default-features
  --features measure` must always succeed (T0.4 Check) — this keeps every plugin decoupled
  from every other plugin.
- **Run time**: `[plugins.<id>] enabled = bool` in config; unset → `Manifest::default_on`.
  `Registry::new(&config)` resolves both and exposes `enabled()` in dispatch order.
- `config::CATALOGUE` is the single list of `(id, default_on)`; a test asserts the registry's
  manifests match it, so a new plugin cannot be half-registered.

## 7. Data

One SQLite file, WAL mode, opened per invocation (hooks are short-lived processes; SQLite
handles the concurrency). Migrations are `crates/rtok-store/migrations/NNNN_<slug>/up.sql`,
embedded by the store crate, applied once each and recorded in `schema_migrations`. Editing
an applied migration is forbidden; add the next directory.

| Table | Written by | Read by |
|-------|-----------|---------|
| `hosts`, `providers`, `models`, `sessions` | `Store` upsert | every `calls` row |
| `calls` + `call_io` | dispatcher, mcp, proxy | `rtok stats`, doctor |
| `hook_sessions` | dispatcher: a hook stdin's session fields, once per distinct set (not with `store_raw`) | `call_io` readers, which splice them back into the body |
| `tokens` | same surfaces (`before`/`after`/`mcp`) | `rtok stats --plugin <id>` |
| `logs` | core + plugins via `Ctx::log` | doctor, debug |
| `events` | (superseded; 0001 leftover) | — |
| `measurements` | `Ctx::record` (optional `call_id`; a hook row carries `once_key`, unique per call delivery, T245) | `rtok stats --plugin <id>`, bench |
| `archive` | `cmd`, `read`, `archive`, `call_io` spill | `rtok expand`, `guard` |
| `read_cache` | `read` | `read` (dedup) |
| `notes` + `notes_fts` | `memory` | `memory` |
| `note_files` | `memory` (`mem_save`, `remember:`: files the body names, the checkpoint's paths; T374) | `memory` prompt recall (RRF boost for files the session read or the prompt names; T452: up to 2 notes of a file the prompt names are recalled without a text match) |
| `usage` | `proxy` (optional `call_id`) | `measure` |

Raw payloads live on disk under `archive_dir/<id>`; the DB holds size, sha256 and path.

## 8. Measurement is the product

The metric is **context-token-turns**: a tool result of T tokens produced at turn t of an
N-turn session costs T × (N − t), because it is re-sent (cached or not) on every later turn.
Output tokens are counted separately. Estimates come from `tokens::estimate` and are
labelled as such; real counts come only from proxy `usage` rows.

The honesty metric for any lossless shortening is the **expand rate**: how often the model
had to ask for the original. Gates in `plan.md` keep a plugin only if the expand rate stays
under 5 %.

## 9. Extending

- **New plugin**: `docs/plugin-authoring.md` — module, manifest, feature, registry push,
  `CATALOGUE` entry, README + AGENTS, one test, one measurement path.
- **New hook event**: add fields to `HookInput`, a view struct in `plugin.rs`, an accessor,
  a fixture, and a trait method with a default body.
- **New host**: a `src/agents/<host>/` folder — `mod.rs` implements `Agent` (variants with
  binaries and app paths, config files, installed-module markers, apply) and `README.md`
  carries the module table the parity test reads — plus, if the payload differs, a field
  mapping into `HookInput`. The plugins do not change.
- **New surface**: a new module under `src/` that builds a `Registry` and calls the trait; operator TUI (`src/tui/`, D17) and web (`src/web/`, D20) read `Store`/`stats`/`doctor` through the D23 model; both call `Plugin::dashboard_page`. They do not run on the hook path;
  add a `Surface` variant so manifests can declare it.
- **New API wire format** (e.g. Gemini): implement `Wire` in `src/proxy/<name>.rs` — route
  match, tool-result accessor, usage parser for body and SSE — plus fixtures. Plugins do not
  change; `usage.api` gets a new value.

## 10. Testing strategy

- Unit tests next to the code (`cargo test`); every task in `plan.md` has one machine Check.
- `tests/deps.rs` asserts heavy-dep ownership. `diesel` is only `rtok-store` (nothing under `src/` names it). `ratatui` and `crossterm` are only `src/tui/` on the `rtok` package. `printpdf` is only `src/report/`. `wasmi` is only `src/plugins/wasm.rs`. `tree-sitter*` is only `src/plugins/`. `rmcp` is only `src/mcp`. `axum` is `src/proxy/`, `src/web/`, and `src/mcp/` (HTTP). `dotenvy` is only `rtok-config`. `rtok-plugin-sdk` depends on no workspace crate. `rtok-hook` is std-only.
- Fixture-driven: hook payloads in `tests/fixtures/hooks/`, golden filter cases in
  `tests/cmd_golden/`, per-language outline fixtures for `read`.
- Latency harness (`tests/latency.rs`, T2.2) asserts p95 < 10 ms for a hook round trip.
- `just full-check` = `fmt --check` + `clippy --all-targets --all-features -D warnings` + tests +
  the single-feature build; CI runs these gates on macOS and Linux with the toolchain from
  `mise.toml`. `just check` (T420) runs the same gates only for what the change touches.
- `examples/hello_plugin.rs` runs in CI and asserts a measurement row was written.

## 11. Not in v0.1

v0.1 has no LLM-based compression, embeddings, type-resolved call graph (the `graph` plugin
is a tree-sitter-tags index), semantic response cache, or WASM plugin host. Those remain
**v0.2+** (`plan.md` Later versions, `ideas.md` Later, `roadmap.md` Later), not discarded.
`rtok tui` (P15) and `rtok demon` (P20) shipped in-tree after promotion from Later.
Adapters over third-party tools stay out of *this repo* at every version (D6); a later WASM
host loads plugins that live outside this repo.

## 12. Batch / Flex pass

Batch, Flex, and model routing are **proxy-only** concerns. Hooks and MCP never see LLM
HTTP bodies; only `rtok proxy` (`src/proxy/`) sits on `ANTHROPIC_BASE_URL` /
`OPENAI_BASE_URL`. See `docs/batch-flex.md`.

Data flow for one proxied request (`src/proxy/mod.rs` `handle` → `shape_request`):

```
client ──POST /v1/…──► axum fallback
                         │
                         ├─ plain? (proxy/core/plugins.proxy disabled)
                         │     └─ byte-forward, no record/compress/prepare
                         │
                         └─ shape_request
                               ├─ record      → calls / call_io
                               ├─ compress    → Plugin::proxy_filter (mode=compress)
                               ├─ tools_rewrite (opt-in)
                               ├─ prepare     → Wire shaping (include_usage today;
                               │                Flex service_tier, src/proxy/flex.rs)
                               └─ forward ──► provider upstream
                                                │
                                                └─ tee response → usage when Wire matches
```

| Traffic | Wire match today | Behaviour |
|---------|------------------|-----------|
| Sync chat (`/v1/messages`, `/v1/chat/completions`, `/v1/responses`, Gemini generate) | yes | record + optional compress/prepare + usage |
| Batch (`/v1/batches`, `/v1/messages/batches`, …) | no (exact path match) | fallback pass-through; call row without usage parsing; observe/result parsing **planned** |
| Flex | same sync wires | `[proxy.lanes.<lane>] flex` sets `service_tier` in `prepare` (`src/proxy/flex.rs`); the tier the response names is stored on `calls.service_tier` and priced at the `<model>@flex` row (T385.12.2) |
| Model routing (D9) | `internal` and opted-in `bulk` calls on the JSON-body wires; never the `agent` lane | `[proxy.routing.<lane>]` swaps `model` before the call is recorded (`src/proxy/route.rs`, T385.10), so `calls`/`usage` name the serving model; the quality gate (I-101) is the operator's bench, named in `gate` |

No transparent sync→Batch conversion: agents need the reply on the same HTTP request.
Config keys `[proxy.batch]`, `[proxy.flex]`, `[proxy.routing]` are documented in
`docs/config.md`.

