// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T159: Claude Code's `WorktreeCreate` / `WorktreeRemove` hooks (`research.md` §18.3, docs
//! checked 2026-10-02) routed through the same code `rtok worktree add` / `remove` run, so a
//! worktree the host creates gets rtok's location, name, owner lock and agent claim. T995.1:
//! CodeBuddy has the same two hooks with the same payloads (`research.md` §18.6), so `--host`
//! only changes whose agent and lock the worktree gets.
//!
//! The host replaces its own create/remove with these hooks, so the contract differs from
//! every other hook: `WorktreeCreate` prints the path (not JSON) and fails the creation on any
//! non-zero exit; `WorktreeRemove` counts exit 0 as removed. An error here therefore exits
//! non-zero with nothing destroyed, and each plugin's `scripts/worktree.sh` turns a failed
//! create into a plain `git worktree add` at the host's default path (fail open).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

use super::remove::{self, Caller};
use super::{State, claim, clean, git, inventory};
use crate::config::Config;
use crate::store::{AgentDetail, Store};

/// The hosts whose plugin sends these hooks. The name is also the lock owner when no rtok agent
/// can be named (no store, or no session id).
pub const HOSTS: [&str; 2] = ["claude", "codebuddy"];

pub fn handles(event: &str) -> bool {
    matches!(event, "WorktreeCreate" | "WorktreeRemove")
}

/// The fields of the two payloads this module reads (`research.md` §18.3); the host's others
/// (`transcript_path`, …) are ignored.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Payload {
    session_id: String,
    cwd: String,
    agent_id: Option<String>,
    name: String,
    worktree_path: String,
}

/// `Some(path)` for a create (what the host reads from stdout), `None` for a remove or while
/// worktrees are off. `host` is `--host`, Claude Code when absent.
pub fn run(
    event: &str,
    host: Option<&str>,
    stdin: impl Read,
    cfg: &Config,
) -> Result<Option<String>> {
    let host = host.unwrap_or(HOSTS[0]);
    ensure!(
        HOSTS.contains(&host),
        "{host} sends no {event}; worktree hooks come from {}",
        HOSTS.join(", ")
    );
    let max = u64::from(cfg.core.hook_max_input_bytes);
    let mut buf = Vec::new();
    stdin.take(max + 1).read_to_end(&mut buf)?;
    ensure!(
        buf.len() as u64 <= max,
        "{event} payload is over {max} bytes"
    );
    let payload: Payload = serde_json::from_slice(&buf).context("bad payload")?;
    if !cfg.worktree.enabled {
        return without_rtok(event, &payload);
    }
    let store = Store::open(&cfg.core.db_path).ok();
    if event == "WorktreeCreate" {
        let path = create(&payload, cfg, store.as_ref(), host)?;
        return Ok(Some(path.display().to_string()));
    }
    let note = remove(&payload, store.as_ref(), host)?;
    crate::log::stderr_ln(&format!("rtok: {}: {note}", payload.worktree_path));
    Ok(None)
}

/// `[worktree] enabled = false` (T410): what the host does without rtok. A create prints no
/// path, so `worktree.sh` makes the host's own worktree; a remove is a plain
/// `git worktree remove`, never `--force`, so a dirty worktree stays and the host says why.
fn without_rtok(event: &str, p: &Payload) -> Result<Option<String>> {
    let path = Path::new(&p.worktree_path);
    if event == "WorktreeRemove" && path.is_dir() {
        ensure!(!p.cwd.is_empty(), "payload has no cwd");
        git::remove(Path::new(&p.cwd), path)?;
    }
    Ok(None)
}

fn create(p: &Payload, cfg: &Config, store: Option<&Store>, host: &str) -> Result<PathBuf> {
    ensure!(
        !p.name.is_empty() && !p.cwd.is_empty(),
        "payload has no name or cwd"
    );
    let cwd = Path::new(&p.cwd);
    // The host asks again for a name it already got (a resumed `--worktree` session): hand back
    // that worktree rather than fail on "one worktree per task".
    let task = p.name.to_ascii_lowercase();
    let known = |r: &super::Record| r.path.exists() && r.owner().is_some_and(|o| o.task == task);
    // One spelling for both answers: git lists `C:/…` on Windows, `add` plans `C:\…`.
    let native = |p: PathBuf| fs_kit::path::canonicalize(&p).unwrap_or(p);
    if let Some(r) = git::list(cwd)?.iter().skip(1).find(|r| known(r)) {
        return Ok(native(r.path.clone()));
    }
    let agent = session_agent(store, p, host);
    let owner = agent.is_none().then(|| host.to_string());
    let auto_add = cfg.plugins.graph.auto_add_projects;
    let plan = claim::add(
        store,
        cwd,
        &cfg.worktree,
        (&p.name, None),
        (agent.as_ref(), owner),
        auto_add,
    )?;
    Ok(native(plan.path))
}

/// The agent row of the payload's session — or, inside a sub-agent, of that sub-agent
/// (`agent_id`); registering is an upsert, so a session no other hook has seen yet gets a row.
fn session_agent(store: Option<&Store>, p: &Payload, host: &str) -> Option<AgentDetail> {
    let store = store.filter(|_| !p.session_id.is_empty())?;
    let host = store.host_id_or_other(host).ok()?;
    let sub = p.agent_id.as_deref().filter(|s| !s.is_empty());
    let id = store
        .register_agent(host, &p.session_id, sub, Some(&p.cwd), None)
        .ok()?;
    store.agent_detail(&id).ok()?
}

/// Removes what rtok would remove for its own agent; leaves everything else in place and says why
/// (the `Err`). Tagged caches go either way: they hold no source (T152).
fn remove(p: &Payload, store: Option<&Store>, host: &str) -> Result<String> {
    ensure!(!p.worktree_path.is_empty(), "payload has no worktree_path");
    let path = Path::new(&p.worktree_path);
    if !path.exists() {
        return Ok("already gone".into());
    }
    // From the main checkout: the session's `cwd` may be the worktree itself, which `remove`
    // refuses to take from under its caller.
    let entries = inventory(path)?;
    let main = &entries
        .first()
        .context("git lists no worktree")?
        .record
        .path;
    let real = |p: &Path| crate::fs::canon(p);
    let Some(entry) = entries
        .iter()
        .find(|e| crate::fs::same_path(&real(&e.record.path), &real(path)))
    else {
        bail!("not a worktree of {}", main.display());
    };
    if entry.state == State::Main {
        bail!("is the main checkout");
    }
    let (agent, owner) = caller(store, p, entry.record.owner().and_then(|o| o.agent), host);
    let who = Caller {
        agent: agent.as_deref(),
        owner: Some(&owner),
    };
    // An unmerged branch keeps its commits: the worktree goes, the branch stays.
    let keep_branch = entry.state == State::Unmerged;
    // Windows refuses to delete any process's current directory, and the host may start this
    // hook inside the worktree; git then drops its record but leaves the directory behind.
    let _ = std::env::set_current_dir(main);
    let done = remove::run(main, &p.worktree_path, &who, keep_branch, store);
    match &done {
        Ok(removed) => {
            let released = store.map(|s| super::claim::release(s, &removed.path));
            if let Some(Err(e)) = released {
                crate::log::stderr_ln(&format!("warning: {e:#}"));
            }
        }
        Err(_) => forget_caches(main, path),
    }
    done.map(|removed| removed.note)
}

/// `(agent id, owner)` to remove as: the lock's own agent when it belongs to the payload's
/// session (a sub-agent's worktree is removed under its parent's `session_id`), else the
/// session's first agent, else no agent and the plain host name an agent-less lock carries.
fn caller(
    store: Option<&Store>,
    p: &Payload,
    locked: Option<String>,
    host: &str,
) -> (Option<String>, String) {
    let rows = store.and_then(|s| {
        s.agents_of_sessions(std::slice::from_ref(&p.session_id))
            .ok()
    });
    let rows = rows.unwrap_or_default();
    let pick = |id: &String| rows.iter().find(|a| &a.id == id);
    let agent = locked.as_ref().and_then(pick).or(rows.first());
    let owner = claim::owner(None, agent, store).unwrap_or_else(|_| host.into());
    (agent.map(|a| a.id.clone()), owner)
}

/// A worktree that stays still sheds its idle build output; a failure only warns.
fn forget_caches(main: &Path, worktree: &Path) {
    let policy = clean::Policy {
        idle: Duration::ZERO,
        now: SystemTime::now(),
    };
    let swept = clean::run(main, &[worktree.to_path_buf()], &policy, true);
    if let Err(e) = swept {
        crate::log::stderr_ln(&format!("warning: caches not cleaned: {e:#}"));
    }
}
