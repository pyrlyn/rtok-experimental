#!/usr/bin/env bash
# Copyright (c) 2026 Ivan Tugay
# SPDX-License-Identifier: GPL-3.0-or-later
# Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

# T53.4: start Jaeger + Grafana on shifted ports, flush a fixture ledger, assert APIs.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

if ! command -v docker >/dev/null 2>&1; then
  echo "skip: docker CLI not found (use Colima — docs/colima.md)"
  exit 0
fi

if ! docker info >/dev/null 2>&1; then
  echo "skip: docker daemon not running (start Colima — docs/colima.md)"
  exit 0
fi

JAEGER_UI=16687
JAEGER_OTLP=4319
GRAFANA_UI=3001
GRAFANA_OTLP=4320

cleanup() {
  docker rm -f rtok-otel-jaeger rtok-otel-grafana >/dev/null 2>&1 || true
  [[ -n "${home:-}" ]] && rm -rf "$home"
}
trap cleanup EXIT

docker rm -f rtok-otel-jaeger rtok-otel-grafana >/dev/null 2>&1 || true
docker run -d --name rtok-otel-jaeger \
  -p "${JAEGER_UI}:16686" -p "${JAEGER_OTLP}:4318" \
  jaegertracing/jaeger:2.11.0 >/dev/null
docker run -d --name rtok-otel-grafana \
  -p "${GRAFANA_UI}:3000" -p "${GRAFANA_OTLP}:4318" \
  grafana/otel-lgtm >/dev/null

home="$(mktemp -d "${TMPDIR:-/tmp}/rtok-otel-check.XXXXXX")"
export RTOK_HOME="$home"
mkdir -p "$home/archive"

bin="${root}/target/release/rtok"
if [[ ! -x "$bin" ]]; then
  mise exec -- cargo build -q --release --bin rtok
fi

# Seed the same ledger shape as tests/otel.rs (hook + mcp + proxy + log + usage).
mise exec -- cargo test -q -p rtok --lib it::otel::seed_fixture_ledger -- --exact --ignored >/dev/null

flush() {
  local endpoint="$1"
  cat >"$home/config.toml" <<EOF
[otel]
endpoint = "${endpoint}"
service_name = "rtok"
flush_secs = 30
EOF
  "$bin" otel flush
}

jaeger_report="$(flush "http://127.0.0.1:${JAEGER_OTLP}")"
echo "$jaeger_report"
[[ "$jaeger_report" == *"spans"* ]] || [[ "$jaeger_report" == *"3 spans"* ]]
jaeger_json="$(curl -fsS "http://127.0.0.1:${JAEGER_UI}/api/traces?service=rtok&limit=5")"
echo "$jaeger_json" | rg -q 'execute_tool'

grafana_report="$(flush "http://127.0.0.1:${GRAFANA_OTLP}")"
echo "$grafana_report"
trace_id="$(printf '%s' "$jaeger_json" | python3 "${root}/tools/otel_trace_id.py")"
if ! curl -fsS "http://127.0.0.1:${GRAFANA_UI}/api/datasources/proxy/uid/tempo/api/traces/${trace_id}" >/dev/null 2>&1; then
  curl -fsS "http://127.0.0.1:${GRAFANA_UI}/api/search?tags=service.name%3Drtok" | rg -q "${trace_id}"
fi

prom_json="$(curl -fsS -G "http://127.0.0.1:${GRAFANA_UI}/api/prometheus/api/v1/query" --data-urlencode 'query=rtok_calls_total')"
echo "$prom_json" | rg -q 'rtok_calls_total'

echo "otel-check: jaeger execute_tool spans, tempo trace ${trace_id}, prometheus rtok_calls_total — ok"
