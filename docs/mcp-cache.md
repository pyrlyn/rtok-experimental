# MCP tool-result memo

`rtok mcp` memoizes the results of read-only tools, so an identical repeated request is
answered without redoing the store / FTS work. The memo is transparent to any client model or
agent: a hit returns exactly the bytes the real call returned for the identical request.

## How it works

- **What is cached:** only `docs_resolve`, `docs_query`, `docs_get`, `mem_search`, `mem_get`
  and `mem_pack` (`MEMO_TOOLS` in `src/mcp/memo.rs`). File-system tools (`read`, `search`,
  `tree`, graph) are never cached because files change without any MCP call. Writing and
  stateful tools are never cached either.
- **Key:** the tool name, the caller scope (cwd and linked agent id) and every argument,
  `agent` included. Object keys are sorted; array order is kept. The full key is stored and
  compared on every read, so a `key_hash` collision can never serve another request's value.
- **Two layers:** an in-process LRU, and the `mcp_memo` table in the rtok store (written
  through on every store, read on an in-process miss), so the memo survives restarts.
- **Invalidation:**
  - any successful call of a tool outside the memo (except `ping`, `expand`) clears both
    layers;
  - a row older than `cache_ttl_secs` is not served;
  - on startup, rows from another rtok version, or for a tool whose definition (name,
    description, schema) changed, are dropped.
- **Errors** are never cached.
- **Metrics:** entries, bytes and invalidated rows are logged at startup (`rtok log`, source
  `mcp`, name `memo`), and hits, misses, entries and bytes every 100 lookups.

## Enable / disable

On by default. Under `[mcp]` in the config:

```toml
cache_enabled     = true   # false: off
cache_ttl_secs    = 300    # 0: off
cache_max_entries = 256    # 0: off
```

`RTOK_MCP_CACHE=0` (or `false`, `off`) turns the memo fully off for one process, e.g. to
compare runs with and without it.

## Test

```bash
cargo nextest run -p rtok-store -E 'test(/mcp_memo/)'   # persistence, invalidation, eviction, collisions
cargo nextest run -p rtok -E 'test(/mcp::memo|tools_list_is_byte_stable|memo_tools|errors_are_not/)'
```

## Limitations

- A write made by another process (for example `rtok mem save` from the CLI) does not
  clear this server's memo; `cache_ttl_secs` bounds how stale an answer can be.
- `docs_*` reads a local index; a `rtok docs fetch` in another process is subject to the
  same TTL bound.
- The vector extension (`sqlite-vec`) is not loaded: Diesel's bundled SQLite cannot load it
  (see `crates/rtok-store/src/embed.rs`). The memo does not need it.
- Token savings are indirect: a hit returns the same text, so the model reads the same
  tokens. The gain is repeated work and latency, plus a stable `tools/list` prefix for
  host-side prompt caching ([prompt-cache.md](prompt-cache.md)).
