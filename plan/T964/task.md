---
status: in progress
priority: P3
complexity: 1
readiness: 100%
agent: Claude Code / claude-opus-5-5
---

# T964. Plugin READMEs match the code

Docs. Drawing the per-plugin D2 diagrams for T959 showed several plugin docs that no longer
describe the code: `toon` claims default off and "Manifest only" while `default_on: true` and
`proxy_filter` ship; `inject` names the removed `core.inject_budget_tokens`; `json_tree` calls
its MCP surface "MCP tool results" though only the `rtok mcp -- <server>` wrapper folds;
`guard` omits `deny_grep_glob`; the SDK plan names a `[plugins.wasm] fuel_per_call` key that the
config rejects; `archive` (and `inject`, `guard`) still say "Manifest only".

Done: each of those docs says what the code does, the code is unchanged, and the matching rows
of `docs/plugins.md` with its `docs/ru` / `docs/uk` translations agree.

Check: every changed claim is traced to the code (`Manifest`, `crates/rtok-config`,
`src/plugins/wasm.rs`, `src/mcp/wrap.rs`); `just docs-check`.

Plan: read each plugin's `mod.rs` and its `[plugins.<id>]` section in
`crates/rtok-config/src/lib.rs` and `default.toml`; fix `src/plugins/{toon,inject,json_tree,guard,archive}/README.md`
and `crates/rtok-plugin-sdk/PLAN.md` (the wasm sandbox paragraph); fix the `toon` row of
`docs/plugins.md` and its translations; run `just docs-check`.
