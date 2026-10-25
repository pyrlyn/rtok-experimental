# docs plugin

![An MCP call resolves a crate from Cargo.lock and searches a locally cached rustdoc index; only rtok docs fetch reaches docs.rs and fills the cache](docs.svg)

Source: [`docs.d2`](docs.d2). Plugin docs: [README](../../../src/plugins/docs/README.md). Index: [all diagrams](../README.md).

`docs` gives an agent the documentation of the exact crate version a project builds with,
without pasting whole pages into the conversation. Three MCP tools resolve a dependency from
`Cargo.lock`, return ranked snippets from a cached docs.rs rustdoc index, and load one item
body on request. The plugin is off by default (`[plugins.docs] enabled = false`) and has no
hook methods.

## Walkthrough

1. **A tool call arrives.** The agent calls `docs_resolve`, `docs_query` or `docs_get` on
   `rtok mcp`. `src/mcp.rs` routes each name to the matching function in
   `src/plugins/docs/mod.rs`.
2. **Pick up workspace notes.** `docs_resolve` and `docs_query` first run `ingest_llms`: a
   `llms-full.txt` or `llms.txt` in the working directory is split on `#` headings into the
   pseudo-crate `llms`, filed under a version `local:<cwd>` so workspaces never see each
   other's text. Unchanged bytes touch nothing; a missing or empty file drops that workspace's
   cache.
3. **Resolve.** `resolve_lock` reads `Cargo.lock` from the working directory with the `toml`
   crate (no `cargo` subprocess) and returns the first package with that name. The name and
   version must pass `safe_part`, because both end up in a URL and a cache path.
   `docs_resolve` answers `{name, version, cached}`, where `cached` comes from
   `doc_crate_cached`.
4. **Miss without an error.** A name that is not in `Cargo.lock` gives `not in Cargo.lock:
   <name>`; a crate version that is not in the store gives `not cached: rtok docs fetch
   <name>`. Both are ordinary one-line results.
5. **Query.** For a cached version, `docs_query` calls `search_doc_items`, an FTS5 `MATCH` over
   `doc_items_fts`, limited to `query_limit` hits with `snippet_chars` per snippet. It then
   keeps snippets one by one while the token estimate of the JSON stays within `max_tokens`.
   No socket is opened.
6. **Measure.** When at least one snippet is returned, `docs_query` records a `Measurement`
   (`plugin: "docs"`, kind `query`, `ref_id` `<name>@<version>`) whose before side is the full
   text of every hit it considered.
7. **Load one item.** `docs_get(id)` returns the path, kind and full docs text of one cached
   item from `doc_items`, or fails with `unknown doc id`.
8. **Fetch, on request only.** `rtok docs fetch <name>` calls `run_fetch`, which resolves the
   version from `Cargo.lock`, GETs `https://docs.rs/crate/<name>/<version>/json.zst` through
   `http_get`, decodes the zstd stream and `index_rustdoc` keeps every item that has a docs
   string. The compressed file and its sha256 are written under
   `<home>/docs/<name>/<version>/`, and `replace_doc_crate` fills `doc_crates` and
   `doc_items`; a database trigger keeps `doc_items_fts` in step.

## Where it lives

| Box | Code |
| --- | --- |
| MCP routing | `src/mcp.rs` (`invoke`, `docs_resolve`, `docs_query`, `docs_get`) |
| Tool definitions | `src/plugins/docs/mod.rs` (`DocsPlugin::mcp_tools`) |
| `ingest_llms` | `src/plugins/docs/mod.rs` (`ingest_llms`, `split_llms`, `local_version`) |
| `resolve_lock` | `src/plugins/docs/mod.rs` (`resolve_lock`, `safe_part`) |
| Query and budget | `src/plugins/docs/mod.rs` (`docs_query`); `crates/rtok-store/src/docs.rs` (`search_doc_items`) |
| `docs_get` | `src/plugins/docs/mod.rs` (`docs_get`); `crates/rtok-store/src/docs.rs` (`get_doc_item`) |
| Fetch | `src/cli/docs.rs`; `src/plugins/docs/mod.rs` (`run_fetch`, `fetch_crate`, `http_get`, `index_rustdoc`) |
| Tables | `crates/rtok-store/src/schema.rs` (`doc_crates`, `doc_items`); migration `0039_docs_fts` (`doc_items_fts`) |
| Config | `crates/rtok-config/src/lib.rs` (`Docs`: `enabled`, `query_limit`, `snippet_chars`, `max_tokens`) |

## Invariants

- Default off, and no hook method: SessionStart, PreCompact and prompt submit are untouched.
- `docs_resolve`, `docs_query` and `docs_get` never open a socket; `rtok docs fetch` is the
  only network path.
- A crate name or version that is not path-safe is never used to build a URL or a file path.
- An unknown crate is a one-line miss, not an error exit.
- A query that returns snippets records `docs`/`query`; with no row there is no saving claim.
- Store SQL lives in `crates/rtok-store/src/docs.rs`; the FTS5 table is a virtual table the
  Diesel DSL cannot model.

## Related

[Overview](../overview.md) · [Plugin contract](../plugin-contract.md) ·
[MCP flow](../mcp-flow.md) · [read](read.md) · [memory](memory.md)
