// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The folder layout of the `disk` adapter (T955): one folder per task, `<dir>/<id>/task.md`,
//! and a `<dir>/<id>/done.md` closing report whose presence means the task is closed. This is
//! the layout of the `plan/` directories the project plan skill describes.
//!
//! `task.md` is the author's file, so a write changes only the `status`, `priority` and `agent`
//! lines of its front matter; `complexity`, `readiness`, the heading and the body stay byte for
//! byte. What the layout has no key for (blockers, conflicts, the parallel marker) is refused
//! instead of dropped.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result, bail};

use super::adapter::{ClaimLock, Filter, Taken, TaskAdapter};
use super::disk::{lock_claim, now, warn_skipped};
use super::{NewTask, Status, Task, TaskId};

const TASK: &str = "task.md";
const DONE: &str = "done.md";

/// Highest priority the front matter can carry (`P3`); rtok's `4` has no `P` word.
const PRIORITY_P_MAX: u8 = 3;

/// `done.md` of a closed ("won't do") task starts with this; a done task's report does not.
const DROPPED: &str = "Dropped";

/// The `complexity` and `readiness` of a task created through rtok; its author sets real ones.
const NEW_HEAD: &str = "complexity: 3\nreadiness: 0%";

pub struct PlanDirAdapter {
    dir: PathBuf,
}

impl PlanDirAdapter {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Whether `dir` already holds the folder layout: some `<id>/task.md` in it.
    pub fn holds_folders(dir: &Path) -> bool {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| e.path().join(TASK).is_file())
    }

    fn folder(&self, id: &TaskId) -> PathBuf {
        self.dir.join(id.to_string())
    }

    /// The task in `folder`, or `None` when it has no `task.md`.
    fn read(&self, id: &TaskId) -> Result<Option<Task>> {
        let folder = self.folder(id);
        let path = folder.join(TASK);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("disk tasks: {}", path.display())),
        };
        let done_path = folder.join(DONE);
        let done = match std::fs::read_to_string(&done_path) {
            Ok(done) => Some(done),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(e).with_context(|| format!("disk tasks: {}", done_path.display()));
            }
        };
        let stamped = if done.is_some() { &done_path } else { &path };
        let task = parse(id, &text, done.as_deref(), mtime(stamped))
            .with_context(|| format!("disk tasks: {}", path.display()))?;
        Ok(Some(task))
    }

    /// Every task folder in `dir` that parses, and the ids of the ones that do not. Names that
    /// are not task ids (`README.md`, a draft folder, an id past the supported depth) stay
    /// theirs; `tests/plan_dir.rs` is what polices them.
    fn scan(&self) -> Result<(Vec<Task>, Vec<TaskId>)> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Vec::new(), Vec::new()));
            }
            Err(e) => {
                return Err(e).with_context(|| format!("disk tasks: {}", self.dir.display()));
            }
        };
        let (mut found, mut skipped) = (Vec::new(), Vec::new());
        for entry in entries {
            let path = entry?.path();
            let Some(id) = folder_id(&path) else {
                continue;
            };
            // One malformed task.md must not take the whole plan down; a command that names
            // its id still gets the error from `read`.
            match self.read(&id) {
                Ok(Some(task)) => found.push(task),
                Ok(None) => {}
                Err(e) => {
                    warn_skipped(&e);
                    skipped.push(id);
                }
            }
        }
        Ok((found, skipped))
    }

    /// Rewrite the three keys of `id`'s `task.md` from `task`. The caller holds the claim lock.
    fn store(&self, id: &TaskId, task: &Task) -> Result<()> {
        if !task.blocked_by.is_empty() || !task.conflicts_with.is_empty() || task.parallel {
            bail!(
                "disk tasks: {id}: the folder layout has no place for blockers, conflicts or the \
                 parallel marker"
            );
        }
        let path = self.folder(id).join(TASK);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("disk tasks: {}", path.display()))?;
        let text = with_head(&text, task).with_context(|| format!("{id}"))?;
        fs_kit::write::atomic(&path, text)?;
        Ok(())
    }

    fn closed_as(&self, id: &TaskId) -> Result<Option<Status>> {
        Ok(self.read(id)?.map(|t| t.status).filter(|s| !s.is_active()))
    }
}

impl TaskAdapter for PlanDirAdapter {
    fn name(&self) -> &'static str {
        "disk"
    }

    fn create(&self, task: &NewTask, id: &TaskId) -> Result<Task> {
        let title = task.title.trim();
        if title.is_empty() || title.contains(['\n', '\r']) {
            bail!("disk tasks: a task needs a one-line title");
        }
        let folder = self.folder(id);
        if folder.join(TASK).exists() {
            return Err(anyhow::Error::new(Taken(id.clone()))
                .context(format!("disk tasks: {}", self.dir.display())));
        }
        let mut body = task.description.trim_end().to_string();
        // `tests/plan_dir.rs` wants exactly one `Check:` line on an active task; a task made
        // without one gets a prompt to fill in rather than failing the check.
        if !body.lines().any(|l| l.starts_with("Check:")) {
            if !body.is_empty() {
                body.push_str("\n\n");
            }
            body.push_str("Check: say how to prove this task is done.");
        }
        let text = format!(
            "---\nstatus: todo\npriority: P{}\n{NEW_HEAD}\nagent:\n---\n\n# {id}. {title}\n\n{body}\n",
            super::DEFAULT_PRIORITY
        );
        std::fs::create_dir_all(&folder)
            .with_context(|| format!("disk tasks: {}", folder.display()))?;
        fs_kit::write::atomic(folder.join(TASK), text)?;
        self.read(id)?
            .with_context(|| format!("disk tasks: {id} vanished after create"))
    }

    fn list(&self, filter: &Filter) -> Result<Vec<Task>> {
        Ok(filter.select(self.scan()?.0))
    }

    fn get(&self, id: &TaskId) -> Result<Option<Task>> {
        self.read(id)
    }

    fn write_status(&self, id: &TaskId, status: Status) -> Result<Task> {
        let _guard = lock_claim(&self.dir)?;
        let Some(mut task) = self.read(id)? else {
            bail!("disk tasks: no task {id} in {}", self.dir.display());
        };
        if task.status == status {
            return Ok(task);
        }
        if !task.status.is_active() {
            bail!(
                "disk tasks: {id} is {} and has a done.md; reopening it is the creator's call, \
                 remove {} yourself",
                task.status,
                self.folder(id).join(DONE).display()
            );
        }
        if status.is_active() {
            task.status = status;
            if status == Status::Open {
                task.assignee = None;
            }
            self.store(id, &task)?;
        } else {
            let text = match status {
                Status::Closed => format!("{DROPPED}: closed with `rtok task status`.\n"),
                _ => "Done: closed with `rtok task status`.\n".to_string(),
            };
            fs_kit::write::atomic(self.folder(id).join(DONE), text)?;
        }
        self.read(id)?
            .with_context(|| format!("disk tasks: {id} vanished"))
    }

    fn write_report(&self, id: &TaskId, report: &str) -> Result<()> {
        let _guard = lock_claim(&self.dir)?;
        let Some(status) = self.closed_as(id)? else {
            bail!("disk tasks: {id} is not finished, so it has no closing report");
        };
        let report = report.trim();
        if report.is_empty() {
            bail!("disk tasks: the closing report is empty");
        }
        let text = if status == Status::Closed && !report.starts_with(DROPPED) {
            format!("{DROPPED}: {report}\n")
        } else {
            format!("{report}\n")
        };
        fs_kit::write::atomic(self.folder(id).join(DONE), text)?;
        Ok(())
    }

    fn claim_guard(&self) -> Result<Box<dyn ClaimLock>> {
        Ok(Box::new(lock_claim(&self.dir)?))
    }

    fn save(&self, task: &Task) -> Result<Task> {
        let Some(old) = self.read(&task.id)? else {
            bail!("disk tasks: no task {} in {}", task.id, self.dir.display());
        };
        if !old.status.is_active() || !task.status.is_active() {
            bail!(
                "disk tasks: {} is finished; its done.md is the record",
                task.id
            );
        }
        let mut task = task.clone();
        if task.assignee.as_deref().is_some_and(str::is_empty) {
            task.assignee = None;
        }
        self.store(&task.id, &task)?;
        self.read(&task.id)?
            .with_context(|| format!("disk tasks: {} vanished", task.id))
    }

    fn max_id(&self, prefix: &str) -> Result<Option<TaskId>> {
        let prefix = prefix.to_ascii_uppercase();
        let (found, skipped) = self.scan()?;
        // A skipped folder still owns its name, or the next task would be given its id.
        Ok(found
            .into_iter()
            .map(|t| t.id)
            .chain(skipped)
            .filter(|id| id.prefix() == prefix)
            .max())
    }
}

/// The id a folder is named after, when the name is exactly that id's spelling.
fn folder_id(path: &Path) -> Option<TaskId> {
    if !path.is_dir() {
        return None;
    }
    let name = path.file_name()?.to_str()?;
    let id: TaskId = name.parse().ok()?;
    (id.to_string() == name).then_some(id)
}

fn mtime(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or_else(now, |d| d.as_secs() as i64)
}

/// `(front matter, body)` of a `task.md`.
fn split(text: &str) -> Result<(&str, &str)> {
    let Some(rest) = text.strip_prefix("---\n") else {
        bail!("no front matter (the file must start with ---)");
    };
    let Some(end) = rest.find("\n---\n") else {
        bail!("front matter is not closed with ---");
    };
    Ok((&rest[..end], &rest[end + 5..]))
}

fn value<'a>(head: &'a str, key: &str) -> Option<&'a str> {
    head.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
        .map(str::trim)
}

fn parse(id: &TaskId, text: &str, done: Option<&str>, stamp: i64) -> Result<Task> {
    // A finished task's task.md is history, and the oldest ones predate the front matter: they
    // only have to not stop the listing.
    let finished = done.is_some();
    let (head, body) = match split(text) {
        Ok(parts) => parts,
        Err(_) if finished => ("", text),
        Err(e) => return Err(e),
    };
    let heading = format!("# {id}. ");
    let mut lines = body.lines().skip_while(|l| !l.starts_with("# "));
    let title = match lines.next().and_then(|l| l.strip_prefix(&heading)) {
        Some(title) => title,
        None if finished => "",
        None => bail!("no `{heading}Title` heading"),
    };
    let title = if title.trim().is_empty() && finished {
        id.to_string()
    } else {
        title.trim().to_string()
    };
    let description = lines.collect::<Vec<_>>().join("\n").trim().to_string();

    let (status, priority, assignee) = match done {
        Some(done) => {
            let status = if done.trim_start().starts_with(DROPPED) {
                Status::Closed
            } else {
                Status::Done
            };
            (status, super::DEFAULT_PRIORITY, None)
        }
        None => {
            let status = match value(head, "status") {
                Some("todo") => Status::Open,
                Some("in progress") => Status::InProgress,
                other => bail!("status {other:?}, want todo or in progress"),
            };
            let priority = value(head, "priority")
                .and_then(|p| p.strip_prefix('P')?.parse::<u8>().ok())
                .filter(|p| *p <= PRIORITY_P_MAX);
            let Some(priority) = priority else {
                bail!("priority {:?}, want P0..P3", value(head, "priority"));
            };
            let agent = value(head, "agent").filter(|a| !a.is_empty());
            (status, priority, agent.map(str::to_string))
        }
    };
    Ok(Task {
        description,
        status,
        priority,
        assignee,
        created_at: stamp,
        updated_at: stamp,
        ..Task::open(id.clone(), title, "", 0)
    })
}

/// `text` with its `status`, `priority` and `agent` lines set from `task`.
fn with_head(text: &str, task: &Task) -> Result<String> {
    let status = match task.status {
        Status::Open => "todo",
        Status::InProgress => "in progress",
        _ => bail!("only todo and in progress are front matter statuses"),
    };
    if task.priority > PRIORITY_P_MAX {
        bail!("priority {} has no P word (P0..P3)", task.priority);
    }
    let agent = task.assignee.as_deref().unwrap_or_default();
    if agent.contains(['\n', '\r']) {
        bail!("the agent name must be one line");
    }
    // `tests/plan_dir.rs`: an agent exactly when the task is in progress.
    if agent.is_empty() == (task.status == Status::InProgress) {
        bail!("an in-progress task needs an agent and a todo one has none; use `rtok task claim`");
    }
    let (head, body) = split(text)?;
    let mut seen = 0;
    let lines: Vec<String> = head
        .lines()
        .map(|l| {
            let key = l.split(':').next().unwrap_or_default();
            let new = match key {
                "status" => format!("status: {status}"),
                "priority" => format!("priority: P{}", task.priority),
                "agent" if agent.is_empty() => "agent:".to_string(),
                "agent" => format!("agent: {agent}"),
                _ => return l.to_string(),
            };
            seen += 1;
            new
        })
        .collect();
    if seen != 3 {
        bail!("front matter lacks status, priority or agent");
    }
    Ok(format!("---\n{}\n---\n{body}", lines.join("\n")))
}

/// The rules `tests/plan_dir.rs` holds this repository's plan to; the tests below hold what the
/// adapter writes to the same ones.
#[cfg(test)]
#[path = "../../tests/common/plan_rules.rs"]
mod plan_rules;

#[cfg(test)]
mod tests {
    use super::super::adapter::set_status;
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rtok-plandir-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("plan")
    }

    fn id(s: &str) -> TaskId {
        s.parse().unwrap()
    }

    fn new(title: &str, description: &str) -> NewTask {
        NewTask {
            title: title.into(),
            description: description.into(),
            parent: None,
        }
    }

    const HAND_MADE: &str = "---\nstatus: todo\npriority: P1\ncomplexity: 4\nreadiness: 30%\nagent:\n---\n\n# T7. Hand made\n\nWhy.\n\nCheck: run it.\n";

    /// The plan rules' complaints about every folder of `dir`.
    fn plan_errors(dir: &Path) -> Vec<String> {
        let mut errs = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                errs.extend(plan_rules::folder_errors(&path));
            }
        }
        errs
    }

    #[test]
    fn create_claim_and_close_leave_a_plan_that_passes_the_plan_rules() {
        let dir = tmp("flow");
        let disk = PlanDirAdapter::new(dir.clone());
        let made = disk
            .create(
                &new("Ship it", "Why it exists.\n\nCheck: run it."),
                &id("T1"),
            )
            .unwrap();
        assert_eq!(
            (made.title.as_str(), made.status),
            ("Ship it", Status::Open)
        );
        assert_eq!(made.description, "Why it exists.\n\nCheck: run it.");
        disk.create(&new("Sub", ""), &id("T1.1")).unwrap();
        assert_eq!(plan_errors(&dir), Vec::<String>::new(), "created");
        assert_eq!(
            disk.get(&id("T1.1")).unwrap().unwrap().parent,
            Some(id("T1"))
        );
        assert!(
            disk.create(&new("Again", ""), &id("T1")).is_err(),
            "id taken"
        );
        assert!(disk.create(&new("two\nlines", ""), &id("T2")).is_err());

        let mut task = disk.get(&id("T1")).unwrap().unwrap();
        task.status = Status::InProgress;
        task.assignee = Some("Claude Code / opus".into());
        let claimed = disk.save(&task).unwrap();
        assert_eq!(claimed.assignee.as_deref(), Some("Claude Code / opus"));
        assert_eq!(plan_errors(&dir), Vec::<String>::new(), "claimed");

        let text = std::fs::read_to_string(dir.join("T1/task.md")).unwrap();
        assert!(text.starts_with(
            "---\nstatus: in progress\npriority: P2\ncomplexity: 3\nreadiness: 0%\nagent: Claude Code / opus\n---\n"
        ));
        assert!(text.contains("# T1. Ship it\n"), "{text}");

        set_status(&disk, &id("T1.1"), Status::Closed, false).unwrap();
        disk.write_report(&id("T1.1"), "not needed").unwrap();
        set_status(&disk, &id("T1"), Status::Done, false).unwrap();
        disk.write_report(&id("T1"), "Shipped in a PR.").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("T1.1/done.md")).unwrap(),
            "Dropped: not needed\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("T1/done.md")).unwrap(),
            "Shipped in a PR.\n"
        );
        // Closing leaves task.md as it was.
        assert_eq!(
            std::fs::read_to_string(dir.join("T1/task.md")).unwrap(),
            text
        );

        assert_eq!(plan_errors(&dir), Vec::<String>::new(), "closed");
        assert!(disk.list(&Filter::default()).unwrap().is_empty());
        let all = disk
            .list(&Filter {
                all: true,
                ..Filter::default()
            })
            .unwrap();
        let states: Vec<_> = all.iter().map(|t| (t.id.to_string(), t.status)).collect();
        assert_eq!(
            states,
            [("T1".into(), Status::Done), ("T1.1".into(), Status::Closed)]
        );
        assert_eq!(disk.max_id("t").unwrap(), Some(id("T1.1")));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn a_created_task_has_the_shape_the_plan_check_wants() {
        let dir = tmp("shape");
        let disk = PlanDirAdapter::new(dir.clone());
        disk.create(&new("Bare", ""), &id("T3")).unwrap();
        let text = std::fs::read_to_string(dir.join("T3/task.md")).unwrap();
        let (head, body) = split(&text).unwrap();
        let keys: Vec<&str> = head.lines().map(|l| l.split(':').next().unwrap()).collect();
        assert_eq!(
            keys,
            ["status", "priority", "complexity", "readiness", "agent"]
        );
        assert_eq!(body.lines().filter(|l| l.starts_with("Check:")).count(), 1);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn a_save_changes_three_lines_and_nothing_else() {
        let dir = tmp("bytes");
        std::fs::create_dir_all(dir.join("T7")).unwrap();
        std::fs::write(dir.join("T7/task.md"), HAND_MADE).unwrap();
        let disk = PlanDirAdapter::new(dir.clone());
        let mut task = disk.get(&id("T7")).unwrap().unwrap();
        assert_eq!((task.priority, task.title.as_str()), (1, "Hand made"));
        task.status = Status::InProgress;
        task.assignee = Some("A / b".into());
        task.priority = 0;
        disk.save(&task).unwrap();
        let want = HAND_MADE
            .replace("status: todo", "status: in progress")
            .replace("priority: P1", "priority: P0")
            .replace("agent:\n", "agent: A / b\n");
        assert_eq!(
            std::fs::read_to_string(dir.join("T7/task.md")).unwrap(),
            want
        );
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn what_the_layout_cannot_store_is_refused() {
        let dir = tmp("refuse");
        let disk = PlanDirAdapter::new(dir.clone());
        disk.create(&new("A", ""), &id("T1")).unwrap();
        let task = disk.get(&id("T1")).unwrap().unwrap();
        for bad in [
            Task {
                blocked_by: vec![id("T2")],
                ..task.clone()
            },
            Task {
                parallel: true,
                ..task.clone()
            },
            Task {
                priority: 4,
                ..task.clone()
            },
            Task {
                status: Status::InProgress,
                ..task.clone()
            },
            Task {
                assignee: Some("x".into()),
                ..task.clone()
            },
            Task {
                status: Status::InProgress,
                assignee: Some("x\ny: 1".into()),
                ..task.clone()
            },
        ] {
            assert!(disk.save(&bad).is_err(), "{bad:?}");
        }
        assert!(!disk.stores_markers());
        assert_eq!(disk.get(&id("T1")).unwrap().unwrap(), task);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn a_closed_task_is_not_reopened_by_a_command() {
        let dir = tmp("reopen");
        let disk = PlanDirAdapter::new(dir.clone());
        disk.create(&new("A", ""), &id("T1")).unwrap();
        set_status(&disk, &id("T1"), Status::Done, false).unwrap();
        let err = disk.write_status(&id("T1"), Status::Open).unwrap_err();
        assert!(err.to_string().contains("done.md"), "{err}");
        assert!(disk.write_report(&id("T9"), "x").is_err());
        disk.create(&new("B", ""), &id("T2")).unwrap();
        assert!(disk.write_report(&id("T2"), "x").is_err(), "not finished");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn folders_that_are_not_tasks_are_left_alone() {
        let dir = tmp("foreign");
        let disk = PlanDirAdapter::new(dir.clone());
        assert!(
            disk.list(&Filter::default()).unwrap().is_empty(),
            "no dir yet"
        );
        disk.create(&new("Real", ""), &id("T1")).unwrap();
        std::fs::write(dir.join("README.md"), "# plan\n").unwrap();
        for name in ["P8b", "draft", "t5", "T0.1"] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            std::fs::write(dir.join(name).join(TASK), "no front matter\n").unwrap();
        }
        assert_eq!(disk.list(&Filter::default()).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn a_broken_task_is_skipped_by_reads_and_fails_by_its_id() {
        let dir = tmp("broken");
        let disk = PlanDirAdapter::new(dir.clone());
        disk.create(&new("Real", ""), &id("T1")).unwrap();
        std::fs::create_dir_all(dir.join("T4")).unwrap();
        // Valid front matter, but no `# T4. Title` heading.
        std::fs::write(dir.join("T4/task.md"), HAND_MADE).unwrap();
        std::fs::create_dir_all(dir.join("T9")).unwrap();
        std::fs::write(dir.join("T9/task.md"), "no front matter\n").unwrap();
        disk.create(&new("Also real", ""), &id("T2")).unwrap();

        let listed: Vec<_> = disk
            .list(&Filter::default())
            .unwrap()
            .iter()
            .map(|t| t.id.to_string())
            .collect();
        assert_eq!(listed.len(), 2);
        assert!(listed.contains(&"T1".into()) && listed.contains(&"T2".into()));
        assert_eq!(disk.get(&id("T1")).unwrap().unwrap().title, "Real");

        let err = disk.get(&id("T4")).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains(&format!("T4{}task.md", std::path::MAIN_SEPARATOR)),
            "{msg}"
        );
        assert!(msg.contains("heading"), "{msg}");

        // The skipped folder keeps its id, and nothing writes into it.
        assert_eq!(disk.max_id("T").unwrap(), Some(id("T9")));
        assert!(disk.create(&new("Again", ""), &id("T9")).is_err());
        let before = std::fs::read(dir.join("T9/task.md")).unwrap();
        assert!(disk.write_status(&id("T9"), Status::InProgress).is_err());
        assert!(disk.write_status(&id("T9"), Status::Done).is_err());
        assert!(disk.write_report(&id("T9"), "x").is_err());
        assert!(disk.save(&Task::open(id("T9"), "x", "", 0)).is_err());
        assert_eq!(std::fs::read(dir.join("T9/task.md")).unwrap(), before);
        assert!(!dir.join("T9/done.md").exists());
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn a_finished_task_with_old_front_matter_still_lists() {
        let dir = tmp("old-done");
        std::fs::create_dir_all(dir.join("T2")).unwrap();
        std::fs::write(
            dir.join("T2/task.md"),
            "---\nstatus: done\nodd: 1\n---\n\n# T2. Old\n",
        )
        .unwrap();
        std::fs::write(dir.join("T2/done.md"), "Done in a PR.\n").unwrap();
        let disk = PlanDirAdapter::new(dir.clone());
        let got = disk.get(&id("T2")).unwrap().unwrap();
        assert_eq!((got.title.as_str(), got.status), ("Old", Status::Done));
        std::fs::create_dir_all(dir.join("T3")).unwrap();
        std::fs::write(dir.join("T3/task.md"), "no front matter at all\n").unwrap();
        std::fs::write(dir.join("T3/done.md"), "Dropped: old.\n").unwrap();
        let bare = disk.get(&id("T3")).unwrap().unwrap();
        assert_eq!((bare.title.as_str(), bare.status), ("T3", Status::Closed));
        assert!(PlanDirAdapter::holds_folders(&dir));
        assert!(!PlanDirAdapter::holds_folders(&dir.join("nope")));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
