// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { type ReactNode, useMemo, useState } from "react";
import { Empty } from "../../../states";
import { type Column, DataTable } from "../../../ui/DataTable";
import { Pill } from "../../../ui/Pill";
import { Select } from "../../../ui/Select";
import { compact, hms } from "../../format";
import type { Finished } from "../../../api/snapshot.gen";
import { distinct, type FeedFilter, filterFeed } from "./feedFilter";
import { outsideScope } from "./scope";

const none: FeedFilter = { caller: "", tool: "", project: "" };

/** Failed rows are red and an interrupted one says so; nothing in the feed is clickable. */
function Cell({ row, children }: { row: Finished; children: ReactNode }) {
    return <span className={row.ok ? "" : "text-delta-fg"}>{children}</span>;
}

/** The project column also says when a call crossed out of the scope; the canvas ignores such a call. */
const columnsFor = (scope: ReadonlySet<string> | null): Column<Finished>[] => [
    {
        id: "time",
        header: "time",
        width: "72px",
        cell: (r) => <Cell row={r}>{hms(Math.floor(r.at / 1000))}</Cell>,
    },
    { id: "tool", header: "tool", width: "80px", cell: (r) => <Cell row={r}>{r.tool}</Cell> },
    { id: "target", header: "symbol", cell: (r) => <Cell row={r}>{r.target ?? "-"}</Cell> },
    // The server names the caller from the store's agent and host rows (T329.34), else the session id.
    {
        id: "caller",
        header: "caller",
        width: "136px",
        cell: (r) => <Cell row={r}>{r.caller}</Cell>,
    },
    {
        id: "project",
        header: "project",
        cell: (r) => (
            <Cell row={r}>
                {r.project ?? "-"}
                {outsideScope(scope, r.project) && (
                    <>
                        {" "}
                        <Pill tone="warn">outside scope</Pill>
                    </>
                )}
            </Cell>
        ),
    },
    {
        id: "result",
        header: "result",
        cell: (r) =>
            r.interrupted ? (
                <Pill tone="warn">interrupted</Pill>
            ) : r.ok ? (
                <Cell row={r}>
                    {r.backend ?? "-"} · {compact(r.before - r.after)} saved ·{" "}
                    {Math.round(r.ms ?? 0)} ms
                </Cell>
            ) : (
                <Cell row={r}>{r.error ?? "failed"}</Cell>
            ),
    },
];

export function CallFeed({
    feed,
    scope,
}: {
    feed: readonly Finished[];
    scope: ReadonlySet<string> | null;
}) {
    const [filter, setFilter] = useState(none);
    const columns = useMemo(() => columnsFor(scope), [scope]);
    const rows = useMemo(() => filterFeed(feed, filter), [feed, filter]);
    const pick = (key: keyof FeedFilter, label: string) => (
        <Select
            label={`filter by ${label}`}
            value={filter[key]}
            onChange={(v) => setFilter({ ...filter, [key]: v })}
        >
            <option value="">all {label}s</option>
            {distinct(feed, key).map((v) => (
                <option key={v} value={v}>
                    {v}
                </option>
            ))}
        </Select>
    );
    return (
        <div className="flex flex-col gap-2">
            <div className="flex flex-wrap gap-2">
                {pick("caller", "caller")}
                {pick("tool", "tool")}
                {pick("project", "project")}
            </div>
            <DataTable
                label="graph calls"
                rows={rows}
                columns={columns}
                getRowId={(r) => r.call}
                height={240}
                empty={<Empty title="No calls yet" />}
            />
        </div>
    );
}
