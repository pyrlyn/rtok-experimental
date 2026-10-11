// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import type { Decorator, Meta, StoryObj } from "@storybook/react-vite";
import type { ReactElement } from "react";
import { expect, userEvent, within } from "storybook/test";
import { richSnapshot } from "./fixtures";
import { serve, withData } from "./storyData";
import { Tasks } from "./Tasks";
import {
    tasksEmpty,
    tasksFailed,
    tasksReading,
    tasksReady,
    tasksUnconfigured,
} from "./tasksFixtures";

const answering = (tasks: typeof richSnapshot.tasks) => serve({ ...richSnapshot, tasks });

const meta = { title: "Pages/Tasks" } satisfies Meta;
export default meta;

// The chosen view is remembered, so a story that switches to the list would leak into the next.
const freshView: Decorator = (Story) => {
    try {
        localStorage.removeItem("rtok-tasks-view");
    } catch {
        // No storage: the page falls back to the board anyway.
    }
    return <Story />;
};

const page = (connect = serve(richSnapshot)): StoryObj => ({
    render: (): ReactElement => <Tasks />,
    decorators: [freshView, withData(connect)],
});

export const TasksBoard: StoryObj = {
    ...page(),
    play: async ({ canvasElement }) => {
        const canvas = within(canvasElement);
        for (const column of ["open", "in progress", "done", "closed"]) {
            await expect(await canvas.findByRole("region", { name: column })).toBeVisible();
        }
        await expect(canvas.getByText("Ship the task board")).toBeVisible();
        const link = canvas.getByRole("link", { name: /github #128/ });
        await expect(link).toHaveAttribute("href", "https://github.com/acme/app/issues/128");
        await expect(link).toHaveAttribute("rel", "noreferrer noopener");
    },
};
export const TasksLight: StoryObj = { ...TasksBoard, globals: { theme: "light" } };

/** 375 px: the columns stack and nothing makes the page scroll sideways. */
export const TasksPhone: StoryObj = {
    ...TasksBoard,
    globals: { viewport: { value: "mobile1", isRotated: false } },
    play: async ({ canvasElement }) => {
        await within(canvasElement).findByRole("region", { name: "open" });
        await expect(window.innerWidth).toBeLessThan(768);
        const root = document.documentElement;
        await expect(root.scrollWidth).toBeLessThanOrEqual(root.clientWidth);
    },
};
export const TasksPhoneLight: StoryObj = {
    ...TasksPhone,
    globals: { theme: "light", viewport: { value: "mobile1", isRotated: false } },
};

export const TasksList: StoryObj = {
    ...page(),
    play: async ({ canvasElement }) => {
        const canvas = within(canvasElement);
        await userEvent.click(await canvas.findByRole("button", { name: "list" }));
        await expect(await canvas.findByRole("table", { name: "tasks" })).toBeVisible();
    },
};

export const TasksFiltered: StoryObj = {
    ...page(),
    play: async ({ canvasElement }) => {
        const canvas = within(canvasElement);
        await userEvent.type(await canvas.findByRole("searchbox"), "docs");
        await expect(canvas.getByText("Docs in three languages")).toBeVisible();
        await expect(canvas.queryByText("Pin the lockfile")).toBeNull();
    },
};

export const TasksTrimmed = page(answering(tasksReady(undefined, 14)));
export const TasksNoTasksYet = page(answering(tasksEmpty));
export const TasksNoSection = page(answering(tasksUnconfigured));
export const TasksReading = page(answering(tasksReading));
export const TasksFailed = page(answering(tasksFailed));
