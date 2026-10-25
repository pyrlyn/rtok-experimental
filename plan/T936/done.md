# T936. Done

`worktree list` and `worktree clean` no longer pay for what they do not show. The under-1-s
target is not met on this repository; the floor is below.

- `worktree list` (and the Worktrees dashboard page, the MCP `worktree_list`, `agents junk`) does
  not walk the tagged build caches: `Row.cache_bytes` is `null`, the table prints `-` and its
  footer says `build cache not sized (--sizes)`. `rtok worktree list --sizes` keeps the old
  walk (`list::rows_sized`). Source bytes are unchanged; `modified` is the newest source file.
  `--sizes` is exempt in `tests/config_coverage.rs`: a stored `true` would bring the slow walk
  back.
- `worktree clean` listed the worktrees through the full `inventory` (a `git status` and a merge
  check per worktree) and walked them one after another. It now lists paths with `git worktree
  list` (`orphans` takes the paths) and walks the worktrees in parallel (`par_map`).
- Docs (`docs/`, `docs/ru/`, `docs/uk/` `agents-and-worktrees.md`), the completions snapshots and
  the `help-subcommands` case follow the new flag.

PR: https://github.com/pyrlyn/rtok/pull/1260

## Timings

Debug builds, run from the main checkout with 38 worktrees (15.4 GB of source files, 94 GB of
build caches as `--sizes` counts them), machine load average 15 to 40 from other sessions.
Before is `pyrlyn/main` b7183a941, after is this branch; three runs each, seconds.

| command | before | after |
| --- | --- | --- |
| `worktree list --json` | 11.8, 8.8, 9.7 | 9.1, 5.5, 5.1 |
| `worktree list --json --sizes` | n/a | 8.9, 8.5, 7.8 |
| `worktree clean --idle 0d` (dry run) | 27.3, 26.3, 34.1 | 16.7, 15.9, 27.8 |

`worktree add t936-timing probe` took 4 min 41 s once, at load 22 to 34 (not repeated; before
this change it is the same code). It was removed again with `worktree remove`.

## Floor, not fixed

- `list` still runs a `git status` and a `git merge-tree` per worktree for the state column, and
  walks every source file. Untagged folders count as source: a cleaned worktree keeps
  `target/test-home` (about 66k files, 1.3 GB) that nothing tags, so 15 GB of "source" is walked.
- `clean` has to walk each worktree to find its caches and sum their bytes, which is its job.
- `worktree add` spends its time cloning the sibling `target/` and `node_modules/` trees
  (`seed::from_siblings`, T953), file by file; it is not touched here.

Follow-ups worth a task: a `CACHEDIR.TAG` (or a skip) for `target/test-home`, and a budget or
cache for the `list` state column.

## Checked

`list::tests::usage_sources_skips_tagged_caches`, `list::tests::an_unsized_row_has_no_cache_bytes_in_the_table`,
`tests/worktree.rs::list_splits_cache_from_source_and_reports_an_orphan` (default `null`, `--sizes`
bytes), `just check` (4190 tests passed, 8 skipped).
