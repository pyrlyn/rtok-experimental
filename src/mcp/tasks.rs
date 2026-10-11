// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T441.6: MCP `task_create`, `task_list`, `task_get`, `task_status` and `task_next`, the
//! agent-facing side of `rtok task …`. They call the same `tasks::run` functions as the CLI and
//! answer with the JSON its `--json` prints.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::config::Config;
use crate::plugin::{Runtime, ToolDef};
use crate::tasks::run::{Project, filter, require_agent};
use crate::tasks::{NewTask, Status, TaskId};

pub fn defs() -> [ToolDef; 12] {
    [
        ToolDef {
            name: "task_create",
            description: "Add a task to this project's plan under the next free id (A12, or A12.3 under parent). Returns the task.",
            input_schema: json!({"type":"object","properties":{"title":{"type":"string"},"description":{"type":"string"},"parent":{"type":"string","description":"parent task id, e.g. A12"}},"required":["title"]}),
        },
        ToolDef {
            name: "task_list",
            description: "This project's tasks: active ones by default, subtasks after their parent. status narrows to those statuses (open, in-progress, done, closed); all adds finished ones.",
            input_schema: json!({"type":"object","properties":{"status":{"type":"array","items":{"type":"string"}},"all":{"type":"boolean"},"parent":{"type":"string"}}}),
        },
        ToolDef {
            name: "task_get",
            description: "One task with its description and the ids of its subtasks.",
            input_schema: json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}),
        },
        ToolDef {
            name: "task_status",
            description: "Read a task's status, or set it to open, in-progress, done or closed. Finishing a task with active subtasks is refused unless force.",
            input_schema: json!({"type":"object","properties":{"id":{"type":"string"},"status":{"type":"string"},"force":{"type":"boolean"}},"required":["id"]}),
        },
        ToolDef {
            name: "task_next",
            description: "The first ready task: free or stale, not blocked by an active task. A leaf when the plan has no blockers. null when nothing is ready.",
            input_schema: json!({"type":"object","properties":{}}),
        },
        ToolDef {
            name: "task_ready",
            description: "Tasks that can be claimed, highest priority first (0 before 2). stale is true when the assignee was last seen more than 30 minutes ago.",
            input_schema: json!({"type":"object","properties":{}}),
        },
        ToolDef {
            name: "task_claim",
            description: "Claim a task for this agent (in-progress). Omit id to take the first ready task. changed is false when you already hold it. On GitHub and GitLab the assignee write is last-write-wins, not compare-and-set.",
            input_schema: json!({"type":"object","properties":{"id":{"type":"string"},"agent":{"type":"string","description":"rtok agent id; defaults to RTOK_AGENT_ID"}}}),
        },
        ToolDef {
            name: "task_release",
            description: "Clear the assignee and set the task open. Only the holder, unless force.",
            input_schema: json!({"type":"object","properties":{"id":{"type":"string"},"agent":{"type":"string"},"force":{"type":"boolean"}},"required":["id"]}),
        },
        ToolDef {
            name: "task_dep",
            description: "Record that id waits on blocker. A cycle is refused and nothing is written.",
            input_schema: json!({"type":"object","properties":{"id":{"type":"string"},"blocker":{"type":"string"}},"required":["id","blocker"]}),
        },
        ToolDef {
            name: "task_priority",
            description: "Set priority from 0 (highest) to 4. 2 is the default and is not stored.",
            input_schema: json!({"type":"object","properties":{"id":{"type":"string"},"level":{"type":"integer"}},"required":["id","level"]}),
        },
        ToolDef {
            name: "task_conflict",
            description: "Record that id and other touch the same files, so they should not run at once (symmetric; remove takes it off). task_ready flags a task whose conflict partner is in progress.",
            input_schema: json!({"type":"object","properties":{"id":{"type":"string"},"other":{"type":"string"},"remove":{"type":"boolean"}},"required":["id","other"]}),
        },
        ToolDef {
            name: "task_parallel",
            description: "Mark a task safe to run beside any other task; on false takes the mark off.",
            input_schema: json!({"type":"object","properties":{"id":{"type":"string"},"on":{"type":"boolean","description":"default true"}},"required":["id"]}),
        },
    ]
}

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn id(args: &Value) -> Result<TaskId> {
    match arg(args, "id") {
        Some(id) => id.parse(),
        None => bail!("`id` is required, e.g. A12"),
    }
}

/// The answer of the task tool `name`.
pub fn call(cx: &Runtime, name: &str, args: &Value) -> Result<String> {
    let cwd = std::env::current_dir()?;
    // Read for the cwd now, not at server start: `roots/list` can move the cwd into the
    // project after launch, and `[tasks] prefix` lives in that project's `.rtok.toml`.
    let cfg = Config::load()?;
    let project = Project::open(&cfg.tasks, &cwd)?;
    let out = match name {
        "task_create" => {
            let Some(title) = arg(args, "title") else {
                bail!("`title` is required");
            };
            let new = NewTask {
                title: title.into(),
                description: arg(args, "description").unwrap_or_default().into(),
                parent: arg(args, "parent").map(str::parse).transpose()?,
            };
            serde_json::to_value(project.create(&cx.store, &new)?)?
        }
        "task_list" => {
            let statuses: Vec<String> = match &args["status"] {
                Value::Array(items) => items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect(),
                Value::String(s) => s.split(',').map(|s| s.trim().to_string()).collect(),
                _ => Vec::new(),
            };
            let all = args["all"].as_bool().unwrap_or(false);
            let f = filter(&statuses, all, arg(args, "parent"))?;
            serde_json::to_value(project.adapter().list(&f)?)?
        }
        "task_get" => serde_json::to_value(project.show(&id(args)?)?)?,
        "task_status" => {
            let status = arg(args, "status").map(str::parse::<Status>).transpose()?;
            let force = args["force"].as_bool().unwrap_or(false);
            serde_json::to_value(project.status(&cx.store, &id(args)?, status, force)?)?
        }
        "task_next" => serde_json::to_value(project.next(&cx.store)?)?,
        "task_ready" => serde_json::to_value(project.ready(&cx.store)?)?,
        "task_claim" => {
            let agent = require_agent(&cx.store, arg(args, "agent"))?;
            let id = arg(args, "id").map(str::parse::<TaskId>).transpose()?;
            serde_json::to_value(project.claim(&cx.store, id.as_ref(), &agent)?)?
        }
        "task_release" => {
            let agent = require_agent(&cx.store, arg(args, "agent"))?;
            let force = args["force"].as_bool().unwrap_or(false);
            serde_json::to_value(project.release(&cx.store, &id(args)?, &agent, force)?)?
        }
        "task_dep" => {
            let Some(blocker) = arg(args, "blocker") else {
                bail!("`blocker` is required");
            };
            serde_json::to_value(project.block(&id(args)?, &blocker.parse()?)?)?
        }
        "task_conflict" => {
            let Some(other) = arg(args, "other") else {
                bail!("`other` is required");
            };
            let remove = args["remove"].as_bool().unwrap_or(false);
            serde_json::to_value(project.conflict(&id(args)?, &other.parse()?, remove)?)?
        }
        "task_parallel" => {
            let on = args["on"].as_bool().unwrap_or(true);
            serde_json::to_value(project.set_parallel(&id(args)?, on)?)?
        }
        "task_priority" => {
            let Some(level) = args["level"].as_u64() else {
                bail!("level is required, 0 to 4");
            };
            let level = u8::try_from(level).context("level is outside 0–4")?;
            serde_json::to_value(project.set_priority(&id(args)?, level)?)?
        }
        other => bail!("unknown tool {other}"),
    };
    Ok(out.to_string())
}

#[cfg(test)]
mod tests {
    use super::defs;

    /// The shipped skill is the rule line that sends agents here (T441.10): it must name every
    /// tool this server lists, and no `task_*` tool it lacks.
    #[test]
    fn the_task_skill_names_exactly_these_tools() {
        let skill = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/skills/rtok-tasks/SKILL.md"
        ));
        let named: std::collections::BTreeSet<&str> = skill
            .split('`')
            .skip(1)
            .step_by(2)
            .filter(|s| s.starts_with("task_"))
            .collect();
        let listed: std::collections::BTreeSet<&str> = defs().iter().map(|d| d.name).collect();
        assert_eq!(named, listed);
    }
}
