---
status: in progress
priority: P2
complexity: 3
readiness: 90%
agent: Claude Code / claude-sonnet-5-5
---

# T936. `rtok worktree list --json`, `clean` and `add` take 8–10 s

Codex tool outputs on 2026-10-11: `worktree list --json` 10.5 s, `worktree clean … --idle 0d`
10.4 s, `worktree add` 8.6 s and 3.6 s; most other calls under 1 s. Likely the disk-size scan
of large `target/` folders. T527 and T528 cover correctness, not speed.

Done when: `list` does not size build folders unless asked (or caches the size), and the three
commands finish under 1 s on the rtok repo with its worktrees, or the floor is written up.

Check: a timing run before and after on the rtok repo recorded here; tests unchanged;
`just check`.

Plan: `list::rows` stops walking tagged build caches (`usage_sources`), `rows_sized` and
`worktree list --sizes` keep the old walk; `cache_bytes` becomes `null` when unsized. `clean`
lists worktree paths with `git worktree list` instead of the full `inventory` (git status and
merge checks decide nothing there) and runs its per-worktree walks in parallel. Files:
`src/worktree/{list,clean,ops}.rs`, `src/cli/worktree.rs`, `tests/worktree.rs`, docs. Verify:
timings before and after on this repository; `just check`.
