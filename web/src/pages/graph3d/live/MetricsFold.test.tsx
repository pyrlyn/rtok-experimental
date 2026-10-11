// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

afterEach(cleanup);
import { MetricsFold } from "./MetricsFold";

describe("MetricsFold", () => {
    it("starts folded and toggles aria-expanded and the hiding class", () => {
        render(
            <MetricsFold strip={<span>2 running</span>}>
                <p>cards</p>
            </MetricsFold>,
        );
        const toggle = screen.getByRole("button", { name: /2 running/ });
        const body = screen.getByText("cards").parentElement!;
        expect(toggle.getAttribute("aria-expanded")).toBe("false");
        expect(toggle.getAttribute("aria-controls")).toBe(body.id);
        expect(body.className).toContain("max-[899px]:hidden");
        fireEvent.click(toggle);
        expect(toggle.getAttribute("aria-expanded")).toBe("true");
        expect(body.className).not.toContain("max-[899px]:hidden");
        fireEvent.click(toggle);
        expect(toggle.getAttribute("aria-expanded")).toBe("false");
    });

    it("hides the strip from 900 px up so the wide layout is unchanged", () => {
        render(<MetricsFold strip="x">y</MetricsFold>);
        expect(screen.getByRole("button").className).toContain("min-[900px]:hidden");
    });
});
