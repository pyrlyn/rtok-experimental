// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Claim, dependency, priority and marker labels the GitHub and GitLab adapters share (T442,
//! T441.13.4). `rtok`, `rtok:<id>` and the status labels stay as they are; these are the only
//! ones `save` rewrites. A user's own labels are kept.

use super::{DEFAULT_PRIORITY, PRIORITY_MAX, Task, TaskId};

pub const OWNER_PREFIX: &str = "rtok:owner:";
pub const NEEDS_PREFIX: &str = "rtok:needs:";
pub const PRIORITY_PREFIX: &str = "rtok:p:";
pub const CONFLICTS_PREFIX: &str = "rtok:conflicts:";
/// Exact label, not a prefix: it carries no value. Not a task id, so `label_id` skips it.
pub const PARALLEL: &str = "rtok:parallel";

/// A label `save` owns: the assignee, a blocker, a non-default priority, or a marker.
pub fn is_managed(label: &str) -> bool {
    label.starts_with(OWNER_PREFIX)
        || label.starts_with(NEEDS_PREFIX)
        || label.starts_with(PRIORITY_PREFIX)
        || label.starts_with(CONFLICTS_PREFIX)
        || label == PARALLEL
}

/// What the managed labels of an issue say about its task.
pub struct Meta {
    pub priority: u8,
    pub assignee: Option<String>,
    pub blocked_by: Vec<TaskId>,
    pub conflicts_with: Vec<TaskId>,
    pub parallel: bool,
}

/// The [`Meta`] carried by `labels`. Unknown or out-of-range values are dropped, so a
/// hand-edited label cannot push a task outside `0..=4`.
pub fn read_meta<S: AsRef<str>>(labels: &[S]) -> Meta {
    let mut priority = DEFAULT_PRIORITY;
    let mut assignee = None;
    let mut blocked_by = Vec::new();
    let mut conflicts_with = Vec::new();
    let mut parallel = false;
    for label in labels {
        let label = label.as_ref();
        if let Some(rest) = label.strip_prefix(OWNER_PREFIX) {
            if assignee.is_none() && !rest.is_empty() {
                assignee = Some(rest.to_string());
            }
        } else if let Some(rest) = label.strip_prefix(NEEDS_PREFIX) {
            if let Ok(id) = rest.parse() {
                blocked_by.push(id);
            }
        } else if let Some(rest) = label.strip_prefix(PRIORITY_PREFIX)
            && let Ok(n) = rest.parse::<u8>()
            && n <= PRIORITY_MAX
        {
            priority = n;
        } else if let Some(rest) = label.strip_prefix(CONFLICTS_PREFIX) {
            if let Ok(id) = rest.parse() {
                conflicts_with.push(id);
            }
        } else if label == PARALLEL {
            parallel = true;
        }
    }
    for ids in [&mut blocked_by, &mut conflicts_with] {
        ids.sort();
        ids.dedup();
    }
    Meta {
        priority,
        assignee,
        blocked_by,
        conflicts_with,
        parallel,
    }
}

/// The managed labels `task` should carry. Priority `2` adds none.
pub fn managed_for(task: &Task) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(agent) = task.assignee.as_deref().filter(|s| !s.is_empty()) {
        out.push(format!("{OWNER_PREFIX}{agent}"));
    }
    for id in &task.blocked_by {
        out.push(format!("{NEEDS_PREFIX}{id}"));
    }
    if task.priority != DEFAULT_PRIORITY {
        out.push(format!("{PRIORITY_PREFIX}{}", task.priority));
    }
    for id in &task.conflicts_with {
        out.push(format!("{CONFLICTS_PREFIX}{id}"));
    }
    if task.parallel {
        out.push(PARALLEL.to_string());
    }
    out
}

/// `existing` with its managed labels replaced by [`managed_for`]. Order of the other labels
/// is kept, so a user's `bug` stays where they put it.
pub fn merge_labels(existing: &[String], task: &Task) -> Vec<String> {
    let mut out: Vec<String> = existing
        .iter()
        .filter(|l| !is_managed(l))
        .cloned()
        .collect();
    out.extend(managed_for(task));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_round_trip_and_a_hand_edited_label_is_ignored() {
        let labels = [
            "bug",
            "rtok:conflicts:A3",
            "rtok:conflicts:A2",
            "rtok:conflicts:A2",
            "rtok:conflicts:not-an-id",
            "rtok:parallel",
        ];
        let meta = read_meta(&labels);
        let ids = |v: &[&str]| {
            v.iter()
                .map(|s| s.parse().unwrap())
                .collect::<Vec<TaskId>>()
        };
        assert_eq!(meta.conflicts_with, ids(&["A2", "A3"]));
        assert!(meta.parallel);
        assert!(labels.iter().filter(|l| is_managed(l)).count() == 5);
        assert!(!read_meta(&["rtok:parallels"]).parallel);
    }
}
