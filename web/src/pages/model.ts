// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import type {
  CallRow,
  PluginPage,
  Report,
  Saving,
  SessionTotals,
  Snapshot,
  UsagePage,
} from "../api/snapshot.gen";
import { fmt } from "./format";

/** Σ tokens a call put through the model, `null` when the call carried no usage. */
export const tokensOf = (c: CallRow): number | null =>
  c.input == null ? null : c.input + (c.cache_create ?? 0) + (c.cache_read ?? 0) + (c.output ?? 0);

export const savedOf = (p: PluginPage): number | null =>
  p.stats ? p.stats.est_before - p.stats.est_after : null;

export const SURFACES = ["hook", "mcp", "proxy"] as const;
export type Surface = (typeof SURFACES)[number];

const sum = <T>(xs: readonly T[], f: (x: T) => number) => xs.reduce((s, x) => s + f(x), 0);

export interface Bucket {
  t: number;
  hook: number;
  mcp: number;
  proxy: number;
  err: number;
}

/**
 * Calls over time: equal buckets over the window the frame carries. The snapshot holds only the
 * last ledger rows, so this is a recent window, not the whole history.
 */
export function callBuckets(calls: readonly CallRow[], n = 24) {
  const now = Math.floor(Date.now() / 1000);
  const ts = calls.map((c) => c.ts);
  const t1 = ts.length ? Math.max(...ts) : now;
  const t0 = ts.length ? Math.min(...ts) : now - 3600;
  const step = Math.max(60, t1 - t0 + 1) / n;
  const buckets: Bucket[] = Array.from({ length: n }, (_, i) => ({
    t: t0 + i * step,
    hook: 0,
    mcp: 0,
    proxy: 0,
    err: 0,
  }));
  for (const c of calls) {
    const b = buckets[Math.min(n - 1, Math.floor((c.ts - t0) / step))];
    if (!b) continue;
    if ((SURFACES as readonly string[]).includes(c.surface)) b[c.surface as Surface]++;
    if (!c.ok) b.err++;
  }
  return { buckets, step };
}

/** Mirrors `saving_text` in `src/doctor/project.rs`: no measurement row, no number. */
export function savingText(s: Saving | null, days: number): string {
  if (!s) return "no measurements yet";
  const per = `≈ ${fmt(s.saved_per_call)} tok per call over ${fmt(s.calls)} calls`;
  return s.project_calls != null && s.approx_tokens != null
    ? `${per}; here ≈ ${fmt(s.approx_tokens)} tok (${fmt(s.project_calls)} calls in ${days} days)`
    : per;
}

export type CheckState = "pass" | "warn" | "fail" | "skip";
export interface Check {
  st: CheckState;
  label: string;
  detail: string;
  /** The `Report` field the row reads: the report has no verdict of its own. */
  field: string;
}

/**
 * Doctor rows derived from `Report` (admin.js `doctorChecks`); `null` is a failed probe, unless
 * `pending`: the first probe has not landed yet (T945).
 */
export function doctorChecks(d: Report | null, pending = false): Check[] {
  if (!d && pending) {
    return [
      {
        st: "skip",
        label: "doctor probe",
        detail: "running doctor…",
        field: "Snapshot.doctor_pending",
      },
    ];
  }
  if (!d) {
    return [
      {
        st: "fail",
        label: "doctor probe",
        detail: "doctor did not answer this tick; `rtok doctor` has the details",
        field: "Snapshot.doctor = null",
      },
    ];
  }
  const checks: Check[] = [
    {
      st: d.hooks_total > 0 ? "pass" : "fail",
      label: "hooks installed",
      detail: `${d.hooks_total} hooks across ${Object.keys(d.hooks_by_event).length} events`,
      field: "hooks_total",
    },
    {
      st: d.mcp.length ? "pass" : "warn",
      label: "MCP servers",
      detail: d.mcp.map((s) => `${s.name} (${s.tools} tools)`).join(", ") || "none probed",
      field: "mcp[]",
    },
    {
      st: d.proxy ? "pass" : "warn",
      label: "Anthropic proxy chain",
      detail: d.proxy || "ANTHROPIC_BASE_URL not set",
      field: "proxy",
    },
    {
      st: d.proxy_openai ? "pass" : "skip",
      label: "OpenAI proxy chain",
      detail: d.proxy_openai || "not configured",
      field: "proxy_openai",
    },
  ];
  if (d.mcp_tool_search_disabled) {
    checks.push({
      st: "warn",
      label: "MCP tool search",
      detail: `${d.mcp_tool_search.state} (${d.mcp_tool_search.source ?? "default"})`,
      field: "mcp_tool_search_disabled",
    });
  }
  checks.push(
    {
      st: d.bash_max_output_length ? "pass" : "skip",
      label: "BASH_MAX_OUTPUT_LENGTH",
      detail: d.bash_max_output_length || "(unset)",
      field: "bash_max_output_length",
    },
    {
      st: d.auto_compact_window ? "pass" : "skip",
      label: "autoCompactWindow",
      detail: d.auto_compact_window || "(unset)",
      field: "auto_compact_window",
    },
  );
  for (const r of d.instructions?.rows ?? []) {
    if (r.warn) {
      checks.push({
        st: "warn",
        label: `instructions: ${r.name}`,
        detail: `${fmt(r.tokens)} tokens, ${r.path}`,
        field: "instructions.rows[].warn",
      });
    }
  }
  for (const [text, names] of d.instructions?.duplicates ?? []) {
    checks.push({
      st: "warn",
      label: "duplicate instruction",
      detail: `"${String(text)}" in ${Array.isArray(names) ? names.join(", ") : String(names)}`,
      field: "instructions.duplicates",
    });
  }
  if (d.skills) {
    const warned = d.skills.rows.filter((r) => r.warn_desc || r.warn_body || r.warn_never);
    checks.push({
      st: warned.length ? "warn" : "pass",
      label: "skills audit",
      detail: warned.length
        ? warned
            .map((r) => {
              const why = [
                r.warn_desc && "long description",
                r.warn_body && "body > 8 KB",
                r.warn_never && "never invoked",
              ].filter(Boolean);
              return `${r.name}: ${why.join(", ")}`;
            })
            .join(" · ")
        : `${d.skills.rows.length} skills OK`,
      field: "skills.rows[].warn_*",
    });
  }
  for (const o of d.overlaps ?? []) {
    checks.push({ st: "warn", label: "host overlap", detail: o, field: "overlaps[]" });
  }
  if (d.tools_rewrite_advice) {
    checks.push({
      st: "warn",
      label: "tools rewrite",
      detail: d.tools_rewrite_advice,
      field: "tools_rewrite_advice",
    });
  }
  return checks;
}

export const recentSessions = (sessions: readonly SessionTotals[], n = 6) =>
  [...sessions].sort((a, b) => b.last_activity - a.last_activity).slice(0, n);

/** Everything the overview shows, derived once from one snapshot (admin.js `derive`). */
export function overview(snap: Snapshot) {
  const u = snap.usage;
  // A plugin without Measurement rows has no estimate; counting it would dilute the saving.
  const measured = snap.plugins.filter((p) => p.stats && p.stats.est_before > 0);
  const estBefore = sum(measured, (p) => p.stats?.est_before ?? 0);
  const estAfter = sum(measured, (p) => p.stats?.est_after ?? 0);
  const saved = estBefore - estAfter;
  const ctx = u.input + u.cache_create + u.cache_read;
  const ms = snap.calls.flatMap((c) => (c.ms == null ? [] : [c.ms])).sort((a, b) => a - b);
  const quantile = (q: number) =>
    ms.length ? (ms[Math.min(ms.length - 1, Math.floor(q * ms.length))] ?? null) : null;
  const { buckets, step } = callBuckets(snap.calls);
  const bySurface = Object.fromEntries(
    SURFACES.map((s) => {
      const rows = snap.calls.filter((c) => c.surface === s);
      return [s, { n: rows.length, err: rows.filter((c) => !c.ok).length }];
    }),
  ) as Record<Surface, { n: number; err: number }>;
  return {
    usage: u,
    ctx,
    measured: measured
      .map((plugin) => ({ plugin, saved: savedOf(plugin) ?? 0 }))
      .sort((a, b) => b.saved - a.saved),
    estBefore,
    estAfter,
    saved,
    deltaPct: estBefore ? saved / estBefore : NaN,
    cacheHit: ctx ? u.cache_read / ctx : NaN,
    failed: snap.calls.filter((c) => !c.ok).length,
    p50: quantile(0.5),
    p95: quantile(0.95),
    buckets,
    step,
    bySurface,
    // Sessions alive in each bucket: started before it ends and not ended before it starts.
    liveSeries: buckets.map(
      (b) =>
        snap.sessions.filter(
          (s) => s.started_at <= b.t + step && (s.ended_at == null || s.ended_at >= b.t),
        ).length,
    ),
    live: snap.sessions.filter((s) => s.ended_at == null).length,
    hosts: new Set(snap.sessions.map((s) => s.host)).size,
    enabled: snap.plugins.filter((p) => p.enabled).length,
    checks: doctorChecks(snap.doctor, snap.doctor_pending),
    recent: recentSessions(snap.sessions),
  };
}

export function matchesSession(s: SessionTotals, liveOnly: boolean, query: string): boolean {
  const q = query.trim().toLowerCase();
  return (
    (!liveOnly || s.ended_at == null) &&
    (!q || [s.id, s.host, s.model, s.project, s.provider].join(" ").toLowerCase().includes(q))
  );
}

export interface LogLine {
  /** Position in the frame, which is newest first; the page shows it as the line number. */
  i: number;
  raw: string;
  ts: string;
  level: string;
  source: string;
  name: string;
  msg: string;
}

const LOG_RE = /^(\d{4}-\d\d-\d\d \d\d:\d\d:\d\d) (\S+) ([^/\s]+)\/([^:]+): (.*)$/;

/** A line that is not in the `ts LEVEL source/name: msg` shape is kept whole, as info. */
export function parseLog(raw: string, i: number): LogLine {
  const m = LOG_RE.exec(raw);
  if (!m) return { i, raw, ts: "", level: "info", source: "", name: "", msg: raw };
  const [, ts = "", level = "info", source = "", name = "", msg = ""] = m;
  return { i, raw, ts, level: level.toLowerCase(), source, name, msg };
}

export const LEVELS = ["all", "info", "warn", "error"] as const;
export type Level = (typeof LEVELS)[number];

export function matchesLog(l: LogLine, level: Level, query: string): boolean {
  const q = query.trim().toLowerCase();
  return (level === "all" || l.level === level) && (!q || l.raw.toLowerCase().includes(q));
}

/** The modules a host row is checked for, in the order the design lists them. */
export const MODULES = ["hooks", "mcp", "proxy", "plugin"] as const;

/** The Usage page's report, as `rtok agents usage --json` prints it; `null` while the logs are read. */
export type UsageReport = NonNullable<UsagePage["report"]>;
