# T964. Done

Docs only; no code changed. Each claim was read off the code on `main` at `b7183a941`:

- `toon`: `default_on: true` and `proxy_filter` (`src/plugins/toon/mod.rs`); the proxy calls it
  in `compress` only (`src/proxy/mod.rs`). README now says on, proxy plus MCP `expand`,
  implemented. The `toon` row of `docs/plugins.md` (and `docs/ru`, `docs/uk`) said `MCP`; it
  now says `proxy, MCP \`expand\``.
- `inject`: the key is `plugins.inject.budget_tokens`; `core.inject_budget_tokens` is the T12.1
  legacy spelling carried over by `crates/rtok-config`. README config block and step 3 fixed.
- `json_tree`: besides `proxy_filter`, the only MCP path is `json_tree_fold` in
  `src/mcp/wrap.rs` (the `rtok mcp -- <server>` wrapper), which skips `read` and `search`.
- `guard`: documented `deny_grep_glob` (T50.4) from `native_redirect` and its test.
- wasm: `crates/rtok-plugin-sdk/PLAN.md` now names the fixed `FUEL` budget; the config rejects
  `fuel_per_call` (`wasm_unknown_key_is_denied`), and no `Store` memory limit is set.
- `archive`, `inject`, `guard`, `toon`: "Manifest only" replaced with what ships.
- Every plugin README's `Spec` row pointed at a `spec (replaces)` catalogue in `plan.md` §1
  that never existed under that name; it now links the **Replaces** column of
  `docs/plugins.md`.
- `docs/plugins.md` (and `docs/ru`, `docs/uk`): "no WASM in v0.1" replaced with the
  `wasm-host` feature (not default) plus `[plugins.wasm] enabled`; the `archive` row now lists
  proxy, MCP `expand` and `rtok archive rewrite` (manifest: proxy, MCP, CLI), the `guard` row
  adds `rtok guard check` (manifest: hook, CLI).

PR: see the T964 PR on pyrlyn/rtok. Checked with `just docs-check`.
