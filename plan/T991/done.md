# T991. Done

Reduced config fixture initialization and isolated the doctor CLI snapshot with the existing
trycmd filesystem sandbox. Resolved the full-suite blockers: stale snapshot totals, ambient
logging, Dart project selection, hook scheduling, the deferred SessionEnd unlock race and
worktree fixtures that accidentally depended on real free disk space.

The complete nextest run decreased from 435.341 s to 198.771 s including setup, excluding
compilation: 54.3% less time (2.19x faster) in the local successful before/after pair.
Both variants passed all 4,236 tests, with the same 8 ignored tests. See
[validation.md](validation.md) for commands, raw summaries, component timings and limitations.

Checked:
- Pinned Rust 1.98.1, 16 nextest threads; all fixes and regression cases held equal between
  variants. The baseline's temporary CLI timeout cap was not reached in its successful run.
- Final `just check` selected `full-check` and exited 0: formatting, Clippy, full nextest,
  minimal-feature build, duplication, JavaScript lint/format and 43 Python tests.
- Focused regression checks passed for config reload/path isolation, Dart LSP, environmental
  isolation, all three SessionEnd cases and all four worktree rollback/free-space cases.

PR: this branch.

Status: done 2026-10-24.

Model: OpenAI / GPT-6.
