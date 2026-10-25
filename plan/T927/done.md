# T927. Done

`memory/observe` no longer writes a measurement row, and 0052 drops the rows already stored.

- `src/plugins/memory/observe.rs`: `capture` stores the observation and records nothing. A
  saving row was false (the agent still receives the whole tool output) and a `memory/brief`-style
  cost row would be false too (the narrative reaches the agent only through `obs_get` and recall,
  which meter themselves), so there is no row at all.
- `crates/rtok-store/migrations/0052_drop_observe_measurements/up.sql`: deletes the
  `memory`/`observe` rows (a Diesel migration, so existing stores heal on the next open).
- Tests: `plugins::memory::tests::post_tool_stores_a_scrubbed_observation_once` and
  `a_call_delivered_again_after_the_dedup_window_is_observed_once` now assert no `observe` row
  and no `observe` group in `measurement_totals`; `tests_store::migration_0052_drops_observe_rows_only`
  seeds `memory/observe` (x2), `memory/brief` and `read/observe` rows and checks only the first
  two go.
- `tests/one_call_once.rs`: the Cursor MCP case counted the observe row; it now expects the one
  row of the shortened result.
- `src/plugins/memory/AGENTS.md`: the observation invariant states the no-row rule.

PR: https://github.com/pyrlyn/rtok/pull/1258

## Checked

`cargo nextest run -p rtok-store`: 217 passed. `cargo nextest run --bin rtok --lib memory`: 129
passed. `just check` (fmt, clippy, then the workspace tests) stopped at the first failure, so the
workspace tests were rerun with `cargo nextest run --workspace --no-fail-fast`: 4189 run, 4187
passed, 2 failed: `one_call_once` (this change, fixed above, 2/2 pass now) and `web_e2e
answers_echo_the_request_id_and_only_that_one` (passes on rerun; the machine was at load 50-60).
`hook_session_end_lock session_end_unlocks_that_agents_worktree_and_gc_drops_it` (T958) fails
intermittently under that load (it passed in the full run) and does not touch measurements.

There is no `rtok gain` command in this tree; the ledger's saved total is the Savings section of
`rtok report --format md`. Both runs used a `sqlite3 .backup` copy of the live store taken at the
same moment (the live `~/.rtok/rtok.db` was never opened for writing), one copy per run:

| | before (installed `rtok 0.17.0-alpha.2`) | after (this branch) |
| --- | ---: | ---: |
| `memory/observe` rows | 28,016 | 0 |
| `memory` saved (est tokens) | 31,998,465 | 8,645,347 |
| total saved (est tokens) | 69,219,932 | 45,866,814 |
| `Measurement` rows | 163,172 | 135,156 |

The 23,353,118 difference is exactly the `memory/observe` group (est before 25,727,954, est after
2,374,836). The card counted 3,255 rows because the store has kept growing since.

Not in scope: the other `memory` kinds still book savings (saved est tokens in the copy: `recall` 6.2M,
`prompt_recall` 2.5M, `pre_compact` 0.14M, `handoff` 0.06M), each against a different
baseline; they were not audited here.
