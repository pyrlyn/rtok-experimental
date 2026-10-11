// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import type { Scene } from "../scene";
import type { CallsView } from "../../../api/snapshot.gen";
import { outsideScope } from "./scope";

/** What the live canvas adds to one node. */
export interface Lit {
  /** 0 to 1: how often the node was queried in the heat window, fading with age. */
  heat: number;
  /** Accent slots of the calls running on the node (0 to `MAX_ACCENTS - 1`). */
  accents: number[];
  /** A call on the node failed a moment ago. */
  failed: boolean;
  /** Calls whose own node the cap folded into "+N more", counted on the nearest node that holds them. */
  folded: number;
}

export interface LiveState {
  lit: Map<number, Lit>;
  /** Running calls beyond the accents: drawn as one "busy" count, not as nodes. */
  busy: number;
  /** The nodes the camera frames: running calls and calls that ended a moment ago, sorted. Empty means the overview. */
  focus: number[];
  /** Folded calls no visible node holds: they count on the "+N more" group itself. */
  group: number;
}

export interface LiveOptions {
  /** Names of the projects in scope; calls on any other do not light the canvas. `null` means no scope. */
  scope?: ReadonlySet<string> | null;
  /** Nodes the cap left out of the drill-down. Without any, a target nobody drew is just unknown. */
  more?: number;
}

/** The accents: running calls beyond this many are only counted ("busy"). */
export const MAX_ACCENTS = 8;
const FAIL_FLASH_MS = 5000;
/** A call that ended this long ago still holds the camera: most calls end before a frame shows them running. */
export const FOCUS_MS = 4000;
/** About this many recent calls on one node make it fully hot. */
const FULL_HEAT = 3;

const bare = (label: string) => label.replace(/^⚠ /, "");

/**
 * The overview draws projects, so a call lights its project; the drill-down draws symbols and
 * files, so it lights the node named like the call's target. A call on something the picture
 * does not hold lights nothing (the totals still count it).
 */
export function liveState(
  scene: Scene,
  store: CallsView,
  now: number,
  { scope = null, more = 0 }: LiveOptions = {},
): LiveState {
  // `[plugins.graph] live_heat_window_s`: heat fades over this long.
  const heatMs = store.heat_window_s * 1000;
  const overview = scene.label === "registered projects";
  const key = (c: { project: string | null; target: string | null }) =>
    overview ? c.project : c.target;
  const byKey = new Map<string, number[]>();
  for (const n of scene.nodes) {
    const k = overview ? n.label : bare(n.label);
    byKey.set(k, [...(byKey.get(k) ?? []), n.id]);
  }
  const lit = new Map<number, Lit>();
  const focus = new Set<number>();
  const at = (id: number) => {
    let l = lit.get(id);
    if (!l) lit.set(id, (l = { heat: 0, accents: [], failed: false, folded: 0 }));
    return l;
  };
  // A folded node has no position, but its file or directory may be drawn: a target that is a
  // path counts on the drawn node whose path holds it, the longest first.
  const holders = scene.nodes.filter((n) => n.root).sort((a, b) => b.root.length - a.root.length);
  let group = 0;
  const fold = (target: string | null) => {
    if (!more || !target) return;
    const holder = holders.find((n) => target === n.root || target.startsWith(`${n.root}/`));
    if (holder) at(holder.id).folded++;
    else group++;
  };
  for (const r of store.feed) {
    const age = now - r.at;
    const k = key(r);
    if (r.interrupted || age > heatMs || !k || outsideScope(scope, r.project)) continue;
    const drawn = byKey.get(k);
    if (!drawn && !overview) fold(r.target);
    for (const id of drawn ?? []) {
      if (age <= FOCUS_MS) focus.add(id);
      const l = at(id);
      l.heat = Math.min(1, l.heat + (1 - age / heatMs) / FULL_HEAT);
      if (!r.ok && age < FAIL_FLASH_MS) l.failed = true;
    }
  }
  store.running.forEach((r, slot) => {
    if (slot >= MAX_ACCENTS) return;
    const k = key(r);
    if (outsideScope(scope, r.project)) return;
    const drawn = k ? byKey.get(k) : undefined;
    if (!drawn && !overview) fold(r.target);
    for (const id of drawn ?? []) {
      at(id).accents.push(slot);
      focus.add(id);
    }
  });
  return {
    lit,
    busy: Math.max(0, store.running.length - MAX_ACCENTS),
    focus: [...focus].sort((a, b) => a - b),
    group,
  };
}

/**
 * What the canvas says about calls on folded nodes: the drawn node that holds them, or the
 * "+N more" group when none does. Largest first, so the busiest ancestor leads.
 */
export function foldedCounters(scene: Scene, live: LiveState, more: number) {
  const counters = scene.nodes.flatMap((n) => {
    const calls = live.lit.get(n.id)?.folded ?? 0;
    return calls ? [{ label: bare(n.label), calls }] : [];
  });
  if (live.group) counters.push({ label: `+${more} more`, calls: live.group });
  return counters.sort((a, b) => b.calls - a.calls);
}

const ACCENT_ROLES = ["--pyr-accent-fg", "--pyr-delta-fg", "--pyr-success-fg", "--pyr-warn-fg"];

/** Four brand hues, the second four with a dashed ring, so eight calls tell apart without a categorical scale. */
export const accentOf = (slot: number) => ({
  color: `var(${ACCENT_ROLES[slot % ACCENT_ROLES.length]})`,
  dash: slot >= ACCENT_ROLES.length ? "3 2" : undefined,
});
