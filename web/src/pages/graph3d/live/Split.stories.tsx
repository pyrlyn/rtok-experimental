// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { Split } from "./Split";

const meta = {
    title: "graph/Split",
    component: Split,
    args: {
        explorer: <p>explorer body</p>,
        live: <p>live body</p>,
    },
} satisfies Meta<typeof Split>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Maximised: Story = {
    play: async ({ canvasElement }) => {
        const c = within(canvasElement);
        await userEvent.click(c.getByRole("button", { name: "Maximise live graph" }));
        await expect(c.getByRole("button", { name: "Restore live graph" })).toBeTruthy();
        await expect(c.queryByRole("separator")).toBeNull();
        await userEvent.keyboard("{Escape}");
        await expect(c.getByRole("button", { name: "Maximise live graph" })).toBeTruthy();
    },
};
