---
status: todo
priority: P1
complexity: 3
readiness: 0%
agent:
---

# T983. `rtok bench` scores each run, not each task

From `ideas.md` I-140 (`research.md` §47, T385.3 live half). Today `pass` counts tasks after all runs of an arm; later runs and arms start from earlier edits in whatever directory `rtok bench` started in; and a failed or timed-out run is averaged in as zero cost, so a broken arm looks cheap.

The default suite runs `claude -p` in a per-task fixture directory, resets it before every run, runs `check` after every run, and leaves a failed or timed-out run out of the cost mean while counting it as a failure. The §47 hand protocol (`--runs 1`, a `check` that resets the fixture) is no longer needed.

Done means: every run starts from a clean fixture, pass rate is per run, and the cost mean only covers runs that finished. Tests use a fake agent; no real agent runs (`AGENTS.md`).

Check: a test with a fake agent that edits the fixture and fails on one run proves the next run starts clean, the pass count is per run, and the failed run is not in the mean; `docs/agents/benchmarks.md` describes the new scoring.
