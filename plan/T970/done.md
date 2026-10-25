`rtok worktree add`, MCP `worktree_add` and the `WorktreeCreate` hook all go through `claim::add`, so the change is made once there and in `add::run`.

Free space: `rtok_sys::free_bytes` (rustix `statvfs` on Unix, `GetDiskFreeSpaceExW` on Windows; both bindings were already dependencies, so no new crate) reads the volume of the nearest existing ancestor of the target, before the fetch and before anything is created. Below `[worktree] min_free_mib` (default 2048, 0 disables) the add refuses with one line naming the free MiB, the key and the floor. A reading that fails warns and lets the add go on.

All or nothing: `add::Undo` is a rollback list. `run` records the branch (only when it did not exist before) and the worktree before it calls git, so a `git worktree add` that dies half-way with the branch made and a half-filled directory is cleaned up; `claim::add_with` records the claim next. On any failure the steps run in reverse: release the claim, unlock and remove the directory and prune, delete the branch. `Undo::keep` ends it when the add went through. A branch or directory that existed before the call is never recorded. The claim write now comes before the best-effort steps (symbol index copy, cache seed, project registration), so the one step that can still fail fails before the expensive seed. Its error no longer says the worktree "stays locked": it was removed.

Injection: `add::Seams { free_bytes, create }` stands in for the volume reading and for the git step; `claim::add_with` takes it, `claim::add` passes `Seams::HOST`. `add` now takes `(agent, owner)` as one tuple argument.

`.config/nextest.toml` sets `RTOK_WORKTREE_MIN_FREE_MIB=0` for the test run, so a nearly full host disk does not fail unrelated worktree tests (it did, at 1.6 GiB free); the floor's own tests inject the reading.

Docs: `docs/config.md`, `docs/agents-and-worktrees.md` and their `ru`/`uk` copies; `crates/rtok-config/default.toml`; trycmd `config-init`, `config-show`, `report-md`.

Checked: `tests/worktree.rs` module `all_or_nothing` (full volume refused with nothing created and the root directory not even made; floor 0 and an unreadable volume proceed; an injected git failure that leaves a half-made directory and a branch, and one that leaves a complete locked worktree, both leave no directory, branch, record or lock, and the same task then adds; a pre-existing branch stays; an undone claim is released and a kept one is not); `rtok-sys` `free_bytes` test; whole `worktree`, `surface_parity`, `config_coverage` test binaries and the lib worktree/config tests green; both `clippy` invocations of `just check` and `cargo fmt --check` clean. `cli_trycmd` passes except `report-md.toml` line 484, which differs only because the shared `CARGO_TARGET_DIR` puts the binary outside `[CWD]/target`. `just check` was not run: the host had under 15 GiB free.

PR: this branch.

Status: done 2026-10-23
Model: Claude Code / claude-sonnet-5-5
