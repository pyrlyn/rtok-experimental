// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok worktree gc` (T153): remove finished worktrees and their merged branches, and
//! drop the records of worktrees whose directory is already gone. [`decide`] is pure;
//! [`run`] applies one verdict at a time and never forces anything.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rtok_sys::proc::{ProgressRunner, Unit};
use serde::Serialize;

use super::list::newest_until;
use super::{Entry, State, inventory, par_map};
use crate::render::Col;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Merged, clean, idle and not held by anyone else: remove the worktree.
    Remove,
    /// Merged, clean and idle past `stale_lock` under someone else's lock (T418): the
    /// session that took it is gone and nothing is left to lose. Holds the lock's owner.
    Reclaim(String),
    /// A finished task ([`Entry::done`]), idle, that a live agent or someone else's lock
    /// would otherwise keep (T453). Holds what it overrides.
    Finished(String),
    /// The directory is gone: drop this one record (never a blanket `prune`).
    DropRecord,
    Keep(String),
}

pub struct Policy<'a> {
    /// Whose locks may be opened; any other lock holds until it goes stale (T418).
    pub owner: Option<&'a str>,
    pub idle: Duration,
    /// Untouched this long, a foreign lock on a merged, clean worktree is abandoned.
    pub stale_lock: Duration,
    pub now: SystemTime,
    /// Live rtok agents (T282): a worktree bound to one is never removed (T285).
    pub live: std::collections::HashSet<String>,
    /// Worktrees a post-create script adopted before any agent took them (T289.5): no agent
    /// to ask yet, so they are held like a live agent's.
    pub pending: Vec<PathBuf>,
}

impl Policy<'_> {
    fn age(&self, modified: SystemTime) -> Duration {
        self.now.duration_since(modified).unwrap_or_default()
    }
}

/// `modified(within)` yields the newest mtime under the worktree (T151). It is a walk, so it
/// is asked only once no cheaper rule has decided, and `within` is the age that settles the
/// question asked: the walk may stop at the first file younger than that, which answers it
/// as the newest file would. `current` says the command runs from inside the worktree.
pub fn decide(
    entry: &Entry,
    modified: impl FnOnce(Duration) -> Option<SystemTime>,
    current: bool,
    p: &Policy,
) -> Verdict {
    let keep = |why: &str| Verdict::Keep(why.into());
    let record = &entry.record;
    if entry.state == State::Main {
        return keep("main checkout");
    }
    if current {
        return keep("the worktree this command runs from");
    }
    let agent = record.owner().and_then(|o| o.agent);
    let live = agent.filter(|a| p.live.contains(a)).map(|agent| {
        let short: String = agent.chars().take(8).collect();
        format!("agent {short} is live")
    });
    let pending = || {
        let dir = crate::fs::canon(&record.path);
        p.pending.iter().any(|d| crate::fs::same_path(&dir, d))
    };
    let live = live.or_else(|| pending().then(|| "pending claim, no agent has taken it".into()));
    let held = record.held_against(p.owner);
    let owner = || {
        let owner = record.owner().map(|o| o.owner);
        owner.map_or(", owner unknown".into(), |o| format!(" by {o}"))
    };
    // The age of the newest file, asked once on whichever path decides; an unreadable mtime is
    // no evidence of age.
    let age = |within: Duration| modified(within).map(|m| p.age(m));
    // T453: every commit of a finished task is in the base, so neither its lock nor its live
    // agent (a sub-agent's worktree is bound to its long-lived parent) has anything to keep.
    if entry.done && (live.is_some() || held) {
        let why = live.unwrap_or_else(|| format!("locked{}", owner()));
        return match age(p.idle).is_some_and(|a| a >= p.idle) {
            true => Verdict::Finished(why),
            false => Verdict::Keep(why),
        };
    }
    if let Some(why) = live {
        return Verdict::Keep(why);
    }
    if held {
        let owner = owner();
        // Only a merged, clean worktree: a dirty, unmerged or vanished one may still be
        // the only copy of someone's work, so its lock holds however old it is.
        let fresh = p.idle.max(p.stale_lock);
        return match entry.state == State::Merged && age(fresh).is_some_and(|a| a >= fresh) {
            true => Verdict::Reclaim(owner),
            false => keep(&format!("locked{owner}")),
        };
    }
    match entry.state {
        State::Stale => Verdict::DropRecord,
        State::Dirty => keep("uncommitted changes"),
        State::Unmerged => match &record.branch {
            Some(b) => keep(&format!("not merged into the base; check `gh pr view {b}`")),
            None => keep("detached HEAD not merged into the base"),
        },
        _ if age(p.idle).is_some_and(|a| a < p.idle) => keep("modified within the idle window"),
        _ => Verdict::Remove,
    }
}

#[derive(Debug, Serialize)]
pub struct Outcome {
    pub path: String,
    pub branch: Option<String>,
    /// What a removal frees, counted before it runs; 0 unless the action is `remove`.
    pub bytes: u64,
    pub files: u64,
    /// `remove`, `drop-record` or `keep`.
    pub action: &'static str,
    /// Why it was kept, what was done, or git's message when a step failed.
    pub note: String,
    pub failed: bool,
}

impl Verdict {
    /// The action `gc` takes and what it says about it before `--yes`.
    pub fn plan(&self) -> (&'static str, String) {
        match self {
            Verdict::Remove => ("remove", "merged, clean and idle".into()),
            Verdict::Reclaim(owner) => ("remove", format!("merged, clean, abandoned lock{owner}")),
            Verdict::Finished(over) => ("remove", format!("finished task, idle, though {over}")),
            Verdict::DropRecord => ("drop-record", "directory is gone".into()),
            Verdict::Keep(why) => ("keep", why.clone()),
        }
    }
}

/// A worktree of the repository and the verdict `gc` reaches for it.
pub struct Judged {
    pub entry: Entry,
    pub verdict: Verdict,
}

/// Every worktree of the repository `cwd` is in, judged by [`decide`], and the main
/// checkout's path (where removals must run from, since `cwd` may be a worktree that goes).
/// `agents junk` reads the same verdicts, so the two never disagree about what is removable.
pub fn judge(cwd: &Path, policy: &Policy) -> anyhow::Result<(PathBuf, Vec<Judged>)> {
    let here = crate::fs::canon(cwd);
    let entries = inventory(cwd)?;
    let repo = entries
        .first()
        .map_or(cwd, |main| main.record.path.as_path())
        .to_path_buf();
    // The walks are the expensive part: side by side, and stopped at the first recent file.
    let verdicts = par_map(&entries, |_, entry| {
        let path = &entry.record.path;
        let current = path
            .canonicalize()
            .is_ok_and(|p| crate::fs::path_starts_with(&here, &crate::fs::canon(&p)));
        let modified = |within| newest_until(path, |m| policy.age(m) < within);
        decide(entry, modified, current, policy)
    });
    let judged = entries.into_iter().zip(verdicts);
    Ok((
        repo,
        judged
            .map(|(entry, verdict)| Judged { entry, verdict })
            .collect(),
    ))
}

pub fn run(cwd: &Path, policy: &Policy, yes: bool) -> anyhow::Result<Vec<Outcome>> {
    let (repo, judged) = judge(cwd, policy)?;
    let acting = |v: &Verdict| yes && v.plan().0 != "keep";
    let count = judged.iter().filter(|j| acting(&j.verdict)).count();
    // One removal keeps its own spinner; several get a bar over the worktrees (T276.2).
    let mut bar = (count > 1)
        .then(|| ProgressRunner::batch("removing worktrees…", count as u64, Unit::Items));
    // Removals stay one at a time: each takes the repository's locks.
    let mut outcomes = Vec::with_capacity(judged.len());
    for Judged { entry, verdict } in &judged {
        let (action, planned) = verdict.plan();
        let (bytes, files) = match action {
            "remove" => crate::removal_preview::size_of(&entry.record.path),
            _ => (0, 0),
        };
        let done = acting(verdict).then(|| {
            if let Some(bar) = bar.as_mut() {
                bar.step(
                    &entry
                        .record
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                );
            }
            super::remove::detach(&repo, &entry.record, entry.merged)
        });
        outcomes.push(Outcome {
            path: entry.record.path.display().to_string(),
            branch: entry.record.branch.clone(),
            bytes,
            files,
            action,
            failed: matches!(done, Some(Err(_))),
            note: match done {
                Some(Ok(note)) => note,
                Some(Err(e)) => format!("failed, kept: {e:#}"),
                None => planned,
            },
        });
    }
    if let Some(bar) = bar {
        bar.finish();
    }
    Ok(outcomes)
}

pub fn to_table(outcomes: &[Outcome], yes: bool) -> String {
    let dash = || "-".to_string();
    let mut lines = vec![["action", "path", "branch"].map(String::from).to_vec()];
    lines.extend(outcomes.iter().map(|o| {
        let branch = o.branch.clone().unwrap_or_else(dash);
        vec![o.action.into(), o.path.clone(), branch]
    }));
    let cols = [Col::left(0), Col::left(0), Col::right(0)];
    let notes = outcomes.iter().map(|o| o.note.as_str());
    let mut out = super::noted_table(&cols, &lines, notes);
    let planned = outcomes.iter().filter(|o| o.action != "keep").count();
    if !yes && planned > 0 {
        let removals = outcomes.iter().filter(|o| o.action == "remove");
        let removals = removals.map(|o| (o.path.as_str(), o.bytes, o.files));
        let preview = crate::removal_preview::render(removals);
        if !preview.is_empty() {
            out.push('\n');
            out.push_str(&preview);
        }
        out.push_str(&format!(
            "\ndry run: {planned} to act on, nothing changed; rerun with --yes\n"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::Record;
    use super::*;
    use rstest::rstest;

    fn at(secs: u64) -> SystemTime {
        std::time::UNIX_EPOCH + Duration::from_secs(secs)
    }

    const ME: &str = "Claude Code / sonnet";
    const MINE: &str = "Claude Code / sonnet | t1 | 2026-09-22";
    const THEIRS: &str = "Cursor / grok | t2 | 2026-09-22";

    fn label(verdict: Verdict) -> String {
        match verdict {
            Verdict::Remove => "remove".to_string(),
            Verdict::Reclaim(owner) => format!("reclaim:{owner}"),
            Verdict::Finished(over) => format!("finished:{over}"),
            Verdict::DropRecord => "drop-record".to_string(),
            Verdict::Keep(why) => format!("keep:{why}"),
        }
    }

    #[rstest]
    #[case::main(State::Main, None, false, 0, "keep:main checkout")]
    #[case::current(
        State::Merged,
        None,
        true,
        0,
        "keep:the worktree this command runs from"
    )]
    #[case::finished(State::Merged, None, false, 0, "remove")]
    #[case::my_lock(State::Merged, Some(MINE), false, 0, "remove")]
    #[case::foreign_lock(
        State::Merged,
        Some(THEIRS),
        false,
        600,
        "keep:locked by Cursor / grok"
    )]
    #[case::bare_lock(State::Merged, Some(""), false, 600, "keep:locked, owner unknown")]
    #[case::abandoned_lock(State::Merged, Some(THEIRS), false, 0, "reclaim: by Cursor / grok")]
    #[case::abandoned_bare_lock(State::Merged, Some(""), false, 0, "reclaim:, owner unknown")]
    #[case::dirty_theirs(State::Dirty, Some(THEIRS), false, 0, "keep:locked by Cursor / grok")]
    #[case::unmerged_theirs(
        State::Unmerged,
        Some(THEIRS),
        false,
        0,
        "keep:locked by Cursor / grok"
    )]
    #[case::active(
        State::Merged,
        None,
        false,
        990,
        "keep:modified within the idle window"
    )]
    #[case::live_agent(
        State::Merged,
        Some("Claude Code / sonnet | t1 | 2026-09-22 | agent 0193ab12-live"),
        false,
        0,
        "keep:agent 0193ab12 is live"
    )]
    #[case::ended_agent(
        State::Merged,
        Some("Claude Code / sonnet | t1 | 2026-09-22 | agent 0193ab12-gone"),
        false,
        0,
        "remove"
    )]
    #[case::dirty(State::Dirty, None, false, 0, "keep:uncommitted changes")]
    #[case::unmerged(
        State::Unmerged,
        None,
        false,
        0,
        "keep:not merged into the base; check `gh pr view t1`"
    )]
    #[case::gone(State::Stale, None, false, 0, "drop-record")]
    #[case::gone_mine(State::Stale, Some(MINE), false, 0, "drop-record")]
    #[case::gone_theirs(State::Stale, Some(THEIRS), false, 0, "keep:locked by Cursor / grok")]
    #[case::gone_bare_lock(State::Stale, Some(""), false, 0, "keep:locked, owner unknown")]
    fn verdicts(
        #[case] state: State,
        #[case] locked: Option<&str>,
        #[case] current: bool,
        #[case] modified: u64,
        #[case] want: &str,
    ) {
        let record = Record {
            branch: Some("t1".into()),
            locked: locked.map(Into::into),
            ..Record::default()
        };
        let entry = Entry {
            record,
            state,
            merged: state == State::Merged,
            done: false,
        };
        let policy = Policy {
            owner: Some(ME),
            idle: Duration::from_secs(100),
            stale_lock: Duration::from_secs(500),
            now: at(1_000),
            live: ["0193ab12-live".to_string()].into(),
            pending: Vec::new(),
        };
        let asked = std::cell::Cell::new(None);
        let walk = |within| {
            asked.set(Some(within));
            Some(at(modified))
        };
        let got = label(decide(&entry, walk, current, &policy));
        assert_eq!(got, want);
        // The walk runs only where its answer can still change the verdict, told the age
        // that settles it: the idle window, or the stale-lock age under a foreign lock.
        let held = want.starts_with("reclaim:") || want.starts_with("keep:locked");
        let within = match want {
            "remove" | "keep:modified within the idle window" => Some(100),
            _ if held && state == State::Merged => Some(500),
            _ => None,
        };
        assert_eq!(asked.get(), within.map(Duration::from_secs), "{want}");
    }

    #[test]
    fn without_an_owner_every_lock_is_a_hard_stop() {
        let record = Record {
            locked: Some(MINE.into()),
            ..Record::default()
        };
        let entry = Entry {
            record,
            state: State::Merged,
            merged: true,
            done: false,
        };
        let policy = Policy {
            owner: None,
            idle: Duration::ZERO,
            stale_lock: Duration::ZERO,
            now: at(1_000),
            live: Default::default(),
            pending: Vec::new(),
        };
        let verdict = decide(&entry, |_| None, false, &policy);
        assert_eq!(verdict, Verdict::Keep(format!("locked by {ME}")));
    }

    const LIVE: &str = "Cursor / grok | t1 | 2026-09-22 | agent 0193ab12-live";

    /// T453: a finished task goes past a live agent or a foreign lock once idle; inside the
    /// idle window, or with an unreadable mtime, it keeps the reason it had before.
    #[rstest]
    #[case::live(Some(LIVE), Some(0), "finished:agent 0193ab12 is live")]
    #[case::live_busy(Some(LIVE), Some(990), "keep:agent 0193ab12 is live")]
    #[case::theirs(Some(THEIRS), Some(0), "finished:locked by Cursor / grok")]
    #[case::theirs_busy(Some(THEIRS), Some(990), "keep:locked by Cursor / grok")]
    #[case::theirs_unreadable(Some(THEIRS), None, "keep:locked by Cursor / grok")]
    #[case::bare_lock(Some(""), Some(0), "finished:locked, owner unknown")]
    #[case::mine(Some(MINE), Some(0), "remove")]
    #[case::unlocked(None, Some(0), "remove")]
    fn a_finished_task_is_kept_by_no_lock_and_no_live_agent(
        #[case] locked: Option<&str>,
        #[case] modified: Option<u64>,
        #[case] want: &str,
    ) {
        let record = Record {
            branch: Some("t1".into()),
            locked: locked.map(Into::into),
            ..Record::default()
        };
        let entry = Entry {
            record,
            state: State::Merged,
            merged: true,
            done: true,
        };
        let policy = Policy {
            owner: Some(ME),
            idle: Duration::from_secs(100),
            stale_lock: Duration::from_secs(500),
            now: at(1_000),
            live: ["0193ab12-live".to_string()].into(),
            pending: Vec::new(),
        };
        let got = label(decide(&entry, |_| modified.map(at), false, &policy));
        assert_eq!(got, want);
    }

    /// T289.5: a worktree a post-create script parked for the next agent is not an orphan: it is
    /// kept, and goes only once its finished task has been idle.
    #[rstest]
    #[case::unfinished(false, Some(0), "keep:pending claim, no agent has taken it")]
    #[case::finished_busy(true, Some(990), "keep:pending claim, no agent has taken it")]
    #[case::finished_idle(true, Some(0), "finished:pending claim, no agent has taken it")]
    fn a_pending_claim_is_not_an_orphan(
        #[case] done: bool,
        #[case] modified: Option<u64>,
        #[case] want: &str,
    ) {
        let record = Record {
            path: "/w/parked".into(),
            branch: Some("t1".into()),
            ..Record::default()
        };
        let entry = Entry {
            record,
            state: State::Merged,
            merged: true,
            done,
        };
        let policy = Policy {
            owner: Some(ME),
            idle: Duration::from_secs(100),
            stale_lock: Duration::from_secs(500),
            now: at(1_000),
            live: Default::default(),
            pending: vec!["/w/parked".into()],
        };
        assert_eq!(
            label(decide(&entry, |_| modified.map(at), false, &policy)),
            want
        );
    }
}
