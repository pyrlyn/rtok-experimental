# T972. Done

Hints only, as the creator decided: the default read roots did not change.

- The outside-cwd refusal (`resolve_with`) names both ways out: the host's own file tool and
  `[plugins.read] allow_paths`.
- `roots::moved_hint`: a missing absolute path with a directory named like a worktree or client
  root the server knows, where the same file exists, ends with
  ` (moved? it exists at <path>)`. MCP `read` and `outline` append it. Outside `rtok mcp` the
  roots are not enabled, so it returns nothing and hooks never spawn git. This covers the ten
  2026-10-22 reads of `/Users/listepo/GitHub/listepo/_worktrees/rtok-t962/...`.
- `src/plugins/read/README.md` says what the errors name.

Checked:
- `cargo nextest run -p rtok --lib -E 'test(/plugins::read::|plugins::graph::.*outline/)'`:
  111 passed, including the new
  `a_missing_path_under_an_old_worktree_root_names_where_the_file_is_now` (a real git repo with a
  linked worktree).
- `-E 'test(/mcp::|outline|graph::/)'`: 376 passed.
- `cargo clippy -p rtok`, with the `just lint` flags.

PR: this branch.
