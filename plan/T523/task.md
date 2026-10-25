---
status: in progress
priority: P2
complexity: 2
readiness: 90%
agent: Claude Code / sonnet-5.5
---

# T523. `worktree add` takes `T504 read-hook-same-path` as an id plus a slug

Found 2026-10-10: 27 `task id must match [a-z0-9]…` errors since 2026-10-06. Every one is an id and words in one argument (`T504 read-hook-same-path`, `M16.5 test toml`, `T330.5.3 junk-stale-worktrees`). `ident` in `src/worktree/add.rs` lower-cases, then rejects the space. Agents pass the id and the slug the way the branch is named, as one string. Nine further adds failed with `already exists: one worktree per task` on a retry of an add that had already landed.

Done when: one argument `T504 read-hook-same-path` plans task `t504` and branch `t504-read-hook-same-path`; spaces in the slug become dashes; a bare id is unchanged. `add` of a path whose branch is already that task returns the existing plan instead of failing. A string that is not an id after the split still fails the existing check.

Check: `plan` tests for the logged `T504 read-hook-same-path`, `M16.5 test toml` and `M16.3 push.rs through the config module`, and for an existing path of the same task; `just check`.

## Execution plan

Reimplemented on current main; the stale `origin/t523-worktree-id-slug` (4e819e65a) was only a reference.

1. `src/worktree/add.rs`: `split_id_slug` feeds `plan`; `plan` stops refusing an existing path. `run` takes the store and, when the path exists, returns the existing plan (`Added::Existing`) only if git lists a worktree there whose lock names this task and is claimable by this owner/agent (`claim::held_against`); any other holder still gets `already exists: one worktree per task`.
2. `src/worktree/claim.rs`: `add_with` skips claim, seeding, project registration and rollback for an existing plan.
3. Docs: `src/cli/worktree.rs` help, trycmd help and zsh completion snapshots, MCP `worktree_add` schema text.
4. Tests: unit (`add.rs`) for the logged `T504 read-hook-same-path`, `M16.5 test toml`, `M16.3 push.rs through the config module`, bare id, a split that is not an id; integration (`tests/worktree.rs`) for the retry (same owner prints the same path, different owner refuses), MCP retry; the hook test's "two words" name becomes "../evil".
5. Verify: `cargo nextest run --lib --test worktree --test cli_trycmd --test surface_parity --test config_coverage`, then `just check`.
