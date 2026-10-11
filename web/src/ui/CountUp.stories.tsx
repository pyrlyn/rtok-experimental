// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import type { Meta, StoryObj } from "@storybook/react-vite";
import { useEffect, useState } from "react";
import { expect, waitFor, within } from "storybook/test";
import { fmt } from "../pages/format";
import { CountUp } from "./CountUp";

/** Mounts on `from`, then receives the server's `to`, as a card does on a new frame. */
function NewFrame({ from, to }: { from: number; to: number }) {
    const [value, setValue] = useState(from);
    useEffect(() => setValue(to), [to]);
    return (
        <p role="status" aria-label="calls" className="text-xl font-semibold">
            <CountUp value={value} format={fmt} />
        </p>
    );
}

const meta = {
    component: NewFrame,
    args: { from: 1200, to: 3456 },
} satisfies Meta<typeof NewFrame>;
export default meta;
type Story = StoryObj<typeof meta>;

export const LandsOnTheServersValue: Story = {
    play: async ({ canvasElement }) => {
        await waitFor(() =>
            expect(within(canvasElement).getByRole("status")).toHaveTextContent("3,456"),
        );
    },
};

export const FromZero: Story = { args: { from: 0, to: 98765 } };
