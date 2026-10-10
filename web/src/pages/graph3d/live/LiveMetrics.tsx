// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { Chip } from "../../../ui/Chip";
import { Kpi } from "../../../ui/Kpi";
import { Pill } from "../../../ui/Pill";
import { compact, fmt, pct } from "../../format";
import { type CallsStore, windowTotals, WINDOWS, type WindowId } from "./callsStore";

const seconds = (ms: number) => `${Math.max(0, Math.round(ms / 1000))}s`;

/**
 * The numbers come from `summary` and the `Measurement` samples of the end events, which are the
 * rows `rtok stats` adds up, so a window here equals `rtok stats` over the same calls.
 */
export function LiveMetrics({
    store,
    now,
    window,
    onWindow,
    frozen,
    pending,
    onFreeze,
}: {
    store: CallsStore;
    now: number;
    window: WindowId;
    onWindow(next: WindowId): void;
    frozen: boolean;
    /** Calls that arrived while the picture was held. */
    pending: number;
    onFreeze(on: boolean): void;
}) {
    const t = windowTotals(store, window, now);
    const saved = t.before - t.after;
    const tools = Object.entries(t.tools).sort((a, b) => b[1].calls - a[1].calls);
    const top = Math.max(1, ...tools.map(([, v]) => v.calls));
    const listed = Object.values(t.backends).reduce((n, v) => n + v, 0);
    return (
        <div className="flex flex-col gap-2">
            <div className="flex flex-wrap items-center gap-2">
                <div role="group" aria-label="window" className="flex gap-1">
                    {WINDOWS.map((w) => (
                        <Chip
                            key={w.id}
                            pressed={window === w.id}
                            onPressedChange={() => onWindow(w.id)}
                        >
                            {w.label}
                        </Chip>
                    ))}
                </div>
                <div className="ml-auto flex items-center gap-2">
                    {frozen && pending > 0 && (
                        <span role="status" className="text-2xs text-fg-muted">
                            {fmt(pending)} held
                        </span>
                    )}
                    <Chip pressed={frozen} onPressedChange={onFreeze}>
                        {frozen ? "Unfreeze" : "Freeze"}
                    </Chip>
                </div>
            </div>
            <div className="grid grid-cols-2 gap-2 xl:grid-cols-3">
                <Kpi
                    label="running"
                    value={fmt(store.running.length)}
                    sub={
                        store.running.length ? store.running.map((r) => r.tool).join(", ") : "idle"
                    }
                />
                <Kpi
                    label="calls"
                    value={fmt(t.calls)}
                    tone={t.failed ? "warn" : "default"}
                    sub={`${fmt(t.failed)} failed`}
                />
                <Kpi label="tokens sent" value={compact(t.after)} />
                <Kpi label="without rtok" value={compact(t.before)} />
                <Kpi
                    label="saved"
                    value={compact(saved)}
                    tone="saved"
                    sub={t.before ? pct(saved / t.before, 0) : "-"}
                />
            </div>
            {store.running.length > 0 && (
                <ul aria-label="running calls" className="flex flex-col gap-1 text-xs">
                    {store.running.map((r) => (
                        <li key={r.call} className="flex flex-wrap items-center gap-2">
                            <b>{r.tool}</b>
                            <span className="truncate text-fg-muted">{r.target ?? "-"}</span>
                            <span className="text-fg-subtle">{r.project ?? "-"}</span>
                            <span className="ml-auto text-fg-subtle">{seconds(now - r.at)}</span>
                        </li>
                    ))}
                </ul>
            )}
            <ul aria-label="calls per tool" className="flex flex-col gap-1 text-xs">
                {tools.map(([tool, v]) => (
                    <li key={tool} className="grid grid-cols-[6rem_1fr_auto] items-center gap-2">
                        <span>{tool}</span>
                        <span
                            role="presentation"
                            style={{ width: `${(v.calls / top) * 100}%` }}
                            className="h-2 rounded-full bg-accent/60"
                        />
                        <span className="text-fg-subtle">
                            {fmt(v.calls)} · {compact(v.saved)} saved
                        </span>
                    </li>
                ))}
            </ul>
            {listed > 0 && (
                <p className="flex flex-wrap items-center gap-1.5 text-2xs text-fg-subtle">
                    backend
                    {Object.entries(t.backends).map(([b, n]) => (
                        <Pill key={b}>
                            {b} {pct(n / listed, 0)}
                        </Pill>
                    ))}
                </p>
            )}
        </div>
    );
}
