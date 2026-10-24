---
status: in progress
priority: P2
complexity: 4
readiness: 50%
agent: Claude Code / claude-opus-5-5
---

# T995. Every host's own worktrees go through `rtok worktree`, or are turned off

The creator wants worktrees to be created and removed by `rtok worktree` on every host, not only on Claude Code (T159). Each host gets the strongest control it offers (`research.md` §18.6):

1. A host that can run an outside command in place of its own create and remove uses `rtok worktree`.
2. A host that only has an off switch gets it turned off in the user config.
3. A host that only runs a script after it creates a worktree runs `rtok worktree adopt` there.
4. Any other host is documented as outside rtok.

Done means T995.1–T995.4 are closed and `docs/agents-and-worktrees.md` (with `docs/ru` and `docs/uk`) states each host's level.

Execution plan:

- T995.1: CodeBuddy hooks.
- T995.2: Command Code hooks.
- T995.3: Gemini and Grok off switches.
- T995.4: Zed and Droid post-create `adopt`.

Each subtask lands as its own PR, and this task closes after the last one.

Check: every subtask has `done.md` and the host table in `docs/agents-and-worktrees.md` covers the hosts in `research.md` §18.6.
