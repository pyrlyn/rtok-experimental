---
name: rtok-worktrees
description: Load before editing a repo: each task gets its own git worktree via rtok unless AGENTS.md or the user says otherwise.
---

# rtok-worktrees

Every task that edits a repository gets its own worktree, never the shared main checkout,
unless `AGENTS.md` or the user says otherwise. Use rtok, not raw `git worktree`
(no `rtok`? `ketch install pyrlyn/rtok`).

Start with `rtok worktree whoami`: your agent id (quote it when you report), where new
worktrees go, the worktree you are in and the ones you already hold. rtok 0.15.1 and older
lack it: run `rtok agents whoami` for the id and `rtok worktree list` for the rest.

## Create

MCP `worktree_add` (task, slug) or, inside the repository,
`rtok worktree add <task> [slug]` (`--owner "<provider> / <model>"` when no agent is bound; else `RTOK_OWNER` or the login user); both print the path.
A worktree the host made itself: `worktree_adopt` or `rtok worktree adopt --task <task>`.

## Finish

When the task is done: `worktree_finish` (path) or `rtok worktree finish` inside the worktree
commits what is left, pushes, opens the PR with squash auto-merge and unlocks. It does not
close the task: once the PR is merged, a running `rtok mcp` closes an rtok task, then removes
the worktree and its branch.

Without `rtok mcp`, or for a worktree you did not finish, after the PR is merged:
`worktree_remove` or `rtok worktree remove <path|task>` from the main checkout, then delete
the remote branch if the forge did not. `rtok worktree gc --yes` sweeps
many; `rtok worktree clean --yes` frees build caches and keeps the worktree;
`rtok worktree list` shows them all.

## Never

- `rm -rf` a worktree, `--force` past a refusal, `git worktree prune`.
- Touch a worktree locked by someone else or holding changes you did not make: report it.
