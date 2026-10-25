# T988 done

`claim::owner` (`src/worktree/claim.rs`) no longer fails without an agent: after `--owner` and the agent's `<host> / <model>` it takes a non-empty `RTOK_OWNER`, else `USER`, else `USERNAME`, else `unknown`, and prints `warning: no agent bound; owner `…`` to stderr. Only `worktree add` reaches the new branch: `adopt`/`claim` always bind an agent, and `finish` skips the owner check when it has neither. The lock still names no agent, so it holds against any other caller.

Docs (`docs/{,ru/,uk/}agents-and-worktrees.md`), `skills/rtok-worktrees/SKILL.md`, the `--owner` help and its goldens say so.

Checked: `tests/worktree.rs` `add_binds_the_worktree_to_the_calling_agent` covers `USER`, `RTOK_OWNER`, `USERNAME` past an empty `RTOK_OWNER`, `unknown`, the warning, the agent-less lock, and an agent refused with `locked by tester`. Local `just check`: fmt, jscpd, clippy and Python green; nextest 4276/4276 with `graph_lsp_gate lsp_backend_outlines_dart_main` excluded — it fails on this machine alone too (Dart SDK 3.13.7, `.dart_tool/rtok-lsp-xdg` not created), unrelated to this change.
