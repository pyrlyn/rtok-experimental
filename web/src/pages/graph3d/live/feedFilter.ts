// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import type { Finished } from "../../../api/snapshot.gen";

/** Empty strings mean "all". The totals ignore it: it only narrows the rows the feed lists. */
export interface FeedFilter {
  caller: string;
  tool: string;
  project: string;
}

export function filterFeed(feed: readonly Finished[], f: FeedFilter): Finished[] {
  return feed.filter(
    (r) =>
      (!f.caller || r.caller === f.caller) &&
      (!f.tool || r.tool === f.tool) &&
      (!f.project || r.project === f.project),
  );
}

export const distinct = (feed: readonly Finished[], key: "caller" | "tool" | "project"): string[] =>
  [...new Set(feed.flatMap((r) => (r[key] ? [r[key]] : [])))].sort();
