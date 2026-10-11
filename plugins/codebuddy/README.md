# rtok CodeBuddy Code / WorkBuddy plugin

One plugin tree for both Tencent hosts: CodeBuddy Code (CLI) and WorkBuddy (desktop), which embeds
the same plugin system. Part of the [host plugin packages](../README.md); the installer
(`rtok agents install codebuddy`) is the follow-up task T560.2, so for now install by hand:

```bash
codebuddy plugin marketplace add <path to this directory> --name rtok
codebuddy plugin install rtok@rtok -s user
```

Remove with `codebuddy plugin uninstall rtok@rtok -y`.

Files:

- `.codebuddy-plugin/plugin.json` — manifest (`name` `rtok`). CodeBuddy reads `.codebuddy-plugin/`
  before `.workbuddy-plugin/` and `.claude-plugin/`, so this one directory serves both hosts.
- `.codebuddy-plugin/marketplace.json` — this directory's own one-plugin marketplace (`rtok`,
  plugin `rtok`, source `./`).
- `hooks/hooks.json` — the rows `agents::hook_events` lists for `codebuddy` (checked by
  `tests/hook_manifests.rs`): PreToolUse on `Bash`, `Read` and `Skill`, PostToolUse `*`,
  UserPromptSubmit, SessionStart, PreCompact, PostCompact, SessionEnd; `timeout` 5 s. No
  SubagentStart: CodeBuddy's hooks page only names it, with no input or output shape. The command
  runs `rtok-hook` then `rtok hook` from PATH, each with `--host codebuddy`, and falls back to
  `scripts/hook.sh` only when PATH has neither.
- `scripts/hook.sh` — resolves `rtok` from PATH or the ketch store; a missing `rtok` fails the hook
  open (exit 0), printing `ketch install pyrlyn/rtok` once on SessionStart.
- No `.mcp.json` (D33): MCP is the `mcpServers.rtok` entry `rtok agents install codebuddy` writes into the host's own MCP file, so the plugin never adds a second rtok server.
- `.rtok-plugin-version` — the version stamp `tools/plugin-versions.sh` keeps equal to `Cargo.toml`.

`--host codebuddy` changes only the output: stdin is Claude's shape, but CodeBuddy's PreToolUse
reads a rewrite from `hookSpecificOutput.modifiedInput` where Claude says `updatedInput`
(`codebuddy_output` in `src/hooks/mod.rs`). Decisions and `additionalContext` pass through.

`${CODEBUDDY_PLUGIN_ROOT}` is the plugin's cache directory; `scripts/hook.sh` is POSIX shell, so
on Windows it needs Git Bash on PATH.

## Docs

- Plugins (layout, manifest directories, `${CODEBUDDY_PLUGIN_ROOT}`): https://www.codebuddy.ai/docs/cli/plugins-reference
- Marketplaces (`marketplace.json`, `codebuddy plugin marketplace add`): https://www.codebuddy.ai/docs/cli/plugin-marketplaces
- Hooks (events, stdin, `hookSpecificOutput.modifiedInput`): https://www.codebuddy.ai/docs/cli/hooks
- MCP (`.mcp.json`, `mcpServers`): https://www.codebuddy.ai/docs/cli/mcp
- WorkBuddy documents no plugin or hook surface of its own (https://www.codebuddy.ai/docs/workbuddy/Changelog, checked 2026-10-11).
