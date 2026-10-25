---
status: in progress
priority: P2
complexity: 3
readiness: 90%
agent: Claude Code / claude-opus-5-5
---

# T929. Dedup stubs whose original is no longer in the agent's context (compaction, subagents)

After a Codex `compacted` event the agent got `identical to a result N turns ago` stubs for
results it no longer had, and re-read with `lines` mode or `expand`. 36
`unchanged since <sha8> (N lines)` MCP stubs went to agents that never saw the file, 30 of them
subagents (2026-09-14..10-16). T127 kept session-only scoping on purpose (`research.md` §17.3);
T122 and T531 are done.

Done when: a compaction (PreCompact/PostCompact or the host's equivalent) resets the dedup and
re-read windows of that session, and a subagent's first read of a file is never a stub (needs
the creator's OK to revisit T127's scoping).

Check: hook tests: a stub before compaction, the full result after; a subagent's first read is
whole; `just check`.

The creator gave the OK to revisit T127's session-only scoping (2026-10-21).

Execution plan:
1. `rtok-store`: `Store::forget_seen(session, context)` drops the session's `read_cache` rows and
   re-tags the context's `archive` rows with a sentinel `agent_id`, so `archive_in_session` no
   longer hits them (the bodies stay for `expand`); a repeat archive of the same body reclaims
   the row. `Store::session_has_subagent(session)`.
2. `rtok-plugin-sdk`/`src/plugin.rs`: `ReadCache::forget_seen` and `shared_session` (default off).
3. `src/hooks/mod.rs`: PreCompact, PostCompact and SessionStart `compact` call `forget_seen` for the
   payload's context.
4. `src/plugins/read/mod.rs`, `read/hook.rs`: while the session has a sub-agent, MCP `read` returns
   no stub, delta or remembered pointer (MCP cannot see the caller). The creator wants the caller
   passed in too ("both"); that part, a hook-added `agent` argument, is the follow-up T929.1, and
   this fallback stays under it.
5. Tests: store, hook (stub before compaction, full result after), read (first read in a session with
   a sub-agent is whole); `just check`.
