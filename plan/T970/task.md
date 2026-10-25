---
status: in progress
priority: P2
complexity: 4
readiness: 100%
agent: Claude Code / claude-sonnet-5-5
---

# T970. `rtok worktree add` creates all or nothing and checks free space first

On 2026-10-22 21:36:22, `git worktree add` failed with `No space left on device` on the disk
that also broke CI. It left a half-made worktree, and three later adds failed with `already
exists`.

Done when: before `git worktree add`, `rtok worktree add` (CLI and MCP `worktree_add`) checks
the free space on the target volume and refuses with a one-line hint when the space is below a
configurable floor. On any failure after that point, it removes the directory, the branch it
created and the claim record. A branch or directory that existed before the call stays.

Check: tests that inject a failed `git worktree add` and a full-volume reading, then assert
nothing is left behind; `just check`.

Plan: `rtok-sys` gains `free_bytes(path)` (rustix `statvfs` on Unix, `GetDiskFreeSpaceExW` on Windows; both crates already depend on the OS binding). `[worktree] min_free_mib` (default 2048, 0 off) is read in `claim::add` and handed to `add::run_with`, which checks the nearest existing ancestor of the target before the fetch. `add::Undo` records branch, worktree and claim as they are created and takes them back in reverse on drop unless kept; a pre-existing branch is never recorded. `add::Seams` makes the free-space reading and the `git worktree add` step injectable. Tests: `tests/worktree.rs` with injected failures against a scratch repository; docs (en, ru, uk), default.toml and trycmd expectations; verify with the focused nextest runs, clippy, `just check`.
