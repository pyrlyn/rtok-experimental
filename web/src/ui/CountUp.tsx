// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { useEffect, useRef, useState } from "react";
import { prefersReducedMotion } from "../pages/graph3d/webgl";

export const COUNT_MS = 600;

const easeOut = (t: number) => 1 - (1 - t) ** 3;

/**
 * The number to draw for `value`: it eases from what is on screen to the new value over
 * `COUNT_MS`. The first value is drawn as is (a page opening is not an event), and under
 * `prefers-reduced-motion` every change jumps.
 */
export function useCountUp(value: number): number {
    const [shown, setShown] = useState(value);
    const drawn = useRef(value);
    useEffect(() => {
        const from = drawn.current;
        if (from === value) return;
        // Read per change: the preference can flip while the page is open.
        if (prefersReducedMotion() || !Number.isFinite(value) || !Number.isFinite(from)) {
            drawn.current = value;
            setShown(value);
            return;
        }
        const t0 = performance.now();
        let raf = 0;
        const step = () => {
            const t = Math.min(1, (performance.now() - t0) / COUNT_MS);
            // The last frame is the server's value itself, never a rounded neighbour of it.
            drawn.current = t >= 1 ? value : from + (value - from) * easeOut(t);
            setShown(drawn.current);
            if (t < 1) raf = requestAnimationFrame(step);
        };
        raf = requestAnimationFrame(step);
        return () => cancelAnimationFrame(raf);
    }, [value]);
    return shown;
}

/**
 * A figure that counts up to `value`. While it moves, the changing text is hidden from assistive
 * technology and the final figure sits in a visually hidden span, so a reader never hears the
 * intermediate numbers; at rest it is plain text.
 */
export function CountUp({
    value,
    format = (n) => String(n),
}: {
    value: number;
    format?(n: number): string;
}) {
    const shown = useCountUp(value);
    if (shown === value) return <>{format(value)}</>;
    return (
        <>
            <span aria-hidden="true">{format(Math.round(shown))}</span>
            <span className="sr-only">{format(value)}</span>
        </>
    );
}
