---
status: todo
priority: P2
complexity: 2
readiness: 0%
agent:
---

# T995.3. Gemini CLI and Grok CLI built-in worktrees turned off by `rtok setup`

Neither host can hand its worktree create or remove to rtok, but each has an off switch (`research.md` §18.6):

- Gemini: `experimental.worktrees = false` in the user `settings.json`.
- Grok: `[hints] new_session_worktree_mode = "never"` and `fork_worktree_mode = "never"` in `~/.grok/config.toml`.

The creator chose to turn them off.

Done means:

- Install writes these keys, and only when the user has not set them.
- `--remove` takes out only the values rtok wrote.
- Every other byte stays.
- The host READMEs say that an explicit `grok -w` still makes a worktree.

Check: `just check`
