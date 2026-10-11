// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { useMemo, useState } from "react";
import type { TaskBoard, TaskCard, TaskColumn, TasksPage } from "../api/snapshot.gen";
import { Empty } from "../states";
import { readStored, writeStored } from "../storage";
import { Chip } from "../ui/Chip";
import { DataTable, type Column } from "../ui/DataTable";
import { Panel } from "../ui/Panel";
import { Pill, type PillTone } from "../ui/Pill";
import { Search } from "../ui/Search";
import { Count, Kv, responsive, Split, Toolbar, useMinWidth, WithSnapshot } from "./parts";

/** The board's columns, left to right: the statuses of `rtok task`. */
export const COLUMNS: readonly { id: TaskColumn; title: string }[] = [
    { id: "open", title: "open" },
    { id: "in-progress", title: "in progress" },
    { id: "done", title: "done" },
    { id: "closed", title: "closed" },
];

/** `rtok task`'s default priority, which the CLI leaves out of its output too. */
const DEFAULT_PRIORITY = 2;

const VIEW_KEY = "rtok-tasks-view";
const VIEWS = ["board", "list"] as const;
type View = (typeof VIEWS)[number];

const STATUS_TONE: Record<TaskColumn, PillTone> = {
    open: "muted",
    "in-progress": "info",
    done: "ok",
    closed: "muted",
};

const statusTitle = (id: TaskColumn) => COLUMNS.find((c) => c.id === id)?.title ?? id;

export interface Marker {
    key: string;
    tone: PillTone;
    label: string;
}

/** What a card says besides its title. Quiet by default: a plain task shows none. */
export function markers(t: TaskCard): Marker[] {
    const out: Marker[] = [];
    if (t.priority !== DEFAULT_PRIORITY) {
        out.push({
            key: "priority",
            tone: t.priority === 0 ? "fail" : t.priority === 1 ? "warn" : "muted",
            label: `P${t.priority}`,
        });
    }
    if (t.blocked_by.length > 0)
        out.push({ key: "needs", tone: "info", label: `needs ${t.blocked_by.join(", ")}` });
    if (t.conflicts_with.length > 0)
        out.push({
            key: "conflicts",
            tone: "warn",
            label: `conflicts ${t.conflicts_with.join(", ")}`,
        });
    if (t.parallel) out.push({ key: "parallel", tone: "ok", label: "parallel" });
    return out;
}

/** The cards that match the search box, in `column`, most urgent first (ids are already in order). */
export function cardsIn(tasks: readonly TaskCard[], column: TaskColumn, query: string): TaskCard[] {
    const q = query.trim().toLowerCase();
    return tasks
        .filter(
            (t) =>
                t.status === column &&
                (q === "" ||
                    [t.id, t.title, t.assignee, t.parent].some((v) =>
                        v?.toLowerCase().includes(q),
                    )),
        )
        .sort((a, b) => a.priority - b.priority);
}

export function Tasks() {
    return <WithSnapshot>{(snap) => <TasksBody page={snap.tasks} />}</WithSnapshot>;
}

function TasksBody({ page }: { page: TasksPage }) {
    if (page.state === "unconfigured") {
        return (
            <Empty
                title="No tasks configured"
                hint="This project has no [tasks] section. Run `rtok task init` in it to start one."
            />
        );
    }
    if (!page.board) {
        // The server's own sentence: it is reading, or says why it could not.
        return (
            <Panel title="tasks" hint="rtok task list --all">
                <p
                    role={page.state === "reading" ? "status" : "alert"}
                    className="text-xs text-fg-muted"
                >
                    {page.text.trim()}
                </p>
            </Panel>
        );
    }
    return <Tasked board={page.board} />;
}

function Tasked({ board }: { board: TaskBoard }) {
    const [view, setView] = useState<View>(() => {
        const stored = readStored(VIEW_KEY);
        return VIEWS.find((v) => v === stored) ?? "board";
    });
    const [query, setQuery] = useState("");
    const choose = (next: View) => {
        setView(next);
        writeStored(VIEW_KEY, next);
    };
    const shown = useMemo(
        () => COLUMNS.reduce((n, c) => n + cardsIn(board.tasks, c.id, query).length, 0),
        [board.tasks, query],
    );

    return (
        <div className="flex flex-col gap-3">
            <Toolbar>
                <div className="w-56 max-w-full">
                    <Search
                        label="Filter tasks"
                        placeholder="id, title, agent"
                        value={query}
                        onChange={setQuery}
                    />
                </div>
                <div role="group" aria-label="View" className="flex flex-wrap gap-1.5">
                    {VIEWS.map((v) => (
                        <Chip key={v} pressed={view === v} onPressedChange={() => choose(v)}>
                            {v}
                        </Chip>
                    ))}
                </div>
                <Count>
                    {shown} of {board.tasks.length} · {board.adapter}
                </Count>
            </Toolbar>
            {board.tasks.length === 0 ? (
                <Empty title="No tasks yet" hint='Add one with `rtok task create "<title>"`.' />
            ) : view === "board" ? (
                <Columns board={board} query={query} />
            ) : (
                <List board={board} query={query} />
            )}
            {board.hidden > 0 && (
                <p className="text-2xs text-fg-subtle">
                    {board.hidden} older done or closed tasks are not shown; `rtok task list --all`
                    has them.
                </p>
            )}
        </div>
    );
}

function Columns({ board, query }: { board: TaskBoard; query: string }) {
    return (
        <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-4">
            {COLUMNS.map(({ id, title }) => {
                const cards = cardsIn(board.tasks, id, query);
                return (
                    <section
                        key={id}
                        aria-label={title}
                        className="flex min-w-0 flex-col gap-2 rounded-lg border border-border bg-surface-2/40 p-2.5"
                    >
                        <header className="flex items-center gap-2 px-1">
                            <h2 className="text-xs font-bold">{title}</h2>
                            <Pill tone={STATUS_TONE[id]}>{cards.length}</Pill>
                        </header>
                        {cards.length === 0 ? (
                            <p className="px-1 py-2 text-2xs text-fg-subtle">none</p>
                        ) : (
                            <ul className="flex flex-col gap-2">
                                {cards.map((t) => (
                                    <Card key={t.id} task={t} />
                                ))}
                            </ul>
                        )}
                    </section>
                );
            })}
        </div>
    );
}

function Card({ task: t }: { task: TaskCard }) {
    const marks = markers(t);
    return (
        <li className="flex min-w-0 flex-col gap-1.5 rounded-md border border-border bg-surface p-2.5 text-xs">
            <div className="flex min-w-0 items-baseline gap-2">
                <b className="shrink-0 font-mono text-2xs text-accent-fg">{t.id}</b>
                <span className="min-w-0 break-words">{t.title}</span>
            </div>
            {(t.parent || t.assignee || t.external) && (
                <p className="flex flex-wrap gap-x-3 gap-y-0.5 text-2xs text-fg-muted">
                    {t.parent && <span>in {t.parent}</span>}
                    {t.assignee && <span>{t.assignee}</span>}
                    <Issue task={t} />
                </p>
            )}
            {marks.length > 0 && (
                <div aria-label="markers" role="group" className="flex flex-wrap gap-1">
                    {marks.map((m) => (
                        <Pill key={m.key} tone={m.tone}>
                            {m.label}
                        </Pill>
                    ))}
                </div>
            )}
        </li>
    );
}

/** The issue on GitHub or GitLab. The server only sends http(s) links, so the href is safe to follow. */
function Issue({ task: t }: { task: TaskCard }) {
    if (!t.external) return null;
    return (
        <a
            href={t.external.url}
            target="_blank"
            rel="noreferrer noopener"
            className="text-accent-fg hover:underline"
        >
            {t.external.adapter} #{t.external.number}
        </a>
    );
}

function List({ board, query }: { board: TaskBoard; query: string }) {
    const [selectedId, setSelectedId] = useState<string>();
    const rows = useMemo(
        () => COLUMNS.flatMap((c) => cardsIn(board.tasks, c.id, query)),
        [board.tasks, query],
    );
    const selected = rows.find((t) => t.id === selectedId) ?? rows[0];
    const wide = useMinWidth(768);
    const full = useMemo<Column<TaskCard>[]>(
        () => [
            {
                id: "id",
                header: "id",
                width: "72px",
                cell: (t) => <b className="font-mono text-accent-fg">{t.id}</b>,
            },
            { id: "title", header: "title", cell: (t) => t.title },
            {
                id: "status",
                header: "status",
                width: "104px",
                cell: (t) => <Pill tone={STATUS_TONE[t.status]}>{statusTitle(t.status)}</Pill>,
            },
            {
                id: "priority",
                header: "prio",
                width: "48px",
                align: "right",
                cell: (t) => `P${t.priority}`,
            },
            {
                id: "agent",
                header: "agent",
                width: "120px",
                cell: (t) => <span className="text-fg-muted">{t.assignee ?? "-"}</span>,
            },
        ],
        [],
    );
    const columns = responsive(full, wide, ["id", "title", "status"]);

    return (
        <Split
            list={
                <Panel title="tasks" hint="Enter opens a task's details">
                    <DataTable
                        label="tasks"
                        rows={rows}
                        columns={columns}
                        getRowId={(t) => t.id}
                        height={480}
                        selectedId={selected?.id}
                        onSelect={(t) => setSelectedId(t.id)}
                        empty={<Empty title="No task matches" hint="Clear the filter." />}
                    />
                </Panel>
            }
            detail={selected && <Detail task={selected} />}
        />
    );
}

function Detail({ task: t }: { task: TaskCard }) {
    return (
        <Panel
            title={t.id}
            action={<Pill tone={STATUS_TONE[t.status]}>{statusTitle(t.status)}</Pill>}
        >
            <p className="text-sm break-words">{t.title}</p>
            <Kv
                rows={[
                    ["parent", t.parent ?? "none"],
                    ["priority", `P${t.priority}`],
                    ["agent", t.assignee ?? "none"],
                    ["needs", t.blocked_by.join(", ") || "none"],
                    ["conflicts", t.conflicts_with.join(", ") || "none"],
                    ["parallel", t.parallel ? "yes" : "no"],
                    ["issue", t.external ? <Issue key="issue" task={t} /> : "none"],
                ]}
            />
        </Panel>
    );
}
