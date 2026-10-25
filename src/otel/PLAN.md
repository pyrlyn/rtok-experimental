# otel — design (D19, P16)

**Problem, in numbers.** rtok records every hook, MCP call and proxied request — `calls`, `call_io`, `usage`, `tokens`, `measurements`, `logs` — and shows it only through `rtok stats`. The user wants the same rows as traces, logs and metrics in Jaeger, Grafana, SigNoz and Maple, with full content, and without paying for it where rtok is paid for: a hook must still exit in ≤ 10 ms (Gate P2), so no network call may run in it.

## Alternatives (surveyed 2026-09-04)

| Option | Version | Gets right | Gets wrong |
|---|---|---|---|
| `opentelemetry` + `opentelemetry_sdk` + `opentelemetry-otlp` (`http-json`) | 0.32 / 0.32.1 / 0.32 | Spec-tracked encoders, batch processor, protobuf option; `SpanBuilder::with_start_time` allows back-dating | Three crates plus `prost`/`http` deps outside §2; an in-process tracer records spans as they happen, so a 10 ms hook process must flush over the network before it exits — or back-date everything from the ledger, at which point the SDK is a JSON serializer with a runtime |
| `tracing` + `tracing-opentelemetry` | 0.1 / 0.32 | `#[instrument]` ergonomics, one subscriber for logs and spans | Code spans, not ledger rows: token counts and savings would need a second bookkeeping path beside D8; same on-exit flush problem |
| OTel Collector `otlpjsonfilereceiver` | contrib 0.13x | Zero network in rtok; any backend via the Collector | A second process the user must run and keep alive; the file is a second copy of the ledger; still needs the encoder |
| **Hand-rolled OTLP/HTTP JSON projected from the ledgers** (chosen) | OTLP 1.11, semconv GenAI (Development) | No new crate — `serde_json` + `reqwest` are in §2; the DB is the queue: per-stream watermark, at-least-once, ids derived from row ids so a resend is byte-identical; export runs on timers and in a detached child, never in a hook | rtok owns ~150 lines of encoder that must track the OTLP JSON mapping rules (stable since 1.0; pinned by unit tests) and the GenAI attribute names (Development status; renames are a one-file change) |
| OpenLLMetry / Langfuse SDKs (outside the stack) | traceloop-sdk 0.4x, langfuse 3.x | Define the practical GenAI attribute set the backends render (`gen_ai.usage.*`, `gen_ai.input.messages`) | Python-side instrumentation of an application; nothing for a CLI hook |

## Mechanism

1. **Streams and watermarks.** `otel_export(stream, mark)`: `calls` by `id`, `logs` by `id`, `sessions` by `ended_at` (`>=`, so a tie resends rather than drops). A flush reads ≤ 1 000 rows past each mark, posts, and advances the mark only on 2xx.
2. **Ids.** `trace_id = sha256("rtok:session:" + session_id)[..16]`, `span_id = sha256("rtok:" + kind + ":" + id)[..8]`. Deterministic, so the same row always becomes the same span and duplicates merge in every backend.
3. **Mapping** (GenAI semconv; span kind INTERNAL unless noted):

| Ledger row | Span | Attributes |
|---|---|---|
| `sessions` (root, sent when `ended_at` is set) | `invoke_agent {host}` | `gen_ai.operation.name=invoke_agent`, `gen_ai.agent.name={host}`, `gen_ai.conversation.id`, `rtok.project`, `rtok.cwd`, `rtok.source` |
| `calls` hook `PreToolUse` / `PostToolUse` | `execute_tool {tool_name}` | `gen_ai.operation.name=execute_tool`, `gen_ai.tool.name`, `gen_ai.tool.call.id` (= `tool_use_id`), `gen_ai.tool.call.arguments` (= `tool_input`), `gen_ai.tool.call.result` (= `tool_response`, Post only), `rtok.hook.event` — all read from `call_io.request_json` |
| `calls` hook, other events | `hook {event}` | `rtok.hook.event`; `UserPromptSubmit` adds `gen_ai.input.messages` `[{role:user, parts:[{type:text, content}]}]` |
| `calls` mcp `tool_call` | `execute_tool {name}` | `gen_ai.operation.name=execute_tool`, `gen_ai.tool.name`, `rtok.plugin`, `gen_ai.tool.call.arguments` / `result` from `call_io` |
| `calls` proxy `api_request` (CLIENT) | `chat {model}` | `gen_ai.operation.name=chat`, `gen_ai.provider.name`, `gen_ai.request.model`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_creation.input_tokens` (from `usage`), `gen_ai.input.messages` / `gen_ai.output.messages` (wire bodies, raw), `rtok.api` |
| every `calls` row | — | parent = session span, `gen_ai.conversation.id`, `rtok.surface`, `rtok.kind`, `rtok.call.id`, status `Error` + `error.type` when `ok = 0`; start `ts × 10⁹`, end = start + `ms × 10⁶` |
| `tokens` (per call) | event `rtok.plugin.run` | `rtok.plugin`, `rtok.phase`, `rtok.source`, `rtok.tokens`, `rtok.bytes` |
| `measurements` (per call) | event `rtok.measurement` | `rtok.plugin`, `rtok.kind`, `rtok.before_bytes`, `rtok.after_bytes`, `rtok.est_before`, `rtok.est_after`, `rtok.ref_id` |
| `logs` | log record | `severityNumber` from `level`, body = `message`, `rtok.source`, `rtok.name`, `rtok.plugin`, `rtok.fields`; `traceId` / `spanId` when `session` / `call_id` are set |
| `usage`, `measurements`, `calls` (whole table) | sums | `rtok.tokens{gen_ai.token.type, gen_ai.request.model, gen_ai.provider.name}`, `rtok.tokens.saved{rtok.plugin, rtok.kind}`, `rtok.calls{rtok.surface, rtok.kind, rtok.ok}` — cumulative, monotonic |

4. **Content (D4).** `content = false` by default, even when an endpoint resolves: bodies, arguments and results are omitted unless the user opts in. With `content = true`, values go on the span up to `content_bytes` (64 KiB). Past that the attribute is cut and `rtok.archive.id` names the archive row, so `rtok expand <id>` still holds every byte. Disabling content keeps ids and counts.
5. **Transport.** `POST {endpoint}/v1/traces | /v1/logs | /v1/metrics`, `Content-Type: application/json`, headers from `[otel] headers` or `OTEL_EXPORTER_OTLP_HEADERS` (`k=v,k2=v2` — SigNoz `signoz-ingestion-key`, Grafana Cloud `Authorization=Basic …`, Maple its key). Timeout `flush_secs`. Any failure is one `logs` row (`source = otel`) and the mark stays.
6. **Triggers.** `proxy`: tokio interval. `mcp`: a thread. Hooks: `Stop` / `SessionEnd` spawn `rtok otel flush` detached (~1 ms) and return. `rtok otel flush` for everything else.

## Rejected

- **gRPC / protobuf OTLP** — needs `tonic` + `prost`; no backend in the list requires it.
- **In-process tracer in hooks** — the network call lands in the 10 ms budget; a `flush_secs` batch that outlives the process needs a daemon.
- **A daemon (`rtok otel serve`)** — v0.1 has no daemon (D-deferred, `plan.md` v0.2+); the ledger is the queue.
- **`gen_ai.client.token.usage` histogram** — the semconv instrument is a histogram per request; a cumulative sum answers every dashboard question here without misusing the name. `rtok.tokens` it is.
- **Normalising wire `messages` into the semconv role/parts shape** — three wires, three shapes; ship the raw body now (it is the fullest record), normalise in v0.2 once a backend renders the parts shape differently from raw JSON.
- **Sub-second span starts** — `calls.ts` is whole seconds; spans in one second share a start and are ordered by `rtok.call.id`. Millisecond `ts` is a migration for a later phase if a trace ever reads wrong.

Target: a finished session is one trace in Jaeger within `flush_secs + 2` s of `Stop`, every `calls` row a span and every `usage` row four token attributes, with `rtok hook` p95 ≤ 10 ms unchanged while export is on.
Falsified by: any of Jaeger, Grafana, SigNoz or Maple rejecting the hand-rolled body, or a hook p95 above 10 ms with an endpoint set — either means the projection must move into a resident process, which is the daemon this design refuses.
