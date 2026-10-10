// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { useCallback, useEffect, useRef, useState } from "react";
import { useCallStream } from "../../../api/query";
import { type CallsStore, emptyStore, fold, sweep } from "./callsStore";

/**
 * The call stream as a store the page can freeze. The latest store always takes every batch,
 * so a freeze holds the picture and nothing else: the totals are exact the moment it lifts.
 * Painting is batched to one state update per animation frame, however many batches arrive.
 */
export function useCalls() {
  const latest = useRef<CallsStore>(emptyStore);
  const frozen = useRef(false);
  const frame = useRef(0);
  const [shown, setShown] = useState<CallsStore>(emptyStore);
  const [now, setNow] = useState(() => Date.now());
  const [held, setHeld] = useState(false);
  const heldAt = useRef(0);
  const [pending, setPending] = useState(0);

  const paint = useCallback(() => {
    frame.current = 0;
    if (frozen.current) setPending(latest.current.all.calls - heldAt.current);
    else setShown(latest.current);
  }, []);
  const schedule = useCallback(() => {
    if (!frame.current) frame.current = requestAnimationFrame(paint);
  }, [paint]);

  useCallStream((batch) => {
    latest.current = fold(latest.current, batch, Date.now());
    schedule();
  });

  useEffect(() => {
    const timer = setInterval(() => {
      const t = Date.now();
      const next = sweep(latest.current, t);
      if (next !== latest.current) {
        latest.current = next;
        schedule();
      }
      if (!frozen.current) setNow(t);
    }, 1000);
    return () => {
      clearInterval(timer);
      cancelAnimationFrame(frame.current);
    };
  }, [schedule]);

  const freeze = useCallback((on: boolean) => {
    frozen.current = on;
    heldAt.current = latest.current.all.calls;
    setHeld(on);
    setPending(0);
    if (!on) {
      setNow(Date.now());
      setShown(latest.current);
    }
  }, []);
  return { store: shown, now, frozen: held, freeze, pending };
}
