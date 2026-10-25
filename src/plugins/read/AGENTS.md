# Agent notes — `read`

**Owns** `src/plugins/read/**` (`mod.rs`, `outline.rs`, `cache.rs`, `search.rs`, `hook.rs`),
tree-sitter grammar features `lang-*` in `Cargo.toml`.

**Contract**: the `Plugin` trait, `Ctx` and the host capabilities come from the published
`rtok-plugin-sdk` crate (`crates/rtok-plugin-sdk`), not from `crate::plugin` — import them as
`rtok_plugin_sdk::…`.

**Invariants**
- Root guard first: reject anything outside cwd/`allow_paths` (plus, in `rtok mcp` only, the worktrees of the cwd's and every `roots/list` root's repository, and those roots — `roots.rs`, T351, T435) before touching the filesystem.
- Capped output always carries an archive id; the full content is retrievable via `expand`.
- Dedup responses are < 80 chars and write a `Measurement { kind: "dedup" }`.
- Tool descriptions ≤ 60 estimated tokens each (T4.1 test enforces it).
- The Read-advice hook never denies files under `native_max_bytes`, nor a native `Read` with `limit` 1..=`range_max_lines` (default 300, T383; was 5 under T127) of any file — the edit gate stays cheap — nor a path `resolve` refuses or an image/PDF (T930: the host's own Read is the only tool that can show it). Every deny writes a `read`/`deny` cost row.

**Dependencies allowed**: `tree-sitter`, `tree-sitter-tags`, per-language grammars behind
features, `ignore` for gitignore-aware search. Justify each in the commit message.

**Checks**: `plan/` T4.2–T4.6; golden per-language fixtures are 20-line files.
