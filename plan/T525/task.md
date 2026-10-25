---
status: in progress
priority: P2
complexity: 3
readiness: 60%
agent: Claude Code / sonnet-5.5
---

# T525. SubagentStart and PreCompact under the 10 ms hook budget

T428 covers SessionStart. On the live store for 2026-10-03 through 2026-10-10, SessionStart's median is already 3.8 ms (149 of 609 runs over 10 ms; the tail is the quiet-host re-run T428 left open). Two other events miss `[hook] max_ms` on the median. PreToolUse's median is 0.8 ms, so the budget is reachable. About 7,500 `hook … slow` warnings this week are mostly the PreToolUse and PostToolUse tail; that tail is not this task.

| event | n | p50 | p95 | max | over 10 ms | over 1 s |
| --- | --- | --- | --- | --- | --- | --- |
| SubagentStart | 758 | 48.6 ms | 760 ms | 3093 ms | 605 | 24 |
| PreCompact | 189 | 25.7 ms | 206 ms | 2551 ms | 167 | 1 |

Done when: a trace names the work inside SubagentStart and PreCompact. That work is under 10 ms at the median on this store, or the card records why those two events get a higher budget. `tests/latency.rs` covers both.

Check: the latency cases green; five manual runs of each event on the rtok repo print no `slow` warning; `just check`.

## Execution plan

1. Add `tests/latency.rs` cases that seed what a live store holds (40,000 hook rows of other sessions, a 100-row ledger, 40 notes, a 2 MB transcript) and compare each event's median with a `PreToolUse` spawned right before it, so host load drops out.
2. Trace each event in a release build with a temporary per-phase timer (dropped before the commit) and name the work.
3. Fix what dominates in `SubagentStart`; record `PreCompact`'s cost and budget in `done.md`.
4. Check: `cargo nextest run -p rtok-store`, the latency cases in release, `just check`.
