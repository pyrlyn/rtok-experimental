import type { ReactNode } from "react";
import type {
    AgentModules,
    InstructionRow,
    ModuleState,
    ProjectReport,
    Report,
    Snapshot,
} from "../api/snapshot.gen";
import { Kpi } from "../ui/Kpi";
import { Panel } from "../ui/Panel";
import { Pill } from "../ui/Pill";
import { compact, fmt, pct } from "./format";
import { doctorChecks, MODULES, savingText, type CheckState } from "./model";
import { DoctorFix } from "./DoctorFix";
import { CheckPill, Kv, WithSnapshot } from "./parts";

const SUMMARY = [
    ["pass", "pass", "ok"],
    ["warn", "warn", "warn"],
    ["fail", "fail", "fail"],
    ["skip", "not set", "default"],
] as const satisfies readonly (readonly [CheckState, string, "ok" | "warn" | "fail" | "default"])[];

const none = <span className="text-xs text-fg-muted">not set</span>;
const list = "divide-y divide-border/60 -m-4";
const row = "flex items-start gap-3 px-4 py-2.5";

export function Doctor() {
    return <WithSnapshot>{(snap) => <DoctorBody snap={snap} />}</WithSnapshot>;
}

function DoctorBody({ snap }: { snap: Snapshot }) {
    const d = snap.doctor;
    const checks = doctorChecks(d, snap.doctor_pending);
    const checksPanel = (
        <Panel
            title="checks"
            hint="derived from the doctor report; it has no verdict of its own"
            className="xl:col-span-7"
        >
            <ul className={list}>
                {checks.map((c) => (
                    <li key={c.label + c.detail} className={row}>
                        <span className="mt-0.5 w-14 shrink-0">
                            <CheckPill state={c.st} />
                        </span>
                        <div className="min-w-0 flex-1">
                            <div className="text-xs font-semibold">{c.label}</div>
                            <div className="text-2xs break-words text-fg-muted">{c.detail}</div>
                        </div>
                        <code className="hidden shrink-0 text-2xs text-fg-muted sm:block">
                            {c.field}
                        </code>
                    </li>
                ))}
            </ul>
        </Panel>
    );
    return (
        <div className="flex flex-col gap-3">
            <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
                {SUMMARY.map(([st, label, tone]) => (
                    <Kpi
                        key={st}
                        label={label}
                        value={checks.filter((c) => c.st === st).length}
                        tone={tone}
                    />
                ))}
            </div>
            <div className="grid grid-cols-1 gap-3 xl:grid-cols-12">
                {checksPanel}
                {d && <Details d={d} />}
            </div>
            <DoctorFix />
        </div>
    );
}

function Details({ d }: { d: Report }) {
    const events = Object.entries(d.hooks_by_event);
    const top = Math.max(1, ...events.map(([, n]) => n));
    const rs = d.read_share;
    return (
        <>
            <div className="flex flex-col gap-3 xl:col-span-5">
                <Panel title="hooks" hint={`${d.hooks_total} total`}>
                    {events.length ? (
                        <ul className="flex flex-col gap-1.5">
                            {events.map(([name, n]) => (
                                <li key={name} className="flex items-center gap-2 text-xs">
                                    <span className="w-36 truncate text-fg-muted">{name}</span>
                                    <div className="h-1.5 flex-1 rounded-full bg-surface-3">
                                        <div
                                            className="h-full rounded-full bg-accent-fg"
                                            style={{ width: `${(n / top) * 100}%` }}
                                        />
                                    </div>
                                    <span className="w-6 text-right">{n}</span>
                                </li>
                            ))}
                        </ul>
                    ) : (
                        <p className="text-xs text-fg-muted">No hooks installed.</p>
                    )}
                </Panel>
                <Panel title="proxy chains">
                    <Chain label="anthropic" chain={d.proxy} />
                    <Chain label="openai" chain={d.proxy_openai} />
                    {d.mcp_tool_search_disabled && (
                        <p className="text-2xs text-warn-fg">
                            mcp_tool_search {d.mcp_tool_search.state} (
                            {d.mcp_tool_search.source ?? "default"})
                        </p>
                    )}
                </Panel>
            </div>
            <Panel title="MCP servers" hint={`${d.mcp.length} probed`} className="xl:col-span-7">
                {d.mcp.length ? (
                    <ul className={list}>
                        {d.mcp.map((s) => (
                            <li key={s.name} className="px-4 py-2">
                                <div className="flex items-baseline gap-2">
                                    <span className="text-xs font-semibold">{s.name}</span>
                                    <span className="ml-auto text-2xs text-fg-muted">
                                        {s.tools} tools · ~{fmt(s.desc_tokens)} desc tok
                                    </span>
                                </div>
                                <div className="text-2xs break-all text-fg-muted">{s.cmd}</div>
                            </li>
                        ))}
                    </ul>
                ) : (
                    <p className="text-xs text-fg-muted">No MCP server probed.</p>
                )}
            </Panel>
            <Panel title="environment" className="xl:col-span-5">
                <Kv
                    rows={[
                        ["BASH_MAX_OUTPUT_LENGTH", d.bash_max_output_length ?? "(unset)"],
                        ["autoCompactWindow", d.auto_compact_window ?? "(unset)"],
                        [
                            "read share",
                            rs ? (
                                <>
                                    {pct(rs.share)}{" "}
                                    <span className="text-fg-subtle">
                                        grep {compact(rs.grep_tokens)} · glob{" "}
                                        {compact(rs.glob_tokens)} · read {compact(rs.read_tokens)}
                                    </span>
                                </>
                            ) : (
                                "-"
                            ),
                        ],
                        [
                            "skills desc bytes",
                            d.skills ? (
                                <>
                                    {fmt(d.skills.desc_bytes)}{" "}
                                    <span className="text-fg-subtle">
                                        ≈ {fmt(Math.round(d.skills.desc_bytes / 4))} tok per request
                                    </span>
                                </>
                            ) : (
                                "-"
                            ),
                        ],
                    ]}
                />
            </Panel>
            {d.instructions && (
                <Panel title="instructions" className="xl:col-span-7">
                    <ul className={list}>
                        <InstructionItems rows={d.instructions.rows} />
                    </ul>
                </Panel>
            )}
            <Panel title="agents × modules" className="xl:col-span-5">
                <Agents agents={d.agents} />
            </Panel>
            {d.project && <Project p={d.project} />}
        </>
    );
}

function InstructionItems({ rows }: { rows: InstructionRow[] }) {
    return rows.map((r) => (
        <li key={r.path} className="px-4 py-2">
            <div className="flex items-center gap-2">
                <span className="text-xs font-semibold">{r.name}</span>
                {r.warn && <Pill tone="warn">warn</Pill>}
                <span className="ml-auto text-2xs text-fg-muted">{fmt(r.tokens)} tok</span>
            </div>
            <div className="text-2xs break-all text-fg-muted">{r.path}</div>
        </li>
    ));
}

const inner = "divide-y divide-border/60";
const sub = "mb-1.5 text-2xs font-semibold tracking-wide text-fg-muted uppercase";

function Project({ p }: { p: ProjectReport }) {
    const days = p.window_days;
    const g = p.graph;
    return (
        <Panel title="project" hint={p.root} className="xl:col-span-12">
            <section aria-label="languages">
                <h3 className={sub}>languages</h3>
                {p.languages.length ? (
                    <ul className={inner}>
                        {p.languages.map((l) => (
                            <li key={l.language} className="flex flex-col gap-0.5 py-2">
                                <div className="flex flex-wrap items-center gap-2 text-xs">
                                    <span className="font-semibold">{l.language}</span>
                                    <span className="text-fg-muted">
                                        {fmt(l.files)} files{l.marker && ` · ${l.marker}`}
                                    </span>
                                    {l.server && (
                                        <Pill tone={l.installed ? "ok" : "warn"}>
                                            {l.server} {l.installed ? "installed" : "missing"}
                                        </Pill>
                                    )}
                                </div>
                                {l.install && (
                                    <code className="text-2xs break-all">install: {l.install}</code>
                                )}
                                {l.note && <div className="text-2xs text-fg-muted">{l.note}</div>}
                                {l.server && (
                                    <div className="text-2xs text-fg-muted">
                                        language server saving: {savingText(l.saving, days)}
                                    </div>
                                )}
                            </li>
                        ))}
                    </ul>
                ) : (
                    <p className="text-xs text-fg-muted">None found.</p>
                )}
                {p.walk_cut && (
                    <p className="text-2xs text-warn-fg">
                        File counts are a lower bound: the walk stopped at its cap.
                    </p>
                )}
            </section>
            {g && (
                <section aria-label="graph">
                    <h3 className={sub}>graph</h3>
                    <Kv
                        rows={[
                            [
                                "index",
                                `${g.registered ? "registered" : "not registered"}, ${
                                    g.state ?? (g.indexed ? "indexed" : "not indexed")
                                }${g.files != null ? `, ${fmt(g.files)} files` : ""}`,
                            ],
                            ["health", g.score != null ? `${g.score}/100` : "-"],
                            [
                                "backend",
                                g.served_by ? `${g.backend} (served by ${g.served_by})` : g.backend,
                            ],
                        ]}
                    />
                    {g.reasons.map((r) => (
                        <div key={r.text} className="text-2xs break-all text-warn-fg">
                            {r.text} <span className="text-fg-muted">(fix: {r.fix})</span>
                        </div>
                    ))}
                    {g.next.map((n) => (
                        <div key={n} className="text-2xs break-all text-fg-muted">
                            next: {n}
                        </div>
                    ))}
                </section>
            )}
            <section aria-label="project instructions">
                <h3 className={sub}>instructions</h3>
                {p.instructions.rows.length ? (
                    <ul className={inner}>
                        <InstructionItems rows={p.instructions.rows} />
                    </ul>
                ) : (
                    <p className="text-xs text-fg-muted">No AGENTS.md or CLAUDE.md.</p>
                )}
            </section>
            <section aria-label="plugins off here">
                <h3 className={sub}>plugins off here</h3>
                {p.plugins.length ? (
                    <ul className="flex flex-col gap-1.5">
                        {p.plugins.map((x) => (
                            <li key={x.plugin} className="text-xs">
                                <span className="font-semibold">{x.plugin}</span>{" "}
                                <span className="text-2xs text-fg-muted">
                                    set <code>[plugins.{x.plugin}] enabled = true</code> in{" "}
                                    {x.where} · saving: {savingText(x.saving, days)}
                                </span>
                            </li>
                        ))}
                    </ul>
                ) : (
                    <p className="text-xs text-fg-muted">None.</p>
                )}
            </section>
        </Panel>
    );
}

function Chain({ label, chain }: { label: string; chain: string }) {
    const hops = chain ? chain.split("→").map((h) => h.trim()) : [];
    return (
        <div className="flex flex-wrap items-center gap-1.5 text-xs">
            <span className="w-16 text-fg-muted">{label}</span>
            {hops.length
                ? hops.map((h, i) => (
                      <span key={h + i} className="flex items-center gap-1.5">
                          {i > 0 && <span aria-label="to">→</span>}
                          <Pill tone="info">{h}</Pill>
                      </span>
                  ))
                : none}
        </div>
    );
}

const moduleTone: Record<ModuleState, [ReactNode, "ok" | "warn" | "muted"]> = {
    installed: ["on", "ok"],
    not_installed: ["off", "warn"],
    not_supported: ["n/a", "muted"],
};

function Agents({ agents }: { agents: AgentModules[] }) {
    return (
        <ul className="flex flex-col gap-3">
            {agents.map((a) => (
                <li key={a.host + a.kind}>
                    <div className="mb-1.5 text-xs font-semibold">
                        {a.host} <span className="font-normal text-fg-muted">{a.kind}</span>
                    </div>
                    <div className="grid grid-cols-2 gap-1.5 sm:grid-cols-4">
                        {MODULES.map((m) => {
                            const r = a.modules.find((x) => x.name === m);
                            const [text, tone] = r ? moduleTone[r.state] : ["-", "muted" as const];
                            return (
                                <div
                                    key={m}
                                    className="rounded-md border border-border bg-bg/60 px-2 py-1"
                                    title={r?.note}
                                >
                                    <div className="flex items-center justify-between gap-1">
                                        <span className="text-2xs text-fg-muted">{m}</span>
                                        <Pill tone={tone}>{text}</Pill>
                                    </div>
                                    {r && r.state !== "installed" && r.note && (
                                        <div className="mt-1 truncate text-2xs text-fg-muted">
                                            {r.note}
                                        </div>
                                    )}
                                </div>
                            );
                        })}
                    </div>
                </li>
            ))}
        </ul>
    );
}
