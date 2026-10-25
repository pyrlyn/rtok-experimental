---
status: in progress
priority: P2
complexity: 3
readiness: 100%
agent: Claude Code / claude-opus-5-5
---

# T973. Agent lookup: shared cwd, unlinked sessions, `--agent <provider> / <model>`

2026-10-22 logs: `mcp/whoami: ambiguous: agents 5fa4ff6f, 8b7fdbe4 share this cwd`, 6×
`not linked to an agent session`, and `--agent OpenAI / GPT-6: unknown`. An agent copied the
plan front matter's `agent` value into `--agent` (`src/worktree/claim.rs` `caller`). T281 (the
session ↔ MCP link) and T966 (one lookup, not in the installed alpha.2) cover part of this.

Done when: after T966, list which of these still happen on main. Fix the rest that fall outside
T281: `--agent` given a `<provider> / <model>` string says it wants an rtok agent id and how to
find one. An ambiguous or unlinked `whoami` names the next step. A Codex session resolves like a
Claude Code one, where its hooks give a session id.

Check: tests for each message and for a Codex session lookup; `just check`.

Plan (scope narrowed by the creator on 2026-10-23: only what T281 does not cover; linking by
PID and Codex session lookup stay with T281). On main after T966 and T924:
- The ambiguous message already says `pass agent with the id of yours (whoami)`, and so does the
  `Outside` one. The 2026-10-22 lines with `use RTOK_AGENT_ID` came from the installed alpha.2.
- Still bare: `Link::None` → `not linked to an agent session`, 58 of the day's MCP errors
  (whoami 23, worktree_adopt 9, worktree_add 8, agent_inbox 8, …).
- `--agent OpenAI / GPT-6` → `--agent …: unknown`.
Steps:
1. `src/mcp.rs` `agent()`: `Link::None` says why (registry off, or no live agent of the host ran
   a hook here since the server started) and what to do (retry, `rtok doctor`, pass `agent`).
2. `src/worktree/claim.rs` `caller`: a flag with `/` or a space says it wants an rtok id and
   names `rtok agents whoami`.
3. Unit tests in both files; `cargo nextest run -p rtok --lib -E 'test(/mcp::|worktree::claim/)'`.
