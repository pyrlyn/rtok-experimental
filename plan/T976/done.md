Done: `rtok graph review` scores only definitions a hunk touches (it used to score the first 8
definitions of each changed file, touched or not). Support code is listed with `support`, score
0 and no `untested`: `fixtures/`, `fixture/`, `testdata/`, `bench/`, `benches/` and
`examples/` directories (`is_support_path` in `src/plugins/graph/mod.rs`), or a set
`linguist-vendored`, `linguist-generated` or `linguist-documentation` attribute read with
`git check-attr`; an explicit `false` or unset attribute overrides a directory, and a failing
`check-attr` leaves the directories alone to decide. Levels fit the reachable 0.05..0.6 score:
`high` above 0.45, `medium` above 0.25. Files scoring 0 get lines only after every scored file
got all its regions. JSON files carry `support`. New `docs/review.md` (why, how, score, levels,
support code, budget, output, CI, limits), linked from `docs/commands.md`. The `docs/ru` and
`docs/uk` translations were left out on the creator's word.

Check: `cargo nextest run --lib -E 'test(/review/) | test(/support/)'`, 41 passed (new: support
directories, `check-attr` parsing, unscored files read last, only touched definitions scored,
support code end to end with a `.gitattributes` override). `rtok graph review --since
31633156a~1` lists the bench files as `support` with no `untested`. `just check`: fmt, clippy
and jscpd pass; `plugins::graph::watch::tests::watcher_reindexes_new_file_while_calls_read_nothing`
and `an_edit_in_a_linked_project_updates_its_index` fail here and on a clean `main` (31633156a)
alike, so the rest of the workspace was run with `--no-fail-fast` excluding those two: 4233
passed, 4 failed. The 4 (`cli_trycmd` `cli` and `cli_snapshots_ignore_ambient_rust_log` on
`config-init.toml` and `report-md.toml`, `graph_lsp_gate` `lsp_backend_outlines_dart_main`,
`worktree` `mcp_worktree_tools_act_for_the_linked_agent`) fail on a clean `main` on this host
too.

Not done: Rust unit tests in a `#[cfg(test)]` module of the same file still do not count as
tests, so their code is listed `untested` (documented under Limits).
