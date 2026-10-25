---
status: todo
priority: P1
complexity: 4
readiness: 0%
agent:
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
