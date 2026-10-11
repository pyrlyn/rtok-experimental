# AGENTS.md — `plugins/codebuddy`

Agent rules for the **CodeBuddy Code / WorkBuddy** host package. Humans: [`README.md`](README.md). Shared rules: [`../AGENTS.md`](../AGENTS.md).

## Contracts

| Item | Location / rule |
| --- | --- |
| Manifest / entry | `.codebuddy-plugin/plugin.json`, `.codebuddy-plugin/marketplace.json`, `hooks/hooks.json` |
| Hooks | `hooks/hooks.json` → `rtok hook <Event> --host codebuddy` (Claude-shaped stdin; output via `codebuddy_output`) |
| MCP | none in the plugin (D33): `rtok agents install codebuddy` writes `mcpServers.rtok` into the host's MCP file |
| Installer | `src/agents/codebuddy/` is plan T560.2 |
| Versions | `tools/plugin-versions.sh` lists this tree; `plugin.json` keeps a `version` field |

## Do

- Keep README.md and AGENTS.md updated with behaviour changes.
- Call rtok CLIs instead of reimplementing policy.
- Update `tests/codebuddy_plugin.rs` when changing the manifests.

## Do not

- Rename `modifiedInput` back to `updatedInput`: CodeBuddy ignores it.
- Dual-wire the plugin and own `settings.json` hooks (D21).
