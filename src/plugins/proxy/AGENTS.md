# Agent notes — `proxy`

**Owns** `src/plugins/proxy/**`, `src/proxy/**` (`mod.rs`, `cli.rs`).

**Contract**: the `Plugin` trait, `Ctx` and the host capabilities come from the published
`rtok-plugin-sdk` crate (`crates/rtok-plugin-sdk`), not from `crate::plugin` — import them as
`rtok_plugin_sdk::…`.

**Invariants**
- Passthrough is byte-exact: the T5.1 test compares response bytes against a mock upstream.
- Added latency < 20 ms per request (definition of done #4). Do not buffer SSE streams.
- Never modify `system` or the last 2 turns. `tools[]` stays byte-identical unless `[proxy.tools_rewrite]` is on (T59.5): then descriptions truncate at a sentence boundary and allow/deny drop names; `input_schema` is never touched; a dropped tool's later call is still forwarded.
- Every request inserts one `usage` row with all four counters; session id comes from
  `metadata.user_id` or a header, else a request hash.
- Bind `127.0.0.1` only.

**Dependencies allowed**: `tokio`, `axum`/`hyper`, `reqwest` (streaming). One-line reason each.

**Checks**: `plan.md` T5.1, T5.2.

**Batch / Flex / routing** (see `docs/batch-flex.md`)
- Batch ≠ Flex ≠ model routing. Never auto-convert sync agent turns into Batch jobs.
- Batch paths (`/v1/batches`, `/v1/messages/batches`, …) miss every `Wire::matches` (exact
  sync paths only) and ride the axum fallback: byte-forward today; observe + result usage
  parsing are **planned** under `[proxy.batch]`.
- Flex is OpenAI `service_tier` on sync Chat/Responses wires — inject/override in `prepare`
  (**planned**, `[proxy.flex]`), not a separate HTTP API.
- Model routing (D9, T385.10) swaps `model` for `internal` and `bulk` calls only
  (`src/proxy/route.rs`, `Lane::route`), opt-in per lane rule with a named quality `gate` (I-101);
  the `agent` lane is never routed. Sticky upstream (I-84) is still **planned**. Hooks/MCP never
  see this traffic.
- Extension points: new `Wire` / path matchers in `src/proxy/` for Batch observe; `prepare`
  for Flex; routing rules in `src/proxy/route.rs`, applied first in `shape_request`. Do not teach hooks or MCP about LLM HTTP.

