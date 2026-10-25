// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The `disk` adapter (T441 §7): one Markdown file per task, `<dir>/R12 - <slug>.md`, with a
//! YAML front matter and the description as the body — the Backlog.md layout, readable and
//! diffable in the repository. Done and closed tasks move to `<dir>/done/` (§8), so the plan is
//! a directory listing. Writes are temp file + rename.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::adapter::{ClaimLock, Filter, Taken, TaskAdapter};
use super::{NewTask, Status, Task, TaskId};

/// Longest slug in a file name; the title itself lives in the front matter.
const SLUG_MAX: usize = 48;

/// How a claim waits for another process's lock before giving up.
const LOCK_TRIES: u32 = 50;
const LOCK_WAIT: Duration = Duration::from_millis(20);

pub struct DiskAdapter {
    dir: PathBuf,
}

/// The front matter. `description` is the body, not a key.
#[derive(Serialize, Deserialize)]
struct Head {
    id: TaskId,
    title: String,
    status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<TaskId>,
    created_at: i64,
    updated_at: i64,
    #[serde(
        default = "super::default_priority",
        skip_serializing_if = "super::is_default_priority"
    )]
    priority: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    blocked_by: Vec<TaskId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    conflicts_with: Vec<TaskId>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    parallel: bool,
}

/// Holds the OS lock on `<dir>/.claim.lock`. The lock goes with the handle (dropped on panic,
/// released by the kernel when the process dies), and the file is never unlinked, so a late
/// release cannot remove a successor's lock.
#[derive(Debug)]
pub(super) struct DiskGuard {
    _file: File,
}

impl ClaimLock for DiskGuard {}

impl DiskAdapter {
    /// Tasks live in `dir` (`[tasks.disk] dir` joined to the project root).
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn done_dir(&self) -> PathBuf {
        self.dir.join("done")
    }

    /// The task files of `dir` that parse, and the ones that do not. A missing directory is an
    /// empty plan.
    fn scan(&self, dir: &Path) -> Result<Scan> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Scan::default()),
            Err(e) => return Err(e).with_context(|| format!("disk tasks: {}", dir.display())),
        };
        let mut out = Scan::default();
        for entry in entries {
            let path = entry?.path();
            // Only `<id> - <slug>.md` is ours; a README or notes beside the tasks stay theirs.
            let Some(name_id) = file_id(&path) else {
                continue;
            };
            match load(&path, &name_id) {
                Ok(task) => out.found.push((path, task)),
                Err(error) => {
                    warn_skipped(&error);
                    out.skipped.push(Skipped { name_id, error });
                }
            }
        }
        Ok(out)
    }

    /// The tasks of `dir` that parse; a broken file is warned about and left out.
    fn tasks(&self, dir: &Path) -> Result<Vec<Task>> {
        Ok(self.scan(dir)?.found.into_iter().map(|(_, t)| t).collect())
    }

    /// The file holding `id`, in the plan or the archive. Its own broken file is an error, so
    /// no write goes near it; another task's broken file is not.
    fn find(&self, id: &TaskId) -> Result<Option<(PathBuf, Task)>> {
        for dir in [self.dir.clone(), self.done_dir()] {
            let Scan { found, skipped } = self.scan(&dir)?;
            if let Some(broken) = skipped.into_iter().find(|s| &s.name_id == id) {
                return Err(broken.error);
            }
            if let Some(hit) = found
                .into_iter()
                .find(|(path, _)| file_id(path).as_ref() == Some(id))
            {
                return Ok(Some(hit));
            }
        }
        Ok(None)
    }

    fn write(&self, dir: &Path, task: &Task) -> Result<PathBuf> {
        std::fs::create_dir_all(dir).with_context(|| format!("disk tasks: {}", dir.display()))?;
        let path = dir.join(file_name(&task.id, &task.title));
        fs_kit::write::atomic(&path, render(task)?)?;
        Ok(path)
    }

    /// Write `task` over its current file, moving it between the plan and `done/` when the
    /// status crosses that line. The caller holds [`lock_claim`].
    fn place(&self, old: Option<PathBuf>, task: &Task) -> Result<Task> {
        let dir = if task.status.is_active() {
            self.dir.clone()
        } else {
            self.done_dir()
        };
        let new = self.write(&dir, task)?;
        // The new file is complete before the old one goes: a crash in between leaves the task
        // in both directories (the plan copy wins in `find`), never in neither.
        if let Some(old) = old
            && new != old
        {
            std::fs::remove_file(&old).with_context(|| format!("disk tasks: {}", old.display()))?;
        }
        Ok(task.clone())
    }
}

impl TaskAdapter for DiskAdapter {
    fn name(&self) -> &'static str {
        "disk"
    }

    fn create(&self, task: &NewTask, id: &TaskId) -> Result<Task> {
        if task.title.trim().is_empty() {
            bail!("disk tasks: a task needs a title");
        }
        if self.find(id)?.is_some() {
            return Err(anyhow::Error::new(Taken(id.clone()))
                .context(format!("disk tasks: {}", self.dir.display())));
        }
        let now = now();
        let task = Task::open(
            id.clone(),
            task.title.trim().to_string(),
            task.description.clone(),
            now,
        );
        self.write(&self.dir, &task)?;
        Ok(task)
    }

    fn list(&self, filter: &Filter) -> Result<Vec<Task>> {
        let mut tasks = self.tasks(&self.dir)?;
        if filter.wants_finished() {
            tasks.extend(self.tasks(&self.done_dir())?);
        }
        Ok(filter.select(tasks))
    }

    fn get(&self, id: &TaskId) -> Result<Option<Task>> {
        Ok(self.find(id)?.map(|(_, t)| t))
    }

    fn write_status(&self, id: &TaskId, status: Status) -> Result<Task> {
        let _guard = lock_claim(&self.dir)?;
        let Some((old, mut task)) = self.find(id)? else {
            bail!("disk tasks: no task {id} in {}", self.dir.display());
        };
        task.status = status;
        task.updated_at = now();
        self.place(Some(old), &task)
    }

    fn claim_guard(&self) -> Result<Box<dyn ClaimLock>> {
        Ok(Box::new(lock_claim(&self.dir)?))
    }

    fn stores_markers(&self) -> bool {
        true
    }

    fn save(&self, task: &Task) -> Result<Task> {
        let Some((old, _)) = self.find(&task.id)? else {
            bail!("disk tasks: no task {} in {}", task.id, self.dir.display());
        };
        let mut task = task.clone();
        task.updated_at = now();
        if task.assignee.as_deref().is_some_and(|s| s.is_empty()) {
            task.assignee = None;
        }
        task.blocked_by.sort();
        task.blocked_by.dedup();
        task.conflicts_with.sort();
        task.conflicts_with.dedup();
        self.place(Some(old), &task)
    }

    fn max_id(&self, prefix: &str) -> Result<Option<TaskId>> {
        let prefix = prefix.to_ascii_uppercase();
        let mut ids: Vec<TaskId> = Vec::new();
        for dir in [self.dir.clone(), self.done_dir()] {
            let Scan { found, skipped } = self.scan(&dir)?;
            // A skipped file still owns its name, or the next task would be given its id.
            ids.extend(found.into_iter().map(|(_, t)| t.id));
            ids.extend(skipped.into_iter().map(|s| s.name_id));
        }
        Ok(ids.into_iter().filter(|id| id.prefix() == prefix).max())
    }
}

/// What a directory scan found: the tasks, and the files that are named like one but cannot be
/// used.
#[derive(Default)]
struct Scan {
    found: Vec<(PathBuf, Task)>,
    skipped: Vec<Skipped>,
}

struct Skipped {
    /// The id the file name carries; the front matter of a broken file cannot be trusted.
    name_id: TaskId,
    error: anyhow::Error,
}

/// Parse one task file, refusing one whose front-matter id is not its name's.
fn load(path: &Path, name_id: &TaskId) -> Result<Task> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("disk tasks: {}", path.display()))?;
    let task = parse(&text).with_context(|| format!("disk tasks: {}", path.display()))?;
    // `place` writes under the front-matter id, so a copied file with a foreign id would
    // overwrite that task's file; it is skipped like a broken one, and `find` fails for its name.
    if &task.id != name_id {
        bail!(
            "disk tasks: {}: file name says {name_id} but the front matter says {}",
            path.display(),
            task.id
        );
    }
    Ok(task)
}

/// Warn that a task file was left out of a read. Every command and MCP call rescans, so the
/// same file is reported once per process, not once per call.
pub(super) fn warn_skipped(error: &anyhow::Error) {
    static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let message = format!("{error:#}");
    let first = SEEN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashSet::new)
        .insert(message.clone());
    if first {
        log::warn!("skipping a task that does not parse: {message}");
    }
}

/// An exclusive `flock` on the stable `<dir>/.claim.lock`, polled for a bounded time. A holder
/// that is paused keeps the lock, so a waiter gets the "locked" error instead of racing it; a
/// crashed holder's lock is released by the OS, so no leftover file ever blocks a claim.
pub(super) fn lock_claim(dir: &Path) -> Result<DiskGuard> {
    std::fs::create_dir_all(dir).with_context(|| format!("disk tasks: {}", dir.display()))?;
    let path = dir.join(".claim.lock");
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("disk tasks: {}", path.display()))?;
    for _ in 0..LOCK_TRIES {
        if rtok_sys::try_lock_exclusive(&file)
            .with_context(|| format!("disk tasks: {}", path.display()))?
        {
            return Ok(DiskGuard { _file: file });
        }
        thread::sleep(LOCK_WAIT);
    }
    bail!("disk tasks: {} is locked", path.display())
}

pub(super) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The id a file name carries: `R12 - ship-it.md` → `R12`.
fn file_id(path: &Path) -> Option<TaskId> {
    if path.extension()? != "md" {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    stem.split_once(" - ")?.0.parse().ok()
}

fn file_name(id: &TaskId, title: &str) -> String {
    format!("{id} - {}.md", slug(title))
}

/// Lower-case ASCII letters and digits joined by single dashes; `task` when nothing is left.
fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= SLUG_MAX {
            break;
        }
    }
    let out = out.trim_end_matches('-');
    if out.is_empty() {
        "task".to_string()
    } else {
        out.to_string()
    }
}

fn render(task: &Task) -> Result<String> {
    let head = Head {
        id: task.id.clone(),
        title: task.title.clone(),
        status: task.status,
        parent: task.parent.clone(),
        created_at: task.created_at,
        updated_at: task.updated_at,
        priority: task.priority,
        assignee: task.assignee.clone(),
        blocked_by: task.blocked_by.clone(),
        conflicts_with: task.conflicts_with.clone(),
        parallel: task.parallel,
    };
    let yaml = serde_saphyr::to_string(&head)?;
    let body = task.description.trim_end();
    Ok(if body.is_empty() {
        format!("---\n{yaml}---\n")
    } else {
        format!("---\n{yaml}---\n\n{body}\n")
    })
}

fn parse(text: &str) -> Result<Task> {
    let text = text.replace("\r\n", "\n");
    let Some(rest) = text.strip_prefix("---\n") else {
        bail!("no front matter (the file must start with ---)");
    };
    let Some((yaml, body)) = rest
        .split_once("\n---\n")
        .or_else(|| rest.strip_suffix("\n---").map(|yaml| (yaml, "")))
    else {
        bail!("front matter is not closed with ---");
    };
    let head: Head = serde_saphyr::from_str(yaml)?;
    let mut blocked_by = head.blocked_by;
    blocked_by.sort();
    blocked_by.dedup();
    let mut conflicts_with = head.conflicts_with;
    conflicts_with.sort();
    conflicts_with.dedup();
    let assignee = head.assignee.filter(|s| !s.is_empty());
    Ok(Task {
        description: body.trim().to_string(),
        status: head.status,
        parent: head.parent,
        created_at: head.created_at,
        updated_at: head.updated_at,
        priority: head.priority,
        assignee,
        blocked_by,
        conflicts_with,
        parallel: head.parallel,
        ..Task::open(head.id, head.title, "", 0)
    })
}

#[cfg(test)]
mod tests {
    use super::super::adapter::set_status;
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rtok-disk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn id(s: &str) -> TaskId {
        s.parse().unwrap()
    }

    fn new(title: &str) -> NewTask {
        NewTask {
            title: title.into(),
            description: String::new(),
            parent: None,
        }
    }

    #[test]
    fn create_get_and_list_the_plan() {
        let dir = tmp("plan");
        let disk = DiskAdapter::new(dir.clone());
        let task = NewTask {
            description: "Why and what done means.\n\nSecond paragraph.".into(),
            ..new("Ship the: allocator!")
        };
        disk.create(&task, &id("R1")).unwrap();
        disk.create(&new("Second"), &id("R2")).unwrap();
        disk.create(&new("Sub"), &id("R2.1")).unwrap();
        assert!(dir.join("R1 - ship-the-allocator.md").is_file());

        let got = disk.get(&id("r1")).unwrap().unwrap();
        assert_eq!(got.title, "Ship the: allocator!");
        assert_eq!(got.description, task.description);
        assert_eq!(got.status, Status::Open);
        assert_eq!(
            disk.get(&id("R2.1")).unwrap().unwrap().parent,
            Some(id("R2"))
        );
        assert_eq!(disk.get(&id("R9")).unwrap(), None);

        let ids: Vec<String> = disk
            .list(&Filter::default())
            .unwrap()
            .iter()
            .map(|t| t.id.to_string())
            .collect();
        assert_eq!(ids, ["R1", "R2", "R2.1"]);
        let subs = disk
            .list(&Filter {
                parent: Some(id("R2")),
                ..Filter::default()
            })
            .unwrap();
        assert_eq!(subs.len(), 1);
        assert!(disk.create(&new("Again"), &id("R1")).is_err(), "id taken");
        assert!(disk.create(&new("  "), &id("R3")).is_err(), "title");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn done_tasks_leave_the_plan_and_can_come_back() {
        let dir = tmp("done");
        let disk = DiskAdapter::new(dir.clone());
        disk.create(&new("Parent"), &id("R1")).unwrap();
        disk.create(&new("Child"), &id("R1.1")).unwrap();

        let err = set_status(&disk, &id("R1"), Status::Done, false).unwrap_err();
        assert!(err.to_string().contains("R1.1"), "{err}");

        set_status(&disk, &id("R1.1"), Status::Closed, false).unwrap();
        set_status(&disk, &id("R1"), Status::Done, false).unwrap();
        assert!(disk.list(&Filter::default()).unwrap().is_empty());
        assert!(dir.join("done/R1 - parent.md").is_file());
        assert!(!dir.join("R1 - parent.md").exists());

        let all = disk
            .list(&Filter {
                all: true,
                ..Filter::default()
            })
            .unwrap();
        assert_eq!(all.len(), 2);
        let closed = disk
            .list(&Filter {
                statuses: vec![Status::Closed],
                ..Filter::default()
            })
            .unwrap();
        assert_eq!(closed[0].id, id("R1.1"));

        set_status(&disk, &id("R1"), Status::InProgress, false).unwrap();
        assert!(dir.join("R1 - parent.md").is_file());
        assert_eq!(disk.max_id("r").unwrap(), Some(id("R1.1")));
        assert_eq!(disk.max_id("Q").unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_forced_parent_closes_over_open_subtasks() {
        let dir = tmp("force");
        let disk = DiskAdapter::new(dir.clone());
        disk.create(&new("Parent"), &id("R1")).unwrap();
        disk.create(&new("Child"), &id("R1.1")).unwrap();
        set_status(&disk, &id("R1"), Status::Done, true).unwrap();
        assert!(set_status(&disk, &id("R7"), Status::Done, false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn foreign_files_are_ignored_and_broken_ones_are_skipped_but_keep_their_id() {
        let dir = tmp("foreign");
        let disk = DiskAdapter::new(dir.clone());
        assert!(
            disk.list(&Filter::default()).unwrap().is_empty(),
            "no dir yet"
        );
        disk.create(&new("Real"), &id("R1")).unwrap();
        std::fs::write(dir.join("README.md"), "# tasks\n").unwrap();
        std::fs::write(dir.join("R2 - notes.txt"), "x").unwrap();
        assert_eq!(disk.list(&Filter::default()).unwrap().len(), 1);
        let broken = dir.join("R3 - broken.md");
        std::fs::write(&broken, "no front matter\n").unwrap();
        disk.create(&new("Next"), &id("R4")).unwrap();

        let listed = disk.list(&Filter::default()).unwrap();
        assert_eq!(listed.len(), 2, "the broken file is left out");
        assert_eq!(disk.get(&id("R4")).unwrap().unwrap().title, "Next");
        assert_eq!(disk.max_id("r").unwrap(), Some(id("R4")));
        std::fs::remove_file(dir.join("R4 - next.md")).unwrap();
        assert_eq!(
            disk.max_id("r").unwrap(),
            Some(id("R3")),
            "ids are not reused"
        );

        // Its own id still fails with the file named, and no write touches it.
        let err = disk.get(&id("R3")).unwrap_err();
        assert!(format!("{err:#}").contains("R3 - broken.md"), "{err:#}");
        assert!(disk.create(&new("Again"), &id("R3")).is_err());
        assert!(disk.write_status(&id("R3"), Status::Done).is_err());
        assert!(disk.save(&Task::open(id("R3"), "x", "", 0)).is_err());
        assert_eq!(std::fs::read(&broken).unwrap(), b"no front matter\n");
        // README, notes, R1, the broken file and the claim lock: nothing new.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_whose_front_matter_id_differs_from_its_name_is_skipped_untouched() {
        let dir = tmp("identity");
        let disk = DiskAdapter::new(dir.clone());
        disk.create(&new("target"), &id("R2")).unwrap();
        let copy = dir.join("R1 - copy.md");
        std::fs::write(
            &copy,
            "---\nid: R2\ntitle: target\nstatus: open\ncreated_at: 1\nupdated_at: 2\n---\n\nCopied body\n",
        )
        .unwrap();
        let target = dir.join("R2 - target.md");
        let before = (
            std::fs::read(&copy).unwrap(),
            std::fs::read(&target).unwrap(),
        );

        // Reads that do not name the copy go on without it...
        let listed = disk.list(&Filter::default()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(disk.get(&id("R2")).unwrap().unwrap().description, "");
        assert_eq!(disk.max_id("r").unwrap(), Some(id("R2")));
        // ...and R2 is the real file's, so writes to it never see the copy.
        disk.write_status(&id("R2"), Status::InProgress).unwrap();
        disk.write_status(&id("R2"), Status::Open).unwrap();

        // Every path that would write under the copy's name fails before any mutation.
        for err in [
            disk.get(&id("R1")).unwrap_err(),
            disk.write_status(&id("R1"), Status::InProgress)
                .unwrap_err(),
            disk.save(&Task::open(id("R1"), "x", "", 0)).unwrap_err(),
            disk.create(&new("x"), &id("R1")).unwrap_err(),
        ] {
            let msg = format!("{err:#}");
            assert!(
                msg.contains("R1 - copy.md") && msg.contains("R1") && msg.contains("R2"),
                "{msg}"
            );
        }
        assert_eq!(std::fs::read(&copy).unwrap(), before.0);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_mismatch_in_done_is_skipped_and_a_lower_case_name_still_matches() {
        let dir = tmp("identity-done");
        std::fs::create_dir_all(dir.join("done")).unwrap();
        std::fs::write(
            dir.join("done/R3 - a.md"),
            "---\nid: R4\ntitle: a\nstatus: done\ncreated_at: 1\nupdated_at: 2\n---\n",
        )
        .unwrap();
        let disk = DiskAdapter::new(dir.clone());
        let all = Filter {
            all: true,
            ..Filter::default()
        };
        assert!(disk.list(&all).unwrap().is_empty());
        let err = disk.get(&id("R3")).unwrap_err();
        assert!(format!("{err:#}").contains("R3 - a.md"), "{err:#}");
        std::fs::remove_file(dir.join("done/R3 - a.md")).unwrap();

        std::fs::write(
            dir.join("r5 - b.md"),
            "---\nid: R5\ntitle: b\nstatus: open\ncreated_at: 1\nupdated_at: 2\n---\n",
        )
        .unwrap();
        assert!(disk.get(&id("R5")).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_file_round_trips_through_render_and_parse() {
        let task = Task {
            title: "Title: with \"quotes\" and # hash".into(),
            description: "Body\n---\nafter a rule".into(),
            status: Status::InProgress,
            created_at: 10,
            updated_at: 20,
            conflicts_with: vec![id("R2"), id("R3")],
            parallel: true,
            ..Task::open(id("R4"), "", "", 0)
        };
        let text = render(&task).unwrap();
        assert!(text.starts_with("---\nid: R4\n"), "{text}");
        assert!(
            text.contains("conflicts_with:") && text.contains("parallel: true"),
            "{text}"
        );
        assert_eq!(parse(&text).unwrap(), task);
        assert_eq!(parse(&text.replace('\n', "\r\n")).unwrap(), task);
        let bare = Task {
            description: String::new(),
            ..task
        };
        assert_eq!(parse(&render(&bare).unwrap()).unwrap(), bare);
    }

    #[test]
    fn a_file_without_claim_keys_still_lists_and_completes() {
        let dir = tmp("old");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("R4 - old.md"),
            "---\nid: R4\ntitle: Old\nstatus: open\ncreated_at: 1\nupdated_at: 2\n---\n\nBody\n",
        )
        .unwrap();
        let disk = DiskAdapter::new(dir.clone());
        let got = disk.get(&id("R4")).unwrap().unwrap();
        assert_eq!(got.priority, super::super::DEFAULT_PRIORITY);
        assert!(got.assignee.is_none());
        assert!(got.blocked_by.is_empty());
        assert_eq!(got.description, "Body");
        set_status(&disk, &id("R4"), Status::Done, false).unwrap();
        assert!(dir.join("done/R4 - old.md").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_leftover_lock_file_is_taken_and_a_held_one_is_kept() {
        let dir = tmp("lock");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".claim.lock");
        std::fs::File::create(&path).unwrap();
        let guard = lock_claim(&dir).unwrap();
        drop(guard);
        assert!(path.is_file(), "the lock file is stable, never unlinked");

        let held = lock_claim(&dir).unwrap();
        let err = lock_claim(&dir).unwrap_err();
        assert!(err.to_string().contains("locked"), "{err}");
        drop(held);
        let again = lock_claim(&dir).unwrap();
        drop(again);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_lock_file_does_not_let_a_second_claimant_in() {
        let dir = tmp("lock-held-old");
        let a = lock_claim(&dir).unwrap();
        // A paused holder looks "stale" by mtime; the lock must still exclude.
        let file = OpenOptions::new()
            .write(true)
            .open(dir.join(".claim.lock"))
            .unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(11))
            .unwrap();
        drop(file);
        let err = lock_claim(&dir).unwrap_err();
        assert!(err.to_string().contains("locked"), "{err}");
        drop(a);
        // A's release leaves the file and the exclusion free for the next claimant.
        let b = lock_claim(&dir).unwrap();
        let err = lock_claim(&dir).unwrap_err();
        assert!(err.to_string().contains("locked"), "{err}");
        drop(b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Child half of [`a_killed_holder_releases_the_lock`]: holds the lock until killed.
    #[test]
    fn lock_holder_child() {
        let Ok(dir) = std::env::var("RTOK_DISK_LOCK_CHILD") else {
            return;
        };
        let _guard = lock_claim(Path::new(&dir)).unwrap();
        std::fs::write(Path::new(&dir).join("child-ready"), "").unwrap();
        thread::sleep(Duration::from_secs(60));
    }

    #[test]
    fn a_killed_holder_releases_the_lock() {
        use std::process::Command;

        let dir = tmp("lock-kill");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tasks::disk::tests::lock_holder_child"])
            .env("RTOK_DISK_LOCK_CHILD", &dir)
            .spawn()
            .unwrap();
        let ready = dir.join("child-ready");
        for _ in 0..500 {
            if ready.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(ready.exists(), "child never locked");
        let err = lock_claim(&dir).unwrap_err();
        assert!(err.to_string().contains("locked"), "{err}");
        child.kill().unwrap();
        child.wait().unwrap();
        drop(lock_claim(&dir).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn slugs_are_short_and_safe() {
        assert_eq!(slug("Fix ../../etc/passwd"), "fix-etc-passwd");
        assert_eq!(slug("Привет"), "task");
        assert!(slug(&"word ".repeat(40)).len() <= SLUG_MAX);
    }
}
