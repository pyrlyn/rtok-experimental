// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { useState } from "react";
import { useConnection } from "../../../api/query";
import type { ProjectRow } from "../../../api/snapshot.gen";
import { Panel } from "../../../ui/Panel";
import type { DrillState } from "../drillState";
import { CallFeed } from "./CallFeed";
import type { WindowId } from "./callsStore";
import { LiveGraph } from "./LiveGraph";
import { LiveMetrics } from "./LiveMetrics";
import { useCalls } from "./useCalls";

/**
 * Part 2 of the graph page (T329 §8b). It exists only while visible, and the call stream is
 * subscribed only while it exists: a hidden live graph asks the server for nothing.
 */
export function LiveSection({ rows, drill }: { rows: ProjectRow[]; drill: DrillState | null }) {
    const calls = useCalls();
    const [window, setWindow] = useState<WindowId>("5m");
    const connection = useConnection();
    return (
        <Panel title="live graph" hint={connection === "open" ? "read-only" : "reconnecting"}>
            <LiveGraph rows={rows} drill={drill} store={calls.store} now={calls.now} />
            <LiveMetrics
                store={calls.store}
                now={calls.now}
                window={window}
                onWindow={setWindow}
                frozen={calls.frozen}
                pending={calls.pending}
                onFreeze={calls.freeze}
            />
            <CallFeed feed={calls.store.feed} />
        </Panel>
    );
}
