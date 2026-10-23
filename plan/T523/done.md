# T523 done

`worktree add` takes `T504 read-hook-same-path` as an id plus a slug: the first word is the id, the rest is the slug with spaces turned into dashes (`src/worktree/add.rs`). An add on an existing task path returns that plan when git lists a linked worktree there, its lock names the same task and the caller holds it; another owner is still refused with the holder named, and a string that is not an id after the split still fails. The CLI, MCP `worktree_add` and the WorktreeCreate hook share the path through `claim::add_with`. Docs (en/ru/uk), help text and trycmd snapshots are updated.

PR: https://github.com/pyrlyn/rtok/pull/1279

Checked: unit tests for the three logged strings and the retry, written first and seen failing; integration tests in `tests/worktree.rs` for CLI, MCP and hook; `cargo nextest run --workspace --no-fail-fast` gave 4243 passed. The failures are the trycmd cases `config-init` and `report-md`, which also fail on main, and one load timeout of `clink_covers_every_rtok_subcommand_and_parses`, which passes alone.
