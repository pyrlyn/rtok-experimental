// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { useCallback, useEffect, useRef, useState } from "react";
import { useCallStream } from "../../../api/query";
import type { CallsView, WindowView } from "../../../api/snapshot.gen";

export const emptyCalls: CallsView = {
  now: 0,
  running: [],
  feed: [],
  heat_window_s: 300,
  windows: [],
};
export const emptyWindow: WindowView = {
  label: "",
  calls: 0,
  failed: 0,
  before: 0,
  after: 0,
  tools: [],
  backends: {},
  fallbacks: 0,
  caps: 0,
  symbols: 0,
  crossed: 0,
  symbols_returned: 0,
  files_touched: 0,
  projects_hit: 0,
  latency: null,
  spark: { span_ms: 0, calls: [], saved: [] },
};

/** The server totals "since start" last, so its count is every call the stream has seen. */
export const totalCalls = (v: CallsView) => v.windows.at(-1)?.calls ?? 0;

/**
 * The server's calls frames as a picture the page can freeze. The totals are the server's
 * (`rtok tui` shows the same ones), so a freeze only holds the frame on screen: the latest frame
 * keeps arriving, and the totals are exact the moment it lifts.
 */
export function useCalls() {
  const latest = useRef<CallsView>(emptyCalls);
  const frozen = useRef(false);
  const heldAt = useRef(0);
  // The frame's `now` is the server's clock: the page follows it by an offset, so rows age on
  // one clock between frames whatever the browser's says.
  const skew = useRef(0);
  const [shown, setShown] = useState<CallsView>(emptyCalls);
  const [now, setNow] = useState(() => Date.now());
  const [held, setHeld] = useState(false);
  const [pending, setPending] = useState(0);

  useCallStream((view) => {
    latest.current = view;
    skew.current = Date.now() - view.now;
    if (frozen.current) {
      setPending(totalCalls(view) - heldAt.current);
      return;
    }
    setShown(view);
    setNow(view.now);
  });

  useEffect(() => {
    const timer = setInterval(() => {
      if (!frozen.current) setNow(Date.now() - skew.current);
    }, 1000);
    return () => clearInterval(timer);
  }, []);

  const freeze = useCallback((on: boolean) => {
    frozen.current = on;
    heldAt.current = totalCalls(latest.current);
    setHeld(on);
    setPending(0);
    if (!on) {
      setNow(Date.now() - skew.current);
      setShown(latest.current);
    }
  }, []);
  return { view: shown, now, frozen: held, freeze, pending };
}
