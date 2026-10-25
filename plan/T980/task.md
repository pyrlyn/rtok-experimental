---
status: todo
priority: P1
complexity: 3
readiness: 0%
agent:
---

# T980. One wall-clock budget for the junk section of `rtok doctor`

From `ideas.md` I-132. In T666 `rtok doctor`, run from a checkout with about ten worktrees of 20+ GB `target/` each, was still in `worktree::list::walk` after 4 minutes. From the code: `junk_worktrees::items` gives each worktree its own `AGENT_SCAN_LIMIT` (10 s) in `disk_usage_until`, so the bound grows with the worktree count, and `gc::judge`'s `newest_until` has no deadline at all. The Hosts page junk card shares the path.

Step 1 times where the minutes go (`list::rows`, `gc::judge`, the size walks) on such a checkout and picks the budget from that. Then one deadline covers the whole section: past it, print the sizes measured so far as a lower bound and a line saying the scan was cut at N s, pointing at `rtok agents junk list`.

Done means: `doctor` and the junk card return within the budget whatever the worktree count, and the cut is visible in the output.

Check: a test with fake worktrees whose size walk blocks past the budget returns within the budget plus a small margin and prints the cut line; the timing from step 1 is in `research.md`.
