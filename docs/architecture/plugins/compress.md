# compress plugin

![After archive has replaced an old tool result with a pointer, compress swaps the pointer for a deterministic extractive summary, or for a stored model summary that a background queue produced; expand returns the original](compress.svg)

Source: [`compress.d2`](compress.d2). Plugin notes: [PLAN](../../../src/plugins/compress/PLAN.md) (this plugin has no README). Index: [all diagrams](../README.md).

`compress` goes one step further than `archive` for blocks that `archive` already replaced
with a pointer. In `proxy.mode = "compress"` it replaces the pointer with a short structured
view (type, title, narrative, facts, files) built without a model. Optionally a background
queue asks a model for a better summary once, stores it, and later requests show that
instead. The original stays in the archive, and the view always ends with `expand(<id8>)`.

## Walkthrough

1. **Run after archive.** The proxy runs the `proxy_filter` of every enabled plugin on the
   parsed request. `archive` runs first and leaves `[archived <id8>: ...]` pointers in old
   tool results; `CompressPlugin::proxy_filter` then returns early unless
   `[plugins.compress] enabled` is true and `proxy.mode` is `compress`.
2. **Stay outside the live zone.** `compress::rewrite` reuses `archive::outside_live_zone`, so
   the last `[plugins.archive] keep_turns` turns are never touched.
3. **Check the block.** `summarize_block` reads `cx.archive_decision(tool_use_id)`. It
   continues only when the decision exists, is not expanded, its pointer starts with
   `[archived ` and the block's text still equals that stored pointer. Anything else is left
   alone.
4. **Choose a view.** With `[plugins.compress.llm] enabled` and a summary stored for this
   `(archive_id, version)` in `archive_summaries`, `stored_view` builds
   `[compress <id8>]`, `type: tool_output`, `summary: ...`, `expand(<id8>)`. Otherwise
   `extractive_summary` reads the original through `cx.get_archive` and builds a title, a
   narrative of the next three distinct lines, up to three facts (error lines first) and
   up to three file lines; fact and file lines are clipped by `archive::clip`.
5. **Offer the id.** When the LLM flag is on and no summary is stored yet, the archive id is
   pushed onto the thread-local `OFFERED` list and the block keeps the extractive view.
6. **Keep only a saving.** A view that is not shorter than the pointer is discarded and the
   pointer stays. Otherwise the block text is replaced and a `Measurement`
   (`plugin: "compress"`, kind `summary`) is returned.
7. **Hand the ids to the queue.** After the filters, the proxy drains `take_offered()` and,
   if `state.summaries` exists, calls `Queue::submit` with `Auth::from_headers`, which
   returns credentials only for the Anthropic wire. The call returns at once.
8. **Summarise in the background.** `Queue::admit` takes an in-flight slot (`max_in_flight`)
   and tries each id once per process. `Compressor::try_run` skips a cached id, a non-text
   payload, one over `max_input_bytes`, diff or source text unless `code = true`, and one
   with more than `max_spans` must-keep spans.
9. **Ask and guard.** `Compressor::ask` posts one `/v1/messages` request to the proxy's own
   address (or `[plugins.compress.llm] url`) with the lane header set to `internal`, so the
   spend is tagged and `[proxy.routing]` applies. `must_keep::dropped` checks that the
   summary still carries every span (paths, identifiers, error codes, numbers, commands,
   ids); a summary that is empty, not shorter than the original or that lost a span is not
   stored.
10. **Store.** `put_archive_summary` writes the summary beside the archive row. It never
    changes afterwards, so the view is byte-stable from the turn it first appears.
11. **Expand.** `expand(<id8>)` returns the original bytes, and an expanded decision stops
    `compress` from rewriting that block.

## Where it lives

| Box | Code |
| --- | --- |
| `CompressPlugin::proxy_filter` | `src/plugins/compress/mod.rs` (`CompressPlugin`, `proxy_filter`) |
| `archive::proxy_filter`, `outside_live_zone` | `src/plugins/archive/mod.rs` |
| `summarize_block`, `stored_view` | `src/plugins/compress/mod.rs` |
| `extractive_summary` | `src/plugins/compress/mod.rs` (uses `archive::clip`) |
| `OFFERED`, `take_offered` | `src/plugins/compress/mod.rs`; drained in `src/proxy/mod.rs` |
| Queue | `src/plugins/compress/queue.rs` (`Queue`, `submit`, `admit`) |
| Compressor client | `src/plugins/compress/llm.rs` (`Compressor`, `Auth`, `version`, `is_code`); prompt in `prompt.toml` |
| Must-keep spans | `src/plugins/compress/must_keep.rs` (`spans`, `dropped`) |
| Tables | `crates/rtok-store/src/schema.rs` (`archive_decisions`, `archive_summaries`, `measurements`) |
| Config | `crates/rtok-config/src/lib.rs` (`Compress`, `CompressLlm`) |

## Invariants

- Lossless: the archive row written by `archive` is untouched, and `expand <id>` always
  returns the original bytes.
- Deterministic: the extractive view is a pure function of the archived text, and a stored
  model summary never changes, so a replayed request is byte-identical.
- `[plugins.compress.llm]` is off by default; with it off no client and no queue exist.
- A model summary is stored only when it keeps every must-keep span of the original.
- The model call goes through the proxy's `internal` lane, and only the Anthropic wire's
  credentials are forwarded to it.
- Any store or archive read error leaves the block alone; each saving is a `Measurement`
  row, and no row means no claim.

## Related

[Overview](../overview.md) · [Plugin contract](../plugin-contract.md) ·
[archive](archive.md) · [proxy](proxy.md) · [toon](toon.md)
