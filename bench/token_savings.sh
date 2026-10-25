#!/usr/bin/env bash
# Offline token-savings bench: fixed fixtures, no model calls, no API keys.
# Prints one `SAVINGS ...` line per mechanism; the tests behind them also assert the floors.
#   bench/token_savings.sh
set -euo pipefail
cd "$(dirname "$0")/.."
cargo nextest run --lib --no-capture -E 'test(/savings_/)' 2>&1 | grep '^SAVINGS'
