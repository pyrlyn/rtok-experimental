# CodeBuddy Code and WorkBuddy

`rtok agents install codebuddy` — CodeBuddy Code CLI (`codebuddy`) and the WorkBuddy desktop
app, which embeds the same CLI. Both read one file layout under a config directory:
`[setup.codebuddy] dir` for the CLI (default `~/.codebuddy`, what `$CODEBUDDY_CONFIG_DIR`
moves) and `workbuddy_dir` for WorkBuddy (default `~/.workbuddy`). The desktop variant counts
as present when `/Applications/WorkBuddy.app`, `~/Applications/WorkBuddy.app` or
`%LOCALAPPDATA%\Programs\WorkBuddy` exists.

Without the plugin, setup writes two files per directory and touches only its own entries:
the `hooks` key of `settings.json` (Claude's layout, `timeout` in seconds) and
`mcpServers.rtok` in the first existing of `.mcp.json`, `mcp.json` and, for the default CLI
directory, the legacy `~/.codebuddy.json` (none exists: `.mcp.json` is created). The MCP file
is edited as JSONC, so comments and trailing commas survive.

## Modules

| Module | Support | Why |
| --- | --- | --- |
| hooks | yes | `hooks` in `settings.json` runs `hook <Event> --host codebuddy` on PreToolUse (Bash, Read, Skill), PostToolUse, UserPromptSubmit, SessionStart, PreCompact, PostCompact, SessionEnd (timeout 5); each line resolves `rtok` from PATH then `~/.ketch/bin/rtok`, else fails open |
| mcp | yes | `mcpServers.rtok` → `rtok mcp --host codebuddy` as `{type: "stdio", command, args}` (off with `[setup] mcp = false`) |
| proxy | no | CodeBuddy Code takes its endpoint from the CODEBUDDY_BASE_URL environment variable; there is no config file to point at the proxy |
| plugin | `--yes` | install runs `codebuddy plugin marketplace add <resolved plugins/codebuddy> --name rtok` then `codebuddy plugin install rtok@rtok -s user`; remove runs `codebuddy plugin uninstall rtok@rtok -s user -y`. WorkBuddy runs the same calls with `CODEBUDDY_CONFIG_DIR=<workbuddy_dir>` (unverified, see below). rtok never writes `plugins/`, that store is CodeBuddy's. While `plugins/installed_plugins.json` names `rtok@rtok`, setup takes back the `settings.json` hooks instead of adding them (D21); `mcpServers.rtok` is independent of the plugin (T275/D33) |

Both variants need the `codebuddy` CLI for the plugin. With WorkBuddy present and no
`codebuddy` on PATH the offer says so and the own files in `workbuddy_dir` are written
instead. A plugin install is reported as `already installed` on a repeat and runs nothing.

Unverified, from a third-party report (https://github.com/lorelum/lorelum/issues/205, WorkBuddy
5.5.6 on Windows) and absent from WorkBuddy's own docs: the app keeps its plugin registry in
`~/.workbuddy`, so `CODEBUDDY_CONFIG_DIR=~/.workbuddy codebuddy plugin …` installs into it, and
it reads hooks and MCP from the same file names as the CLI. Not verified on a live install:
`installed_plugins.json`'s schema (setup looks for the quoted `rtok@rtok` id) and the WorkBuddy
app paths.

`--host codebuddy` answers PreToolUse rewrites as `hookSpecificOutput.modifiedInput`, the key
CodeBuddy reads instead of Claude's `updatedInput` (T560.1). SubagentStart is not registered:
CodeBuddy's hooks page names the event without an input or output shape.

## rtok plugins this host reaches

Hooks carry `hook` and `cli`, MCP carries `mcp`. Nothing carries `proxy`.

Reachable: measure, cmd, read, json_tree, archive, inject, guard, memory, graph, toon, docs
Not reachable: proxy, compress

## Docs

Host documentation setup writes against; re-check the links when this host changes.

- Hooks (`settings.json` `hooks`, events, `modifiedInput`): https://www.codebuddy.ai/docs/cli/hooks
- MCP (`.mcp.json`, `mcp.json`, `~/.codebuddy.json`, `mcpServers`): https://www.codebuddy.ai/docs/cli/mcp
- Plugins (`codebuddy plugin`, `installed_plugins.json`, the cache): https://www.codebuddy.ai/docs/cli/plugins-reference
- Plugin marketplaces (`marketplace add`): https://www.codebuddy.ai/docs/cli/plugin-marketplaces
- Environment variables (`CODEBUDDY_CONFIG_DIR`, `CODEBUDDY_BASE_URL`): https://www.codebuddy.ai/docs/cli/env-vars
- WorkBuddy changelog (no plugin or hook docs as of 5.7.6): https://www.codebuddy.ai/docs/workbuddy/Changelog
