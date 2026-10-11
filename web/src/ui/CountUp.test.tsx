// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

// @vitest-environment happy-dom
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { COUNT_MS, CountUp } from "./CountUp";

let now = 0;
let frames: FrameRequestCallback[] = [];

const reduced = (on: boolean) =>
    vi.stubGlobal("matchMedia", (q: string) => ({ matches: on && q.includes("reduce") }));

/** Runs the queued animation frame at `ms` after the start. */
const frame = (ms: number) => {
    now = ms;
    const run = frames;
    frames = [];
    act(() => run.forEach((f) => f(ms)));
};

beforeEach(() => {
    now = 0;
    frames = [];
    reduced(false);
    vi.spyOn(performance, "now").mockImplementation(() => now);
    vi.stubGlobal("requestAnimationFrame", (f: FrameRequestCallback) => frames.push(f));
    vi.stubGlobal("cancelAnimationFrame", () => {
        frames = [];
    });
});
afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
});

const text = (c: HTMLElement) => c.textContent;

describe("CountUp", () => {
    test("draws the first value as is", () => {
        const { container } = render(<CountUp value={40} />);
        expect(text(container)).toBe("40");
        expect(frames).toHaveLength(0);
    });

    test("counts up on a new value and lands on it", () => {
        const { container, rerender } = render(<CountUp value={0} />);
        rerender(<CountUp value={100} />);
        frame(COUNT_MS / 2);
        const mid = Number(container.querySelector("[aria-hidden]")?.textContent);
        expect(mid).toBeGreaterThan(0);
        expect(mid).toBeLessThan(100);
        frame(COUNT_MS);
        expect(text(container)).toBe("100");
        expect(container.querySelector("[aria-hidden]")).toBeNull();
    });

    test("a reader sees the final value only while it moves", () => {
        const { container, rerender } = render(<CountUp value={0} />);
        rerender(<CountUp value={100} />);
        frame(COUNT_MS / 4);
        expect(container.querySelector(".sr-only")?.textContent).toBe("100");
        expect(container.querySelector("[aria-hidden='true']")).not.toBeNull();
    });

    test("jumps under prefers-reduced-motion", () => {
        reduced(true);
        const { container, rerender } = render(<CountUp value={0} />);
        rerender(<CountUp value={100} />);
        expect(text(container)).toBe("100");
        expect(frames).toHaveLength(0);
    });

    test("restarts from where it is when a newer total arrives", () => {
        const { container, rerender } = render(<CountUp value={0} />);
        rerender(<CountUp value={100} />);
        frame(COUNT_MS / 2);
        rerender(<CountUp value={200} />);
        frame(COUNT_MS / 2 + COUNT_MS);
        expect(text(container)).toBe("200");
    });

    test("formats both the moving and the final text", () => {
        const { container, rerender } = render(<CountUp value={0} format={(n) => `${n} t`} />);
        rerender(<CountUp value={10} format={(n) => `${n} t`} />);
        frame(COUNT_MS);
        expect(text(container)).toBe("10 t");
    });
});
