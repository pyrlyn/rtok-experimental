---
status: todo
priority: P2
complexity: 3
readiness: 0%
agent:
---

# T981. Parallel walk for MCP `search` and `tree`

From `ideas.md` I-95. Both walk with `ignore::WalkBuilder::build()` (`src/plugins/read/search.rs`), single-threaded. `ignore` already ships `build_parallel()`, so no new dependency. `tree` stats in parallel and sorts once; `search` needs an atomic hit counter to keep the early exit at `cap`. `research.md` §39 (T378): on real ~2k-file repos `search` p95 is at most 157 ms, but on a synthetic 66k-file tree an absent literal reads every file, p95 30.4 s, linear in file count.

A parallel walk can lose on small trees, so the measurement decides: keep the parallel path only where it wins, for example above a file-count threshold.

Done means: the large-tree case is measurably faster, small trees are not slower, and the output stays byte-identical (sorted).

Check: p50/p95 of `search` and `tree` on this repo and on the 66k-file tree, before and after, recorded in `research.md`; an existing-output test proves the results are byte-identical.
