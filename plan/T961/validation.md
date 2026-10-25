# T961 validation

Host: macOS arm64, repository mise toolchain. Build environment:
`CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2`.
Commands unset inherited `RUST_LOG=warn` to keep the existing no-TTY stderr test isolated.

## Regression proof

With the project policy temporarily bypassed (the baseline's unrestricted behavior),
`cargo test -p rtok-config project_ -- --nocapture` failed the new TOML and dotenv tests:
the TOML telemetry endpoint became `https://attacker.invalid/collect`, and dotenv accepted
the project override without warning. Four other selected tests passed. The bypass was removed.

The first green config run passed 97 tests. Subsequent review found an interaction with
trusted legacy settings; ignored-source tagging now happens after legacy folding and a
third regression covers user legacy config, environment and CLI flag precedence.

Final focused check: `cargo test -p rtok-config` passed all **98** unit tests and
0 doc-tests. `cargo fmt --all` and `git diff --check` passed.

`just check` selected `full-check`. Earlier attempts found a Clippy `collapsible_if`
warning (guard rewritten as a let-chain), a stale `config init --dry-run` snapshot
(updated for two new guidance comments), and a task-card formatting error (`Check:`
now starts its own line). None of those interrupted runs is a passing gate.

Final `env -u RUST_LOG mise exec -- just check` with the build environment above exited
**0**, selecting `full-check`:

- Formatting, jscpd, JS/TS lint/format and all three Clippy invocations passed.
- Python: **43 passed**.
- Workspace nextest: **4,191 passed, 8 skipped**. Existing test
  `agents::cursor::tests::dry_run_offer_names_plugin_and_local` was marked `LEAK`, counted
  as passed by nextest. Run ID: `50e37c99-0dc9-48c2-b6fb-67a469310ae4`.
- Minimal production build (`--no-default-features --features measure`) passed with
  unused-import/dead-code warnings in unchanged code.
- `ptools swarfr` skipped its check because `swarfr` is not installed.
- Targeted `cargo test --test plan_dir`: **2 passed** after the task-card fix.
- `git diff --check` passed.

## Design and compatibility

The config crate remains the only config I/O owner. Reused figment providers, the existing
environment alias resolver, legacy folding and load-note surfaces. No dependency, schema
field or version changes. A source-specific policy is needed because schema validation
alone cannot distinguish a checkout-controlled endpoint from a user-owned one.

Both automatic project sources share one restriction function. Rejected dotenv values
are removed before first-definition precedence, so they cannot mask home dotenv values.
Ignored-source rows show only retained safe values; warnings name the path and key.
Parse errors retain the existing fail-open path. Explicit `--config`/`RTOK_CONFIG`, process
environment and home dotenv remain deliberate trusted inputs. No interactive trust store
or command was added.

Restrictions cover entire sensitive sections and additional host/command/path settings,
including legacy `core` logging and `dashboard` aliases. Safe project options keep their
provenance and linked worktrees still use their own checkout's config. Some formerly
accepted project settings must move to user config; the English, Russian and Ukrainian
config guides list the policy and migration options.

No Linux/Windows execution, benchmark or dependency-advisory scan was performed.
