# Token savings defaults

rtok is a model- and agent-agnostic layer: everything on this page is a deterministic
rewrite of bytes the client already sends or receives. Nothing depends on which model or
agent sits behind the proxy, nothing picks a model, and nothing calls one.

## What is on by default

| Mechanism | Config | Default | What it does |
|-----------|--------|---------|--------------|
| Tool catalog rewrite | `[proxy.tools_rewrite] enabled`, `compact_schema` | `true`, `true` | Cuts each tool `description` to `max_description_tokens` at a sentence boundary and property descriptions to their first sentence; drops `default`/`examples` that restate the obvious. Names, types, `required` and `enum` stay byte for byte. The original catalog is archived, so `expand <id>` returns it. |
| Failure lines under archive pointers | `[plugins.archive] signal_lines` | `8` | When a large tool result is archived, elided lines matching `error`, `panic`, `fail`, `warning`, `exception` or `traceback` are shown under the pointer as `N:line` (the `expand` grep format). |
| Semantic response cache | `[plugins.proxy.semantic_cache] enabled` | `false` | Unchanged: stays off until Gate P31 (see `src/plugins/proxy/PLAN.md`). This release only adds tests that a stored answer is never served to a different request. |

Both rewrites are pure functions of the input, so the same request produces the same bytes
on every turn and the provider prompt cache keeps hitting (see [prompt-cache.md](prompt-cache.md)).

## Enable or disable

```toml
[proxy.tools_rewrite]
enabled = false          # forward tools[] byte for byte
# compact_schema = false # keep description truncation, leave input_schema alone

[plugins.archive]
signal_lines = 0         # pointer = head + tail only, as before
```

Non-agent lanes still need their own `[proxy.lanes.<lane>] tools_rewrite = true`.

## Test and bench

```sh
bench/token_savings.sh                              # offline, fixed fixtures, no API calls
cargo nextest run --lib -E 'test(/semantic_cache/)' # cache-key negative tests
```

The bench prints one `SAVINGS` line per mechanism and fails if a floor is missed:

| Mechanism | Fixture | Before | After |
|-----------|---------|--------|-------|
| `tools_rewrite` | `tests/fixtures/mcp_catalog_github.json` | 4863 est. tokens, 14587 bytes | 3302 est. tokens (−32 %), 9905 bytes; floor ≤ 70 % |
| `signal_lines` | 400-line build log with two failures in the elided middle | pointer 387 bytes, 1 `expand` call to see the failures | pointer 514 bytes, 0 `expand` calls |

Token counts are rtok's estimator, not a provider tokenizer.

## Limitations

- Truncated descriptions can drop usage hints a model would have read; the full text is one
  `expand` away, but nothing tells the model to look. Turn the rewrite off if a tool is
  misused after it.
- `signal_lines` matches keywords, not meaning: a line naming a `fail_fast` flag counts, and a
  failure worded otherwise does not. Each inlined line adds up to 200 chars to the pointer.
- A pointer is stored once per tool result, so changing `signal_lines` affects only results
  archived after the change.
- No live A/B (`rtok bench` with `RTOK_BENCH_LIVE=1`) was run for these defaults; the numbers
  above are offline fixture measurements.
