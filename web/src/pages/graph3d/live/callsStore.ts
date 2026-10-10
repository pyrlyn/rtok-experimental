// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import type { CallBatch, GraphEvent } from "../../../api/snapshot.gen";

export const FEED_ROWS = 200;
/** A call with no end event this long after its start is shown as interrupted: its process is gone. */
export const INTERRUPT_MS = 120_000;
/** Concurrent calls drawn with their own accent; the rest merge into one "busy" count. */
export const MAX_ACCENTS = 8;
export const HEAT_MS = 300_000;

export const WINDOWS = [
  { id: "1m", label: "1 min", ms: 60_000 },
  { id: "5m", label: "5 min", ms: 300_000 },
  { id: "15m", label: "15 min", ms: 900_000 },
  { id: "all", label: "since open", ms: Infinity },
] as const;
export type WindowId = (typeof WINDOWS)[number]["id"];
const LONGEST_BUCKET_MS = 900_000;

export interface Running {
  call: string;
  tool: string;
  target: string | null;
  project: string | null;
  session: string;
  /** Browser clock: the elapsed time and the interrupt timeout must not depend on the server's. */
  at: number;
  done: number | null;
  total: number | null;
}

export interface Finished {
  call: string;
  tool: string;
  target: string | null;
  project: string | null;
  session: string;
  ok: boolean;
  error: string | null;
  backend: string | null;
  ms: number | null;
  before: number;
  after: number;
  at: number;
  interrupted: boolean;
}

export interface Totals {
  calls: number;
  failed: number;
  before: number;
  after: number;
  /** Counted from the events a batch lists, so under a burst the bars cover the listed calls; `calls` is exact. */
  tools: Record<string, { calls: number; saved: number }>;
  backends: Record<string, number>;
}

export interface CallsStore {
  running: Running[];
  /** Newest first. */
  feed: Finished[];
  /** One per batch, kept for the longest window. */
  buckets: (Totals & { at: number })[];
  all: Totals;
}

const noTotals = (): Totals => ({
  calls: 0,
  failed: 0,
  before: 0,
  after: 0,
  tools: {},
  backends: {},
});

export const emptyStore: CallsStore = { running: [], feed: [], buckets: [], all: noTotals() };

function add(into: Totals, from: Totals) {
  into.calls += from.calls;
  into.failed += from.failed;
  into.before += from.before;
  into.after += from.after;
  for (const [tool, t] of Object.entries(from.tools)) {
    const own = (into.tools[tool] ??= { calls: 0, saved: 0 });
    own.calls += t.calls;
    own.saved += t.saved;
  }
  for (const [b, n] of Object.entries(from.backends))
    into.backends[b] = (into.backends[b] ?? 0) + n;
}

const ended = (e: GraphEvent, at: number): Finished => ({
  call: e.call,
  tool: e.tool,
  target: e.target,
  project: e.project,
  session: e.session,
  ok: e.ok,
  error: e.error,
  backend: e.backend,
  ms: e.ms,
  before: e.samples.reduce((n, s) => n + s.est_before, 0),
  after: e.samples.reduce((n, s) => n + s.est_after, 0),
  at,
  interrupted: false,
});

/**
 * Folds one batch in. Totals come from the batch `summary`, which counts every event, so a
 * burst that the batch cut down to 100 listed events still adds up to what `rtok stats` sums.
 */
export function fold(store: CallsStore, batch: CallBatch, now: number): CallsStore {
  const ends = batch.events.filter((e) => e.phase === "end");
  const rows = ends.map((e) => ended(e, now));
  const done = new Set(ends.map((e) => e.call));
  const running = new Map(store.running.map((r) => [r.call, r]));
  for (const e of batch.events) {
    if (e.phase === "end") running.delete(e.call);
    else if (!done.has(e.call)) {
      running.set(e.call, {
        call: e.call,
        tool: e.tool,
        target: e.target,
        project: e.project,
        session: e.session,
        at: running.get(e.call)?.at ?? now,
        done: e.done,
        total: e.total,
      });
    }
  }
  const bucket: Totals = {
    calls: batch.summary.ends,
    failed: batch.summary.failed,
    before: batch.summary.est_before,
    after: batch.summary.est_after,
    tools: {},
    backends: {},
  };
  for (const [i, e] of ends.entries()) {
    const t = (bucket.tools[e.tool] ??= { calls: 0, saved: 0 });
    t.calls += 1;
    t.saved += rows[i]!.before - rows[i]!.after;
    if (e.backend) bucket.backends[e.backend] = (bucket.backends[e.backend] ?? 0) + 1;
  }
  const all = structuredClone(store.all);
  add(all, bucket);
  return {
    running: [...running.values()],
    // A late end for a call already marked interrupted replaces the mark.
    feed: [...rows.reverse(), ...store.feed.filter((r) => !done.has(r.call))].slice(0, FEED_ROWS),
    buckets: [...store.buckets, { ...bucket, at: now }].filter(
      (b) => now - b.at <= LONGEST_BUCKET_MS,
    ),
    all,
  };
}

/** Moves a call that never ended into the feed, marked, so it stops counting as running. */
export function sweep(store: CallsStore, now: number): CallsStore {
  const stale = store.running.filter((r) => now - r.at > INTERRUPT_MS);
  if (!stale.length) return store;
  const rows: Finished[] = stale.map((r) => ({
    ...r,
    ok: false,
    error: null,
    backend: null,
    ms: null,
    before: 0,
    after: 0,
    interrupted: true,
  }));
  return {
    ...store,
    running: store.running.filter((r) => !stale.includes(r)),
    feed: [...rows.reverse(), ...store.feed].slice(0, FEED_ROWS),
  };
}

/** The window is summed from the store's buckets on every call, never kept as a second counter. */
export function windowTotals(store: CallsStore, id: WindowId, now: number): Totals {
  if (id === "all") return store.all;
  const ms = WINDOWS.find((w) => w.id === id)!.ms;
  const sum = noTotals();
  for (const b of store.buckets) if (now - b.at <= ms) add(sum, b);
  return sum;
}

export interface FeedFilter {
  session: string;
  tool: string;
  project: string;
}

/** Empty strings mean "all". */
export function filterFeed(feed: readonly Finished[], f: FeedFilter): Finished[] {
  return feed.filter(
    (r) =>
      (!f.session || r.session === f.session) &&
      (!f.tool || r.tool === f.tool) &&
      (!f.project || r.project === f.project),
  );
}

export const distinct = (
  feed: readonly Finished[],
  key: "session" | "tool" | "project",
): string[] => [...new Set(feed.flatMap((r) => (r[key] ? [r[key]] : [])))].sort();
