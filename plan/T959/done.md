# T959 done

PR: https://github.com/pyrlyn/rtok/pull/1247

- `docs/architecture/` holds 23 D2 diagrams with their committed SVGs and walkthrough pages: overview, layers, crates, hook/MCP/proxy flows, plugin contract, store, and one per plugin (15). `README.md` indexes them; `architecture.md` links it.
- `d2-render` (dev-dependency, native backend) renders in-process. `tests/diagrams.rs`, part of `just docs-check`, fails on a stale or missing SVG, a diagram without a page, or a page without its `docs/ru` / `docs/uk` translation. `just diagrams` re-renders and fails on constructs the native backend would drop.
- `docs/ru/architecture/` and `docs/uk/architecture/` translate every page. The landing sync excludes `architecture/`, because it mirrors only Markdown and the pages would lose their SVGs.
- `d2-render` needed no changes, because grids were already supported. The jactionlint baseline was refreshed for the deliberate `@main` pyrlyn/ci refs from #1152, which had been blocking the pre-push hook.
- The plugin README drift found along the way was fixed separately in T964 (#1248).

Checked: `cargo test -p rtok --test diagrams` passes with every SVG fresh and every page mirrored. `just docs-check` is green. Locally, `just check` passed fmt, clippy and dup, and 4189 of 4191 tests passed. The two `cli_trycmd` failures came from the shared `CARGO_TARGET_DIR` path in a report snapshot and are unrelated to this change. All relative links in the 72 pages resolve. Each SVG was viewed once in a browser.
