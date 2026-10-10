// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import type { DrillFocus, DrillRequest, ProjectRow } from "../../api/snapshot.gen";
import { asString } from "../../tableSearch";

/** Where the drill-down is, carried in the route's search params so back/forward step through it (T329.22). */
export interface DrillState {
  /** The registry id as text, which is what the server's frame names back. */
  project: string;
  /** Files shown as their definitions. */
  expand: string[];
  focus: DrillFocus | null;
  /** How far a focus reaches in calls; the server clamps to the same 1 to 4. */
  depth: number;
}

export const DEPTH_DEFAULT = 1;
export const DEPTH_MAX = 4;
/** More files than a person expands by hand; a link carrying more is junk. */
const MAX_EXPAND = 50;
const MAX_TEXT = 500;

const text = (v: unknown) => asString(v)?.slice(0, MAX_TEXT) || undefined;

/** Untrusted search params in; `null` (the level-1 overview) unless they name a project. */
export function parseDrill(raw: Record<string, unknown>): DrillState | null {
  const project = text(raw.p);
  if (!project || !/^\d{1,9}$/.test(project)) return null;
  const files = Array.isArray(raw.x) ? raw.x : raw.x === undefined ? [] : [raw.x];
  const path = text(raw.fp);
  const name = text(raw.fn);
  const depth = Math.trunc(Number(asString(raw.d) ?? DEPTH_DEFAULT));
  return {
    project,
    expand: [...new Set(files.map(text).filter((f): f is string => !!f))].slice(0, MAX_EXPAND),
    focus: path && name ? { path, name } : null,
    depth: depth >= 1 && depth <= DEPTH_MAX ? depth : DEPTH_DEFAULT,
  };
}

/** The state as search params; defaults are left out so the overview and a plain drill have short URLs. */
export function drillSearch(s: DrillState | null) {
  return {
    p: s?.project,
    x: s?.expand.length ? s.expand : undefined,
    fp: s?.focus?.path,
    fn: s?.focus?.name,
    d: s?.focus && s.depth !== DEPTH_DEFAULT ? s.depth : undefined,
  };
}

export interface Crumb {
  label: string;
  /** Where a click goes; `undefined` for the place the page is at. */
  to?: DrillState | null;
}

/** `All projects / rtok / src/plugins/graph/drill.rs`: each step but the last leads back up. */
export function breadcrumb(s: DrillState, name: string): Crumb[] {
  const here = s.focus?.path ?? s.expand.at(-1);
  return [
    { label: "All projects", to: null },
    here ? { label: name, to: { ...s, expand: [], focus: null } } : { label: name },
    ...(here ? [{ label: here }] : []),
  ];
}

/** Toggles a file in or out of the expanded set. A focus draws only its neighbourhood, so it ends here or the expansion would not show. */
export const toggleFile = (s: DrillState, path: string): DrillState => ({
  ...s,
  focus: null,
  expand: s.expand.includes(path) ? s.expand.filter((p) => p !== path) : [...s.expand, path],
});

export const focusOn = (s: DrillState, focus: DrillFocus): DrillState => ({ ...s, focus });

export const openProject = (id: number): DrillState => ({
  project: String(id),
  expand: [],
  focus: null,
  depth: DEPTH_DEFAULT,
});

/** A symbol of another project: that project, nothing expanded, the symbol focused. */
export const openSymbol = (project: number, focus: DrillFocus): DrillState => ({
  ...openProject(project),
  focus,
});

/** What the server is asked for a drill state; part 1 and the live picture ask the same, so one reply serves both. */
export const drillRequest = (state: DrillState, limit: number): DrillRequest => ({
  project: state.project,
  expand: state.expand,
  focus: state.focus,
  depth: state.focus ? state.depth : null,
  limit,
  query: "",
});

/** The index numbers are the version: when watch re-indexes, the page asks again. */
export const drillVersion = (row: ProjectRow | undefined) => [
  row?.index?.rows,
  row?.index?.files,
  row?.index?.pending,
  row?.index?.indexed_at,
];
