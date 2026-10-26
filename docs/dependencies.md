# Dependencies: optional stacks, duplicates, unused crates

This page explains how rtok keeps its dependency graph small, what is behind a feature, and how
to check it.

## How it works

Three rules:

1. **Heavy, rarely used output formats are opt-in.** The default build links neither the PNG
   rasteriser nor the PDF renderer:

   | Feature     | Turns on                                                                  | Used by                          |
   |-------------|---------------------------------------------------------------------------|----------------------------------|
   | `graph-png` | `resvg`, `usvg`, `tiny-skia`, the font stack (`fontdb`, `skrifa`, `harfrust`, `read-fonts`) | `rtok graph export --format png`, the TUI and web page PNG export |
   | `pdf`       | `printpdf`, `lopdf`, `allsorts-azul`                                      | `rtok report --format pdf`       |

   Without the feature the command still parses (`--format png` / `--format pdf` stay in the
   CLI and in the web protocol schema) but fails with a message that names the feature. JSON,
   SVG, Markdown and HTML are unaffected. `graph-png` implies `graph`.

2. **One version per crate where we choose it.** Our own requirements follow the version the
   rest of the tree already links (for example the `tokio-tungstenite` dev-dependency follows
   axum 0.8), so `cargo tree -d` lists only duplicates that come from third-party crates.

3. **No unused crates.** `cargo machete --with-metadata` must report nothing except
   `workspace-hack` (hakari-managed, a known false positive). Crates that are listed only to
   switch on a feature are on the crate's `[package.metadata.cargo-machete] ignored` list with
   a comment (`libsqlite3-sys` in `rtok-store`, which enables the bundled SQLite with FTS5).

## Enable

```sh
cargo build --features graph-png,pdf          # both formats
cargo build --features pdf                    # PDF reports only
cargo install --path . --features graph-png   # an installed binary with PNG export
```

## Disable

They are off by default; a plain `cargo build` / `cargo test` does not compile them. To drop
other plugins too, use `--no-default-features --features ...` (see `just build-min`).

## Test

```sh
cargo nextest run --workspace                            # default features: no font/PDF crates
cargo nextest run --workspace --features graph-png,pdf   # adds the PNG and PDF tests
cargo tree -e normal,dev | grep -E 'resvg|printpdf'      # empty on a default build
cargo tree -d -e normal,dev,build                        # remaining duplicate versions
cargo machete --with-metadata                            # only workspace-hack lines
cargo hakari generate && cargo hakari verify             # after any dependency change
```

`tests/deps.rs` guards the rules: `font_and_pdf_stacks_are_not_default` fails if either
feature lands in `default` or if `resvg`/`printpdf` stop being optional. PNG and PDF tests
(`tests/graph_export_image.rs`, `tests/report_pdf.rs`, the draw/exporter unit tests) are
`#[cfg(feature = ...)]` and run only under their feature; without it a test checks that the
error names the feature.

## Remaining duplicate versions

These come from third-party crates and cannot be unified from this repository today:

| Crate (versions)                                   | Why two copies                                                        |
|----------------------------------------------------|-----------------------------------------------------------------------|
| `base64` 0.22 / 0.23                               | axum 0.8, httpmock and tiktoken-rs are on 0.22; hyper-util and reqwest moved to 0.23 |
| `sha2`/`digest`/`block-buffer`/`crypto-common`/`cpufeatures` 0.10 / 0.11 | the workspace is on sha2 0.11; the `d2-render` dev-dependency still uses 0.10 |
| `rand`/`rand_core`/`getrandom` 0.9 / 0.10 (0.3 / 0.4) | tungstenite 0.29 (axum) is on rand 0.9; proptest and rmcp are on 0.10 |
| `darling` 0.21 / 0.24, `syn` 2 / 3                 | diesel's `dsl_auto_type` is on darling 0.21 / syn 2                    |
| `toml` 0.8 / 0.9, `toml_edit`, `toml_datetime`, `serde_spanned`, `winnow` | figment 0.10 needs toml 0.8; diesel_migrations uses 0.9; we use toml_edit 0.25 |
| `similar` 2 / 3                                    | insta and httpmock (dev) are on 2; change-preview is on 3              |
| `thiserror` 1 / 2, `downcast-rs` 1 / 2             | portable-pty (dev) via filedescriptor                                  |
| `bit-set`/`bit-vec` 0.8 / 0.11                     | fancy-regex (jsonschema, tiktoken-rs; dev only) vs regex-automata users |
| `shlex` 1 / 2                                      | `cc`/bindgen build scripts are on 1; rtok uses 2                       |
| `supports-color` 2 / 3                             | owo-colors 4 on 2, other crates on 3                                   |
| `serde_json` (build)                               | the same version twice: once for proc-macros/build scripts (host), once for the target |

`find-msvc-tools` shows up on macOS because `cc` depends on it unconditionally (not
target-gated). It is a build-script dependency compiled for the host; it is never linked into
rtok or a test binary.

## Limitations

- A binary built without `graph-png` cannot export PNG; the web page's PNG button returns the
  error. Release builds that should offer PNG/PDF must pass the features explicitly.
- Most remaining duplicates need upstream releases (axum, diesel, figment, d2-render).
- `lang-kotlin` and `lang-csharp` keep only the hand-written tags queries; their grammar crates
  were never loaded and are no longer dependencies. Real Kotlin/C# parsing would need them back.
