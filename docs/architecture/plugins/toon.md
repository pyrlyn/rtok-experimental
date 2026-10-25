# toon plugin

![The proxy hands a tool result to toon, which skips live-zone, already-decided and non-tabular blocks, and otherwise archives the JSON array and replaces it with a TOON table and an expand pointer](toon.svg)

Source: [`toon.d2`](toon.d2). Plugin docs: [README](../../../src/plugins/toon/README.md). Index: [all diagrams](../README.md).

`toon` re-encodes a JSON array of uniform objects, found in an old tool result, as a header
line plus one comma-separated line per row. Repeating every key in every row is what costs
tokens in the original. The plugin runs only on the proxy surface, through
`Plugin::proxy_filter`, and only rewrites when its own token estimate says the result is
smaller.

## Walkthrough

1. **The proxy offers tool results.** `rtok proxy` builds a `WireRequest`; `ToonPlugin::proxy_filter`
   returns at once when `[plugins.toon] enabled` is false (the manifest has `default_on: true`).
2. **Live zone is skipped.** `rewrite` takes `min_rows` from config and passes the results
   through `archive::outside_live_zone`, so the last `[plugins.archive] keep_turns` turns stay
   byte-identical.
3. **A stored decision wins.** `rewrite_block` reads the tool-result text with
   `archive::block_text` (or the JSON of a bare array) and looks up
   `cx.archive_decision(tool_use_id)`. An expanded id, or a pointer written by `archive` or
   `json_tree` (`skips_foreign_pointer`), is left alone. A pointer that starts with
   `[toon ` is replayed as written and measured as a `toon`/`encode` row.
4. **Only uniform tables qualify.** `tabular_keys` accepts an array of at least `min_rows`
   objects (config key `min_rows`) that have at least three keys, identical key sets and only
   scalar values, and whose keys need no quoting.
5. **Encode and gate on size.** `encode` writes `[N]{k1,k2,…}:` and one indented line per row;
   `needs_quotes` quotes any cell that would otherwise read back as a different value. The block
   is encoded only when the estimate of pointer plus TOON is below the estimate of the JSON, so
   a table that would grow is left untouched and leaves no archive decision.
6. **Archive first, then replace.** `cx.put_archive` stores the original text,
   `cx.put_archive_decision` pins the replacement for later turns, and the block becomes
   `[toon <8-char id>]` followed by the table. `Measurement { plugin: "toon", kind: "encode" }` is returned
   and recorded by the host.
7. **Recovery.** `rtok expand <id>` returns the original JSON and records a `toon`/`expand`
   row; after that the id is never re-encoded.

## Where it lives

| Box | Code |
| --- | --- |
| ToonPlugin::proxy_filter, enabled? | `src/plugins/toon/mod.rs` (`ToonPlugin::proxy_filter`) |
| outside_live_zone | `src/plugins/archive/` (`outside_live_zone`, `block_text`) |
| archive_decision | `src/plugins/toon/mod.rs` (`rewrite_block`, `skips_foreign_pointer`, `PREFIX`) |
| tabular_keys | `src/plugins/toon/mod.rs` (`tabular_keys`) |
| encode | `src/plugins/toon/mod.rs` (`encode`, `encode_cell`, `needs_quotes`) |
| archive blobs and decisions | `crates/rtok-store/` (`archive_decisions`, `measurements`), `archive_dir` |
| expand | `src/expand.rs` (`fetch`) |
| Config | `crates/rtok-config/` (`[plugins.toon]` `enabled`, `min_rows`) |

## Invariants

- Lossless: the original text is archived before the block is replaced, and `expand <id>`
  returns it. `decode` exists in the test build as the round-trip oracle for `encode`.
- A table that would not shrink is left as is, and no archive decision is written for it.
- Recent turns (`keep_turns`) are never touched, and a stored decision is replayed as written,
  so earlier turns stay byte-stable.
- `toon` never replays another plugin's pointer (`archive`, `json_tree`), so one block yields one
  measurement row.
- No `Measurement` row, no saving claim: every rewrite returns a `toon`/`encode` row.

## Related

[Overview](../overview.md) · [Plugin contract](../plugin-contract.md) · [archive](archive.md) ·
[json_tree](json_tree.md) · [proxy](proxy.md)
