// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { useNavigate, useSearch } from "@tanstack/react-router";
import { lazy, Suspense, useMemo } from "react";
import { Empty } from "../states";
import { DataTable, type Column } from "../ui/DataTable";
import { Kpi } from "../ui/Kpi";
import { Panel } from "../ui/Panel";
import { Pill } from "../ui/Pill";
import { fmt } from "./format";
import { OtherLines, responsive, TextPage, useMinWidth, WithSnapshot } from "./parts";
import { Projects } from "./Projects";
import { drillSearch, type DrillState, openProject, parseDrill } from "./graph3d/drillState";
import { Split } from "./graph3d/live/Split";
import { parseGraph, type DeadSymbol, type GraphView } from "./text";

// The overview, its force layout and Three.js load only when the graph page opens.
const ProjectsOverview = lazy(() =>
    import("./graph3d/ProjectsOverview").then((m) => ({ default: m.ProjectsOverview })),
);
const DrillView = lazy(() => import("./graph3d/DrillView").then((m) => ({ default: m.DrillView })));
const LiveSection = lazy(() =>
    import("./graph3d/live/LiveSection").then((m) => ({ default: m.LiveSection })),
);

export function Graph() {
    // The drill-down lives in the URL (T329.22): each step is a history entry, so Back climbs out.
    const drill = parseDrill(useSearch({ strict: false }) as Record<string, unknown>);
    const navigate = useNavigate();
    const go = (next: DrillState | null) =>
        void navigate({ search: ((prev: object) => ({ ...prev, ...drillSearch(next) })) as never });
    return (
        <WithSnapshot>
            {(snap) => (
                <>
                    {snap.projects && <Projects rows={snap.projects} />}
                    {snap.projects && snap.projects.length > 0 && (
                        <Suspense fallback={null}>
                            <Split
                                explorer={
                                    drill ? (
                                        // Another project is a new picture: the layout and zoom start over.
                                        <DrillView
                                            key={drill.project}
                                            state={drill}
                                            rows={snap.projects}
                                            go={go}
                                        />
                                    ) : (
                                        <ProjectsOverview
                                            rows={snap.projects}
                                            onOpen={(id) => go(openProject(id))}
                                        />
                                    )
                                }
                                live={<LiveSection rows={snap.projects} drill={drill} />}
                            />
                        </Suspense>
                    )}
                    <TextPage
                        page="graph"
                        text={snap.graph}
                        command="rtok graph status"
                        absent={
                            <Panel title="graph">
                                <Empty
                                    title="Graph page off"
                                    hint="The graph feature is not built in, or the store read failed (Snapshot.graph = null)."
                                />
                            </Panel>
                        }
                        note="`rtok graph status` index health plus `rtok graph dead`"
                    >
                        {(text) => <GraphBody view={parseGraph(text)} />}
                    </TextPage>
                </>
            )}
        </WithSnapshot>
    );
}

function GraphBody({ view: v }: { view: GraphView }) {
    const wide = useMinWidth(768);
    const full = useMemo<Column<DeadSymbol>[]>(
        () => [
            { id: "symbol", header: "symbol", cell: (r) => <b>{r.name}</b> },
            { id: "kind", header: "kind", width: "88px", cell: (r) => <Pill>{r.kind}</Pill> },
            {
                id: "location",
                header: "location",
                cell: (r) => (
                    <span className="text-fg-muted">
                        {r.path}
                        <span className="text-fg-subtle">:{r.line}</span>
                    </span>
                ),
            },
        ],
        [],
    );
    const columns = responsive(full, wide, ["symbol", "location"]);
    return (
        <>
            <p className="truncate text-2xs text-fg-subtle">
                root <span className="text-fg-muted">{v.root}</span>
            </p>
            <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
                <Kpi label="rows" value={fmt(v.rows)} sub="tree-sitter-tags defs + refs" />
                <Kpi label="files" value={fmt(v.files)} />
                <Kpi
                    label="pending"
                    value={fmt(v.pending.length)}
                    sub={v.pending.length ? "stale since last index" : "index is fresh"}
                    tone={v.pending.length ? "warn" : "ok"}
                />
                <Kpi
                    label="watch"
                    value={v.watch ? "on" : "off"}
                    tone={v.watch ? "ok" : "default"}
                    sub={`indexed ${v.indexedAt}`}
                />
            </div>
            <div className="grid grid-cols-1 gap-3 xl:grid-cols-[minmax(0,1fr)_minmax(0,2fr)]">
                <Panel title="pending files">
                    {v.pending.length === 0 ? (
                        <Empty title="Nothing pending" />
                    ) : (
                        <ul className="flex flex-col divide-y divide-border/60 text-xs">
                            {v.pending.map((p) => (
                                <li key={p} className="flex items-center gap-2 py-2">
                                    <Pill tone="warn">pending</Pill>
                                    <span className="truncate">{p}</span>
                                </li>
                            ))}
                        </ul>
                    )}
                </Panel>
                <Panel title="dead symbols" hint={`${v.dead.length} unreferenced`}>
                    <DataTable
                        label="dead symbols"
                        rows={v.dead}
                        columns={columns}
                        getRowId={(r) => `${r.path}:${r.line}:${r.name}`}
                        empty={<Empty title="No unreferenced definitions" />}
                    />
                    {v.deadNote && <p className="text-2xs text-fg-subtle">{v.deadNote}</p>}
                </Panel>
            </div>
            <OtherLines lines={v.other} />
        </>
    );
}
