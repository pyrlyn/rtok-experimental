// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

// Tasks page data (T441.13.7) for `?sample`, stories and tests. Every task is made up; the shape
// is the server's `TasksPage`, whose cards are `rtok task list --all --json` minus the descriptions.
import type { TaskCard, TasksPage } from "../api/snapshot.gen";

const card = (id: string, title: string, rest: Partial<TaskCard> = {}): TaskCard => ({
  id,
  title,
  status: "open",
  parent: null,
  priority: 2,
  assignee: null,
  blocked_by: [],
  conflicts_with: [],
  parallel: false,
  external: null,
  updated_at: 1_790_000_000,
  ...rest,
});

export const tasksFixture: TaskCard[] = [
  card("A1", "Ship the task board", { status: "in-progress", priority: 0, assignee: "agent-1" }),
  card("A1.1", "Read-only API", { status: "done", parent: "A1" }),
  card("A1.2", "Board page", { status: "in-progress", parent: "A1", assignee: "agent-2" }),
  card("A1.3", "Docs in three languages", {
    parent: "A1",
    blocked_by: ["A1.2"],
    parallel: true,
  }),
  card("A2", "Pin the lockfile", { priority: 1, conflicts_with: ["A4"] }),
  card("A3", "Export tasks to GitHub issues", {
    external: { adapter: "github", number: 128, url: "https://github.com/acme/app/issues/128" },
    blocked_by: ["A1", "A2"],
  }),
  card("A4", "Rename the cache directory", { priority: 3, conflicts_with: ["A2"] }),
  card("A5", "A title long enough to wrap onto several lines in a narrow column of the board", {
    parent: "A1",
  }),
  card("A6", "Drop the legacy flag", { status: "closed" }),
  card("A7", "Fix the flaky shard", {
    status: "done",
    priority: 1,
    external: { adapter: "gitlab", number: 9, url: "https://gitlab.com/acme/app/-/issues/9" },
  }),
];

export const tasksReady = (tasks: TaskCard[] = tasksFixture, hidden = 0): TasksPage => ({
  state: "ready",
  text: "",
  board: { adapter: "disk", tasks, hidden },
});

export const tasksUnconfigured: TasksPage = {
  state: "unconfigured",
  text: "no [tasks] section: run `rtok task init` in the project\n",
  board: null,
};

export const tasksReading: TasksPage = { state: "reading", text: "reading tasks…\n", board: null };

export const tasksFailed: TasksPage = {
  state: "failed",
  text: "tasks did not answer: github: HTTP 401\n",
  board: null,
};

export const tasksEmpty: TasksPage = tasksReady([]);
