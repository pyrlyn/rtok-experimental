# `archive`

Shrinks old, large tool results in the live zone — without breaking the
prompt cache.

| | |
|---|---|
| Surfaces | proxy `compress` mode; MCP `expand`; `rtok archive rewrite --stdin` (pi `context`) |
| Spec | the **Replaces** column of [`docs/plugins.md`](../../../docs/plugins.md) |
| Default | on (active only when the proxy runs in `compress` mode, or when a carrier invokes it) |

## Mechanism

For each `tool_result` block that is older than `keep_turns` turns from the end **and** larger
than `min_tokens`, the content becomes:

```
[archived <id>: first 8 lines … last 4 lines · N tokens · expand(<id>)]
```

`<id>` is the first 8 hex characters of the archive id. `expand` resolves that prefix.
The measurement stores the full id. A decision already on disk is replayed as written, so a
cached prefix does not change.

Decisions are keyed by `tool_use_id` and persisted, so every later request rewrites the same
block identically — the frozen prefix stays byte-stable and cache hits survive. Never touched:
`system`, `tools`, the last `keep_turns` turns, or any block whose id was `expand`ed.

The live zone has two carriers, and they share one function — `rewrite` over
`ToolResultRef`/`BlobRef`, never a second implementation:

1. **The proxy** (`compress` mode) applies it to every forwarded request through the
   provider wires.
2. **The pi `context` event** applies the same rewrite without a proxy: pi fires `context`
   before every LLM call with the full message array, and the extension
   (`plugins/pi/extensions/rtok.ts`) shells out to `rtok archive rewrite --stdin`
   (T70.2). Input bytes come back verbatim when nothing is eligible — the extension's
   "no change" test is a string compare, and replaying the same array is byte-identical
   because every pointer decision is persisted. pi sessions carry results as
   `role: "toolResult"` messages keyed by `toolCallId`; the pi view of the wire lives in
   `src/plugins/archive/pi.rs`.

## Config

```toml
[plugins.archive]
enabled = true
keep_turns = 4
min_tokens = 1500
```

## Tasks

See `roadmap.md` § `archive`. Checks in `plan/`.

T5.3 live-zone rewrite · T5.4 `expand` through the proxy · T11.4 across wires ·
T70.2 pi `context` carrier.

## Status

Implemented: live-zone rewrite (T5.3), `expand` through the proxy (T5.4), across wires
(T11.4), pi `context` carrier (T70.2).
