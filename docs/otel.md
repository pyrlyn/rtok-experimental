# OpenTelemetry export

rtok already records every hook, MCP call and proxied request in `~/.rtok/rtok.db`. The
exporter projects those rows onto OTLP/HTTP JSON and posts them to any collector — Jaeger,
Grafana, SigNoz, Maple — using the OpenTelemetry GenAI semantic conventions. It is a
projection, not a second recorder: nothing runs on the hook path, and a trace can always be
rebuilt from the database (D19, `plan.md` P16; design in `src/otel/PLAN.md`).

Off until an endpoint resolves.

## Turn it on

```toml
# ~/.rtok/config.toml
[otel]
endpoint      = "http://localhost:4318"   # OTLP/HTTP base URL; "" = $OTEL_EXPORTER_OTLP_ENDPOINT
headers       = ""                        # "k=v,k2=v2"; "" = $OTEL_EXPORTER_OTLP_HEADERS
service_name  = "rtok"
content       = false                     # set true only if you want message bodies, tool arguments and results on spans
content_bytes = 65536                     # per attribute; past it, `rtok.archive.id` → `rtok expand <id>`
flush_secs    = 5                         # proxy / mcp flush interval, and the POST timeout
```

Or with no config file at all:

```bash
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
```

Then:

```bash
rtok otel status
```

```bash
rtok otel flush
```

`status` prints the resolved endpoint, each watermark, how many rows are pending and the last
exporter error. `flush` posts one batch (1 000 rows per stream) and prints what it sent. Both
exit 0 with `otel: no endpoint` when export is off.

## When it flushes

| Surface | Trigger |
|---|---|
| `rtok proxy` | a timer every `flush_secs` |
| `rtok mcp` | a timer every `flush_secs`, and once more at stdin EOF |
| hooks | `Stop` and `SessionEnd` spawn a detached `rtok otel flush` and return |
| anything | `rtok otel flush` |

A hook never opens a socket: it hands the work to a child process, which is why hook p95 stays
under 10 ms with export on. Delivery is at-least-once behind a per-stream watermark that only
advances on a 2xx; span ids are derived from row ids, so a re-sent span is byte-identical and
collectors merge it. Concurrent flushers (proxy timer, mcp timer, hook-spawned child) take an
exclusive file lock beside the DB so they hand off rather than double-post the same batch (T16.9).

**Known debt (T16.9):** `proxy` and `mcp` timer flushes can overlap a detached `rtok otel flush`
from `Stop`/`SessionEnd`. Concurrent processes may race the same watermark and double-export a
batch; pending counts in `rtok otel status` can then disagree with what the collector received.
Until T16.9 lands, prefer a single long-running surface when debugging export, or flush by hand
with `rtok proxy` / `rtok mcp` stopped.

## What you see

| Ledger row | Span | Key attributes |
|---|---|---|
| a session | `invoke_agent {host}` (root) | `gen_ai.agent.name`, `gen_ai.conversation.id`, `rtok.project`, `rtok.cwd` |
| `PreToolUse` / `PostToolUse` | `execute_tool {tool}` | `gen_ai.tool.name`, `gen_ai.tool.call.id`, `gen_ai.tool.call.arguments`, `gen_ai.tool.call.result` |
| any other hook | `hook {event}` | `rtok.hook.event`; `UserPromptSubmit` adds `gen_ai.input.messages` |
| an MCP tool call | `execute_tool {name}` | `gen_ai.tool.name`, `rtok.plugin`, arguments and result |
| a proxied request | `chat {model}` (CLIENT) | `gen_ai.provider.name`, `gen_ai.request.model`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_creation.input_tokens`, both message bodies |

Every span also carries `gen_ai.conversation.id`, `rtok.surface`, `rtok.kind` and `rtok.call.id`,
and an error sets the span status plus `error.type`. Plugin runs and savings ride along as span
events (`rtok.plugin.run`, `rtok.measurement` with `rtok.tokens.saved`). `logs` rows become log
records on the same trace. Three cumulative sums go to `/v1/metrics`: `rtok.tokens`
(by `gen_ai.token.type` and model; provider-billed only, so a response the semantic cache
served is left out), `rtok.tokens.saved` (by plugin) and `rtok.calls`.

Message content, tool arguments and results are omitted by default, even when an endpoint is
configured. Set `content = true` only if you want those values exported. With content enabled,
anything longer than `content_bytes` is cut and the span names the archive id, so the full bytes
stay retrievable with `rtok expand <id>`.

## Backends

Local `docker run` / `docker compose` examples below need a running Docker-compatible engine.
Prefer Colima from mise (`docs/colima.md`); Docker Desktop is not required.


### Jaeger

```bash
docker run --rm -p 16686:16686 -p 4318:4318 jaegertracing/jaeger:2.11.0
```

`endpoint = "http://localhost:4318"`, then open <http://localhost:16686> and pick the `rtok`
service. Jaeger 2.x serves traces only: it answers 404 to `/v1/logs` and `/v1/metrics`, which
`rtok otel flush` reports as `not served: logs, metrics` and skips. The `logs` watermark stays
put, so `rtok otel status` keeps counting pending log rows — expected, not an error.

### Grafana

```bash
docker run --rm -p 3000:3000 -p 4318:4318 grafana/otel-lgtm
```

`endpoint = "http://localhost:4318"`, then <http://localhost:3000> (admin/admin): Explore →
Tempo for traces, Loki for logs, Prometheus for `rtok_tokens_total` and `rtok_tokens_saved_total`.

Grafana Cloud instead:

```toml
[otel]
endpoint = "https://otlp-gateway-<region>.grafana.net/otlp"
headers  = "Authorization=Basic <base64 of instanceID:token>"
```

### SigNoz

Self-hosted, from a SigNoz checkout:

```bash
docker compose -f deploy/docker/docker-compose.yaml up -d
```

`endpoint = "http://localhost:4318"`. SigNoz Cloud instead:

```toml
[otel]
endpoint = "https://ingest.<region>.signoz.cloud:443"
headers  = "signoz-ingestion-key=<key>"
```

### Maple

```toml
[otel]
endpoint = "https://api.maple.dev/otlp"   # your workspace's OTLP URL
headers  = "Authorization=Bearer <key>"
```

Maple reads the GenAI attributes directly, so `chat` spans show the model, the token counts and
the messages without extra mapping.

## Troubleshooting

- `otel: no endpoint` — neither `[otel] endpoint` nor `OTEL_EXPORTER_OTLP_ENDPOINT` is set.
- Nothing appears — run `rtok otel flush` by hand; a failure prints and is stored, so
  `rtok otel status` shows the last error and the pending counts.
- Rows keep piling up — the watermark only advances on a 2xx. Check the collector's own log;
  a wrong path (`/v1/traces` is appended to the base URL) or a missing key is the usual cause.
- Traces but no prompts — `content = false`, or the body was longer than `content_bytes`.
- `not served: logs, metrics` — the backend has no pipeline for that stream (Jaeger); rtok
  skips it without logging, since a logged error would itself be a pending log row.
- `rtok_calls_total` but no `rtok_tokens_total` — the ledger has no `usage` rows; tokens come
  from the proxy or an imported transcript, hooks alone only count calls.

## Checking the payload without a backend

`tools/otlp_validator.py` is a standalone OTLP/HTTP JSON receiver that re-implements the spec's
encoding rules and reports what a backend would show:

```bash
python3 tools/otlp_validator.py 4318
```

Point `endpoint` at `http://127.0.0.1:4318`, run `rtok otel flush`, then stop it with Ctrl-C to
print the spans, logs and metrics it received plus any spec violations.
