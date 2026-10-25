---
status: in progress
priority: P2
complexity: 3
readiness: 80%
agent: Claude Code / claude-opus-5-5
---

# T995.2. Command Code `worktree.createHook`/`removeHook` route through `rtok worktree`

Command Code can replace its git worktree backend with two scripts (`research.md` §18.6):

- Create gets `COMMAND_CODE_WORKTREE_NAME` and prints the absolute path.
- Remove gets `COMMAND_CODE_WORKTREE_PATH` and `COMMAND_CODE_WORKTREE_NAME`.

Done means `rtok setup` / `rtok agents install commandcode` writes both keys into `~/.commandcode/settings.json`, and `--remove` takes them out. The scripts reach the same create and remove code as T995.1, owned by host `commandcode`. A failed rtok still makes or removes a plain git worktree.

Check: `just check`

Plan:

1. Command Code 1.64.0 (`dist/cli.mjs`, read 2026-10-25) runs each key as a shell command with the variables above, gives it no stdin, and makes no worktree when the hook fails. So both keys hold `<rtok> hook WorktreeCreate|WorktreeRemove --host commandcode`, with no launcher in between.
2. `src/worktree/host.rs`: add `commandcode` to `HOSTS` and build its payload from the environment and the working directory. Command Code passes no session id, so the agent is the one live Command Code agent in the repository: extract that lookup from `claim::bind_unattended` and reuse it. A create that fails, or one made while `[worktree] enabled = false`, falls back to a plain `git worktree add` at `.commandcode/worktrees/<name>` on `worktree-<name>`, the pattern the other launchers use.
3. `src/agents/commandcode/mod.rs`: `run` writes `worktree.createHook`/`removeHook` only where the user has none, and `--remove` takes them out only while they still hold rtok's command.
4. Tests: installer unit tests (insert, keep a foreign hook, strip only ours) and `tests/worktree.rs` (env-driven create binds the live agent, the fallback, remove). Docs: `src/agents/commandcode/README.md`, host table in `docs/agents-and-worktrees.md` (en, ru, uk).
