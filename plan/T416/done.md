# T416. Done

The renderer left `src/render.rs` for the `change-preview` crate in `pyrlyn/crates`, released by that repository's release-plz pipeline. rtok depends on the crates.io version (`Cargo.toml`: `change-preview = "0.1.0"`, workspace dependency of the `rtok` crate) and `toolchain.md` lists it. T416.1-T416.4 moved every dry-run preview onto it in #993 (https://github.com/pyrlyn/rtok/pull/993).

Check: `change-preview` 0.1.0 is on crates.io (https://crates.io/crates/change-preview, published 2026-10-11, checked 2026-10-23); rtok's CI builds against it on `main`. The crate's own `cargo test` and `cargo clippy -- -D warnings` run in `pyrlyn/crates` CI before its release.
