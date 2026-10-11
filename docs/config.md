# Configuration

One file, `~/.rtok/config.toml`, holds every setting rtok has. **Every CLI flag is a config
key**, so anything you can pass on the command line you can also make permanent, and
`rtok config show --sources` always tells you where a value came from.

Status: T12.1–T12.2 are done — every table below is a typed section in `crates/rtok-config/src/lib.rs` (unknown
key = error), `crates/rtok-config/default.toml` is embedded and written by `rtok config init` with each
assignment commented out, so an untouched key follows the current default;
`core.inject_budget_tokens` has moved to `plugins.inject.budget_tokens`, and layering
(user < project < env < flags, `figment`-based) plus `config show [--sources] [--json]` and
`config get <key>` are live. `config set`/`validate` and the flag-coverage test land in T12.3–T12.4.

## Precedence

Lowest to highest. Later layers override earlier ones key by key.

1. **Built-in defaults** — the values in the reference file below (`crates/rtok-config/default.toml`,
   embedded in the binary).
2. **User file** — `~/.rtok/config.toml` (directory overridable with `RTOK_HOME`; file
   overridable with `RTOK_CONFIG=<path>` or `--config <path>`).
3. **Project file** — `<git root>/.rtok.toml`, if present. Same schema; typically only
   `[plugins.read] allow_paths`, `[plugins.cmd] rules`, `[plugins.inject] modes`.
4. **`.env` files** — `RTOK_*` lines (same names as the environment layer) from the nearest
   `.env` at or above the working directory, then `~/.rtok/.env`; the project file wins.
   Parsed only, never exported: commands run through `rtok run` do not inherit them, and other
   keys in a project's `.env` are ignored. A malformed file is reported on stderr and skipped.
5. **Environment** — `RTOK_<SECTION>_<KEY>` in upper snake case, e.g. `RTOK_PROXY_PORT=8791`,
   `RTOK_PLUGINS_CMD_REWRITE=false`, `RTOK_STATS_SINCE=7d`. Lists are comma-separated.
   Existing short names stay as aliases: `RTOK_UPSTREAM`, `RTOK_OPENAI_UPSTREAM`.
6. **Command-line flags** — `rtok proxy --port 8791`.

Rules:

- Positional per-call arguments (`hook <event>`, `expand <id>`, `run -- <cmd>`,
  `setup <host>`, `memory import <file>`, `--save-baseline <name>`) are not settings and have no key. Everything
  else does.
- Flag `--foo-bar` on subcommand `baz` ↔ key `baz.foo_bar`. Plugin settings live under
  `plugins.<id>.<key>`.
- Unknown keys are an error in `rtok config validate` and a warning (stderr, once) elsewhere.
  Hooks never fail on config problems: they log and use defaults (fail open).
- Paths accept `~`. Relative paths are relative to the file they appear in.
- Durations: `30d`, `12h`, `15m`. Sizes: plain integers in bytes or tokens as named.

## `rtok config`

| Command | Does |
|---------|------|
| `rtok config show [--sources] [--json]` | effective config after all layers; `--sources` annotates each key with `default / user / project / env / flag` |
| `rtok config init [--force]` | write the reference file (below) to `~/.rtok/config.toml`; never overwrites without `--force` |
| `rtok config validate [path]` | parse, reject unknown keys and out-of-range values with line numbers; exit 1 on error |
| `rtok config path` | print the resolved user file path (and project file if any) |
| `rtok config get <key>` | print one effective value, e.g. `rtok config get proxy.port` |
| `rtok config set <key> <value>` | edit the user file in place, preserving comments (uses `toml_edit`) |

Credentials never print: `otel.headers` (it carries OTLP ingestion keys) and `mcp.token` (the
`rtok mcp --http` bearer token) show as `<redacted>` once set, in `show`, `get`, `set` and `rtok report` — its source still shows. Read the file itself
to see the value.

## Reference file

This is the reference with assignments shown uncommented, so each value below is the
default. `rtok config init` writes the same text with those assignments commented out;
uncomment a line to pin it. Logging is `[log]` only (D26). An old file's
`core.log_file` / `log_level` / `log_to_db` still loads once with a warning and folds into
`[log]`; `rtok config validate` rejects those keys because they are absent from the reference
schema.

```toml
# rtok configuration. Every CLI flag has a key here; flags and RTOK_* env vars override.
# Precedence: defaults < this file < <git root>/.rtok.toml < env < flags.
# Docs: docs/config.md. Check: `rtok config validate`. Where a value came from: `rtok config show --sources`.

[core]
enabled     = true                    # false = plain proxy (no business logic); HTTP stays up until process exit
db_path     = "~/.rtok/rtok.db"       # one SQLite file, WAL (decision D8)
archive_dir = "~/.rtok/archive"       # raw payloads for `rtok expand <id>` (decision D4)
session_env = "CLAUDE_SESSION_ID"     # env var consulted for the session id when stdin has none
call_io_inline_bytes = 65536          # MCP/API bodies larger than this go to archive (hooks never archive)
hook_max_input_bytes = 8388608        # rtok hook <event> stdin cap (8 MiB); over it, exits 0 unmodified, no archiving or hashing (T201)
retain_calls_days    = 30             # 0 = keep `calls` forever
retain_hook_bodies_days = 3           # hook stdin bodies cleared after N days, rows kept; 0 = as long as `calls` (T352)
store_raw = false                     # true = save request bodies verbatim; false strips escapes, control chars, harness wrappers, trailing blanks (T431)

[log]                                 # rtok's own log (D26); `rtok logs` reads it
path      = "~/.rtok/logs/rtok.log"   # rotated siblings live beside it: rtok.log.1 … .5
max_bytes = 1048576                   # rotate past 1 MiB (validate: ≥ 1024)
files     = 5                         # generations kept; older ones are deleted, never archived (validate: ≤ 20)
lines     = 200                       # what `rtok logs` prints when --lines is not given
level     = "info"                    # error | warn | info | debug
to_db     = true                      # also write a `logs` row for `rtok otel`
tspin     = "auto"                    # `rtok logs` through tailspin: auto = terminal and tspin on PATH | always | off (T225.1)

[estimator]                           # chars per token per class, a heuristic (no accuracy figure measured yet); `rtok stats --calibrate` rewrites
code  = 3.5
prose = 4.2
json  = 3.0
cjk   = 1.0

# ── surfaces ────────────────────────────────────────────────────────────────

[hook]                                # rtok hook <event>
host      = "claude"                  # claude | cursor | copilot | devin | cline — payload field mapping (T10.1, T46.3, T87, T94)
max_ms    = 10                        # soft budget; over it, the event is logged as slow
fail_open = true                      # any error → `{}` and exit 0; false only for debugging

[agents]                              # the rtok agent registry (T282, D34); see agents-and-worktrees.md
enabled    = true                     # false = hooks skip agent register/touch/end and message push (session bookkeeping is unaffected)
idle       = "30m"                    # `live()`'s window: no `ended_at` and `last_seen` within this of now
push_bytes = 1024                     # framed messages pushed per UserPromptSubmit/PostToolUse; the rest → "and N more" (T288)

[agents.usage]                        # rtok agents usage (T358)
source = "logs"                       # logs = the agents' own session files (Claude Code, Codex, OpenCode, Kilo, Copilot CLI, Gemini CLI); rtok = what passed through rtok; both
hosts  = []                           # [] = every host; else host ids, e.g. ["claude", "codex"]
since  = ""                           # "" = all time; a date (2026-09-01, whole days in tz) or a duration (30d)
until  = ""                           # "" = through today; a date, inclusive
period = "monthly"                    # monthly | daily: the bottom table
by     = "agent"                      # agent | model: what the middle table groups by
tz     = ""                           # IANA zone for day and month boundaries; "" = the system zone

[agents.usage.dirs]                   # where `rtok agents usage` reads each host's own records (T358.3); Claude Code and Codex use [stats] transcripts_dir / codex_dir
opencode = ["~/.local/share/opencode"] # the opencode*.db files in it; an untouched default follows $XDG_DATA_HOME
kilo     = ["~/.local/share/kilo"]     # the kilo*.db files in it; an untouched default follows $XDG_DATA_HOME
copilot  = ["~/.copilot/session-state"] # */events.jsonl; an untouched default follows $COPILOT_HOME
gemini   = ["~/.gemini/tmp"]           # */chats/session-*; an untouched default follows $GEMINI_CLI_HOME
droid    = ["~/.factory/sessions"]     # listed as unsupported when present: Factory does not document the token fields
pi       = ["~/.pi/agent/sessions"]    # */*.jsonl; an untouched default follows $PI_CODING_AGENT_SESSION_DIR, else $PI_CODING_AGENT_DIR/sessions
kimi     = ["~/.kimi-code/sessions"]   # Kimi Code: */*/agents/*/wire.jsonl; an untouched default follows $KIMI_CODE_HOME
grok     = ["~/.grok/sessions"]        # listed as unsupported when present: xAI points at `grok usage`, which rtok does not run; follows $GROK_HOME
zcode    = ["~/.zcode"]                # listed as unsupported when present: ZCode does not document its session records
antigravity = ["~/.gemini/antigravity"] # listed as unsupported when present: Google does not document Antigravity's local data

[agents.junk]                         # rtok agents junk list|clear: age floors, protected paths, your own junk paths
keep_logs_days          = 30          # `logs` entries (the agents' documented log folders) modified within this many days stay; 0-3650
temp_min_age_hours      = 24          # `temp` entries touched within this many hours stay; 0-87600
stale_session_days      = 30          # `--kind sessions` only: sessions not touched for more than this many days are junk, time is the only criterion; 0-3650; `--session-days N` for one run
stale_worktree_days     = 14          # a merged, clean worktree `rtok worktree gc` would remove is junk (`stale-worktrees`, review) once idle this many days; 0-3650
crash_dump_min_age_days = 7           # a crash dump in an `extra` crash-dumps folder older than this many days is safe to clear, a younger one is review; 0-3650
exclude                 = []          # globs (~ = home) never touched, nor any folder holding a match, e.g. ["~/.claude/debug/keep-*"]; a bad glob keeps everything
extra                   = []          # paths you vouch for as junk (D36), e.g. [{ host = "cursor", kind = "cache", path = "~/Library/Application Support/Cursor/CachedData" }]; kind = cache | temp | logs | crash-dumps; host = a host id or rtok

[mcp]                                 # rtok mcp
tools                   = []          # [] = all tools from enabled plugins; else an allow-list; `expand` always stays listed (D4)
max_description_tokens  = 60          # enforced by a test (T4.1)
max_result_chars        = 20000       # above this, head/tail + archive id
http                    = "127.0.0.1:8791"  # `rtok mcp --http` with no address; keep it on loopback
http_tools              = ["read", "search", "tree"]  # HTTP allow-list, used instead of `tools`; `expand` always stays listed; tools that act as the calling agent (`whoami`, `agent_send`, `worktree_add`, …) never are
token                   = ""          # bearer token for --http; prefer RTOK_MCP_TOKEN; empty = --http refuses to start (T401)
public_url              = ""          # tunnel URL (https://…); its host and origin are the only foreign ones accepted

[proxy]                               # rtok proxy
enabled         = true                # false = plain reverse proxy (bypass compress/bookkeeping); does NOT stop HTTP
bind            = "127.0.0.1"
port            = 8790
mode            = "passthrough"       # passthrough | compress
upstream        = "https://api.anthropic.com"      # RTOK_UPSTREAM; chain behind another proxy for A/B
openai_upstream = "https://api.openai.com"         # RTOK_OPENAI_UPSTREAM (D11)
gemini_upstream = "https://generativelanguage.googleapis.com"  # RTOK_GEMINI_UPSTREAM (T51.3)
timeout_s       = 600                 # upstream request timeout
include_usage   = true                # OpenAI streaming: add stream_options.include_usage when missing (T11.2)
context_management = false            # Anthropic /v1/messages only: add clear_tool_uses edit + beta header (T51.2, opt-in)
dry_run         = false               # --dry-run: print effective [proxy] settings and exit, don't serve
# TLS: Mozilla webpki roots (`use_preconfigured_tls`). Corporate CAs: SSL_CERT_FILE (PEM, curl). See "TLS and corporate CAs".

[proxy.tools_rewrite]                 # T59.5; off: request bytes stay identical
enabled = false
max_description_tokens = 60           # 0 = no truncate; sentence boundary; estimator Class::Prose
allow = []                            # empty = keep all names not in deny
deny = []                             # drop these names from tools[]; later calls still forward

[proxy.lanes]                         # T385.1; tag each request's lane in the ledger (calls.kind); bytes stay identical
enabled = true                        # false = every request an untagged api_request; x-rtok-lane and /lane/<name>/ forwarded as sent

[proxy.lanes.bulk]                    # sync scripts; the agent lane follows the global switches, batch and files are never rewritten
compress            = false           # proxy.mode = "compress" rewrites (archive, compress, noise strip)
toon                = false           # the toon filter inside that pass; needs compress
tools_rewrite       = false           # proxy.tools_rewrite
context_management  = false           # proxy.context_management
semantic_cache      = false           # plugins.proxy.semantic_cache, lookup and store
flex                = false           # OpenAI service_tier = "flex" on this lane; see [proxy.flex]
timeout_s           = 0               # read timeout for this lane; 0 = proxy.timeout_s, but 900 or more with flex = true (OpenAI's Flex guide uses 15 min)
upstream            = ""              # base URL for every request on this lane, any wire; "" = proxy.upstream / openai_upstream / gemini_upstream
max_in_flight       = 0               # requests upstream at once; 0 = no cap (the agent lane is never capped)
max_queued          = 8               # with max_in_flight: requests that wait for a slot; one more gets 429 + Retry-After

[proxy.lanes.embeddings]              # same keys as bulk
compress            = false
toon                = false
tools_rewrite       = false
context_management  = false
semantic_cache      = false
flex                = false
timeout_s           = 0
upstream            = ""
max_in_flight       = 0
max_queued          = 8

[proxy.lanes.meta]                    # same keys as bulk (models, token counting)
compress            = false
toon                = false
tools_rewrite       = false
context_management  = false
semantic_cache      = false
flex                = false
timeout_s           = 0
upstream            = ""
max_in_flight       = 0
max_queued          = 8

[proxy.lanes.internal]                # same keys as bulk (rtok's own model calls)
compress            = false
toon                = false
tools_rewrite       = false
context_management  = false
semantic_cache      = false
flex                = false
timeout_s           = 0
upstream            = ""
max_in_flight       = 0
max_queued          = 8

[proxy.batch]                         # provider Batch result files
parse_results       = false           # read a fetched results file into one usage row per request; the body is forwarded as is

[proxy.flex]                          # OpenAI Flex tier; which lanes get it is [proxy.lanes.<lane>] flex
force               = false           # overwrite a service_tier the client sent (off: a client value is never changed)
on_429              = "none"          # Flex has no capacity: none = the 429 goes to the client | backoff = retry on Flex | default = retry once on service_tier "auto" (the client's own tier, if force replaced one)
retries             = 3               # backoff only: retries before giving up; at most 5
backoff_ms          = 1000            # backoff only: delay before the first retry, doubled each time, capped at 30 s; a 429's Retry-After (seconds) can lengthen it, up to that cap

[proxy.routing]                       # model routing; only the internal and bulk lanes, never the agent lane
enabled             = false           # off: every call keeps the model the client sent

[proxy.routing.bulk]                  # one rule per lane; it acts only with from, to and gate all set
from                = []              # request models it applies to: an exact id or a prefix ending in * (empty = off)
to                  = ""              # the model that serves a matching call (empty = off)
provider            = ""              # provider slug recorded for a routed call (empty = the wire's own); pair with [proxy.lanes.<lane>] upstream
gate                = ""              # where the quality bench for this swap is kept (a path, URL or run id); empty = off

[proxy.routing.internal]              # same keys as bulk (rtok's own model calls)
from                = []
to                  = ""
provider            = ""
gate                = ""

[web]                                 # rtok web (same data as rtok tui)
host = "127.0.0.1"                    # --host
port = 3333                           # --port

[tui]                                 # rtok tui (same data as rtok web)
tab       = ""                        # "" = first tab          (--tab <page>)
tick_secs = 2                         # model re-read cadence, same as the web 2 s tick (--tick-secs)

[ui]                                  # rtok's own lines on a terminal; pipes, --json stay plain
emoji = true                          # emoji before status/warn/error lines (RTOK_UI_EMOJI)
color = true                          # colour them; NO_COLOR/CLICOLOR_FORCE apply (RTOK_UI_COLOR)

[stats]                               # rtok stats
since           = "30d"
format          = "table"             # table | json      (--json)
plugin          = ""                  # "" = all         (--plugin <id>)
transcripts_dir = "~/.claude/projects"
codex_dir       = "~/.codex/sessions" # Codex CLI logs → one more `api` row (T49.2); OpenCode and Copilot CLI are read by `rtok agents usage` ([agents.usage.dirs], T358.3); Cursor stores carry no token counts (surveyed 2026-09-17), so they are not read
calibrate_samples = 30                # per class        (--calibrate)
baseline        = ""                  # default name for --compare; "" = none
price           = false               # show per-model USD costs (--price)
# USD per MTok rows for --price (T49.1). Sources, fetched 2026-09-17 (Anthropic claude-fable-5-1,
# claude-opus-5-5 and claude-sonnet-5-5: 2026-10-08):
# Anthropic claude-* rows: https://platform.claude.com/docs/en/about-claude/pricing
# (input / 5m cache write / cache read / output). OpenAI gpt-5 / gpt-5-mini:
# https://platform.openai.com/docs/pricing (short-context input / cached input /
# output; no separate write price, so cache_write = input). Models without a row
# print `-`, never a guess; add dated rows of your own the same way.
[stats.prices."claude-fable-5-1"]
input = 10.0
cache_write = 12.5
cache_read = 0.25
output = 50.0
[stats.prices."claude-opus-5-5"]
input = 4.0
cache_write = 5.0
cache_read = 0.2
output = 20.0
[stats.prices."claude-sonnet-5-5"]
input = 2.0
cache_write = 2.5
cache_read = 0.1
output = 10.0
[stats.prices."claude-sonnet-5"]
input = 2.0
cache_write = 2.5
cache_read = 0.2
output = 10.0
[stats.prices."claude-haiku-4-5"]
input = 1.0
cache_write = 1.25
cache_read = 0.1
output = 5.0
[stats.prices."gpt-5"]
input = 1.25
cache_write = 1.25
cache_read = 0.125
output = 10.0
[stats.prices."gpt-5-mini"]
input = 0.25
cache_write = 0.25
cache_read = 0.025
output = 2.0
# Batch and Flex tiers, fetched 2026-10-08: the key is `<model>@batch` or `<model>@flex`, and
# `--price` costs Batch-lane usage at the `@batch` row. Anthropic Batch is 50 % off input and
# output with the cache multipliers stacked on top (pricing page above; no Flex tier). OpenAI
# Batch and Flex list the same rates (https://developers.openai.com/api/docs/pricing).
[stats.prices."claude-fable-5-1@batch"]
input = 5.0
cache_write = 6.25
cache_read = 0.125
output = 25.0
[stats.prices."claude-opus-5-5@batch"]
input = 2.0
cache_write = 2.5
cache_read = 0.1
output = 10.0
[stats.prices."claude-sonnet-5-5@batch"]
input = 1.0
cache_write = 1.25
cache_read = 0.05
output = 5.0
[stats.prices."claude-sonnet-5@batch"]
input = 1.0
cache_write = 1.25
cache_read = 0.1
output = 5.0
[stats.prices."claude-haiku-4-5@batch"]
input = 0.5
cache_write = 0.625
cache_read = 0.05
output = 2.5
[stats.prices."gpt-5@batch"]
input = 0.625
cache_write = 0.625
cache_read = 0.0625
output = 5.0
[stats.prices."gpt-5@flex"]
input = 0.625
cache_write = 0.625
cache_read = 0.0625
output = 5.0
[stats.prices."gpt-5-mini@batch"]
input = 0.125
cache_write = 0.125
cache_read = 0.0125
output = 1.0
[stats.prices."gpt-5-mini@flex"]
input = 0.125
cache_write = 0.125
cache_read = 0.0125
output = 1.0

[report]                              # rtok report (D24: renders the operator model, computes nothing)
format = "md"                         # md; html (T22.2), pdf (T22.3), --ai (T22.4)
out    = ""                           # "" = stdout     (--out <path>)
since  = "30d"                        # how far back the report reads
ai     = false                        # model-shaped rendering instead of --format (--ai)
budget_tokens = 8000                  # --ai drops whole sections past this (T22.4)

[bench]                               # rtok bench
tasks    = "bench/tasks.toml"
runs     = 3
dry_run  = false
timeout_s = 900                       # per task run
suite    = ""                         # "" = T9.1 six tasks; "graph" = T68.9 with/without MCP
[bench.configs]                       # name = settings file passed to `claude --settings`
a = "bench/configs/legacy.json"
b = "bench/configs/rtok.json"

[doctor]                              # rtok doctor
settings_path   = "~/.claude/settings.json"
claude_json     = "~/.claude.json"
mcp_json        = ".mcp.json"
probe_timeout_ms = 500                # per proxy hop /health probe
mcp_timeout_ms  = 15000               # per MCP server tools/list (uvx/npx servers start slowly)
instruction_warn_tokens = 1000        # --instructions: flag files above this
instructions    = false               # run the instruction audit by default (--instructions)

[setup]                               # rtok agents install / remove <host>
dry_run      = false
yes          = false                  # required by --replace
backup       = true                   # <name>.bak-<ts> beside each file, before setup and remove touch it
backup_files = 5                      # .bak-* generations kept per file; older ones are deleted (0 = keep all)
hook_timeout_s = 5                    # timeout written into each hook entry
modes        = []                     # e.g. ["terse", "yagni"]   (--mode)
mcp          = true                   # also register the MCP server   (--mcp)
proxy        = false                  # also set the base URL          (--proxy)
[setup.claude]
settings_path = "~/.claude/settings.json"
[setup.cursor]
hooks_path    = "~/.cursor/hooks.json"  # today: beforeShellExecution only (T10.11 wires PostToolUse)
[setup.codex]
config_path   = "~/.codex/config.toml"
[setup.opencode]
config_path   = "~/.config/opencode/opencode.json"
[setup.kilo]
config_path   = "~/.config/kilo/kilo.json"      # kilo.jsonc is merged by Kilo, never rewritten
[setup.pi]
extensions_path = "~/.pi/agent/extensions"
tools           = false                     # pi.registerTool for read/search/graph/memory (T70.3)
[setup.omp]
extensions_path = "~/.omp/agent/extensions" # oh my pi: plugins/pi is linked here (T92)
mcp_path        = "~/.omp/agent/mcp.json"
[setup.zcode]
config_path   = "~/.zcode/cli/config.json"
[setup.kimi]
config_path   = "~/.kimi-code/config.toml"  # mcp.json is read beside it
[setup.copilot]
dir           = "~/.copilot"                # mcp-config.json, hooks/rtok.json
[setup.commandcode]
dir           = "~/.commandcode"            # settings.json (hooks key), mcp.json
[setup.aider]
config_path   = "~/.aider.conf.yml"         # openai-api-base → rtok proxy (--proxy)
[setup.windsurf]
config_path   = "~/.codeium/windsurf/mcp_config.json"
[setup.zed]
config_path   = "~/.config/zed/settings.json"
[setup.cline]
hooks_path = "~/Documents/Cline/Hooks"
mcp_path = "~/.cline/data/settings/cline_mcp_settings.json" # CLI; the extension uses the VS Code globalStorage settings file
[setup.gemini]
dir = "~/.gemini" # settings.json (hooks, mcpServers)
[setup.codewhale]
dir = "~/.codewhale" # config.toml ([[hooks.hooks]]), mcp.json (mcpServers)
[setup.mimo]
config_path   = "~/.config/mimocode/mimocode.json" # mcp (OpenCode-fork shape)
[setup.antigravity]
plugins_path     = "~/.gemini/config/plugins"          # Antigravity 2.0 / IDE: plugins/antigravity linked here
cli_plugins_path = "~/.gemini/antigravity-cli/plugins" # agy plugin install stages here; read only
[setup.devin]
config_path   = "~/.config/devin/config.json"  # mcp_config.json is read beside it; Windows: %APPDATA%\devin\
[setup.roo]
mcp_path      = ""                               # empty: <Code user dir>/globalStorage/rooveterinaryinc.roo-cline/settings/mcp_settings.json
[setup.qwen]
dir           = "~/.qwen"                    # settings.json (hooks, mcpServers); QWEN_HOME moves the directory
[setup.codebuddy]
dir           = "~/.codebuddy"               # CodeBuddy Code: settings.json (hooks), .mcp.json, plugins/; CODEBUDDY_CONFIG_DIR moves it
workbuddy_dir = "~/.workbuddy"               # WorkBuddy Desktop: the same files (embedded CodeBuddy Code; unverified)

[expand]                              # rtok expand <id>
max_lines = 0                         # 0 = unlimited   (--lines a-b is per call)
max_rate  = 0.05                      # re-read ceiling; above it the report flags lossy compression (T22.5)

[filter]                              # rtok filter --stdin (T10.2)
cmd = ""                              # command family hint when the caller knows it (--cmd)

[worktree]                            # rtok worktree add | claim | remove | list | gc
enabled = true                        # false: every `rtok worktree` command (list too) says worktrees are not enabled, MCP lists no worktree_* tool, and Claude's WorktreeCreate/WorktreeRemove hooks do what Claude does without rtok
root = "~/.rtok/worktrees"            # where `rtok worktree add` creates worktrees, as <root>/<repo>-<task>; `~` expands

[tasks]                               # task adapters (T441); usually set per project in .rtok.toml
adapter = "disk"                      # disk | github | gitlab
prefix = ""                           # task id prefix, 1–8 ASCII letters (R → R12, R2.1); empty: first letter of the project name

[tasks.disk]
dir = "tasks"                         # one Markdown file per task, relative to the project root; done ones go to <dir>/done

[tasks.github]
repo = ""                             # owner/name; empty: the origin remote
project = 0                           # repo owner's Projects v2 number: issues join it and its Status follows the task; 0 = issues only

[tasks.gitlab]                        # status::in-progress | status::done | status::wont-do labels; a subtask links to its parent (relates_to)
url = "https://gitlab.com"            # https base URL; set it for a self-hosted instance; token: GITLAB_TOKEN, GITLAB_ACCESS_TOKEN, GL_TOKEN, else glab
project = ""                          # group/name or numeric id; empty: the origin remote

[tasks.export.github]                 # `rtok task export github`: mirror the disk tasks into GitHub Issues, one way
repo = ""                             # owner/name; empty: [tasks.github] repo, else the origin remote

[tasks.export.linear]                 # `rtok task export linear`: mirror the disk tasks into Linear, one way; token in LINEAR_API_KEY
team = ""                             # the team's short key, e.g. "ENG"; empty: the export refuses to run

[otel]                                # OpenTelemetry export (D19); off until endpoint resolves
endpoint      = ""                    # OTLP/HTTP base URL, e.g. "http://localhost:4318"; "" = $OTEL_EXPORTER_OTLP_ENDPOINT
headers       = ""                    # "k=v,k2=v2", e.g. "signoz-ingestion-key=…"; "" = $OTEL_EXPORTER_OTLP_HEADERS
service_name  = "rtok"                # resource service.name
content       = true                  # gen_ai.input/output.messages, tool arguments and results on spans
content_bytes = 65536                 # per attribute; beyond it rtok.archive.id → `rtok expand <id>`
flush_secs    = 5                     # proxy / mcp flush interval, and the POST timeout

# ── plugins ─────────────────────────────────────────────────────────────────

[plugins.measure]
enabled = true

[plugins.cmd]
enabled  = true
rewrite  = true                       # PreToolUse(Bash) → `rtok run -- …`
shell    = ""                         # "" = $SHELL
rules    = "~/.rtok/rules.toml"       # extra filter rules; missing → built-in rules/default.toml
rules_dir = "~/.rtok/rules.d"         # drop-ins: every *.toml merges after rules in name order (T50.2)
trailer_min_lines = 40                # add `[rtok <id> · N lines · expand …]` above this
fail_tail_lines   = 80                # non-zero exit → last N lines verbatim
never_wrap = ["rtok", "sudo"]         # first-word deny list; heredocs, `&`, -i are always skipped

[plugins.read]
enabled          = true
default_mode     = "full"             # full | lines | map | signatures
max_chars        = 20000              # above this, head/tail + archive id
native_max_bytes = 32768              # PreToolUse(Read) deny threshold; never below this
range_max_lines = 300              # T383: a native Read with limit 1..=N passes the hook; 0 = unranged only
advice           = true               # false = never deny native Read
allow_paths      = []                 # extra roots outside cwd
search_max       = 50
search_max_bytes = 1048576             # search skips files larger than this (T55.5)
tree_depth       = 2
delta            = true               # T58.1: changed re-read → unified diff vs last archive (7.3 % of Read bytes, 2026-09-18, `rtok stats --since 90d`)
delta_max_ratio  = 0.6                # full file when the diff is not below this fraction

[plugins.json_tree]
enabled = false                     # fold repeated nested JSON in old tool results; off until a measurement shows a saving

[plugins.archive]
enabled    = true
keep_turns = 4                        # never touch the last N turns
min_tokens = 1500                     # only rewrite tool results above this (estimated)
head_lines = 8
tail_lines = 4
tiers      = false                    # opt-in tiered loading (default off); OpenViking L0/L1/L2 behaviour spec is AGPL-3.0 — rtok does not vendor, link, or subprocess it (D6); gates native impl in T33.2
live_blobs = false                    # shrink nested JSON dumps + data: blobs in user blocks, never results/system/tools/last-2-turns (T51.1, opt-in)
skills     = true                     # archive skill bodies outside keep_turns (T61.2); off with skills = false

[plugins.proxy]
enabled = true                        # the proxy plugin (usage capture); the server itself is [proxy]

[plugins.inject]
enabled       = true
budget_tokens = 800                   # per turn, all injections together (decision D5)
modes_dir     = "~/.rtok/modes"
modes         = []                    # same as [setup].modes; setup writes here

[plugins.guard]
enabled      = true
window_turns = 8
deny_grep_glob = false           # opt-in: deny native Grep/Glob, point at MCP search/tree (T50.4)
grep_symbol = false              # opt-in: a Grep for one identifier (`foo`, `fn foo`, `class Foo`, `\bfoo\(`) is denied with its 1-5 indexed definitions + reference count; index lookup only (T369)
skills = false                   # opt-in (Claude Code): deny a Skill whose SKILL.md exceeds skill_max_bytes with its map + `expand <id>` (T62.1)
skill_max_bytes = 8192           # bodies at or under this load whole; so does any skill with allowed-tools / model / context / agent in its frontmatter

[plugins.memory]
enabled        = true
recall_titles  = 5                    # SessionStart: last N titles + ids
recall_tokens  = 200
prompt_recall  = 5                    # UserPromptSubmit: 0 = off; N = ranked titles per turn (T69.5)
checkpoint_tokens = 400               # PreCompact → SessionStart(compact)
search_limit   = 5
sync_tokens    = 300                  # rtok memory sync: CLAUDE.md / AGENTS.md block (T69.6)
startup_recall = true                 # SessionStart(startup) restores newest session:* note (T71.2)
handoff        = true                 # T59.6 sub-agent digest MCP tool
spawn_brief        = true             # T130: SubagentStart pointer digest
spawn_brief_tokens = 300              # T130: token budget for the spawn brief

[plugins.memory.embed]
enabled    = false                    # P29: FTS5-only when false; vector search is opt-in
provider   = "local"                  # "local" | "openai"
model      = "all-MiniLM-L6-v2"
dimensions = 384
hybrid     = true                     # when enabled: RRF(fts5, knn) for mem_search and prompt_recall; the hook reads stored vectors only; false = knn only

[plugins.graph]
enabled    = true
max_tokens = 2000                     # per response; beyond it: head + "N more, expand <id>"
impact_tokens = 1500                  # impact: files grouped and ranked up to this budget, then "+K files, M refs not shown"; --all or 0 = every row
map_tokens = 0                        # SessionStart repo map cap (D5 share next to memory.recall_tokens); 0 = off until a P7 A/B passes
map_rank   = "refs"                   # SessionStart map order: refs = references per name; pagerank = files by personalized PageRank, personalized by recently edited files and the last checkpoint after a compact
body_lines = 40                       # symbol(): source lines shown per definition
auto_index = true                     # true = every call walks the tree; false = index once, then `rtok graph index` or the watcher (a hook-staled file reads as missing until then)
auto_add_projects = true               # T329.6: register a directory in the project registry when a hooked session starts there, a worktree is made or adopted through `rtok worktree` (named by its branch), or a graph MCP call runs there; false = the registry changes only through the page and the CLI
backend    = "tags"                   # tags | lsp | auto | text: tags = tree-sitter index (default); lsp = language server from PATH, tags when it cannot answer; auto = per project and language, server first, tags second, text search when no grammar parses the project; text = in-process text search only (no call graph, no dead)
lsp_timeout_ms = 40000                # how long one language-server wait (starting, still indexing) may take before the request falls back to tags
backend_by_language = {}              # backend for one language, e.g. { go = "tags", rust = "lsp" }
watch      = "off"                    # off | notify: background re-index inside `rtok mcp` (P8d)
auto_link_references = true           # T329.8: follow references in manifests (Cargo path, npm file:/link:, go replace, Python path, submodules) into other directories, register and auto-link them
reference_depth = 3                   # T329.8: reference levels followed from the project (A -> B is 1); reaching it is shown and logged
max_auto_projects = 20                # T329.8: most projects references may add to the registry; reaching it is shown and logged
alerts = true                         # T329.17: alert when a project in the scope is missing, unreachable, has its backend down or a broken manifest link; shown by rtok doctor, graph projects and graph answers
health_check_interval_s = 60          # T329.17: seconds between health checks in rtok mcp and rtok web (an alert needs two in a row); 0 = off
live_heat_window_s = 300            # T329.34: seconds a node stays warm on the live graph canvas after a call (rtok web, rtok tui)
live_max_events_per_s = 50          # T329.34: most call events the live panel lists per second; the counters still count every call
live_feed_rows = 200                # T329.34: finished calls the live feed keeps (1-1000)

[plugins.toon]
enabled  = true
min_rows = 5

[plugins.compress]
enabled = true                        # extractive summaries of archived tool output; runs only in proxy.mode = "compress"

[plugins.docs]
enabled        = false                # local rustdoc from Cargo.lock + cached docs.rs JSON (T455)
query_limit    = 5                    # docs_query hit ceiling
snippet_chars  = 400                  # chars per FTS snippet
max_tokens     = 800                  # whole-reply estimate cap; extra hits are dropped

[plugins.wasm]
enabled = false                      # off by default; no .wasm loaded until T32.2 host + `wasm-host` feature
dir     = "~/.rtok/plugins"          # scan one level for *.wasm; D6 — this repo never vendors third-party plugins
```

### `[plugins.proxy.semantic_cache]` — opt-in response cache (P31)

Off by default until Gate P31 (zero false hits on the P9 set). When enabled (T31.2), the proxy may
serve a prior response when a normalized prompt is similar enough; a false hit is a wrong answer, so
this stays opt-in. Env: `RTOK_PLUGINS_PROXY_SEMANTIC_CACHE_ENABLED=true`.

| Key | Default | Meaning |
|-----|---------|---------|
| `enabled` | `false` | Master switch; proxy bytes stay identical when off |
| `threshold` | `0.99` | Cosine similarity floor for the semantic tier |
| `ttl_s` | `300` | Entry TTL in seconds |
| `max_messages` | `1` | Skip cache when `messages` length exceeds this |
| `require_empty_tools` | `true` | Do not cache turns with non-empty `tools[]` |
| `embed_backend` | `"hash"` | `"hash"` = direct tier only until P29 embeddings |
| `cache_by_model` | `true` | Partition cache entries by model |
| `cache_by_provider` | `true` | Partition cache entries by provider |

```toml
[plugins.proxy.semantic_cache]
enabled = false
threshold = 0.99
ttl_s = 300
max_messages = 1
require_empty_tools = true
embed_backend = "hash"
cache_by_model = true
cache_by_provider = true
```



### `[proxy.lanes]`

Every proxied request is classified into a lane and the lane is written to the ledger
(`calls.kind`). The path decides first: Batch (`/v1/messages/batches`, `/v1/batches`,
Gemini `:batchGenerateContent`), `files`, `embeddings` and `meta` (`/v1/models`,
`count_tokens`) name their own lane. A sync chat call is an `agent` turn unless the caller
says otherwise with an `x-rtok-lane: bulk|internal` header or a `/lane/<name>/` path prefix
(`/lane/bulk/v1/messages`). Both are stripped before the request goes upstream. There are
no heuristics: an unmarked request is `agent`, the lane every request had before lanes.

| Key | Type | Default | Meaning |
|-----|------|---------|---------|
| `enabled` | bool | `true` | `false` records every request as a plain `api_request` and forwards the header and prefix untouched |

The agent lane keeps `calls.kind = api_request`; the others record `api_request:<lane>`
(`api_request:bulk`, `api_request:batch`, ...). Request bytes are not changed by the lane.

The `agent` lane has no table: every global switch decides for it exactly as it did before
lanes existed (prompt-cache stability). `batch` and `files` have none either: their bodies
(Batch JSONL, uploads) are always forwarded verbatim, and `stream_options` shaping is
skipped on them too. `bulk`, `embeddings`, `meta` and `internal` each read a
`[proxy.lanes.<lane>]` table, and every switch in it is off by default, so those lanes are
forwarded byte for byte until you opt one in. A lane switch only narrows its global
counterpart: a rewrite runs on a lane when the global switch *and* the lane switch are on.

| Key | Global switch it narrows | Default | Meaning |
|-----|--------------------------|---------|---------|
| `compress` | `proxy.mode = "compress"` | `false` | archive, compress and terminal-noise strip |
| `toon` | `plugins.toon.enabled` | `false` | the `toon` filter inside that pass; needs `compress` |
| `tools_rewrite` | `proxy.tools_rewrite.enabled` | `false` | `tools[]` description rewrite |
| `context_management` | `proxy.context_management` | `false` | Anthropic server-side context editing |
| `semantic_cache` | `plugins.proxy.semantic_cache.enabled` | `false` | cache lookup and store |
| `flex` | none | `false` | OpenAI `service_tier = "flex"` on chat and responses calls, see [`[proxy.flex]`](#proxyflex) |
| `timeout_s` | `proxy.timeout_s` | `0` | read timeout in seconds for this lane; `0` = `proxy.timeout_s`, raised to 900 when the lane has `flex = true` (the OpenAI Flex guide raises its SDK timeout to 15 minutes because Flex requests time out more often) |
| `upstream` | `proxy.upstream`, `openai_upstream`, `gemini_upstream` | `""` | base URL for every request on this lane, whatever its wire (a gateway or local server that speaks them); `""` = the wire's own upstream |
| `max_in_flight` | none | `0` | requests on this lane upstream at once, counted until the response stream ends; `0` = no cap |
| `max_queued` | none | `8` | with `max_in_flight`: requests that wait for a slot; one more is answered `429` with `Retry-After: 1` and never reaches upstream |

Each lane has its own slots and its own queue, and the `agent` lane has neither, so a bulk
burst can fill its own lane but never delays an agent turn. A request turned away by a full
queue gets an Anthropic-shaped `rate_limit_error` that the provider SDKs back off and retry
on; it writes no `calls` row (it never went upstream), only a `warn` log line. With the proxy
switched off (`proxy.enabled`, `core.enabled` or `plugins.proxy.enabled` false) no request is
capped or queued. `batch` and `files` have no `upstream`: a Batch job's create, poll and
results always go to the provider that owns it.

Model routing is not a lane switch: it has its own table, [`[proxy.routing]`](#proxyrouting).
The `agent` lane has no `flex` switch: a live turn never changes tier unless the client asks
for it.

### `[proxy.batch]`

Provider Batch observation. The Batch calls themselves (create, poll, list, cancel, results) are
already tagged `api_request:batch` by `[proxy.lanes]`, and their bodies are never rewritten.

| Key | Type | Default | Meaning |
|-----|------|---------|---------|
| `parse_results` | bool | `false` | After a results file was forwarded, write one `usage` row per succeeded request: Anthropic `GET /v1/messages/batches/{id}/results`, and OpenAI `GET /v1/files/{id}/content` when its lines are Batch results (that call is then re-tagged `api_request:batch`). Errored, expired and malformed lines are skipped. Needs `[proxy.lanes] enabled`; the response bytes are untouched. |

```toml
[proxy.batch]
parse_results = false
```

### `[proxy.flex]`

OpenAI Flex processing bills a chat or responses call at Batch rates in exchange for latency
([OpenAI: Flex processing](https://developers.openai.com/api/docs/guides/flex-processing), checked
2026-10-08). With `flex = true` on a lane (`bulk`, `internal`, `embeddings` or `meta`; `agent`, `batch`
and `files` have none), rtok sets `service_tier = "flex"` on that lane's calls to `/v1/chat/completions`
and `/v1/responses`. Anthropic has no Flex tier and its wire is never touched.

A `service_tier` the client sent (`auto`, `default`, `priority`, `flex`) is never overwritten unless
`force = true`. When the field is absent, rtok adds it in front of the object and forwards every
other byte as the client sent it.

When Flex has no capacity OpenAI answers `429 Resource Unavailable` and does not charge for it. The
guide names no error code, so rtok treats any `429` on a request it set to Flex that way. A request
whose Flex tier came from the client is never retried: that `429` is the client's to handle.

| Key | Type | Default | Meaning |
|-----|------|---------|---------|
| `force` | bool | `false` | Overwrite a client-supplied `service_tier` with `flex` |
| `on_429` | string | `"none"` | `none` hands the `429` to the client; `backoff` retries on Flex with doubling delays; `default` retries once with `service_tier = "auto"` (standard processing, which costs more) |
| `retries` | int | `3` | `backoff` only: retries before the last `429` goes to the client; at most `5` |
| `backoff_ms` | int | `1000` | `backoff` only: wait before the first retry, doubled each time, capped at 30 s |

The client waits while rtok retries and sees only the final response. A `default` retry restores the
`service_tier` the client sent when `force` replaced it, and uses `auto` only when the client sent
none. A `Retry-After` on the `429` (delta-seconds only; a date or other value is ignored) lengthens
the wait to the larger of it and the backoff delay, never past 30 s. One that asks for more is not
waited for: `backoff` hands the `429` to the client, `default` retries on the fallback tier at
once. A `408` is never retried and reaches the client unchanged. Retries are logged at `warn`.

```toml
[proxy.lanes.bulk]
flex = true

[proxy.flex]
force = false
on_429 = "backoff"
```

### `[proxy.routing]`

Model routing (D9) moves a call to a cheaper model, and only where the quality risk is lowest:
rtok's own `internal` calls and the opted-in `bulk` lane (`docs/research/optimization.md` §4.2).
The `agent` lane is never routed: a live turn runs on the model the operator chose. `batch` and
`files` are never rewritten either. Off by default, and no rule ships.

| Key | Type | Default | Meaning |
|-----|------|---------|---------|
| `enabled` | bool | `false` | Master switch; off, every call keeps the model the client sent |

`[proxy.routing.bulk]` and `[proxy.routing.internal]` each hold one rule. A rule acts only when all
four keys below are set; a half-written rule does nothing.

| Key | Type | Default | Meaning |
|-----|------|---------|---------|
| `from` | string list | `[]` | Request models the rule applies to: an exact id, or an id ending in `*` for a prefix (`claude-opus-*`). A model not listed is forwarded as it is |
| `to` | string | `""` | The model that serves a matching call |
| `provider` | string | `""` | Provider slug recorded for a routed call, when `to` lives at another provider than the wire's own; `""` = the wire's provider |
| `gate` | string | `""` | Where the quality bench for this swap is kept (a path, URL or run id) |

`gate` is the I-101 quality gate. Whether the cheaper model answers well enough is a bench result,
not something a proxy can see, so rtok asks the operator to run that bench on the lane's real
workload first and to name it here; it does not read or judge it. Without it the rule stays off.

Only the JSON `model` field is replaced (the body is re-serialised); Gemini names its model in the
path, which routing does not rewrite, so a Gemini call is never routed. The swap happens before
the call is recorded, so `calls` and `usage` name the model that served it (and `provider`, when
set), `rtok stats --price` prices the lane at that model, and the log row of the call keeps the
model the client asked for. Each routed call also gets a `model_route` measurement row; it claims
no token saving, because the saving is dollars: compare `cost by lane` from `rtok stats --price`
for the same workload with routing off and on. Pair a cross-provider rule with
`[proxy.lanes.<lane>] upstream` for the gateway that serves `to`; rtok does not translate between
wires, so `to` must be a model that wire's API accepts.

```toml
[proxy.routing]
enabled = true

[proxy.routing.bulk]
from = ["claude-sonnet-*"]
to = "claude-haiku-4-5"
gate = "bench/bulk-haiku-2026-10.md"
```

### Stats prices (`[stats.prices]`)

`rtok stats --price` prices the proxy `usage` rows in USD: each leg at its
`$` per MTok row, `cost` their sum, `saved` what the cache reads saved versus
uncached input price — the only saving computable from the `usage` rows alone.
A model without a row prints `-` for both dollar columns (its token counts
still print); add a dated row of your own rather than guessing. The shipped
rows were read off the providers' pricing pages on 2026-09-17 (sources in
`crates/rtok-config/default.toml`); re-check them when your bill disagrees. `stats.price`
defaults the `--price` display on (`RTOK_STATS_PRICE=true` works too).

A row keyed `<model>@batch` (or `<model>@flex`) prices that model on the Batch (or Flex) tier. `--price` lists
usage of Batch-lane calls under `<model>@batch` and costs it at that row; with no such row it prints `-`
rather than borrow the standard rate. Calls the provider reports serving on Flex (`service_tier` in the response,
recorded on `calls.service_tier`) are listed under `<model>@flex` and costed the same way. `rtok stats` and `rtok report`
add a `lane/tier` table of cache counters once traffic ran off the agent lane or on a tier other than `standard` /
`default`; a call whose response named no tier prints `-`.

Once traffic ran off the agent lane, `--price` also prints `cost by lane` (the JSON `cost.lanes`): each
lane's USD, with every call priced at the model recorded for it, so a call moved by
[`[proxy.routing]`](#proxyrouting) is costed at the model that served it. A `*` marks a lane that also used
a model without a price row, so its figure is a floor.

### WASM plugin host (`[plugins.wasm]`)

Out-of-tree `.wasm` plugins (P32, decision D6). This repo writes every catalogue plugin from
scratch and **never vendors third-party `.wasm` blobs** — operators install them under
`plugins.wasm.dir` on their machine. Default `enabled = false`: in-tree builds and the default
config do not load WASM. The Wasmi host and Cargo feature `wasm-host` land in T32.2; until then
the flag is visible in `rtok config show --sources` but has no loader.

### Graph backends (`[plugins.graph]`)

`backend = "tags"` (default) answers from the tree-sitter index. `"lsp"` routes `symbol` / `callers` /
`impact` / `outline` / `explore` through a language server from `PATH` and gives the tags answer when
the server cannot answer. `"auto"` chooses per project and language: the server when the project's
language has one installed, tags otherwise, text search when no grammar parses the project, and every
answer says which mode spoke. `"text"` (T329.10) pins plain text search: word-boundary patterns for
definitions and mentions over the files the `search` tool walks, in process (no `rg`, `grep` or `ssh`
is started), with no call graph, so `dead` and the `to` chains of `impact` answer "not available in
text mode".

`backend_by_language` overrides `backend` for one language, named by the project's marker file:
`rust`, `c`, `typescript`, `dart`, `go`, `python`, `javascript`, `java`, `ruby`, `php`, `elixir`,
`swift` (`go = "tags"` keeps Go on the index while the rest use `auto`). `lsp_timeout_ms` (default 40000) is how long one server wait may take
before the request falls back to tags. Setup walkthrough for Rust (rust-analyzer) and Dart (Dart SDK),
and the `auto` answer format: `docs/lsp.md`.

### Terminal output (`[ui]`)

rtok's own lines for a person at a terminal — `ok …`, `… started` / `… stopped`, `warning: …`,
`Error: …`, the `graph index` summary, `--help` — carry an emoji and a colour by default:
✅ success (green), 💡 status (cyan), ❗ warning (yellow), ❌ error (red). A line that names an operation takes that operation's icon instead (📚 index, 🚀 start, 🛑 stop, 🔗 link, 🧹 remove, …), padded so the text after it starts in one column.

```toml
[ui]
emoji = false   # RTOK_UI_EMOJI=false
color = false   # RTOK_UI_COLOR=false
```

- **Emoji** need `emoji = true` *and* a terminal on that stream.
- **Colour** needs `color = true` *and* a stream that takes colour: a terminal, `NO_COLOR`
  unset, `TERM` not `dumb` — or `CLICOLOR_FORCE` / `FORCE_COLOR` forcing it on a pipe.
  `color = false` also turns off the diff, state-word and log-level colours.
- What agents read never changes: hook and MCP JSON, filtered command output, `--json`, and
  anything written to a pipe or a file stay plain, byte for byte.
- `--help` colour is clap's own decision (terminal, `NO_COLOR`, `CLICOLOR_FORCE`): it prints
  before the config is read.

## TLS and corporate CAs

`rtok proxy` and OpenTelemetry export share one rustls client config: Mozilla
roots via `webpki-roots`, handed to reqwest with `use_preconfigured_tls`. They
do not use the macOS Security.framework verifier. Corporate or private CAs:
set `SSL_CERT_FILE` to a PEM bundle (curl's convention). Those certificates
extend the Mozilla set. If the variable is set, a missing, empty, or
unparsable file fails startup with the path in the error (curl parity).
Unset keeps Mozilla roots only. `rtok hook` never opens TLS.

## OpenTelemetry

`[otel]` turns on the exporter (`docs/otel.md`). Off until `endpoint` or
`OTEL_EXPORTER_OTLP_ENDPOINT` names a collector; nothing runs on the hook path.

## Mapping table (flags → keys)

| Subcommand | Flag | Key |
|-----------|------|-----|
| global | `--config <path>` | (selects the file; not a key) |
| global | `RTOK_HOME` | (selects the directory; env only, not a clap flag) |
| reading | `--json` | `stats.format` on `stats`; otherwise an action (the `web::model` page as JSON, not a stored key). On `stats`, `info`, `config show`, `doctor`, `plugins`, `agents list`, `agents sessions`, `agents whoami`, `agents show`, `agents inbox`, `worktree whoami`, `logs`, `demon status`, `otel status` |
| `hook` | `--host` | `hook.host` |
| `proxy` | `--port`, `--upstream`, `--mode`, `--dry-run` | `proxy.port`, `proxy.upstream`, `proxy.mode`, `proxy.dry_run` |
| `web` | `--host`, `--port` | `web.host`, `web.port` (`rtok dashboard` is the deprecated spelling) |
| `tui` | `--tab`, `--tick-secs` | `tui.tab`, `tui.tick_secs` |
| `stats` | `--since`, `--plugin`, `--compare`, `--calibrate`, `--cache`, `--price` | `stats.since`, `stats.format`, `stats.plugin`, `stats.baseline`, (`--calibrate`, `--cache` are actions; their knobs are `stats.calibrate_samples`), `stats.price` (`stats.prices.*` are data) |
| `report` | `--format`, `--out`, `--since`, `--ai` | `report.format`, `report.out`, `report.since`, `report.ai` (`report.budget_tokens` caps `--ai`) |
| `bench` | `--tasks`, `--runs`, `--dry-run`, `--timeout`, `--suite` | `bench.*` |
| `doctor` | `--instructions` | `doctor.instructions` |
| `agents install` | `--dry-run`, `--yes`, `--mode`, `--mcp`, `--proxy`, `--remove`, `--replace`, `--cli`, `--desktop`, `--all` | `setup.*` (`--remove`, `--replace`, `--cli`, `--desktop`, `--all` are actions) |
| `agents remove` | `--dry-run` | `setup.dry_run` (the command itself is the `--remove` action) |
| `agents list` | — | reads the host configs and `<bin> --version` (`--json` is the reading row) |
| `agents whoami` | — | reads `RTOK_AGENT_ID` and resolves it through the store (T283); no key, no `setup.*` (`--json` is the reading row) |
| `worktree whoami` | — | reads `RTOK_AGENT_ID` and `[worktree] root` (T411); no key of its own (`--json` is the reading row) |
| `task init` | `--adapter`, `--prefix` | `tasks.adapter`, `tasks.prefix`: written into the checkout's `.rtok.toml` (T441.5) |
| `task export` | `--dry-run`, `--json` | `tasks.export.github.repo`, `tasks.export.linear.team` (T441.13.5, T441.13.6); `--dry-run` is per call (no key) |
| `task create` / `list` / `status` | `--description`, `--body-file`, `--parent`, `--status`, `--all`, `--force` | per call (no key): what one task is and which rows one call shows; `[tasks]` picks the adapter and the prefix |
| `agents junk list` / `clear` | `--agent`, `--kind`, `--include review`, `--older-than`, `--session-days`, `--trash`, `--bytes`, `--items`, `--sort`, `--min-size`, `--yes` | per call (no key): what one run lists or removes; `--session-days` is `agents.junk.stale_session_days` for one run; `agents.junk.keep_logs_days`, `.temp_min_age_hours`, `.crash_dump_min_age_days`, `.stale_worktree_days`, `.exclude`, `.extra` have no flag |
| `agents usage` | `--source`, `--host`, `--since`, `--until`, `--daily` / `--monthly`, `--tz` | `agents.usage.source`, `.hosts`, `.since`, `.until`, `.period`, `.tz`, plus `.dirs.<host>` with no flag (`--unpriced` picks the view of one call, `--json` is the reading row) |
| `agents sessions` | `--all` | (action: also lists ended sessions; live vs idle follows `agents.idle`) |
| `agents show` | — | resolves an id prefix through the store (T284); live vs idle follows `agents.idle` (`--json` is the reading row) |
| `agents status` | — | writes the calling agent's (`RTOK_AGENT_ID`) status text, ≤ 120 chars (T284); no key |
| `agents send` | `--all-live` | per call (no key, no `setup.*`): one message to every live agent of the caller's project (T287); `[agents] idle` decides "live" |
| `agents inbox` | `--unread` | per call (no key, no `setup.*`): which rows one read shows (T287); `--json` is the reading row |
| `expand` | `--lines`, `--grep` (regex, literal fallback; hits print as `N:line`), `--context N` (lines around each grep hit, windows merged with `--`) | per call (no key); `expand.max_lines` caps; `expand.max_rate` is the report ceiling (T22.5) |
| `filter` | `--cmd` | `filter.cmd` |
| `config init`, `config set`, `memory import`, `graph index` | `--dry-run` | (action: renders the change as a git diff and writes nothing) |
| `memory export` | `--project` | per call (no key): narrows one dump to a project's notes |

The coverage test (T12.4) walks the clap command tree and fails if a non-positional flag
appears without a key in `crates/rtok-config/default.toml`, so this table cannot silently drift.

## Env var examples

```bash
RTOK_PROXY_MODE=compress rtok proxy
RTOK_PLUGINS_READ_ALLOW_PATHS=/opt/src,/srv/lib rtok mcp
RTOK_PLUGINS_WASM_ENABLED=true rtok config show --sources
RTOK_STATS_SINCE=7d rtok stats
RTOK_CONFIG=./ci-config.toml rtok bench --dry-run
```

## Debug log (`RUST_LOG`)

`[log]` is the operator's log: a file with a configured level, plus `logs` rows. For debugging
there is a second, stderr-only stream behind the `log` facade and `env_logger` (T225). It is off
until `RUST_LOG` is set, so no hook, `mcp` or `proxy` run prints anything new by default:

```bash
RUST_LOG=rtok=debug rtok stats                       # argv, then every [log] line as it is written
RUST_LOG=rtok::log=info rtok proxy                   # only the mirrored [log] stream
RUST_LOG=rtok=debug RUST_LOG_STYLE=never rtok mcp    # no colour; stdout stays the MCP channel
```

The mirror ignores `[log] level`: the file keeps `info`, stderr shows what `RUST_LOG` asks for.
The variable is `RUST_LOG`, not `RTOK_LOG`: `RTOK_<SECTION>_<KEY>` names belong to the
environment layer above, and `RTOK_LOG` would collide with the `[log]` table.

`just logs [flags]` opens the file in [tailspin](https://github.com/bensadeh/tailspin) (`tspin`,
pinned in `mise.toml`), which highlights levels, dates, numbers and paths; `just logs -f` follows
it. The debug stream pipes the same way: `RUST_LOG=rtok=debug rtok stats 2>&1 >/dev/null | tspin`.

`rtok logs` and `rtok logs watch` use tailspin themselves (T225.1). With `[log] tspin = "auto"`,
the default, the numbered rows go through `tspin --print` when stdout is a terminal and `tspin`
is on `PATH`; `"always"` does so on a pipe too, `"off"` keeps rtok's own colours. Without `tspin`
the output is what it was. `rtok logs export` and `--json` never go through it.

## Why one file and not flags-only

Hooks are spawned by the host with a fixed command line; the only way to tune them is a
file. The proxy and MCP server run for hours; restarting them to change a flag is a
regression. And a bench needs two complete, reproducible configurations — which is a file
per configuration, not a shell history.

## Legacy keys

`[dashboard]` folds into `[web]` (T21.3). `core.inject_budget_tokens` folds into
`plugins.inject.budget_tokens` (T12.1). `core.log_file` / `log_level` / `log_to_db` fold into
`[log].path` / `level` / `to_db` (T24.5, D26). Each prints one warning on load and is then
dropped; `rtok config validate` rejects them because they are absent from the reference schema.

`setup.restart_prompt_timeout_seconds` is retired (T138): the post-install restart question is an
`inquire` confirm that waits for an answer (Enter, Esc or Ctrl-C mean No). A file that still sets
it gets the unknown-key warning on load and an error from `rtok config validate`.
