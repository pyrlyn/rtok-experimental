# Model routing (D9)

**Status: opt-in for `internal` and `bulk` calls only; agent-turn routing is planned / Later.**
Off by default, no rule ships, and the `agent` lane is never routed. **Separate line of work from
Batch and Flex** (`docs/batch-flex.md`): those change *how* or *when* a
request is billed on the same (or async) surface; model routing changes
*which* `model` string the sync chat wires forward.

Source of intent: `plan.md` decision **D9**, `research.md` §16.3 option
**#3** (and sticky-upstream #1 / I-84), `ideas.md` I-101 token-saving backlog.
Behaviour that is not wired in `src/proxy/` is marked **planned**.

## What it is

D9 (*Agents are provider-agnostic*): route by job — low-cost models for
mechanical work, mid-tier for coding, high-performance for research only
after the user confirms. Host product names are not the implementer.

`research.md` §16.3 #3 (*Model / tier routing by job*):

- Cheap model for format / classify / expand-prep
- Mid for edit
- Expensive only after confirm
- Impact is mostly **$** (small raw-token change)
- Needs a **router policy + measurement** so claimed “savings” are dollars,
  not vibes

rtok today is a middleware surface (hooks, MCP, API proxy) — **not** a
model router by default. Agents (or the host) still pick the model; the
proxy leaves `model` as the client sent it.

## How it works in rtok

Only under `rtok proxy` (`src/proxy/`), on the JSON chat wires that carry `model` in the body
(`/v1/messages`, `/v1/chat/completions`, `/v1/responses`). Hooks and MCP never see LLM HTTP
bodies, so they cannot implement D9. Gemini names its model in the path, which routing does not
rewrite, so a Gemini call is never routed.

- **Which calls.** `internal` (rtok's own model calls) and the `bulk` lane
  (`src/proxy/lane.rs`, `Lane::route`). Agent turns, `batch`, `files`, `embeddings` and `meta`
  are never routed. Agent-turn routing needs the I-101 bench first and stays Later.
- **Policy table.** `[proxy.routing]` (`enabled`, off) with one rule per lane
  (`[proxy.routing.bulk]`, `[proxy.routing.internal]`): `from` (exact ids or `prefix*`), `to`,
  `provider` and `gate`. A rule acts only when `from`, `to` and `gate` are all set. Moving the
  lane to another provider's gateway is `[proxy.lanes.<lane>] upstream`.
- **Quality gate (I-101).** rtok cannot see whether the cheaper model is good enough, so `gate`
  must name the operator's bench for that swap; rtok does not read or judge it. It ships no
  default rule and makes no claim about quality.
- **Rewrite.** `shape_request` swaps the body's `model` before the call is recorded, so
  `calls.model_id`, `usage.model` (and the provider, when the rule names one) are the model that
  served the call. The log row of the call keeps `model route on the <lane> lane: <from> -> <to>`
  and a `model_route` measurement row marks it (no token saving is claimed).
- **Dollars.** `rtok stats --price` adds `cost by lane`, each lane priced at the models that
  served it. Compare it for one workload with routing off and on; that difference is the only
  saving rtok reports for routing.

No transparent sync→Batch conversion; routing does not enqueue Batch or set Flex
`service_tier`. Those stay under `[proxy.batch]` / `[proxy.flex]`.

## Configuration

See [config.md](config.md#proxyrouting). Example:

```toml
[proxy.routing]
enabled = true

[proxy.routing.bulk]
from = ["claude-sonnet-*"]
to = "claude-haiku-4-5"
gate = "bench/bulk-haiku-2026-10.md"
```

Sticky upstream (`sticky`, I-84) is not part of this and is not a key yet.

## Status

| Piece | Status |
|-------|--------|
| Decision D9 | recorded in `plan.md` |
| `[proxy.routing]` rules for `internal` and `bulk` | shipped, off by default |
| `model` rewrite before the call is recorded | shipped (JSON-body wires) |
| `cost by lane` in `rtok stats --price` | shipped |
| Quality bench behind a swap (I-101) | **operator's**: a real bench needs real models and workload, which rtok does not ship |
| Gemini (model in the path) | not routed |
| Agent-turn routing | **Later** — blocked on the I-101 bench |
| Sticky upstream (I-84) | Decision-shaped; may land with prompt-cache work independently |

## Related

- [batch-flex.md](batch-flex.md) — Batch / Flex levers (separate from routing)
- [prompt-cache.md](prompt-cache.md) — why sticky upstream matters for $
- [config.md](config.md) — `[proxy.*]` reference (when fields land)
- `plan.md` — D9; Batch/Flex roadmap S6 / T256 stub
- `research.md` §16.3 — options #1 (sticky) and #3 (model / tier routing)
- `ideas.md` I-101 — token-saving backlog card
- `ideas.md` I-84 — sticky upstream + prompt-cache affinity
