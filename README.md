> [!NOTE]
> This project is under active development. We need "testing volunteers": try it and report what breaks.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/logo-wordmark-dark.svg">
  <img src="assets/logo-wordmark.svg" alt="rtok" width="200">
</picture>

[![Quality Gate Status](https://sonarcloud.io/api/project_badges/measure?project=listepo_rtok&metric=alert_status)](https://sonarcloud.io/summary/new_code?id=listepo_rtok) [![Coverage](https://sonarcloud.io/api/project_badges/measure?project=listepo_rtok&metric=coverage)](https://sonarcloud.io/component_measures?id=listepo_rtok&metric=coverage) [![Tests](https://img.shields.io/sonar/tests/listepo_rtok?server=https%3A%2F%2Fsonarcloud.io&compact_message)](https://sonarcloud.io/component_measures?id=listepo_rtok&metric=tests)

`rtok` reduces the context that AI coding agents must carry. It is one Rust binary with
three surfaces: Claude Code hooks, an MCP server, and an API proxy. Each reduction is
measured, and shortened payloads stay retrievable by id.

Ten methods, one process, one ledger. A saving that is not a row in that ledger does not
exist — including rtok's own.

## Install

macOS on Apple silicon, and Linux x86-64:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/pyrlyn/rtok/releases/latest/download/rtok-installer.sh | sh
```

Or with [ketch](https://github.com/pyrlyn/ketch), Homebrew, npm or PyPI (the npm and PyPI
package is `rtok-cli`; the command is `rtok`):

```bash
ketch install pyrlyn/rtok
brew install pyrlyn/tap/rtok
npm i -g rtok-cli
uv tool install rtok-cli
```

Building from source, codesigning notes, shell completions and man pages:
[docs/install.md](docs/install.md). First steps: [docs/getting-started.md](docs/getting-started.md).

## Start with Claude Code

```bash
rtok agents install claude --dry-run   # print what would be written, touch nothing
rtok agents install claude             # eight hooks and the MCP entry; settings backed up first
rtok doctor
```

`rtok agents update` refreshes installed hosts after an upgrade; `rtok agents uninstall claude`
takes everything back out. Details and the proxy: [docs/claude-code.md](docs/claude-code.md).
Other hosts (Cursor, Codex, OpenCode, Zed, VS Code and more): [docs/agents.md](docs/agents.md).

Worked examples — pricing your current stack, `rtok run`, the code graph, plugins, config
sources and OpenTelemetry export: [docs/examples.md](docs/examples.md). How savings are
measured: [docs/measuring.md](docs/measuring.md).

## Commands

| Command | Purpose |
|---|---|
| `rtok agents install claude` | install Claude Code hooks and MCP registration (`--dry-run`) |
| `rtok agents uninstall claude` | take hooks, MCP registration and proxy variable back out (`--dry-run`) |
| `rtok agents update [claude,…]` | bring every installed host (or the named ones) up to date after an rtok upgrade: stale hooks, MCP command, proxy URL and plugin links rewritten in place, the rest reinstalled (`--dry-run`) |
| `rtok agents install cursor` / `codex` / `opencode` / `kilo` / `pi` / `zcode` / `kimi` / `copilot` / `aider --proxy` / `windsurf` / `zed` / `vscode` | register the other supported host integrations |
| `rtok hook <event>` | hook entry point (JSON on stdin, JSON on stdout) |
| `rtok mcp` | serve read, memory, graph, and expansion tools over stdio |
| `rtok mcp -- <server argv>` | wrap a foreign stdio MCP server: long `tools/call` text blocks are archived and cut by the `[mcp]` rule, everything else passes byte-for-byte, `rtok expand <id>` returns the raw block |
| `rtok proxy` | capture API usage; optionally archive older tool results |
| `rtok web` | local React UI (embedded in the binary) + WebSocket API at `http://127.0.0.1:3333` (default `[web] host`/`port`; `--host`, `--port`; `rtok dashboard` is the deprecated spelling). Open it as `127.0.0.1`/`localhost`: `/ws` refuses cross-site and DNS-name origins |
| `rtok stats` | report transcript and proxy measurements |
| `rtok discover` | Bash commands no filter covers yet, ranked by the output tokens they sent to the model |
| `rtok agents usage` | tokens and estimated cost per agent and month or day, from the agents' own session files (Claude Code, Codex, OpenCode, Kilo, Copilot CLI, Gemini CLI, pi, Kimi Code; Droid, Grok, ZCode and Antigravity are listed as unsupported) or what passed through rtok (`--tz`, `--since`, `--daily`, `--unpriced`, `--json`) |
| `rtok bench` | run the fixed A/B schedule |
| `rtok doctor` | inspect hooks, MCP servers and the proxy chain |
| `rtok worktree add` / `claim` / `adopt` / `list` / `remove` / `gc` / `clean` / `migrate` | per-task git worktrees bound to agents — see [docs/agents-and-worktrees.md](docs/agents-and-worktrees.md) |
| `rtok task init` / `create` / `list` / `status` / `next` / `claim` / `export` / `sync` | numbered tasks shared by every checkout and agent — see [docs/tasks.md](docs/tasks.md) |
| `rtok batch submit` / `status` / `fetch` | provider Batch jobs through `rtok proxy` |
| `rtok run -- <cmd>` | run, archive, and format a command result |
| `rtok filter --stdin` | filter a payload without executing it (OpenCode) |
| `rtok expand <id>` | retrieve an archived original (`--lines`, `--grep`) |
| `rtok plugins` | list plugins: id, enabled, surfaces |
| `rtok config show --sources` | show effective configuration and its source |
| `rtok graph index [path]` | build the symbol index for a tree |
| `rtok graph projects` | list the registered projects with their index status (`add`, `select`, `remove`, `link` and `unlink` change the registry; `reindex` re-reads a project's files into its index) |
| `rtok memory import` / `export` / `retire` / `pin` / `revise` / `history` | manage agent-written notes |
| `rtok otel flush` / `status` | export the ledgers over OTLP, or report the watermarks |

Every command with flags and examples: [docs/commands.md](docs/commands.md).

## Plugins

| Plugin | Surface | What it does |
|---|---|---|
| `measure` | stats, bench, proxy | records before/after tokens and provider usage |
| `cmd` | PreToolUse Bash | wraps commands, formats output, and archives originals |
| `read` | MCP, PreToolUse Read | provides bounded reads, searches, trees, and re-read deduplication |
| `archive` | proxy, expand | replaces eligible old tool results with stable archive pointers |
| `proxy` | API proxy | passes traffic through and captures usage |
| `inject` | SessionStart, UserPromptSubmit | emits byte-stable context within a token budget |
| `guard` | PreToolUse | prevents repeated reads and commands within a turn window |
| `memory` | MCP, PreCompact | stores agent-written notes with progressive disclosure; FTS5 plant-and-recall 20/20 (`research.md` §14) |
| `graph` | MCP | indexes symbols and references with bounded responses |
| `toon` | proxy, MCP | encodes old tabular JSON tool results when that is smaller; on by default |

Plugin details and configuration live in [`docs/config.md`](docs/config.md),
[`architecture.md`](architecture.md), and each `src/plugins/<id>/README.md`. Every call rtok
records can be exported as OpenTelemetry traces, logs and metrics to Jaeger, Grafana, SigNoz or
Maple — see [`docs/otel.md`](docs/otel.md).

## How it compares

rtok replaces a stack of separate hooks, MCP servers and proxies with one process and one
ledger. It has no live end-to-end cost win yet, and its reference lookups find fewer sites than
an LSP. Tool-by-tool detail and evidence: **[docs/comparison.md](docs/comparison.md)**.
For prompt-rules packs such as ponytail, see
[rtok vs ponytail and other prompt-level tools](docs/comparison.md#1a-rtok-vs-ponytail-and-other-prompt-level-tools).

## Guarantees and caveats

- Hooks fail open: errors return unmodified input and exit successfully.
- Archive and proxy reductions are lossless: `rtok expand <id>` retrieves the original
  payload. A regenerable command result may instead be re-run.
- A token saving only counts when a `Measurement` row records it.
- Proxy compression preserves the cached prefix and never rewrites system instructions,
  tool definitions, or the newest tool-result turns.
- Estimates are a heuristic (chars per token) until matched with provider usage; no accuracy
  figure is measured yet. No live A/B cost reduction has been established yet.

## Development

```bash
just check        # format, lint and tests for what the diff touches
just full-check   # the whole gate, as CI runs it
just readme-check # runs the smoke block below
```

Test selection, manual npm/PyPI packaging and the rest: [docs/development.md](docs/development.md).
Release steps: [docs/release.md](docs/release.md). Writing an external plugin:
[docs/plugin-authoring.md](docs/plugin-authoring.md).

```bash
# check
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
rtok() {
  RTOK_HOME="$tmp/home" \
  RTOK_STATS_TRANSCRIPTS_DIR="$tmp/transcripts" \
  RTOK_SETUP_CLAUDE_SETTINGS_PATH="$tmp/settings.json" \
  mise exec -- cargo run -q -- "$@"
}
rtok --version
rtok config init
rtok config validate
rtok agents install claude --dry-run
rtok stats --since 1h
rtok plugins
rtok proxy --dry-run
rtok otel status
```

## License

You can use this project under **any** of the following licenses, at your choice:

1. [GNU GPLv3](LICENSE): free for open source applications on any platform, including embedded systems.
2. [Royalty-free License](LICENSE-ROYALTY-FREE.md): free for proprietary desktop, mobile, and web applications, as long as you disclose that your application uses this project. Embedded systems are not covered.
3. [Commercial license](PRICING.md): for proprietary applications, including embedded systems, without the attribution requirement.

<!-- license-sync:start -->
Commercial use not covered by the GPLv3 or the Royalty-free License requires a separate paid
license — see [PRICING.md](PRICING.md).
<!-- license-sync:end -->
