// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { useId, useState, type ReactNode } from "react";
import { focusRing } from "../../../ui/cx";

/**
 * Below 900 px (the width the page stacks at, see Split) the cards fold into a one-line strip so
 * the explorer above is not pushed off a phone screen. The cards stay in the DOM and are only
 * hidden by CSS, so the 900 px breakpoint stays a single media query with no resize listener.
 */
export function MetricsFold({ strip, children }: { strip: ReactNode; children: ReactNode }) {
    const [open, setOpen] = useState(false);
    const body = useId();
    return (
        <div>
            <button
                type="button"
                aria-expanded={open}
                aria-controls={body}
                onClick={() => setOpen((v) => !v)}
                className={`${focusRing} flex w-full items-center gap-3 rounded-md border border-border bg-surface px-3 py-2 text-left text-xs min-[900px]:hidden`}
            >
                <span className="flex min-w-0 flex-1 flex-wrap gap-x-3">{strip}</span>
                <span aria-hidden="true" className="text-fg-subtle">
                    {open ? "Less" : "More"}
                </span>
            </button>
            <div id={body} className={`${open ? "" : "max-[899px]:hidden"} mt-2 min-[900px]:mt-0`}>
                {children}
            </div>
        </div>
    );
}
