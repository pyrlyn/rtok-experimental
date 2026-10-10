// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { describe, expect, test } from "vitest";
import { project } from "../../../api/sampleRows";
import { buildScene } from "../scene";
import {
  distinct,
  emptyStore,
  FEED_ROWS,
  filterFeed,
  fold,
  INTERRUPT_MS,
  MAX_ACCENTS,
  sweep,
  windowTotals,
} from "./callsStore";
import { batch, end, event } from "./callsFixtures";
import { liveState } from "./lit";

const T = 1_000_000;

describe("fold", () => {
  test("a start runs, its end moves it to the feed with the measured tokens", () => {
    const running = fold(emptyStore, batch([event({ call: "a" })]), T);
    expect(running.running.map((r) => r.call)).toEqual(["a"]);
    const done = fold(running, batch([end("a", 100, 30)]), T + 50);
    expect(done.running).toEqual([]);
    expect(done.feed[0]).toMatchObject({ call: "a", before: 100, after: 30, backend: "tags" });
    expect(done.all).toMatchObject({ calls: 1, before: 100, after: 30 });
    expect(done.all.tools.callers).toEqual({ calls: 1, saved: 70 });
  });

  test("a failed end counts and keeps its error", () => {
    const s = fold(emptyStore, batch([end("a", 0, 0, { ok: false, error: "no backend" })]), T);
    expect(s.all.failed).toBe(1);
    expect(s.feed[0]).toMatchObject({ ok: false, error: "no backend" });
  });

  test("totals count the events a burst batch left out", () => {
    const s = fold(emptyStore, batch([end("a", 100, 30)], 400), T);
    expect(s.all.calls).toBe(401);
    expect(s.all.before).toBe(100 + 4000);
    expect(s.all.after).toBe(30 + 1600);
  });

  test("the feed keeps the newest rows first, up to the cap", () => {
    let s = emptyStore;
    for (let i = 0; i < FEED_ROWS + 5; i++) s = fold(s, batch([end(`c${i}`, 2, 1)]), T + i);
    expect(s.feed).toHaveLength(FEED_ROWS);
    expect(s.feed[0]!.call).toBe(`c${FEED_ROWS + 4}`);
    expect(s.all.calls).toBe(FEED_ROWS + 5);
  });
});

describe("windows", () => {
  test("each window is summed from the store's batches", () => {
    let s = fold(emptyStore, batch([end("old", 10, 5)]), T);
    s = fold(s, batch([end("mid", 20, 5)]), T + 6 * 60_000);
    s = fold(s, batch([end("new", 40, 5)]), T + 9 * 60_000);
    const now = T + 9 * 60_000 + 1000;
    expect(windowTotals(s, "1m", now).before).toBe(40);
    expect(windowTotals(s, "5m", now).before).toBe(60);
    expect(windowTotals(s, "15m", now).before).toBe(70);
    expect(windowTotals(s, "all", now).before).toBe(70);
  });

  test("a batch older than the longest window leaves the buckets but not the total", () => {
    let s = fold(emptyStore, batch([end("old", 10, 5)]), T);
    s = fold(s, batch([end("new", 40, 5)]), T + 16 * 60_000);
    expect(s.buckets).toHaveLength(1);
    expect(windowTotals(s, "all", T + 16 * 60_000).calls).toBe(2);
  });
});

describe("interrupted calls", () => {
  test("a call with no end after the timeout is marked and stops running", () => {
    const s = fold(emptyStore, batch([event({ call: "a" })]), T);
    expect(sweep(s, T + INTERRUPT_MS)).toBe(s);
    const swept = sweep(s, T + INTERRUPT_MS + 1);
    expect(swept.running).toEqual([]);
    expect(swept.feed[0]).toMatchObject({ call: "a", interrupted: true });
    expect(swept.all.calls).toBe(0);
  });

  test("a late end replaces the mark", () => {
    const swept = sweep(fold(emptyStore, batch([event({ call: "a" })]), T), T + INTERRUPT_MS + 1);
    const s = fold(swept, batch([end("a", 10, 5)]), T + INTERRUPT_MS + 2);
    expect(s.feed.map((r) => [r.call, r.interrupted])).toEqual([["a", false]]);
  });
});

describe("feed filters", () => {
  test("agent, tool and project narrow the rows; blank means all", () => {
    const s = fold(
      emptyStore,
      batch([
        end("a", 2, 1, { session: "x", tool: "callers", project: "p" }),
        end("b", 2, 1, { session: "y", tool: "impact", project: "p" }),
        end("c", 2, 1, { session: "y", tool: "callers", project: "q" }),
      ]),
      T,
    );
    const none = { session: "", tool: "", project: "" };
    expect(filterFeed(s.feed, none)).toHaveLength(3);
    expect(filterFeed(s.feed, { ...none, session: "y" }).map((r) => r.call)).toEqual(["c", "b"]);
    expect(filterFeed(s.feed, { ...none, session: "y", tool: "callers" })).toHaveLength(1);
    expect(distinct(s.feed, "project")).toEqual(["p", "q"]);
  });
});

describe("liveState", () => {
  const scene = buildScene([project(1, "rtok"), project(2, "ketch")], {
    query: "",
    scopeOnly: false,
  });
  const id = (name: string) => scene.nodes.find((n) => n.label === name)!.id;

  test("a running call gives its project an accent; a finished one leaves heat that fades", () => {
    const run = fold(emptyStore, batch([event({ call: "a", project: "rtok" })]), T);
    expect(liveState(scene, run, T).lit.get(id("rtok"))!.accents).toEqual([0]);
    const done = fold(run, batch([end("a", 2, 1, { project: "rtok" })]), T);
    const fresh = liveState(scene, done, T).lit.get(id("rtok"))!.heat;
    const later = liveState(scene, done, T + 200_000).lit.get(id("rtok"))!.heat;
    expect(fresh).toBeGreaterThan(later);
    expect(liveState(scene, done, T + 400_000).lit.size).toBe(0);
    expect(liveState(scene, done, T).lit.has(id("ketch"))).toBe(false);
  });

  test("a failed call flashes red briefly", () => {
    const s = fold(emptyStore, batch([end("a", 0, 0, { ok: false, project: "ketch" })]), T);
    expect(liveState(scene, s, T + 1000).lit.get(id("ketch"))!.failed).toBe(true);
    expect(liveState(scene, s, T + 9000).lit.get(id("ketch"))!.failed).toBe(false);
  });

  test("more than eight running calls merge into a busy count", () => {
    const calls = Array.from({ length: MAX_ACCENTS + 3 }, (_, i) => event({ call: `c${i}` }));
    const s = fold(emptyStore, batch(calls), T);
    const live = liveState(scene, s, T);
    expect(live.busy).toBe(3);
    expect(live.lit.get(id("rtok"))!.accents).toHaveLength(MAX_ACCENTS);
  });
});
