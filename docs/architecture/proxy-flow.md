# Proxy flow

![rtok proxy: a request is classified by lane, checked for loops and capacity, shaped by plugins unless plain, forwarded, and its streamed response tee'd into the ledger](proxy-flow.svg)

Source: [`proxy-flow.d2`](proxy-flow.d2). Index: [all diagrams](README.md).

`rtok proxy` sits on `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL`. It is the only surface that sees
the whole LLM request, so it is where tool results can be shortened for every host and where
the provider's real `usage` is recorded. The response is never altered: it streams to the
client as it arrives and a copy is read afterwards for bookkeeping.

## Walkthrough

1. **Classify.** `proxy::handle` (the axum fallback route) runs `lane::classify`: `agent` for an
   interactive turn; `bulk`, `batch`, `files`, `embeddings`, `meta` and `internal` for scripts,
   Batch jobs, uploads, embedding calls, model listing and token counting, and rtok's own calls.
   The lane picks the policy (`[proxy.lanes.<lane>]`); the agent lane keeps the global switches, `batch` and `files` are never rewritten.
2. **Refuse loops.** Every forwarded request carries `x-rtok-proxied: <hops>`. One that
   already passed an rtok proxy is forwarded as is; at `MAX_HOPS` the proxy answers 508.
3. **Cap bulk lanes.** `gate` gives each capped lane its own in-flight slots and a short line,
   so a bulk burst cannot take the provider's connections or rate limit from the agent; a
   full line is refused.
4. **Plain or shaped.** With `proxy.enabled` or `core.enabled` off (or an already-proxied
   request) the proxy is a plain reverse proxy: bytes forward, nothing is recorded, and the
   live view (memory only, `GET /live`) shows the traffic.
5. **Shape.** Otherwise `shape_request` runs on the blocking pool:
   `route::apply` may swap the model for the lane, `record` writes the `calls` row
   (`api_request`, or `api_request:<lane>` off the agent lane) and `call_io`; in `compress`
   mode every enabled plugin's `Plugin::proxy_filter` shortens the tool results the matching
   `Wire` exposes and returns `Measurement`s; `noise::strip` removes ANSI and control
   characters; `prepare` applies the wire's shaping (for example `include_usage` on OpenAI
   streams); then the opt-in `context_edits`, `rewrite_tools` and Flex `service_tier`.
6. **Cache.** The opt-in two-tier `semantic_cache` may answer a repeated request without a
   provider call.
7. **Forward and tee.** The request goes to `proxy.upstream` (or `proxy.openai_upstream`), with
   `identity` encoding so the copy stays readable. Chunks stream to the client unchanged and
   are tee'd into a buffer as they pass.
8. **Finish.** A spawned task parses the provider's `usage` with the same `Wire`, and writes
   `usage`, `tokens` (source `provider`) and the response `call_io`. With
   `[proxy.batch] parse_results`, a forwarded Batch results file also yields `usage` rows.

## Where it lives

| Box | Code |
| --- | --- |
| Server and handler | `src/proxy/mod.rs` (`serve`, `handle`, `shape_request`, `compress`, `record`, `finish`) |
| Lanes, gate, routing | `src/proxy/lane.rs`, `src/proxy/gate.rs`, `src/proxy/route.rs` |
| Wire formats | `src/proxy/wire.rs`, `anthropic.rs`, `openai_chat.rs`, `openai_responses.rs`, `gemini.rs` |
| Shaping | `src/proxy/noise.rs`, `src/proxy/flex.rs`, `src/proxy/tools_rewrite.rs` |
| Cache, live view, Batch | `src/proxy/semantic_cache.rs`, `src/proxy/live.rs`, `src/proxy/batch_results.rs` |
| Plugin-side filters | [proxy plugin](plugins/proxy.md), [compress plugin](plugins/compress.md) |

## Invariants

- The response body reaches the client byte for byte; bookkeeping happens after, off the
  request path, and its failures are only logged.
- A chain of rtok proxies shapes and records a request once.
- Batch and Flex are proxy-only concerns; hooks and MCP never see LLM HTTP bodies
  (`docs/batch-flex.md`).

## Related

[Overview](overview.md) · [Hook flow](hook-flow.md) · [MCP flow](mcp-flow.md) ·
[Store](store.md) · [proxy plugin](plugins/proxy.md) · [compress](plugins/compress.md)
