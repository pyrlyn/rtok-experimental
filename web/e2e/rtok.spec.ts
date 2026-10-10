// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { readFileSync } from "node:fs";
import type { Page } from "@playwright/test";
import { mockMachine } from "../src/api/sampleDoctor";
import type { Selection } from "../src/api/snapshot.gen";
import { PAGES } from "../src/pages";
import { expect, test } from "./fixtures";
import { ARCHIVED_MARKER } from "./rtok";

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

test("a graph call from another process reaches the read-only live graph", async ({
  page,
  rtok,
}) => {
  rtok.addProject();
  await page.goto("/#/graph");
  await expect(page.getByText("Waiting for graph calls")).toBeVisible();
  const canvas = page.getByTestId("graph-live");
  await expect(canvas).toHaveCSS("pointer-events", "none");

  // The stream has no replay, so a call made before the server saw the subscription is lost:
  // the call is repeated until the first one lands.
  const row = page.getByRole("table", { name: "graph calls" }).getByText("no_such_symbol").first();
  await expect(async () => {
    rtok.mcp("callers", { name: "no_such_symbol" });
    await expect(row).toBeVisible({ timeout: 2000 });
  }).toPass({ timeout: 20_000 });
  await expect(page.getByText("Waiting for graph calls")).toHaveCount(0);
});
