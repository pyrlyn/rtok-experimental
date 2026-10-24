---
status: done
priority: P2
complexity: 3
readiness: 100%
agent: OpenAI / GPT-6
---

# T991. Measure and reduce test launch time

Measure the full test suite and reduce evidenced fixture bottlenecks. Reuse existing config writers and skip only the annotated template initialization for fresh fixtures; preserve reload behavior and existing settings. Isolate the doctor CLI snapshot with trycmd's existing filesystem sandbox so it scans fixture data instead of the actual repository's worktrees. Preserve all test and output assertions. Verify with complete before/after runs and just check/full-check.

## Execution plan

1. Time the existing TUI suite and shared configuration fixture separately from compilation.
2. In `src/testutil.rs`, initialize fresh fixture files without reference documentation, using the existing config writer and validator.
3. Prove that paths remain isolated after reload and existing settings survive repeated fixture preparation.
4. Resolve the local disk and toolchain blockers without changing unrelated worktrees or the user's toolchain configuration.
   Fix the Dart LSP gate's dependency on the Rust checkout's server by selecting its fixture project explicitly and verifying an actual LSP measurement; use the same gate in both timing variants.
   Correct the two CLI snapshot aggregate counts left stale by T970's `worktree.min_free_mib` addition; preserve their existing content assertions.
   Isolate ambient logging in the two stderr contract tests, and update the unlinked-agent assertion for the existing actionable error explanation; preserve its refusal and no-new-worktree checks.
   Use the existing nextest exclusive scheduling mechanism for guard-check and the locked-store hook test: their unlocked setup writes failed under full-suite I/O pressure but passed in isolation. Preserve their assertions and time limits, and apply the scheduling equally to both timing variants.
   Await SessionEnd's documented deferred unlock before asserting ownership and GC behavior; cover both an available store and a deliberately held writer with the existing lock fixture.
   Keep worktree rollback/retry and existing-branch checks on their existing injected free-space reading; exercise real git without depending on the host's available bytes.
5. Run the complete workspace suite before and after with the pinned compiler, the same workload and thread count; compare execution separately from compilation and disclose any diagnostic timeout override.
   After baseline timings, sandbox `tests/trycmd/doctor.toml` using the existing shared `input` directory, preserving its empty stats fixture and every stdout assertion. The diagnostic process sample identified real-worktree size scans as the dominant delay.
6. Run `just check` with its full-check gates, then record the evidence and close the task.

Check: complete workspace tests before and after with comparable timings, regression coverage, and successful just check/full-check gates.

Complete: both variants passed all 4,236 tests (8 ignored); nextest time including setup fell from 435.341 s to 198.771 s (54.3%, 2.19x). Final `just check` passed the full gate. See `validation.md` for commands, timings and local measurement limits.
