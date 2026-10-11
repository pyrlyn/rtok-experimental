// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { type ReactNode, useEffect, useState } from "react";
import { Chip } from "../../../ui/Chip";

export type Part = "explorer" | "live";

/**
 * Which part fills the page, if any. Deliberately not remembered (unlike the splitter): a page that
 * opens already maximised hides the other part with no hint of why. Escape restores.
 */
export function useMaximised() {
    const [maximised, setMaximised] = useState<Part | null>(null);
    useEffect(() => {
        if (!maximised) return;
        const onKey = (e: KeyboardEvent) => {
            // A menu or dialog that already used Escape keeps it.
            if (e.key === "Escape" && !e.defaultPrevented) setMaximised(null);
        };
        window.addEventListener("keydown", onKey);
        return () => window.removeEventListener("keydown", onKey);
    }, [maximised]);
    return [maximised, setMaximised] as const;
}

/**
 * One part with its maximise button. A maximised part is a fixed overlay rather than a remounted
 * tree, so the explorer keeps its zoom and layout; the other part is hidden by the caller.
 */
export function MaximisablePart({
    label,
    maximised,
    onToggle,
    className,
    children,
}: {
    label: string;
    maximised: boolean;
    onToggle: (next: boolean) => void;
    className: string;
    children: ReactNode;
}) {
    return (
        <div
            className={`${className} ${maximised ? "fixed inset-0 z-40 overflow-auto bg-surface p-4" : ""}`}
        >
            <div className="mb-2 flex justify-end">
                <Chip pressed={maximised} onPressedChange={onToggle}>
                    {maximised ? `Restore ${label}` : `Maximise ${label}`}
                </Chip>
            </div>
            {children}
        </div>
    );
}
