// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Store-backed `rtok worktree` operations. The CLI parses flags and prints; the open lives here.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Result, bail};

use super::add::Plan;
use super::claim::{self, Adopted};
use super::list::Row;
use super::remove::Removed;
use super::whoami::Whoami;
use crate::config::Config;
use crate::store::Store;

fn opened(cfg: &Config) -> Option<Store> {
    Store::open(&cfg.core.db_path).ok()
}

pub fn add(
    cfg: &Config,
    cwd: &Path,
    task: &str,
    slug: Option<&str>,
    owner: Option<String>,
    agent: Option<&str>,
) -> Result<Plan> {
    let store = opened(cfg);
    let agent = claim::caller(store.as_ref(), agent)?;
    claim::add(
        store.as_ref(),
        cwd,
        &cfg.worktree.root,
        (task, slug),
        agent.as_ref(),
        owner,
        cfg.plugins.graph.auto_add_projects,
    )
}

pub fn claim(
    cfg: &Config,
    path: &Path,
    agent: Option<&str>,
    owner: Option<String>,
) -> Result<Adopted> {
    let store = opened(cfg);
    let Some(agent) = claim::caller(store.as_ref(), agent)? else {
        bail!("no agent to bind: pass --agent or set RTOK_AGENT_ID");
    };
    claim::bind(
        store.as_ref(),
        path,
        &agent,
        owner,
        None,
        false,
        cfg.plugins.graph.auto_add_projects,
    )
}

pub fn adopt(
    cfg: &Config,
    path: Option<&Path>,
    task: Option<&str>,
    agent: Option<&str>,
    owner: Option<String>,
) -> Result<Adopted> {
    let store = opened(cfg);
    let cwd;
    let path = match path {
        Some(path) => path,
        None => {
            cwd = std::env::current_dir()?;
            &cwd
        }
    };
    let Some(agent) = claim::caller(store.as_ref(), agent)? else {
        let Some(store) = store.as_ref() else {
            bail!("no agent to bind: pass --agent or set RTOK_AGENT_ID");
        };
        return claim::bind_unattended(
            store,
            (path, task),
            &cfg.agents.idle,
            cfg.plugins.graph.auto_add_projects,
        );
    };
    claim::bind(
        store.as_ref(),
        path,
        &agent,
        owner,
        task,
        true,
        cfg.plugins.graph.auto_add_projects,
    )
}

pub fn remove(
    cfg: &Config,
    target: &str,
    agent: Option<&str>,
    owner: Option<String>,
    keep_branch: bool,
    dry_run: bool,
) -> Result<Removed> {
    let store = opened(cfg);
    let agent = claim::caller(store.as_ref(), agent)?;
    let cwd = std::env::current_dir()?;
    let run = if dry_run {
        super::remove::preview_for_agent
    } else {
        super::remove::for_agent
    };
    run(
        store.as_ref(),
        &cwd,
        target,
        (agent.as_ref(), owner),
        keep_branch,
    )
}

pub fn whoami(cfg: &Config, cwd: &Path) -> Whoami {
    let store = opened(cfg);
    // An unknown `RTOK_AGENT_ID` still leaves the root and the cwd worth printing.
    let agent = claim::caller(store.as_ref(), None).ok().flatten();
    super::whoami::run(cwd, &cfg.worktree.root, agent.as_ref(), store.as_ref())
}

pub fn list(cfg: &Config, cwd: &Path) -> Result<Vec<Row>> {
    let mut rows = super::list::rows(cwd)?;
    let store = opened(cfg);
    super::list::attribute_with_store(&mut rows, store.as_ref(), &cfg.agents.idle);
    Ok(rows)
}

/// Live agent ids for `worktree gc`. No store means no live agents (T285).
pub fn live_ids(cfg: &Config) -> HashSet<String> {
    Store::open(&cfg.core.db_path)
        .and_then(|s| s.live_agents(&cfg.agents.idle))
        .map(|agents| agents.into_iter().map(|a| a.id).collect())
        .unwrap_or_default()
}

/// Worktrees `adopt` parked for the next agent (T285's `live_ids` counterpart, T289.5).
pub fn pending_paths(cfg: &Config) -> Vec<std::path::PathBuf> {
    Store::open(&cfg.core.db_path)
        .and_then(|s| s.pending_worktrees())
        .map(|rows| {
            rows.into_iter()
                .map(|(p, _)| crate::fs::canon(Path::new(&p)))
                .collect()
        })
        .unwrap_or_default()
}
