# Local builds: one test binary and target/ size

How rtok keeps a local `target/` small and rebuilds short: one integration-test binary,
separate target dirs for build variants, and an optional compiler cache.

## How it works

**One integration-test binary (T985).** `Cargo.toml` sets `autotests = false` and declares a
single `[[test]] name = "it"` at `tests/it/main.rs`. Every former `tests/<name>.rs` is now
`tests/it/<name>.rs` and is the module `<name>` of that binary; shared helpers are
`tests/it/common/` (`use crate::common;`), snapshots are `tests/it/snapshots/`. A change in
`src/` relinks one executable instead of about 130, and `target/` holds one copy of the
dependency code instead of one per file. nextest still runs every test in its own process,
so process-wide state (env vars, current dir, globals) stays isolated as before.

**Other variants in their own target dir.** Changing `RUSTFLAGS`, `CARGO_PROFILE_DEV_*` or the
feature set changes the hash of every unit, so cargo builds a second full set next to the
first one in `target/debug/`. Build such variants with a separate `CARGO_TARGET_DIR`:

```bash
CARGO_TARGET_DIR=target/variant-nofeat cargo build --no-default-features --features measure
CARGO_TARGET_DIR=target/variant-cfg RUSTFLAGS="--cfg foo" cargo test --no-run
```

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

- Variant dirs: delete `target/variant-*/` to get the space back.
- sccache: remove `rustc-wrapper` / unset `RUSTC_WRAPPER`; `sccache --stop-server`.
- The single test binary is the layout of the repo, not a switch. New test files go to
  `tests/it/<name>.rs` plus one `mod <name>;` line in `tests/it/main.rs`.

## Test

```bash
cargo nextest run --workspace                       # everything, as before
cargo nextest run --test it -E 'test(/^worktree::/)'  # one former test file
cargo test --test it -- plan_dir:: ideas_md::        # libtest filters by module path
just test-changed                                   # picks modules by name from the diff
```

`tools/test-changed.sh` now maps a diff to module names in the `it` binary
(`binary(it) & test(/^(a|b)::/)`) and only adds `--test it` when a module is selected.

## Limitations

- A test file is no longer its own crate: `#![...]` crate-level attributes do not work in
  `tests/it/<name>.rs`, `crate::` means `tests/it/main.rs`, and `include_str!` paths are one
  level deeper (`../fixtures/...`, `../../docs/...`).
- `cargo test` (libtest, not nextest) runs all modules in one process, on threads; tests
  that change env vars or the current dir can race across modules there. `just test` and CI
  use nextest, where each test is its own process.
- Any change to any test module recompiles the whole `it` crate (seconds, not minutes; no
  dependency is rebuilt).
- A variant target dir is a full build the first time (nothing shared with `target/debug/`).
