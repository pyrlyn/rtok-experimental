# json_tree plugin

![json_tree: the proxy filter or an MCP result wrapper folds a nested JSON tool result into hoisted values and element templates, archiving the original](json_tree.svg)

Source: [`json_tree.d2`](json_tree.d2). Plugin docs: [README](../../../src/plugins/json_tree/README.md). Index: [all diagrams](../README.md).

`json_tree` shrinks large nested JSON (a design file, an AST, a deep API response) without
cutting it. Values used by two or more objects are printed once under `VARS:`, and object bodies
that repeat become `EL-` templates, so the model sees the structure with the repetition removed
instead of an archive head and tail. The plugin is off by default and rewrites a block only when
the fold is estimated cheaper than the JSON it replaces.

## Walkthrough

1. **Two entry points.** In `compress` mode the proxy calls `JsonTreePlugin::proxy_filter` on each
   request. Separately, `mcp::wrap::shorten_result` (`src/mcp/wrap.rs`) calls `fold_json` on an MCP
   tool result that is over the rule's `max_lines`. Both do nothing unless
   `[plugins.json_tree] enabled` is true.
2. **Only old results.** The proxy path goes through `archive::outside_live_zone`, which drops the
   last `[plugins.archive] keep_turns` turns, so the results the model is working on are untouched.
3. **A stored decision wins.** `rewrite_block` reads `archive_decision(tool_use_id)`. A block the
   model already expanded, or one `toon` already handled, is left alone; a `json_tree` pointer
   already stored is replayed as written, so the request prefix stays byte-stable.
4. **Fold.** The text is parsed, then `fold_json` returns `None` for a value under `MIN_BYTES`, a
   value with no nested object, or one `toon::tabular_keys` accepts (a uniform table is toon's).
   Otherwise `tally` counts repeated object and array values into `V-` ids, `collect_bodies` counts
   repeated element bodies (ignoring the identity keys `id` and `name`, and `children`) into `EL-`
   ids, and `render` prints `VARS:`, `ELEMENTS:` and the tree with `template=EL-...`.
5. **Rewrite only if cheaper.** The folded preview must estimate fewer tokens than the original
   (`cx.estimate` with `Class::Code`). If so, `put_archive` stores the original,
   `put_archive_decision` stores the pointer, and the block becomes `[json-tree <id>]` followed by
   the folded text. The MCP path additionally requires the fold to fit `max_lines` and be smaller.
6. **Record.** One `Measurement { plugin: "json_tree", kind: "fold" }` row per rewritten block,
   with the full archive sha in `ref_id`.
7. **Recover.** `rtok expand <id>` returns the archived bytes unchanged.

## Where it lives

| Box | Code |
| --- | --- |
| `proxy_filter` | `src/plugins/json_tree/mod.rs` (`impl Plugin for JsonTreePlugin`) |
| `outside_live_zone` | `src/plugins/archive/mod.rs` (`outside_live_zone`) |
| `rewrite_block`, `measurement` | `src/plugins/json_tree/mod.rs` |
| `fold_json`, `fold_json_with` | `src/plugins/json_tree/mod.rs` (`tally`, `collect_bodies`, `render`) |
| `toon::tabular_keys` | `src/plugins/toon/` |
| MCP wrapper | `src/mcp/wrap.rs` (`shorten_result`, `json_tree_fold`) |
| pointer prefix | `src/expand.rs` (`JSON_TREE_PREFIX`, `[json-tree `) |

## Invariants

- Lossless: the original JSON is archived before the block is rewritten, and the printed pointer
  carries the 8-character archive id.
- No saving without a `Measurement` row, and no rewrite unless the fold estimates fewer tokens.
- `default_on` is false; `archive` and `toon` leave a `[json-tree ` pointer alone.
- The MCP path never folds the `read` or `search` tools.
- A parse failure, a store error or an unfoldable value leaves the block as it was.

## Related

[Overview](../overview.md) · [Plugin contract](../plugin-contract.md) · [Proxy flow](../proxy-flow.md) · [MCP flow](../mcp-flow.md) · [toon plugin](toon.md) · [archive plugin](archive.md)
