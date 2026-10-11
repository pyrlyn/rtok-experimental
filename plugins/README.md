# Host plugins

Packaged integrations that wire **rtok** into a host IDE/CLI: hooks, MCP, and (where the host allows) a single plugin unit (D21: one `rtok mcp` per store).

This folder is **not** `src/plugins/` (rtok’s own reduction methods: guard, archive, memory, …). Those are Rust crates inside the binary. Here each subdirectory is a **host marketplace / local plugin package** that the matching `rtok agents install <host>` path links or copies.

## When to use what

| Path | Role |
| --- | --- |
| `plugins/<host>/` | Host-facing package (manifest + hooks/MCP/scripts or a TS extension) |
| `src/agents/<host>/` | Installer / doctor / remove logic that edits the host’s config |
| `src/hooks/`, `rtok hook` | Shared hook I/O used by many hosts |
| `rtok mcp` | Shared MCP server |

## Claude Code is not here on purpose

There is **no** `plugins/claude/`. Claude Code and Claude Desktop load hooks and MCP from their own settings files; they do not expose a plugin directory to link. Integration is:

- `rtok agents install claude` → `src/agents/claude/`
- hooks via `rtok hook <event>` in `settings.json`
- MCP in `~/.claude.json`
- optional proxy via `ANTHROPIC_BASE_URL`

See [`src/agents/claude/README.md`](../src/agents/claude/README.md) and [`docs/agents.md`](../docs/agents.md). Do not invent a `plugins/claude` package that cannot install.

## Writing a new host plugin

1. **Read an existing peer** closest to the host’s model:
   - Manifest + hooks.json (no MCP: T275) → start from `cursor/` or `zcode/`; a plugin that
     does ship its own MCP → `gemini/` or `grok/`
   - Single JSON manifest (hooks only, no MCP: T275) → `kimi/`
   - TypeScript extension API → `opencode/` or `pi/`
   - MCP-only (hooks cannot rewrite) → `antigravity/`
2. **Keep D21**: one unit that owns hooks and MCP together when the host supports both; one `rtok mcp` process per store.
3. **Prefer `rtok hook` / `rtok mcp` / `rtok guard check` / `rtok filter`** over reimplementing logic in the package.
4. **Fail open** when `rtok` is missing (unless the host only blocks on a specific exit code); print `ketch install pyrlyn/rtok`.
5. **Wire install** in `src/agents/<host>/` and document surfaces in that agent’s README table.
6. **Add tests** the way peers do (`*_plugin.rs`, manifest parity tests, or Node tests under the package).
7. **Ship docs** — every package **must** have `README.md` (humans) and `AGENTS.md` (agents). See [`AGENTS.md`](AGENTS.md).

### Hooks

- Prefer the shared CLI: `rtok hook <Event>` (add `--host <name>` when the host envelope needs it).
- Match the host’s event names and timeout field (`timeout` seconds vs `timeoutMs`).
- Do not duplicate deny/rewrite policy in shell wrappers; call rtok.

### MCP

- Default server entry: command that runs `rtok mcp` (absolute bin from install, or a small launcher script that resolves PATH / ketch store).
- Launchers (`scripts/mcp.sh`, `scripts/mcp.cmd`) should stay thin; host-specific path expansion (e.g. `${ZCODE_PLUGIN_ROOT}`) belongs in that host’s scripts, not a forced shared copy.

### RTOK surfaces the package usually touches

`hook`, `mcp`, sometimes `guard check`, `filter`, `run`, `archive` — declared per host in `src/agents/<host>/README.md`.

## Packages in this tree

| Directory | Host | Shape |
| --- | --- | --- |
| [`antigravity/`](antigravity/) | Antigravity | MCP-only plugin dir |
| [`codebuddy/`](codebuddy/) | CodeBuddy Code + WorkBuddy | `.codebuddy-plugin` manifest + `hooks/hooks.json` (`--host codebuddy`); MCP from the host file (D33) |
| [`codex/`](codex/) | Codex (CLI + app) | `.codex-plugin` manifest + hooks + local marketplace (no MCP: T275) |
| [`commandcode/`](commandcode/) | Command Code | hooks + MCP launcher (no plugin-bundle format) |
| [`copilot/`](copilot/) | GitHub Copilot CLI | legacy `plugin.json` + camelCase hooks (no MCP: T275) |
| [`cursor/`](cursor/) | Cursor | hooks + launchers (no MCP: T275) |
| [`devin/`](devin/) | Devin (CLI + Desktop) | `.devin-plugin` manifest + root `hooks.json` + MCP |
| [`gemini/`](gemini/) | Gemini CLI | `gemini-extension.json` (embedded MCP) + `hooks/hooks.json` |
| [`qwen/`](qwen/) | Qwen Code | `qwen-extension.json` + `hooks/hooks.json` (no MCP: settings.json owns it) |
| [`grok/`](grok/) | Grok Build | Claude-layout plugin, hooks only (no MCP: T275) |
| [`kimi/`](kimi/) | Kimi Code | single `kimi.plugin.json` (no MCP: T275) |
| [`opencode/`](opencode/) | OpenCode (+ Kilo) | `rtok.ts` plugin |
| [`pi/`](pi/) | pi | extension + skill (no MCP by default) |
| [`zcode/`](zcode/) | ZCode | hooks + launcher (no MCP: T275) |

## Further reading

- [`AGENTS.md`](AGENTS.md) — rules for agents working in this tree
- [`docs/agents.md`](../docs/agents.md) — host support matrix
- [`docs/getting-started.md`](../docs/getting-started.md) — first-time wire-up
- [`TODO-docs.md`](TODO-docs.md) — known documentation gaps
