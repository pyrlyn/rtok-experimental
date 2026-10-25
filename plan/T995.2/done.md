# T995.2 done

Command Code's worktree create and remove now go through `rtok worktree`.

- `rtok agents install commandcode` writes `worktree.createHook` = `<rtok> hook WorktreeCreate --host commandcode` and `worktree.removeHook` = `<rtok> hook WorktreeRemove --host commandcode` into `~/.commandcode/settings.json` (`src/agents/commandcode/mod.rs`).
  - A key the user already set stays, and rtok's command is not written there.
  - `--remove` takes a key out only while it still holds rtok's command, and drops a `worktree` object left empty.
  - `is_rtok_cmd` is shared with the existing hook matcher.
- Command Code 1.64.0 (`dist/cli.mjs`, `runCreateHook` / `runRemoveHook`, read 2026-10-25) runs the value as a shell command with `COMMAND_CODE_WORKTREE_NAME` / `_PATH`. It sends no stdin, names no session, and on a failed create makes no worktree.
  - So `src/worktree/host.rs` adds `commandcode` to `HOSTS` and builds its payload from the environment and the working directory.
  - The agent is the one live Command Code agent in the repository. `claim::sole_live_agent` is extracted from `bind_unattended` and shared by both.
  - When rtok refuses the name (Command Code allows `/`), fails, or has worktrees turned off, rtok makes a plain worktree at `.commandcode/worktrees/<name>` on `worktree-<name>` itself (`git::add_plain`). This is the plugin launchers' pattern, not Command Code's own `~/.commandcode/worktrees/<repo>/` default.
- Docs: `src/agents/commandcode/README.md` (Worktrees section, docs link) and the host table plus the `enabled = false` note in `docs/agents-and-worktrees.md` (en, ru, uk).

Not covered: a missing rtok binary. The value names rtok directly, so `enter_worktree` then fails with exit 127, as every other rtok hook in that file does; `rtok agents install commandcode --remove` takes the keys out.

Checked:

- `agents::commandcode` lib tests (11): the new `worktree_hooks_route_to_rtok_and_a_users_own_hook_stays` covers the insert being idempotent, remove, a user's `createHook` and `baseRef` surviving both, and only the free key coming and going.
- `tests/worktree.rs` `commandcode_worktree_hooks_read_the_environment_and_fall_back_to_plain_git`:
  - An env-only create binds the live agent's lock under `.rtok/worktrees/`, and remove takes it away.
  - `feat/auth` gives `.commandcode/worktrees/feat-auth` on `worktree-feat-auth`.
  - `[worktree] enabled = false` gives `.commandcode/worktrees/cc-2`.
  - Fail-first: with `live_agent` disabled the test fails.
- `host_docs`, `agents_doc`, `worktree`, `hook_manifests`, `surface_parity`, `config_coverage`: 72 passed.
- Local `just check`: 3415 of 3416 run passed. The one failure is `hook_session_end_lock`, which fails only under this machine's load; it passed in CI on #1296 and passes alone. CI on this PR ran the full gate.
