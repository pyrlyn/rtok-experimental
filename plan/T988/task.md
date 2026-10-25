---
status: in progress
priority: P1
complexity: 2
readiness: 10%
agent: Claude Code / claude-opus-5-5
---

# T988. `worktree add` without an agent falls back to an owner instead of failing

Claude Code's `WorktreeCreate` hook runs `rtok worktree add` before the new session has an rtok agent id, so every new session failed with "--owner is required when no agent is known". The owner only names who holds the lock; with no agent there is still a sensible name: `RTOK_OWNER`, else the OS login user.

Done means: `claim::owner` picks `--owner`, else the agent's `<host> / <model>`, else a non-empty `RTOK_OWNER`, else `$USER`/`$USERNAME`, else `unknown`; the last two cases print a one-line stderr warning naming the owner used. The lock still names no agent, so a lock taken this way keeps holding against other callers. Tests, docs (en/ru/uk), the skill and the `--owner` help say so.

Plan: change `owner` in `src/worktree/claim.rs` (shared by `add`, `adopt`/`claim` and `finish`); replace the error assertion in `tests/worktree.rs` (`add_binds_the_worktree_to_the_calling_agent`) with the fallback, the `RTOK_OWNER` path, the warning and a lock-holds check; update `src/cli/worktree.rs` help, `docs/{,ru/,uk/}agents-and-worktrees.md`, `skills/rtok-worktrees/SKILL.md`; run `just check`.

Check: `cargo nextest run --test worktree add_binds_the_worktree_to_the_calling_agent` passes and `just check` is green.
