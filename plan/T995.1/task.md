---
status: in progress
priority: P2
complexity: 3
readiness: 0%
agent: Claude Code / claude-opus-5-5
---

# T995.1. CodeBuddy `WorktreeCreate`/`WorktreeRemove` route through `rtok worktree`

CodeBuddy has the same worktree hooks as Claude Code (`research.md` §18.6): stdin JSON, the path on the last stdout line, and a non-zero create exit fails the creation with no git fallback. Done means `rtok setup` / `rtok agents install codebuddy` writes both hooks into CodeBuddy's user settings and `--remove` takes them out. The hooks run the code T159 runs (`src/worktree/host.rs`), with the lock owner and agent host set to `codebuddy`. A failed rtok still yields a worktree, through the same plain-git fallback `plugins/claude/scripts/worktree.sh` uses.

Execution plan:

1. Make `worktree::host` take the host name instead of the `claude` constant.
2. Share the launcher fallback rather than copy it.
3. Add the two entries to the CodeBuddy installer, which writes only our entries and leaves the rest byte-for-byte.
4. Add tests next to T159's (install/remove round trip, create/remove through the hook).
5. Update the CodeBuddy README and the docs table.

Check: `just check`
