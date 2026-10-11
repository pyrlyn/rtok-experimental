// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok task export` (T441.13.5, T441.13.6): a one-way mirror of the disk tasks into an
//! issue tracker. The files stay the source of truth; the exporter never reads an issue back
//! into a task. Each backend only lists what the tracker holds and applies one action; the
//! diff, the dry run and the report are shared here.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use self::github::GithubExporter;
use self::linear::LinearExporter;
use super::github::{self as adapter_github, GithubAdapter};
use super::{Task, TaskId, remote};

pub mod github;
pub mod linear;

/// The backends `rtok task export` takes.
pub const BACKENDS: [&str; 2] = ["github", "linear"];

/// The exporter `backend` names, with its target and token resolved. `key` is the project's
/// counter key ([`crate::project::project_key`]), which names the GitHub repo when none is set.
pub fn open(cfg: &crate::config::Tasks, key: &str, backend: &str) -> Result<Box<dyn Exporter>> {
    match backend {
        "github" => {
            let configured = [&cfg.export.github.repo, &cfg.github.repo]
                .into_iter()
                .find(|r| !r.is_empty())
                .map_or("", String::as_str);
            let repo = adapter_github::repo(configured, key)?;
            let token = remote::token(
                "github",
                &["GH_TOKEN", "GITHUB_TOKEN"],
                &["gh", "auth", "token"],
            )?;
            // No Projects v2 board: the mirror is issues only.
            let adapter =
                GithubAdapter::new(adapter_github::API, &repo, &token, remote::WRITE_GAP)?;
            Ok(Box::new(GithubExporter::new(adapter, &repo)))
        }
        "linear" => {
            let team = cfg.export.linear.team.trim();
            if team.is_empty() {
                bail!("linear export: set [tasks.export.linear] team to the team's short key");
            }
            let key = remote::token("linear", &["LINEAR_API_KEY"], &[])?;
            Ok(Box::new(LinearExporter::new(
                linear::API,
                team,
                &key,
                remote::WRITE_GAP,
            )?))
        }
        other => bail!(
            "rtok task export: unknown backend {other:?}; use {}",
            BACKENDS.join(" or ")
        ),
    }
}

/// What a run does to one task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Create,
    Update,
    /// The task is done or closed locally and still open on the tracker.
    Close,
    /// The task is active locally and finished on the tracker.
    Reopen,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Close => "close",
            Self::Reopen => "reopen",
        }
    }
}

/// One write the tracker needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Action {
    pub id: TaskId,
    pub kind: Kind,
    /// What differs, e.g. `title`, `body`, `status`; empty for a create.
    pub fields: Vec<&'static str>,
    /// The issue's link; for a create, known only after the write.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// A tracker the tasks are mirrored into.
pub trait Exporter {
    /// `github` or `linear`, for messages and the report.
    fn name(&self) -> &'static str;
    /// Where the issues go (`owner/name`, a team key), for the report.
    fn target(&self) -> String;
    /// Every rtok issue the tracker holds, read the way the task adapters read them, with
    /// `external` set. Issues no rtok id names are not here: the exporter never touches them.
    fn remote(&self) -> Result<Vec<Task>>;
    /// Make the tracker match `task` for `action`; returns the issue's link.
    fn apply(&self, action: &Action, task: &Task) -> Result<Option<String>>;
    /// Whether the tracker keeps the assignee and the blockers. One that does not must not
    /// have them compared, or every run would see them drift.
    fn keeps_claims(&self) -> bool {
        true
    }
}

/// What a run found or did.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Report {
    pub backend: &'static str,
    pub target: String,
    pub dry_run: bool,
    pub actions: Vec<Action>,
    /// Tasks whose issue already matches.
    pub unchanged: usize,
    /// Finished tasks with no issue: history is not exported.
    pub skipped: usize,
}

/// The `plan`: the actions that bring the tracker in line with `local`, in id order so a
/// parent is created before its subtasks. A finished task with no issue is skipped; an issue
/// with no local task is left alone.
pub fn diff(local: &[Task], remote: &[Task], claims: bool) -> (Vec<Action>, usize, usize) {
    let mut held: BTreeMap<&TaskId, &Task> = BTreeMap::new();
    for t in remote {
        held.entry(&t.id).or_insert(t);
    }
    let mut ordered: Vec<&Task> = local.iter().collect();
    ordered.sort_by(|a, b| a.id.cmp(&b.id));
    // A subtask's issue hangs under its parent's, so a finished parent of an active subtask
    // is exported too (created, then closed) instead of leaving the subtask orphaned.
    let parents: Vec<&TaskId> = local
        .iter()
        .filter(|t| t.status.is_active())
        .filter_map(|t| t.parent.as_ref())
        .collect();
    let (mut actions, mut unchanged, mut skipped) = (Vec::new(), 0, 0);
    for task in ordered {
        let Some(have) = held.get(&task.id) else {
            if task.status.is_active() || parents.contains(&&task.id) {
                actions.push(Action {
                    id: task.id.clone(),
                    kind: Kind::Create,
                    fields: if task.status.is_active() {
                        Vec::new()
                    } else {
                        vec!["status"]
                    },
                    url: None,
                });
            } else {
                skipped += 1;
            }
            continue;
        };
        let fields = drifted(task, have, claims);
        if fields.is_empty() {
            unchanged += 1;
            continue;
        }
        let kind = match (task.status.is_active(), have.status.is_active()) {
            (false, true) => Kind::Close,
            (true, false) => Kind::Reopen,
            _ => Kind::Update,
        };
        actions.push(Action {
            id: task.id.clone(),
            kind,
            fields,
            url: have.external.as_ref().map(|x| x.url.clone()),
        });
    }
    (actions, unchanged, skipped)
}

/// Text compares without the line ending and edge whitespace a web editor adds.
fn text(s: &str) -> String {
    s.replace("\r\n", "\n").trim().to_string()
}

/// The fields of `remote` that no longer match `local`, in the order the report names them.
pub(crate) fn drifted(local: &Task, remote: &Task, claims: bool) -> Vec<&'static str> {
    let mut out = Vec::new();
    if text(&local.title) != text(&remote.title) {
        out.push("title");
    }
    if text(&local.description) != text(&remote.description) {
        out.push("body");
    }
    if local.status != remote.status {
        out.push("status");
    }
    if local.priority != remote.priority {
        out.push("priority");
    }
    if claims {
        if local.assignee != remote.assignee {
            out.push("assignee");
        }
        if local.blocked_by != remote.blocked_by {
            out.push("needs");
        }
        if local.conflicts_with != remote.conflicts_with {
            out.push("conflicts");
        }
        if local.parallel != remote.parallel {
            out.push("parallel");
        }
    }
    out
}

/// Plan against what the tracker holds and, unless `dry_run`, apply it. Stops at the first
/// failed write; the actions before it are done, and a re-run picks up from there.
pub fn run(exporter: &dyn Exporter, local: &[Task], dry_run: bool) -> Result<Report> {
    let remote = exporter
        .remote()
        .with_context(|| format!("{} export: listing the existing issues", exporter.name()))?;
    let (mut actions, unchanged, skipped) = diff(local, &remote, exporter.keeps_claims());
    if !dry_run {
        let by_id: BTreeMap<&TaskId, &Task> = local.iter().map(|t| (&t.id, t)).collect();
        let total = actions.len();
        for (done, action) in actions.iter_mut().enumerate() {
            let task = by_id[&action.id];
            let url = exporter.apply(action, task).with_context(|| {
                format!(
                    "{} export: {} {} failed after {done} of {total} writes",
                    exporter.name(),
                    action.kind.word(),
                    action.id
                )
            })?;
            action.url = url.or(action.url.take());
        }
    }
    Ok(Report {
        backend: exporter.name(),
        target: exporter.target(),
        dry_run,
        actions,
        unchanged,
        skipped,
    })
}

/// `rtok task export`'s text: a summary line, then one line per action.
pub fn render(r: &Report) -> String {
    let count = |k: Kind| r.actions.iter().filter(|a| a.kind == k).count();
    let mut out = format!(
        "{} ({}): {} create, {} update, {} close, {} reopen, {} unchanged",
        r.backend,
        r.target,
        count(Kind::Create),
        count(Kind::Update),
        count(Kind::Close),
        count(Kind::Reopen),
        r.unchanged
    );
    if r.skipped > 0 {
        let _ = write!(out, ", {} finished not exported", r.skipped);
    }
    if r.dry_run {
        out.push_str(" (dry run: nothing sent)");
    }
    out.push('\n');
    for a in &r.actions {
        let _ = write!(out, "{} {}", a.kind.word(), a.id);
        if !a.fields.is_empty() {
            let _ = write!(out, "  ({})", a.fields.join(", "));
        }
        if let Some(url) = &a.url {
            let _ = write!(out, "  {url}");
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::Status;
    use super::*;

    fn task(id: &str, status: Status) -> Task {
        Task {
            status,
            ..Task::open(id.parse().unwrap(), "Title", "Body", 0)
        }
    }

    #[test]
    fn a_missing_issue_is_created_and_a_finished_one_is_skipped() {
        let local = [
            task("R2.1", Status::Open),
            task("R1", Status::Open),
            task("R3", Status::Done),
        ];
        let (actions, unchanged, skipped) = diff(&local, &[], true);
        let ids: Vec<String> = actions.iter().map(|a| a.id.to_string()).collect();
        assert_eq!(ids, ["R1", "R2.1"], "parents first");
        assert!(actions.iter().all(|a| a.kind == Kind::Create));
        assert_eq!((unchanged, skipped), (0, 1));
    }

    #[test]
    fn drift_names_the_fields_and_picks_the_kind() {
        let mut open = task("R1", Status::Open);
        open.description = "New\r\n".into();
        let stale = task("R1", Status::Open);
        let (a, ..) = diff(
            std::slice::from_ref(&open),
            std::slice::from_ref(&stale),
            true,
        );
        assert_eq!(
            (a[0].kind, a[0].fields.clone()),
            (Kind::Update, vec!["body"])
        );

        let done = Task {
            title: "Renamed".into(),
            ..task("R1", Status::Done)
        };
        let (a, ..) = diff(std::slice::from_ref(&done), &[stale], true);
        assert_eq!(a[0].kind, Kind::Close);
        assert_eq!(a[0].fields, ["title", "status"]);

        let (a, ..) = diff(&[open], &[done], true);
        assert_eq!(a[0].kind, Kind::Reopen);
    }

    #[test]
    fn a_matching_issue_is_unchanged_and_claims_are_ignored_when_not_kept() {
        let mut local = task("R1", Status::Open);
        local.assignee = Some("agent".into());
        local.description = " Body\n".into();
        let remote = task("R1", Status::Open);
        let (a, unchanged, _) = diff(
            std::slice::from_ref(&local),
            std::slice::from_ref(&remote),
            false,
        );
        assert!(a.is_empty());
        assert_eq!(unchanged, 1);
        let (a, ..) = diff(&[local], &[remote], true);
        assert_eq!(a[0].fields, ["assignee"]);
    }

    #[test]
    fn linear_needs_a_team_before_anything_else() {
        let err = open(&crate::config::Tasks::default(), "R", "linear")
            .err()
            .expect("an empty team is refused");
        assert!(format!("{err:#}").contains("[tasks.export.linear] team"));
    }

    #[test]
    fn markers_count_as_claims() {
        let mut local = task("R1", Status::Open);
        local.parallel = true;
        local.conflicts_with = vec!["R2".parse().unwrap()];
        let remote = task("R1", Status::Open);
        let (a, ..) = diff(
            std::slice::from_ref(&local),
            std::slice::from_ref(&remote),
            true,
        );
        assert_eq!(a[0].fields, ["conflicts", "parallel"]);
    }

    #[test]
    fn the_report_reads_like_a_plan() {
        let r = Report {
            backend: "github",
            target: "me/app".into(),
            dry_run: true,
            actions: vec![Action {
                id: "R1".parse().unwrap(),
                kind: Kind::Update,
                fields: vec!["title"],
                url: Some("https://x/1".into()),
            }],
            unchanged: 2,
            skipped: 1,
        };
        assert_eq!(
            render(&r),
            "github (me/app): 0 create, 1 update, 0 close, 0 reopen, 2 unchanged, 1 finished not exported (dry run: nothing sent)\nupdate R1  (title)  https://x/1\n"
        );
    }
}
