Dropped as a duplicate of T950, which T954 (#1230) ported to this id after T950 had already shipped in #1196. The creator's choice recorded in `plan/T950/done.md` is "after `stale_lock`": a vanished record under a foreign lock is dropped once the lock's date is older than `stale_lock`, or at once when the lock's agent has ended.

Check: the T950 drop-record cases in `src/worktree/gc.rs` on `main`; see `plan/T950/done.md`.
