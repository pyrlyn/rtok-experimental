# T969. Done

`note_slow` now keeps one tally per event in the store's `kv` table (`hook:slow:<event>`: last
warning time, calls, worst ms), so it survives across the short-lived hook processes. Inside the
interval a slow call only updates the tally; once `[hook] slow_warn_s` (default 300 s, `0` = warn on
every slow call) has passed, one `warn` row (and the stderr line) reports the call count and the
worst time since the last warning. A fast run never touches the store. A store error skips the
bookkeeping and the hook still exits 0. `max_ms` is unchanged and the per-call `calls` rows are
untouched (`set_call_ms` runs as before).

Checked: unit tests `slow_tally_*` and `note_slow_logs_one_row_per_event_per_interval` in
`src/hooks/mod.rs`; `config_coverage`, `surface_parity`; trycmd snapshots (`config-init`,
`config-show`, `report-md`) regenerated. PR: this branch.
