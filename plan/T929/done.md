# T929. Done

A compaction now resets what dedup remembers, and a session with a sub-agent gets no re-read
stubs from MCP `read`.

- **Compaction.** PreCompact, PostCompact and SessionStart `compact` (whichever the host sends;
  the reset is idempotent) call `Store::forget_seen` for the payload's context. It drops the
  session's re-read cache rows and re-tags that context's `archive` rows with a sentinel
  `agent_id`, so `archive_in_session` no longer answers with a pointer to them. The bodies stay
  for `expand`. Printing the same body again hands the row back to the session
  (`insert_archive_row_conn` reclaims a forgotten row of the same session only). No migration.
- **Sub-agents, fallback half ("b").** MCP `read` cannot tell a parent from a sub-agent
  (`context = None`). Once the session has a sub-agent row (`Store::session_has_subagent`), it
  returns no `unchanged` stub, delta or content-hash pointer and remembers nothing; the native
  Read hook's delta deny is skipped too. An unreadable store counts as "has a sub-agent". The
  creator allowed revisiting T127's session-only scoping (2026-10-21).
- **Not done here ("a").** The creator also wants the caller passed in so the parent keeps its own
  stubs; that is T929.1 (a hook-added `agent` argument; the fallback above stays under it). It
  needs a hook matcher per host, installed and checked by `agents install` and `doctor`, so it
  gets its own change.
- **Known gap.** A capped body archived by MCP `read` under context `None` before a sub-agent
  appeared can still match a later `identical_result` from the native hooks of the parent; the
  compaction reset covers it, the sub-agent window does not.

Files: `crates/rtok-store/src/{cache,lib,agents}.rs`, `crates/rtok-plugin-sdk/src/host.rs`,
`src/plugin.rs`, `src/hooks/mod.rs`, `src/plugins/read/{mod,hook}.rs`.

Checked: store tests (forget, reclaim, no takeover by another session, sub-agent row), hook tests
for all three compaction events and a non-compact SessionStart, a read test (first and repeat read
whole in a session with a sub-agent); `graph_truth`; lib tests of `rtok`, `rtok-store` and
`rtok-plugin-sdk` (2966 passed); the `justfile` clippy gates and `cargo fmt --check`. A local
`just check` ran out of disk twice, so CI on the PR is the full run.

PR: https://github.com/pyrlyn/rtok/pull/1259
