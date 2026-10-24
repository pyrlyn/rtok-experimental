# T991 validation

## Change

Fresh `testutil::config_file_in` fixtures start with an empty file written through
the existing atomic config writer. The existing `set_all_with` still writes and
validates every rebased path, and `Config::load_from` still loads the result.
Existing files are preserved. No dependency, production config initialization,
test selection, timeout, or concurrency setting changed.

Previously each fresh fixture initialized the annotated reference config and
`reveal_commented_key` scanned that whole document once for each path. TUI, web,
graph model, and other tests reuse this helper. The existing config writer and
validator already provide the necessary implementation; no new helper or package
was introduced.

Regression coverage checks paths after a further reload and an existing setting
after repeated fixture preparation, including creation of a missing parent.

## Local diagnostic timings

Measured on 2026-10-24, macOS arm64, Rust 1.99.1, baseline `31633156a`.
These are fixture preparation timings, not token savings or a whole-suite speedup.

Both exact `config_in` and `config_file_in` function bodies were extracted from
the baseline and working source and linked into one small unoptimized Rust driver
against the same already-built `rtok_config` and `serde_json` libraries. This
avoided rebuilding the full application after the disk filled. Each sample used
a fresh nested directory; before/after order alternated. Compilation and
subsequent correctness assertions were outside the timed interval.

| Pair | Before, seconds | After, seconds |
| --- | ---: | ---: |
| 1 | 0.719591 | 0.030796 |
| 2 | 0.709703 | 0.025899 |
| 3 | 0.723630 | 0.027488 |
| 4 | 0.709412 | 0.031208 |
| 5 | 0.723215 | 0.033262 |

Median: 0.719591 s to 0.030796 s, approximately 23.4 times faster for this helper.
The same driver passed assertions for reload equality, containment of every
absolute path, retention of `tui.tab = "plugins"` across repeated setup, and
equality of the old and new configurations created at the same path.
Raw sample output is in [fixture-timing.md](fixture-timing.md).

Driver invocation used `mise exec rust@1.99.1 -- rustc --edition=2024` with
`--extern rtok_config`, `--extern serde_json`, and `-L dependency=target/debug/deps`.
Execution used private `HOME` and `TMPDIR`, `RTOK_HOST_SANDBOX=1`, and
`RTOK_WORKTREE_MIN_FREE_MIB=0`. No real agents were invoked.

Before the change, nextest ran 91 TUI tests successfully in 20.319 s after a
92-second dependency build. A warm run including the existing path-isolation
test passed 92 tests in 13.775 s. The old unit binary directly ran the 91 TUI
tests in 11.84 s with 16 threads. Concurrent machine activity makes these
diagnostic baselines unsuitable for a whole-suite speedup claim. There is no
post-change TUI-suite measurement.

## Verification and remaining gate

- Exact before/after fixture driver: passed all assertions and five timing pairs.
- `mise exec rust@1.99.1 -- cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.
- Focused nextest with the repository's pinned Rust 1.98.1: compilation failed
  with `No space left on device`; tests did not run.
- Retry with Rust 1.99.1 (the main checkout's pre-existing local toolchain
  setting), `CARGO_INCREMENTAL=0`: the root test binary again could not be built
  because the disk filled. Neither toolchain configuration was edited.
- `just check` using `CARGO="mise exec rust@1.99.1 -- cargo"`: stopped at Clippy
  on 34 existing `redundant_field_names` diagnostics in unchanged `rtok-store`
  code; the run also reported deprecated `Atomic::fetch_update` in `rtok-sys`.
  It did not reach the test gate. Those unrelated files were not changed.

Only this task's failed build artifacts were cleaned. Other worktrees, the
concurrent runner, and the user's existing `mise.toml` edit were left intact.
The task remains open until focused nextest and `just check` can complete.
