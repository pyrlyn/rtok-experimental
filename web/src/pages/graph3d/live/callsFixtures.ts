// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import type { CallsView, Finished, Running, WindowView } from "../../../api/snapshot.gen";

export const NOW = 1_000_000;

export const running = (call: string, over: Partial<Running> = {}): Running => ({
  call,
  tool: "callers",
  target: "open_index",
  project: "rtok",
  session: "session-abcdef",
  caller: "claude 3f9a1c2e",
  at: NOW,
  ...over,
});

/** A finished call whose measurement cut `before` tokens down to `after`. */
export const done = (
  call: string,
  before: number,
  after: number,
  over: Partial<Finished> = {},
): Finished => ({
  call,
  tool: "callers",
  target: "open_index",
  project: "rtok",
  session: "session-abcdef",
  caller: "claude 3f9a1c2e",
  ok: true,
  error: null,
  backend: "tags",
  ms: 12,
  before,
  after,
  at: NOW,
  interrupted: false,
  ...over,
});

export const LABELS = ["1 min", "5 min", "15 min", "since start"];

/** What the server would total for `feed`; `over` sets the figures that come from the batch summary. */
export function windowOf(
  label: string,
  feed: readonly Finished[],
  over: Partial<WindowView> = {},
): WindowView {
  const tools = new Map<string, { calls: number; saved: number }>();
  const backends: Record<string, number> = {};
  for (const r of feed) {
    const t = tools.get(r.tool) ?? { calls: 0, saved: 0 };
    tools.set(r.tool, { calls: t.calls + 1, saved: t.saved + r.before - r.after });
    if (r.backend) backends[r.backend] = (backends[r.backend] ?? 0) + 1;
  }
  return {
    label,
    calls: feed.length,
    failed: feed.filter((r) => !r.ok).length,
    before: feed.reduce((n, r) => n + r.before, 0),
    after: feed.reduce((n, r) => n + r.after, 0),
    tools: [...tools].map(([tool, v]) => ({ tool, ...v })).sort((a, b) => b.calls - a.calls),
    backends,
    fallbacks: 0,
    caps: 0,
    symbols: 0,
    crossed: 0,
    symbols_returned: 0,
    files_touched: 0,
    projects_hit: 0,
    latency: null,
    spark: { span_ms: 300_000, calls: [], saved: [] },
    ...over,
  };
}

/** A `calls` frame in which all four windows total the same `feed`, unless `windows` says otherwise. */
export function view(over: Partial<CallsView> & { totals?: Partial<WindowView> } = {}): CallsView {
  const { totals, ...rest } = over;
  const feed = rest.feed ?? [];
  return {
    now: NOW,
    running: [],
    feed,
    heat_window_s: 300,
    windows: LABELS.map((label) => windowOf(label, feed, totals)),
    ...rest,
  };
}
