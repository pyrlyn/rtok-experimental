// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

// @vitest-environment happy-dom
import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, test } from "vitest";
import { connectSample } from "../api/sample";
import { project } from "../api/sampleRows";
import type { ProjectRow, Snapshot } from "../api/snapshot.gen";
import { richSnapshot } from "./fixtures";
import { mount, serving, wire } from "./testHelpers";

afterEach(cleanup);

const withProjects = (projects: ProjectRow[] | null): Snapshot => ({ ...richSnapshot, projects });
const current = () => within(screen.getByLabelText("current project"));

describe("graph page projects", () => {
    test("the selector lists projects and the header shows the selected one", async () => {
        mount(connectSample, "/graph");
        const list = await screen.findByRole("list", { name: "projects" });
        expect(
            within(list)
                .getAllByRole("button")
                .map((b) => b.textContent),
        ).toEqual(["rtokokmanual", "ketchstalemanual"]);
        expect(current().getByText("rtok")).toBeTruthy();
        expect(screen.queryByLabelText("find a project")).toBeNull();
    });

    test("every index state has its own indicator", async () => {
        const states = ["ok", "not indexed", "indexing", "stale", "failed", "missing"];
        const rows = states.map((state, i) =>
            project(i + 1, `p${i}`, { state, selected: i === 0, missing: state === "missing" }),
        );
        for (const [i, state] of states.entries()) {
            const sel = rows.map((p, j) => ({ ...p, selected: i === j }));
            const view = mount(serving(withProjects(sel)), "/graph");
            await screen.findByLabelText("current project");
            expect(current().getByText(state)).toBeTruthy();
            view.unmount();
        }
    });

    test("a missing project is shown but cannot be selected", async () => {
        const rows = [
            project(1, "a", { selected: true }),
            project(2, "b", { state: "missing", missing: true }),
        ];
        mount(serving(withProjects(rows)), "/graph");
        const b = await screen.findByRole("button", { name: /^b/ });
        expect((b as HTMLButtonElement).disabled).toBe(true);
    });

    test("above ten projects the selector searches", async () => {
        const rows = Array.from({ length: 12 }, (_, i) =>
            project(i + 1, `proj${i}`, { selected: i === 0 }),
        );
        mount(serving(withProjects(rows)), "/graph");
        fireEvent.change(await screen.findByLabelText("find a project"), {
            target: { value: "proj11" },
        });
        const list = screen.getByRole("list", { name: "projects" });
        expect(within(list).getAllByRole("button")).toHaveLength(1);
    });

    test("selecting asks the server and the next snapshot moves the selection (second tab)", async () => {
        const rows = [project(1, "a", { selected: true }), project(2, "b")];
        const w = wire(withProjects(rows));
        mount(w.connect, "/graph");
        fireEvent.click(await screen.findByRole("button", { name: /^b/ }));
        await waitFor(() => expect(w.sent).toHaveLength(1));
        expect(w.sent).toEqual([{ project: { action: "select", project: "2" } }]);
        // The button waits for the answer: busy and not clickable until the snapshot arrives.
        // Scoped to the selector: other panels can also show a button named after project b.
        const list = within(screen.getByRole("list", { name: "projects" }));
        const asked = list.getByRole("button", { name: /^b/ }) as HTMLButtonElement;
        expect(asked.getAttribute("aria-busy")).toBe("true");
        expect(asked.disabled).toBe(true);
        expect(current().getByText("a")).toBeTruthy();
        w.push(withProjects(rows.map((p) => ({ ...p, selected: p.id === 2 }))));
        expect(
            await within(await screen.findByLabelText("current project")).findByText("b"),
        ).toBeTruthy();
    });

    test("selecting against the sample server moves the header", async () => {
        mount(connectSample, "/graph");
        fireEvent.click(await screen.findByRole("button", { name: /^ketch/ }));
        expect(await current().findByText("ketch")).toBeTruthy();
        expect(current().queryByText("rtok")).toBeNull();
    });

    test("a refusal from the server shows under the selector", async () => {
        const w = wire(withProjects([project(1, "a", { selected: true }), project(9, "gone")]));
        mount(w.connect, "/graph");
        const button = await screen.findByRole("button", { name: /^gone/ });
        fireEvent.click(button);
        await waitFor(() => expect(w.sent).toHaveLength(1));
        w.message("project 9 is gone");
        expect(await screen.findByText("project 9 is gone")).toBeTruthy();
        expect((button as HTMLButtonElement).disabled).toBe(false);
        expect(button.getAttribute("aria-busy")).toBeNull();
    });

    test("link, link both ways and unlink send their requests", async () => {
        const rows = [
            project(1, "a", {
                selected: true,
                links: [{ kind: "auto", name: "c", reason: "path dep", to: 3 }],
            }),
            project(2, "b"),
            project(3, "c"),
        ];
        const w = wire(withProjects(rows));
        mount(w.connect, "/graph");
        const links = within(await screen.findByRole("region", { name: "links" }));
        expect(links.getByText("path dep")).toBeTruthy();
        expect(links.queryByRole("option", { name: "c" })).toBeNull();
        fireEvent.change(links.getByLabelText("link to"), { target: { value: "2" } });
        fireEvent.click(links.getByLabelText("both ways"));
        fireEvent.click(links.getByRole("button", { name: "link" }));
        fireEvent.click(links.getByRole("button", { name: "unlink c" }));
        // The live graph subscribes to the call stream on its own; this test is about the link requests.
        const writes = () => w.sent.filter((m) => !("calls" in m));
        await waitFor(() => expect(writes()).toHaveLength(2));
        expect(writes()).toEqual([
            { project: { action: "link", from: "1", to: "2", both: true } },
            { project: { action: "unlink", from: "1", to: "3", both: false } },
        ]);
    });

    test("linking against the sample server shows the link", async () => {
        mount(connectSample, "/graph");
        const links = within(await screen.findByRole("region", { name: "links" }));
        fireEvent.click(links.getByRole("button", { name: "unlink ketch" }));
        expect(await links.findByText(/is not linked to another project/)).toBeTruthy();
        fireEvent.change(links.getByLabelText("link to"), { target: { value: "2" } });
        fireEvent.click(links.getByRole("button", { name: "link" }));
        expect(await links.findByRole("button", { name: "unlink ketch" })).toBeTruthy();
    });

    test("no registry degrades to the plain graph page; an empty one says how to add", async () => {
        mount(serving(withProjects(null)), "/graph");
        expect(await screen.findByText("pending files")).toBeTruthy();
        expect(screen.queryByRole("list", { name: "projects" })).toBeNull();
        cleanup();
        mount(serving(withProjects([])), "/graph");
        expect(await screen.findByText("No projects")).toBeTruthy();
    });
});
