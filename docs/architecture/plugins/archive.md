# archive plugin

![A request reaches archive through the proxy filter or the pi context carrier; old large tool results are archived, replaced by a pointer and replayed byte for byte, and expand returns the original](archive.svg)

Source: [`archive.d2`](archive.d2). Plugin docs: [README](../../../src/plugins/archive/README.md). Index: [all diagrams](../README.md).

`archive` shrinks tool results that have left the live zone: results older than
`keep_turns` turns and larger than `min_tokens`. The original goes to the archive store, and
the block the model sees becomes a pointer line plus the first and last lines of the
original. The decision is stored per `tool_use_id`, so every later request replays the same
bytes and the provider's prompt cache stays valid.

## Walkthrough

1. **A carrier hands over the request.** In `proxy.mode = "compress"` the proxy runs
   `Plugin::proxy_filter` of every enabled plugin; `ArchivePlugin::proxy_filter` returns early
   unless the mode is `compress`. Without a proxy, pi fires its `context` event and the
   extension shells out to `rtok archive rewrite --stdin`, which calls
   `archive::pi::rewrite_stdin`. Both carriers use the same `rewrite` and `rewrite_blobs`
   functions; `pi::tool_results` and `pi::live_blobs` only present pi's message array in
   the wire-normalised shape (`ToolResultRef`, `BlobRef`).
2. **Pick the lanes.** `rewrite(req.tool_results(), cx)` always runs. `rewrite_blobs` runs only
   when `[plugins.archive] live_blobs` is true and `rewrite_skills` only when `skills` is
   true; both are off by default.
3. **Stay outside the live zone.** `outside_live_zone` drops every result whose `turn` is
   below `keep_turns`. `toon` calls the same function, so the boundary is decided once.
4. **Look up an earlier decision.** `rewrite_block` calls `cx.archive_decision(tool_use_id)`.
   An expanded id, or a row whose pointer starts with the `toon` or `json_tree` prefix, is
   left alone (those plugins share `archive_decisions` and already measured their own
   saving). Any other stored pointer is replayed as written and measured again.
5. **Check size.** With no decision yet, `cx.estimate(text, Class::Code)` below `min_tokens`
   leaves the block unchanged and writes nothing.
6. **Archive and write the pointer.** `cx.put_archive` stores the original bytes. `pointer()`
   builds `[archived <id8>: N lines · T tokens · expand(<id8>)]` followed by `head_lines`
   and `tail_lines` of the original, each line cut by `clip`. Up to `signal_lines` elided
   lines that look like failures (`error`, `panic`, `fail`, `warning`, `exception`,
   `traceback`, case-insensitive) follow the `… N lines …` marker as `N:line`, the format of
   `expand` grep, so the usual follow-up `expand` is not needed. With `tiers = true`,
   `tier_live` emits the pointer line alone (L0) and adds a lossless extract (L1) for results
   at exactly turn `keep_turns`.
7. **Persist the decision.** `cx.put_archive_decision` stores the pointer text before the
   request is forwarded. A store error at any step leaves the block unchanged.
8. **Measure.** Each rewritten block yields a `Measurement` (`plugin: "archive"`, kind
   `pointer`, `tier_l0`, `tier_l1`, `live_blob` or `skill`). `record_run` files them under one
   `plugin_run` call with `before` and `after` token estimates.
9. **Expand.** The model calls the MCP `expand` tool with the 8-character id. `expand::fetch_range`
   resolves the prefix, reads the original from the archive, sets `expanded_ts` and records
   an `expand` measurement in one transaction. From the next request on, `rewrite_block`
   sees the expanded decision and leaves the original text alone.

## Where it lives

| Box | Code |
| --- | --- |
| `ArchivePlugin::proxy_filter`, mode check | `src/plugins/archive/mod.rs` (`ArchivePlugin`, `proxy_filter`) |
| pi carrier | `src/plugins/archive/pi.rs` (`rewrite_stdin`, `tool_results`, `live_blobs`); `src/cli/archive.rs` |
| Lanes | `src/plugins/archive/mod.rs` (`rewrite`, `rewrite_blobs`, `rewrite_skills`) |
| Live-zone boundary | `src/plugins/archive/mod.rs` (`outside_live_zone`) |
| Decision, size gate | `src/plugins/archive/mod.rs` (`rewrite_block`, `rewrite_blob`, `rewrite_skill`) |
| Pointer text | `src/plugins/archive/mod.rs` (`pointer`, `pointer_line`, `clip`, `tier_live`, `l1_extract`) |
| Measurement and run row | `src/plugins/archive/mod.rs` (`record_run`) |
| `expand` | `src/expand.rs` (`fetch_range`, `resolve_id`); `src/mcp.rs` (`expand_text`) |
| Tables | `crates/rtok-store/src/schema.rs` (`archive`, `archive_decisions`, `measurements`) |
| Config | `crates/rtok-config/src/lib.rs` (`Archive` section: `keep_turns`, `min_tokens`, `head_lines`, `tail_lines`, `signal_lines`, `tiers`, `live_blobs`, `skills`) |

## Invariants

- Lossless: the original is stored before the block changes, and `expand <id>` returns it.
- Deterministic: the pointer text is stored per `tool_use_id` (blobs: per content hash), so a
  replay is byte-identical and the prompt prefix does not move.
- Never touched: `system`, `tools`, the last `keep_turns` turns, an expanded id, and image
  or other non-text parts (`block_text` returns `None` for them).
- Fail open: any store error leaves the block as it was.
- One `Measurement` per rewritten block; no row, no saving claim.
- `live_blobs`, `skills` and `tiers` are opt-in and default to false.

## Related

[Overview](../overview.md) · [Plugin contract](../plugin-contract.md) ·
[proxy](proxy.md) · [toon](toon.md) · [compress](compress.md) · [cmd](cmd.md)
