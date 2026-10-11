// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { Link } from "@tanstack/react-router";
import { useCallback, useState } from "react";
import logo from "@brand/logo/rtok-mark.svg";
import type { Page } from "./pages";
import { readStored, writeStored } from "./storage";
import { focusRing } from "./ui/cx";
import { Icon } from "./ui/Icon";

// Display order and grouping only; `PAGES` stays the list of what exists, and a test fails
// when a page is in no group or in two.
export const NAV_GROUPS = [
    {
        label: "Monitor",
        pages: ["overview", "stats", "usage", "tasks", "calls", "sessions", "logs"],
    },
    {
        label: "Configure",
        pages: ["plugins", "hosts", "skills", "config", "services", "worktrees"],
    },
    { label: "Diagnose", pages: ["doctor", "graph"] },
] as const satisfies readonly { label: string; pages: readonly Page["id"][] }[];

const KEY = "rtok-sidebar";

export function useSidebarCollapsed() {
    const [collapsed, setCollapsed] = useState(() => readStored(KEY) === "collapsed");
    const toggle = useCallback(() => {
        const next = !collapsed;
        writeStored(KEY, next ? "collapsed" : "expanded");
        setCollapsed(next);
    }, [collapsed]);
    return { collapsed, toggle };
}

// Narrow screens get a bottom tab bar (icon over label, centred); from md up it is the sidebar,
// and `collapsed` shrinks that to icons only. The label stays in the DOM (screen-reader only)
// so a collapsed link keeps its name; `title` shows it to a pointer.
const navLink = (collapsed: boolean) =>
    `${focusRing} flex min-w-16 shrink-0 flex-col items-center justify-center gap-1 rounded-md px-2 py-1.5 text-2xs md:h-9 md:min-w-0 md:text-xs text-fg-muted hover:bg-surface-2 hover:text-fg aria-[current=page]:bg-accent/15 aria-[current=page]:text-accent-fg ${
        collapsed
            ? "md:flex-row md:justify-center md:gap-0 md:px-0 md:py-0"
            : "md:flex-row md:justify-start md:gap-3 md:px-2.5 md:py-0"
    }`;

const toggleButton = `${focusRing} mt-auto hidden h-9 items-center rounded-md text-xs text-fg-muted hover:bg-surface-2 hover:text-fg md:flex`;

export function Sidebar({ collapsed, onToggle }: { collapsed: boolean; onToggle: () => void }) {
    const label = collapsed ? "md:sr-only" : "";
    return (
        <nav
            aria-label="Admin screens"
            className="glass fixed inset-x-2 bottom-2 z-40 flex gap-0.5 overflow-x-auto p-1 md:sticky md:inset-auto md:top-0 md:m-3 md:h-[calc(100vh-1.5rem)] md:flex-col md:overflow-y-auto"
        >
            <div
                className={`mb-2 hidden items-center gap-2.5 border-b border-border pt-2 pb-3 md:flex ${
                    collapsed ? "justify-center" : "px-2.5"
                }`}
            >
                <img src={logo} alt="" width={28} height={28} className="rounded-md" />
                <span className={`flex flex-col leading-none ${collapsed ? "md:hidden" : ""}`}>
                    <span className="text-sm font-bold tracking-wordmark">RTOK</span>
                    <span className="mt-1 text-2xs text-fg-subtle">
                        <span className="text-delta-fg">Δ</span>tok
                    </span>
                </span>
            </div>
            {NAV_GROUPS.map((group, i) => (
                <div
                    key={group.label}
                    role="group"
                    aria-labelledby={`nav-${group.label}`}
                    // On a phone the groups dissolve so the bar stays one scrolling row.
                    className={`contents md:flex md:flex-col md:gap-0.5 ${
                        collapsed && i > 0 ? "md:mt-1 md:border-t md:border-border md:pt-2" : ""
                    }`}
                >
                    <span
                        id={`nav-${group.label}`}
                        className={`sr-only ${
                            collapsed
                                ? ""
                                : "md:not-sr-only md:px-2.5 md:pt-3 md:pb-1 md:text-2xs md:font-semibold md:tracking-wider md:text-fg-subtle md:uppercase"
                        }`}
                    >
                        {group.label}
                    </span>
                    {group.pages.map((id) => (
                        <Link key={id} to={`/${id}`} title={id} className={navLink(collapsed)}>
                            <Icon name={id} />
                            <span className={label}>{id}</span>
                        </Link>
                    ))}
                </div>
            ))}
            <button
                type="button"
                onClick={onToggle}
                aria-expanded={!collapsed}
                title={collapsed ? "Expand the sidebar" : "Collapse the sidebar"}
                className={`${toggleButton} ${collapsed ? "justify-center" : "gap-3 px-2.5"}`}
            >
                <Icon name="sidebar" />
                <span className={label}>Sidebar</span>
            </button>
        </nav>
    );
}
