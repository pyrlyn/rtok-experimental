# `toon`

Tabular JSON → TOON (Token-Oriented Object Notation). On by default (T127); a block is
rewritten only when the encoding is smaller than the JSON it replaces.

| | |
|---|---|
| Surfaces | proxy `compress` mode (`proxy_filter`); MCP `expand` returns the original |
| Spec | the **Replaces** column of [`docs/plugins.md`](../../../docs/plugins.md) |
| Default | on |

## Mechanism

Arrays of at least `min_rows` uniform objects (same keys, scalar values) in a tool result are
re-encoded as a header row plus one line per record. Vendor benchmark: −42.6 % tokens on
tabular data. Non-tabular JSON, a table that would not shrink, and the last
`plugins.archive.keep_turns` turns are left untouched. The original is archived; the encoded
block carries an `expand` id.

## Config

```toml
[plugins.toon]
enabled = true
min_rows = 5
```

## Tasks

See `roadmap.md` § `toon`. Checks in `plan/`.

T11.7 `proxy_filter` TOON on tabular JSON (default on). A table that would not shrink is left as is.

## Status

Implemented: `proxy_filter` (T11.7), on by default (T127).
