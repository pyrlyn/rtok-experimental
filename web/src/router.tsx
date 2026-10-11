// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import {
    createHashHistory,
    createRootRoute,
    createRoute,
    createRouter,
    redirect,
    type RouteComponent,
    type RouterHistory,
} from "@tanstack/react-router";
import { Calls } from "./pages/Calls";
import { Config } from "./pages/Config";
import { Doctor } from "./pages/Doctor";
import { Graph } from "./pages/Graph";
import { Hosts } from "./pages/Hosts";
import { Logs } from "./pages/Logs";
import { Overview } from "./pages/Overview";
import { Plugins } from "./pages/Plugins";
import { Services } from "./pages/Services";
import { Sessions } from "./pages/Sessions";
import { Skills } from "./pages/Skills";
import { Stats } from "./pages/Stats";
import { Usage } from "./pages/Usage";
import { Worktrees } from "./pages/Worktrees";
import { Tasks } from "./pages/Tasks";
import { PAGES, type Page } from "./pages";
import { drillSearch, parseDrill } from "./pages/graph3d/drillState";
import { NotFound, Shell } from "./Shell";
import { validateTableSearch } from "./tableSearch";

const screens: Record<Page["id"], RouteComponent> = {
    overview: Overview,
    sessions: Sessions,
    doctor: Doctor,
    logs: Logs,
    plugins: Plugins,
    calls: Calls,
    skills: Skills,
    stats: Stats,
    graph: Graph,
    hosts: Hosts,
    config: Config,
    services: Services,
    worktrees: Worktrees,
    tasks: Tasks,
    usage: Usage,
};

const root = createRootRoute({ component: Shell, notFoundComponent: NotFound });
const index = createRoute({
    getParentRoute: () => root,
    path: "/",
    beforeLoad: () => {
        throw redirect({ to: `/${PAGES[0].id}` });
    },
});
const pageRoutes = PAGES.map((page) =>
    createRoute({
        getParentRoute: () => root,
        path: page.id,
        component: screens[page.id],
        // The palette opens a page on one row (`?id=`); kept a string even when it looks numeric.
        // The list pages add their filters, search text and sort (T414.10), which are untrusted
        // like any link, so they are validated here and not only where a page reads them.
        // The graph page also carries its drill-down state (T329.22).
        validateSearch: (s: Record<string, unknown>): Record<string, unknown> => ({
            ...(s.id == null ? {} : { id: String(s.id) }),
            ...validateTableSearch(page.id, s),
            ...(page.id === "graph" ? drillSearch(parseDrill(s)) : {}),
        }),
    }),
);

export const routeTree = root.addChildren([index, ...pageRoutes]);

// Hash history: the page is served as one static file and `?sample` lives in the real
// query string, which a path-based router would drop on the first navigation.
export const createAppRouter = (history: RouterHistory = createHashHistory()) =>
    createRouter({ routeTree, history });
