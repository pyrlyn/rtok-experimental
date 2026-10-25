---
status: todo
priority: P2
complexity: 1
readiness: 0%
agent:
---

# T987. Re-check `research.md` §25 against Claude Code's plugin MCP dedup

From `ideas.md` I-117 (T332 side findings). Claude Code's MCP docs (https://code.claude.com/docs/en/mcp, checked 2026-10-09) say plugin MCP servers are matched "by endpoint" against user, project and local servers, and the higher-ranked hand-written entry wins. That contradicts `research.md` §25 ("two servers", Claude Code 2.1.267) and T331.11's "plugin copy is kept first" default. Also unknown: whether Devin runs two servers when install writes `mcp_config.json` and the plugin ships `.mcp.json`.

Done means: §25 states what the installed Claude Code version does, with the docs URL, the version and the date; if the default from T331.11 is wrong, a follow-up task is filed on the creator's word.

Check: §25 carries a dated row with the Claude Code version and the docs URL, and either confirms T331.11's default or names the follow-up.
