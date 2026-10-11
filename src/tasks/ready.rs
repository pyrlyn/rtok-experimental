// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Which tasks can be claimed, and which dependency edges form a cycle (T442).
//!
//! Ready work is an open task with nobody on it, or any active task whose assignee was last
//! seen more than [`super::STALE_SECS`] ago, and nothing in `blocked_by` is still active.
//! A missing, done or closed blocker does not block. When the plan has no `blocked_by` edges
//! at all, a task with an active subtask is also not ready, which is what `next` did before
//! claims existed. A stale in-progress task still blocks other tasks until someone takes it.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{ReadyItem, STALE_SECS, Status, Task, TaskId};

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// `seen` is the assignee's `agents.last_seen`. No row means the agent is gone. A store
/// error is not stale: a claim must not be taken because the lookup failed.
pub fn holder_is_stale(seen: Result<Option<i64>, anyhow::Error>, now: i64) -> bool {
    match seen {
        Ok(None) => true,
        Ok(Some(ts)) => now.saturating_sub(ts) > STALE_SECS,
        Err(_) => false,
    }
}

/// `stale` reports whether that assignee can be replaced. It is only called for a non-empty
/// assignee.
pub fn select_ready(tasks: &[Task], stale: impl Fn(&str) -> bool) -> Vec<ReadyItem> {
    let any_blocks = tasks.iter().any(|t| !t.blocked_by.is_empty());
    let active: HashSet<&TaskId> = tasks
        .iter()
        .filter(|t| t.status.is_active())
        .map(|t| &t.id)
        .collect();
    let mut ready: Vec<ReadyItem> = tasks
        .iter()
        .filter_map(|task| {
            let holder = task.assignee.as_deref().filter(|s| !s.is_empty());
            let is_stale = holder.is_some_and(&stale);
            let claimable = match task.status {
                Status::Open => holder.is_none() || is_stale,
                Status::InProgress => is_stale,
                Status::Done | Status::Closed => false,
            };
            if !claimable {
                return None;
            }
            if task
                .blocked_by
                .iter()
                .any(|blocker| active.contains(blocker))
            {
                return None;
            }
            if !any_blocks {
                let child = tasks.iter().any(|other| {
                    other.parent.as_ref() == Some(&task.id) && other.status.is_active()
                });
                if child {
                    return None;
                }
            }
            Some(ReadyItem {
                task: task.clone(),
                stale: is_stale,
                conflicts_in_progress: conflicting_in_progress(task, tasks),
            })
        })
        .collect();
    ready.sort_by(|a, b| {
        a.task
            .priority
            .cmp(&b.task.priority)
            .then(a.task.id.cmp(&b.task.id))
    });
    ready
}

/// Conflict partners of `task` that are in progress. A conflict is symmetric, so an edge written
/// on only one side (a hand edit, a half-finished write) still counts from both.
fn conflicting_in_progress(task: &Task, tasks: &[Task]) -> Vec<TaskId> {
    let mut out: Vec<TaskId> = tasks
        .iter()
        .filter(|other| other.status == Status::InProgress && other.id != task.id)
        .filter(|other| {
            task.conflicts_with.contains(&other.id) || other.conflicts_with.contains(&task.id)
        })
        .map(|other| other.id.clone())
        .collect();
    out.sort();
    out
}

/// The first active id in `blocked_by`, if one is still open or in progress.
pub fn active_blocker<'a>(task: &'a Task, tasks: &'a [Task]) -> Option<&'a TaskId> {
    let active: HashSet<&TaskId> = tasks
        .iter()
        .filter(|t| t.status.is_active())
        .map(|t| &t.id)
        .collect();
    task.blocked_by
        .iter()
        .find(|blocker| active.contains(blocker))
}

/// Cycles in the wait-graph. An edge `task → blocker` means the task waits on `blocked_by`.
/// A parent also waits on each child, so a parent cannot close while the child is open and a
/// child that depends on its parent is a cycle. Each cycle is rotated so its lowest id is
/// first. Empty when nothing cycles. Order follows the id order of the nodes.
pub fn find_cycles(tasks: &[Task]) -> Vec<Vec<TaskId>> {
    let mut adj: BTreeMap<TaskId, BTreeSet<TaskId>> = BTreeMap::new();
    for task in tasks {
        adj.entry(task.id.clone()).or_default();
        for blocker in &task.blocked_by {
            adj.entry(task.id.clone())
                .or_default()
                .insert(blocker.clone());
            adj.entry(blocker.clone()).or_default();
        }
        if let Some(parent) = &task.parent {
            adj.entry(parent.clone())
                .or_default()
                .insert(task.id.clone());
            adj.entry(task.id.clone()).or_default();
        }
    }
    let mut color: BTreeMap<TaskId, u8> = BTreeMap::new();
    let mut stack = Vec::new();
    let mut cycles = Vec::new();
    for node in adj.keys().cloned().collect::<Vec<_>>() {
        if color.get(&node).copied().unwrap_or(0) == 0 {
            dfs(&node, &adj, &mut color, &mut stack, &mut cycles);
        }
    }
    cycles.sort();
    cycles.dedup();
    cycles
}

fn dfs(
    node: &TaskId,
    adj: &BTreeMap<TaskId, BTreeSet<TaskId>>,
    color: &mut BTreeMap<TaskId, u8>,
    stack: &mut Vec<TaskId>,
    cycles: &mut Vec<Vec<TaskId>>,
) {
    color.insert(node.clone(), 1);
    stack.push(node.clone());
    if let Some(next) = adj.get(node) {
        for to in next {
            match color.get(to).copied().unwrap_or(0) {
                1 => {
                    if let Some(at) = stack.iter().position(|id| id == to) {
                        cycles.push(rotate_lowest(stack[at..].to_vec()));
                    }
                }
                0 => dfs(to, adj, color, stack, cycles),
                _ => {}
            }
        }
    }
    stack.pop();
    color.insert(node.clone(), 2);
}

/// Lowest id first. The repeated closing id of a walk is not included.
fn rotate_lowest(mut cycle: Vec<TaskId>) -> Vec<TaskId> {
    if cycle.len() >= 2 && cycle.first() == cycle.last() {
        cycle.pop();
    }
    if let Some(at) = cycle
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.cmp(b.1))
        .map(|(i, _)| i)
    {
        cycle.rotate_left(at);
    }
    cycle
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> TaskId {
        s.parse().unwrap()
    }

    fn task(s: &str) -> Task {
        Task::open(id(s), s, "", 0)
    }

    #[test]
    fn ready_keeps_the_leaf_until_something_is_blocked() {
        let mut tasks = vec![task("R1"), task("R1.1"), task("R2")];
        let ids: Vec<_> = select_ready(&tasks, |_| false)
            .into_iter()
            .map(|i| i.task.id.to_string())
            .collect();
        assert_eq!(ids, ["R1.1", "R2"]);

        tasks[2].blocked_by = vec![id("R9")];
        let ids: Vec<_> = select_ready(&tasks, |_| false)
            .into_iter()
            .map(|i| i.task.id.to_string())
            .collect();
        // R9 is not an active task, so it does not block, and the leaf rule is off.
        assert_eq!(ids, ["R1", "R1.1", "R2"]);
    }

    #[test]
    fn an_active_blocker_and_a_live_claim_leave_the_queue() {
        let mut tasks = vec![task("R1"), task("R2"), task("R3")];
        tasks[0].priority = 1;
        tasks[1].blocked_by = vec![id("R1")];
        tasks[2].assignee = Some("live".into());
        tasks[2].status = Status::InProgress;
        let ready = select_ready(&tasks, |_| false);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].task.id, id("R1"));
        assert!(!ready[0].stale);

        tasks[2].status = Status::Open;
        let stale = select_ready(&tasks, |agent| agent == "live");
        assert!(stale.iter().any(|i| i.task.id == id("R3") && i.stale));
    }

    #[test]
    fn a_conflict_with_a_task_in_progress_flags_the_ready_task_from_either_side() {
        let mut tasks = vec![task("R1"), task("R2"), task("R3"), task("R4")];
        tasks[0].status = Status::InProgress;
        tasks[0].assignee = Some("live".into());
        tasks[1].conflicts_with = vec![id("R1")];
        tasks[2].conflicts_with = vec![id("R4")];
        tasks[0].conflicts_with = vec![id("R4")];
        let ready = select_ready(&tasks, |_| false);
        let flagged = |n: &str| {
            ready
                .iter()
                .find(|i| i.task.id == id(n))
                .map(|i| i.conflicts_in_progress.clone())
        };
        assert_eq!(flagged("R1"), None, "the task in progress is not ready");
        assert_eq!(flagged("R2"), Some(vec![id("R1")]), "flagged, not hidden");
        assert_eq!(
            flagged("R4"),
            Some(vec![id("R1")]),
            "R1 names R4 only on its own side"
        );
        assert_eq!(flagged("R3"), Some(vec![]), "R4 is not in progress");
    }

    #[test]
    fn a_missing_agent_is_stale_and_a_store_error_is_not() {
        assert!(holder_is_stale(Ok(None), 0));
        assert!(!holder_is_stale(Ok(Some(1_000)), 1_000 + STALE_SECS));
        assert!(holder_is_stale(Ok(Some(1_000)), 1_000 + STALE_SECS + 1));
        assert!(!holder_is_stale(Err(anyhow::anyhow!("db")), 9_999));
    }

    #[test]
    fn cycles_start_at_the_lowest_id_and_include_a_parent() {
        assert!(find_cycles(&[task("R1"), task("R2")]).is_empty());
        let mut a = task("R2");
        let mut b = task("R1");
        a.blocked_by = vec![id("R1")];
        b.blocked_by = vec![id("R2")];
        assert_eq!(find_cycles(&[a, b]), vec![vec![id("R1"), id("R2")]]);

        let child = task("R1.1");
        let mut blocked = child.clone();
        blocked.blocked_by = vec![id("R1")];
        let cycle = find_cycles(&[task("R1"), blocked]);
        assert_eq!(cycle, vec![vec![id("R1"), id("R1.1")]]);
    }
}
