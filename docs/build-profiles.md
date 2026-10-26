# Local builds: one test binary and target/ size

How rtok keeps a local `target/` small and rebuilds short: one test executable for the root
crate, line-table-only debug info, one shared `target/debug/` for every build, and an optional
compiler cache.

## How it works

**One test executable (T985).** The root crate `rtok` has exactly one test executable: the
lib's unit-test binary (`target/debug/deps/rtok-<hash>`). The integration tests are compiled into
it: `src/lib.rs` ends with `#[cfg(test)] #[path = "../tests/it/main.rs"] mod it;` and declares
`#[cfg(test)] extern crate self as rtok;`, so `use rtok::...` in a test keeps working. Every former
`tests/<name>.rs` is `tests/it/<name>.rs`, the module `it::<name>`; shared helpers are
`tests/it/common/` (`use crate::it::common;`), snapshots are `tests/it/snapshots/`.
`Cargo.toml` sets `autotests = false` and `test = false` on both bins (`rtok`, `rtok-hook`), the
examples and the bench, so `cargo test` / `cargo nextest` build no other test executable for this
package. A change in `src/` relinks one executable instead of about 130 (or three: lib unit tests,
the bin's unit tests, `it`), and `target/` holds one copy of the code instead of one per binary.
nextest still runs every test in its own process, so process-wide state stays isolated.

Tests that run the CLI call `crate::it::common::bin_path("rtok")` (or `"rtok-hook"`): cargo sets
`CARGO_BIN_EXE_<name>` only for `[[test]]` targets, so the path is found at run time next to the
test executable (`target/<profile>/rtok`). Under nextest the setup script `test-bins-*` in
`.config/nextest.toml` builds both binaries before the run; under `cargo test` the helper runs
`cargo build -p rtok --bin <name>` once per process (a no-op when up to date). CI builds them
before `cargo nextest archive`, the archive carries them (`[profile.default.archive]`), and the
test jobs set `RTOK_TEST_BINS_READY=1` to skip the build.

`cfg(test)` is now on for the integration tests too. Code that takes a shortcut under
`cfg!(test)` changes what they see; `model::BACKGROUND_STATS_IN_TESTS` turns the real Stats
scan back on for `it::stats_model`.

The other workspace members (`rtok-store`, `rtok-config`, `rtok-agent-sdk`, ...) still have their
own small unit-test binary each; moving them is a follow-up.

**Debug info: line tables for workspace code, none for dependencies.** `Cargo.toml` sets
`[profile.dev] debug = "line-tables-only"` (backtraces keep `file:line`, full DWARF is gone)
and `[profile.dev.package."*"] debug = false` (the ~450 dependencies carry no debug info;
`"*"` does not match workspace members). Build scripts and proc macros also get
`debug = false` via `[profile.dev.build-override]`. This shrinks rlibs, `incremental/` and the
`it` binary, and cuts link time. `profile.dist` inherits `dev`, so releases get the same.

**One shared target dir, no variant dirs.** Changing `RUSTFLAGS`, `CARGO_PROFILE_DEV_*`,
`debug` or the feature set changes the hash of every unit, so cargo builds a second full set of
artifacts. Do not build such variants routinely, and do not give them their own
`CARGO_TARGET_DIR`: a separate dir per variant is a full copy each and the
pruner cleans each dir on its own, so the total grows with every variant. Do not build a
`debug = false` (or full-debug) second set in `target/debug/` either. Build with the defaults in
`Cargo.toml`; when a one-off variant is unavoidable, build it in the shared `target/` and run
`target-prune` (with `--max-size`) afterwards, or `cargo clean` when done.

**sccache (optional, per machine).** Not configured in the repo: a repo-level
`rustc-wrapper` would fail for everyone without sccache installed. CI has its own setup.
Locally, set it in your user config or shell (see Enable).

## Enable

Nothing to enable for the test binary: it is in `Cargo.toml`.

sccache, per machine (`mise use -g sccache` or `cargo install sccache`), either in
`~/.cargo/config.toml`:

```toml
[build]
rustc-wrapper = "sccache"
```

or per shell: `export RUSTC_WRAPPER=sccache`. Check hits with `sccache --show-stats`.
sccache does not cache incremental units, so workspace crates still compile on a cache hit
for dependencies; `CARGO_INCREMENTAL=0` makes them cacheable too, at the cost of slower
small edits.

## Disable

- Full debug info for a one-off debugger session, without editing `Cargo.toml`:
  `cargo build --config 'profile.dev.debug=true' --config 'profile.dev.package."*".debug=true'`.
  It rebuilds every crate with a second hash in `target/debug/`; `cargo clean` (or
  `target-prune`) afterwards.
- sccache: remove `rustc-wrapper` / unset `RUSTC_WRAPPER`; `sccache --stop-server`.
- The single test executable is the layout of the repo, not a switch. New test files go to
  `tests/it/<name>.rs` plus one `mod <name>;` line in `tests/it/main.rs`. To get a separate
  `[[test]]` back, remove the `mod it` from `src/lib.rs`, add `[[test]] name = "it"`, and replace
  `crate::it::` with `crate::` in `tests/it`.

## Test

```bash
cargo nextest run --workspace                          # everything, as before
cargo nextest run -p rtok --lib -E 'test(/^it::worktree::/)'  # one former test file
cargo test -p rtok --lib -- it::plan_dir:: it::ideas_md::      # libtest filters by module path
just test-changed                                      # picks modules by name from the diff
cargo test --no-run -p rtok 2>&1 | grep Executable      # exactly one line for rtok
```

`tools/test-changed.sh` maps a diff to `test(/^it::(a|b)::/)` for integration modules and
`kind(lib) & !test(/^it::/)` for unit tests, all in the one lib test binary.

## Limitations

- No local variables or types of dependencies in a debugger; stepping into a dependency
  shows assembly. Workspace code has line info only (breakpoints by line work, variable
  inspection is limited).

- A test file is no longer its own crate: it is a module of the lib, compiled with
  `cfg(test)` and the lib's lints (`#![forbid(unsafe_code)]`). `crate::` is the lib root, so
  test helpers are `crate::it::...`; `include_str!` paths are one level deeper
  (`../fixtures/...`). Integration tests can see `pub(crate)` items; keep them on the public API.
- The tested `rtok`/`rtok-hook` binaries are built with the default features, whatever features
  the test run uses.
- `cargo nextest archive` no longer carries `target/debug/rtok` as a test dependency; a test
  machine without it builds it once through `bin_path` (needs the source and cargo).
- `cargo test` (libtest, not nextest) runs all modules in one process, on threads; tests
  that change env vars or the current dir can race across modules there. `just test` and CI
  use nextest, where each test is its own process.
- Any change to any test, or to `src/`, recompiles the `rtok` lib in test mode together with all
  integration tests (no dependency is rebuilt).
