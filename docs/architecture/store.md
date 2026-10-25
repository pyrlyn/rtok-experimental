# Store

![The SQLite store: surfaces, plugins and coordination code write through rtok-store into one WAL database grouped by domain, raw payloads spill to archive_dir, and expand, stats, the operator model and OTLP export read it](store.svg)

Source: [`store.d2`](store.d2). Index: [all diagrams](README.md).

rtok keeps one SQLite file (D8). Hooks are short-lived processes, so each invocation opens the
file, writes its rows and exits; WAL mode lets the many concurrent writers (every hook, the
MCP server, the proxy) share it. The file is the product's evidence: a saving is a
`measurements` row or it does not exist.

## Walkthrough

1. **One owner.** `rtok-store` is the only crate that names Diesel. `Store::open_with` opens
   the file with a bounded lock wait, switches it to WAL, applies the embedded migrations
   (`crates/rtok-store/migrations/NNNN_<slug>/`, recorded in `schema_migrations`) and keeps
   the file owner-only. The crate prints nothing; housekeeping problems come back as
   `StoreWarning`.
2. **Identity rows** (`hosts`, `providers`, `models`, `sessions`) are upserted and referenced
   by every call. `hook_sessions` stores a hook's session fields once per distinct set, so
   `call_io` need not repeat them.
3. **The ledger.** Each surface writes one `calls` row per event, MCP call or API request, and
   its request/response bytes in `call_io` (inline up to `core.call_io_inline_bytes`, else
   spilled to `archive_dir`). `tokens` holds before/after/provider counts, `measurements` the
   plugins' savings (a hook row carries `once_key` so a duplicate delivery counts once),
   `usage` the provider's real usage from the proxy, `logs` the log lines.
4. **Archive.** Anything shortened is stored whole: the `archive` row holds size, sha256 and
   path, the bytes live under `archive_dir/<id>`. `rtok expand <id>` (and the `expand` MCP
   tool) read them back. `read_cache` lets `read` skip a file the session already saw.
5. **Memory.** `notes` with its FTS5 index `notes_fts`, `note_files`, versions, optional
   embeddings and the `observations` the memory plugin extracts.
6. **Graph and docs.** The `graph` plugin's symbol index (`symbols`, `symbol_stale`,
   `file_rank`, `extractor`), its event log and the project registry; the `docs` plugin's
   `doc_crates` / `doc_items`.
7. **Coordination.** Agent rows, messages between agents, worktree claims and task claims and
   counters, which the `worktree_*`, `agent_*` and `task_*` tools read and write.
8. **Readers.** `rtok stats`, `report` and `doctor` aggregate the ledger; `src/model` builds the
   D23 operator model that `rtok tui` and `rtok web` render; the OTLP exporter streams the
   ledger past its `otel_export` watermark. Retention and vacuum run from the long-lived
   surfaces (`spawn_retention`).

## Where it lives

| Box | Code |
| --- | --- |
| Open, WAL, permissions | `crates/rtok-store/src/handle.rs` |
| Schema and migrations | `crates/rtok-store/src/schema.rs`, `crates/rtok-store/migrations/` |
| Calls, tokens, measurements | `crates/rtok-store/src/call_log.rs`, `measure_rows.rs` |
| Archive | `crates/rtok-store/src/archive_rows.rs`, `archive_summaries.rs`, `src/expand.rs` |
| Notes and observations | `crates/rtok-store/src/note_store.rs`, `note_files.rs`, `observations.rs` |
| Symbols | `crates/rtok-store/src/symbols.rs`, `symbol_graph.rs` |
| Coordination | `crates/rtok-store/src/agents.rs`, `messages.rs`, `worktree_claims.rs`, `task_claims.rs` |
| Retention | `crates/rtok-store/src/retention.rs` |

## Invariants

- Database access goes through Diesel; raw SQL appears only where Diesel cannot express it
  (migrations, FTS5), inside the store crate.
- An applied migration is never edited; a change is the next migration directory.
- A locked store never fails a hook: the hook passes through and defers what it must.

## Related

[Overview](overview.md) · [Plugin contract](plugin-contract.md) · [Hook flow](hook-flow.md) ·
[Proxy flow](proxy-flow.md) · [archive](plugins/archive.md) · [memory](plugins/memory.md) ·
[graph](plugins/graph.md) · [measure](plugins/measure.md)
