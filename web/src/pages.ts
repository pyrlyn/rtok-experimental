// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import type { Snapshot } from "./api/snapshot.gen";

// Mirrors `model::pages()` (src/web/model.rs): page id and the snapshot field it reads. The
// route tree, sidebar and tab bar are all built from this one list, so a page cannot be
// routable and missing from the nav. `tests/surface_parity.rs` (`spa_page_list_matches_the_model`) fails when this list drifts.
export const PAGES = [
  { id: "overview", field: "usage" },
  { id: "plugins", field: "plugins" },
  { id: "calls", field: "calls" },
  { id: "sessions", field: "sessions" },
  { id: "doctor", field: "doctor" },
  { id: "logs", field: "logs" },
  { id: "skills", field: "skills" },
  { id: "stats", field: "stats" },
  { id: "graph", field: "graph" },
  { id: "hosts", field: "hosts" },
  { id: "config", field: "config" },
  { id: "services", field: "services" },
  { id: "worktrees", field: "worktrees" },
  { id: "usage", field: "agent_usage" },
  { id: "tasks", field: "tasks" },
] as const satisfies readonly { id: string; field: keyof Snapshot }[];

export type Page = (typeof PAGES)[number];
