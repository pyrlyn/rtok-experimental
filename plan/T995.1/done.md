# T995.1 done

CodeBuddy's `WorktreeCreate` and `WorktreeRemove` now run the code T159 runs for Claude Code.

- The hooks ship in the plugin (`plugins/codebuddy/hooks/hooks.json`), not in `settings.json`: D21 makes the plugin the only home of rtok's hooks. So `rtok agents install codebuddy`, which installs the plugin, adds them, and `--remove`, which uninstalls it, takes them out. The task text said user settings; the plugin is the same reach without dual-wiring.
- Both entries run `scripts/worktree.sh <event> codebuddy`, a byte-for-byte copy of `plugins/claude/scripts/worktree.sh`. The launcher now takes the host as its second argument (`claude` when absent, anything but `claude`/`codebuddy` exits 1) and passes it on as `rtok hook <event> --host <host>`. When rtok is missing or fails, it falls back to a plain worktree at `.<host>/worktrees/<name>` on `worktree-<name>`.
- `src/worktree/host.rs` takes `--host` (`HOSTS = ["claude", "codebuddy"]`, Claude Code when absent, any other host refused). The lock owner and the session's agent row come from that host.
- CodeBuddy has no seeded host row, so the hook dispatcher files its agents under `other`. The new `Store::host_id_or_other` gives the worktree hook the same row, and `src/plugin.rs` reuses it instead of its own fallback.
- Docs: the CodeBuddy installer and plugin READMEs, the plugin's `AGENTS.md`, and the host table in `docs/agents-and-worktrees.md` (en, ru, uk).

Checked:

- `tests/worktree.rs` `codebuddy_worktree_hooks_bind_and_remove_for_its_session_agent`: create binds the worktree to the CodeBuddy session's agent under `.rtok/worktrees/`, `--host cursor` is refused with nothing printed, and remove takes the worktree away.
- `tests/plugin_scripts.rs`: the launcher fallback gives `.claude/worktrees/foo` and `.codebuddy/worktrees/bar` on `worktree-<name>`.
- `tests/codebuddy_plugin.rs`: both `hooks.json` entries are pinned and the two launchers are byte-equal.
- Focused run of the three files plus `hook_manifests` and `codebuddy_host`: 21 passed.
- Local `cargo nextest run --workspace --no-fail-fast` (load average ~45 from other sessions): 4283 of 4285 passed. The two that failed are timing tests that never touch this change: `hook_session_end_lock` (a busy store defers SessionEnd to a child, so the unlock lands after the assert; it passes alone) and `cli_trycmd` (180 s timeout). Fmt, clippy and the other gates passed. CI on the PR ran the full gate on an idle runner.
