---
status: todo
priority: P2
complexity: 3
readiness: 0%
agent:
---

# T995.2. Command Code `worktree.createHook`/`removeHook` route through `rtok worktree`

Command Code can replace its git worktree backend with two scripts (`research.md` §18.6):

- Create gets `COMMAND_CODE_WORKTREE_NAME` and prints the absolute path.
- Remove gets `COMMAND_CODE_WORKTREE_PATH` and `COMMAND_CODE_WORKTREE_NAME`.

Done means `rtok setup` / `rtok agents install commandcode` writes both keys into `~/.commandcode/settings.json`, and `--remove` takes them out. The scripts reach the same create and remove code as T995.1, owned by host `commandcode`. A failed rtok still makes or removes a plain git worktree.

Check: `just check`
