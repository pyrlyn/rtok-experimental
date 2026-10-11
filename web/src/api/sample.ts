// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

// Offline data source (`?sample`) and the snapshot fixture shared by Storybook, Vitest and
// offline e2e. Typed as `Snapshot`, so a schema change breaks `tsc` here instead of drifting.
import type { Connect, Connection } from "./ws";
import { alertRow, call, plugin, project, stats } from "./sampleRows";
import { applyProject } from "../pages/projectLogic";
import { mockMachine } from "./sampleDoctor";
import { mockJunk, sampleJunkCard } from "./sampleJunk";
import { diffReply } from "./sampleDiff";
import { exportAnswer } from "./sampleExport";
import { drillReply } from "./sampleDrill";
import type { Report, Snapshot } from "./snapshot.gen";
// The text pages have no live source offline, so `?sample` shows the same made-up text the
// page stories use; every value is sample data.
import {
  configText,
  graphText,
  hostsText,
  servicesText,
  skillRows,
  skillsHeader,
  statsText,
  worktreesText,
} from "../pages/textFixtures";
import { savingsDays } from "../pages/savingsFixtures";
import { usageBoth, usagePage } from "../pages/usageFixtures";
import { tasksReady } from "../pages/tasksFixtures";

const shellStats = {
  cache_create: 1_200,
  cache_read: 48_000,
  est_after: 9_000,
  est_before: 31_000,
  input: 5_400,
  output: 2_100,
  rows: 42,
};

// Enough variety for every overview block: a warning, a failing check and a skipped one.
const doctor: Report = {
  agents: [
    {
      host: "claude-code",
      kind: "cli",
      modules: [
        { name: "hooks", note: "", state: "installed" },
        { name: "mcp", note: "", state: "installed" },
        { name: "proxy", note: "rtok agent setup --proxy", state: "not_installed" },
      ],
    },
  ],
  auto_compact_window: null,
  bash_max_output_length: "30000",
  hooks_by_event: { PostToolUse: 1, PreToolUse: 1, SessionStart: 1 },
  hooks_total: 3,
  instructions: {
    duplicates: [],
    rows: [
      { name: "CLAUDE.md", path: "CLAUDE.md", tokens: 1_450, warn: false },
      { name: "AGENTS.md", path: "apps/rtok/AGENTS.md", tokens: 5_200, warn: true },
    ],
  },
  mcp: [{ cmd: "rtok mcp", desc_tokens: 640, name: "rtok", tools: 9 }],
  mcp_tool_search: { source: "heuristic: ANTHROPIC_BASE_URL set", state: "unknown" },
  mcp_tool_search_disabled: true,
  overlaps: ["host-native grep duplicates the rtok search tool ([mcp] search = false)"],
  proxy: "claude-code → rtok proxy → api.anthropic.com",
  proxy_openai: "",
};

const session = "sample-session";

export const sampleSnapshot: Snapshot = {
  type: "snapshot",
  agent_usage: usagePage(usageBoth),
  tasks: tasksReady(),
  calls: [
    call(14, {
      surface: "proxy",
      kind: "request",
      name: "messages",
      plugin: null,
      ms: 1_820,
      model: "claude-sonnet-5-5",
      input: 900,
      cache_read: 40_000,
      output: 300,
      api: "anthropic",
      provider: "anthropic",
      ts: 1_790_000_620,
    }),
    call(13, {
      surface: "mcp",
      kind: "mcp",
      name: "symbol",
      plugin: "graph",
      ms: 31,
      ts: 1_790_000_580,
    }),
    call(12, { ts: 1_790_000_540 }),
    call(11, {
      name: "PreToolUse",
      ok: 0,
      error: "plugin shell panicked: index not built",
      ms: 3_400,
      ts: 1_790_000_500,
    }),
    call(10, {
      surface: "mcp",
      kind: "mcp",
      name: "read",
      plugin: "read",
      ms: 12,
      ts: 1_790_000_460,
    }),
    call(9, { ts: 1_790_000_420 }),
    call(8, {
      surface: "proxy",
      kind: "request",
      name: "messages",
      plugin: null,
      ms: 940,
      model: "claude-sonnet-5-5",
      input: 700,
      cache_read: 31_000,
      output: 210,
      api: "anthropic",
      provider: "anthropic",
      ts: 1_790_000_380,
    }),
    call(7, {
      surface: "mcp",
      kind: "mcp",
      name: "search",
      plugin: "graph",
      ms: 41,
      ts: 1_790_000_300,
    }),
    call(6, { ts: 1_790_000_260 }),
    call(5, { name: "SessionStart", plugin: null, ms: 2, ts: 1_790_000_220 }),
    call(4, {
      surface: "mcp",
      kind: "mcp",
      name: "outline",
      plugin: "read",
      ms: 9,
      ts: 1_790_000_180,
    }),
    call(3, { ts: 1_790_000_140 }),
    {
      api: "anthropic",
      cache_create: 0,
      cache_read: 12_000,
      error: null,
      host: "claude-code",
      id: 2,
      input: 800,
      kind: "hook",
      model: "claude-sonnet-5-5",
      ms: 4,
      name: "PostToolUse",
      ok: 1,
      output: 120,
      parent_id: null,
      plugin: "shell",
      provider: "anthropic",
      session,
      surface: "hook",
      ts: 1_790_000_100,
    },
    call(1, {
      surface: "mcp",
      kind: "mcp",
      name: "read",
      plugin: "read",
      ms: 12,
      ts: 1_790_000_000,
    }),
  ],
  config: configText,
  doctor,
  graph: graphText,
  projects: [
    project(1, "rtok", {
      selected: true,
      backend: {
        backend: "lsp",
        checked_at: 1_790_000_000,
        config: "auto",
        language: "rust",
        next_probe_at: null,
        reason: null,
        server: true,
      },
      links: [{ kind: "manual", name: "ketch", reason: "shared store", to: 2 }],
    }),
    project(2, "ketch", {
      state: "stale",
      index: null,
      backend: {
        backend: "tags",
        checked_at: 1_790_000_000,
        config: "auto",
        language: "rust",
        next_probe_at: 1_790_000_300,
        reason: "rust-analyzer is not installed",
        server: false,
      },
      links: [
        { kind: "manual", name: "docs-site", reason: null, to: 3 },
        ...[4, 5, 6].map((to) => ({
          kind: "auto" as const,
          name: `share-${to - 3}`,
          reason: "shared deps",
          to,
        })),
      ],
    }),
    // One project whose directory is gone, and three on one share that stopped answering: the
    // page shows the second as a single grouped alert.
    project(3, "docs-site", {
      index: null,
      missing: true,
      state: "missing",
      alerts: [alertRow("missing", "docs-site", "the directory no longer exists")],
    }),
    ...[1, 2, 3].map((n) =>
      project(n + 3, `share-${n}`, {
        alerts: [alertRow("unreachable", `share-${n}`, "/Volumes/share did not answer in 2 s")],
      }),
    ),
    project(7, "notes"),
  ],
  hosts: hostsText,
  junk: sampleJunkCard,
  logs: [
    "rtok hook PostToolUse ok 4 ms",
    "2026-10-02 12:03:20 ERROR hook/PreToolUse: plugin shell panicked: index not built",
    "2026-10-02 12:02:40 WARN proxy/messages: slow upstream (1820 ms)",
    "2026-10-02 12:01:05 INFO mcp/read: archived 4 files",
  ],
  plugins: [
    plugin("shell", {
      stats: shellStats,
      summary: "Shrinks noisy shell output.",
      title: "Shell",
    }),
    plugin("read", {
      enabled: false,
      surfaces: ["mcp"],
      summary: "Reads files through the archive.",
      title: "Read",
    }),
    plugin("graph", {
      stats: stats(48_000, 21_000, 19),
      surfaces: ["mcp"],
      summary: "Code graph: symbols, callers and impact.",
    }),
    plugin("archive", {
      stats: stats(14_000, 9_500, 7),
      surfaces: ["hook", "mcp"],
      summary: "Lossless archive behind expand.",
    }),
    plugin("ledger", { saves_tokens: false, surfaces: ["proxy"], summary: "Usage ledger." }),
    plugin("budget", { enabled: false, surfaces: ["proxy"], summary: "Injection budget." }),
  ],
  ref_ids: { "2": "sample-archive-id", "10": "sample-archive-read", "13": "sample-archive-graph" },
  services: servicesText,
  sessions: [
    {
      api: "openai",
      cache_create: 800,
      cache_read: 21_000,
      ended_at: null,
      host: "codex",
      id: "9b2e4d1a-sample",
      input: 2_900,
      last_activity: 1_790_000_620,
      model: "gpt-5.5",
      output: 640,
      project: "ketch",
      provider: "openai",
      started_at: 1_790_000_100,
    },
    {
      api: "anthropic",
      cache_create: 1_200,
      cache_read: 48_000,
      ended_at: null,
      host: "claude-code",
      id: session,
      input: 5_400,
      last_activity: 1_790_000_100,
      model: "claude-sonnet-5-5",
      output: 2_100,
      project: "rtok",
      provider: "anthropic",
      started_at: 1_789_999_000,
    },
    {
      api: "anthropic",
      cache_create: 300,
      cache_read: 9_000,
      ended_at: 1_789_999_900,
      host: "claude-code",
      id: "41c7f0aa-sample",
      input: 1_100,
      last_activity: 1_789_999_900,
      model: "claude-sonnet-5-5",
      output: 400,
      project: "rtok",
      provider: "anthropic",
      started_at: 1_789_999_200,
    },
  ],
  skills: { header: skillsHeader, rows: skillRows },
  stats: statsText,
  usage: {
    cache_create: 1_200,
    cache_read: 48_000,
    ctt: 90_000,
    est_after: 9_000,
    est_before: 31_000,
    input: 5_400,
    output: 2_100,
    rows: 42,
    turns: [4_000, 6_500, 9_000, 7_200, 11_000, 14_000, 12_500, 16_000],
    savings: savingsDays,
  },
  worktrees: worktreesText,
};

export const isSampleRequested = (search: string): boolean =>
  new URLSearchParams(search).has("sample");

export const connectSample: Connect = (handlers) => {
  let snapshot = structuredClone(sampleSnapshot);
  const machine = mockMachine();
  const junk = mockJunk();
  let stopped = false;
  // Frames land on a microtask so callers can register a reply handler after `send`.
  const later = (fn: () => void) =>
    queueMicrotask(() => {
      if (!stopped) fn();
    });
  const emit = () => handlers.onFrame({ type: "snapshot", snapshot: structuredClone(snapshot) });

  handlers.onState("connecting");
  later(() => {
    handlers.onState("open");
    emit();
  });

  const connection: Connection = {
    send(message) {
      if (stopped) return false;
      if ("expand" in message) {
        const { expand: id } = message;
        later(() => handlers.onFrame({ type: "expand", id, text: `sample payload for ${id}` }));
        return true;
      }
      if ("project" in message) {
        snapshot = {
          ...snapshot,
          projects: applyProject(snapshot.projects ?? [], message.project),
        };
        later(emit);
        return true;
      }
      if ("doctor" in message) {
        const { action, selection } = message.doctor;
        later(() =>
          action === "plan"
            ? handlers.onFrame({ type: "doctorplan", plan: machine.plan(selection) })
            : handlers.onFrame({ type: "doctorfixed", fixed: machine.apply(selection) }),
        );
        return true;
      }
      if ("junk" in message) {
        const { action, paths } = message.junk;
        later(() =>
          action === "plan"
            ? handlers.onFrame({ type: "junkplan", plan: junk.plan() })
            : handlers.onFrame({ type: "junkcleared", cleared: junk.apply(paths) }),
        );
        return true;
      }
      if ("calls" in message) {
        // The sample has no other process to watch; the stream starts with the live graph (T329.26).
        return true;
      }
      if ("diff" in message) {
        const { diff } = message;
        later(() => handlers.onFrame(diffReply(diff, snapshot.projects ?? [])));
        return true;
      }
      if ("graph" in message) {
        const { graph } = message;
        later(() => handlers.onFrame(drillReply(graph, snapshot.projects ?? [])));
        return true;
      }
      if ("export" in message || "import" in message) {
        const answer = exportAnswer(message, snapshot.projects ?? []);
        if (answer) later(() => handlers.onFrame(answer));
        return true;
      }
      const { key, value } = message.set;
      const id = /^plugins\.([^.]+)\.enabled$/.exec(key)?.[1];
      const plugin = snapshot.plugins.find((p) => p.id === id);
      if (!plugin) {
        later(() => handlers.onFrame({ type: "message", text: `refused key ${key}` }));
        return true;
      }
      snapshot = {
        ...snapshot,
        plugins: snapshot.plugins.map((p) => (p === plugin ? { ...p, enabled: value } : p)),
      };
      later(emit);
      return true;
    },
    close() {
      stopped = true;
      handlers.onState("closed");
    },
  };
  return connection;
};
