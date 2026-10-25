Dropped as a duplicate of T949, which T954 (#1230) ported to this id after T949 had already shipped in #1196. `gc::Policy` carries the ended agents (`Store::ended_agent_ids`), and `gc::decide` reclaims a merged, clean worktree under an ended agent's lock once the idle window passes.

Check: `an_ended_agents_lock_holds_only_through_the_idle_window` in `src/worktree/gc.rs` on `main` already has this card's cases (ended and merged → reclaim, live → keep, ended and unmerged → keep).
