# Local builds: one test binary, profiles and target/ size

How rtok keeps a local `target/` small and rebuilds short: one integration-test binary, named
cargo profiles instead of environment flags, split debug info, separate target dirs for
other variants, and an optional compiler cache.

## How it works

**One integration-test binary (T985).** `Cargo.toml` sets `autotests = false` and declares a
single `[[test]] name = "it"` at `tests/it/main.rs`. Every former `tests/<name>.rs` is now
`tests/it/<name>.rs` and is the module `<name>` of that binary; shared helpers are
`tests/it/common/` (`use crate::common;`), snapshots are `tests/it/snapshots/`. A change in
`src/` relinks one executable instead of about 130, and `target/` holds one copy of the
dependency code instead of one per file. nextest still runs every test in its own process,
so process-wide state (env vars, current dir, globals) stays isolated as before.

**Profiles instead of flag churn.** Changing `CARGO_PROFILE_DEV_DEBUG`, `RUSTFLAGS` or the
feature set changes the hash of every unit, so cargo builds a second full set next to the
first one in `target/debug/`. The common switches are named profiles that build into their
own dir and leave `target/debug/` alone:

| profile | dir | what changes |
|---|---|---|
| `dev` (default) | `target/debug/` | line tables only, no debug info for dependencies |
| `dev-nodebug` | `target/dev-nodebug/` | no debug info at all |
| `dev-fulldebug` | `target/dev-fulldebug/` | full DWARF, dependencies included |

**Split debug info.** `[profile.dev] split-debuginfo = "unpacked"`: on macOS the DWARF stays
in the object files and no `dsymutil` runs per link; on Linux it goes to `.dwo` files beside
the objects, so the linker does not copy it into every executable. Windows (MSVC) writes
`.pdb` files anyway and ignores the setting.

**Other variants in their own target dir.** `RUSTFLAGS` and feature sets cannot be a profile.
Build them with a separate `CARGO_TARGET_DIR` so they do not double `target/debug/`:

```bash
CARGO_TARGET_DIR=target/variant-nofeat cargo build --no-default-features --features measure
CARGO_TARGET_DIR=target/variant-cfg RUSTFLAGS="--cfg foo" cargo test --no-run
```

**sccache (optional, per machine).** Not configured in the repo: a repo-level
`rustc-wrapper` would fail for everyone without sccache installed. CI has its own setup.
Locally, set it in your user config or shell (see Enable).

## Enable

Nothing to enable for the test binary, the profiles and split debug info: they are in
`Cargo.toml`. Use a profile with `--profile`:

```bash
cargo build --profile dev-nodebug
cargo nextest run --cargo-profile dev-nodebug
```

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

- Profiles: do not pass `--profile`; `dev` is unchanged. Delete `target/dev-nodebug/` or
  `target/dev-fulldebug/` to get the space back.
- Split debug info: `CARGO_PROFILE_DEV_SPLIT_DEBUGINFO=packed` (macOS: a `.dSYM` per
  binary) or `off`, for one build.
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
- A new profile is a full build the first time (its own dir, nothing shared with `dev`).
- Split debug info: tools that expect a `.dSYM` (some macOS crash reporters) need `packed`.
