// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import {
    type RefObject,
    useEffect,
    useImperativeHandle,
    useReducer,
    useRef,
    useState,
} from "react";
import { accentOf, type Lit, type LiveState } from "./live/lit";
import { edgeHow, type Scene, type SceneNode } from "./scene";
import type { Positions, Vec3 } from "./useLayout";
import type { ViewApi, ViewEvents } from "./webgl";

export interface View2DProps extends ViewEvents {
    scene: Scene;
    positions: Positions;
    api: RefObject<ViewApi | null>;
    /** The read-only live picture (T329.26): no input of any kind, and the nodes carry the call state. */
    live?: LiveState;
}

interface Box {
    x: number;
    y: number;
    w: number;
    h: number;
}

const PAD = 30;

function fitBox(scene: Scene, map: Map<number, Vec3>): Box {
    const pts = scene.nodes.flatMap((n) => {
        const p = map.get(n.id);
        return p ? [{ p, r: n.radius }] : [];
    });
    if (!pts.length) return { x: -100, y: -100, w: 200, h: 200 };
    const x0 = Math.min(...pts.map(({ p, r }) => p[0] - r));
    const x1 = Math.max(...pts.map(({ p, r }) => p[0] + r));
    const y0 = Math.min(...pts.map(({ p, r }) => p[1] - r));
    const y1 = Math.max(...pts.map(({ p, r }) => p[1] + r));
    return { x: x0 - PAD, y: y0 - PAD, w: x1 - x0 + 2 * PAD, h: y1 - y0 + 2 * PAD };
}

/** One mark per shape, so a file, a type and a function read apart without colour. */
function Mark({ n }: { n: SceneNode }) {
    const r = n.radius;
    const paint = {
        // `style`, not attributes: only CSS resolves the role's `var()`.
        style: { fill: n.hollow ? "none" : n.color, stroke: n.color },
        strokeWidth: n.hollow ? 1.5 : 0,
        strokeDasharray: n.hollow ? "3 2" : undefined,
    };
    if (n.shape === "cube") return <rect x={-r} y={-r} width={2 * r} height={2 * r} {...paint} />;
    if (n.shape === "octahedron") {
        const d = r * 1.3;
        return <polygon points={`0,${-d} ${d},0 0,${d} ${-d},0`} {...paint} />;
    }
    return <circle r={r} {...paint} />;
}

/** Heat is a soft halo, each running call a pulsing ring in its own accent, a failure a red ring. */
function Glow({ n, lit }: { n: SceneNode; lit: Lit }) {
    const pulse = "motion-safe:animate-pulse";
    return (
        <>
            {lit.heat > 0 && (
                <circle
                    r={n.radius * (1.4 + lit.heat)}
                    opacity={0.15 + 0.45 * lit.heat}
                    style={{ fill: n.color }}
                    data-testid="heat"
                />
            )}
            {lit.accents.map((slot, i) => (
                <circle
                    key={slot}
                    r={n.radius * (1.5 + 0.3 * i)}
                    fill="none"
                    strokeWidth={1.5}
                    strokeDasharray={accentOf(slot).dash}
                    style={{ stroke: accentOf(slot).color }}
                    className={pulse}
                    data-testid="accent"
                />
            ))}
            {lit.failed && (
                <circle
                    r={n.radius * 1.8}
                    fill="none"
                    strokeWidth={1.5}
                    style={{ stroke: "var(--pyr-danger-fg)" }}
                    className={pulse}
                    data-testid="failed"
                />
            )}
        </>
    );
}

/** The same scene flat: SVG over the layout's x and y. Pan by dragging, zoom with the wheel. */
export default function Scene2D({
    scene,
    positions,
    api,
    select,
    menu,
    hover,
    open,
    live,
}: View2DProps) {
    const readOnly = live !== undefined;
    const [, redraw] = useReducer((n: number) => n + 1, 0);
    // `null` follows the layout: the box fits whatever is drawn.
    const [manual, setManual] = useState<Box | null>(null);
    const svg = useRef<SVGSVGElement>(null);
    const drag = useRef<{ x: number; y: number; box: Box; moved: boolean } | null>(null);
    useEffect(() => positions.subscribe(redraw), [positions]);

    const at = (id: number) => positions.map.get(id);
    const byId = new Map(scene.nodes.map((n) => [n.id, n]));
    const fitted = fitBox(scene, positions.map);
    const box = manual ?? fitted;

    useImperativeHandle(api, () => ({
        fit: () => setManual(null),
        reset: () => setManual(null),
        focus: (id) => {
            const p = positions.map.get(id);
            if (p) setManual({ x: p[0] - 50, y: p[1] - 50, w: 100, h: 100 });
        },
        screenOf: (id) => {
            const p = positions.map.get(id);
            const el = svg.current;
            const m = el?.getScreenCTM();
            if (!p || !el || !m) return null;
            const pt = new DOMPoint(p[0], p[1]).matrixTransform(m);
            return { x: pt.x, y: pt.y };
        },
    }));

    const toWorld = (e: { clientX: number; clientY: number }) => {
        const m = svg.current!.getScreenCTM()!.inverse();
        return new DOMPoint(e.clientX, e.clientY).matrixTransform(m);
    };
    const onWheel = (e: React.WheelEvent) => {
        const w = toWorld(e);
        const k = e.deltaY > 0 ? 1.15 : 1 / 1.15;
        setManual({
            x: w.x - (w.x - box.x) * k,
            y: w.y - (w.y - box.y) * k,
            w: box.w * k,
            h: box.h * k,
        });
    };
    const onPointerDown = (e: React.PointerEvent) => {
        drag.current = { x: e.clientX, y: e.clientY, box, moved: false };
    };
    const onPointerMove = (e: React.PointerEvent) => {
        const d = drag.current;
        if (!d) return;
        const m = svg.current!.getScreenCTM()!;
        const dx = (e.clientX - d.x) / m.a;
        const dy = (e.clientY - d.y) / m.d;
        if (Math.hypot(e.clientX - d.x, e.clientY - d.y) > 4) d.moved = true;
        if (d.moved) setManual({ ...d.box, x: d.box.x - dx, y: d.box.y - dy });
    };

    // The live picture takes no input at all, so no handler is attached and the browser keeps
    // the default cursor; `pointer-events-none` also covers every child.
    const camera = readOnly
        ? {}
        : {
              onWheel,
              onPointerDown,
              onPointerMove,
              onPointerUp: () => (drag.current = null),
              onPointerLeave: () => {
                  drag.current = null;
                  hover(null);
              },
          };

    return (
        <svg
            ref={svg}
            data-testid={readOnly ? "graph-live" : "graph-2d"}
            role={readOnly ? "img" : "group"}
            aria-label={`${readOnly ? "Live " : ""}2D graph of the ${scene.label}`}
            viewBox={`${box.x} ${box.y} ${box.w} ${box.h}`}
            className={`size-full min-h-72 touch-none text-fg ${readOnly ? "pointer-events-none" : ""}`}
            {...camera}
        >
            <defs>
                <marker
                    id="arrow-2d"
                    viewBox="0 0 10 10"
                    refX="9"
                    refY="5"
                    markerWidth="6"
                    markerHeight="6"
                    orient="auto-start-reverse"
                >
                    <path d="M0 0L10 5L0 10z" fill="currentColor" />
                </marker>
            </defs>
            {scene.edges.map((e) => {
                const a = at(e.from);
                const b = at(e.to);
                if (!a || !b) return null;
                const to = byId.get(e.to)!;
                const len = Math.hypot(b[0] - a[0], b[1] - a[1]) || 1;
                const k = (len - to.radius - 3) / len;
                const how = edgeHow(e);
                return (
                    <line
                        key={e.id}
                        x1={a[0]}
                        y1={a[1]}
                        x2={a[0] + (b[0] - a[0]) * k}
                        y2={a[1] + (b[1] - a[1]) * k}
                        stroke="currentColor"
                        strokeWidth={e.width * 0.5}
                        strokeDasharray={e.dashed ? "4 3" : undefined}
                        opacity={e.inScope ? 0.8 : 0.3}
                        markerEnd="url(#arrow-2d)"
                        data-testid={readOnly ? "edge-live" : "edge-2d"}
                        onPointerEnter={(ev) =>
                            hover({
                                text: `${byId.get(e.from)?.label} → ${to.label} (${how})`,
                                x: ev.clientX,
                                y: ev.clientY,
                            })
                        }
                        onPointerLeave={() => hover(null)}
                    >
                        <title>{how}</title>
                    </line>
                );
            })}
            {scene.nodes.map((n) => {
                const p = at(n.id);
                if (!p) return null;
                const input = readOnly
                    ? { "aria-hidden": true }
                    : {
                          role: "button",
                          tabIndex: 0,
                          "aria-label": `${n.label}, ${n.state}${n.selected ? ", selected" : ""}`,
                          "aria-pressed": n.selected,
                          className:
                              "cursor-pointer outline-none focus-visible:[&>:first-child]:stroke-accent",
                          onPointerDown: (e: React.PointerEvent) => e.stopPropagation(),
                          onClick: () => select(n.id),
                          onDoubleClick: () => (open ? open(n.id) : api.current?.focus(n.id)),
                          onKeyDown: (e: React.KeyboardEvent) =>
                              (e.key === "Enter" || e.key === " ") &&
                              (e.preventDefault(), select(n.id)),
                          onContextMenu: (e: React.MouseEvent) => (
                              e.preventDefault(),
                              menu(n.id, e.clientX, e.clientY)
                          ),
                          onPointerEnter: (e: React.PointerEvent) =>
                              hover({
                                  text: `${n.label} · ${n.state}`,
                                  x: e.clientX,
                                  y: e.clientY,
                              }),
                          onPointerLeave: () => hover(null),
                      };
                const l = live?.lit.get(n.id);
                return (
                    <g
                        key={n.id}
                        transform={`translate(${p[0]} ${p[1]})`}
                        opacity={n.dim ? 0.35 : 1}
                        data-testid={readOnly ? "node-live" : "node-2d"}
                        data-hot={l && l.heat > 0 ? "" : undefined}
                        data-running={l?.accents.length ? "" : undefined}
                        {...input}
                    >
                        {l && <Glow n={n} lit={l} />}
                        <Mark n={n} />
                        {n.selected && (
                            <circle
                                r={n.radius * 1.5}
                                fill="none"
                                stroke="currentColor"
                                strokeWidth={1}
                            />
                        )}
                        <text
                            y={-n.radius - 3}
                            textAnchor="middle"
                            fontSize={7}
                            fill="currentColor"
                            className="pointer-events-none select-none font-mono"
                        >
                            {n.label}
                        </text>
                    </g>
                );
            })}
        </svg>
    );
}
