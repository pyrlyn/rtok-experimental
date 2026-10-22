# T960 validation

Host: macOS arm64. Commands use the repository's `mise` toolchain with
`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_INCREMENTAL=0`,
and `CARGO_BUILD_JOBS=2` to limit build storage.

## Regression proof

Before product changes:

- `cargo test -p rtok-log a_failed_rotation_keeps_the_incoming_line` failed with
  `IsADirectory` when rotation tried to replace a directory; the incoming line was lost.
- `cargo test --lib mcp::http::tests::http_expand_refuses_foreign_archives_but_keeps_its_own_lossless`
  failed because a foreign session's payload was returned successfully.

After changes:

- `cargo test -p rtok-log`: 12 passed, including failure/contended lock handling,
  Unix private modes and symlink refusal, and existing rotation behavior.
- `cargo test --lib mcp::http::tests`: 6 passed, including foreign full IDs and prefixes,
  duplicate content, restart isolation, stdio compatibility, and prefix collisions.
- `cargo test --test mcp_http`: 3 passed over actual loopback HTTP.
- `cargo test --lib expand::tests`: 28 passed.
- `cargo test -p rtok-store archive_in_session`: 4 passed.
- `cargo fmt --all` and `git diff --check`: passed.

The FIFO fixture is Linux-only because rustix does not expose `mkfifoat` on Apple targets.
It was not executed locally. Windows behavior was not executed locally either.

## Full gate

`env -u RUST_LOG mise exec -- just check` exited **0** with the build variables above:

- Formatting, all three Clippy invocations, JS/TS lint/format and the duplication gate passed.
- Python: 43 passed.
- Nextest: 4,193 passed, 8 skipped. One existing test,
  `agents::commandcode::tests::dry_run_names_the_settings_file_and_writes_nothing`, was
  marked `LEAK`; nextest counted it as passed and returned success.
- The minimal production build (`--no-default-features --features measure`) passed with
  unused-import/dead-code warnings in unchanged code.
- `ptools swarfr` reported that `swarfr` is not installed and skipped that check.
- Final `git diff --check` passed.

Earlier attempts found the root logging wrapper's outdated directory-count expectation
(fixed for the new lock file), exhausted build storage (only this worktree's cache was
cleaned), and reached 3,672 passing tests before `tui_without_a_tty_exits_one_with_one_clean_stderr_line`
failed because the host exports `RUST_LOG=warn`. That integration test does not clear the
variable and therefore receives an extra diagnostic line. The same test passes with
`env -u RUST_LOG cargo test --test tui_tty` (1 passed); product code was not changed for it.
None of the interrupted attempts is a passing full gate.

An exploratory `cargo test --lib --no-default-features` did not build because existing
tests reference disabled `compress`/`guard` plugins and graph request fields. The supported
default-feature tests above ran successfully; the mandatory gate includes its own minimal
production build. No dependency-advisory scan or benchmark was run.
