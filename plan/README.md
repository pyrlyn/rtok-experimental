# rtok

https://github.com/pyrlyn/rtok

Token-reduction CLI for AI coding agents: hooks, MCP server, API proxy; measured reductions, pluggable methods.

## Tasks

Every task is a folder `plan/<id>/` (`plan/T954/`, `plan/T5.1/`):

- `task.md` — the task. Active tasks open with front matter (`status`, `priority`,
  `complexity`, `readiness`, `agent`), then `# <id>. Title`, why it exists, what done means,
  and exactly one `Check:` line.
- `done.md` — the closing report: what was done, the PR, how it was checked. A folder with
  `done.md` is a closed task; one without it is active.

List the active tasks: `for d in plan/*/; do [ -f "$d/done.md" ] || echo "${d%/}"; done`.
`tests/plan_dir.rs` checks the format. Everything below is reference and findings, not tasks.

## Log triage 2026-10-20

Found by reading rtok's own logs (`~/.rtok/errors.log`, `rtok.log`, the `calls`, `logs`,
`measurements` and `observations` tables of `~/.rtok/rtok.db`) and the agent transcripts of
2026-09-20 to 2026-10-20: Claude Code (`~/.claude/projects`, about 2,050 sessions), Codex
(`~/.codex/sessions`, Codex desktop logs), Cursor (`~/.cursor`, Cursor app logs). Gemini CLI
has no rtok entry, so nothing there. Problems that stopped after a merged fix (sibling-worktree
`path outside cwd`, `invalid line range`, `--idle 0s`, the `cwd /` MCP timeouts) are not
filed. Counts are as of 2026-10-20; the installed binary was v0.17.0-alpha.2 (2026-10-18).

Tasks T920–T942 below come from it.

## Audit 2026-10-19

Three independent review passes over `main` at `cb0b8811`: pass 1 walked the modules, pass 2
followed threats from each entry point (hooks, stdio and HTTP MCP, proxy, dashboard, WASM
plugins, files read from a checkout, CI), pass 3 compared docs, comments and tests with the
code. Nothing was exploited. Overlap: A1 and A5 close residuals that #1064 left open (S4 for
`search`, S3 for logs); nothing repeats #601 or #1061. Open PRs #1166 and #1169 print shorter
archive ids, which makes A11 easier to hit. To take a task, give it a T-id, a row and a `Check:`.

| # | Severity | Finding | Passes | State |
| --- | --- | --- | --- | --- |
| A1 | medium | HTTP `search` printed lines of `.env` and keys that `read` refuses | 2, 3 | fixed |
| A2 | low | Secret list missed `.git-credentials`, `.envrc`, `.pgpass`, keystores | 1 | fixed |
| A3 | medium | Proxy logged reqwest errors with the full URL, Gemini `?key=` included | 1, 2 | fixed |
| A4 | low | `docs fetch` joined a `Cargo.lock` version into a cache path unchecked | 2 | fixed |
| A5 | low | `rtok.log` and `errors.log` were created `0644` | 1, 3 | fixed |
| A6 | low | Log `level`, `source` and `name` could carry a newline and forge a line | 1 | fixed |
| A7 | low | Proxy paths with `..` or `%2e%2e` climbed out of the upstream base path | 2 | fixed |
| A8 | low | Semantic cache ignored `openai-project` in the caller key | 2 | fixed |
| A9 | info | Web handler comments said the Origin check alone guards apply | 3 | fixed |
| A10 | high | A checkout's `.rtok.toml` or `.env` can redirect data and keys or run a command | 2, 3 | planned |
| A11 | medium | HTTP `expand` is always on and resolves any archive by an 8-char prefix | 2, 3 | fixed in T960; [review](T960/review.md) |
| A12 | low | WASM guests have a fuel limit but no memory limit | 1 | planned |
| A13 | low | Log rotation lock sits in the shared temp dir under a predictable name | 1, 2 | fixed in T960; [review](T960/review.md) |
| A14 | low | Existing log files and `~/.rtok` keep their old loose modes | 3 | planned |
| A15 | low | Proxy buffers up to 256 MiB per request with no global cap when lanes are off | 1 | planned |
| A16 | low | `[otel] content = true` by default; a generic `OTEL_EXPORTER_OTLP_ENDPOINT` ships prompts | 3 | planned |

Tests added for A1 to A8: `walk_filter_drops_credential_files_only_when_denied`,
`secret_names_are_recognised`, `upstream_errors_never_carry_the_query_key`,
`resolve_lock_refuses_path_like_names_and_versions`, `a_new_log_file_is_owner_only`,
`no_field_can_start_a_second_line`, `join_upstream_refuses_dot_segments`,
`openai_project_is_part_of_the_caller`.

### A10. Project config can redirect egress and credentials (high)

Where: `crates/rtok-config/src/layers.rs`, `assemble`, `read_dotenv`, `dotenv_pairs`; the
project layer is `<git root>/.rtok.toml`, the dotenv layer reads the nearest `.env` first.
What: both layers accept every key ("same schema", `docs/config.md`). A cloned repository can
set `[otel] endpoint` (with `content = true`, the default), and `Stop`/`SessionEnd` hooks spawn
`rtok otel flush` in that checkout (`src/hooks/mod.rs` → `otel::export::spawn_child`), shipping
prompt and tool text to the repository's server. It can also set
`plugins.memory.embed.url` plus `api_key_env` naming any variable (`src/store/http_embed.rs`
sends it as a bearer token), `tasks.gitlab.url` (the GitLab token follows), `proxy.upstream` /
`openai_upstream` / `gemini_upstream`, `plugins.memory.extract.command` (run by
`rtok memory extract`), `log.path`, `core.archive_dir` and `plugins.read.allow_paths`.
Change: give project layers (the project file and a project `.env`, not `~/.rtok/.env`) a
deny list of key prefixes: `otel.`, `proxy.`, `mcp.`, `web.`, `log.`, `core.archive_dir`,
`core.home`, `tasks.*.url`, `plugins.memory.embed.url`, `plugins.memory.embed.api_key_env`,
`plugins.memory.extract.`, `plugins.compress.*.url`. Drop such keys with one stderr notice
naming the file and key, keep them in `config show --sources` as "ignored (project)". Split
`read_dotenv` so the project file and the home file are separate layers. Optionally add
`rtok trust` (a per-repo opt-in kept in `~/.rtok/trusted.toml`, keyed by canonical root) for
teams that do want to share one of these keys. Update `docs/config.md` layer 3 and 4.
Why: opening an untrusted repository with an agent should not send its prompts or a user's
tokens elsewhere. Same class as the `.env` issue found in stator's audit.

### A11. HTTP `expand` reaches the whole archive (medium)

Resolved in T960 with per-runtime archive grants rather than a first-writer session filter;
see the [implementation and compatibility review](T960/review.md).

Where: `src/mcp.rs` `Server::new` (keeps `expand` listed whatever `tools` says),
`src/mcp/http.rs` `http_server`, `src/expand.rs` `resolve_id` and `fetch_range`,
`crates/rtok-store/src/archive_rows.rs` `archive_ids_with_prefix`.
What: over HTTP a token holder can call `expand` with any 8 hex chars; the prefix is matched
against every archive in the store (all sessions and projects, including bodies archived from
files `read` now refuses), and the ambiguity error lists longer candidates. Nothing limits the
rate of guesses. Change: on the HTTP surface resolve only archives whose `session` is one this
server wrote (filter `archive::session`), never print candidates in the ambiguity error there,
and count failed lookups per connection (stop answering after a small number). Keep stdio as is.
Why: the HTTP surface is for remote callers; it should expose only what it produced.

### A12. WASM memory limit (low)

Where: `src/plugins/wasm.rs` `WasmPlugin::load_with_fuel`, `WasmHost`.
What: fuel stops an endless guest, but `memory.grow` up to 4 GiB is allowed. Change: keep a
`wasmtime::StoreLimits` in `WasmHost` (memory size cap, for example 64 MiB, one instance, one
table) and call `store.limiter`. Why: a bad plugin should fail, not exhaust the hook's memory.

### A13. Rotation lock in the shared temp dir (low)

Resolved in T960 with a sibling lock and best-effort rotation;
see the [implementation and compatibility review](T960/review.md).

Where: `crates/rtok-log/src/lib.rs` `open_rotate_lock`.
What: the lock is `$TMPDIR/rtok-log-rotate-<hash>.lock`, the hash from `DefaultHasher` (fixed
keys) over the log path, so it is predictable. On Linux `/tmp` is shared: another account can
create the file first; `open` then fails, `write_line` returns the error and every line is
dropped once the log reaches `max_bytes`. Change: put the lock beside the log
(`<log dir>/.rotate.lock`, excluded from junk scans) or under the per-user runtime dir, open it
with `O_NOFOLLOW` and mode `0600`; if the lock cannot be opened, append without rotating.
Why: another user must not be able to silence rtok's logs.

### A14. Old loose modes stay (low)

Where: `crates/rtok-store/src/handle.rs` (tightens the db only), `crates/rtok-log/src/lib.rs`,
`src/doctor.rs`. What: A5 and S3 set modes on new files; an existing `~/.rtok` (`0755`),
`archive/` files and logs keep theirs. Change: one `rtok doctor` check that reports and, with
`--fix`, sets `0700` on the home, `archive/` and the log dir and `0600` on their files.
Why: the upgrade path is where most installs are.

### A15. Proxy memory per request (low)

Where: `src/proxy/mod.rs` `MAX_BODY_BYTES`, `handle`. What: each request may buffer 256 MiB,
and with lanes off nothing caps concurrent requests, so a few local clients (or anyone, with a
wide `proxy.bind`) can exhaust memory. Change: a `proxy.max_body_bytes` setting (default far
below 256 MiB) and a global in-flight cap applied even with lanes off. Why: fail one request,
not the proxy.

### A16. Telemetry content on by default (low)

Where: `crates/rtok-config/src/lib.rs` `Otel` (`content: bool = true`) and `Otel::resolve`
(falls back to `OTEL_EXPORTER_OTLP_ENDPOINT`). What: a shell that exports the generic OTLP
variable for another tool makes rtok send prompt and tool text there. Change: default `content`
to false, or send content only when `[otel] endpoint` (not the generic variable) is set; say
so in `docs/otel.md`. Why: content export should be an explicit choice.

## Security audit findings (2026-10-14)

Find-only security review of `main` at `cd9644881` (OWASP Top 10, injection, path handling,
access control, data at rest, CI supply chain). Nothing was fixed or exploited. Line numbers are
as of that commit. Overlap with the closed audit PR #601 (config validation, `filter`, `memory
import`, exit codes): none; these are new. To take one, give it a T-id, a row and a `Check:` line.

| # | Severity | Finding | Where |
| --- | --- | --- | --- |
| S1 | high | `rtok mcp --http` started in `/` or `$HOME` lets a token holder read any file there | `src/plugins/read/mod.rs:212`, `src/mcp/http.rs:110` |
| S2 | medium | Dashboard WebSocket has no authentication; any local user (or LAN host if bound wide) can change config, run `doctor --fix` and clear junk | `src/web/mod.rs:226`, `:259`, `:433` |
| S3 | medium | `~/.rtok` store, archive and logs are world-readable (prompts, tool output, file contents) | `crates/rtok-store/src/handle.rs:18`, `crates/rtok-store/src/lib.rs:444` |
| S4 | medium | `read`/`search` (also over HTTP) serve secrets inside the project: `.env`, `.git/config`, keys | `src/plugins/read/mod.rs:212` |
| S5 | low | Proxy `/live` and `/health` answer without auth; with `proxy.bind = 0.0.0.0` they leak models and paths | `src/proxy/mod.rs:255` |
| S6 | low | `host_authority` takes the last comma-separated `Host` value | `src/web/mod.rs:301` |
| S7 | low | Resident hook socket is chmod-ed after `bind`, a short window at the umask mode | `src/hooks/resident.rs:71` |
| S8 | low | `rtok mcp --http` is plain HTTP; the bearer token crosses the network in clear unless a TLS proxy fronts it | `src/mcp/http.rs:110` |

### S1. HTTP MCP `read` exposes the whole launch directory

The path guard allows everything under the process cwd (`resolve_with`). `walk_root_ok` refuses
`/` and `$HOME` for walks (`search`, `tree`), but a single-file `read` has no such check. A
`rtok mcp --http` started from a home directory (launchd, a service unit, Claude.app's `/`) lets
any holder of the bearer token read `~/.ssh/id_*`, `~/.aws/credentials`, `~/.rtok/rtok.db`.
Impact: remote file disclosure for the HTTP surface (`http_tools` includes `read` by default).
Fix: refuse to start `--http` when cwd is unwalkable unless `allow_paths` names a project, and
apply `walk_root_ok` to `read` roots on the HTTP surface.

### S2. Dashboard WebSocket is unauthenticated

`origin_allowed` returns `true` when no `Origin` header is sent, which covers every non-browser
client. The socket accepts `set` (allow-listed config keys), `doctor` `apply` (writes host
configs), `junk` `apply` (removes files), `expand` (archived prompt and tool bodies), `graph` and
`export`. On a shared machine any local user can reach `127.0.0.1:3333`; with `web.host =
0.0.0.0` any LAN host can. The browser side (CSRF, DNS rebinding) is handled well.
Impact: broken access control, data disclosure and destructive actions as the owner.
Fix: issue a per-run random token (printed in the URL like Jupyter), require it on `/ws`, refuse
non-loopback `web.host` without it.

### S3. Data at rest is world-readable

`create_dir_all` and the SQLite/archive writers use the umask. On the author's Mac: `~/.rtok`
0755, `rtok.db` 0644 (917 MB with `call_io` request/response bodies), `archive/` 0755,
`errors.log` 0644. Other local accounts can read prompts, code and tool output.
Impact: data leak on multi-user hosts and in backups. Fix: create `~/.rtok`, the archive dir and
log dir with 0700 and files with 0600; `rtok doctor` warns on and fixes looser modes.

### S4. No secret-file deny list on `read`

Nothing stops `read` from returning `.env`, `.env.*`, `.git/config` (embedded tokens), `*.pem`,
`id_*`. Over stdio the host's own tools can read them too, but the HTTP surface is meant for
remote callers, and archived copies then land in the world-readable store (S3).
Fix: a default `plugins.read.deny_globs` (overridable), enforced for HTTP always.

### S5. Proxy status routes without auth

`/live` returns method, path, provider and model of recent calls; `/health` returns config
flags. Harmless on loopback, a small leak when `proxy.bind` is widened. Fix: serve them only on
loopback peers, or behind the same token as S2.

### S6. Ambiguous `Host` handling

`host_authority` splits on `,` and keeps the last part. A repeated or comma-joined `Host` is not
a valid request; picking one part can let a crafted request pass the Origin/Host equality.
Fix: reject a `Host` containing `,` (400).

### S7. Socket mode set after bind

`UnixListener::bind` creates the socket with the umask mode and `set_permissions` runs after.
Fix: create it inside a 0700 directory (or set umask around bind).

### S8. Plain-HTTP bearer token

The docs should say that `--http` on a non-loopback address must sit behind TLS; better, refuse
a non-loopback bind without `public_url` using `https`.

### Not run or blocked

`cargo audit` could not load `~/.cargo/advisory-db` (directory present but broken; left as is).
Tests and lints were not re-run (find-only, no code changed). CI workflows were reviewed:
`graph-review-comment.yml` handles untrusted artifacts well (size caps, PR head SHA check).

### Improvement ideas (opinion)

- I-S1. A single "exposure" check in `rtok doctor`: wide binds, missing tokens, file modes.
- I-S2. Pin GitHub Actions to commit SHAs in `release.yml` (currently tags like `@v6`).
- I-S3. Split `~/.rtok/archive` into hashed subdirectories; it holds tens of thousands of files
  in one directory, slow to list and back up.
- I-S4. Retention for `call_io` bodies by default (the store reached 917 MB here).
- I-S5. A `security.md` threat model naming each surface (hooks, stdio MCP, HTTP MCP, proxy,
  dashboard) and who is trusted on it, so new routes get a checklist.

## Cloud review findings (2026-10-08)

New bugs, dead code and moves from a read-only Cursor cloud review of `main` at `a060af58` (agent `bc-e07c8099-22ee-5d00-b1c7-3bd1ad623d52`; full report: `cloud/rtok.md` in the private `listepo/roadmap` repo). They take ids T457–T470, ordered P0, P1, P2, P3. **confirmed** means seen in the tree; **suspected** means plausible from the code but not proven (nothing was run on Windows). Line numbers are as of the review. The report's own labels T436–T449 are roadmap ids, not this plan's. T457, T458 and T459 are done (`done.md`). None of the rest is in the task table yet: to take one, add its row and a card with a `Check:` line.

| ID | Priority | Kind | Status | Where | Fix |
| --- | --- | --- | --- | --- | --- |
| T464 | P2 | move | compared | `crates/rtok-agent-sdk/src/lib.rs:84-163` (`backup`) → crates `file-backup` | Not a drop-in (checked 2026-10-10): rtok writes to a `_backup/` folder, keeps N generations (`prune_backups`, `stale_backups`) and dedups against any file there; `file-backup` 0.1.1 writes beside the file and never prunes. The creator chose to extend `file-backup` first (crates roadmap T12, with its T4 symlink fix, which the open crates PR #34 also touches), then depend on it here. |
| T465 | P2 | move | compared | `tools/dist-generate.sh`, `tools/release.sh` → `pyrlyn/ci` | Not shared copies (diffed 2026-10-10): `dist-generate.sh` differs from ketch's and swarfr's in 239 and 102 lines; `release.sh` differs from ketch, cox, runa and swarfr in 130, 121, 63 and 75 lines. Nothing to fold until the repos agree on one release flow; no move without a new creator OK. |
| T466 | P2 | move | compared | `.github/actions/rustup-toolchain-cache` → `pyrlyn/ci/.github/actions/` | Only rtok uses it (`ci.yml`, `graph-review.yml`; checked 2026-10-10 across `apps/` and `packages/`). `pyrlyn/ci` already ships `setup-rust` (mise-pinned Rust and `rust-cache`); sharing would mean folding the toolchain cache into it. No move without a new creator OK. |
| T470 | P3 | move | suspected | `tools/test-changed.sh`, `tools/selective-check.sh` → `scoped-check` / `pyrlyn/ci` | Overlap was claimed but not diffed against cox or `pyrlyn/ci`. Compare first; move only what matches. |

Already tracked here, not added again: `src/render.rs` → `change-preview` is T416.1; per-host MCP code → `crates/rtok-mcp` and the unused `rtok_mcp::ops::apply` are T277; `OPERATION_ICONS` → a shared icon crate is T436.3; the test-only `VersionFile::write`/`::new` and the stale `#[allow(dead_code)]` on `read_installed` were fixed on T279 (2026-10-21: all three are used by `agents update`).









## Reference

Historical phase notes (P0–P39) live in `done.md`. Companion evidence: `research.md`, `architecture.md`. Per-plugin plan: `roadmap.md`. Unapproved propositions: `ideas.md`.

Claim a `todo` row before work: set Status to `in progress` and Agent to `Provider / model`. Before stopping unfinished work, set Status to `todo` and clear Agent. When the Check passes, move the task entirely to `done.md` (Do/Check + Check result) and drop it from this table, its card, and `todo.md`.

### Decisions (read before any task)

| # | Decision | Why (evidence in `research.md`) |
| --- | --- | --- |
| D1 | **Rust, one static binary, in-tree plugins behind one trait + Cargo features.** No WASM, no subprocess plugins, no daemon **on the hook path in v0.1** (`rtok demon` supervises the long-running surfaces only, D22). A later WASM plugin host is P32 (landed); this repo still does not wrap third-party tools (D6). | Hooks run on every tool call; Rust cold start vs Python hooks. |
| D2 | **One binary, three surfaces:** `rtok hook` (Claude Code hooks), `rtok mcp` (MCP server), `rtok proxy` (`ANTHROPIC_BASE_URL`). | PostToolUse hooks cannot modify tool results. Only PreToolUse rewrite, MCP tool replacement, or a proxy can shrink what the model sees. |
| D3 | **Measurement first.** Nothing ships until `rtok stats` reads real usage from session logs and the proxy. Every plugin logs before/after. Metric = *context-token-turns* (tokens × turns they stay in context), plus output tokens. | Vendor claims vs measured savings; nobody in the stack measures end to end. |
| D4 | **Lossless by default.** Every compression keeps the original retrievable via `rtok expand <id>` / MCP `expand`. Lossy only where the source is regenerable (re-run the command). | Trust is the product. |
| D5 | **Injection budget.** All SessionStart/UserPromptSubmit injections go through one plugin with a per-turn token cap (default 800) and a byte-stable prefix. | Injections are re-read (cached) every turn. |
| D6 | **Every plugin is native, written from scratch in this repo. No third-party plugins.** A plugin never spawns, links, imports, or reads the data of another tool. Third parties extend rtok from outside through the public plugin API (`rtok-plugin-sdk`, `Registry::from_plugins`, `docs/plugin-authoring.md`), never through this repo. | One code path per method is what `Measurement` can attribute. |
| D7 | **Prompt "modes" (terse, YAGNI) are data files, not code.** | Measured effect must be A/B tested, not assumed. |
| D8 | **One SQLite file** (`~/.rtok/rtok.db`, WAL). Schema is D13. Raw archived payloads on disk under `~/.rtok/archive/` (D4); the DB holds indexes and inline JSON under a size cap. | Field tools converge on SQLite (+FTS5). |
| D9 | **Agents are provider-agnostic.** Route by job: low-cost for mechanical work; mid-tier for coding; high-performance for research only after the user confirms. Host names are products, not the implementer. | User constraint: cost-aware routing, any provider. |
| D10 | **Retire, don't stack.** Phase 9 replaces the legacy hook pile with ≤ 8 and drops every tool the A/B bench cannot justify. | Duplicated responsibilities across bash/read/memory/graph tools. |
| D11 | **The proxy speaks both wire formats.** Anthropic Messages and OpenAI Chat Completions / Responses are `Wire` adapters behind one proxy. Hosts point `ANTHROPIC_BASE_URL` or `OPENAI_BASE_URL` at rtok. | One proxy, two parsers is cheaper than two proxies. |
| D12 | **One config file holds every setting; every CLI flag is a config key.** Precedence: defaults < user file < `<git root>/.rtok.toml` < `RTOK_<SECTION>_<KEY>` env < flags. `rtok config show --sources` tells where each value came from. | Hooks, long-running servers, and benches need one precedence rule. |
| D13 | **Core persists through a sync ORM on bundled SQLite.** Diesel (`sqlite` + bundled `libsqlite3-sys` with FTS5). Plugins never write SQL; `Store` is the only DB owner. Hook path: metadata always, body only if under the inline cap — never archive, never fail the hook (D1). | Diesel is sync, so the ≤ 10 ms hook path stays blocking and fail-open. |
| D14 | **CLI is clap 4 (derive); config layers are figment; TOML writes are toml_edit.** Env is `RTOK_<SECTION>_<KEY>` looked up in a leaf table from `Config::default()`. | Clap owns the subcommand tree; Figment tracks per-key provenance. |
| D15 | **Every plugin is designed against alternatives before it is built.** Each catalogue plugin has `src/plugins/<id>/PLAN.md`. | From-scratch only pays if the design beats what it retires. |
| D16 | **One task = one PR.** Each task gets its own branch (or worktree) off `origin/main` and lands through its own pull request; never commit to `main` directly. The PR carries the `<task-id>: <title>` commit and the `plan.md` → `done.md` move. Delete the branch after merge. A task that needs several PRs is split into subtasks `Tn.m`, one PR each; the parent card stays as the epic and moves to `done.md` with its last subtask. | Every change passes CI before it reaches `main`; concurrent agents stop colliding in one checkout. |
| D18 | **The graph index lives in SQLite with the ledgers (D8).** LadybugDB and Grafeo were gated, frozen, then removed (P39). No live `lbug` / `graph-lbug` / `symbols_lbug.rs` / `grafeo` feature flags. SQL for symbols lives only in `src/store/symbols.rs`. D6 holds: no spawned graph tool. | Both graph-store candidates were priced and deleted per the gate. Survey archive: `src/plugins/graph/PLAN.md`. |
| D19 | **Observability is a projection of the ledgers, never a second recorder.** OpenTelemetry export reads existing rows and posts OTLP/HTTP JSON. Nothing runs on the hook path. | Delivery is at-least-once behind a per-stream watermark. |
| D20 | **Local web UI is an operator surface, not a catalogue plugin.** `rtok web` serves axum + the embedded React SPA (`web/`, T310). The SPA is a static bundle, not linked code, and `rtok hook` never loads it. | Pulling a UI stack into `rtok hook` would fail the size/latency gate. |
| D21 | **Every new plugin is plugin and MCP as one unit, a singleton, with one call path per capability.** Host plugins load in that host's desktop app and its CLI. Missing `rtok`: fail open and print that it must be installed with ketch. | Duplicate MCP processes and duplicate call paths break D18 and measurement. |
| D22 | **`rtok demon` supervises rtok's own long-running surfaces**, not a catalogue plugin. Allow-list names (`proxy`, `mcp`, `dashboard`). Nothing in it is on the hook path. | The proxy is the wire hop; when it dies every host silently loses it. |
| D23 | **`rtok tui` and `rtok web` are two renderings of one operator model.** A page that exists on one surface and not the other is a defect. | Two independently built surfaces drift. |
| D24 | **`rtok report` renders; it never computes a number of its own.** It reads the D23 operator model. Recommendations are rules over those rows, never an LLM call. | A saving that is not a `Measurement` row does not exist (D3). |
| D25 | **The plugin contract is `rtok-plugin-sdk`.** Required methods are explicit; event methods keep no-op defaults. `rtok` is the only dispatcher. | One contract, no in-tree shortcut. |
| D26 | **One log with two readers:** a rotating text file and the `logs` table OTel exports. Bounded (`max_bytes` 1 MiB, `files` 5). | An unbounded log on a long-running proxy is a disk-full bug. |
| D27 | **Anything a command prints, or the store keeps, is a page on `rtok web` and `rtok tui`.** Writing actions may appear on both, each calling the same function as its CLI command with the same guards (dry-run plan, then confirm); every write action on one UI surface needs its counterpart on the other (D23 applies to writes too; creator decision 2026-10-10, T346). | The two surfaces plus CLI must not disagree about what a session is, or about what a write does. |
| D28 | **The agent-host contract is `rtok-agent-sdk`.** Installers go through it; host-specific code stays in `src/setup/<host>.rs`. | One write cycle, one plugin-offer body. |
| D29 | **Unit tests prefer a virtual filesystem (`testutil::Vfs`) over host TempDir/std::fs.** Pure path/content/size logic must not require real disk; Windows/macOS quirks are simulated in Vfs. Migrate hottest suites first (read/search/cmd/setup) as T56.x — not a big-bang rewrite of e2e. | Hermetic tests; reproducible CI; path-case and spaced-path bugs (T55) need a simulated FS. |
| D30 | **HTTPS uses webpki Mozilla roots (`use_preconfigured_tls`); one binary.** Corporate CAs via `SSL_CERT_FILE` (curl parity, fail closed). reqwest 0.13 `rustls` still links `rustls-platform-verifier`; `otool` showed Security.framework still present (T53.3). A second hook binary was rejected. | I-32: 1.3–1.5 ms dyld; dropping the `rustls` feature does not compile. |
| D31 | **Claude Code's `WorktreeCreate`/`WorktreeRemove` hooks route through `rtok worktree` and are exempt from the 10 ms rule (T159).** The 10 ms / unmodified-input budget (D1) binds the per-tool-call hot path. These two events fire once per worktree, replace the host's own create/remove, and must spawn git (a fetch and `git worktree add`). Fail open means the host never loses the ability to work: on any rtok error `WorktreeCreate` still prints a path, made by plain git at the host's own default (`<repo>/.claude/worktrees/<name>`, branch `worktree-<name>`); `WorktreeRemove` deletes nothing on error, never forces, and exits non-zero so the host keeps the directory and shows the reason (the host counts exit 0 as removed). Plugin only: `scripts/worktree.sh` carries the fallback, so `settings.json` never gets these entries. | The host's contract (`research.md` §18.3, re-checked 2026-10-02): a create hook that prints no path fails the session, so "print nothing, exit 0" is not available here. |
| D32 | **An optional resident hook process (T178).** `rtok hook --serve` answers `rtok-hook`, a std-only client, over a Unix socket (Windows: a named pipe); `rtok demon` supervises it as the service `hook`, or the hook starts it detached, rate-limited by a lock file. This supersedes D1's "no daemon on the hook path" and D22's "nothing in it is on the hook path" for the `hook` service only. Without it everything works as today: the client runs `rtok hook` when the resident is absent or refuses (another version or config environment), and prints `{}` when it does not answer within 50 ms. | Process start is ~11 ms of the ~14 ms Claude Code waits per hook (research.md §19); a fresh process cannot meet the 10 ms budget. |
| D33 | **rtok's MCP lives in each agent's own config, not in its plugins (T275, amends D21 for MCP).** Install and update always write the config entry `rtok`; only `remove` takes it out, and a plugin no longer suppresses or strips it. Where an agent would show a plugin server next to the config entry (Claude Code and Desktop, Cursor, Copilot, Codex, VS Code, ZCode, Kimi, Grok; `research.md` §25), the rtok plugin ships no MCP server and keeps its hooks, skills and agents. Gemini keeps both, since settings.json wins over an extension's same-name server. Same-name entries across one agent's files are left to the agent to merge. Hooks keep D21 unchanged. `rtok doctor --fix` never removes the `rtok` config entry (T332). |
| D34 | **rtok gives every agent session its own id and owns its worktrees the same way on every host (T281–T290, creator request 2026-09-27).** The agent id is a random UUIDv4 issued by rtok per host session (sub-agents get their own, with a parent), shown as its first 8 hex chars; any unique prefix of 4+ chars is accepted. Not UUIDv7: its leading hex is a timestamp, so agents started within the same minute would share the short id (found 2026-09-27; `started_at` keeps the order). The host's session id is kept alongside but never used as the identity: it collides across hosts and is missing on several (`research.md` §26). A worktree is bound to one agent by the git lock reason `<owner> \| <task-id> \| <date> \| agent <uuid>` (the old 3-field form stays valid) and a store row; the lock is the source of truth. Every host gets the same root, naming, lock, list, remove and gc: Claude Code redirects its own worktrees through `WorktreeCreate`/`WorktreeRemove` (T159), hosts with a post-create script adopt theirs (T289), all others use the skill and the MCP tools. Messages between agents and from the user are local, capped, framed as information from another agent and never as instructions. |
| D35 | **Smart memory is on by default for every agent, from one event module (T291–T293, creator 2026-09-27).** `prompt_recall` (5), `startup_recall`, `handoff` and `spawn_brief` default on. This overrides the T131 gate that would have left `spawn_brief` off until a measured net saving. Bodies stay out of the always-on prompt; the index names `mem_get`. One module under `src/agents/` generates every host manifest from that host's event-name map; a host with no equivalent event is MCP-only for it. Hook path stays sync, ≤10 ms, fail-open, no LLM (D13). No vector read while `[plugins.memory.embed] enabled` is false (the default). T454: when `enabled` and `hybrid` are both true, `prompt_recall` reads vectors already stored (`search_notes_hybrid_stored`) and does not call `embed_stale`. SQLite stays the only store (D8). A later private-repo sync is outside this repo; T294 only makes an export row able to carry a tombstone. Evidence: `research.md` §27. T390 shipped the table (`src/agents/hook_events.rs`) and a manifest drift test in place of generating the manifests. |
| D36 | **Agent junk deletes only what has evidence; history and credentials are never cleared by default (T338, T339).** `rtok agents junk clear` deletes a path only when `research.md` §22 documents it (official docs or source), when it carries a valid `CACHEDIR.TAG`, or when the user names it in `[agents.junk] extra`; platform cache roots and Electron subfolders without a §22 row are listed read-only with their size, never cleared. Credentials and token files are never read for expiry or deleted. Session history and snapshots are `never` for default and `--include review` runs; `--kind sessions` removes a whole session unit only on hosts whose §22 row documents it and its index; `stale_session_days` defaults to 30. Creator approved (C for T338, C for T339, 30 days) on 2026-10-03. | `research.md` §22.1, §22.2: hosts refresh tokens and prune their own sessions (Claude Code, Gemini: 30 days), sessions share trees with memory and indexes, and undocumented paths (Cursor) sit next to chat history. |

### Architecture

```
                    ┌──────────────── rtok (one binary) ────────────────┐
 Claude Code ──hook─┤ hook <event>  → EventBus → plugins (Pre/Post/...) │
 Cursor/OpenCode ───┤ mcp           → tools: read search tree run expand │
                    │                 mem_save mem_search symbol callers │
 ANTHROPIC_BASE_URL ┤ proxy         → usage capture, live-zone rewrite   │──▶ api.anthropic.com
                    │ stats | bench | doctor | setup | run | expand      │
                    └──────────┬─────────────────────┬──────────────────┘
                         ~/.rtok/rtok.db        ~/.rtok/archive/
```

`Ctx` gives every plugin: the DB handle, the token estimator, the archive store, config, session id. `Measurement { plugin, kind, before_bytes, after_bytes, est_before, est_after, ref_id }` is the only way savings enter the DB.

Plugin catalogue (v0.1). Every plugin is native Rust written from scratch here (D6).

| id | spec (replaces; evidence in research.md) | surface | mechanism |
| --- | --- | --- | --- |
| `measure` | rtk gain, headroom savings, lean-ctx gain | `stats`, `bench`, proxy | session JSONL ingest + proxy `usage`; context-token-turns |
| `cmd` | rtk hook, lean-ctx ctx_shell | PreToolUse(Bash) → `rtok run` | archive raw output; per-family formatters + TOML rules; pointer trailer |
| `read` | lean-ctx ctx_read/search/tree | MCP `read`,`search`,`tree` + PreToolUse(Read) | modes via tree-sitter-tags; re-read dedup |
| `archive` | token-optimizer archive_result, headroom CCR | proxy live zone + `expand` | replace old large `tool_result` with pointer + head/tail |
| `proxy` | headroom proxy | `ANTHROPIC_BASE_URL`, `OPENAI_BASE_URL` | passthrough + SSE; usage capture; never touches system, tools, or last 2 turns |
| `inject` | caveman shrink-hook, ponytail/caveman modes | SessionStart, UserPromptSubmit | budgeted, byte-stable prefix; modes as markdown |
| `guard` | token-optimizer refetch_guard | PreToolUse | identical read/command within N turns → deny |
| `memory` | engram, claude-mem | MCP `mem_save/search/get` | agent-written notes, SQLite FTS5 |
| `graph` | codebase-memory-mcp, serena, codegraph | MCP `symbol`,`callers`,`outline` | tree-sitter-tags index in SQLite; optional LSP is T30.2 |
| `toon` (on by default) | caveman toon, TOON | proxy/MCP | tabular JSON → TOON |

### Working agreement

- Graph backends: do not claim tasks that reintroduce `lbug` / `graph-lbug` / `symbols_lbug.rs` / `grafeo` / `graph-grafeo` / cmake-for-liblbug. Symbol index is SQLite only.
- One task = one branch = one PR (D16). Never skip the Check.
- Read `research.md` §3 (hook contract) before any hook task. Hook input is JSON on stdin; output is JSON on stdout; exit 0. Exit 2 blocks (PreToolUse only). PostToolUse can only add context.
- Fail open: any plugin error → log to DB and return the unmodified input/empty output. A hook that crashes must still exit 0 in ≤ 10 ms.
- No new dependency without a one-line justification in the commit message.
- Don't duplicate code or logic: find the existing helper and reuse it, or extract one shared helper at the responsible layer.
- Code style: `cargo fmt`, `cargo clippy -D warnings`, `cargo nextest run` green before every Check.
- Anything unmeasurable is a bug in the plan: add a `Measurement` before adding a feature.
- Every new CLI flag gets a key in `config/default.toml` and a row in `docs/config.md` in the same commit (D12).
- No plugin shells out to, links, imports from, or reads the data of a third-party tool (D6).
- Every new plugin obeys D21.
---

## Review 2026-09-17 — bug hunt (post #37–#47)

Scope: `origin/main` after cross-platform agent fixes #37–#47. Local WIP from other agents was stashed (`preserve-other-agents-wip-before-docs-review-bugs-plan`) and not reviewed. No code fixes in this pass — findings tracked as T55.x.

### Blockers

None for the macOS/Linux happy path on current main. Windows correctness gaps below are P1.

### Should fix

1. ~~**T55.1**~~ — done in #49 (`display_rel` case-insensitive strip).
2. ~~**T55.2**~~ — done in #49 (`never_wrap` case-insensitive stem).
3. ~~**T55.3**~~ — done in #49 (`file_uri` percent-encoding).
4. ~~**T55.4**~~ — done in #49 (host-shell-safe wrap / cmd quoting).
5. ~~**T55.5**~~ — done in #49 (`search_max_bytes`).
6. ~~**T55.6**~~ — done in #49 (`call` in mcp.cmd).

### Nits

1. ~~**T55.7**~~ — done (`skip_word`; quoted `cd` paths bucket by family).
2. ~~**`expand::parse_range` when start > line count**~~ — done 2026-09-23: a start past the last line errors (`start exceeds line count N`) instead of printing nothing with exit 0.
3. **`guard::strip_wrap`** — updated in #49 for PowerShell `''`.
4. **T55.8 / T55.9 / T55.10** — filed from the T55.7 code read: guard `read:` keys survive a mutating Bash, guard Bash key cwd-blind, three copies of `cmd_stem`.

### Residual still open (called out before)

- T55.1–T55.6 closed by #49.
- PATH / single-quote parsers — rtok: T55.7 closed; larger residual in ketch.
- Guard false denies — T55.8 (P2), T55.9; flag-aware read-only classes promoted from I-38 as T57.1.
- Test VFS migration — D29 / T56.x (`Vfs` helper in #49).
- T48.3 still todo: Cursor plugin mcp.json still bare `rtok mcp`.

### Second pass (2026-09-17, later the same day)

Scope: hook dispatcher/types, guard, cmd (hook/run/rules/formatters), read (mod/cache/hook/search), archive, proxy (mod/wire/anthropic/openai_chat/semantic_cache), expand, store (archive/decisions/read_cache paths), mcp session handling. Four findings reproduced with a scratch integration test (written, run, deleted — `cargo nextest run --test zz_review_repro` → 4 failed exactly as predicted); two filed from code read. No code fixes in this pass — findings tracked as T55.11–T55.16, propositions as I-39/I-40.

- ~~**T55.11 (P2)**~~ — done: `expand` never froze the owning session's pointer (every expand caller runs under session `expand` / `mcp-<pid>`, decisions belong to the proxy session; expand rate stayed 0, toon attribution dead). Reproduced.
- **T55.12 (P2)** — Windows `wrap_quote` `''` quoting is wrong under Git Bash (Claude Code's Windows shell): apostrophes silently dropped from the rewritten command. Code read.
- ~~**T55.13 (P3)**~~ — done: Copilot `Read` inputs use `path`; `guard::cache_key` and `read::hook` match `file_path` only → dedup and advice never fire on that host. Reproduced.
- ~~**T55.14 (P3)**~~ — done: the semantic-cache key ignored tool_result text; two requests differing only in tool results hashed identically and were both eligible under defaults. Reproduced.
- **T55.15 (P3)** — `live_blobs` overwrites image/document `source.data` / `image_url.url` with pointer text → invalid request when the flag is on. Reproduced.
- ~~**T55.16 (P3)**~~ — done: the guard deny read the full archived body (and estimated over it) on the ≤ 10 ms PreToolUse path. Code read.

### Out of scope this pass

Concurrent agent WIP on local `main` (stashed as `preserve-other-agents-wip-before-docs-review-bugs-plan`). Half-finished T48–T53 cards on that WIP were not judged as shipped bugs.

---

## Note 2026-09-17 — testing library candidates

Shared catalog: [`listepo/rust.md`](../../rust.md) → *Testing candidates*.
Catalog only — no blanket `Cargo.toml` adds (Working agreement: justify each dep).

Fits for rtok (1–3):

1. `vfs` (crates.io) — evaluate against in-house T56 `src/testutil.rs` `Vfs`
   before adopting; mandate stays "prefer VFS over host TempDir".
2. `mockall` — trait mocks for provider / plugin host seams when hand fakes
   get noisy (`httpmock` stays for HTTP).
3. `tokio-test` — async unit helpers beyond `#[tokio::test]` where time/task
   control matters.

Already covered: `assert_cmd`, `divan`, `httpmock`, `insta`, `rstest`,
`trycmd`, `similar`. Skip `test-case` / `expect-test` / `mockito` duplicates;
`testcontainers` / `bolero`/`honggfuzz` only if a measured e2e/fuzz gap appears.

---

## Note 2026-10-02 — Empryo-derived tasks (T368–T378)

Source: study of [proxysoul/Empryo](https://github.com/proxysoul/Empryo) (formerly SoulForge) at `669ff91`. **Idea-only, clean-room: Empryo is BSL 1.1, no code copied.** Every card cites Empryo only for the idea; implementations are written from the card.

Take first, in order: T369, T372, T373, T374. Then T375, T377, T378 (gated on I-95). T368, T370, T371 and T376 are done.

The 14 portable ideas and where each landed:

1. PageRank repo map — T370 (done; extends T52.3 / I-28).
2. Edge confidence / name IDF — T368.
3. Git co-change — T371 (done).
4. Blast radius under a budget — T377.
5. Trigram index — T378 (gated on I-95).
6. Clone detection (MinHash) — not now: T52.4 / I-29 (dead code) has no measured question that duplicates answer, and Empryo's own perf PR #220 shows the cold-scan cost; revisit after T68.9.
7. Grep → symbol intercept — T369 (builds on I-08 / T50.4).
8. Post-edit diagnostics delta — does not fit: rtok does not run edits; it is the agent loop's job (cox).
9. Backend fallback / LSP hygiene — T376.
10. Compound tools (rename, project check) — does not fit: they need the agent's edit loop and approvals; rtok stays read/measure.
11. Deterministic compaction state — T375 (checkpoint); LLM compaction itself is the host's.
12. Memory RRF / file affinity — T373, T374.
13. Edit robustness (fuzzy `old_string`) — does not fit now: I-43 / T58.3 measured 1.3 % `old_string` misses; below the gate.
14. Shell output compress / tee — already covered by the `cmd` rules (`src/plugins/cmd/rules.rs`) and the archive (`expand <id>`).

### T644. Tests remove the temp dirs they create

The tests leak their temp dirs into `$TMPDIR`: on the creator's Mac about 24,000 `rtok-mem-*` (`Runtime::in_memory` in `src/plugin.rs` creates one per call and never removes it), 6,300 `rtok-tui-*`, 2,000 `rtok-guard-*`, 1,300 `rtok-usage-*` and more. A directory that size makes `realpath` in it slow under load (T643). Each test helper that makes a temp dir removes it (a guard or `tempfile::TempDir`), `Runtime::in_memory` first.

Check: a test shows a dropped `Runtime::in_memory` leaves no directory behind, and a full `just test` run adds no `rtok-*` entry to `$TMPDIR`.

## Beat sqz, lean-ctx, token-optimizer and caveman per category (T672–T686)

Source: `docs/research/competitors-techniques.md` (techniques, licenses, links). Every card: keep 100 % of hashes, paths and WARN/ERROR lines, never more tokens than raw, stderr and exit line kept on failure.

T905 (idea I-133): `podman ps` uses the `docker ps` formatter. Done (`done.md`).

T909: archive and compress pointers print the 8-char archive id. Done (`done.md`).

T910: hook, skill, handoff, MCP wrap and expand omitted markers print the 8-char archive id. Done (`done.md`).

T911: `stats` counts a re-expand of the 8-char archive id pointers print. Done (`done.md`).

T914: `toon` and `json_tree` pointers print the 8-char archive id. Done (`done.md`).
T913: the report Expand list prints the 8-char archive id. Done (`done.md`).
T912: guides and `rtok config init` name the 8-char trailer id. Done (`done.md`).

## Completed work without a task id

Moved from `done.md`; these were recorded without an id.

### Web UI: UI kit, viewport-fill, std-widgets controls (creator request 2026-09-22)

No T-number — a direct creator request rather than a `plan.md` card, recorded here retroactively per the `done.md` convention. Creator request 2026-09-22, in three steps as the work proceeded: the Slint WASM UI displayed badly — it did not fill the browser viewport and icons overflowed their nav buttons; then "inherit and reuse std-widgets where possible"; then "create a UI kit and use it without duplicating code and styles".

Do (Command Code / claude-fable-5, 2026-09-22): `crates/rtok-webui/ui/kit.slint` (new) holds the shared look — `Card`, `TitleText`, `SectionText`, `MetaText`, `BodyText`, `PageIcon`, `NavItem`, `ListRow` — with tokens from `theme.slint` (new `title-size`/`body-size`). `app.slint` went from ~1161 to ~830 lines and only composes (7 card panels, 6 list rows, 2 theme buttons, every text style dedup'd; one `default-font-family` remains). std-widgets reused with inheritance: `PluginEnableToggle` inherits `CheckBox`, the skills filter is a `CheckBox`, and expand is a `Button` (Slint 1.18's `CheckBox.toggled` takes no arguments — handlers use `self.checked`). Viewport fill: the wasm window sizes to `innerWidth`/`innerHeight` and tracks `resize` (`web-sys` +`EventTarget`); before this it stayed at the preferred 960×640. Icons pinned at 16px with min/preferred/max constraints (layouts ignore plain `width`); `MetricCell` label/value hug (`alignment: center`); the stats card hugs its rows (`if`-only children reported zero preferred height — bound `preferred-height` to a wrapper layout); equal-width icon buttons in the mobile bar; `Palette.color-scheme` set imperatively in `init`/`changed dark` (declarative `Global.prop:` bindings are a parse error in Slint 1.18) themes the native widgets. Pre-existing fixes: a missing brace in `tests/e2e.rs::snapshot_error_reaches_the_banner` (the webui test suite did not compile at HEAD) and `filter_expand` cfg-gated (dead_code on native).

Check result (2026-09-22): rtok-webui `cargo test` (SLINT_EMIT_DEBUG_INFO=1) 15/15 with all e2e label contracts intact (tabs, `toggle cmd`, `never invoked only`, `session a`, `skill hot`/`skill cold`); `cargo test --test surface_parity` 9/9; `just webui-check`, webui clippy `-D warnings`, `just dup`, `slint-viewer --check` green; pixel-reviewed renders at 375/390/820/834/960/1600/2560 (dark + light + error banner + all pages) — no clipping/overlap introduced; real-browser verification (Chrome, agent-browser): canvas rect == viewport exactly at 1440×900 / 375×667 / 2560×1440, empty console; wasm bundle 4,254,541 B vs the 4,500,000 B gate (`tests/web_wasm.rs`). Shipped as PR #187 (branch `webui-ui-kit`, commits `4c63936`, `6a38aa2`) with mobile/tablet/desktop screenshots in `site/static/images/`.

Deviations: (1) a pre-existing nit kept open — at 375px the detail viewport clips long lines at the horizontal scroll edge (content laid out at preferred width; same pixels before this work). (2) The `skills` nav icon temporarily reused `icons/doctor.svg` (no `skills.svg` existed) — resolved in the same-day `app.slint`/`kit.slint` rework, which adds `icons/skills.svg` and wires it into `kit.slint`'s `PageIcon` for `page-id == "skills"`. (3) No `plan.md` card — direct creator request; recorded here retroactively per the `done.md` convention.

Status: done 2026-09-22
Model: Command Code / claude-fable-5
