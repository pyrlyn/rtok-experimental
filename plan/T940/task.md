---
status: in progress
priority: P3
complexity: 2
readiness: 80%
agent: Cursor / grok 4.6
---

# T940. Graph `references` writes hit "database is locked"; session-only projects auto-registered

53 `graph/references … failed: database is locked` warnings (2026-10-05..20), 46 of them for
`~/Library/Application Support/ClaudeBar/Probe`, an app probe with 615 sessions that ran only
SessionStart/SessionEnd and was auto-added as a project; `~/GitHub` is a project too.

Done when: a session that ran no tool does not register a project, the locked `references`
write backs off instead of warning each time, and the existing probe project rows are dropped.

Check: tests for the no-tool session and the back-off; `just check`.

## Execution

1. Stop `SessionStart` from calling `auto_add_project`. Register on the first tool hook
   (`PreToolUse`, `AfterMCPExecution`) so a SessionStart/SessionEnd-only probe never joins
   the registry. Keep the MCP and worktree origins as they are.
2. `follow::refresh`: run `retry_locked` (T934, no second retry loop) around the references
   write. A lock that outlasts the retries fails open with no `graph/references` warning.
3. Store method drops `origin = session` projects whose sessions never recorded a tool hook
   or an MCP call, via `remove_project`. `refresh` runs it. Fixtures only; never the live db.
4. Tests: toolless session vs first tool; locked write backs off; probe-like rows dropped.
   Then `just check`.
