The store and the archive now have age and size limits that run on their own, and `rtok archive prune` runs them on demand.

Three `[core]` keys (defaults in `crates/rtok-config/default.toml`, `docs/config.md` and its ru/uk copies): `retain_archive_days = 90`, `max_archive_mb = 1024`, `max_store_mb = 2048`; 0 turns a limit off. `Store::prune` (`crates/rtok-store/src/prune.rs`) extends the existing retention instead of adding a second mechanism: `Store::housekeeping` (run at proxy/mcp start by `spawn_retention`) calls it right after `run_retention`, and the CLI calls the same method.

What a prune does: deletes archive bodies (row, summaries, decisions, read-cache link, `call_io` pointer, file) older than `retain_archive_days`, then the oldest ones until the archive fits `max_archive_mb`; purges calls past `retain_calls_days`; while the database plus WAL is over `max_store_mb`, halves call retention down to a floor of 7 days. After any deletion it runs the pre-T352 `VACUUM` conversion when needed, `incremental_vacuum` and `PRAGMA wal_checkpoint(TRUNCATE)` (the one new statement, in `sql_ext` with a why-comment).

Live sessions: a session with a call in the last 14 days (and not ended after it) is open. A body that session wrote, or that its archive decisions or read cache name, is never a candidate, so `rtok expand <id>` stays lossless for it. A session closed without `SessionEnd` stops being protected after 14 days.

`rtok archive prune [--dry-run]` prints bodies deleted (or that would be), calls deleted, and the store size before and after. `rtok info` prints a `warning` line, and a `warnings` array in `--json`, when the db plus WAL or the archive directory is over its limit.

Check: store tests `tests_prune::the_age_limit_spares_what_an_open_session_can_expand`, `the_archive_size_limit_drops_the_oldest_idle_bodies_first`, `the_store_size_limit_shortens_call_retention_and_empties_the_wal` (4 passed with `housekeeping_converts_a_pre_t352_store`); `info::tests::a_store_or_archive_over_its_limit_warns_and_zero_is_off`; CLI `tests/archive_prune.rs` (info warns, dry run deletes nothing, real run frees and clears the warning) and trycmd `archive-prune-dry-run`; completions, help, `config-init`, `config-show`, `report-md` and `info-json` snapshots regenerated; `surface_parity`, `config_coverage` entries added; `just check` green. PR: this branch.

Not previewed by `--dry-run`: the calls part (the size loop depends on how far each purge shrinks the file). `../../rust.md` needs no change (no new crate).
