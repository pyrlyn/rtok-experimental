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
    no_seed: bool,
) -> Result<Plan> {
    let store = opened(cfg);
    let agent = claim::caller(store.as_ref(), agent)?;
    let mut wt = cfg.worktree.clone();
    wt.seed &= !no_seed;
    claim::add(
        store.as_ref(),
        cwd,
        &wt,
        (task, slug),
        (agent.as_ref(), owner),
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

pub fn finish(
    cfg: &Config,
    path: &Path,
    agent: Option<&str>,
    owner: Option<String>,
    opts: &super::finish::Options,
) -> Result<super::finish::Finished> {
    let store = opened(cfg);
    let agent = claim::caller(store.as_ref(), agent)?;
    super::finish::for_agent(store.as_ref(), path, (agent.as_ref(), owner), opts)
}

pub fn whoami(cfg: &Config, cwd: &Path) -> Whoami {
    let store = opened(cfg);
    // An unknown `RTOK_AGENT_ID` still leaves the root and the cwd worth printing.
    let agent = claim::caller(store.as_ref(), None).ok().flatten();
    super::whoami::run(cwd, &cfg.worktree.root, agent.as_ref(), store.as_ref())
}

/// `sized` also walks the build caches for their bytes (T936); the default does not.
pub fn list(cfg: &Config, cwd: &Path, sized: bool) -> Result<Vec<Row>> {
    let mut rows = if sized {
        super::list::rows_sized(cwd)?
    } else {
        super::list::rows(cwd)?
    };
    let store = opened(cfg);
    super::list::attribute_with_store(&mut rows, store.as_ref(), &cfg.agents.idle);
    Ok(rows)
}

/// `rtok worktree migrate` (T660.5), with what the store says holds worktrees. No store: only
/// the locks hold.
pub fn migrate(
    cfg: &Config,
    cwd: &Path,
    owner: Option<&str>,
    yes: bool,
) -> Result<Vec<super::migrate::Outcome>> {
    use super::migrate::{Held, Me};
    let store = opened(cfg);
    let agent = claim::caller(store.as_ref(), None)?;
    let live = store
        .as_ref()
        .and_then(|s| s.live_agents(&cfg.agents.idle).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|a| Some((a.id, std::path::PathBuf::from(a.cwd?))))
        .collect();
    let held = Held {
        claims: store
            .as_ref()
            .and_then(|s| s.open_worktree_claims().ok())
            .unwrap_or_default(),
        pending: pending_paths(cfg),
        live,
    };
    let me = Me {
        agent: agent.as_ref().map(|a| a.id.as_str()),
        owner,
    };
    super::migrate::run(cwd, &cfg.worktree.root, &me, &held, store.as_ref(), yes)
}

/// Live agent ids for `worktree gc`. No store means no live agents (T285).
pub fn live_ids(cfg: &Config) -> HashSet<String> {
    Store::open(&cfg.core.db_path)
        .and_then(|s| s.live_agents(&cfg.agents.idle))
        .map(|agents| agents.into_iter().map(|a| a.id).collect())
        .unwrap_or_default()
}

/// Ended agent ids for `worktree gc` (T949). No store means none ended, so every lock holds.
pub fn ended_ids(cfg: &Config) -> HashSet<String> {
    Store::open(&cfg.core.db_path)
        .and_then(|s| s.ended_agent_ids())
        .map(|ids| ids.into_iter().collect())
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
