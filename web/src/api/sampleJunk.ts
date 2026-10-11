// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

// Sample junk for the Hosts page's card (T330.7): the numbers `agents junk list` totals, a dry
// run, and a stand-in for the server that removes only the paths it is told to.
import type { Cleared, JunkCard, Planned } from "./snapshot.gen";

const MB = 1024 * 1024;

export const sampleJunkCard: JunkCard = {
  agents: [
    {
      name: "rtok",
      total_bytes: 24 * MB,
      kinds: [
        { kind: "log", class: "safe", items: 2, size_bytes: 3 * MB },
        { kind: "archive", class: "safe", items: 1, size_bytes: 20 * MB },
      ],
      freed_default_bytes: 23 * MB,
      freed_review_bytes: 23 * MB,
    },
    {
      name: "claude",
      total_bytes: 900 * MB,
      kinds: [
        { kind: "cache", class: "safe", items: 1, size_bytes: 310 * MB },
        { kind: "logs", class: "review", items: 4, size_bytes: 12 * MB },
      ],
      freed_default_bytes: 310 * MB,
      freed_review_bytes: 322 * MB,
    },
  ],
  total_bytes: 924 * MB,
  freed_default_bytes: 333 * MB,
  freed_review_bytes: 345 * MB,
};

const item = (agent: string, kind: string, path: string, bytes: number, over = {}): Planned => ({
  action: "clear",
  agent,
  bytes,
  failed: false,
  files: 1,
  kind,
  last_used: 1_789_000_000,
  note: "",
  path,
  planned: true,
  reason: "unused for 30 days",
  ...over,
});

export const samplePlanItems: Planned[] = [
  item("rtok", "log", "/home/u/.rtok/logs/rtok.log.7", 3 * MB),
  item("claude", "cache", "/home/u/.claude/cache", 310 * MB),
  item("claude", "temp", "/home/u/.claude/shell-snapshots", 2 * MB, {
    action: "skip",
    note: "agent running",
    planned: false,
  }),
];

const total = (items: Planned[], pick: (i: Planned) => boolean) =>
  items.filter(pick).reduce((sum, i) => sum + i.bytes, 0);

/** `plan` changes nothing; `apply` removes only the planned items whose path was confirmed. */
export function mockJunk() {
  const removed = new Set<string>();
  const left = () => samplePlanItems.filter((i) => !removed.has(i.path));
  return {
    plan(): Cleared {
      const items = left();
      return {
        yes: false,
        items,
        planned_bytes: total(items, (i) => i.planned),
        freed_bytes: 0,
      };
    },
    apply(paths: string[]): Cleared {
      const items = left()
        .filter((i) => paths.includes(i.path))
        .map((i) => ({ ...i, note: i.planned ? "removed" : i.note }));
      for (const i of items) if (i.planned) removed.add(i.path);
      return {
        yes: true,
        items,
        planned_bytes: total(items, (i) => i.planned),
        freed_bytes: total(items, (i) => i.action === "clear"),
      };
    },
  };
}
