#!/bin/sh
# Copyright (c) 2026 Ivan Tugay
# SPDX-License-Identifier: GPL-3.0-or-later
# Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

# Stdio command for a by-hand `cmd mcp add rtok`; the installer registers the server itself (T275/D33).
# The desktop app may start without a shell PATH: resolve by hand, builtins only.
for bin in "$(command -v rtok 2>/dev/null)" "$HOME/.ketch/bin/rtok" \
  /usr/local/bin/rtok /opt/homebrew/bin/rtok; do
  if [ -n "$bin" ] && [ -x "$bin" ]; then
    exec "$bin" mcp
  fi
done
printf '%s\n' "rtok is not installed." "" "Install with ketch:" "  ketch install pyrlyn/rtok" "" "If ketch is not installed:" "  curl -fsSL https://raw.githubusercontent.com/pyrlyn/ketch/main/install.sh | bash" "  ketch install pyrlyn/rtok" >&2
exit 1
