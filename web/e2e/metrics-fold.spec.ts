// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { expect, test } from "./fixtures";

// The metrics wait for the first call frame, so the page is opened and the card is looked for by
// its label; 2D keeps the WebGL canvas out of a headless run.
test.beforeEach(async ({ page, rtok }) => {
  await page.addInitScript(() => localStorage.setItem("rtok.graph.view", "2d"));
  rtok.addProject();
});

test("at 375 px the live metrics are a strip that expands and collapses", async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 800 });
  await page.goto("/#/graph");
  const strip = page.getByRole("button", { name: /running.*calls.*saved/ });
  const cards = page.getByText("latency p50");
  await expect(strip).toHaveAttribute("aria-expanded", "false");
  await expect(cards).toBeHidden();
  await strip.focus();
  await page.keyboard.press("Enter");
  await expect(strip).toHaveAttribute("aria-expanded", "true");
  await expect(cards).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(cards).toBeHidden();
});

test("from 900 px up the cards are shown and there is no strip", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 900 });
  await page.goto("/#/graph");
  await expect(page.getByText("latency p50")).toBeVisible();
  await expect(page.getByRole("button", { name: /running.*calls.*saved/ })).toBeHidden();
});
