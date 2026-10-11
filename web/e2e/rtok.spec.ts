// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { existsSync, readFileSync } from "node:fs";
import { basename } from "node:path";
import type { Page } from "@playwright/test";
import { mockMachine } from "../src/api/sampleDoctor";
import type { Selection } from "../src/api/snapshot.gen";
import { PAGES } from "../src/pages";
import { expect, test } from "./fixtures";
import { ARCHIVED_MARKER, type Rtok, STALE_LOGS } from "./rtok";

// `PAGES` mirrors `model::pages()`; `tests/surface_parity.rs` pins the two together, so this
// loop covers every page the server knows.
for (const { id } of PAGES) {
  test(`${id} page renders from the live snapshot`, async ({ page }) => {
    const problems: string[] = [];
    page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
    // A CSP violation or a failed asset load reaches the console as an error.
    page.on("console", (m) => m.type() === "error" && problems.push(`console: ${m.text()}`));

    await page.goto(`/#/${id}`);

    await expect(page.getByRole("heading", { level: 1 })).toHaveText(id);
    await expect(page).toHaveTitle(`${id} · rtok`);
    await expect(
      page.getByRole("navigation", { name: "Admin screens" }).getByRole("link", { name: id }),
    ).toHaveAttribute("aria-current", "page");
    await expect(page.getByRole("banner").getByText("live", { exact: true })).toBeVisible();
    const main = page.getByRole("main");
    await expect(main.locator("[aria-busy]")).toHaveCount(0);
    await expect(main.getByText("Something went wrong")).toHaveCount(0);
    await expect(main.getByText("Offline")).toHaveCount(0);
    expect(problems).toEqual([]);
  });
}

test("pause freezes the header on a visible paused state and resume lifts it", async ({ page }) => {
  await page.goto("/#/overview");
  const header = page.getByRole("banner");
  await expect(header.getByText(/^updated \d+[smhd] ago$/)).toBeVisible();

  const pause = header.getByRole("button", { name: "Pause live updates" });
  await pause.click();
  await expect(pause).toHaveAttribute("aria-pressed", "true");
  await expect(header.getByText("paused", { exact: true })).toBeVisible();

  await pause.click();
  await expect(pause).toHaveAttribute("aria-pressed", "false");
  await expect(header.getByText("paused", { exact: true })).toHaveCount(0);
});

test("plugin toggle round-trips through /ws and the config file", async ({ page, rtok }) => {
  await page.goto("/#/plugins");
  const toon = page.getByRole("switch", { name: "toggle toon" });
  await expect(toon).toHaveAttribute("aria-checked", "true");

  await toon.click();
  // The switch only asks: it moves once the next snapshot from the server says so.
  await expect(toon).toHaveAttribute("aria-checked", "false");
  await expect
    .poll(() => readFileSync(rtok.configPath, "utf8"))
    .toMatch(/\[plugins\.toon\][^[]*enabled\s*=\s*false/);

  await toon.click();
  await expect(toon).toHaveAttribute("aria-checked", "true");
  await expect
    .poll(() => readFileSync(rtok.configPath, "utf8"))
    .toMatch(/\[plugins\.toon\][^[]*enabled\s*=\s*true/);
});

type Rule = { reply?: string; hold?: boolean };

/**
 * Sits between the page and `/ws`. For each request the page sends, `rule` may answer it itself
 * (`reply`) instead of the server and may hold it back (`hold`) until the returned `release()`,
 * so the wait the user sees can be observed.
 */
async function interceptSocket(page: Page, rule: (request: unknown) => Rule | undefined) {
  let release = () => {};
  const gate = new Promise<void>((open) => (release = open));
  await page.routeWebSocket(/\/ws$/, (ws) => {
    const server = ws.connectToServer();
    ws.onMessage((message) => {
      const { reply, hold } = rule(JSON.parse(String(message))) ?? {};
      const deliver = () => (reply === undefined ? server.send(message) : ws.send(reply));
      if (hold) void gate.then(deliver);
      else deliver();
    });
  });
  return release;
}

test("the plugin switch spins in its old position until the server answers", async ({
  page,
  rtok,
}) => {
  const release = await interceptSocket(page, () => ({ hold: true }));
  await page.goto("/#/plugins");
  const toon = page.getByRole("switch", { name: "toggle toon" });
  await expect(toon).toHaveAttribute("aria-checked", "true");

  await toon.click();
  await expect(toon).toHaveAttribute("aria-busy", "true");
  await expect(toon).toBeDisabled();
  await expect(toon).toHaveAttribute("aria-checked", "true");
  // Held, so the spinner is still there and nothing was written yet.
  await page.waitForTimeout(500);
  await expect(toon).toHaveAttribute("aria-busy", "true");
  expect(readFileSync(rtok.configPath, "utf8")).not.toMatch(
    /\[plugins\.toon\][^[]*enabled\s*=\s*false/,
  );

  release();
  await expect(toon).toHaveAttribute("aria-checked", "false");
  await expect(toon).not.toHaveAttribute("aria-busy");
  await expect(toon).toBeEnabled();
});

test("doctor apply keeps its spinner until the answer and drops it after", async ({ page }) => {
  const machine = mockMachine();
  const frame = (type: string, key: string, body: unknown) => JSON.stringify({ type, [key]: body });
  // The e2e store has nothing to fix, so the mocked machine supplies the plan; only the apply
  // is held.
  const release = await interceptSocket(page, (request) => {
    const doctor = (request as { doctor?: { action: string; selection: Selection } }).doctor;
    if (doctor?.action === "plan")
      return { reply: frame("doctorplan", "plan", machine.plan(doctor.selection)) };
    if (doctor?.action === "apply")
      return { reply: frame("doctorfixed", "fixed", machine.apply(doctor.selection)), hold: true };
    return undefined;
  });
  await page.goto("/#/doctor");
  const fix = page.getByRole("region", { name: "fix" });
  await fix.getByRole("button", { name: /^Fix selected/ }).click();
  const confirm = fix.getByRole("button", { name: "Confirm" });
  await confirm.click();
  await expect(confirm).toHaveAttribute("aria-busy", "true");
  await expect(confirm).toBeDisabled();
  await page.waitForTimeout(500);
  await expect(confirm).toHaveAttribute("aria-busy", "true");

  release();
  await expect(fix.getByRole("status")).toContainText("entries");
  await expect(fix.locator("[aria-busy]")).toHaveCount(0);
});

test("clear safe junk plans without deleting and deletes only on the confirmed message", async ({
  page,
  rtok,
}) => {
  const sent: { junk?: { action: string; paths: string[] } }[] = [];
  await page.routeWebSocket(/\/ws$/, (ws) => {
    const server = ws.connectToServer();
    ws.onMessage((message) => {
      sent.push(JSON.parse(String(message)));
      server.send(message);
    });
  });
  const stale = STALE_LOGS.map((name) => rtok.logPath(name));
  await page.goto("/#/hosts");
  const junk = page.getByRole("region", { name: "junk", exact: true });

  await junk.getByRole("button", { name: "Clear safe junk" }).click();
  const plan = junk.getByRole("region", { name: "junk plan" });
  await expect(plan).toContainText("rtok log");
  await expect(plan).toContainText("nothing changed");
  expect(stale.every((p) => existsSync(p))).toBe(true);
  expect(sent.filter((m) => m.junk?.action === "apply")).toHaveLength(0);

  // Cancelling sends nothing and deletes nothing.
  await junk.getByRole("button", { name: "Clear 2 items" }).click();
  await junk.getByRole("button", { name: "Cancel" }).click();
  expect(sent.filter((m) => m.junk?.action === "apply")).toHaveLength(0);
  expect(stale.every((p) => existsSync(p))).toBe(true);

  await junk.getByRole("button", { name: "Clear safe junk" }).click();
  await junk.getByRole("button", { name: "Clear 2 items" }).click();
  await junk.getByRole("button", { name: "Confirm" }).click();
  // The section reloads after the apply, so its loading status can sit beside the result.
  await expect(junk.getByRole("status").filter({ hasText: "Freed" })).toBeVisible();
  expect(stale.some((p) => existsSync(p))).toBe(false);
  const applies = sent.filter((m) => m.junk?.action === "apply");
  expect(applies).toHaveLength(1);
  expect(applies[0]?.junk?.paths.sort()).toEqual(stale.sort());
});

test("expand returns the archived payload of a call", async ({ page }) => {
  await page.goto("/#/calls");
  await page.getByRole("row", { name: /mem_save/ }).click();

  const expand = page.getByRole("button", { name: /^expand [0-9a-f]{64}$/ });
  await expand.click();

  const payload = page.locator("pre");
  await expect(payload).toContainText(ARCHIVED_MARKER);
  await expect(page.getByText(/\d+ lines/)).toBeVisible();
});

test("a filtered and sorted Calls link restores the view after a reload and steps with back", async ({
  page,
}) => {
  await page.goto("/#/calls?surface=mcp&sort=-name");
  const rows = page.getByRole("table", { name: "calls" }).getByRole("row");
  await expect(rows.filter({ hasText: /mem_save/ })).toHaveCount(1);
  await expect(rows.filter({ hasText: /SessionStart/ })).toHaveCount(0);
  await expect(page.getByRole("columnheader", { name: /name/ })).toHaveAttribute(
    "aria-sort",
    "descending",
  );

  await page.reload();
  await expect(page.getByRole("button", { name: /^mcp \d+$/ })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(rows.filter({ hasText: /SessionStart/ })).toHaveCount(0);

  await page.getByRole("button", { name: /^hook \d+$/ }).click();
  await expect(page).toHaveURL(/surface=hook/);
  await expect(rows.filter({ hasText: /mem_save/ })).toHaveCount(0);

  await page.goBack();
  await expect(page).toHaveURL(/surface=mcp/);
  await expect(rows.filter({ hasText: /mem_save/ })).toHaveCount(1);
});

test("a KPI card on the Overview opens its page already filtered", async ({ page }) => {
  await page.goto("/#/overview");
  await page.getByRole("main").getByRole("link", { name: "plugins on" }).click();
  await expect(page).toHaveURL(/#\/plugins\?show=on/);
  await expect(page.getByRole("button", { name: /^enabled/ })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

test("offline takes the whole screen when the server stops; Reconnect brings it back", async ({
  page,
  rtok,
}) => {
  await page.goto("/#/overview");
  const connection = page.getByRole("banner").getByText(/^(live|closed|connecting)$/);
  await expect(connection).toHaveText("live");
  const nav = page.getByRole("navigation", { name: "Admin screens" });
  const offline = page.getByRole("heading", { name: "Offline", exact: true });
  const reconnect = page.getByRole("button", { name: "Reconnect", exact: true });
  await expect(offline).toHaveCount(0);

  await rtok.stop();
  await expect(offline).toBeVisible();
  // Nothing but the offline screen: no navigation, no header, no stale page (T407).
  await expect(nav).toHaveCount(0);
  await expect(connection).toHaveCount(0);
  await expect(reconnect).toBeEnabled();

  await rtok.start();
  // Reconnect skips the client's backoff (capped at 10 s, web/src/api/ws.ts). An automatic retry
  // that lands first hides the button, so the click may never happen; both paths end live.
  void reconnect.click().catch(() => {});
  await expect(connection).toHaveText("live", { timeout: 5_000 });
  await expect(nav).toBeVisible();
  await expect(offline).toHaveCount(0);
});

/** The graph calls the registered home project is asked about, as another process of the store would ask. */
const callers = (rtok: Rtok, times = 1) =>
  rtok.mcp("callers", { name: "no_such_symbol", project: basename(rtok.home) }, times);

/** Every `{"calls":{"subscribe":...}}` the page sends, in order. */
function subscriptions(page: Page) {
  const seen: boolean[] = [];
  page.on("websocket", (ws) =>
    ws.on("framesent", (f) => {
      const m = /"calls":\{"subscribe":(true|false)\}/.exec(String(f.payload));
      if (m) seen.push(m[1] === "true");
    }),
  );
  return seen;
}

/** The live part draws 3D by default where WebGL works; the 2D tests ask for the SVG picture. */
const viewing = (page: Page, view: "2d" | "3d") =>
  page.addInitScript((v) => localStorage.setItem("rtok.graph.view", v), view);

test("a graph call from another process lights its node on the read-only live graph", async ({
  page,
  rtok,
}) => {
  await viewing(page, "2d");
  rtok.addProject();
  await page.goto("/#/graph");
  await expect(page.getByText("Waiting for graph calls")).toBeVisible();
  const live = page.getByTestId("graph-live");
  await expect(live).toHaveCSS("pointer-events", "none");
  await expect(live).toHaveCSS("cursor", "auto");

  // The server replays the stored events a new subscriber missed, but the call is still repeated
  // until the first one lands: the poll that reads it runs after the call, not with it.
  await expect(async () => {
    callers(rtok);
    await expect(live.locator("[data-testid=node-live][data-hot]")).toHaveCount(1, {
      timeout: 1000,
    });
  }).toPass({ timeout: 20_000 });
  await expect(page.getByText("Waiting for graph calls")).toHaveCount(0);
  await expect(page.getByRole("table", { name: "graph calls" })).toBeVisible();
});

test("the live graph ignores the wheel, the pointer and the keyboard", async ({ page, rtok }) => {
  await viewing(page, "2d");
  rtok.addProject();
  await page.goto("/#/graph");
  const live = page.getByTestId("graph-live");
  await expect(live.getByTestId("node-live")).toHaveCount(1);
  const viewBox = () => live.getAttribute("viewBox");
  const before = await viewBox();
  const box = (await live.boundingBox())!;
  const at = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
  await page.mouse.move(at.x, at.y);
  await page.mouse.wheel(0, 400);
  await page.mouse.down();
  await page.mouse.move(at.x + 80, at.y + 60);
  await page.mouse.up();
  await page.keyboard.press("Tab");
  await page.keyboard.press("Enter");
  expect(await viewBox()).toBe(before);
  // The pointer lands on whatever is under the canvas, never on the canvas or its nodes.
  expect(
    await page.evaluate(
      ([x, y]) => document.elementFromPoint(x!, y!)?.closest("svg")?.getAttribute("data-testid"),
      [at.x, at.y],
    ),
  ).not.toBe("graph-live");
});

test("the live camera frames a call from another process and eases back to the overview", async ({
  page,
  rtok,
}) => {
  await viewing(page, "2d");
  rtok.addProject();
  await page.goto("/#/graph");
  const live = page.getByTestId("graph-live");
  await expect(live.getByTestId("node-live")).toHaveCount(1);
  const overview = (await live.getAttribute("viewBox"))!;
  await expect(async () => {
    callers(rtok);
    await expect(live).not.toHaveAttribute("viewBox", overview, { timeout: 1000 });
  }).toPass({ timeout: 20_000 });
  // The call is old after a few seconds and the camera lets go.
  await expect(live).toHaveAttribute("viewBox", overview, { timeout: 15_000 });
});

test("the 3D live canvas takes no input and its camera holds a call from another process", async ({
  page,
  rtok,
}) => {
  await viewing(page, "3d");
  rtok.addProject();
  await page.goto("/#/graph");
  const live = page.getByTestId("graph-live-3d");
  await expect(live.or(page.getByText(/3D unavailable/))).toBeVisible();
  // No WebGL in this browser: the live part says so and draws 2D, which the other tests cover.
  test.skip(
    (await page.getByText(/3D unavailable/).count()) > 0,
    "WebGL is not available in this browser",
  );
  await expect(live.locator("canvas")).toHaveCSS("pointer-events", "none");
  await expect(live).not.toHaveAttribute("data-framed", /.+/);
  await expect(async () => {
    callers(rtok);
    await expect(live).toHaveAttribute("data-framed", /.+/, { timeout: 1000 });
  }).toPass({ timeout: 20_000 });
  await expect(live).not.toHaveAttribute("data-framed", /.+/, { timeout: 15_000 });
});

test("the splitter keeps its place and the hidden state across a reload and stops the stream", async ({
  page,
  rtok,
}) => {
  await viewing(page, "2d");
  rtok.addProject();
  const stream = subscriptions(page);
  await page.goto("/#/graph");
  const bar = page.getByRole("separator", { name: "resize the live graph" });
  await expect(bar).toHaveAttribute("aria-valuenow", "50");
  await expect.poll(() => stream).toEqual([true]);

  await bar.focus();
  await page.keyboard.press("ArrowRight");
  await expect(bar).toHaveAttribute("aria-valuenow", "55");
  const handle = (await bar.boundingBox())!;
  await page.mouse.move(handle.x + handle.width / 2, handle.y + handle.height / 2);
  await page.mouse.down();
  await page.mouse.move(handle.x - 150, handle.y + handle.height / 2, { steps: 5 });
  await page.mouse.up();
  const dragged = Number(await bar.getAttribute("aria-valuenow"));
  expect(dragged).toBeLessThan(55);

  await page.reload();
  await expect(bar).toHaveAttribute("aria-valuenow", String(dragged));
  await bar.dblclick();
  await expect(bar).toHaveAttribute("aria-valuenow", "50");

  await page.getByRole("button", { name: "Hide live graph" }).click();
  await expect(page.getByTestId("graph-live")).toHaveCount(0);
  await expect.poll(() => stream.at(-1)).toBe(false);
  const sent = stream.length;
  await page.reload();
  await expect(page.getByRole("button", { name: "Show live graph" })).toBeVisible();
  await expect(page.getByRole("banner").getByText("live", { exact: true })).toBeVisible();
  expect(stream).toHaveLength(sent);
});

test("under 900 px the two parts stack and there is no splitter", async ({ page, rtok }) => {
  await viewing(page, "2d");
  rtok.addProject();
  await page.setViewportSize({ width: 800, height: 900 });
  await page.goto("/#/graph");
  await expect(page.getByTestId("graph-live")).toBeVisible();
  await expect(page.getByRole("separator")).toBeHidden();
});

test("each part of the graph page maximises to fill the page and Escape restores", async ({
  page,
  rtok,
}) => {
  await viewing(page, "2d");
  rtok.addProject();
  await page.goto("/#/graph");
  const live = page.getByTestId("graph-live");
  await expect(live).toBeVisible();

  await page.getByRole("button", { name: "Maximise live graph" }).click();
  // The section, not the picture inside it: the section pads the picture on its own.
  const fill = await page.getByRole("region", { name: "live graph" }).boundingBox();
  const view = page.viewportSize();
  expect(fill?.width).toBeGreaterThan((view?.width ?? 0) - 40);
  await expect(live).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("button", { name: "Maximise live graph" })).toBeVisible();

  await page.getByRole("button", { name: "Maximise explorer" }).click();
  await expect(live).toHaveCount(0);
  await page.getByRole("button", { name: "Restore explorer" }).click();
  await expect(live).toBeVisible();
});

test("a 500-call burst leaves the page responsive", async ({ page, rtok }) => {
  await viewing(page, "2d");
  rtok.addProject();
  await page.goto("/#/graph");
  const live = page.getByTestId("graph-live");
  await expect(live).toBeVisible();
  await expect(async () => {
    callers(rtok);
    await expect(page.getByRole("table", { name: "graph calls" })).toBeVisible({ timeout: 1000 });
  }).toPass({ timeout: 20_000 });

  callers(rtok, 500);
  const hide = page.getByRole("button", { name: "Hide live graph" });
  await expect(hide).toBeEnabled();
  await hide.click({ timeout: 2000 });
  await expect(page.getByRole("button", { name: "Show live graph" })).toBeVisible({
    timeout: 2000,
  });
});

test("Compare colours a changed, an added and a removed function and lists them, live graph running", async ({
  page,
  rtok,
}) => {
  await viewing(page, "2d");
  const id = rtok.addGitProject(
    "lib.rs",
    'fn kept() {}\nfn gone() {\n    println!("one");\n    println!("two");\n}\n',
  );
  rtok.editProject("lib.rs", "fn kept(x: i32) {}\nfn fresh() {\n    loop {}\n}\n");
  rtok.indexProject(id);
  const files = encodeURIComponent(JSON.stringify(["lib.rs"]));
  await page.goto(`/#/graph?p=${id}&x=${files}`);
  await page.getByRole("button", { name: "Compare" }).click();

  const side = page.getByRole("region", { name: "compare" });
  await expect(side.getByText("~ 1 changed")).toBeVisible();
  await expect(side.getByText("+ 1 added")).toBeVisible();
  await expect(side.getByText("− 1 removed")).toBeVisible();
  const graph = page.getByTestId("graph-2d");
  // The colour is a brand role and the label says the same in words, so both are checked.
  const node = (label: string) => graph.locator("[data-testid=node-2d]", { hasText: label });
  await expect(graph.getByLabel(/^~ kept, function · changed/)).toBeVisible();
  await expect(graph.getByLabel(/^\+ fresh, function · added/)).toBeVisible();
  await expect(graph.getByLabel(/^− gone, function · removed/)).toBeVisible();
  await expect(node("~ kept").locator("circle").first()).toHaveCSS("fill", /rgb/);
  await expect(side.getByRole("region", { name: "changed" })).toContainText("signature");

  // Compare leaves part 2 alone: the read-only live graph is still there.
  await expect(page.getByTestId("graph-live")).toBeVisible();
});

test("Compare against a saved export sends its text and lists the symbols that appeared", async ({
  page,
  rtok,
}) => {
  await viewing(page, "2d");
  const id = rtok.addGitProject("lib.rs", "fn kept() {}\n");
  rtok.indexProject(id);
  const saved = rtok.exportSymbols(id);
  rtok.editProject("lib.rs", "fn kept() {}\nfn fresh() {}\n");
  rtok.indexProject(id);
  await page.goto(`/#/graph?p=${id}`);
  await page.getByRole("button", { name: "Compare" }).click();
  await page.getByLabel("saved export").setInputFiles({
    name: "before.json",
    mimeType: "application/json",
    buffer: Buffer.from(saved),
  });
  const side = page.getByRole("region", { name: "compare" });
  await expect(side.getByText(/vs export before\.json/)).toBeVisible();
  await expect(side.getByText("+ 1 added")).toBeVisible();
  await expect(side.getByText(/only added and removed symbols/)).toBeVisible();
});

test("Export downloads the JSON the CLI writes, and the page opens it back read-only", async ({
  page,
  rtok,
}) => {
  await viewing(page, "2d");
  const id = rtok.addGitProject("lib.rs", "fn kept() {}\nfn caller() {\n    kept();\n}\n");
  rtok.indexProject(id);
  await page.goto(`/#/graph?p=${id}`);
  await page.getByRole("button", { name: "Export", exact: true }).click();
  const options = page.getByRole("group", { name: "export options" });
  await expect(options.getByText(/File names and symbol names are included/)).toBeVisible();
  await options.getByRole("radio", { name: /Symbol graph of/ }).check();
  const [download] = await Promise.all([
    page.waitForEvent("download"),
    options.getByRole("button", { name: "Download" }).click(),
  ]);
  const text = readFileSync(await download.path(), "utf8");

  // Equal to the byte but for the two times, which move with each run.
  const stable = (s: string) => s.replace(/\s*"(exported_at|indexed_at)": \d+,?/g, "");
  expect(stable(text)).toBe(stable(rtok.exportSymbols(id)));
  expect(download.suggestedFilename()).toBe("rtok-graph-proj-symbols.json");

  await page.getByLabel("open an export").setInputFiles({
    name: "mine.json",
    mimeType: "application/json",
    buffer: Buffer.from(text),
  });
  await expect(page.getByText("viewing export from mine.json")).toBeVisible();
  const symbols = page.getByRole("region", { name: "symbols", exact: true });
  await expect(symbols).toContainText("caller");
  await expect(symbols).toContainText("kept");
  // Read-only: the live pictures give way, and only closing the file is offered.
  await expect(page.getByTestId("graph-live")).toHaveCount(0);
  await page.getByRole("button", { name: "Close export" }).click();
  await expect(page.getByText(/viewing export from/)).toHaveCount(0);
});

test("a project with tasks shows them on the board, a subtask under its parent, and writes nothing", async ({
  page,
  rtok,
}) => {
  // The server reads the project again on its own schedule, so the page is opened after the
  // tasks exist and given a few refreshes to catch up.
  const [parent, child, finished] = rtok.addTasks(["Ship it", "Write the docs", "Cut the release"]);
  await page.goto("/#/tasks");

  const open = page.getByRole("region", { name: "open" });
  await expect(open.getByText("Ship it")).toBeVisible({ timeout: 20_000 });
  await expect(open.getByText("Write the docs")).toBeVisible();
  await expect(open.getByText(`in ${parent}`)).toBeVisible();
  await expect(
    page.getByRole("region", { name: "done" }).getByText("Cut the release"),
  ).toBeVisible();
  expect([parent, child, finished]).toEqual(["E1", "E1.1", "E2"]);

  await page.getByRole("button", { name: "list" }).click();
  const row = page
    .getByRole("table", { name: "tasks" })
    .getByRole("row", { name: /Write the docs/ });
  await row.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("region", { name: child! })).toBeVisible();
  // Read-only: nothing on the page is a form control that could change a task.
  await expect(page.getByRole("main").getByRole("switch")).toHaveCount(0);
});
