---
name: rtok-tasks
description: Plan and track work as rtok tasks (task_create, task_list, task_next, rtok task), not TODO files, unless told otherwise.
---

# rtok-tasks

When the user asks you to plan, split or track work, keep the tasks in rtok, not in a
TODO file or a hand-numbered list, unless `AGENTS.md` or the user names another tracker.
rtok hands out the ids, one counter per project, so parallel agents never collide.

Tools (MCP, same shapes on the CLI with `--json`):

- `task_create` (`rtok task create "<title>" -d "<why, and what done means>"`): add a task;
  `parent` (`--parent R2`) makes a subtask `R2.1`. It returns the id: quote it, never invent one.
- `task_list` (`rtok task list`): the plan; `all` adds finished tasks.
- `task_get` (`rtok task show <id>`): one task with its description and subtasks.
- `task_status` (`rtok task status <id> in-progress|done|closed`): without a status it only
  reads. Set `in-progress` when you start and `done` when it is finished and checked.
- `task_next` (`rtok task next`): the first ready task. With no blockers, that is still the
  lowest open task with no active subtask.
- `task_ready` (`rtok task ready`): every task that can be claimed, highest priority first.
- `task_claim` (`rtok task claim [id] --agent <id>`): claim a task, or the first ready one
  when the id is omitted. Set it when you start. A second claim by you changes nothing.
- `task_release` (`rtok task release <id>`): give it back. Only the holder, unless `--force`.
- `task_dep` (`rtok task dep <id> <blocker>`): `id` waits until `blocker` is done. A cycle
  is refused and nothing is written.
- `task_conflict` (`rtok task conflict <id> <other> [--remove]`): same files, not at once;
  `task_ready` flags it while the partner runs. Disk adapter only.
- `task_parallel` (`rtok task parallel <id> [--off]`): safe beside any task. Disk only.
- `task_priority` (`rtok task priority <id> <0-4>`): 0 is highest, 2 is the default.

Run `rtok task init` once per project to choose the adapter (disk, GitHub or GitLab) and the id
prefix; without it the defaults apply.

Never edit the task files by hand or reuse an id.

No `rtok`? `ketch install pyrlyn/rtok`.
