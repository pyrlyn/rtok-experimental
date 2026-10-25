# `json_tree`

Fold a nested JSON tree into hoisted values and repeated element bodies. Off by default;
a block is rewritten only when the fold estimates fewer tokens than the JSON it replaces.

| | |
|---|---|
| Surfaces | proxy `compress` mode (`proxy_filter`); `rtok mcp -- <server>` wrapper (foreign server's tool results) |
| Spec | the **Replaces** column of [`docs/plugins.md`](../../../docs/plugins.md) |
| Default | **off** |

## Mechanism

Object and array values used by two or more objects are printed once under `VARS:`. An object
body that appears twice, ignoring `id` and `name`, becomes an `EL-` template; occurrences
print `template=EL-…` plus their children. A uniform scalar table is left for `toon`. The
original is archived; `rtok expand <id>` returns those bytes. In the MCP wrapper a fold
replaces the line cut only when it fits the `[mcp]` rule's line cap and is shorter; results
of tools named `read` or `search` are never folded.

## Config

```toml
[plugins.json_tree]
enabled = false
```

## Tasks

See `roadmap.md` § `json_tree`. Checks in `plan/`.

## Status

Off until a `Measurement` row (`plugin: "json_tree"`, `kind: "fold"`) shows a saving.
