// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test } from "vitest";
import { Split } from "./Split";

afterEach(cleanup);

const mount = () =>
    render(
        <Split explorer={<p>explorer body</p>} live={<p data-testid="graph-live">live body</p>} />,
    );

describe("maximise", () => {
    test("the explorer fills the page, drops the live part and restores", () => {
        mount();
        fireEvent.click(screen.getByRole("button", { name: "Maximise explorer" }));
        expect(screen.queryByTestId("graph-live")).toBeNull();
        expect(screen.queryByRole("separator")).toBeNull();
        expect(screen.getByText("explorer body").parentElement?.className).toContain("fixed");
        fireEvent.click(screen.getByRole("button", { name: "Restore explorer" }));
        expect(screen.getByTestId("graph-live")).toBeTruthy();
        expect(screen.getByRole("separator")).toBeTruthy();
    });

    test("the live part fills the page, the explorer stays mounted but hidden, and Escape restores", () => {
        mount();
        fireEvent.click(screen.getByRole("button", { name: "Maximise live graph" }));
        expect(screen.getByText("live body").parentElement?.className).toContain("fixed");
        expect(screen.getByText("explorer body").parentElement?.className).toContain("hidden");
        fireEvent.keyDown(window, { key: "Escape" });
        expect(screen.getByRole("button", { name: "Maximise live graph" })).toBeTruthy();
        expect(screen.getByText("explorer body").parentElement?.className).not.toContain("fixed");
    });

    test("Escape already used by something else does not restore", () => {
        mount();
        fireEvent.click(screen.getByRole("button", { name: "Maximise explorer" }));
        const e = new KeyboardEvent("keydown", { key: "Escape", cancelable: true });
        e.preventDefault();
        window.dispatchEvent(e);
        expect(screen.getByRole("button", { name: "Restore explorer" })).toBeTruthy();
    });

    test("nothing is remembered across a remount", () => {
        const first = mount();
        fireEvent.click(screen.getByRole("button", { name: "Maximise live graph" }));
        first.unmount();
        mount();
        expect(screen.getByRole("button", { name: "Maximise live graph" })).toBeTruthy();
    });
});
