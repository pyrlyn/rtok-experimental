// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { expect, test } from "vitest";
import { done } from "./callsFixtures";
import { distinct, filterFeed } from "./feedFilter";

test("caller, tool and project narrow the rows and blank means all", () => {
  const feed = [
    done("c", 2, 1, { caller: "y", project: "q" }),
    done("b", 2, 1, { caller: "y", tool: "impact", project: "p" }),
    done("a", 2, 1, { caller: "x", project: "p" }),
  ];
  const none = { caller: "", tool: "", project: "" };
  expect(filterFeed(feed, none)).toHaveLength(3);
  expect(filterFeed(feed, { ...none, caller: "y" }).map((r) => r.call)).toEqual(["c", "b"]);
  expect(filterFeed(feed, { ...none, caller: "y", tool: "callers" })).toHaveLength(1);
  expect(distinct(feed, "project")).toEqual(["p", "q"]);
});
