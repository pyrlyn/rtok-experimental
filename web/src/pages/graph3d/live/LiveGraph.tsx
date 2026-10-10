// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { useRef } from "react";
import { useDrill } from "../../../api/query";
import type { ProjectRow } from "../../../api/snapshot.gen";
import { Empty } from "../../../states";
import { drillScene, emptyScene } from "../drillScene";
import { PAGE } from "../DrillView";
import { type DrillState, drillRequest, drillVersion } from "../drillState";
import { buildScene, type Scene } from "../scene";
import Scene2D from "../Scene2D";
import { useStableScene } from "../SceneView";
import { useLayout } from "../useLayout";
import type { ViewApi } from "../webgl";
import type { CallsStore } from "./callsStore";
import { liveState } from "./lit";

const noop = () => {};
const events = { select: noop, menu: noop, hover: noop };

function Canvas({ scene, store, now }: { scene: Scene; store: CallsStore; now: number }) {
    const positions = useLayout(scene);
    const api = useRef<ViewApi | null>(null);
    const live = liveState(scene, store, now);
    // Until the first call the picture is the static graph, dimmed.
    const idle = store.all.calls === 0 && store.running.length === 0;
    return (
        <div className="relative h-80 overflow-hidden rounded-md border border-border bg-bg/60">
            {scene.nodes.length === 0 ? (
                <Empty title="Nothing to draw" />
            ) : (
                <div className={`size-full ${idle ? "opacity-40" : ""}`}>
                    <Scene2D
                        scene={scene}
                        positions={positions}
                        api={api}
                        live={live}
                        {...events}
                    />
                </div>
            )}
            {idle && (
                <p
                    role="status"
                    className="absolute inset-0 grid place-items-center text-xs text-fg-muted"
                >
                    Waiting for graph calls
                </p>
            )}
            {live.busy > 0 && (
                <p
                    role="status"
                    className="absolute top-2 right-2 rounded-md border border-warn/40 bg-warn/10 px-2 py-1 text-2xs text-warn-fg motion-safe:animate-pulse"
                >
                    busy: {live.busy} more
                </p>
            )}
        </div>
    );
}

function Overview({ rows, store, now }: { rows: ProjectRow[]; store: CallsStore; now: number }) {
    const scene = useStableScene(buildScene(rows, { query: "", scopeOnly: false }));
    return <Canvas scene={scene} store={store} now={now} />;
}

function Drilled({
    state,
    rows,
    store,
    now,
}: {
    state: DrillState;
    rows: ProjectRow[];
    store: CallsStore;
    now: number;
}) {
    const ids = useRef(new Map<string, number>()).current;
    const idOf = (id: string) => ids.get(id) ?? (ids.set(id, ids.size + 1), ids.size);
    const row = rows.find((r) => String(r.id) === state.project);
    const graph = useDrill(drillRequest(state, PAGE), drillVersion(row)).data;
    const roots = new Map(rows.map((r) => [r.id, r.root]));
    const scene = useStableScene(
        graph?.state === "ok" ? drillScene(graph, { idOf, roots, selected: null }) : emptyScene,
    );
    return <Canvas scene={scene} store={store} now={now} />;
}

/** Part 2 of the graph page: the level part 1 shows, drawn read-only and lit by the calls arriving. */
export function LiveGraph({
    rows,
    drill,
    store,
    now,
}: {
    rows: ProjectRow[];
    drill: DrillState | null;
    store: CallsStore;
    now: number;
}) {
    // Another project is a new picture: the layout starts over, as in part 1.
    return drill ? (
        <Drilled key={drill.project} state={drill} rows={rows} store={store} now={now} />
    ) : (
        <Overview rows={rows} store={store} now={now} />
    );
}
