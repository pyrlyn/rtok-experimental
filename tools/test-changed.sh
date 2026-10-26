#!/usr/bin/env bash
# Copyright (c) 2026 Ivan Tugay
# SPDX-License-Identifier: GPL-3.0-or-later
# Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

# `just test-changed [rev]` — run only the tests the current change can reach.
#
# T985: the integration tests are one binary (`it`, tests/it/main.rs) with one module per former
# tests/<name>.rs, so there is a single link and the selection is a nextest filter on module
# names: `binary(it) & test(/^(<name>|...)::/)`. `--test it` is only passed when at least one
# module is picked, so a change that reaches no integration test does not link `it` at all.
#
# Selection, cheapest signal first:
#   tests/it/<name>.rs changed  -> module <name>
#   a shared input changed      -> the whole suite (Cargo.toml, crates/, config/, justfile,
#                                  tests/it/main.rs, tests/it/common, fixtures, snapshots: too
#                                  wide to map by name)
#   anything else changed       -> --lib, plus every tests/it/<name>.rs whose file name or body
#                                  mentions one of the changed path's segments
#
# The unit-test binary is then trimmed by name too (`-E 'kind(lib) & test(~seg)'`). Linking it is
# cheap, but running all of it is not: the TUI tests alone take ~75s of the ~76s wall clock.
#
# Markdown goes through the same name match rather than being ignored: `docs/agents.md` reaches
# the host-docs test, `plan/<id>/task.md` reaches the plan tests, and a doc nothing asserts on
# selects nothing but the unit tests.
#
# This is the selection behind `just check` (T420) and `just test-changed`. Targets are picked by
# name, so a test that exercises a module without ever naming it is not selected; `just
# full-check` is the whole gate. rtok's own code graph (`rtok graph affected`) was measured as
# the selector instead and rejected: its walk is keyed on bare symbol names, so one edit to
# src/plugins/checkpoint.rs (`write`, `kind`, `walk`, ...) reached 106 test paths, every test
# target there is, after minutes (research.md §33).
#
# Usage: tools/test-changed.sh [rev]   (default rev: HEAD, i.e. the working tree)
# Env:   TEST_CHANGED_DRY=1  print the nextest arguments instead of running them
set -euo pipefail

rev="${1:-HEAD}"
cargo="${CARGO:-cargo}"

run_nextest() {
  if [[ -n "${TEST_CHANGED_DRY:-}" ]]; then
    echo "nextest: $*"
    exit 0
  fi
  exec $cargo nextest run "$@"
}

# Path segments that name no module worth matching on: too generic to narrow anything.
stoplist='^(mod|lib|main|src|rs|md|tests|test|docs|doc|site|github|workflows|plugins|util|utils|common|core|types|impl|new)$'

# RTOK_CHANGED (newline-separated paths) replaces the git query, so the mapping can be
# exercised without staging a diff. Everything below it is the same code path.
changed="${RTOK_CHANGED-$(
  {
    git diff --name-only "$rev" --
    git ls-files --others --exclude-standard
  } | sort -u
)}"

if [[ -z "$changed" ]]; then
  echo "test-changed: no changes against $rev — running unit tests only"
  run_nextest -p rtok --lib
fi

# The integration modules: tests/it/<name>.rs, without the crate root.
modules() { ls tests/it/*.rs | grep -v '^tests/it/main\.rs$'; }

run_all=0
lib_all=0
targets=()
tokens=()

while IFS= read -r f; do
  [[ -n "$f" ]] || continue
  case "$f" in
    tests/it/main.rs) run_all=1 ;;
    tests/it/*/*) run_all=1 ;;                    # it/common/, it/snapshots/
    tests/it/*.rs) targets+=("$(basename "$f" .rs)") ;;
    tests/*/*) run_all=1 ;;                       # fixtures/, trycmd/, cmd_golden/, node/
    Cargo.toml | Cargo.lock | build.rs | migrations/* | justfile | mise.toml | crates/* | config/* | .config/* | .cargo/*)
      run_all=1
      ;;
    *)
      # src/plugins/graph/lsp.rs -> plugins graph lsp -> graph lsp
      before=${#tokens[@]}
      while IFS= read -r seg; do
        [[ "$seg" =~ $stoplist ]] && continue
        ((${#seg} >= 3)) && tokens+=("$seg")
      done < <(echo "$f" | tr '/.' '\n\n')
      # A source file whose path names no module (src/lib.rs, src/main.rs) cannot be matched by
      # name, so every unit test runs for it.
      if [[ "$f" == src/*.rs ]] && ((${#tokens[@]} == before)); then lib_all=1; fi
      ;;
  esac
done <<<"$changed"

if ((run_all)); then
  echo "test-changed: a shared input changed — running the whole suite"
  run_nextest --workspace
fi

# Every test file that names one of the touched modules, by file name or in its body.
if ((${#tokens[@]})); then
  pattern="$(printf '%s\n' "${tokens[@]}" | sort -u | paste -sd'|' -)"
  while IFS= read -r f; do
    targets+=("$(basename "$f" .rs)")
  done < <(grep -lwiE -- "$pattern" $(modules) || true)
  while IFS= read -r f; do
    targets+=("$(basename "$f" .rs)")
  done < <(modules | grep -iE -- "$pattern" || true)
fi

# `--no-tests=pass`: a unit-test filter that matches nothing is a pass here, not nextest's exit 4.
args=(-p rtok --no-tests=pass)
((${#tokens[@]} || lib_all)) && args+=(--lib)
picked=0
mods=()
while IFS= read -r t; do
  [[ -n "$t" ]] || continue
  mods+=("$t")
  picked=$((picked + 1))
done < <(printf '%s\n' "${targets[@]:-}" | sort -u)
((picked)) && args+=(--test it)
it_filter="binary(it) & test(/^($(printf '%s\n' "${mods[@]:-}" | paste -sd'|' -))::/)"

# Past roughly three quarters of the suite the selection has stopped paying for itself:
# the remaining targets link in parallel anyway, and running all of them is the honest answer.
total=$(modules | wc -l | tr -d ' ')
if ((picked * 4 >= total * 3)); then
  echo "test-changed: $picked of $total modules selected — running the whole suite instead"
  run_nextest --workspace
fi

# Same name match again, this time inside the unit-test binary: it links in seconds but runs for
# over a minute, almost all of it in TUI tests that no source change outside src/tui can reach.
# `-p rtok` alone would run everything, so "nothing reaches the change" has to stop here.
if ((picked == 0 && ${#tokens[@]} == 0 && !lib_all)); then
  echo "test-changed: nothing reaches the change — no tests selected"
  exit 0
fi

if ((lib_all)); then
  ((picked)) && args+=(-E "kind(lib) | $it_filter")
  echo "test-changed: $picked of $total integration modules + every unit test"
elif ((${#tokens[@]})); then
  unit="$(printf 'test(~%s)|' $(printf '%s\n' "${tokens[@]}" | sort -u) | sed 's/|$//')"
  if ((picked)); then
    args+=(-E "kind(lib) & ($unit) | $it_filter")
  else
    args+=(-E "kind(lib) & ($unit)")
  fi
  echo "test-changed: $picked of $total integration modules + the matching unit tests"
else
  args+=(-E "$it_filter")
  echo "test-changed: $picked of $total integration modules, no unit tests"
fi

run_nextest "${args[@]}"
