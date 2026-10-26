#!/bin/sh
# Copyright (c) 2026 Ivan Tugay
# SPDX-License-Identifier: GPL-3.0-or-later
# Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

# POSIX shell tests for the plugin launcher scripts and `rtok skills` as a plugin calls it.
# Usage: RTOK_BIN=target/debug/rtok sh tests/sh/plugin_scripts_test.sh
# Without RTOK_BIN the `rtok skills` cases are skipped; the launcher cases use a fake rtok.
set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
pass=0
fail=0
ok() { pass=$((pass + 1)); printf 'ok   %s\n' "$1"; }
no() { fail=$((fail + 1)); printf 'FAIL %s\n' "$1"; }
check() { if [ "$2" = "$3" ]; then ok "$1"; else no "$1: want [$3] got [$2]"; fi; }

# A fake rtok that echoes its argv, on a PATH with only the basics.
mkdir -p "$TMP/bin" "$TMP/home"
printf '#!/bin/sh\necho "fake $*"\n' >"$TMP/bin/rtok"
chmod +x "$TMP/bin/rtok"
BASE=/usr/bin:/bin

for pair in "claude:" "codebuddy: --host codebuddy" "zcode:"; do
  hook=${pair%%:*}
  extra=${pair#*:}
  out=$(HOME="$TMP/home" PATH="$TMP/bin:$BASE" sh "$ROOT/plugins/$hook/scripts/hook.sh" PreToolUse)
  check "$hook hook.sh forwards the event" "$out" "fake hook PreToolUse$extra"
done
# Missing rtok: silent except on SessionStart (only checked when no system rtok shadows it).
if [ ! -x /usr/local/bin/rtok ] && [ ! -x /opt/homebrew/bin/rtok ]; then
  out=$(HOME="$TMP/home" PATH="$BASE" sh "$ROOT/plugins/claude/scripts/hook.sh" PreToolUse)
  check "claude hook.sh silent without rtok" "$out" ""
  out=$(HOME="$TMP/home" PATH="$BASE" sh "$ROOT/plugins/claude/scripts/hook.sh" SessionStart)
  case $out in *"ketch install pyrlyn/rtok"*) ok "claude hook.sh SessionStart hint" ;; *) no "SessionStart hint: [$out]" ;; esac
  HOME="$TMP/home" PATH="$BASE" sh "$ROOT/plugins/commandcode/scripts/mcp.sh" 2>/dev/null
  check "mcp.sh exits 1 without rtok" "$?" "1"
else
  printf 'skip missing-rtok cases: a system rtok is installed\n'
fi
out=$(HOME="$TMP/home" PATH="$TMP/bin:$BASE" sh "$ROOT/plugins/commandcode/scripts/mcp.sh")
check "mcp.sh runs rtok mcp" "$out" "fake mcp"

if [ -n "${RTOK_BIN:-}" ]; then
  for agent in claude cursor copilot; do
    h="$TMP/sk-$agent"
    d="$h/.$agent/skills/demo"
    mkdir -p "$d" "$h/proj"
    printf -- '---\nname: demo\ndescription: d\n---\n' >"$d/SKILL.md"
    run() { HOME="$h" RTOK_HOME="$h/.rtok" RTOK_AGENT="$agent" "$RTOK_BIN" skills "$@" --project "$h/proj"; }
    run disable demo >/dev/null
    check "$agent disable moves the dir" "$([ -f "$h/.$agent/skills-disabled/demo/SKILL.md" ] && echo y)" "y"
    run enable demo >/dev/null
    check "$agent enable moves it back" "$([ -f "$d/SKILL.md" ] && echo y)" "y"
    case $(run list --bytes) in *"34 B"*" 34 demo"*) ok "$agent list shows size and bytes" ;; *) no "$agent list size" ;; esac
  done
else
  printf 'skip rtok skills cases: RTOK_BIN not set\n'
fi

printf '%s passed, %s failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
