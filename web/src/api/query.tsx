// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

// TanStack Query wiring for the `/ws` stream (T310.3). The server pushes, so the snapshot
// lives in the query cache (written by `setQueryData`, never fetched) and every page reads
// it through `useSnapshot`.
import {
    keepPreviousData,
    QueryClient,
    QueryClientProvider,
    skipToken,
    useMutation,
    useMutationState,
    useQuery,
} from "@tanstack/react-query";
import { createContext, useContext, useEffect, useMemo, useRef, type ReactNode } from "react";
import type {
    CallBatch,
    ClientMessage,
    DrillGraph,
    DrillRequest,
    Fixed,
    Plan,
    ProjectRequest,
    Selection,
    SetRequest,
    Snapshot,
} from "./snapshot.gen";
import type { Connect, Connection, ConnectionState, Frame } from "./ws";

export const snapshotKey = ["snapshot"] as const;
export const connectionKey = ["connection"] as const;
export const pausedKey = ["paused"] as const;

export const EXPAND_TIMEOUT_MS = 10_000;
// A fix writes files and backs each one up first, so it gets longer than a read.
export const DOCTOR_TIMEOUT_MS = 30_000;
// A project select can index the project before the server answers.
export const WRITE_TIMEOUT_MS = 30_000;

export interface Api {
    open(): void;
    close(): void;
    reconnect(): void;
    pause(): void;
    resume(): void;
    expand(id: string): Promise<string>;
    set(request: SetRequest): Promise<void>;
    project(request: ProjectRequest): Promise<void>;
    doctorPlan(selection: Selection): Promise<Plan>;
    doctorApply(selection: Selection): Promise<Fixed>;
    drill(request: DrillRequest): Promise<DrillGraph>;
    /** Starts the graph call stream (T329.26) for as long as a listener is registered; returns the stop. */
    calls(listener: (batch: CallBatch) => void): () => void;
}

// `set` and `project` get no reply of their own: the server answers a write with the next
// snapshot, or with a message when it refuses. So a write is settled by whichever comes first.
interface PendingWrite {
    resolve(): void;
    reject(error: Error): void;
}

interface PendingExpand {
    id: string;
    resolve(text: string): void;
    reject(error: Error): void;
}

// The server answers doctor requests in the order it received them, on one socket, so the
// queue is matched by the kind of frame it waits for. A graph frame names its project and
// not the request, and the server answers those off the executor, so `key` (the project)
// is what ties it back; within one project the order holds.
interface PendingDoctor {
    kind: "doctorplan" | "doctorfixed" | "graph";
    key?: string;
    resolve(frame: Plan | Fixed | DrillGraph): void;
    reject(error: Error): void;
}

export function createApi(
    queryClient: QueryClient,
    connect: Connect,
    expandTimeoutMs = EXPAND_TIMEOUT_MS,
): Api {
    let connection: Connection | null = null;
    let pending: PendingExpand[] = [];
    let pendingDoctor: PendingDoctor[] = [];
    let pendingWrites: PendingWrite[] = [];
    // While paused the socket keeps reading, but frames fold into `held` and the cache keeps the
    // snapshot on screen, so `dataUpdatedAt` stays the age of what the reader sees.
    let paused = false;
    let held: Snapshot | undefined;
    const callListeners = new Set<(batch: CallBatch) => void>();
    const subscribeCalls = (subscribe: boolean) => connection?.send({ calls: { subscribe } });

    const writeSnapshot = (update: (prev: Snapshot | undefined) => Snapshot | undefined) => {
        if (paused) held = update(held ?? queryClient.getQueryData<Snapshot>(snapshotKey));
        else queryClient.setQueryData<Snapshot>(snapshotKey, update);
    };

    const setPaused = (value: boolean) => {
        paused = value;
        queryClient.setQueryData<boolean>(pausedKey, value);
    };

    const rejectAll = (reason: string) => {
        const failed = pending;
        pending = [];
        for (const p of failed) p.reject(new Error(reason));
        const failedDoctor = pendingDoctor;
        pendingDoctor = [];
        for (const p of failedDoctor) p.reject(new Error(reason));
        const failedWrites = pendingWrites;
        pendingWrites = [];
        for (const p of failedWrites) p.reject(new Error(reason));
    };

    const write = (message: ClientMessage) =>
        new Promise<void>((resolve, reject) => {
            const settle = (done: () => void) => () => {
                clearTimeout(timer);
                done();
            };
            const entry: PendingWrite = {
                resolve: settle(resolve),
                reject: (error) => settle(() => reject(error))(),
            };
            const timer = setTimeout(() => {
                pendingWrites = pendingWrites.filter((p) => p !== entry);
                reject(new Error("the server did not answer"));
            }, WRITE_TIMEOUT_MS);
            pendingWrites.push(entry);
            if (!connection?.send(message)) {
                pendingWrites = pendingWrites.filter((p) => p !== entry);
                entry.reject(new Error("not connected"));
            }
        });

    const settleWrites = () => {
        const done = pendingWrites;
        pendingWrites = [];
        for (const p of done) p.resolve();
    };

    const settleDoctor = (
        kind: PendingDoctor["kind"],
        frame: Plan | Fixed | DrillGraph,
        key?: string,
    ) => {
        const i = pendingDoctor.findIndex((p) => p.kind === kind && p.key === key);
        if (i < 0) return;
        const [entry] = pendingDoctor.splice(i, 1);
        entry?.resolve(frame);
    };

    const askDoctor = <T extends Plan | Fixed | DrillGraph>(
        kind: PendingDoctor["kind"],
        message: ClientMessage,
        key?: string,
    ) =>
        new Promise<T>((resolve, reject) => {
            const entry: PendingDoctor = {
                kind,
                key,
                resolve: (frame) => {
                    clearTimeout(timer);
                    resolve(frame as T);
                },
                reject: (error) => {
                    clearTimeout(timer);
                    reject(error);
                },
            };
            const timer = setTimeout(() => {
                pendingDoctor = pendingDoctor.filter((p) => p !== entry);
                reject(new Error(`${kind} timed out`));
            }, DOCTOR_TIMEOUT_MS);
            pendingDoctor.push(entry);
            if (!connection?.send(message)) {
                pendingDoctor = pendingDoctor.filter((p) => p !== entry);
                entry.reject(new Error("not connected"));
            }
        });

    const onFrame = (frame: Frame) => {
        switch (frame.type) {
            case "snapshot":
                // A paused page still settles writes: the server applied them, only the view waits.
                writeSnapshot(() => frame.snapshot);
                settleWrites();
                return;
            case "snapshot_error":
                // A failed tick has no page data; keep the last good one and surface the error.
                writeSnapshot((prev) => prev && { ...prev, error: frame.error });
                settleWrites();
                return;
            case "expand": {
                const done = pending.filter((p) => p.id === frame.id);
                pending = pending.filter((p) => p.id !== frame.id);
                for (const p of done) p.resolve(frame.text);
                return;
            }
            case "doctorplan":
                settleDoctor("doctorplan", frame.plan);
                return;
            case "doctorfixed":
                settleDoctor("doctorfixed", frame.fixed);
                return;
            case "graph":
                settleDoctor("graph", frame.graph, String(frame.graph.project));
                return;
            case "calls":
                for (const listener of callListeners) listener(frame.batch);
                return;
            case "message":
                // The server's refusals do not name the request they answer, so a message fails
                // every request in flight instead of leaving it to the timeout.
                rejectAll(frame.text);
        }
    };

    const onState = (state: ConnectionState) => {
        queryClient.setQueryData<ConnectionState>(connectionKey, state);
        // A new socket starts with no subscription: the stream resumes with the listeners still registered.
        if (state === "open" && callListeners.size) subscribeCalls(true);
        if (state === "closed") rejectAll("connection closed");
    };

    return {
        open() {
            connection ??= connect({ onState, onFrame });
        },
        close() {
            connection?.close();
            connection = null;
        },
        // The socket's own backoff can be seconds away; the offline screen's button skips it.
        reconnect() {
            connection?.close();
            connection = connect({ onState, onFrame });
        },
        // There is nothing to freeze before the first snapshot, and a freeze with no frame would
        // hide the first page behind a pause the reader cannot see.
        pause() {
            if (queryClient.getQueryData(snapshotKey)) setPaused(true);
        },
        resume() {
            setPaused(false);
            const next = held;
            held = undefined;
            if (next) queryClient.setQueryData<Snapshot>(snapshotKey, next);
        },
        expand(id) {
            return new Promise<string>((resolve, reject) => {
                const entry: PendingExpand = {
                    id,
                    resolve: (text) => {
                        clearTimeout(timer);
                        resolve(text);
                    },
                    reject: (error) => {
                        clearTimeout(timer);
                        reject(error);
                    },
                };
                const timer = setTimeout(() => {
                    pending = pending.filter((p) => p !== entry);
                    reject(new Error(`expand ${id} timed out`));
                }, expandTimeoutMs);
                pending.push(entry);
                if (!connection?.send({ expand: id })) {
                    pending = pending.filter((p) => p !== entry);
                    entry.reject(new Error("not connected"));
                }
            });
        },
        set: (request) => write({ set: request }),
        project: (request) => write({ project: request }),
        doctorPlan: (selection) =>
            askDoctor<Plan>("doctorplan", { doctor: { action: "plan", selection } }),
        doctorApply: (selection) =>
            askDoctor<Fixed>("doctorfixed", { doctor: { action: "apply", selection } }),
        // `request.project` is the registry id as text, which is what the frame carries back.
        drill: (request) => askDoctor<DrillGraph>("graph", { graph: request }, request.project),
        calls(listener) {
            callListeners.add(listener);
            if (callListeners.size === 1) subscribeCalls(true);
            return () => {
                if (callListeners.delete(listener) && callListeners.size === 0)
                    subscribeCalls(false);
            };
        },
    };
}

const ApiContext = createContext<Api | null>(null);

export function DataProvider({ connect, children }: { connect: Connect; children: ReactNode }) {
    const queryClient = useMemo(() => new QueryClient(), []);
    const api = useMemo(() => createApi(queryClient, connect), [queryClient, connect]);
    useEffect(() => {
        api.open();
        return () => api.close();
    }, [api]);
    return (
        <QueryClientProvider client={queryClient}>
            <ApiContext value={api}>{children}</ApiContext>
        </QueryClientProvider>
    );
}

function useApi(): Api {
    const api = useContext(ApiContext);
    if (!api) throw new Error("DataProvider is missing");
    return api;
}

// `skipToken` makes these cache-only queries: no fetch, no retry, just what the socket wrote.
const pushed = (queryKey: readonly string[]) =>
    ({ queryKey, queryFn: skipToken, staleTime: Infinity }) as const;

export const useSnapshot = () => useQuery<Snapshot>(pushed(snapshotKey));

export const useConnection = (): ConnectionState =>
    useQuery<ConnectionState>(pushed(connectionKey)).data ?? "connecting";

export const usePaused = (): boolean => useQuery<boolean>(pushed(pausedKey)).data ?? false;

export function usePauseToggle(): (paused: boolean) => void {
    const api = useApi();
    return (paused) => (paused ? api.pause() : api.resume());
}

export function useReconnect(): () => void {
    const api = useApi();
    return () => api.reconnect();
}

// A write is pending from the click until the server answers. `inFlight` lists every request
// still waiting, so each control can show its own spinner while another is being answered;
// `error` is the last failure and clears when the next write starts.
function useWrite<R>(name: string, send: (request: R) => Promise<void>) {
    const { mutate, error } = useMutation({ mutationKey: [name], mutationFn: send });
    const inFlight = useMutationState({
        filters: { mutationKey: [name], status: "pending" },
        select: (m) => m.state.variables as R,
    });
    return { mutate, error, inFlight };
}

export function useSetMutation() {
    const api = useApi();
    return useWrite<SetRequest>("set", (request) => api.set(request));
}

export function useProjectMutation() {
    const api = useApi();
    return useWrite<ProjectRequest>("project", (request) => api.project(request));
}

export function useDoctorApi(): Pick<Api, "doctorPlan" | "doctorApply"> {
    return useApi();
}

/**
 * One drill-down frame (T329.22). `version` is whatever should make the page ask again, such as
 * the project's index numbers; the last frame stays on screen meanwhile so the layout and the
 * zoom survive an update.
 */
export function useDrill(request: DrillRequest, version: readonly unknown[]) {
    const api = useApi();
    return useQuery({
        queryKey: ["drill", request, version],
        queryFn: () => api.drill(request),
        placeholderData: keepPreviousData,
        staleTime: Infinity,
        retry: false,
    });
}

export function useExpandMutation() {
    const api = useApi();
    return useMutation({ mutationFn: (id: string) => api.expand(id) });
}

/** Delivers every call batch while mounted; the socket subscribes only for as long as a page listens. */
export function useCallStream(onBatch: (batch: CallBatch) => void) {
    const api = useApi();
    const latest = useRef(onBatch);
    latest.current = onBatch;
    useEffect(() => api.calls((batch) => latest.current(batch)), [api]);
}
