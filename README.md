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

macOS on Apple silicon, and Linux x86-64. The script is POSIX `sh`, so it behaves the
same whether your shell is bash or zsh; it puts `rtok` in `~/.cargo/bin`.

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/pyrlyn/rtok/releases/latest/download/rtok-installer.sh | sh
```

Or with [ketch](https://github.com/pyrlyn/ketch) — it installs straight from these
GitHub Release archives, no tap or formula:

```bash
curl -fsSL https://raw.githubusercontent.com/pyrlyn/ketch/main/install.sh | bash
ketch install pyrlyn/rtok
```

Or Homebrew (once the formula PR in [pyrlyn/homebrew-tap](https://github.com/pyrlyn/homebrew-tap)
has been merged for that release):

```bash
brew install pyrlyn/tap/rtok
```

Or from npm or PyPI, which carry the same native binary (macOS arm64, Linux x86-64 glibc,
Windows x86-64); no Node or Python code runs once it is installed. Both are published by hand,
so check the registry for the version you want ([docs/release.md](docs/release.md#npm-pypi-and-cratesio)):

```bash
npm i -g rtok-cli            # or: npx rtok-cli --help
uv tool install rtok-cli     # or: pipx install rtok-cli, uvx --from rtok-cli rtok --help
```

The package is `rtok-cli` on both registries; the command it installs is `rtok` (npm also
links `rtok-cli`).
Prefer a global install over `npx`/`uvx` before `rtok agents install`: hosts call `rtok` from
`PATH`, and a cached one-off copy can disappear.

The dist installer also gives you `rtok-update`; run it to move to the newest release.

macOS release binaries are codesigned with the Developer ID (notarisation is still off). A
browser download of a `.tar.xz` may still quarantine the file until Gatekeeper has checked it;
the installer above and `ketch install pyrlyn/rtok` do not set quarantine. Details:
[docs/release.md](docs/release.md).

Building from source works on any platform Rust supports:

```bash
git clone https://github.com/pyrlyn/rtok && cd rtok
mise install
mise exec -- cargo install --path .
rtok --version
```

Shell completions and the man page are generated from the same clap tree as
`--help`, so they never drift from the CLI surface:

```bash
rtok completions bash > ~/.bash_completion.d/rtok   # or zsh, fish, powershell, elvish
rtok completions clink > %LOCALAPPDATA%\clink\rtok.lua  # cmd.exe through Clink
rtok completions --install                           # to $SHELL's own directory; --uninstall undoes it
rtok completions                                     # in a terminal: pick the shells, checked = installed
rtok completions --list                              # shell, installed yes/no, file; for scripts
rtok man | man -l -                                  # or save as manpath/rtok.1
rtok man --dir ~/.local/share/man/man1               # rtok.1 plus rtok-<command>.1 for every subcommand
```

The picker lists every shell with its completions pre-checked when they are installed.
Space toggles, Enter installs the newly checked shells and removes the unchecked ones,
Esc or Ctrl-C changes nothing. Without a terminal, `rtok completions` with no shell fails
instead of waiting; use `--list` to read the state.

Release archives also carry them pre-built in `share/`: `share/man/man1/*.1` and
`share/completions/` (`rtok.bash`, `_rtok`, `rtok.fish`, `rtok.ps1`, `rtok.elv`, `rtok.lua`).

A shorter path (install → doctor → hooks) is also in
[`docs/getting-started.md`](docs/getting-started.md).

## Start with Claude Code

Install rtok's eight hooks and MCP entry. The installer backs up the settings file before
writing it; inspect its changes first if preferred.

```bash
rtok agents install claude --dry-run
rtok agents install claude
rtok doctor
```

`--dry-run` prints exactly what it would write and touches nothing:

```text
+ PreToolUse Bash rtok hook PreToolUse
+ PreToolUse Read rtok hook PreToolUse
+ PostToolUse * rtok hook PostToolUse
+ UserPromptSubmit rtok hook UserPromptSubmit
+ SessionStart rtok hook SessionStart
+ PreCompact rtok hook PreCompact
+ PostCompact rtok hook PostCompact
+ SessionEnd rtok hook SessionEnd
8 additions
```

Each app gets one block: kind and name (`CLI: Claude Code`, `Desktop: Cursor`),
install path and version, config files touched, and the state of every rtok
module (`hooks`, `mcp`, `proxy`, `plugin`) as `✓ installed`,
`✗ not installed (--flag)`, or `− not supported: why`. rtok's own plugins are
listed the same way from the surfaces each one declares (`(off)` when disabled).

`claude` covers both Claude Code and Claude Desktop (Desktop is MCP only, in
`claude_desktop_config.json`). A second run of the same command says
`already installed` instead of repeating a diff. `rtok agents list` prints the
same blocks without writing; `rtok doctor` lists modules for every host under
`agents`.

After upgrading rtok, `rtok agents update` refreshes every host rtok is
installed in (or only the hosts named): a hook or MCP entry written by an older
binary path or with an old timeout is rewritten in place, and a
host that has nothing of rtok is skipped rather than installed into.

Plugin versions and updates: [docs/plugin-versions.md](docs/plugin-versions.md)

`rtok agents uninstall claude` takes it all back out: hook entries, MCP
registration, and the proxy variable. Both install and uninstall copy every file
they touch to `_backup/<name>.bak-<ts>` first; a no-op run leaves no copy,
and identical content is never backed up twice. After a write, each requested
module is read back — a module that did not land is a warning, as is an
`rtok` that is not on `PATH`.

Run the proxy separately when you want provider usage rows and archive compression:

```bash
rtok proxy --mode passthrough
# In another shell, point the host at http://127.0.0.1:8790.
```

Two ways to shrink old tool results on the Anthropic wire, measured on the same
six-turn request (103 729 bytes upstream-bound; `tests/proxy.rs`
`proxy_compress_rewrites_old_tool_results_identically` and
`proxy_anthropic_context_edits_arm_platform_path`):

| path | upstream body | local rows |
| --- | --- | --- |
| `archive` (`mode = "compress"`) | 70 837 B (−31.7 %) | 4 `archive` Measurements, one per rewritten block |
| platform (`[proxy] context_management = true`) | 103 798 B (+69 B field) | 1 zero-delta `proxy/context_management` row naming the path; `archive` stands down (0 rows) |

The platform's own clearing happens server-side and is not locally observable, so by
D3 no saving is claimed for it — only the added field is measured. Keep `archive`
where the platform cannot clear (the OpenAI wires); where both apply, the platform
wins and `archive` skips those turns rather than shrinking twice.

## Examples

### Price the stack you already have

`rtok doctor` reads your host's settings and reports what every installed tool costs per
turn — before you install anything of rtok's. Abridged real output:

```text
hooks 88
  PreToolUse 14
  PostToolUse 12
  SessionStart 14
  UserPromptSubmit 9
  …
mcp
  code-review-graph (30 tools, ~2295 desc tokens)
  serena (22 tools, ~1494 desc tokens)
  lean-ctx (12 tools, ~697 desc tokens)
  rtok (11 tools, ~143 desc tokens)
proxy 8788→8787
mcp_tool_search likely disabled (ANTHROPIC_BASE_URL is set)
autoCompactWindow 300000
skills (68 listed, 12963 desc bytes ≈ 3240 tokens per request)
  design-is plugin:claude-mem desc 374c body 18403B calls - WARN desc>120 WARN body>8K …
  caveman-setup user desc 1c body 10304B calls - WARN body>8K (references/) …
```

Those description tokens are re-sent on every request of every session. That is the number
most tools do not count against themselves. The skills section (2026-09-18) prices the same
listing for the host's skills: 68 listed skills ride as ≈ 3.2 K description tokens per
request, and the `WARN` flags name the bodies that should live in `references/` instead of
`SKILL.md` — measured on this machine by the T61.3 audit.

### Run a command without paying for its output

```bash
rtok run -- cargo test
```

`rtok run` executes the command through your shell, keeps the exit code, writes the raw
output to `~/.rtok/archive/<id>`, and prints a per-family summary. Anything over 40 lines
gets a trailer naming its id:

```text
[rtok 7f3a91 · 412 lines · expand: rtok expand 7f3a91]
```

Nothing is redacted, a non-zero exit prints the last 80 lines verbatim, and the original is
one command away:

```bash
rtok expand 7f3a91                    # the whole thing
rtok expand 7f3a91 --lines 120-180    # just that range
rtok expand 7f3a91 --grep "panicked"  # just the matches, as N:line (regex)
rtok expand 7f3a91 --grep "panicked" --context 3  # matches with 3 lines around each, windows merged with --
rtok expand 7f3a91 --lines 210-230    # then the lines around hit 214
```

With the hooks installed this happens on its own: `PreToolUse` rewrites the agent's Bash
call to `rtok run -- <command>`.

### Ask the code graph instead of grepping

```bash
rtok graph index .
```

```text
indexed 5 files · 551 rows · 0 skipped · 5 read
```

The agent then reaches it over MCP as `symbol`, `callers`, `impact`, `outline`, `explore`,
`graph_diff` and `graph_export` — seven tools whose descriptions cost 165 tokens, in place of a grep-and-read chain. Each takes an
optional `project` (id or directory); without it a call answers for the working directory's
project and the projects it links to (`rtok graph projects link`). Definition
lookups are exact (recall and precision 1.000 over a hand-labelled set); reference lookups
find about a third of the sites, which [docs/comparison.md](docs/comparison.md) explains
rather than hides.

### See what is on and where it runs

```bash
rtok plugins
```

```text
id        enabled   surfaces
measure   on        cli,proxy
cmd       on        hook,cli
read      on        mcp,hook
json_tree off       proxy,mcp
archive   on        proxy,mcp,cli
proxy     on        proxy
inject    on        hook
guard     on        hook,cli
memory    on        mcp,hook
graph     on        mcp,hook
toon      on        proxy,mcp
compress  on        proxy
```

Turn one off with `rtok config set plugins.cmd.enabled false`, or turn `toon` off with
`rtok config set plugins.toon.enabled false` — same thing as a `[plugins.<id>]` table in the
config file.

### Find out where a setting came from

Every flag is a config key, and every key knows its layer:

```bash
rtok config show --sources
```

```text
core.db_path = ~/.rtok/rtok.db (user)
log.level = info (user)
web.port = 3333 (user)
estimator.code = 3.5 (user)
…
```

`rtok proxy --port 8791` and `[proxy] port = 8791` are the same setting reached two ways;
`--sources` says which one won.

### Send the ledger to your observability stack

```bash
rtok otel status
```

```text
endpoint: none (set [otel] endpoint or OTEL_EXPORTER_OTLP_ENDPOINT)
calls: mark 0 · 0 pending
logs: mark 0 · 0 pending
sessions: mark 0
```

Point `[otel] endpoint` at Jaeger, Grafana, SigNoz or Maple and `rtok otel flush` posts
calls, logs and metrics as OTLP/HTTP JSON — see [`docs/otel.md`](docs/otel.md).

## Measure before keeping a reduction

`rtok stats` reads transcript estimates and proxy usage. The proxy's provider-reported
usage is ground truth; transcript estimates use a fixed chars-per-token heuristic
(`[estimator]` in config) and are directional only — no calibration against a real
tokenizer has been measured yet, so treat them as a trend line, not a bound.

```bash
rtok stats --since 7d
rtok stats --save-baseline before-rtok
rtok stats --compare before-rtok
```

A fresh install has nothing to report yet, and says so rather than inventing a number:

```text
sessions 0  lines 0  malformed 0
usage input=0 cache_create=0 cache_read=0 output=0  hit=0.0%  median_context=0
```

What keeps that `hit=` high with rtok installed: [docs/prompt-cache.md](docs/prompt-cache.md).

Batch, Flex, and model routing on the proxy (pass-through vs rewrite, what is planned): [docs/batch-flex.md](docs/batch-flex.md).

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
| `rtok agents usage` | tokens and estimated cost per agent and month or day, from the agents' own session files (Claude Code, Codex, OpenCode, Kilo, Copilot CLI, Gemini CLI, pi, Kimi Code; Droid, Grok, ZCode and Antigravity are listed as unsupported) or what passed through rtok (`--tz`, `--since`, `--daily`, `--unpriced`, `--json`) |
| `rtok bench` | run the fixed A/B schedule |
| `rtok doctor` | inspect hooks, MCP servers and the proxy chain |
| `rtok worktree add <task-id> [<slug>] [--owner "<provider> / <model>"] [--agent <id>]` | create the task's worktree at `<root>/<repo>-<task-id>` on branch `<task-id>[-<slug>]` from a freshly fetched `origin/<default>`, locked with `<owner> \| <task-id> \| <date>[ \| agent <uuid>]`, no upstream; prints the path; binds it to the calling rtok agent (`--agent`, else `RTOK_AGENT_ID`) in the lock and the store, and `--owner` then defaults to `<host> / <model>`; refuses a second worktree for the same task or a root under a temp directory; the root is `[worktree] root`, `~/.rtok/worktrees` by default; `[worktree] enabled = false` turns every `rtok worktree` command into an error |
| `rtok worktree claim <path> [--agent <id>] [--owner <owner>]` | bind an existing worktree to the calling agent: rewrites its lock as v2 only when it has none or the lock is already the caller's (same agent, or an old lock naming `--owner`); never takes another owner's worktree |
| `rtok worktree adopt [<path>] [--task <id>] [--agent <id>] [--owner <owner>] [--json]` | bind the worktree you are in (made by a host's own tool: Cursor, Codex, Windsurf, Devin, Claude, Kilo, Conductor) to the calling agent; the task is `--task`, else the lock's, else the branch's first `-` segment; a worktree in a pool its host evicts (Cursor, Codex, Windsurf, Devin) is claimed in the store only, any other gets the v2 lock like `claim`; never takes another owner's worktree |
| `rtok worktree remove <path\|task-id> [--agent <id>] [--owner <owner>] [--keep-branch] [--json]` | remove the caller's own worktree: unlock, `git worktree remove` (never `--force`), delete the local branch when merged (squash-aware, against a freshly fetched base), release the claim, and print the `git push origin --delete` hint when a remote branch is left; refuses (exit 1) a worktree with uncommitted or untracked files, one locked by another owner or agent, and the one the command runs from; an unmerged branch is refused unless `--keep-branch`, which keeps it |
| `rtok worktree list` | every git worktree of the repository with its owner (the lock reason), bound agent (short id + host) and its state (`live` / `idle` / `ended`), else the newest session the hooks saw working there (`seen <host> <id8>`), state, origin (`main`, `cursor`, `codex`, `windsurf`, `claude`, `kilo`, `conductor` or `other`, from where it lives), source bytes and tagged build-cache bytes, plus orphans git no longer lists (`--json` with full agent ids); read-only |
| `rtok worktree whoami [--json]` | what to know before worktree work: your rtok agent (`RTOK_AGENT_ID`), the `[worktree] root` new worktrees go to, the linked worktree the cwd is in, and the worktrees of this repository bound to you (lock or claim); no size scans; read-only |
| `rtok worktree gc [--yes] [--owner <owner>] [--idle 24h]` | dry run by default; removes worktrees that are merged (squash-aware), clean and idle, deletes their local branch, and drops the record of a worktree whose directory was deleted by hand; a lock naming anyone but `--owner` is a hard stop, and so is a lock bound to a live rtok agent; nothing is forced |
| `rtok worktree clean [<path>…] [--yes] [--idle 24h]` | dry run by default; deletes build caches that carry a valid `CACHEDIR.TAG` and were idle for `--idle`, keeps the worktree and every untagged file; the worktree the command runs from is cleaned only when named; the one deletion `expand` cannot undo — a tagged cache holds no source and the next build recreates it |
| `rtok task init [--adapter disk\|github\|gitlab] [--prefix <letters>]` | write `[tasks]` into the checkout's `.rtok.toml` and print the next id; the prefix defaults to the project name's first letter |
| `rtok task create <title> [-d <text> \| --body-file <path\|->] [--parent <id>] [--json]` | add a task under the next free id (`A12`, or `A12.3` under `--parent`); ids come from a per-project counter in the store, raised past every id the adapter already holds, so parallel agents never collide |
| `rtok task list [--status <s,…>] [--all] [--parent <id>] [--json]` / `show <id>` | active tasks with subtasks indented; one task with its subtasks |
| `rtok task status <id> [<status>] [--force] [--json]` | read or set `open`, `in-progress`, `done` or `closed`; finishing a parent with active subtasks is refused unless `--force`; on disk, finished tasks move to `tasks/done/` |
| `rtok task next [--json]` / `ready [--json]` | the first ready task (with no blockers, still the lowest open task with no active subtask); every claimable task, highest priority first |
| `rtok task claim [id] [--agent <id>] [--json]` / `release <id> [--force]` | claim a task for this agent, or the first ready one; clear the assignee and set it open (only the holder, unless `--force`) |
| `rtok task dep <id> <blocker> [--json]` / `priority <id> <0-4>` | record that `id` waits on `blocker` (a cycle is refused); set priority, `0` highest, `2` the default and not stored |
| `rtok task export github\|linear [--dry-run] [--json]` | mirror the disk tasks one way into GitHub Issues (`[tasks.export.github] repo`) or Linear (`[tasks.export.linear] team`, key in `LINEAR_API_KEY`); the files stay the source of truth and a second run with nothing changed sends no write |
| `rtok task sync [--json]` | raise the store's id counters to the highest ids the adapter holds (never lower them) and report drift: ids above the counter, duplicate ids, issues that kept the `rtok` label but lost `rtok:<id>`; reads the adapter only |
| `rtok batch submit <file.jsonl> [--provider anthropic\|openai] [--url <proxy>]` | create a provider Batch job from a file of provider-shaped requests through `rtok proxy` (the JSONL is sent as written; nothing is converted from a live request); prints the batch object; the key comes only from `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` |
| `rtok batch status <id>` / `fetch <id> <out>` | the provider's state of a batch; its results written to a new file (an existing file is never overwritten); the provider is read from the id (`msgbatch_` is Anthropic) unless `--provider` says so; the proxy tags every call to the `batch` lane, and `stats --price` costs that usage at the `<model>@batch` price row |
| `rtok run -- <cmd>` | run, archive, and format a command result |
| `rtok filter --stdin` | filter a payload without executing it (OpenCode) |
| `rtok expand <id>` | retrieve an archived original (`--lines`, `--grep`) |
| `rtok plugins` | list plugins: id, enabled, surfaces |
| `rtok config show --sources` | show effective configuration and its source |
| `rtok graph index [path]` | build the symbol index for a tree |
| `rtok graph projects` | list the registered projects with their index status (`add`, `select`, `remove`, `link` and `unlink` change the registry) |
| `rtok memory import <file>` | import notes as JSONL |
| `rtok memory export [--project <name>]` | print notes as the JSONL `import` reads; session checkpoints stay behind |
| `rtok memory retire <id> [--superseded-by <id>]` | tombstone a note: never recalled or searched, body kept |
| `rtok memory pin / unpin <id>` | keep a note at the head of SessionStart recall, or drop it back |
| `rtok memory revise <id> --title <t> --body <b>` | save a replacement note and retire the old one |
| `rtok memory history <id>` | earlier title and body kept when an upsert changed the note |
| `rtok otel flush` / `status` | export the ledgers over OTLP, or report the watermarks |

Agent ids, messages between agents and how worktrees bind to them on every host: [docs/agents-and-worktrees.md](docs/agents-and-worktrees.md).

Numbered tasks shared by every checkout and agent of a project, stored on disk (`rtok task`, MCP `task_*`): [docs/tasks.md](docs/tasks.md).

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

Ten tools that each shrink one slice of the context, stacked on one machine, produce a stack
nobody measures end to end. The author's machine before rtok: 88 hook entries, nine MCP
servers, ~8 600 tool-description tokens on every turn, two chained proxies — and every one of
those tools reported a saving while the bill did not move.

| | The field | rtok |
|---|---|---|
| Processes per tool call | up to ~30 subprocesses, several Python | one Rust process, p95 8.25 ms (research.md §2, Gate P17 serialized run 2026-09-09) |
| Hook entries | 88 across 16 events | 8 across 7 events |
| MCP description tokens/turn | ~8 600 across nine servers | ~143 across 11 tools |
| Injection per turn | 3.1 K and up, per tool | one 800-token budget, byte-stable |
| Reversibility | partial | every rewrite has `rtok expand <id>` |
| Measurement | five meters, none of them the bill | one ledger + provider `usage` |
| Failure mode | varies | fail open: exit 0, unmodified input, ≤ 10 ms |

The other side of it: rtok has no live end-to-end cost win yet, its reference lookups find
0.351 of the sites where serena's LSP finds all of them, and it has no LLM compression or
embeddings. Tool-by-tool detail, evidence for every number, and the cases where you should
use something else: **[docs/comparison.md](docs/comparison.md)**.

## Measured results

The committed T9.2 A/B harness schedules six tasks × three runs. Its first run was offline
(`RTOK_BENCH_LIVE` was unset), so both configurations have zero provider usage and equal
task checks. This is a reproducible baseline, **not** evidence of a bill reduction; rerun it
with live traffic before adopting a configuration.

| config | mean input | mean cache | mean output | mean cost USD | passed |
|---|---:|---:|---:|---:|---:|
| A — legacy baseline | 0 | 0 | 0 | 0.0000 | 6/6 |
| B — rtok | 0 | 0 | 0 | 0.0000 | 6/6 |
| delta (B − A) | 0 | 0 | 0 | 0.0000 | 0 |

Sources: [`bench/results/a.json`](bench/results/a.json),
[`bench/results/b.json`](bench/results/b.json), and [`research.md`](research.md).

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
just check
just example
just readme-check
just dist-plan
```

`just check` is the gate for what the change touches: it diffs against the merge base with
`main` (committed and uncommitted) and runs format, lint, the copy-paste, JS and Python checks
and the tests only for that diff. A docs-only change runs no cargo tests; a change to
`Cargo.toml`, `build.rs`, `justfile`, `crates/` or `config/` runs everything. `just full-check`
is the whole gate regardless of the diff, and CI runs the whole gate too.

The selection is `just test-changed`, which also works on its own against any revision. It
builds and runs only the test targets the diff can reach, which is what makes the loop short:
each file under `tests/*.rs` is its own integration-test binary, and cargo links every selected
one before any test runs. The unit-test binary is trimmed the same way, which keeps the slow
TUI tests out of an unrelated edit. It picks targets by name, so it can miss a test that
exercises a module without naming it — run `just full-check` when that matters.

Packaging for npm, PyPI and crates.io is manual and local; no workflow publishes. Build and
try the npm package for this machine without touching a registry:

```bash
just npm-build                                   # target/npm/dist/rtok-cli-*.tgz
tmp=$(mktemp -d) && cd "$tmp" && npm init -y >/dev/null
npm i <repo>/target/npm/dist/rtok-cli-*.tgz      # rtok-cli + the platform tarball
npx rtok --version
```

`just pypi-build` does the same for a wheel (`uv venv && uv pip install target/pypi/dist/*.whl`),
and every `*-publish` recipe takes `--dry-run`. The full steps are in
[docs/release.md](docs/release.md#npm-pypi-and-cratesio).

The README smoke examples below are executed by `just readme-check`.

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

Read [`plan.md`](plan.md) for the current implementation plan, [`done.md`](done.md) for
completed tasks, and [`docs/plugin-authoring.md`](docs/plugin-authoring.md) to build an
external plugin.

## License

You can use this project under **any** of the following licenses, at your choice:

1. [GNU GPLv3](LICENSE): free for open source applications on any platform, including embedded systems.
2. [Royalty-free License](LICENSE-ROYALTY-FREE.md): free for proprietary desktop, mobile, and web applications, as long as you disclose that your application uses this project. Embedded systems are not covered.
3. [Commercial license](PRICING.md): for proprietary applications, including embedded systems, without the attribution requirement.

<!-- license-sync:start -->
Commercial use not covered by the GPLv3 or the Royalty-free License requires a separate paid
license — see [PRICING.md](PRICING.md).
<!-- license-sync:end -->
