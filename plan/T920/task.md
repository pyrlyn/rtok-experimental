---
status: in progress
priority: P1
complexity: 3
readiness: 90%
agent: Claude Code / sonnet-5.5
---

# T920. PostToolUse hook went from ~4 ms to ~50 ms with v0.17.0-alpha.2 (memory observations)

`calls` (surface `hook`, name `PostToolUse`): mean 4.0 ms on 44,982 runs from 2026-10-12 to
the alpha.2 install (2026-10-18 21:29), 53.5 ms on 3,385 runs since, minimum 45 ms. Every
PostToolUse run on 2026-10-19 is over `max_ms` 10 ms. The first `observations` row (T454,
merged 2026-10-09, first shipped in alpha.2) has the same second as the first slow call, and
there is about one row per PostToolUse since. `rtok-hook` prints `{}` after 50 ms
(`src/bin/rtok-hook.rs`), so many PostToolUse answers are now dropped, not only late. T687's
12.5 ms baseline predates this.

Done when: PostToolUse is back near its pre-alpha.2 latency with observations on (the insert
leaves the hook path or gets cheap), and a latency test covers PostToolUse with observations
enabled.

Check: `tests/latency.rs` case with observations on, red before the fix; p50 on the live store
for a day after install recorded in this card; `just check`.

## Execution plan

Measured first (release build, `RTOK_HOME` copies, one `PostToolUse` per spawn with a fresh
`tool_use_id`): the insert is not the cost. In `observe::capture` the SQL insert takes ~0.3 ms and
`synthesize` ~7 ms, all of it compiling the scrub regex `(?i)<private>[\s\S]*?</private>`
(`(?i)` plus the `[\s\S]` union class folds every codepoint: 6.7 ms alone, 0.2 ms as
`(?is)<private>.*?</private>`). The installed alpha.3 (before T927 dropped the per-observation
measurement row) pays ~50 ms CPU more than a skipped tool; main since T927 pays ~8 ms.

1. `tests/latency.rs`: release-only case, a new call per spawn, interleaved with a skipped tool
   (`mem_save`) so load cancels; asserts the observations were stored and the median gap < 4 ms. Red first.
2. `src/plugins/memory/scrub.rs`: the equivalent cheap pattern, plus a multi-line/any-case unit test.
3. Verify: `cargo nextest run --release --test latency`, scrub unit tests, `just check`.
