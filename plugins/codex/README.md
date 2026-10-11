# rtok Codex plugin

One plugin directory for Codex (the `codex` CLI and the Codex desktop app): rtok's hooks (D21).
The layout is Codex's `.codex-plugin/plugin.json` manifest pointing at `hooks/hooks.json`. The
plugin no longer ships an MCP server (T275/D33): `rtok agents install codex` always writes
`[mcp_servers.rtok]` into `config.toml` itself, plugin enabled or not, so there is exactly one
path to that entry instead of two copies to keep in sync.

Install:

- `rtok agents install codex` — the automated path (T140): once `codex` is on PATH it runs
  `codex plugin marketplace add pyrlyn/rtok` against this repo's root marketplace
  (`.agents/plugins/marketplace.json`, one entry `rtok` at `./plugins/codex`), then
  `codex plugin add rtok@rtok`, by default — no flag needed. `rtok agents remove codex` reverses
  it (`codex plugin remove rtok@rtok` then `codex plugin marketplace remove rtok`). A missing or
  failing `codex` fails open onto the file-based hooks/MCP surfaces below instead of failing the
  install.
- By hand, from a local checkout: `codex plugin marketplace add <path to this folder>` — the
  folder is also its own local marketplace (same file, one entry `rtok` at `./`) — then
  `codex plugin add rtok@rtok`. The plugin is enabled on install; Codex asks to trust its hooks
  before they run. Remove: `codex plugin remove rtok@rtok`, then
  `codex plugin marketplace remove rtok`.

The hooks resolve `rtok` from `PATH`, then `~/.ketch/bin/rtok`, else exit 0 silently (Codex only
blocks on an explicit decision) — so a hook shell whose `PATH` lacks ketch's install dir still
finds `rtok`. `commandWindows` keeps the bare `rtok hook <event>` for `cmd.exe`, which cannot run
the POSIX fallback. Install with ketch: `ketch install pyrlyn/rtok`.

While the plugin is enabled it is the only path for the compaction hooks (D21): `rtok agents
install codex` takes its own `~/.codex/hooks.json` copy back instead of adding it, so every event
fires once. `[mcp_servers.rtok]` is independent of the plugin (T275/D33) and is written on every
install/update regardless. It carries `default_tools_approval_mode = "writes"` (T528): Codex runs
rtok's read-only tools (marked `readOnlyHint` in `tools/list`) without asking, and still asks
before a tool that changes a worktree, a task, a note or another agent's inbox. Without the key it
asks before every call, and in code mode, where nobody can answer, the call fails. An existing rtok
entry gains the key once; a value you set yourself is kept.

Files:

- `.codex-plugin/plugin.json` — manifest (name, version, metadata, `hooks` path).
- `hooks/hooks.json` — `PreCompact` and `PostCompact`, each `timeout: 5`, resolving `rtok` from
  `PATH` then `~/.ketch/bin/rtok` (`commandWindows` bare for `cmd.exe`): the same events
  `rtok agents install codex` registers. Checked by `tests/codex_plugin.rs`.
- `.agents/plugins/marketplace.json` — the local marketplace that lists this folder.

Known limits:

- Only the compaction hooks. Codex's `PreToolUse` can rewrite a Bash command (`updatedInput`) and
  `PostToolUse` can add context, but rtok's handling of Codex's tool payloads has not been checked
  on a live session, so those events stay off here and in the installer until it is.
- Verified on codex-cli 0.155.1 in a scratch `CODEX_HOME` (2026-09-21): `marketplace add` accepts
  the `./` entry and `plugin add` copies the tree to `plugins/cache/rtok/rtok/0.0.1/` and enables
  it. Not verified: the hooks firing in a live session (trust prompt).

## Docs

Host documentation this plugin is written against. Re-check every link when the plugin changes.

- Plugins (layout, `.codex-plugin/plugin.json`, `hooks/hooks.json`, marketplaces, `codex plugin marketplace add`): https://developers.openai.com/plugins/build/plugins
- Hooks (events, `matcher`, `timeout` in seconds, plugin-bundled hooks and trust review, `PLUGIN_ROOT`): https://learn.chatgpt.com/docs/hooks
- MCP (`[mcp_servers.<name>]`, stdio servers, written by `rtok agents install codex` itself): https://learn.chatgpt.com/docs/extend/mcp
- Config reference (`~/.codex/config.toml`): https://learn.chatgpt.com/docs/config-file/config-reference

## Package docs

- Agents working on this package: [`AGENTS.md`](AGENTS.md)
- Installer and surface table: [`../../src/agents/codex/README.md`](../../src/agents/codex/README.md)
- All host plugins: [`../README.md`](../README.md)
- Agent rules for `plugins/`: [`../AGENTS.md`](../AGENTS.md)
