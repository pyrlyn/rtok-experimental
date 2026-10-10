// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import type { CallBatch, GraphEvent } from "../../../api/snapshot.gen";

export const event = (over: Partial<GraphEvent> = {}): GraphEvent => ({
  answer_tokens: null,
  backend: null,
  call: "s:1",
  done: null,
  error: null,
  id: 1,
  ms: null,
  ok: true,
  phase: "start",
  project: "rtok",
  samples: [],
  session: "session-abcdef",
  target: "open_index",
  tool: "callers",
  total: null,
  ts_ms: 0,
  ...over,
});

/** An end event whose one measurement row cut `before` tokens down to `after`. */
export const end = (call: string, before: number, after: number, over: Partial<GraphEvent> = {}) =>
  event({
    call,
    phase: "end",
    backend: "tags",
    ms: 12,
    samples: [
      {
        id: 1,
        kind: "graph.callers",
        before_bytes: 0,
        after_bytes: 0,
        est_before: before,
        est_after: after,
      },
    ],
    ...over,
  });

/** A batch whose summary counts exactly the events it lists, plus `omitted` ends of 10 to 4 tokens. */
export function batch(events: GraphEvent[], omitted = 0): CallBatch {
  const ends = events.filter((e) => e.phase === "end");
  return {
    events,
    head: events.length,
    omitted,
    summary: {
      starts: events.filter((e) => e.phase === "start").length,
      ends: ends.length + omitted,
      failed: ends.filter((e) => !e.ok).length,
      est_before:
        ends.reduce((n, e) => n + e.samples.reduce((m, s) => m + s.est_before, 0), 0) +
        omitted * 10,
      est_after:
        ends.reduce((n, e) => n + e.samples.reduce((m, s) => m + s.est_after, 0), 0) + omitted * 4,
    },
  };
}
