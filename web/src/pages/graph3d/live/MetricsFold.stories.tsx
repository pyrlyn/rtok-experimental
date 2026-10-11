// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { page } from "vitest/browser";
import { MetricsFold } from "./MetricsFold";

const meta = {
    title: "Pages/Live graph/Metrics fold",
    component: MetricsFold,
    args: {
        strip: (
            <>
                <span>2 running</span>
                <span>14 calls</span>
                <span>3.1k saved</span>
            </>
        ),
        children: <p>full cards</p>,
    },
} satisfies Meta<typeof MetricsFold>;
export default meta;
type Story = StoryObj<typeof meta>;

/** The toggle is a real button: the keyboard opens and closes the cards. */
export const Folded: Story = {
    play: async ({ canvasElement }) => {
        // The strip exists only below 900 px.
        await page.viewport(375, 800);
        const toggle = within(canvasElement).getByRole("button", {
            name: /2 running/,
        });
        expect(toggle.getAttribute("aria-expanded")).toBe("false");
        toggle.focus();
        await userEvent.keyboard("{Enter}");
        expect(toggle.getAttribute("aria-expanded")).toBe("true");
        await userEvent.keyboard(" ");
        expect(toggle.getAttribute("aria-expanded")).toBe("false");
    },
};
