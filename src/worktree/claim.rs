// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T285: bind a worktree to the calling rtok agent (T282) — in the git lock reason (v2, the
//! source of truth) and in the store's `worktree_claims` (the fast join).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::{Owner, git, origin};
use crate::store::{AgentDetail, Origin, Store};

/// Someone else's lock on a worktree, and what the store last saw of its holder — what
/// `remove` and `adopt` refuse with.
#[derive(Debug)]
pub struct Held {
    /// The lock's `Owner::reason()`, or `"an unknown owner"`.
    pub by: String,
    /// Seconds since the store last saw the holder; `None` when it cannot say.
    pub seen_ago: Option<i64>,
    /// `false` once the holder is gone — an ended agent, or one unseen for gc's
    /// `--stale-lock` (T418's window). Such a claim holds nothing.
    pub holds: bool,
}

impl Held {
    /// `locked by …`, with how long ago the holder was seen when the store can say.
    pub fn locked(&self) -> String {
        match self.seen_ago {
            Some(secs) => format!(
                "locked by {}, holder seen {} ago",
                self.by,
                crate::render::duration(secs)
            ),
            None => format!("locked by {}", self.by),
        }
    }
}

/// The lock `record` has against `(owner, agent)` — `None` when there is none, so the caller
/// may take the worktree outright. Otherwise the holder it names, with [`Held::holds`] saying
/// whether that claim still stands: a holder that has ended, or that the store has not seen
/// for gc's `--stale-lock`, has abandoned it and blocks nothing, while a fresh holder still
/// refuses and the refusal names how long ago they were seen.
pub fn held_against(
    record: &super::Record,
    (owner, agent): (&str, &str),
    store: Option<&Store>,
) -> Option<Held> {
    if record.claimable_by(owner, agent) {
        return None;
    }
    let owned = record.owner();
    let by = owned
        .as_ref()
        .map_or_else(|| "an unknown owner".to_string(), |o| o.reason());
    // Only an agent-named claim can be judged gone. A lock that names no agent, or a holder
    // the store has no row for, keeps holding — a lock outlives a lost store.
    let holder = owned
        .as_ref()
        .and_then(|o| o.agent.as_deref())
        .and_then(|id| store.and_then(|s| s.worktree_claim_holder(id).ok().flatten()));
    let now = i64::try_from(crate::log::now()).unwrap_or(i64::MAX);
    Some(match holder {
        Some(holder) => Held {
            by,
            seen_ago: Some(holder.seen_ago(now)),
            holds: holder.holds(now, super::gc::stale_lock()),
        },
        None => Held {
            by,
            seen_ago: None,
            holds: true,
        },
    })
}

/// The calling agent: `--agent <prefix>` (must resolve), else `RTOK_AGENT_ID` (T283; an id
/// the store does not know is ignored) or the host session's agent (T473), else none.
pub fn caller(store: Option<&Store>, flag: Option<&str>) -> Result<Option<AgentDetail>> {
    let env = crate::shell_agent::shell_agent(store, |k| std::env::var(k).ok());
    let (raw, explicit) = match (flag, env) {
        (Some(flag), _) => (flag.to_string(), true),
        (None, Some(env)) => (env, false),
        (None, None) => return Ok(None),
    };
    // T973: agents copy a plan's `agent: <provider> / <model>` into the flag; "unknown" alone
    // never told them it wants an id.
    if explicit && raw.contains(['/', ' ']) {
        bail!(
            "--agent {raw}: not an rtok agent id (that looks like a plan's `<provider> / <model>`); \
             pass your id or a prefix of it, see `rtok agents whoami`"
        );
    }
    let found = store
        .context("no store")
        .and_then(|s| s.agent_detail(&s.resolve_agent(&raw)?).map_err(Into::into));
    match found {
        Ok(Some(detail)) => Ok(Some(detail)),
        Ok(None) if !explicit => Ok(None),
        Err(_) if !explicit => {
            crate::log::stderr_ln(&format!(
                "warning: RTOK_AGENT_ID {raw} is not in the store; no agent bound"
            ));
            Ok(None)
        }
        Ok(None) => bail!("--agent {raw}: unknown"),
        Err(e) => bail!("--agent {raw}: {e:#}"),
    }
}

/// `--owner`, else `<host> / <model>` of the agent (`<host>` alone while no call has named
/// a model), else an error: without an agent the owner is the only name a lock has.
pub fn owner(
    flag: Option<String>,
    agent: Option<&AgentDetail>,
    store: Option<&Store>,
) -> Result<String> {
    if let Some(owner) = flag {
        return Ok(owner);
    }
    let Some(agent) = agent else {
        bail!("--owner is required when no agent is known (--agent or RTOK_AGENT_ID)");
    };
    let model = store.and_then(|s| s.session_model(&agent.host_session_id).ok().flatten());
    Ok(match model {
        Some(model) => format!("{} / {model}", agent.host),
        None => agent.host.clone(),
    })
}

/// T934: a claim write waits out another process's write lock for about
/// `LOCK_ATTEMPTS × (busy_timeout + LOCK_PAUSE)` before it gives up.
const LOCK_ATTEMPTS: u32 = 8;
const LOCK_PAUSE: std::time::Duration = std::time::Duration::from_millis(250);

/// Record the claim in the store, retrying while another process holds the write lock. A lost
/// claim lets a second agent take the same task, so a store error that outlasts the retries
/// fails the command; the git lock already holds the worktree, and `rtok worktree adopt` stores
/// the claim later. An agent that takes the worktree settles its pending claim (T289.5).
pub fn remember(store: Option<&Store>, path: &Path, agent: &str, task: &str) -> Result<()> {
    remember_with(store, path, agent, task, LOCK_ATTEMPTS, LOCK_PAUSE)
}

fn remember_with(
    store: Option<&Store>,
    path: &Path,
    agent: &str,
    task: &str,
    attempts: u32,
    pause: std::time::Duration,
) -> Result<()> {
    let shown = path.display().to_string();
    store_claim(store, path, agent, task, attempts, pause).with_context(|| {
        format!("claim not stored; {shown} stays locked, run `rtok worktree adopt` once the store is free")
    })
}

/// [`remember`] without the advice: a caller that takes the worktree back words its own.
fn store_claim(
    store: Option<&Store>,
    path: &Path,
    agent: &str,
    task: &str,
    attempts: u32,
    pause: std::time::Duration,
) -> Result<()> {
    let path = crate::fs::canon(path).to_string_lossy().into_owned();
    let Some(store) = store else {
        return Ok(());
    };
    crate::store::retry_locked(attempts, pause, || {
        store.claim_worktree(&path, agent, task)?;
        store.take_pending_worktree(&path).map(drop)
    })
    .map_err(Into::into)
}

/// Release the open claim on `path`, retrying like [`remember`]. An unreleased claim keeps its
/// holder looking busy, so the caller reports a failure.
pub fn release(store: &Store, path: &str) -> Result<()> {
    crate::store::retry_locked(LOCK_ATTEMPTS, LOCK_PAUSE, || {
        store.release_worktree_claim(path)
    })
    .context("claim not released")
}

/// `SessionStart` (T289.5): the session `agent`, whose working directory is `cwd`, takes the
/// worktree around it that a post-create script left pending. Returns the worktree's path.
/// Fail open: it makes no git call (the task was named when the claim was parked) and every
/// store error just means nothing was completed.
pub fn complete_pending(store: &Store, cwd: &Path, agent: &str) -> Option<String> {
    let parked = store.pending_worktrees().ok()?;
    if parked.is_empty() {
        return None;
    }
    let here = crate::fs::canon(cwd);
    let (path, task) = super::deepest(&here, parked, |(p, _)| PathBuf::from(p))?;
    // Taking the row first lets only one of two sessions starting together win it.
    if !store.take_pending_worktree(&path).ok()? {
        return None;
    }
    if store.claim_worktree(&path, agent, &task).is_err() {
        let _ = store.add_pending_worktree(&path, &task);
        return None;
    }
    Some(path)
}

/// Seed the new worktree's symbol rows from the main checkout so the next index
/// skips identical blobs. A copy error is a warning: the worktree already exists.
fn copy_symbol_index(store: Option<&Store>, cwd: &Path, dest: &Path) {
    let Some(store) = store else {
        return;
    };
    let listed = match super::git::list(cwd) {
        Ok(listed) => listed,
        Err(e) => {
            crate::log::stderr_ln(&format!("warning: symbol index not copied: {e:#}"));
            return;
        }
    };
    let Some(main) = listed.first() else {
        return;
    };
    let from = crate::store::canon_root(&main.path);
    let to = crate::store::canon_root(dest);
    if from == to {
        return;
    }
    // Main's own `.rtok/` holds the other worktrees; their files are not main's.
    if let Err(e) = store.copy_symbol_rows(&from, &to, &[".rtok"]) {
        crate::log::stderr_ln(&format!("warning: symbol index not copied: {e:#}"));
    }
}

/// T329.6: with `[plugins.graph] auto_add_projects`, the worktree becomes a project named by its
/// branch. Best effort: the lock and the claim already hold the worktree.
fn register_project(store: Option<&Store>, auto_add: bool, path: &Path, branch: Option<&str>) {
    let Some(store) = store.filter(|_| auto_add) else {
        return;
    };
    if let Err(e) = store.auto_add_project(path, Origin::Worktree, branch) {
        crate::log::stderr_ln(&format!("warning: project not registered: {e:#}"));
    }
}

/// `rtok worktree add`, MCP `worktree_add` and the `WorktreeCreate` hook (T159): create the
/// worktree for `agent` (or for `owner` alone when no agent is known) and record the claim.
/// One path, so every surface binds the lock and the store row the same way.
pub fn add(
    store: Option<&Store>,
    cwd: &Path,
    wt: &crate::config::Worktree,
    id: (&str, Option<&str>),
    who: (Option<&AgentDetail>, Option<String>),
    auto_add: bool,
) -> Result<super::add::Plan> {
    add_with(super::add::Seams::HOST, store, cwd, wt, id, who, auto_add)
}

/// [`add`] on stand-ins for the volume and for git. All or nothing (T970): a step that fails
/// after the worktree exists takes back the worktree, its branch and the claim.
pub fn add_with(
    seams: super::add::Seams,
    store: Option<&Store>,
    cwd: &Path,
    wt: &crate::config::Worktree,
    id: (&str, Option<&str>),
    (agent, owner_flag): (Option<&AgentDetail>, Option<String>),
    auto_add: bool,
) -> Result<super::add::Plan> {
    let owner = owner(owner_flag, agent, store)?;
    let agent_id = agent.map(|a| a.id.as_str());
    let added = super::add::run(cwd, wt, seams, id, (&owner, agent_id), store)?;
    // T523: a retry finds its worktree claimed, seeded and registered by the add that made it.
    let (plan, mut undo) = match added {
        super::add::Added::Existing(plan) => return Ok(plan),
        super::add::Added::New(plan, undo) => (plan, undo),
    };
    if let Some(agent) = agent_id {
        if let Some(store) = store {
            undo.claim(store, &plan.path);
        }
        store_claim(
            store,
            &plan.path,
            agent,
            &plan.task,
            LOCK_ATTEMPTS,
            LOCK_PAUSE,
        )
        .with_context(|| format!("claim not stored; {} removed again", plan.path.display()))?;
    }
    // Both are best effort, so the claim goes first: it is the one step that can still fail.
    copy_symbol_index(store, cwd, &plan.path);
    if wt.seed {
        super::seed::from_siblings(cwd, &plan.path, &wt.seed_dirs);
    }
    register_project(store, auto_add, &plan.path, Some(&plan.branch));
    undo.keep();
    Ok(plan)
}

/// What [`run`] bound: the worktree, its task and origin, and whether a git lock was written.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Adopted {
    pub path: PathBuf,
    pub task: String,
    pub origin: &'static str,
    pub locked: bool,
    /// No agent was bound: the claim waits for the next agent in the worktree (T289.5).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub pending: bool,
    /// The branch the worktree has checked out; the project's name when auto-added.
    #[serde(skip)]
    pub branch: Option<String>,
}

/// The main checkout and the linked worktree that holds `path`. The deepest one wins: an
/// agent's cwd may be a subdirectory of it.
fn linked(path: &Path) -> Result<(PathBuf, super::Record)> {
    let target = crate::fs::canon(path);
    let worktrees = git::list(&target)?;
    let main = worktrees
        .first()
        .context("git lists no worktree")?
        .path
        .clone();
    let Some(record) = super::deepest(&target, worktrees.into_iter().skip(1), |r| r.path.clone())
    else {
        bail!("{} is not a linked worktree", path.display());
    };
    Ok((main, record))
}

/// `task`, else the old lock's, else the branch's first `-` segment.
fn task_of(path: &Path, task: Option<&str>, record: &super::Record) -> Result<String> {
    match (task, record.owner(), record.branch.as_deref()) {
        (Some(task), ..) => Ok(task.to_string()),
        (None, Some(o), _) => Ok(o.task),
        (None, None, Some(branch)) => Ok(branch.split('-').next().unwrap_or(branch).to_string()),
        (None, None, None) => bail!(
            "{}: no lock and no branch to name its task; name it with --task (MCP: `task`)",
            path.display()
        ),
    }
}

/// `rtok worktree claim` / `adopt` and MCP `worktree_adopt`: bind the linked worktree that
/// holds `path` to `agent` when it has no lock, the lock is already theirs, or the holder has
/// ended or gone unseen past gc's `--stale-lock` (T527). The task is
/// `task`, else the old lock's, else the branch's first `-` segment. With `spare_evicting` a
/// worktree in a pool its host evicts (T289, [`origin::evicts`]) gets no git lock: the store
/// claim alone binds it.
pub fn run(
    path: &Path,
    (owner, agent): (&str, &str),
    task: Option<&str>,
    spare_evicting: bool,
    store: Option<&Store>,
) -> Result<Adopted> {
    let (main, record) = linked(path)?;
    if let Some(held) = held_against(&record, (owner, agent), store).filter(|h| h.holds) {
        bail!("{} is {}; not taken", path.display(), held.locked());
    }
    let task = task_of(path, task, &record)?;
    let reason = Owner {
        owner: owner.into(),
        task: task.clone(),
        date: super::add::today()?,
        agent: Some(agent.into()),
    }
    .reason();
    anyhow::ensure!(
        Owner::parse(&reason).is_some_and(|o| o.owner == owner) && owner.is_ascii(),
        "owner `{owner}` must be non-empty ASCII without ` | `"
    );
    let origin = origin::of(&record.path);
    let locked = !(spare_evicting && origin::evicts(origin));
    if locked {
        if let Some(old) = &record.locked {
            git::unlock(&main, &record.path)?;
            if let Err(e) = git::lock(&main, &record.path, &reason) {
                git::lock(&main, &record.path, old)?;
                return Err(e);
            }
        } else {
            git::lock(&main, &record.path, &reason)?;
        }
    }
    Ok(Adopted {
        path: record.path.clone(),
        task,
        origin,
        locked,
        pending: false,
        branch: record.branch.clone(),
    })
}

/// [`run`] for a resolved agent, then the store claim: the one path of the CLI and MCP.
pub fn bind(
    store: Option<&Store>,
    path: &Path,
    agent: &AgentDetail,
    owner_flag: Option<String>,
    task: Option<&str>,
    spare_evicting: bool,
    auto_add: bool,
) -> Result<Adopted> {
    let owner = owner(owner_flag, Some(agent), store)?;
    let done = run(path, (&owner, &agent.id), task, spare_evicting, store)?;
    remember(store, &done.path, &agent.id, &done.task)?;
    register_project(store, auto_add, &done.path, done.branch.as_deref());
    Ok(done)
}

/// `adopt` with no agent to name, as from a host's post-create script (T289.3): the one live
/// agent of the pool's host working in the repository takes the worktree. With none or several,
/// guessing would hand the worktree to another agent, so the claim is parked without an agent
/// and without a git lock (T289.5) and the next agent to adopt or start in it takes it.
pub fn bind_unattended(
    store: &Store,
    (path, task): (&Path, Option<&str>),
    idle: &str,
    auto_add: bool,
) -> Result<Adopted> {
    let (main, record) = linked(path)?;
    let hosts = origin::hosts(origin::of(&record.path));
    // Only a pool whose host runs a post-create script has a caller without a session; for any
    // other worktree a missing agent is still the user's mistake.
    if hosts.is_empty() {
        bail!("no agent to bind: pass --agent or set RTOK_AGENT_ID");
    }
    let repo = main.to_string_lossy();
    let host_ids: Vec<i32> = hosts
        .iter()
        .filter_map(|h| store.host_id(h).ok().flatten())
        .collect();
    // The cwd rule of `agents::link::resolve` (T283.1); only top-level sessions, since a
    // sub-agent shares its parent's cwd and would make every repository ambiguous.
    let mut here: Vec<_> = store
        .live_agents(idle)?
        .into_iter()
        .filter(|a| host_ids.contains(&a.host_id) && a.parent_key.is_empty())
        .filter(|a| {
            a.cwd
                .as_deref()
                .is_some_and(|c| crate::fs::same_dir(c, &repo))
        })
        .collect();
    if here.len() == 1 {
        let agent = store
            .agent_detail(&here.remove(0).id)?
            .context("the live agent vanished")?;
        return bind(Some(store), path, &agent, None, task, true, auto_add);
    }
    let task = task_of(path, task, &record)?;
    store.add_pending_worktree(&crate::fs::canon(&record.path).to_string_lossy(), &task)?;
    register_project(
        Some(store),
        auto_add,
        &record.path,
        record.branch.as_deref(),
    );
    Ok(Adopted {
        path: record.path.clone(),
        task,
        origin: origin::of(&record.path),
        locked: false,
        pending: true,
        branch: record.branch.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::worktree::Record;

    fn store_with(last_seen: i64, ended: bool) -> (Store, String) {
        let store = Store::open_in_memory().unwrap();
        let host = store.host_id("claude").unwrap().unwrap();
        let id = store
            .register_agent(host, "s-held", None, None, None)
            .unwrap();
        store.set_agent_last_seen(&id, last_seen).unwrap();
        if ended {
            store.end_agent(&id, last_seen).unwrap();
        }
        (store, id)
    }

    fn locked(agent: &str) -> Record {
        Record {
            locked: Some(format!("claude | t1 | 2026-10-01 | agent {agent}")),
            ..Record::default()
        }
    }

    #[test]
    fn an_agent_flag_holding_a_provider_and_model_says_it_wants_an_id() {
        let err = caller(None, Some("OpenAI / GPT-6"))
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with("--agent OpenAI / GPT-6: not an rtok agent id")
                && err.contains("rtok agents whoami"),
            "{err}"
        );
    }

    #[test]
    fn an_ended_holder_does_not_block_adopt() {
        let now = i64::try_from(crate::log::now()).unwrap();
        let (store, id) = store_with(now, true);
        let held = held_against(&locked(&id), ("other", "other-agent"), Some(&store)).unwrap();
        assert!(!held.holds, "{held:?}");
    }

    #[test]
    fn a_holder_idle_past_stale_lock_does_not_block_adopt() {
        let (store, id) = store_with(0, false);
        let held = held_against(&locked(&id), ("other", "other-agent"), Some(&store)).unwrap();
        assert!(!held.holds, "{held:?}");
    }

    #[test]
    fn a_holder_seen_today_refuses_and_names_when_they_were_seen() {
        let now = i64::try_from(crate::log::now()).unwrap();
        let (store, id) = store_with(now.saturating_sub(60), false);
        let held = held_against(&locked(&id), ("other", "other-agent"), Some(&store)).unwrap();
        assert!(held.holds, "{held:?}");
        let text = held.locked();
        assert!(
            text.starts_with("locked by claude | t1 | 2026-10-01 | agent ")
                && text.contains("holder seen")
                && text.ends_with(" ago"),
            "{text}"
        );
    }

    #[test]
    fn a_lock_the_store_cannot_judge_keeps_the_old_refusal() {
        let record = Record {
            locked: Some("Cursor / grok | t3 | 2026-09-22".into()),
            ..Record::default()
        };
        let held = held_against(&record, ("other", "other-agent"), None).unwrap();
        assert!(held.holds);
        assert_eq!(held.seen_ago, None);
        assert_eq!(held.locked(), "locked by Cursor / grok | t3 | 2026-09-22");
    }
}

/// T934: a claim write under another process's held write lock.
#[cfg(test)]
mod lock_tests {
    use super::*;
    use crate::store::{HeldWrite, LockWait, hold_write_lock};
    use std::time::Duration;

    fn quick_store(tag: &str) -> (Store, std::path::PathBuf, String) {
        let db = crate::testutil::tmp_dir(tag).join("rtok.db");
        let wait = LockWait {
            busy: Duration::from_millis(20),
            ..LockWait::STEADY
        };
        let store = Store::open_with(&db, wait).unwrap();
        let host = store.host_id("claude").unwrap().unwrap();
        let agent = store
            .register_agent(host, "s-lock", None, None, None)
            .unwrap();
        (store, db, agent)
    }

    #[test]
    fn a_claim_waits_out_a_lock_that_ends_inside_the_retries() {
        let (store, db, agent) = quick_store("claim-lock-ends");
        let holder = hold_write_lock(&db, HeldWrite::Immediate, Duration::from_millis(300));
        let path = Path::new("/w/t934");
        remember_with(
            Some(&store),
            path,
            &agent,
            "t934",
            100,
            Duration::from_millis(20),
        )
        .unwrap();
        holder.join().unwrap();
        let claims = store.open_worktree_claims().unwrap();
        assert_eq!(claims.len(), 1, "{claims:?}");
    }

    #[test]
    fn a_claim_that_outlasts_the_retries_fails_the_command() {
        let (store, db, agent) = quick_store("claim-lock-stays");
        let holder = hold_write_lock(&db, HeldWrite::Immediate, Duration::from_millis(600));
        let path = Path::new("/w/t934");
        let err = remember_with(
            Some(&store),
            path,
            &agent,
            "t934",
            2,
            Duration::from_millis(5),
        )
        .unwrap_err();
        let text = format!("{err:#}");
        assert!(
            text.contains("claim not stored") && text.contains("database is locked"),
            "{text}"
        );
        holder.join().unwrap();
        assert!(store.open_worktree_claims().unwrap().is_empty());
    }

    #[test]
    fn a_release_waits_out_a_lock_too() {
        let (store, db, agent) = quick_store("release-lock-ends");
        store.claim_worktree("/w/t934", &agent, "t934").unwrap();
        let holder = hold_write_lock(&db, HeldWrite::Immediate, Duration::from_millis(300));
        let started = std::time::Instant::now();
        release(&store, "/w/t934").unwrap();
        assert!(started.elapsed() >= Duration::from_millis(100));
        holder.join().unwrap();
        assert!(store.open_worktree_claims().unwrap().is_empty());
    }
}
