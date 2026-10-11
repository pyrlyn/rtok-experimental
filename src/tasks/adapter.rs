// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The task adapter trait (T441 §6) and the rules every adapter shares: what `list` hides and
//! when a parent may close.

use anyhow::{Result, bail};

use super::{NewTask, Status, Task, TaskId};

/// Held across a claim, release or save so two disk writers cannot interleave. Remote
/// adapters return an empty guard: GitHub and GitLab have no compare-and-set for labels.
pub trait ClaimLock: Send {}

/// No lock. Remote adapters use this; the assignee write is last-write-wins.
struct NoLock;
impl ClaimLock for NoLock {}

/// What `list` returns. The default is the plan: every active task, top level and subtasks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// Only these statuses; empty means active ones, unless `all`.
    pub statuses: Vec<Status>,
    /// Done and closed tasks too.
    pub all: bool,
    /// Only the subtasks of this task.
    pub parent: Option<TaskId>,
}

impl Filter {
    pub fn matches(&self, task: &Task) -> bool {
        let status = if self.statuses.is_empty() {
            self.all || task.status.is_active()
        } else {
            self.statuses.contains(&task.status)
        };
        status && (self.parent.is_none() || task.parent == self.parent)
    }

    /// What `list` returns from everything an adapter read: the matches, sorted by id.
    pub fn select(&self, tasks: impl IntoIterator<Item = Task>) -> Vec<Task> {
        let mut out: Vec<Task> = tasks.into_iter().filter(|t| self.matches(t)).collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Whether finished tasks can match, so an adapter knows to read its archive.
    pub fn wants_finished(&self) -> bool {
        self.all || self.statuses.iter().any(|s| !s.is_active())
    }
}

/// `create`'s error when `id` is already stored: a pull, another checkout or another machine
/// numbered past this machine's counter. [`super::run::Project::create`] re-allocates on it.
#[derive(Debug)]
pub struct Taken(pub TaskId);

impl std::fmt::Display for Taken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "task {} already exists", self.0)
    }
}

impl std::error::Error for Taken {}

/// A remote issue that carries rtok's label but no id label: its `rtok:<id>` was removed, or
/// the issue was labelled by hand. Every later call skips it, so `rtok task sync` names it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Stray {
    pub title: String,
    pub url: String,
    /// The id the title starts with, when it still does: the label to put back.
    pub id: Option<TaskId>,
}

/// A task storage backend: plain files on disk, GitHub Issues or GitLab Issues. Ids come from
/// the store's allocator; an adapter only stores them.
pub trait TaskAdapter: Send {
    /// `disk`, `github` or `gitlab`, for error messages.
    fn name(&self) -> &'static str;
    /// Store a new task under `id`; fails when `id` is already taken.
    fn create(&self, task: &NewTask, id: &TaskId) -> Result<Task>;
    /// Tasks matching `filter`, sorted by id.
    fn list(&self, filter: &Filter) -> Result<Vec<Task>>;
    fn get(&self, id: &TaskId) -> Result<Option<Task>>;
    /// Move a task to `status`; done and closed leave the plan (T441 §8). Callers go
    /// through [`set_status`], which guards parents.
    fn write_status(&self, id: &TaskId, status: Status) -> Result<Task>;
    /// The highest id with `prefix`, done ones included: seeding and collision checks.
    fn max_id(&self, prefix: &str) -> Result<Option<TaskId>>;
    /// Items the adapter holds that no id names, for `rtok task sync`. Reads only. Files on
    /// disk are the ids, so only remote adapters have any.
    fn unlabelled(&self) -> Result<Vec<Stray>> {
        Ok(Vec::new())
    }
    /// Held for the whole claim. The default does not lock.
    fn claim_guard(&self) -> Result<Box<dyn ClaimLock>> {
        Ok(Box::new(NoLock))
    }
    /// Write `task` back, including assignee, blockers and priority. `claim` already holds
    /// [`claim_guard`], so this must not take the lock again. The default refuses: an adapter
    /// that cannot store a claim says so instead of pretending it did.
    fn save(&self, _task: &Task) -> Result<Task> {
        bail!("{} tasks: saving a claim is not supported", self.name())
    }

    /// Whether `save` keeps `conflicts_with` and `parallel` (T441.13.2, T441.13.4). An adapter
    /// that does not says so, and the commands that set them refuse instead of dropping them.
    fn stores_markers(&self) -> bool {
        false
    }
}

/// Set a task's status. A parent with active subtasks cannot finish unless `force`: the error
/// lists them, so the plan never shows orphans under a finished task.
pub fn set_status(
    adapter: &dyn TaskAdapter,
    id: &TaskId,
    status: Status,
    force: bool,
) -> Result<Task> {
    if !status.is_active() && !force {
        let open = adapter.list(&Filter {
            parent: Some(id.clone()),
            ..Filter::default()
        })?;
        if !open.is_empty() {
            let ids: Vec<String> = open.iter().map(|t| t.id.to_string()).collect();
            bail!(
                "{id} has open subtasks: {}; finish them first or pass --force",
                ids.join(", ")
            );
        }
    }
    adapter.write_status(id, status)
}
