// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T285 (D34): MCP `worktree_add`, `worktree_list`, `worktree_remove`, `worktree_finish` (T952.1)
//! and `worktree_adopt` (T289.2), the agent-facing side of `rtok worktree add` / `list` /
//! `remove` / `finish`. They call the same functions as the CLI; the only difference is who the
//! agent is: the MCP session's link (T283.1). The optional `agent` argument (T924) only fills a
//! link that is missing or a guess, never overrides a stable one, so a model still cannot claim a
//! worktree in another session's name or pick its own lock owner. The optional `repo` says which
//! repository to act in when the server runs elsewhere (for `worktree_finish` it is the GitHub
//! `owner/name`).

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use rtok_config::root::ProjectRoot;
use serde_json::{Value, json};

use super::{with_agent, with_repo};
use crate::plugin::{Runtime, ToolDef};
use crate::store::AgentDetail;
use crate::worktree::{claim, finish, list, remove};

pub fn add_def() -> ToolDef {
    ToolDef {
        name: "worktree_add",
        description: "Create a git worktree for a task, locked and bound to this session's agent. Returns {path, branch, task, agent, note}; work only inside path.",
        input_schema: with_repo(with_agent(
            json!({"type":"object","properties":{"task":{"type":"string","description":"task id, e.g. T12, or `T12 short-slug` in one string"},"slug":{"type":"string","description":"optional branch suffix"}},"required":["task"]}),
        )),
    }
}

pub fn list_def() -> ToolDef {
    ToolDef {
        name: "worktree_list",
        description: "Every worktree of this repository: path, branch, state, size, and the agent bound to it with its host and live/idle/ended state.",
        input_schema: with_repo(json!({"type":"object","properties":{}})),
    }
}

pub fn remove_def() -> ToolDef {
    ToolDef {
        name: "worktree_remove",
        description: "Remove this agent's own finished worktree by path or task id, with its branch when merged. Refuses a dirty or unmerged worktree (keep_branch removes an unmerged clean one and keeps the branch), another agent's, and the cwd. Never forces.",
        input_schema: with_repo(with_agent(
            json!({"type":"object","properties":{"path":{"type":"string"},"task":{"type":"string"},"keep_branch":{"type":"boolean"}}}),
        )),
    }
}

pub fn finish_def() -> ToolDef {
    ToolDef {
        name: "worktree_finish",
        description: "Send this agent's worktree at path to review: commit leftovers (no credential files), push, open or reuse the PR, squash auto-merge unless no_auto_merge, unlock. Never closes the task.",
        input_schema: json!({"type":"object","properties":{"path":{"type":"string","description":"the worktree or a directory inside it"},"message":{"type":"string","description":"commit message for uncommitted work"},"repo":{"type":"string","description":"owner/name for gh"},"no_auto_merge":{"type":"boolean"}},"required":["path"]}),
    }
}

pub fn adopt_def() -> ToolDef {
    ToolDef {
        name: "worktree_adopt",
        description: "Bind a worktree your host made (not worktree_add) to this session's agent: the one holding path (default: the server's cwd). Returns {path, task, origin, locked}. task names it when the branch cannot (a detached HEAD). Never takes another agent's.",
        input_schema: with_agent(
            json!({"type":"object","properties":{"path":{"type":"string","description":"the worktree or a directory inside it"},"task":{"type":"string"}}}),
        ),
    }
}

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// The directory a call acts in: `repo` (relative to the server's cwd), else the cwd itself.
fn base_dir(args: &Value) -> Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    Ok(match arg(args, "repo") {
        Some(repo) => cwd.join(repo),
        None => cwd,
    })
}

/// A shared server runs in one checkout while its callers work in others, so a worktree made
/// "here" can land in a stranger's repository (T924). The caller's repository is the one its
/// hooks recorded; an agent with none recorded cannot be checked and is not refused.
fn ensure_callers_repo(base: &Path, agent: &AgentDetail) -> Result<()> {
    let (Some(mine), Some(target)) = (
        agent
            .cwd
            .as_deref()
            .and_then(|c| ProjectRoot::find(Path::new(c))),
        ProjectRoot::find(base),
    ) else {
        return Ok(());
    };
    if crate::fs::same_dir(&mine.main.to_string_lossy(), &target.main.to_string_lossy()) {
        return Ok(());
    }
    bail!(
        "{} is in repository {}, not yours ({}); pass `repo` with your repository's path",
        base.display(),
        target.main.display(),
        mine.main.display()
    )
}

pub fn add(cx: &Runtime, agent: &AgentDetail, args: &Value) -> Result<String> {
    let Some(task) = arg(args, "task") else {
        bail!("`task` is required, e.g. T12");
    };
    let cwd = base_dir(args)?;
    ensure_callers_repo(&cwd, agent)?;
    let plan = claim::add(
        Some(&cx.store),
        &cwd,
        &cx.config.worktree,
        (task, arg(args, "slug")),
        (Some(agent), None),
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
        &base_dir(args)?,
        target,
        (Some(agent), None),
        keep_branch,
    )?;
    Ok(serde_json::to_string(&done)?)
}

pub fn finish(cx: &Runtime, agent: &AgentDetail, args: &Value) -> Result<String> {
    let Some(path) = arg(args, "path") else {
        bail!("`path` is required: the task worktree");
    };
    let opts = finish::Options {
        message: arg(args, "message").map(Into::into),
        repo: arg(args, "repo").map(Into::into),
        no_auto_merge: args["no_auto_merge"].as_bool().unwrap_or(false),
    };
    let path = std::env::current_dir()?.join(path);
    let done = finish::for_agent(Some(&cx.store), &path, (Some(agent), None), &opts)?;
    Ok(serde_json::to_string(&done)?)
}

pub fn list(cx: &Runtime, args: &Value) -> Result<String> {
    let cwd = base_dir(args)?;
    // A server launched in `$HOME` has no repository of its own; list the worktrees of
    // the client root that lies in one. Without a usable root the error names what to do,
    // never git's `fatal: not a git repository`.
    #[cfg(feature = "read")]
    let extra = crate::plugins::read::roots::client_roots();
    #[cfg(not(feature = "read"))]
    let extra: Vec<std::path::PathBuf> = Vec::new();
    let dir = crate::project::project_dir(&cwd, &extra)?;
    let mut rows = list::rows(&dir)?;
    list::attribute_with_store(&mut rows, Some(&cx.store), &cx.config.agents.idle);
    Ok(serde_json::to_string(&rows)?)
}
