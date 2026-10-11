// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

// @vitest-environment happy-dom
import { cleanup, fireEvent, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, test } from "vitest";
import { richSnapshot } from "./fixtures";
import { cardsIn, markers } from "./Tasks";
import {
    tasksEmpty,
    tasksFailed,
    tasksFixture,
    tasksReading,
    tasksReady,
    tasksUnconfigured,
} from "./tasksFixtures";
import { mount, serving } from "./testHelpers";

afterEach(() => {
    cleanup();
    localStorage.clear();
});

const withTasks = (tasks: typeof richSnapshot.tasks) => serving({ ...richSnapshot, tasks });
const byId = (id: string) => tasksFixture.find((t) => t.id === id)!;

describe("task helpers", () => {
    test("a plain task has no markers, the others name what they wait on", () => {
        expect(markers(byId("A1.1"))).toEqual([]);
        expect(markers(byId("A3")).map((m) => m.label)).toEqual(["needs A1, A2"]);
        expect(markers(byId("A1.3")).map((m) => m.label)).toEqual(["needs A1.2", "parallel"]);
        expect(markers(byId("A2")).map((m) => m.label)).toEqual(["P1", "conflicts A4"]);
        expect(markers(byId("A1"))[0]).toMatchObject({ label: "P0", tone: "fail" });
    });

    test("a column holds its own status, most urgent first, and the filter narrows it", () => {
        expect(cardsIn(tasksFixture, "open", "").map((t) => t.id)).toEqual([
            "A2",
            "A1.3",
            "A3",
            "A5",
            "A4",
        ]);
        expect(cardsIn(tasksFixture, "open", " DOCS ").map((t) => t.id)).toEqual(["A1.3"]);
        // The parent and the agent are searchable too.
        expect(cardsIn(tasksFixture, "in-progress", "agent-2").map((t) => t.id)).toEqual(["A1.2"]);
        expect(cardsIn(tasksFixture, "closed", "A1")).toEqual([]);
    });
});

describe("tasks page", () => {
    test("one column per status with a card for every task", async () => {
        mount(withTasks(tasksReady()), "/tasks");
        for (const [name, count] of [
            ["open", 5],
            ["in progress", 2],
            ["done", 2],
            ["closed", 1],
        ] as const) {
            const column = await screen.findByRole("region", { name });
            expect(within(column).getAllByRole("listitem")).toHaveLength(count);
        }
        const card = screen.getByText("Export tasks to GitHub issues").closest("li")!;
        expect(card.textContent).toContain("A3");
        expect(card.textContent).toContain("needs A1, A2");
        const link = within(card).getByRole("link");
        expect(link.getAttribute("href")).toBe("https://github.com/acme/app/issues/128");
        expect(link.getAttribute("rel")).toBe("noreferrer noopener");
        expect(screen.getByText("Board page").closest("li")!.textContent).toContain("in A1");
    });

    test("the list view is a table whose rows open the details from the keyboard", async () => {
        mount(withTasks(tasksReady()), "/tasks");
        fireEvent.click(await screen.findByRole("button", { name: "list" }));
        const table = await screen.findByRole("table", { name: "tasks" });
        const rows = within(table).getAllByRole("row").slice(1);
        expect(rows).toHaveLength(tasksFixture.length);
        expect(rows[0]!.getAttribute("tabindex")).toBe("0");

        const target = rows.find((r) => r.textContent?.includes("A3"))!;
        fireEvent.keyDown(target, { key: "Enter" });
        const detail = await screen.findByRole("region", { name: "A3" });
        expect(detail.textContent).toContain("needs");
        expect(detail.textContent).toContain("A1, A2");
        // The choice of view survives a reload.
        expect(localStorage.getItem("rtok-tasks-view")).toBe("list");
    });

    test("a project without a [tasks] section is an empty state, not an error", async () => {
        mount(withTasks(tasksUnconfigured), "/tasks");
        expect(await screen.findByText("No tasks configured")).toBeTruthy();
        expect(screen.queryByRole("alert")).toBeNull();
        expect(screen.queryByRole("button", { name: "list" })).toBeNull();
    });

    test("an empty project, a trimmed board, a read in flight and a failure each say so", async () => {
        const empty = mount(withTasks(tasksEmpty), "/tasks");
        expect(await screen.findByText("No tasks yet")).toBeTruthy();
        empty.unmount();

        const trimmed = mount(withTasks(tasksReady(undefined, 14)), "/tasks");
        expect((await screen.findByText(/14 older done or closed tasks/)).textContent).toContain(
            "rtok task list --all",
        );
        trimmed.unmount();

        const reading = mount(withTasks(tasksReading), "/tasks");
        expect((await screen.findByRole("status")).textContent).toBe("reading tasks…");
        reading.unmount();

        mount(withTasks(tasksFailed), "/tasks");
        expect((await screen.findByRole("alert")).textContent).toContain("HTTP 401");
    });
});
