// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok worktree remove` (T286): an agent removes its own finished worktree. [`detach`] is
//! the single-worktree removal `gc` applies too; [`run`] refuses before it touches anything.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use super::{Record, State, claim, git, inventory};
use crate::store::{AgentDetail, Store};

/// Unlock, remove without `--force`, then delete the branch when `drop_branch`. A failed
/// removal puts the lock back, so a half-done run never strips another run's protection. A
/// branch git will not delete (checked out elsewhere) is reported, not an error: the worktree
/// is already gone, and an Err would keep its claim and read as `failed, kept` in gc.
pub fn detach(repo: &Path, record: &Record, drop_branch: bool) -> Result<String> {
    if record.locked.is_some() {
        git::unlock(repo, &record.path)?;
    }
    if let Err(e) = git::remove(repo, &record.path) {
        if let Some(reason) = &record.locked {
            git::lock(repo, &record.path, reason)?;
        }
        return Err(e);
    }
    let Some(branch) = record.branch.as_deref().filter(|_| drop_branch) else {
        return Ok("removed; branch kept".into());
    };
    if let Err(e) = git::delete_branch(repo, branch) {
        return Ok(format!("removed; branch kept: {e:#}"));
    }
    Ok(match git::has_remote_branch(repo, branch) {
        true => format!("removed with its branch; remote left: git push origin --delete {branch}"),
        false => "removed with its branch".into(),
    })
}

/// Who asks: the rtok agent id and the owner name an old lock carries — either may be unknown.
pub struct Caller<'a> {
    pub agent: Option<&'a str>,
    pub owner: Option<&'a str>,
}

#[derive(Debug, Serialize)]
pub struct Removed {
    /// Canonical, as the claim row stores it.
    pub path: String,
    pub branch: Option<String>,
    pub note: String,
    /// Set by a dry run: what the removal would free. Absent after a real removal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<u64>,
}

impl Removed {
    /// The dry-run text: the shared removal line and totals, then what happens to the branch.
    pub fn preview_text(&self) -> String {
        let body = crate::removal_preview::render([(
            self.path.as_str(),
            self.bytes.unwrap_or_default(),
            self.files.unwrap_or_default(),
        )]);
        format!("{body}{}\n", self.note)
    }
}

/// Remove the worktree at `target` (a path, else a task id) for `who`. Refuses a dirty
/// worktree, anyone else's lock on one that is not [`super::Entry::done`], the one holding
/// `cwd`, and an unmerged branch unless `keep_branch`.
pub fn run(cwd: &Path, target: &str, who: &Caller, keep_branch: bool) -> Result<Removed> {
    remove_or_preview(cwd, target, who, keep_branch, false)
}

/// [`run`] with every refusal and none of the effects: reports the size and file count the
/// removal would free and what happens to the branch.
pub fn preview(cwd: &Path, target: &str, who: &Caller, keep_branch: bool) -> Result<Removed> {
    remove_or_preview(cwd, target, who, keep_branch, true)
}

fn remove_or_preview(
    cwd: &Path,
    target: &str,
    who: &Caller,
    keep_branch: bool,
    dry_run: bool,
) -> Result<Removed> {
    let real = |p: &Path| crate::fs::canon(p);
    let as_path = cwd.join(target);
    let named = as_path.exists();
    let from = if named { as_path.as_path() } else { cwd };
    let base = git::default_base(from);
    if let Err(e) = git::fetch(from, &base) {
        crate::log::stderr_ln(&format!(
            "warning: {e:#}; merged is judged against the local {base}"
        ));
    }
    let entries = inventory(from)?;
    let main = &entries
        .first()
        .context("git lists no worktree")?
        .record
        .path;
    let found: Vec<_> = match named {
        true => entries
            .iter()
            .filter(|e| crate::fs::same_path(&real(&e.record.path), &real(&as_path)))
            .collect(),
        false => entries
            .iter()
            .filter(|e| is_task(&e.record, target))
            .collect(),
    };
    let entry = match found[..] {
        [one] => one,
        [] => bail!("{target}: no worktree with that path or task"),
        _ => bail!("{target}: {} worktrees match; name the path", found.len()),
    };
    let (record, path) = (&entry.record, real(&entry.record.path));
    let shown = path.display();
    if entry.state == State::Main {
        bail!("{shown} is the main checkout");
    }
    if real(cwd).starts_with(&path) {
        bail!("{shown} holds the current directory; run from the main checkout");
    }
    // An unknown caller holds no lock: "" is never a parsed owner or agent.
    let held =
        (!record.claimable_by(who.owner.unwrap_or(""), who.agent.unwrap_or(""))).then(|| {
            record
                .owner()
                .map_or("an unknown owner".into(), |o| o.reason())
        });
    // A finished task has nothing left to protect, and its owner often cannot name itself
    // (T453: no `RTOK_AGENT_ID` in the desktop app's shell), so its lock does not hold.
    if let Some(held) = held.as_ref().filter(|_| !entry.done) {
        bail!("{shown} is locked by {held}; not removed");
    }
    match (entry.state, &record.branch) {
        (State::Stale, _) => bail!("{shown} is gone; `rtok worktree gc --yes` drops its record"),
        (State::Dirty, _) => bail!("{shown} has uncommitted or untracked files"),
        (State::Unmerged, Some(_)) if keep_branch => {}
        (State::Unmerged, Some(b)) => bail!(
            "{shown}: {b} is not merged into {base}; check `gh pr view {b}`, or pass \
             --keep-branch (MCP: keep_branch) to remove the worktree and keep the branch"
        ),
        (State::Unmerged, None) => {
            bail!("{shown}: detached HEAD not merged into {base}; its commits would be lost")
        }
        _ => {}
    }
    let drop_branch = entry.merged && !keep_branch;
    if dry_run {
        let (bytes, files) = crate::removal_preview::size_of(&path);
        let note = match (&record.branch, drop_branch) {
            (Some(b), true) => format!("branch {b}: would be deleted (merged)"),
            (Some(b), false) => format!("branch {b}: would be kept"),
            (None, _) => "detached HEAD: no branch".into(),
        };
        return Ok(Removed {
            path: path.display().to_string(),
            branch: record.branch.clone(),
            note,
            bytes: Some(bytes),
            files: Some(files),
        });
    }
    let mut note = detach(main, record, drop_branch)?;
    if let Some(held) = held {
        note.push_str(&format!("; finished, so the lock by {held} was opened"));
    }
    Ok(Removed {
        path: path.display().to_string(),
        branch: record.branch.clone(),
        note,
        bytes: None,
        files: None,
    })
}

/// `rtok worktree remove` and MCP `worktree_remove`: [`run`] for `agent` (named by `owner_flag`
/// when it gives one), then release the claim. One path, so both surfaces refuse and release
/// the same way. With neither an agent nor an owner there is no name to hold a lock by, so
/// only an unlocked worktree goes.
pub fn for_agent(
    store: Option<&Store>,
    cwd: &Path,
    target: &str,
    (agent, owner_flag): (Option<&AgentDetail>, Option<String>),
    keep_branch: bool,
) -> Result<Removed> {
    let owner = caller_owner(store, owner_flag, agent)?;
    let who = Caller {
        agent: agent.map(|a| a.id.as_str()),
        owner: owner.as_deref(),
    };
    let done = run(cwd, target, &who, keep_branch)?;
    if let Some(Err(e)) = store.map(|s| s.release_worktree_claim(&done.path)) {
        crate::log::stderr_ln(&format!("warning: claim not released: {e:#}"));
    }
    Ok(done)
}

fn caller_owner(
    store: Option<&Store>,
    owner_flag: Option<String>,
    agent: Option<&AgentDetail>,
) -> Result<Option<String>> {
    match (owner_flag, agent) {
        (None, None) => Ok(None),
        (flag, agent) => Ok(Some(claim::owner(flag, agent, store)?)),
    }
}

/// [`for_agent`] that only previews: the same refusals, no removal and no claim release.
pub fn preview_for_agent(
    store: Option<&Store>,
    cwd: &Path,
    target: &str,
    (agent, owner_flag): (Option<&AgentDetail>, Option<String>),
    keep_branch: bool,
) -> Result<Removed> {
    let owner = caller_owner(store, owner_flag, agent)?;
    let who = Caller {
        agent: agent.map(|a| a.id.as_str()),
        owner: owner.as_deref(),
    };
    preview(cwd, target, &who, keep_branch)
}

/// A task id names the lock's task or the branch `<task>[-<slug>]`, case aside.
fn is_task(record: &Record, task: &str) -> bool {
    let task = task.to_ascii_lowercase();
    let locked = record
        .owner()
        .is_some_and(|o| o.task.to_ascii_lowercase() == task);
    let branch = record.branch.as_deref().map(str::to_ascii_lowercase);
    locked || branch.is_some_and(|b| b == task || b.starts_with(&format!("{task}-")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(Some("t286-worktree-remove"), None, true)]
    #[case(Some("T286"), None, true)]
    #[case(Some("t2860"), None, false)]
    #[case(None, Some("me | T286 | 2026-09-27"), true)]
    #[case(Some("main"), None, false)]
    fn a_task_id_names_the_branch_or_the_lock_task(
        #[case] branch: Option<&str>,
        #[case] locked: Option<&str>,
        #[case] hit: bool,
    ) {
        let record = Record {
            branch: branch.map(Into::into),
            locked: locked.map(Into::into),
            ..Record::default()
        };
        assert_eq!(is_task(&record, "t286"), hit);
    }
}
