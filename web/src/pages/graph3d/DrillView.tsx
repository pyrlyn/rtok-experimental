// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { type RefObject, useRef, useState } from "react";
import { useDrill } from "../../api/query";
import type { DrillGraph, ProjectRow } from "../../api/snapshot.gen";
import { Empty } from "../../states";
import { Button } from "../../ui/Button";
import { Chip } from "../../ui/Chip";
import { focusRing } from "../../ui/cx";
import { Panel } from "../../ui/Panel";
import { Pill } from "../../ui/Pill";
import { Result } from "../../ui/Result";
import { Spinner } from "../../ui/Spinner";
import { drillScene, emptyScene } from "./drillScene";
import {
    breadcrumb,
    DEPTH_MAX,
    type DrillState,
    drillRequest,
    drillVersion,
    focusOn,
    openSymbol,
    toggleFile,
} from "./drillState";
import { SceneView, useStableScene } from "./SceneView";
import type { ViewApi } from "./webgl";

/** Nodes the server shows before "+N more"; each click on the control raises the cap by this much. */
export const PAGE = 500;

/** The canvas has no control that can hold a spinner, so a line says a request is out. */
function Busy({ children }: { children: string }) {
    return (
        <p role="status" aria-busy className="flex items-center gap-1.5 text-xs text-fg-muted">
            <span className="text-accent">
                <Spinner size="sm" />
            </span>
            {children}
        </p>
    );
}

function NodeList({
    graph,
    select,
    selected,
}: {
    graph: DrillGraph;
    select(id: string): void;
    selected: string | null;
}) {
    return (
        <ul aria-label="symbol graph" className="flex max-h-[28rem] flex-col gap-1 overflow-auto">
            {graph.nodes.map((n) => (
                <li key={n.id}>
                    <button
                        type="button"
                        aria-pressed={n.id === selected}
                        onClick={() => select(n.id)}
                        className={`${focusRing} flex w-full flex-wrap items-center gap-2 rounded-md border border-border px-2.5 py-1.5 text-left text-xs hover:border-border-strong aria-pressed:border-accent/50 aria-pressed:bg-accent/10`}
                    >
                        <b className="truncate">{n.label}</b>
                        <Pill>{n.kind}</Pill>
                        {n.stale && <Pill tone="warn">stale</Pill>}
                        <span className="ml-auto truncate text-2xs text-fg-subtle">{n.path}</span>
                    </button>
                </li>
            ))}
        </ul>
    );
}

/** Level 2 of the graph page (T329 §8a): one project's files and symbols, drilled from the overview. */
export function DrillView({
    state,
    rows,
    go,
    probe,
}: {
    state: DrillState;
    rows: ProjectRow[];
    go(next: DrillState | null): void;
    probe?: RefObject<ViewApi | null>;
}) {
    const row = rows.find((r) => String(r.id) === state.project);
    const [limit, setLimit] = useState(PAGE);
    const [selected, setSelected] = useState<string | null>(null);
    // The layout keeps the position of an id it has seen, so a string id keeps its number across frames.
    const ids = useRef(new Map<string, number>()).current;
    const own = useRef<ViewApi | null>(null);
    const api = probe ?? own;
    const idOf = (id: string) => ids.get(id) ?? (ids.set(id, ids.size + 1), ids.size);

    const q = useDrill(drillRequest(state, limit), drillVersion(row));
    const graph = q.data;
    const roots = new Map(rows.map((r) => [r.id, r.root]));
    const scene = useStableScene(
        graph?.state === "ok" ? drillScene(graph, { idOf, roots, selected }) : emptyScene,
    );

    const click = (id: string) => {
        const n = graph?.nodes.find((x) => x.id === id);
        if (!n) return;
        if (n.kind === "external")
            return go(openSymbol(n.project, { path: n.path, name: n.label }));
        setSelected(id);
        // The second click on the selected node is the step into it.
        if (id !== selected) return;
        if (n.kind === "file") go(toggleFile(state, n.path));
        else if (n.kind === "function") go(focusOn(state, { path: n.path, name: n.label }));
    };
    const byNumber = (num: number) => graph?.nodes.find((n) => idOf(n.id) === num)?.id;
    const crumbs = breadcrumb(state, graph?.name ?? row?.name ?? `project ${state.project}`);

    return (
        <Panel title="symbol graph" hint={graph ? `${graph.nodes.length} nodes` : undefined}>
            <nav aria-label="breadcrumb">
                <ol className="flex flex-wrap items-center gap-1.5 text-xs">
                    {crumbs.map((c, i) => (
                        <li key={c.label} className="flex items-center gap-1.5">
                            {i > 0 && <span aria-hidden="true">/</span>}
                            {c.to !== undefined ? (
                                <button
                                    type="button"
                                    onClick={() => go(c.to!)}
                                    className={`${focusRing} text-accent-fg underline`}
                                >
                                    {c.label}
                                </button>
                            ) : (
                                <span aria-current="page" className="font-semibold">
                                    {c.label}
                                </span>
                            )}
                        </li>
                    ))}
                </ol>
            </nav>
            {q.error && (
                <Result verb="graph" kind="error">
                    {q.error.message}
                </Result>
            )}
            {!graph ? (
                !q.error && <Busy>Reading the index…</Busy>
            ) : graph.state === "not indexed" ? (
                <Empty
                    title="Not indexed yet"
                    hint={`Run \`rtok graph index --project ${graph.project}\`; the page fills in when the index lands.`}
                />
            ) : graph.state === "missing" ? (
                <Empty title="The directory is missing" hint={graph.root} />
            ) : (
                <SceneView
                    scene={scene}
                    api={api}
                    empty="Nothing to draw"
                    events={{
                        select: (num) => {
                            const id = byNumber(num);
                            if (id) click(id);
                        },
                        menu: () => {},
                    }}
                    list={<NodeList graph={graph} select={click} selected={selected} />}
                    toolbar={
                        state.focus && (
                            <div
                                role="group"
                                aria-label="depth"
                                className="flex items-center gap-1"
                            >
                                {Array.from({ length: DEPTH_MAX }, (_, i) => i + 1).map((d) => (
                                    <Chip
                                        key={d}
                                        pressed={state.depth === d}
                                        onPressedChange={() => go({ ...state, depth: d })}
                                    >
                                        {`depth ${d}`}
                                    </Chip>
                                ))}
                            </div>
                        )
                    }
                >
                    {q.isFetching && <Busy>Updating…</Busy>}
                    {graph.partial && (
                        <Result verb="index" kind="warn">
                            Partial: some files changed since the last index run, so this picture
                            lags the tree.
                        </Result>
                    )}
                    {graph.more > 0 && (
                        <Button pending={q.isFetching} onClick={() => setLimit(limit + PAGE)}>
                            {`+${graph.more} more`}
                        </Button>
                    )}
                </SceneView>
            )}
        </Panel>
    );
}
