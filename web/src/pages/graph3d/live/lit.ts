// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import type { Scene } from "../scene";
import { type CallsStore, HEAT_MS, MAX_ACCENTS } from "./callsStore";

/** What the live canvas adds to one node. */
export interface Lit {
  /** 0 to 1: how often the node was queried in the heat window, fading with age. */
  heat: number;
  /** Accent slots of the calls running on the node (0 to `MAX_ACCENTS - 1`). */
  accents: number[];
  /** A call on the node failed a moment ago. */
  failed: boolean;
}

export interface LiveState {
  lit: Map<number, Lit>;
  /** Running calls beyond the accents: drawn as one "busy" count, not as nodes. */
  busy: number;
}

const FAIL_FLASH_MS = 5000;
/** About this many recent calls on one node make it fully hot. */
const FULL_HEAT = 3;

const bare = (label: string) => label.replace(/^⚠ /, "");

/**
 * The overview draws projects, so a call lights its project; the drill-down draws symbols and
 * files, so it lights the node named like the call's target. A call on something the picture
 * does not hold lights nothing (the totals still count it).
 */
export function liveState(scene: Scene, store: CallsStore, now: number): LiveState {
  const overview = scene.label === "registered projects";
  const key = (c: { project: string | null; target: string | null }) =>
    overview ? c.project : c.target;
  const byKey = new Map<string, number[]>();
  for (const n of scene.nodes) {
    const k = overview ? n.label : bare(n.label);
    byKey.set(k, [...(byKey.get(k) ?? []), n.id]);
  }
  const lit = new Map<number, Lit>();
  const at = (id: number) => {
    let l = lit.get(id);
    if (!l) lit.set(id, (l = { heat: 0, accents: [], failed: false }));
    return l;
  };
  for (const r of store.feed) {
    const age = now - r.at;
    const k = key(r);
    if (r.interrupted || age > HEAT_MS || !k) continue;
    for (const id of byKey.get(k) ?? []) {
      const l = at(id);
      l.heat = Math.min(1, l.heat + (1 - age / HEAT_MS) / FULL_HEAT);
      if (!r.ok && age < FAIL_FLASH_MS) l.failed = true;
    }
  }
  store.running.forEach((r, slot) => {
    if (slot >= MAX_ACCENTS) return;
    const k = key(r);
    for (const id of (k && byKey.get(k)) || []) at(id).accents.push(slot);
  });
  return { lit, busy: Math.max(0, store.running.length - MAX_ACCENTS) };
}

const ACCENT_ROLES = ["--pyr-accent-fg", "--pyr-delta-fg", "--pyr-success-fg", "--pyr-warn-fg"];

/** Four brand hues, the second four with a dashed ring, so eight calls tell apart without a categorical scale. */
export const accentOf = (slot: number) => ({
  color: `var(${ACCENT_ROLES[slot % ACCENT_ROLES.length]})`,
  dash: slot >= ACCENT_ROLES.length ? "3 2" : undefined,
});
