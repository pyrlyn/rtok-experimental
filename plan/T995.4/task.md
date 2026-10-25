---
status: todo
priority: P2
complexity: 3
readiness: 0%
agent:
---

# T995.4. Zed and Factory Droid post-create scripts run `rtok worktree adopt`

Zed and Droid create worktrees themselves and cannot hand that to rtok, but each runs a script after creation (`research.md` §18.6):

- Zed: a `tasks.json` task with `"hooks": ["create_worktree"]`.
- Droid: a profile in `.factory/worktree-setups/*.yml`.

The creator chose `adopt` for such hosts. Done means `rtok agents install zed|droid --project` adds one entry, the same way T289.3 does for Cursor, Kilo, Windsurf and Devin (`src/agents/post_create.rs`), and `uninstall` removes it. Every other byte stays.

Check: `just check`
