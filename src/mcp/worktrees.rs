// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T285 (D34): MCP `worktree_add`, `worktree_list`, `worktree_remove` and `worktree_adopt` (T289.2), the
//! agent-facing side of `rtok worktree add` / `list` / `remove`. They call the same functions as the CLI; the only
//! difference is who the agent is: the MCP session's link (T283.1), never an argument, so a
//! model cannot claim a worktree in another agent's name or pick its own lock owner.

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::plugin::{Runtime, ToolDef};
use crate::store::AgentDetail;
use crate::worktree::{claim, list, remove};

pub fn add_def() -> ToolDef {
    ToolDef {
        name: "worktree_add",
        description: "Create a git worktree for a task, locked and bound to this session's agent. Returns {path, branch, task, agent, note}; work only inside path.",
        input_schema: json!({"type":"object","properties":{"task":{"type":"string","description":"task id, e.g. T12"},"slug":{"type":"string","description":"optional branch suffix"}},"required":["task"]}),
    }
}

pub fn list_def() -> ToolDef {
    ToolDef {
        name: "worktree_list",
        description: "Every worktree of this repository: path, branch, state, size, and the agent bound to it with its host and live/idle/ended state.",
        input_schema: json!({"type":"object","properties":{}}),
    }
}

pub fn remove_def() -> ToolDef {
    ToolDef {
        name: "worktree_remove",
        description: "Remove this agent's own finished worktree by path or task id, with its branch when merged. Refuses a dirty or unmerged worktree (keep_branch removes an unmerged clean one and keeps the branch), another agent's, and the cwd. Never forces.",
        input_schema: json!({"type":"object","properties":{"path":{"type":"string"},"task":{"type":"string"},"keep_branch":{"type":"boolean"}}}),
    }
}

pub fn adopt_def() -> ToolDef {
    ToolDef {
        name: "worktree_adopt",
        description: "Bind a worktree your host made (not worktree_add) to this session's agent: the one holding path (default: the server's cwd). Returns {path, task, origin, locked}. task names it when the branch cannot (a detached HEAD). Never takes another agent's.",
        input_schema: json!({"type":"object","properties":{"path":{"type":"string","description":"the worktree or a directory inside it"},"task":{"type":"string"}}}),
    }
}

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

pub fn add(cx: &Runtime, agent: &AgentDetail, args: &Value) -> Result<String> {
    let Some(task) = arg(args, "task") else {
        bail!("`task` is required, e.g. T12");
    };
    let root = &cx.config.worktree.root;
    let cwd = std::env::current_dir()?;
    let plan = claim::add(
        Some(&cx.store),
        &cwd,
        root,
        (task, arg(args, "slug")),
        Some(agent),
        None,
        cx.config.plugins.graph.auto_add_projects,
    )?;
    crate::agents::task_list::write_for(&cwd, &plan.path, &plan.task);
    Ok(json!({
        "path": plan.path,
        "branch": plan.branch,
        "task": plan.task,
        "agent": agent.id,
        "note": "work only inside `path`; remove it with `worktree_remove` when merged",
    })
    .to_string())
}

pub fn adopt(cx: &Runtime, agent: &AgentDetail, args: &Value) -> Result<String> {
    let path = match arg(args, "path") {
        Some(path) => std::env::current_dir()?.join(path),
        None => std::env::current_dir()?,
    };
    let task = arg(args, "task");
    let done = claim::bind(
        Some(&cx.store),
        &path,
        agent,
        None,
        task,
        true,
        cx.config.plugins.graph.auto_add_projects,
    )?;
    Ok(serde_json::to_string(&done)?)
}

pub fn remove(cx: &Runtime, agent: &AgentDetail, args: &Value) -> Result<String> {
    let Some(target) = arg(args, "path").or_else(|| arg(args, "task")) else {
        bail!("`path` or `task` is required");
    };
    let keep_branch = args["keep_branch"].as_bool().unwrap_or(false);
    let done = remove::for_agent(
        Some(&cx.store),
        &std::env::current_dir()?,
        target,
        (Some(agent), None),
        keep_branch,
    )?;
    Ok(serde_json::to_string(&done)?)
}

pub fn list(cx: &Runtime) -> Result<String> {
    let mut rows = list::rows(&std::env::current_dir()?)?;
    list::attribute_with_store(&mut rows, Some(&cx.store), &cx.config.agents.idle);
    Ok(serde_json::to_string(&rows)?)
}
