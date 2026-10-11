# Command Code

`rtok agents install commandcode` — Command Code CLI (`command-code`, alias `cmd`), which reads
one user directory: `[setup.commandcode] dir` (default `~/.commandcode`). The desktop app reads
the same files, but its bundle path is undocumented, so it is not a separate variant. Setup
writes two files there: `mcp.json` (the user MCP scope, merged, every
other server survives) and the `hooks` key of `settings.json` (merged per event, foreign hook
definitions survive); `remove` takes back exactly rtok's own entries.

## Modules

| Module | Support | Why |
| --- | --- | --- |
| hooks | yes | the `hooks` key of `settings.json` runs `rtok hook <Event> --host commandcode` (`timeout: 5`) on PreToolUse (SHELL READ WRITE EDIT matchers), PostToolUse, SessionStart and Stop (no matcher on lifecycle events — a matcher there never fires) |
| mcp | yes | `mcpServers.rtok` → `rtok mcp --host commandcode` in the user-scope `mcp.json` as `{command, args: ["mcp", "--host", "commandcode"]}` (off with `[setup] mcp = false`) |
| proxy | no | Command Code has no documented base-URL override for the proxy |
| plugin | yes | the offer links `plugins/commandcode` to `~/.commandcode/plugins/rtok` — rtok never writes a host plugin store, there is none documented. The plugin does not replace the config entry: install and update write `mcpServers.rtok` whether the plugin is linked or not, and only `remove` takes it out (T275/D33) |

Command Code's hook protocol is Claude-shaped on the way out (`hookSpecificOutput` with
`permissionDecision`, `permissionDecisionReason`, `additionalContext`) but carries its own
tool names in: `shell_command`, `read_file` (`absolute_path`), `write_file` / `edit_file`
(`file_path`); the project root arrives as `COMMANDCODE_PROJECT_DIR`. `--host commandcode`
maps the names to Claude's `Bash` / `Read` / `Write` / `Edit`, so the plugins see what they
match on; the reply passes through unchanged.

## rtok plugins this host reaches

Hooks carry `hook` and `cli`, MCP carries `mcp`. Nothing carries `proxy`.

Reachable: measure, cmd, read, json_tree, archive, inject, guard, memory, graph, toon, docs
Not reachable: proxy, compress

## Docs

Host documentation setup writes against; re-check the links when this host changes.

- Hooks (events, `settings.json` `hooks` key, matchers, stdin/stdout schema, exit codes): https://commandcode.ai/docs/hooks
- MCP (`cmd mcp`, scopes, `mcp.json` schema): https://commandcode.ai/docs/mcp
- Mods (the TypeScript extension surface; rtok ships hooks + MCP, not a mod): https://commandcode.ai/docs/mods
- Skills: https://github.com/CommandCodeAI/agent-skills
