# T920. Done

A `PostToolUse` observation no longer costs a regex compile per hook process.

- Measured first (release build, fresh `RTOK_HOME`, a new `tool_use_id` per spawn): in
  `observe::capture` the insert took ~0.3 ms and `synthesize` ~7 ms. The 7 ms was compiling
  `(?i)<private>[\s\S]*?</private>`: under `(?i)` the `[\s\S]` union class is case-folded over every
  codepoint (6.7 ms alone; `(?is)<private>.*?</private>` is 0.2 ms and matches the same text).
- `src/plugins/memory/scrub.rs`: the cheap pattern, a comment saying why, and a unit test for a
  multi-line, mixed-case tag.
- `tests/latency.rs`: `latency_hook_post_tool_observation_adds_under_4ms` (release only, like its
  siblings) spawns a new call each time, interleaved with a skipped `mem_save` call so machine load
  cancels, checks one observation was stored per call, and bounds the median gap. The identical-fixture
  case could not see this: its second spawn hits the 5 s dedup and never reaches the insert.
- Numbers (median gap of an observation over a skipped tool, n=200 interleaved): 8.5 ms before, 2.3 ms
  after. Installed alpha.3, which still wrote the measurement row T927 removed, was ~50 ms CPU above a
  skipped tool; main after T927 was +8.5 ms, so T927 took most of the reported 45 ms and this takes the
  rest. The live-store p50 a day after install is not recorded here.

PR: https://github.com/pyrlyn/rtok/pull/1280

Checked: new case red before the fix, green after; `just check` (fmt, lint, build, dup, changed tests)
passes except `worktree::mcp_worktree_tools_act_for_the_linked_agent`, which needs a live host agent in
the process ancestry and fails alone in this session; the other worktree and memory tests pass.
