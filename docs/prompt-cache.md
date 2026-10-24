# Prompt cache

## Does rtok break the prompt cache?

No — measured, not promised. `rtok stats` on this machine (2026-09-18) reports
`hit=97.5%` — 5 620 803 600 cache-read tokens against 145 402 240 cache-creation and
667 038 uncached input tokens over 910 sessions — and the `rtok report` Cache section on
the same date has no bust rows in the whole ledger: zero recorded prompt-cache busts. The
reason is structural, per surface below: byte-stable injection, rewrites kept outside the
conversation's working edge, and hooks that touch each result once. Reproduce both numbers:

```bash
rtok stats 2>/dev/null | head -4
rtok report 2>/dev/null | sed -n '/[Cc]ache/,+6p'
```

### Hooks — filtered once, then the host owns the bytes

`cmd` and `read` run on PreToolUse, before the call: the output is shortened once and the
host stores that text in its transcript. Every later turn replays the host's own stored
bytes — rtok is no longer in the loop, so nothing it touched changes between turns. Same
shape as rtk's cache argument, plus the archive: whatever was cut is retrievable, and the
bytes that stay are fixed for the life of the session.

### inject — byte-stable per turn

The SessionStart injection is recomputed per turn, so it stays inside the cached prefix
only if the bytes never move. They don't: two runs with unchanged state produce identical
output, pinned by `three_500_budget_800_drops_one_and_is_byte_stable`
(`src/plugins/inject/mod.rs`), with `session_start_has_nudges_once_and_stable` covering
the nudge path. Anything time- or count-dependent in inject is a bug by the plugin's own
invariant, not a trade-off.

### archive / proxy — rewrite only outside `keep_turns`, byte-identical pointers

The proxy-side `archive` filter rewrites a tool result into an archive pointer only when
the turn is older than `archive.keep_turns`; `system`, `tools` and the newest turns are
never touched, so the working edge — the part the provider is still writing into the
cache — is untouched. Two tests in `src/plugins/archive/mod.rs` pin the stability:
`six_turn_fixtures_archive_only_turns_1_and_2_on_every_wire` (of six turns, only 1–2 are
rewritten, on every wire format) and `live_blobs_shrink_stably_outside_the_working_edge`
(a rewritten block's pointer is byte-identical when it is rewritten again).

Measured on this machine, 2026-09-18 — `mise exec -- cargo run --quiet -- stats 2>/dev/null | head -4`:

```text
sessions 910  lines 433211  malformed 0
usage input=667038 cache_create=145402240 cache_read=5620803600 output=27472613  hit=97.5%  median_context=63829
api                         input cache_create cache_read output    hit
codex                    12370533            0  243051392 671350  95.2%
```

and `mise exec -- cargo run --quiet -- report 2>/dev/null | sed -n '/[Cc]ache/,+6p'`:

```text
## Cache

Prompt-cache busts by cause, from the proxy's `usage` rows (whole ledger).

No rows in window.
```

That is the whole ledger captured by the proxy: with archive rewriting older tool results
on the wire, no bust was ever recorded and the hit rate is 97.5% (95.2% on the `codex`
api, which reports 0 cache-creation tokens).

### Per lane — the hit rate and the replay test

Once the proxy has served traffic off the agent lane (bulk, internal, …), `rtok stats` adds a
`lane` table beside `api` with the same counters and the prompt-cache hit rate of each lane
(`lanes` in `--json`). A store of agent turns alone prints no such table. The agent lane's
stability is pinned by `tests/proxy_cache_replay.rs`: it replays one growing conversation
through the proxy with every rewrite on and asserts that `tools`, `system` and every message
before the previous turn's live edge reach upstream byte for byte the same.

### guard — denials add no bytes

A `guard` denial is a PreToolUse decision, not content: it suppresses a duplicate call
with the fixed one-line reason `duplicate; rtok expand <id>` and adds no tool-result
bytes to any turn. The result it denies was paid for once and is retrievable through that
id, so the model never pays for a second copy — and the transcript it replays is
unchanged, which is the cache-friendly outcome.

### Losslessness does not cost the cache

Everything above shortens bytes; nothing loses them. Every shortened payload is archived
before the cut and `expand <id>` restores the original (`--lines`, `--grep`). The pointer
in the transcript is byte-stable, so retrievability never rewrites the prefix: the cache
keeps its hits and you keep the full text.

## Sticky routing vs Batch / Flex

Provider prompt-cache hits need a **stable byte prefix** and, on multi-pod upstreams,
affinity to the same cache-bearing backend (I-84 / sticky routing). That is orthogonal to
**Batch** (async pass-through; no live session KV shared with the interactive agent) and
**Flex** (`service_tier` on the same sync wire — cache rules still apply, discount is the
tier). rtok will not convert a sync agent turn into a Batch job just to save money; do that
only with an explicit Batch client. Details: [batch-flex.md](batch-flex.md).

```toml
# Planned [proxy.routing] sticky flag (not loaded today) — pin upstream for cache affinity.
# Flex is [proxy.flex]; Batch observe is [proxy.batch]. None of these rewrite the cached
# system/tools/live-edge bytes the way a careless compress would.
[proxy.routing]
sticky = true
```

## MCP tool-result memo

`rtok mcp` also memoizes read-only tool results, and keeps `tools/list` byte-stable so a
host's cached prefix survives. See [mcp-cache.md](mcp-cache.md).
