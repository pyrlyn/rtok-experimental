# rtok Command Code plugin tree

The install source `rtok agents install commandcode` links from: rtok's hooks and the
stdio command of its MCP server (the installer registers the server itself, T275/D33).
Command Code documents no local plugin-bundle format (no `plugin.json` store like
Copilot's `installed-plugins/`), so this tree is the files the installer places, not
a bundle the host loads: `rtok-hook` is linked once per event, `mcp.sh` is the stdio
MCP command.

Install by hand:

- Link `hooks/rtok-hook` to one name per event Command Code fires (`PreToolUse`,
  `PostToolUse`, `SessionStart`, `Stop`) wherever the host reads its hook commands
  from, and put `{"command": "<link> …", "timeout": 5}` under the `hooks` key of
  `~/.commandcode/settings.json` (matchers: `SHELL|READ|WRITE|EDIT` on `PreToolUse`,
  `.*` on `PostToolUse`, none on lifecycle events).
- Register the MCP: `cmd mcp add rtok -- <path to scripts/mcp.sh>` (user scope).

`rtok` must be on `PATH` or under `~/.ketch/bin` (`ketch install pyrlyn/rtok`);
every hook fails open when it is missing (exit 0, empty stdout except the one
SessionStart note) and the MCP script exits 1 with the ketch hint.

MCP (T275/D33): the installer writes `mcpServers.rtok` in `~/.commandcode/mcp.json`
on every install and update, whether this tree is linked or not. The tree registers
nothing by itself: `scripts/mcp.sh` is only the command for a by-hand
`cmd mcp add rtok`, which writes the same `rtok` key, so the host still runs one server.

Files:

- `hooks/rtok-hook` (executable) — takes its event from its own file name and runs
  `rtok hook <event> --host commandcode`; with `rtok` missing it prints `{}`, exits 0
  and names `ketch install pyrlyn/rtok` on stderr.
- `scripts/mcp.sh` (executable) — `rtok mcp` with the same binary resolution.

## Docs

Host documentation this tree is written against. Re-check every link when it changes.

- Hooks (events, `settings.json` `hooks` key, matchers, stdin/stdout schema, exit codes): https://commandcode.ai/docs/hooks
- MCP (`cmd mcp`, scopes, `mcp.json` schema): https://commandcode.ai/docs/mcp
- Mods (the TypeScript extension surface; rtok ships hooks + MCP, not a mod): https://commandcode.ai/docs/mods
- Skills: https://github.com/CommandCodeAI/agent-skills
