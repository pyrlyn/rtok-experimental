---
status: in progress
priority: P2
complexity: 2
readiness: 100%
agent: Claude Code / claude-sonnet-5-5
---

# T969. One slow-hook warning per event per interval, not one per call

`rtok.log` holds 27,107 `hook/PostToolUse … over max_ms 10 ms` warnings (some over 400 ms), one
per call. The flood hides every other warning in the log. The cause is T920. `max_ms` stays at
10 ms (the hook budget rule).

Done when: `note_slow` (`src/hooks/mod.rs`) writes at most one warning per event per interval
(configurable, a few minutes by default). That warning gives the count and the worst time since
the last one. The `calls` rows that `rtok stats` reads stay one per call.

Check: a unit test for the interval and the summary line; `just check`.

Plan: add `[hook] slow_warn_s` (default 300; 0 = warn on every slow call). `note_slow` keeps a per-event
`kv` row `hook:slow:<event>` (`since count worst`, one get plus one set, slow path only), so the state
survives across the short-lived hook processes. Under the interval it only updates the row; at the
interval it logs one summary (count, worst ms) and stderr prints the same line. Files:
`src/hooks/mod.rs`, `crates/rtok-config` (type + default.toml), `docs/config.md` (+ ru/uk), trycmd
fixtures. Verify: unit tests in `src/hooks/mod.rs`, `config_coverage`, `cli_trycmd`, `just check`.
