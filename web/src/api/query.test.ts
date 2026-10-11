// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { QueryClient } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  connectionKey,
  createApi,
  DOCTOR_TIMEOUT_MS,
  pausedKey,
  snapshotKey,
  WRITE_TIMEOUT_MS,
  type Api,
} from "./query";
import { connectSample, isSampleRequested, sampleSnapshot } from "./sample";
import type { Snapshot } from "./snapshot.gen";
import type { Connect, Connection, Frame, Handlers } from "./ws";

// A scripted connection: the test plays the server by calling the captured handlers.
function scripted() {
  let handlers!: Handlers;
  const sent: unknown[] = [];
  let open = true;
  const connection: Connection = {
    send: (m) => (open ? (sent.push(m), true) : false),
    close: () => handlers.onState("closed"),
  };
  const connect: Connect = (h) => ((handlers = h), connection);
  return { connect, sent, server: () => handlers, setOpen: (v: boolean) => (open = v) };
}

let queryClient: QueryClient;
beforeEach(() => {
  queryClient = new QueryClient();
});
afterEach(() => vi.useRealTimers());

describe("cache wiring", () => {
  test("each snapshot replaces the cached one and connection state is tracked", () => {
    const s = scripted();
    createApi(queryClient, s.connect).open();
    s.server().onState("open");
    s.server().onFrame({ type: "snapshot", snapshot: sampleSnapshot });
    expect(queryClient.getQueryData(connectionKey)).toBe("open");
    expect(queryClient.getQueryData(snapshotKey)).toEqual(sampleSnapshot);

    const next: Snapshot = { ...sampleSnapshot, logs: ["later"] };
    s.server().onFrame({ type: "snapshot", snapshot: next });
    expect(queryClient.getQueryData<Snapshot>(snapshotKey)?.logs).toEqual(["later"]);
  });

  test("a failed tick keeps the last pages and sets the error; the next good frame clears it", () => {
    const s = scripted();
    createApi(queryClient, s.connect).open();
    s.server().onFrame({ type: "snapshot_error", error: "early" });
    expect(queryClient.getQueryData(snapshotKey)).toBeUndefined();

    s.server().onFrame({ type: "snapshot", snapshot: sampleSnapshot });
    s.server().onFrame({ type: "snapshot_error", error: "boom" });
    expect(queryClient.getQueryData<Snapshot>(snapshotKey)).toMatchObject({
      error: "boom",
      logs: sampleSnapshot.logs,
    });
    s.server().onFrame({ type: "snapshot", snapshot: sampleSnapshot });
    expect(queryClient.getQueryData<Snapshot>(snapshotKey)?.error).toBeUndefined();
  });

  test("open is idempotent and close shuts the connection", () => {
    const connect = vi.fn(scripted().connect);
    const api = createApi(queryClient, connect);
    api.open();
    api.open();
    expect(connect).toHaveBeenCalledTimes(1);
    api.close();
    expect(queryClient.getQueryData(connectionKey)).toBe("closed");
  });
});

describe("pause", () => {
  const rows = (n: number): Snapshot => ({
    ...sampleSnapshot,
    logs: Array.from({ length: n }, (_, i) => String(i)),
  });
  const shown = () => queryClient.getQueryData<Snapshot>(snapshotKey)?.logs;

  function paused() {
    const s = scripted();
    const api = createApi(queryClient, s.connect);
    api.open();
    s.server().onFrame({ type: "snapshot", snapshot: rows(1) });
    api.pause();
    return { s, api };
  }

  test("rows stay stable while frames arrive and the newest one lands on resume", () => {
    const { s, api } = paused();
    s.server().onFrame({ type: "snapshot", snapshot: rows(2) });
    s.server().onFrame({ type: "snapshot", snapshot: rows(3) });
    expect(queryClient.getQueryData(pausedKey)).toBe(true);
    expect(shown()).toEqual(["0"]);

    api.resume();
    expect(queryClient.getQueryData(pausedKey)).toBe(false);
    expect(shown()).toEqual(["0", "1", "2"]);
    s.server().onFrame({ type: "snapshot", snapshot: rows(4) });
    expect(shown()).toHaveLength(4);
  });

  test("the cache keeps its update time while paused, so the age is the shown snapshot's", () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_000_000);
    const { s, api } = paused();
    const at = queryClient.getQueryState(snapshotKey)?.dataUpdatedAt;
    vi.setSystemTime(1_009_000);
    s.server().onFrame({ type: "snapshot", snapshot: rows(2) });
    expect(queryClient.getQueryState(snapshotKey)?.dataUpdatedAt).toBe(at);
    api.resume();
    expect(queryClient.getQueryState(snapshotKey)?.dataUpdatedAt).toBe(1_009_000);
  });

  test("a tick error that arrives while paused waits for resume, like any other frame", () => {
    const { s, api } = paused();
    s.server().onFrame({ type: "snapshot_error", error: "boom" });
    expect(queryClient.getQueryData<Snapshot>(snapshotKey)?.error).toBeUndefined();
    api.resume();
    expect(queryClient.getQueryData<Snapshot>(snapshotKey)).toMatchObject({ error: "boom" });
  });

  test("there is nothing to freeze before the first snapshot", () => {
    const s = scripted();
    const api = createApi(queryClient, s.connect);
    api.open();
    api.pause();
    expect(queryClient.getQueryData(pausedKey)).toBeUndefined();
    s.server().onFrame({ type: "snapshot", snapshot: rows(1) });
    expect(shown()).toEqual(["0"]);
  });

  test("the socket stays open and server replies still settle while paused", async () => {
    const { s, api } = paused();
    const text = api.expand("abc");
    s.server().onFrame({ type: "expand", id: "abc", text: "full" });
    await expect(text).resolves.toBe("full");
    expect(s.sent).toEqual([{ expand: "abc" }]);
  });
});

describe("set mutation", () => {
  test("sends the key and value; fails when not connected", async () => {
    const s = scripted();
    const api = createApi(queryClient, s.connect);
    await expect(api.set({ key: "plugins.a.enabled", value: true })).rejects.toThrow(
      "not connected",
    );
    api.open();
    const sent = api.set({ key: "plugins.a.enabled", value: true });
    s.server().onFrame({ type: "snapshot", snapshot: sampleSnapshot });
    await sent;
    expect(s.sent).toEqual([{ set: { key: "plugins.a.enabled", value: true } }]);
    s.setOpen(false);
    await expect(api.set({ key: "plugins.a.enabled", value: false })).rejects.toThrow(
      "not connected",
    );
  });

  test("a write waits for the server's answer: the next snapshot settles it, a message fails it", async () => {
    const s = scripted();
    const api = createApi(queryClient, s.connect);
    api.open();
    let settled = false;
    const written = api.project({ action: "select", project: "1" }).then(() => (settled = true));
    await Promise.resolve();
    expect(settled).toBe(false);
    s.server().onFrame({ type: "snapshot", snapshot: sampleSnapshot });
    await written;
    expect(settled).toBe(true);

    const refused = api.set({ key: "proxy.enabled", value: true });
    s.server().onFrame({ type: "message", text: "refused key proxy.enabled" });
    await expect(refused).rejects.toThrow("refused key proxy.enabled");
  });

  test("a write the server never answers fails instead of spinning for ever", async () => {
    vi.useFakeTimers();
    const s = scripted();
    const api = createApi(queryClient, s.connect);
    api.open();
    const silent = expect(api.set({ key: "plugins.a.enabled", value: true })).rejects.toThrow(
      "did not answer",
    );
    await vi.advanceTimersByTimeAsync(WRITE_TIMEOUT_MS);
    await silent;
  });
});

describe("expand mutation", () => {
  let s: ReturnType<typeof scripted>;
  let api: Api;
  beforeEach(() => {
    vi.useFakeTimers();
    s = scripted();
    api = createApi(queryClient, s.connect, 1000);
    api.open();
  });

  test("resolves on the frame with the matching id only", async () => {
    const p = api.expand("a1");
    expect(s.sent).toEqual([{ expand: "a1" }]);
    s.server().onFrame({ type: "expand", id: "other", text: "wrong" });
    s.server().onFrame({ type: "expand", id: "a1", text: "body" });
    await expect(p).resolves.toBe("body");
  });

  test("rejects on a server message", async () => {
    const p = api.expand("nope");
    s.server().onFrame({ type: "message", text: "unknown archive id: nope" });
    await expect(p).rejects.toThrow("unknown archive id: nope");
  });

  test("rejects on timeout, and a late reply is ignored", async () => {
    const p = api.expand("a1");
    const assertion = expect(p).rejects.toThrow("timed out");
    vi.advanceTimersByTime(1000);
    await assertion;
    expect(() => s.server().onFrame({ type: "expand", id: "a1", text: "late" })).not.toThrow();
  });

  test("rejects when the connection drops or was never open", async () => {
    const dropped = api.expand("a1");
    s.server().onState("closed");
    await expect(dropped).rejects.toThrow("connection closed");

    s.setOpen(false);
    await expect(api.expand("a2")).rejects.toThrow("not connected");
  });
});

describe("?sample source", () => {
  test("is selected by the query string", () => {
    expect(isSampleRequested("?sample")).toBe(true);
    expect(isSampleRequested("?x=1&sample=1")).toBe(true);
    expect(isSampleRequested("")).toBe(false);
  });

  test("serves the fixture offline, flips a plugin on set and answers expand", async () => {
    const api = createApi(queryClient, connectSample);
    api.open();
    await Promise.resolve();
    expect(queryClient.getQueryData(connectionKey)).toBe("open");
    expect(queryClient.getQueryData<Snapshot>(snapshotKey)?.plugins.map((p) => p.enabled)).toEqual([
      true,
      false,
      true,
      true,
      true,
      false,
    ]);

    const expanded = api.expand("abc");
    await api.set({ key: "plugins.read.enabled", value: true });
    await expect(expanded).resolves.toContain("abc");
    await Promise.resolve();
    expect(queryClient.getQueryData<Snapshot>(snapshotKey)?.plugins.map((p) => p.enabled)).toEqual([
      true,
      true,
      true,
      true,
      true,
      false,
    ]);
    expect(sampleSnapshot.plugins[1]?.enabled).toBe(false);
  });

  test("refuses a key outside the plugin allowlist", async () => {
    const api = createApi(queryClient, connectSample);
    api.open();
    // The fixture's first snapshot lands on a microtask and would settle the write too early.
    await Promise.resolve();
    await expect(api.set({ key: "proxy.enabled", value: true })).rejects.toThrow(
      "refused key proxy.enabled",
    );
  });
});

describe("doctor requests", () => {
  const selection = { keep: [], toggled: [{ source: "/p", path: "hooks.Stop[0]" }] };
  const plan = { items: [], diff: "", refused: [] };

  test("a plan and an apply each wait for their own frame, in the order asked", async () => {
    const s = scripted();
    const api = createApi(queryClient, s.connect);
    api.open();
    const planned = api.doctorPlan(selection);
    const applied = api.doctorApply(selection);
    expect(s.sent).toEqual([
      { doctor: { action: "plan", selection } },
      { doctor: { action: "apply", selection } },
    ]);
    s.server().onFrame({ type: "doctorfixed", fixed: { text: "done", code: 0 } });
    s.server().onFrame({ type: "doctorplan", plan });
    await expect(planned).resolves.toEqual(plan);
    await expect(applied).resolves.toEqual({ text: "done", code: 0 });
  });

  test("a refusal, a closed link and a missing link fail the request", async () => {
    const s = scripted();
    const api = createApi(queryClient, s.connect);
    api.open();
    const refused = api.doctorPlan(selection);
    s.server().onFrame({ type: "message", text: "doctor needs an action and a selection" });
    await expect(refused).rejects.toThrow("doctor needs");
    const dropped = api.doctorApply(selection);
    s.server().onState("closed");
    await expect(dropped).rejects.toThrow("connection closed");
    s.setOpen(false);
    await expect(api.doctorPlan(selection)).rejects.toThrow("not connected");
  });

  test("the sample machine plans with the terminal defaults and applies the selection", async () => {
    const api = createApi(queryClient, connectSample);
    api.open();
    const first = await api.doctorPlan({ keep: [], toggled: [] });
    expect(first.items.map((i) => [i.shared, i.selected])).toEqual([
      [false, true],
      [true, false],
      [false, true],
    ]);
    expect(first.diff).not.toContain("/work/app");
    const flipped = await api.doctorPlan({
      keep: [],
      toggled: [{ source: "/work/app/.claude/settings.json", path: "hooks.Stop[0].hooks[0]" }],
    });
    expect(flipped.diff).toContain("/work/app");
    const done = await api.doctorApply({ keep: [], toggled: [] });
    expect(done).toMatchObject({ code: 0 });
    expect(done.text).toContain("2 entries removed");
  });
});

describe("drill requests", () => {
  const request = { project: "1", expand: [], focus: null, depth: null, limit: null, query: "" };
  const frame = (project: number) =>
    ({ type: "graph", graph: { project, nodes: [], edges: [] } }) as unknown as Frame;

  test("a frame settles the request of its own project, whatever the order", async () => {
    const s = scripted();
    const api = createApi(queryClient, s.connect);
    api.open();
    const one = api.drill(request);
    const two = api.drill({ ...request, project: "2" });
    expect(s.sent).toHaveLength(2);
    s.server().onFrame(frame(2));
    s.server().onFrame(frame(1));
    await expect(two).resolves.toMatchObject({ project: 2 });
    await expect(one).resolves.toMatchObject({ project: 1 });
  });

  test("two requests for one project are answered in the order asked", async () => {
    const s = scripted();
    const api = createApi(queryClient, s.connect);
    api.open();
    const first = api.drill(request);
    const second = api.drill({ ...request, limit: 1000 });
    s.server().onFrame({ type: "graph", graph: { project: 1, more: 7 } } as unknown as Frame);
    s.server().onFrame({ type: "graph", graph: { project: 1, more: 0 } } as unknown as Frame);
    await expect(first).resolves.toMatchObject({ more: 7 });
    await expect(second).resolves.toMatchObject({ more: 0 });
  });

  test("a refusal fails the request, and so does a missing link", async () => {
    const s = scripted();
    const api = createApi(queryClient, s.connect);
    api.open();
    const refused = api.drill(request);
    s.server().onFrame({ type: "message", text: "unknown project 1" });
    await expect(refused).rejects.toThrow("unknown project");
    s.setOpen(false);
    await expect(api.drill(request)).rejects.toThrow("not connected");
  });

  test("a request nobody answers times out and leaves the queue", async () => {
    vi.useFakeTimers();
    const s = scripted();
    const api = createApi(queryClient, s.connect);
    api.open();
    const lost = expect(api.drill(request)).rejects.toThrow("graph timed out");
    await vi.advanceTimersByTimeAsync(DOCTOR_TIMEOUT_MS);
    await lost;
  });
});

describe("diff requests", () => {
  const request = { project: "1", from: [], to: null, export: null };
  const frame = (project: string, from: string) =>
    ({ type: "diff", project, diff: { from, to: "working", projects: [] } }) as unknown as Frame;

  test("a frame settles the request of its own project", async () => {
    const s = scripted();
    const api = createApi(queryClient, s.connect);
    api.open();
    const one = api.diff(request);
    const two = api.diff({ ...request, project: "2" });
    expect(s.sent[0]).toEqual({ diff: request });
    s.server().onFrame(frame("2", "v2"));
    s.server().onFrame(frame("1", "v1"));
    await expect(two).resolves.toMatchObject({ from: "v2" });
    await expect(one).resolves.toMatchObject({ from: "v1" });
  });

  test("a refusal fails the request, and a drill frame of the same project does not settle it", async () => {
    const s = scripted();
    const api = createApi(queryClient, s.connect);
    api.open();
    const refused = api.diff(request);
    s.server().onFrame({
      type: "graph",
      graph: { project: 1, nodes: [], edges: [] },
    } as unknown as Frame);
    s.server().onFrame({ type: "message", text: "1 is not a git repository" });
    await expect(refused).rejects.toThrow("not a git repository");
  });
});

describe("call stream", () => {
  const batch = (now: number) =>
    ({
      type: "calls",
      calls: { now, running: [], feed: [], heat_window_s: 300, windows: [] },
    }) as Frame;

  test("the first listener subscribes, the last one unsubscribes, a second shares the stream", () => {
    const s = scripted();
    const api = createApi(queryClient, s.connect);
    api.open();
    const seen: number[] = [];
    const stopA = api.calls((b) => seen.push(b.now));
    const stopB = api.calls((b) => seen.push(b.now * 10));
    expect(s.sent).toEqual([{ calls: { subscribe: true } }]);
    s.server().onFrame(batch(3));
    expect(seen).toEqual([3, 30]);
    stopA();
    expect(s.sent).toHaveLength(1);
    stopB();
    expect(s.sent).toEqual([{ calls: { subscribe: true } }, { calls: { subscribe: false } }]);
  });

  test("a new socket subscribes again while a listener is still registered", () => {
    const s = scripted();
    const api = createApi(queryClient, s.connect);
    api.open();
    api.calls(() => {});
    s.server().onState("closed");
    s.server().onState("open");
    expect(s.sent).toEqual([{ calls: { subscribe: true } }, { calls: { subscribe: true } }]);
  });
});
