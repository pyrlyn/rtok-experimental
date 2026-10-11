#!/bin/sh
# Copyright (c) 2026 Ivan Tugay
# SPDX-License-Identifier: GPL-3.0-or-later
# Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

# rtok hook launcher for the CodeBuddy Code / WorkBuddy plugin (D21: fail open with the ketch
# hint). The desktop app may start without a shell PATH, so the binary is resolved by hand.
for bin in "$(command -v rtok 2>/dev/null)" "$HOME/.ketch/bin/rtok" \
  /usr/local/bin/rtok /opt/homebrew/bin/rtok; do
  if [ -n "$bin" ] && [ -x "$bin" ]; then
    exec "$bin" hook "$1" --host codebuddy
  fi
done
# Silent on every event but SessionStart, so a missing rtok is not a hook error per tool call;
# SessionStart names the install command once, in the Claude-shaped envelope CodeBuddy reads.
if [ "$1" = "SessionStart" ]; then
  printf '%s' '{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"rtok is not installed; run ketch install pyrlyn/rtok to enable it."}}'
fi
exit 0
