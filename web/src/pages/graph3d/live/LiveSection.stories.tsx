// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, waitFor, within } from "storybook/test";
import { project } from "../../../api/sampleRows";
import type { Frame, Connect } from "../../../api/ws";
import { richSnapshot } from "../../fixtures";
import { withData } from "../../storyData";
import { batch, end, event } from "./callsFixtures";
import { LiveSection } from "./LiveSection";

const projects = [
    project(1, "rtok", {
        selected: true,
        links: [{ to: 2, kind: "manual", name: "ketch", reason: null }],
    }),
    project(2, "ketch"),
    project(3, "pyrlyn"),
];
const snapshot = { ...richSnapshot, projects };

/** A server that plays `frames` once the page subscribes to the call stream. */
const streaming =
    (frames: Frame[]): Connect =>
    (h) => {
        h.onState("open");
        h.onFrame({ type: "snapshot", snapshot });
        return {
            send: (m) => {
                if ("calls" in m && m.calls.subscribe)
                    setTimeout(() => frames.forEach(h.onFrame), 0);
                return true;
            },
            close: () => {},
        };
    };

const meta = {
    title: "Pages/Live graph",
    component: LiveSection,
    args: { rows: projects, drill: null },
    decorators: [withData(streaming([]))],
} satisfies Meta<typeof LiveSection>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Waiting: Story = {
    play: async ({ canvasElement }) => {
        await waitFor(() =>
            expect(within(canvasElement).getByText("Waiting for graph calls")).toBeTruthy(),
        );
    },
};

const calls = (): Frame[] => [
    {
        type: "calls",
        batch: batch([
            end("a", 1000, 250, { target: "open_index", project: "rtok", backend: "lsp" }),
            end("b", 400, 100, { target: "store", project: "ketch", tool: "impact" }),
            end("c", 0, 0, { target: "missing", project: "rtok", ok: false, error: "no backend" }),
            event({ call: "d", project: "pyrlyn", target: "run" }),
            event({ call: "e", project: "ketch", target: "go", tool: "explore" }),
        ]),
    },
];

export const Busy: Story = {
    decorators: [withData(streaming(calls()))],
    play: async ({ canvasElement }) => {
        const view = within(canvasElement);
        await waitFor(() => expect(view.getByText("no backend")).toBeTruthy());
        expect(view.getByRole("list", { name: "running calls" })).toBeTruthy();
        await waitFor(() =>
            expect(canvasElement.querySelectorAll("[data-testid=accent]").length).toBe(2),
        );
        expect(canvasElement.querySelectorAll("[data-testid=failed]").length).toBe(1);
    },
};
