// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

// @vitest-environment happy-dom
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { drillServer } from "../../../api/sampleDrill";
import { project } from "../../../api/sampleRows";
import type { Snapshot } from "../../../api/snapshot.gen";
import { richSnapshot } from "../../fixtures";
import { mount, wire } from "../../testHelpers";
import { VIEW_KEY } from "../ProjectsOverview";
import { batch, end, event } from "./callsFixtures";
import { HIDE_KEY, SPLIT_KEY } from "./Split";

const snap: Snapshot = {
    ...richSnapshot,
    projects: [project(1, "rtok", { selected: true }), project(2, "ketch")],
};

beforeEach(() => {
    localStorage.clear();
    localStorage.setItem(VIEW_KEY, "list");
});
afterEach(cleanup);

const canvas = () => screen.findByTestId("graph-live");
const calls = (sent: unknown[]) =>
    sent.flatMap((m) =>
        m && typeof m === "object" && "calls" in m
            ? [(m as { calls: { subscribe: boolean } }).calls.subscribe]
            : [],
    );

describe("live graph", () => {
    test("it subscribes, shows the waiting state, then the call in the metrics and the feed", async () => {
        const w = wire(snap);
        mount(w.connect, "/graph");
        expect(await screen.findByText("Waiting for graph calls")).toBeTruthy();
        await waitFor(() => expect(calls(w.sent)).toEqual([true]));

        act(() => w.frame({ type: "calls", batch: batch([event({ call: "a" })]) }));
        await waitFor(() => expect(screen.queryByText("Waiting for graph calls")).toBeNull());
        expect(
            within(screen.getByRole("list", { name: "running calls" })).getByText("callers"),
        ).toBeTruthy();

        act(() => w.frame({ type: "calls", batch: batch([end("a", 1000, 250)]) }));
        const table = await screen.findByRole("table", { name: "graph calls" });
        await waitFor(() => expect(within(table).getByText(/750 saved/)).toBeTruthy());
        expect(screen.getByText("75%")).toBeTruthy();
    });

    test("a failed call is red in the feed", async () => {
        const w = wire(snap);
        mount(w.connect, "/graph");
        await canvas();
        act(() =>
            w.frame({
                type: "calls",
                batch: batch([end("a", 0, 0, { ok: false, error: "no backend" })]),
            }),
        );
        const cell = await screen.findByText("no backend");
        expect(cell.className).toContain("text-delta-fg");
    });

    test("the canvas takes no input: no handlers, nothing focusable, a default cursor", async () => {
        mount(wire(snap).connect, "/graph");
        const svg = await canvas();
        expect(svg.getAttribute("class")).toContain("pointer-events-none");
        expect(svg.querySelectorAll("[tabindex],[role=button]")).toHaveLength(0);
        const before = svg.getAttribute("viewBox");
        fireEvent.wheel(svg, { deltaY: 100 });
        fireEvent.pointerDown(svg, { clientX: 5, clientY: 5 });
        fireEvent.pointerMove(svg, { clientX: 90, clientY: 90 });
        expect(svg.getAttribute("viewBox")).toBe(before);
    });

    test("freeze holds the picture while the totals keep counting, and unfreeze catches up", async () => {
        const w = wire(snap);
        mount(w.connect, "/graph");
        await canvas();
        act(() =>
            w.frame({ type: "calls", batch: batch([end("a", 100, 50, { target: "alpha" })]) }),
        );
        await screen.findByText("alpha");

        fireEvent.click(screen.getByRole("button", { name: "Freeze" }));
        act(() =>
            w.frame({
                type: "calls",
                batch: batch([end("b", 100, 50, { target: "beta" }), end("c", 100, 50)]),
            }),
        );
        expect(await screen.findByText("2 held")).toBeTruthy();
        expect(screen.queryByText("beta")).toBeNull();

        fireEvent.click(screen.getByRole("button", { name: "Unfreeze" }));
        expect(await screen.findByText("beta")).toBeTruthy();
        await waitFor(() => expect(screen.queryByText("2 held")).toBeNull());
        fireEvent.click(screen.getByRole("button", { name: "since open" }));
        // Three calls of 100 tokens each, none dropped by the freeze.
        expect(await screen.findByText("300")).toBeTruthy();
    });

    test("the window selector recomputes the totals", async () => {
        const w = wire(snap);
        mount(w.connect, "/graph");
        await canvas();
        act(() => w.frame({ type: "calls", batch: batch([end("a", 10, 4)]) }));
        await screen.findByRole("table", { name: "graph calls" });
        for (const label of ["1 min", "5 min", "15 min", "since open"]) {
            fireEvent.click(screen.getByRole("button", { name: label }));
            expect(screen.getByRole("button", { name: label }).getAttribute("aria-pressed")).toBe(
                "true",
            );
        }
    });

    test("a 500-call burst is one frame whose totals count every call", async () => {
        const w = wire(snap);
        mount(w.connect, "/graph");
        await canvas();
        const listed = Array.from({ length: 100 }, (_, i) => end(`c${i}`, 10, 4));
        act(() => w.frame({ type: "calls", batch: batch(listed, 400) }));
        fireEvent.click(screen.getByRole("button", { name: "since open" }));
        await waitFor(() => expect(screen.getByText("500")).toBeTruthy());
    });
});

describe("split", () => {
    test("hiding the live graph stops the stream, and the choice is kept", async () => {
        const w = wire(snap);
        mount(w.connect, "/graph");
        await canvas();
        fireEvent.click(screen.getByRole("button", { name: "Hide live graph" }));
        expect(screen.queryByTestId("graph-live")).toBeNull();
        await waitFor(() => expect(calls(w.sent)).toEqual([true, false]));
        expect(localStorage.getItem(HIDE_KEY)).toBe("1");
    });

    test("the bar resizes with the arrow keys, remembers it and resets on double-click", async () => {
        mount(wire(snap).connect, "/graph");
        await canvas();
        const bar = screen.getByRole("separator");
        expect(bar.getAttribute("aria-valuenow")).toBe("50");
        fireEvent.keyDown(bar, { key: "ArrowRight" });
        expect(bar.getAttribute("aria-valuenow")).toBe("55");
        expect(localStorage.getItem(SPLIT_KEY)).toBe("0.55");
        fireEvent.doubleClick(bar);
        expect(bar.getAttribute("aria-valuenow")).toBe("50");
    });

    test("the live graph follows the drilled project", async () => {
        mount(drillServer(snap), "/graph?p=1");
        expect((await canvas()).getAttribute("aria-label")).toContain("symbols of");
    });
});
