---
status: in progress
priority: P2
complexity: 2
readiness: 90%
agent: Claude Code / claude-sonnet-5-5
---

# T927. `memory/observe` measurement rows claim the whole tool output as saved

3,255 `memory/observe` rows since 2026-10-18 sum to about 1.9M estimated tokens saved (1.72M on
2026-10-19). Example: measurement row 126014, before 99,589 bytes, after 404 bytes. The agent
still gets the whole output; only the stored observation is short. The ledger sums every
(plugin, kind) (`crates/rtok-store/src/measure_rows.rs`) with no exclusion for it, against the
"no Measurement row = no saving claim" rule.

Done when: observations write no saving row (or a cost row like `memory/brief`), and existing
rows are dropped or excluded from the ledger.

Check: a store test that `memory/observe` adds nothing to the saved total; `rtok gain` on a
copy of the live store before and after, recorded here; `just check`.

Plan: (1) `observe::capture` stops calling `cx.record`: the narrative is never sent to the agent
at store time, so neither a saving nor a cost row is true; the later `obs_get`/recall rows
already meter what the agent reads. (2) Migration `0052_drop_observe_measurements` deletes the
existing `memory`/`observe` rows. (3) Tests: plugin test that `observe` adds no row and the
saved total stays 0; store test that the migration drops old rows and keeps others. (4) The Savings
total of `rtok report` (there is no `rtok gain`) on a copy of the live store before/after,
recorded in `done.md`. Files:
`src/plugins/memory/{observe,mod}.rs`, `src/plugins/memory/AGENTS.md`,
`crates/rtok-store/migrations/0052_*`, `crates/rtok-store/src/tests_store.rs`. Verify: `just check`.
