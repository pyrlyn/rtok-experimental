# T991 validation

## Result

The complete nextest run decreased from **435.341 s to 198.771 s**, including setup but
excluding compilation: **54.3% less time, or 2.19 times faster**, in this local pair.
Both variants passed **4,236 tests**, with the same **8 ignored tests**. Final `just check`
selected `full-check` and exited 0.

## Changes and resolved blockers

- Fresh `testutil::config_file_in` fixtures start with an empty file through the existing
  atomic writer. `set_all_with` still validates and writes every rebased path, and
  `Config::load_from` still reloads it. Regression tests check reload equality, path
  containment, retained settings and creation of a missing parent directory.
- The doctor CLI snapshot uses trycmd's existing filesystem sandbox and shared input
  directory. It still runs the real CLI and checks the same stdout, including the ambient
  logging variant. Its empty transcript fixture is preserved. A process sample located the
  old delay in `agents::junk_worktrees::items`, walking the actual checkout's worktrees.
- The Dart LSP test selects its fixture project explicitly and requires an `lsp.outline`
  measurement. It no longer probes the Rust checkout's language server first.
- Snapshot totals left stale by T970 are corrected to 612 config lines and 427 keys.
  Two stderr contract tests clear inherited `RUST_LOG`. The unlinked-agent test checks
  the existing refusal and recovery hint, retaining its unchanged-inventory assertion.
- Guard-check and the locked-store hook test use the existing exclusive nextest scheduling
  mechanism. Their setup writes failed open under parallel I/O pressure but passed alone;
  their assertions and timing bounds are retained.
- The SessionEnd ownership test awaits the documented deferred unlock. Its failed fixture
  later had A unlocked and B still locked, proving the immediate assertion raced the child.
  Both an available store and a deliberately held writer are covered with the existing lock
  fixture; A/B ownership and GC assertions remain. The focused binary passes all three cases.
- Worktree rollback/retry and existing-branch checks keep using their existing injected
  `plenty` free-space reading with real git. Two calls accidentally used `Seams::HOST` and
  failed at the unrelated 2 GiB floor. All four rollback/free-space cases pass after the fix;
  explicit low-space refusal coverage is retained.

No dependency or production behavior changed. No test was removed or newly ignored.

## Measurement method

2026-10-24, macOS 27.0.1 (26A434), arm64 Mac15,9, 16 logical CPUs, 64 GiB RAM.
Rust/Cargo 1.98.1 (repository pin), rustc `48a229cea` (2026-09-01), nextest 0.9.143.
Both variants execute 4,236 tests across 145 binaries, with the same 8 tests ignored by
default. They use 16 test threads, `RUST_LOG=warn`, `CARGO_INCREMENTAL=0` and
`CARGO_PROFILE_DEV_DEBUG=0`, with no retries.

The baseline restores only the old config helper initialization and doctor fixture.
All blocker fixes and added regression cases are identical in both variants.

```sh
RUST_LOG=warn CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 \
  mise exec rust@1.98.1 -- cargo nextest run --workspace \
  --test-threads 16 --no-fail-fast \
  --config-file .rtok/t991-benchmark-nextest.toml

RUST_LOG=warn CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 \
  CARGO="mise exec rust@1.98.1 -- cargo" just check
```

The ignored measurement config is a copy of `.config/nextest.toml` with only the
`binary(cli_trycmd)` termination cap extended from 180 to 600 seconds. A previous baseline
hit 180 seconds under external CI load. Neither CLI case in the successful baseline reached
180 seconds, so the extension did not affect its completed workload. The optimized
`just check` uses the committed config and ordinary limits; shared-input changes select
`full-check`, including the complete nextest suite.

| Measurement | Baseline | Optimized |
| --- | ---: | ---: |
| nextest total, including setup | 435.341 s | 198.771 s |
| setup | 0.130 s | 4.308 s |
| execution after setup | 435.211 s | 194.463 s |
| test compilation reported by Cargo | 8.01 s | 37.33 s |
| CLI snapshots (`cli`) | 106.107 s | 1.048 s |
| CLI snapshots with ambient logging | 101.815 s | 0.985 s |
| result | 4,236 passed; 8 ignored | 4,236 passed; 8 ignored |

Execution excluding setup decreased by 55.3% (2.24 times faster). The headline includes
setup. Baseline whole-command wall time was 458.69 s; final `just check` was 323.32 s.
The latter also includes lint, minimal-feature build and other gates, so these two command
wall times are not used to calculate the speedup.

Local evidence is retained under `.rtok/` in this worktree:
`t991-baseline-sample6.log`, `t991-final-check3.log`, and their `.time` files.

```text
Summary [435.341s] 4236 tests run: 4236 passed (2 slow), 8 skipped
Summary [198.771s] 4236 tests run: 4236 passed, 8 skipped
```

This is one local before/after pair on a shared machine, not a cross-host guarantee.
External Actions archive jobs, builds and desktop applications varied during the investigation;
no external build/archive writers were observed during the final pair's test-phase polls.
Every failed or incomplete run is excluded from the comparison, including ENOSPC, the earlier
CLI timeout, the deferred-unlock race and the host free-space fixture failures. Earlier
4,235-test diagnostics preceded the added SessionEnd regression variant.

## Verification

Final `just check` passed formatting, all three Clippy commands, all 4,236 workspace tests,
the minimal-feature build, duplication detection (408 clones; within the configured limit),
JavaScript lint/format and all 43 Python tests. The optional `swarfr` step was a documented
no-op because that tool is not installed. The command exited 0.

The repository's pinned compiler resolved the earlier Clippy blocker without suppressions
or changes to the main checkout's pre-existing `mise.toml` edit. Disk recovery included
removing a reproducible extracted VM base verified against its retained archive and cleaning
only this task's finished test fixtures. Other worktrees and CI processes were left intact.

## Earlier helper-only microbenchmark

Five alternating old/new pairs on macOS arm64 with Rust 1.99.1 measured exact helper bodies
linked against the same existing libraries. Median fixture preparation decreased from
0.719591 s to 0.030796 s (23.4 times faster). Correctness assertions ran outside the timed
interval and covered reload equality, containment, existing settings and old/new config
equality at the same path. See [raw fixture timings](fixture-timing.md).

That diagnostic measures only fixture preparation. It is separate from the full-suite
comparison above, and neither measurement is a token-saving claim.
