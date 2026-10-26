#!/usr/bin/env bash
# Copyright (c) 2026 Ivan Tugay
# SPDX-License-Identifier: GPL-3.0-or-later
# Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

# `just check` (T420): the gate for what this change touches. `just full-check` is the whole gate.
#
# The change is every file that differs from the merge base with main, committed or not, plus
# untracked files. It decides which `just` recipes run:
#
#   shared input (Cargo.toml, build.rs, justfile, crates/, config/, ...)  -> `just full-check`
#   Markdown, docs/, site/, .github/, brand/                              -> nothing from cargo
#   *.rs under src/ or tests/                                             -> fmt-check, lint, dup,
#                                                                            test-changed, and
#                                                                            build-min for src/
#   *.ts/*.tsx/*.js/*.mjs/*.cjs                                           -> js, dup
#   *.py                                                                  -> python, dup
#   anything else                                                         -> test-changed (by name)
#
# The test selection is the name match in tools/test-changed.sh (research.md §33 says why not
# rtok's code graph). When the merge base cannot be computed, the answer is the full gate: a gate
# that skips what it could not check would be worse than a slow one.
#
# Usage: tools/selective-check.sh        (SELECTIVE_DRY=1 prints the recipes instead of running)
# Env:   CHECK_BASE=<rev> overrides the merge base; RTOK_CHANGED (newline-separated paths)
#        replaces the git query, like in test-changed.sh.
set -euo pipefail

just="${JUST:-just}"

base="${CHECK_BASE:-}"
if [[ -z "$base" ]]; then
  for ref in pyrlyn/main origin/main main; do
    if git rev-parse --verify -q "$ref^{commit}" >/dev/null; then
      base="$(git merge-base HEAD "$ref" 2>/dev/null || true)"
      [[ -n "$base" ]] && break
    fi
  done
fi

full() {
  echo "check: $1 — running the full gate"
  if [[ -n "${SELECTIVE_DRY:-}" ]]; then
    echo "plan: full-check"
    exit 0
  fi
  exec $just full-check
}

[[ -n "$base" ]] || full "no merge base with main"

changed="${RTOK_CHANGED-$(
  {
    git diff --name-only "$base" --
    git ls-files --others --exclude-standard
  } | sort -u
)}"

rust=0 src=0 js=0 py=0 other=0
while IFS= read -r f; do
  [[ -n "$f" ]] || continue
  case "$f" in
    Cargo.toml | Cargo.lock | build.rs | justfile | mise.toml | rust-toolchain* | crates/* | config/* | .config/* | .cargo/* | migrations/* | tests/it/main.rs | tests/it/common/* | tests/it/snapshots/* | tests/fixtures/* | tests/trycmd/* | tools/selective-check.sh | tools/test-changed.sh)
      full "$f is a shared input"
      ;;
    *.md | docs/* | site/* | .github/* | brand/* | LICENSE* | *.txt) ;;
    src/*.rs)
      rust=1 src=1
      ;;
    *.rs) rust=1 ;;
    *.ts | *.tsx | *.js | *.mjs | *.cjs) js=1 ;;
    *.py) py=1 ;;
    *) other=1 ;;
  esac
done <<<"$changed"

steps=()
if ((rust)); then
  steps+=(fmt-check lint)
fi
((src)) && steps+=(build-min)
((rust || js || py)) && steps+=(dup)
((js)) && steps+=(js)
((py)) && steps+=(python)

if ((rust || other)); then
  steps+=("test-changed $base")
fi

if ((${#steps[@]} == 0)); then
  echo "check: no code changed against ${base:0:8} — no cargo tests (run \`just docs-check\` for the Markdown ones, \`just full-check\` for everything)"
  [[ -n "${SELECTIVE_DRY:-}" ]] && echo "plan: (nothing)"
  exit 0
fi

echo "check: against ${base:0:8}: ${steps[*]}"
if [[ -n "${SELECTIVE_DRY:-}" ]]; then
  echo "plan: ${steps[*]}"
  exit 0
fi

# fmt-check fails first, in a second; dup/js/python run beside the cargo chain, which stays
# sequential (one target/, cargo's lock). A failure anywhere fails the run, after the rest finished.
fail=0
pids=()
cargo_steps=()
for s in "${steps[@]}"; do
  case "$s" in
    dup | js | python)
      $just "$s" &
      pids+=($!)
      ;;
    *) cargo_steps+=("$s") ;;
  esac
done
for s in "${cargo_steps[@]}"; do
  # shellcheck disable=SC2086
  $just $s || {
    fail=1
    break
  }
done
for p in "${pids[@]:-}"; do
  [[ -n "$p" ]] && { wait "$p" || fail=1; }
done

exit $fail
