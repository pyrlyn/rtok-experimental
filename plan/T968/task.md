---
status: in progress
priority: P1
complexity: 4
readiness: 100%
agent: Claude Code / claude-sonnet-5-5
---

# T968. Bound the store's size: retention, WAL checkpoint, `rtok archive prune`

On 2026-10-22 `~/.rtok/rtok.db` was 3.4 GB with a 127 MB WAL and the archive 723 MB. Nothing
caps either one, and a bigger file means longer writes and more `database is locked` waits
(T934, T940).

Done when: the store and the archive have a configurable age and size limit that runs on its
own; the WAL is checkpointed and the file vacuumed after a prune; `rtok archive prune` prunes
on demand and prints what it freed; `rtok info` warns when either one is over its limit. A
prune never drops an archive entry an open session can still `expand`.

Check: store tests for the limits and the prune; a CLI test for `rtok archive prune` and the
`rtok info` warning; `just check`.

Plan: (1) `core.retain_archive_days`, `core.max_store_mb`, `core.max_archive_mb` in rtok-config; (2) `Store::prune` in `crates/rtok-store/src/retention.rs` (archive age + size limit that spares sessions with recent calls, call retention shortened toward a floor while the store is over its limit, then `wal_checkpoint(TRUNCATE)` + `VACUUM` in `sql_ext`), run from `Store::housekeeping`; (3) `rtok archive prune [--dry-run]` in `src/cli/archive.rs`; (4) `rtok info` warns when the store or archive is over its limit; (5) docs + trycmd. Verify: store tests, CLI test, `just check`.
