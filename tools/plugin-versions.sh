#!/usr/bin/env bash
# Copyright (c) 2026 Ivan Tugay
# SPDX-License-Identifier: GPL-3.0-or-later
# Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

# T279: the one place that writes and checks every plugin manifest version and every
# `.rtok-plugin-version` file, so `plugins/<host>` and `Cargo.toml` cannot drift apart.
#
# Usage:
#   tools/plugin-versions.sh --set <version>     write <version> into every file below
#   tools/plugin-versions.sh --check <version>   print each file whose version differs,
#                                                 exit 1 if any do, else exit 0
#   tools/plugin-versions.sh --files             print every file this script touches,
#                                                 one per line, repo-root-relative
#
# Called by tools/release.sh in the same commit that raises Cargo.toml/Cargo.lock (also for
# --files, so the commit's `git add` list is not a second copy of these paths), by
# .github/workflows/ci.yml (--check against the Cargo.toml version on every PR) and by
# .github/workflows/release.yml (--check against the tag, `v` stripped, before building).
# docs/plugin-versions.md documents the full scheme; the Rust test in
# tests/plugin_versions.rs reads its own file list from `--files`, so the test and this
# script can never disagree (`just check`).
set -eu

cd "$(dirname "$0")/.."

# plugins/<host>/.rtok-plugin-version, one per host plugin tree (T279 step 1). Not every
# directory under plugins/ is here: `cline` has no manifest of its own (hooks only) and
# `antigravity`'s plugin.json carries no version field for its host to cache by.
VERSION_HOSTS="
claude
codebuddy
codex
copilot
cursor
devin
gemini
grok
qwen
kimi
opencode
pi
zcode
"

# Manifests that carry a top-level "version" field, raised in step with the hosts above.
MANIFEST_FILES="
plugins/claude/.claude-plugin/plugin.json
plugins/codebuddy/.codebuddy-plugin/plugin.json
plugins/codex/.codex-plugin/plugin.json
plugins/cursor/plugin.json
plugins/cursor/.cursor-plugin/plugin.json
plugins/copilot/plugin.json
plugins/gemini/gemini-extension.json
plugins/qwen/qwen-extension.json
plugins/devin/.devin-plugin/plugin.json
plugins/zcode/.zcode-plugin/plugin.json
plugins/grok/.grok-plugin/plugin.json
plugins/kimi/kimi.plugin.json
plugins/pi/package.json
"

usage() {
  echo "usage: $0 --set <version> | --check <version> | --files" >&2
  exit 2
}

if [ "$#" -eq 1 ] && [ "$1" = "--files" ]; then
  mode="--files"
elif [ "$#" -eq 2 ]; then
  mode="$1"
  version="$2"
  case "$version" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *)
      echo "not a SemVer version: '$version'" >&2
      exit 2
      ;;
  esac
else
  usage
fi

version_file() {
  echo "plugins/$1/.rtok-plugin-version"
}

# The manifests are hand-authored with inline arrays (e.g. kimi's hooks list, the `keywords`
# arrays) that a JSON pretty-printer would explode onto their own lines. A sed limited to the
# top-level "version" line keeps every other byte untouched. Each manifest has exactly one
# "version" key (verified below), so a plain substitution cannot hit the wrong one.
manifest_version() {
  sed -n -E 's/.*"version": "([^"]*)".*/\1/p' "$1" | head -n1
}

set_manifest_version() {
  file="$1"
  count=$(grep -c '"version": "' "$file")
  if [ "$count" -ne 1 ]; then
    echo "$file: expected exactly one \"version\" field, found $count" >&2
    exit 1
  fi
  sed -i.bak -E "s/\"version\": \"[^\"]*\"/\"version\": \"$version\"/" "$file"
  rm -f "$file.bak"
}

version_file_version() {
  sed -n -E 's/.*"version":"([^"]*)".*/\1/p' "$1" | head -n1
}

case "$mode" in
  --files)
    for host in $VERSION_HOSTS; do
      version_file "$host"
    done
    for file in $MANIFEST_FILES; do
      printf '%s\n' "$file"
    done
    ;;
  --set)
    for host in $VERSION_HOSTS; do
      printf '{"schema":1,"plugin":"%s","version":"%s"}\n' "$host" "$version" \
        >"$(version_file "$host")"
    done
    for file in $MANIFEST_FILES; do
      [ -f "$file" ] || {
        echo "$file: missing" >&2
        exit 1
      }
      set_manifest_version "$file"
    done
    ;;
  --check)
    bad=0
    for host in $VERSION_HOSTS; do
      file=$(version_file "$host")
      if [ ! -f "$file" ]; then
        echo "$file: missing"
        bad=1
        continue
      fi
      got=$(version_file_version "$file")
      if [ "$got" != "$version" ]; then
        echo "$file: $got (want $version)"
        bad=1
      fi
    done
    for file in $MANIFEST_FILES; do
      if [ ! -f "$file" ]; then
        echo "$file: missing"
        bad=1
        continue
      fi
      got=$(manifest_version "$file")
      if [ "$got" != "$version" ]; then
        echo "$file: $got (want $version)"
        bad=1
      fi
    done
    exit "$bad"
    ;;
  *)
    usage
    ;;
esac
