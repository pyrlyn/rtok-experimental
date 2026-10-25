Dropped as a duplicate. T954 (#1230) ported this card to T958, and T958 shipped the fix in #1244: SessionEnd releases the git lock that names the ending agent, leaves other agents' locks and a parent's lock alone. #1242 then brought this old card back from a branch cut before the port.

Check: `plan/T958/done.md` on `main` covers this card's Check line (`session_end_unlocks_that_agents_worktree_and_gc_drops_it`, `a_subagents_session_end_leaves_the_parents_lock`).
