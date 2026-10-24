---
status: todo
priority: P2
complexity: 3
readiness: 85%
agent:
---

# T991. Measure and reduce test launch time

Measure the existing benchmark fixture and TUI test runtimes, then reduce an evidenced bottleneck in fresh test fixture setup. Reuse existing config writers and skip only the annotated template initialization for fresh fixtures; preserve reload behavior and existing settings. Add regression coverage for path isolation, reload, and settings preservation. Verify with targeted before/after runs and just check.

## Execution plan

1. Time the existing TUI suite and shared configuration fixture separately from compilation.
2. In `src/testutil.rs`, initialize fresh fixture files without reference documentation, using the existing config writer and validator.
3. Prove that paths remain isolated after reload and existing settings survive repeated fixture preparation.
4. Compare the same tests before and after, run `just check`, and record the evidence and limitations.

Check: targeted before/after test runs and just check.

Implementation and focused fixture proof are complete. Full-suite verification remains blocked by local disk exhaustion and existing Rust 1.99 Clippy diagnostics; see `validation.md`. Resume the gate after the environment is ready.
