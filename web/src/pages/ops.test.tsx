// @vitest-environment happy-dom
import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, test } from "vitest";
import { sampleSnapshot } from "../api/sample";
import { richSnapshot } from "./fixtures";
import { matchesLog, matchesSession, parseLog } from "./model";
import { mount, serving } from "./testHelpers";

afterEach(cleanup);

const rowsOf = (table: HTMLElement) => within(table).getAllByRole("row").slice(1);

describe("ops page logic", () => {
    test("a log line splits into its parts and a foreign line stays whole", () => {
        expect(parseLog("2026-10-02 12:03:20 ERROR hook/PreToolUse: shell panicked: x", 3)).toEqual(
            {
                i: 3,
                raw: "2026-10-02 12:03:20 ERROR hook/PreToolUse: shell panicked: x",
                ts: "2026-10-02 12:03:20",
                level: "error",
                source: "hook",
                name: "PreToolUse",
                msg: "shell panicked: x",
            },
        );
        expect(parseLog("rtok hook ok", 0)).toMatchObject({
            level: "info",
            ts: "",
            msg: "rtok hook ok",
        });
    });

    test("log and session filters", () => {
        const warn = parseLog("2026-10-02 12:02:40 WARN proxy/messages: slow", 1);
        expect([matchesLog(warn, "warn", ""), matchesLog(warn, "error", "")]).toEqual([
            true,
            false,
        ]);
        expect(matchesLog(warn, "all", "SLOW")).toBe(true);
        const [live, , ended] = sampleSnapshot.sessions;
        expect([matchesSession(live!, true, ""), matchesSession(ended!, true, "")]).toEqual([
            true,
            false,
        ]);
        expect(matchesSession(ended!, false, "RTOK")).toBe(true);
        expect(matchesSession(ended!, false, "ketch")).toBe(false);
    });
});

describe("sessions", () => {
    test("lists newest first, filters to live and shows the session's calls", async () => {
        mount(serving(sampleSnapshot), "/sessions");
        const table = await screen.findByRole("table", { name: "sessions" });
        expect(rowsOf(table)).toHaveLength(3);
        fireEvent.click(screen.getByRole("switch", { name: "live only" }));
        // The filter lives in the URL, so the rows follow the router, not the click.
        await waitFor(() => expect(rowsOf(table)).toHaveLength(2));
        fireEvent.click(rowsOf(table)[1]!);
        const detail = screen.getByRole("region", { name: "detail" });
        expect(within(detail).getByText("claude-sonnet-5-5")).toBeTruthy();
        expect(within(detail).getByText("calls 14")).toBeTruthy();
        expect(within(detail).getByRole("img", { name: /^input \d+%/ })).toBeTruthy();
    });

    test("says so when there are no sessions", async () => {
        mount(serving({ ...sampleSnapshot, sessions: [] }), "/sessions");
        expect(await screen.findByText("No sessions yet")).toBeTruthy();
    });

    test("a session with no calls in the frame says so", async () => {
        mount(serving(richSnapshot), "/sessions");
        const table = await screen.findByRole("table", { name: "sessions" });
        fireEvent.click(rowsOf(table)[0]!);
        expect(screen.getByText("No calls from this session in the current frame.")).toBeTruthy();
    });
});

describe("doctor", () => {
    test("counts the checks and lists every report section", async () => {
        mount(serving(sampleSnapshot), "/doctor");
        // The check pills reuse the words "pass" and "warn", so pick the KPI label by its style.
        const count = (label: string) =>
            screen.getAllByText(label).find((e) => e.className.includes("uppercase"))
                ?.nextElementSibling?.textContent;
        await screen.findByRole("region", { name: "checks" });
        expect([count("pass"), count("warn"), count("fail"), count("not set")]).toEqual([
            "4",
            "3",
            "0",
            "2",
        ]);
        const proxy = screen.getByRole("region", { name: "proxy chains" });
        expect(within(proxy).getByText("rtok proxy")).toBeTruthy();
        expect(screen.getByRole("region", { name: "MCP servers" }).textContent).toContain(
            "9 tools",
        );
        const agents = screen.getByRole("region", { name: "agents × modules" });
        expect(within(agents).getByText("off")).toBeTruthy();
        expect(within(agents).getByText("rtok agent setup --proxy")).toBeTruthy();
        expect(screen.getByRole("region", { name: "instructions" }).textContent).toContain(
            "5,200 tok",
        );
    });

    test("the project panel gives install commands and claims savings only from measurements", async () => {
        mount(serving(sampleSnapshot), "/doctor");
        const project = await screen.findByRole("region", { name: "project" });
        const langs = within(project).getByRole("region", { name: "languages" });
        expect(within(langs).getByText("rust-analyzer installed")).toBeTruthy();
        expect(within(langs).getByText("typescript-language-server missing")).toBeTruthy();
        expect(langs.textContent).toContain(
            "install: npm install -g typescript-language-server typescript",
        );
        expect(langs.textContent).toContain("here ≈ 43,700 tok (38 calls in 30 days)");
        expect(langs.textContent).toContain("tags only");
        const graph = within(project).getByRole("region", { name: "graph" });
        expect(graph.textContent).toContain("72/100");
        expect(graph.textContent).toContain("fix: rtok graph index /work/app");
        const plugins = within(project).getByRole("region", { name: "plugins off here" });
        const [compress, proxy] = within(plugins).getAllByRole("listitem");
        expect(compress?.textContent).toContain("≈ 610 tok per call over 52 calls");
        expect(compress?.textContent).not.toContain("here");
        expect(proxy?.textContent).toContain("in user config · saving: no measurements yet");
        expect(proxy?.textContent).not.toMatch(/\d/);
    });

    test("without a checkout there is no project panel", async () => {
        mount(
            serving({ ...sampleSnapshot, doctor: { ...sampleSnapshot.doctor!, project: null } }),
            "/doctor",
        );
        await screen.findByRole("region", { name: "checks" });
        expect(screen.queryByRole("region", { name: "project" })).toBeNull();
    });

    test("a failed probe shows the failure and no sections", async () => {
        mount(serving({ ...sampleSnapshot, doctor: null }), "/doctor");
        const checks = await screen.findByRole("region", { name: "checks" });
        expect(within(checks).getByText("doctor probe")).toBeTruthy();
        expect(screen.queryByRole("region", { name: "hooks" })).toBeNull();
    });

    test("before the first probe lands the page says doctor is running", async () => {
        mount(serving({ ...sampleSnapshot, doctor: null, doctor_pending: true }), "/doctor");
        const checks = await screen.findByRole("region", { name: "checks" });
        expect(within(checks).getByText("running doctor…")).toBeTruthy();
        expect(within(checks).queryByText(/did not answer/)).toBeNull();
    });
});

describe("logs", () => {
    test("shows lines newest first, filters by level and by text", async () => {
        mount(serving(sampleSnapshot), "/logs");
        const list = await screen.findByRole("list", { name: "log lines" });
        expect(within(list).getAllByRole("listitem")).toHaveLength(4);
        fireEvent.click(screen.getByRole("button", { name: "error 1" }));
        await waitFor(() => expect(within(list).getAllByRole("listitem")).toHaveLength(1));
        expect(within(list).getByRole("listitem").textContent).toContain("index not built");
        fireEvent.click(screen.getByRole("button", { name: "all 4" }));
        fireEvent.change(screen.getByRole("searchbox", { name: "Filter log lines" }), {
            target: { value: "zzz" },
        });
        expect(await screen.findByText("No line matches")).toBeTruthy();
    });

    test("says so when there are no lines", async () => {
        mount(serving({ ...sampleSnapshot, logs: [] }), "/logs");
        expect(await screen.findByText("No logs yet")).toBeTruthy();
    });
});

describe("doctor fix", () => {
    test("select, diff and confirm go through the mocked machine", async () => {
        const { connectSample } = await import("../api/sample");
        mount(connectSample, "/doctor");
        const panel = within(await screen.findByRole("region", { name: "fix" }));
        const project = await panel.findByRole("checkbox", {
            name: /stop\.sh in \/work\/app\/\.claude/,
        });
        expect((project as HTMLInputElement).checked).toBe(false);
        expect(panel.getByLabelText("diff").textContent).not.toContain("/work/app");

        fireEvent.click(project);
        await within(await screen.findByRole("region", { name: "fix" })).findByText(
            /Fix selected \(3\)/,
        );
        expect(panel.getByLabelText("diff").textContent).toContain("/work/app");

        fireEvent.click(panel.getByText(/Fix selected \(3\)/));
        expect(panel.getByText("Write 3 entries?")).toBeTruthy();
        fireEvent.click(panel.getByText("Cancel"));
        expect(panel.queryByText("Write 3 entries?")).toBeNull();

        fireEvent.click(panel.getByText(/Fix selected \(3\)/));
        fireEvent.click(panel.getByText("Confirm"));
        expect((await panel.findByRole("status")).textContent).toContain("3 entries removed");
    });
});
