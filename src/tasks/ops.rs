// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Store-backed `rtok task` operations. The CLI parses flags and prints; the open lives here.

use std::path::Path;

use anyhow::{Result, bail};

use super::adapter::Filter;
use super::export;
use super::run::Project;
use super::sync::SyncReport;
use super::{ClaimOutcome, NewTask, ReadyItem, Status, Task, TaskId};
use crate::config::Config;
use crate::store::Store;

fn store(cfg: &Config) -> Result<Store> {
    Store::open(&cfg.core.db_path).map_err(Into::into)
}

pub fn seed(cfg: &Config, project: &Project) -> Result<u32> {
    project.seed(&store(cfg)?, None)
}

pub fn create(cfg: &Config, project: &Project, new: &NewTask) -> Result<Task> {
    project.create(&store(cfg)?, new)
}

pub fn set_status(
    cfg: &Config,
    project: &Project,
    id: &TaskId,
    status: Option<Status>,
    force: bool,
) -> Result<Task> {
    project.status(&store(cfg)?, id, status, force)
}

pub fn sync(cfg: &Config, project: &Project) -> Result<SyncReport> {
    super::sync::sync(project, &store(cfg)?)
}

/// `rtok task export <backend>`: mirror the disk tasks into `backend`. Only the disk adapter
/// is a source: exporting a GitHub task list into GitHub would copy it onto itself.
pub fn export(cfg: &Config, cwd: &Path, backend: &str, dry_run: bool) -> Result<export::Report> {
    if cfg.tasks.adapter != "disk" {
        bail!(
            "rtok task export mirrors the disk adapter, but [tasks] adapter is {:?}",
            cfg.tasks.adapter
        );
    }
    let project = Project::open(&cfg.tasks, cwd)?;
    let exporter = export::open(&cfg.tasks, &project.key, backend)?;
    let tasks = project.adapter().list(&Filter {
        all: true,
        ..Filter::default()
    })?;
    export::run(exporter.as_ref(), &tasks, dry_run)
}

pub fn next(cfg: &Config, project: &Project) -> Result<Option<Task>> {
    project.next(&store(cfg)?)
}

pub fn ready(cfg: &Config, project: &Project) -> Result<Vec<ReadyItem>> {
    project.ready(&store(cfg)?)
}

pub fn claim(
    cfg: &Config,
    project: &Project,
    id: Option<&str>,
    agent: Option<&str>,
) -> Result<ClaimOutcome> {
    let store = store(cfg)?;
    let agent = super::run::require_agent(&store, agent)?;
    let id = id.map(str::parse::<TaskId>).transpose()?;
    project.claim(&store, id.as_ref(), &agent)
}

pub fn release(
    cfg: &Config,
    project: &Project,
    id: &str,
    agent: Option<&str>,
    force: bool,
) -> Result<Task> {
    let store = store(cfg)?;
    let agent = super::run::require_agent(&store, agent)?;
    project.release(&store, &id.parse()?, &agent, force)
}
