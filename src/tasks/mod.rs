// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Task adapters (T441): agents create and track tasks through rtok, numbered by one
//! allocator and stored on disk, in GitHub or in GitLab. This module holds the domain types
//! every adapter, the CLI and the MCP tools share (T441.2); `[tasks]` in the config picks the
//! adapter and the id prefix.
//!
//! "Adapter" is the task storage backend. "Provider" stays the proxy's upstream LLM API.

use std::fmt;
use std::str::FromStr;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub mod adapter;
pub mod disk;
pub mod export;
pub mod github;
mod github_project;
pub mod gitlab;
mod meta;
pub mod ops;
mod ready;
pub mod remote;
pub mod run;
pub mod sync;

pub use crate::task_id::{MAX_DEPTH, MAX_PREFIX, TaskId, check_prefix};

/// Priority when a task names none. `0` is the highest, `4` the lowest (T442).
pub const DEFAULT_PRIORITY: u8 = 2;

/// Highest priority number accepted. Above this, `task priority` refuses.
pub const PRIORITY_MAX: u8 = 4;

/// A claim whose agent was last seen longer ago than this can be taken (T442).
pub const STALE_SECS: i64 = 30 * 60;

pub(crate) fn default_priority() -> u8 {
    DEFAULT_PRIORITY
}

pub(crate) fn is_default_priority(priority: &u8) -> bool {
    *priority == DEFAULT_PRIORITY
}

/// The prefix a project gets when `[tasks] prefix` is empty: the first ASCII letter of its
/// name, upper-cased (`rtok` → `R`). `None` when the name has no ASCII letter.
pub fn default_prefix(project: &str) -> Option<String> {
    project
        .chars()
        .find(char::is_ascii_alphabetic)
        .map(|c| c.to_ascii_uppercase().to_string())
}

/// The prefix new ids get: `[tasks] prefix` when set, else [`default_prefix`] of `project`
/// (its name, [`crate::project::project_name`]).
pub fn resolve_prefix(configured: &str, project: Option<&str>) -> Result<String> {
    if !configured.is_empty() {
        check_prefix(configured)?;
        return Ok(configured.to_ascii_uppercase());
    }
    match project.and_then(default_prefix) {
        Some(p) => Ok(p),
        None => {
            bail!("no task prefix: set [tasks] prefix, since the project name has no ASCII letter")
        }
    }
}

/// Where a task stands. `Done` and `Closed` both leave the plan; `Closed` is "won't do",
/// which GitHub (`not_planned`) and GitLab ("Won't do") record natively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Open,
    InProgress,
    Done,
    Closed,
}

impl Status {
    pub const ALL: [Self; 4] = [Self::Open, Self::InProgress, Self::Done, Self::Closed];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InProgress => "in-progress",
            Self::Done => "done",
            Self::Closed => "closed",
        }
    }

    /// Still in the plan: listed by default and blocking a parent from closing.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Open | Self::InProgress)
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Status {
    type Err = anyhow::Error;

    /// Also takes the checklist words Claude Code and Codex use (`pending`, `in_progress`,
    /// `completed`), since agents reach for those first.
    fn from_str(s: &str) -> Result<Self> {
        Ok(
            match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
                "open" | "todo" | "pending" => Self::Open,
                "in-progress" => Self::InProgress,
                "done" | "completed" => Self::Done,
                "closed" | "wontdo" | "won't-do" | "not-planned" => Self::Closed,
                other => {
                    bail!("unknown task status {other:?}: use open, in-progress, done or closed")
                }
            },
        )
    }
}

/// The issue a task lives in on GitHub or GitLab. The rtok id is ours; the number is theirs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalRef {
    /// `github` or `gitlab`.
    pub adapter: String,
    /// GitHub issue number or GitLab issue iid.
    pub number: u64,
    pub url: String,
    /// GraphQL node or global id, for the calls REST cannot make (sub-issues, Status).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

/// What a caller supplies to create a task; the allocator adds the id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewTask {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<TaskId>,
}

/// One task as every adapter returns it, and the JSON shape of the CLI and MCP tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<TaskId>,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds.
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<ExternalRef>,
    /// `0` is highest. [`DEFAULT_PRIORITY`] is omitted from JSON and Markdown.
    #[serde(
        default = "default_priority",
        skip_serializing_if = "is_default_priority"
    )]
    pub priority: u8,
    /// The agent that claimed this task. Empty means nobody.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// Tasks that must finish before this one can be claimed. Only `blocks`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<TaskId>,
    /// Tasks that touch the same files, so two agents should not work on both at once (T441.13.2,
    /// CCPM `conflicts_with`). Symmetric: the disk adapter writes both sides.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts_with: Vec<TaskId>,
    /// Safe to run beside any other task (T441.13.2, CCPM `parallel`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub parallel: bool,
}

impl Task {
    /// An open task with default priority and nobody assigned. `parent` follows the id.
    pub fn open(
        id: TaskId,
        title: impl Into<String>,
        description: impl Into<String>,
        now: i64,
    ) -> Self {
        let parent = id.parent();
        Self {
            id,
            title: title.into(),
            description: description.into(),
            status: Status::Open,
            parent,
            created_at: now,
            updated_at: now,
            external: None,
            priority: DEFAULT_PRIORITY,
            assignee: None,
            blocked_by: Vec::new(),
            conflicts_with: Vec::new(),
            parallel: false,
        }
    }
}

/// One ready task. `stale` means the assignee's `agents.last_seen` is old enough to take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadyItem {
    #[serde(flatten)]
    pub task: Task,
    pub stale: bool,
    /// Conflict partners that are in progress right now (T441.13.2). The task stays ready: this
    /// is a warning to the claimer, not a refusal.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts_in_progress: Vec<TaskId>,
}

/// What `claim` did. `changed` is false when this agent already holds the task, and the
/// file or issue is left untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaimOutcome {
    #[serde(flatten)]
    pub task: Task,
    pub changed: bool,
}

/// Why `claim` refused a task. Callers that try the next ready task match on this.
#[derive(Debug)]
pub enum ClaimConflict {
    Already { id: TaskId, assignee: String },
    NotClaimable { id: TaskId, status: Status },
    Blocked { id: TaskId, by: TaskId },
}

impl fmt::Display for ClaimConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Already { id, assignee } => write!(f, "{id} is claimed by {assignee}"),
            Self::NotClaimable { id, status } => {
                write!(f, "{id} is {status} and cannot be claimed")
            }
            Self::Blocked { id, by } => write!(f, "{id} is blocked by {by}"),
        }
    }
}

impl std::error::Error for ClaimConflict {}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("R12", "R12")]
    #[case("r2.1", "R2.1")]
    #[case(" at7 ", "AT7")]
    #[case("Rtok1.20", "RTOK1.20")]
    fn ids_parse_case_insensitively_and_print_upper_case(#[case] input: &str, #[case] canon: &str) {
        let id: TaskId = input.parse().unwrap();
        assert_eq!(id.to_string(), canon);
        assert_eq!(canon.parse::<TaskId>().unwrap(), id);
    }

    #[rstest]
    #[case("")]
    #[case("R")]
    #[case("12")]
    #[case("R0")]
    #[case("R012")]
    #[case("R1.")]
    #[case("R1..2")]
    #[case("R1.2.3")]
    #[case("R-1")]
    #[case("R1a")]
    #[case("Ж1")]
    #[case("ABCDEFGHI1")]
    #[case("R99999999999")]
    fn malformed_ids_are_refused(#[case] input: &str) {
        assert!(input.parse::<TaskId>().is_err(), "{input:?} parsed");
    }

    #[test]
    fn ids_order_numerically_and_know_their_parent() {
        let mut ids: Vec<TaskId> = ["R10", "R2.1", "R2", "R9", "Q1"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        ids.sort();
        let shown: Vec<String> = ids.iter().map(ToString::to_string).collect();
        assert_eq!(shown, ["Q1", "R2", "R2.1", "R9", "R10"]);
        let sub: TaskId = "R2.1".parse().unwrap();
        assert_eq!(sub.parent(), Some(TaskId::new("r", 2).unwrap()));
        assert_eq!(sub.parent().unwrap().parent(), None);
        assert_eq!(
            TaskId::new("R", 2).unwrap().child(3).unwrap().to_string(),
            "R2.3"
        );
        assert!(sub.child(1).is_err(), "depth past {MAX_DEPTH}");
    }

    #[test]
    fn ids_serialize_as_strings() {
        let id: TaskId = "R2.1".parse().unwrap();
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"R2.1\"");
        assert_eq!(serde_json::from_str::<TaskId>("\"r2.1\"").unwrap(), id);
        assert!(serde_json::from_str::<TaskId>("\"R0\"").is_err());
    }

    #[rstest]
    #[case("rtok", Some("R"))]
    #[case("airtalk", Some("A"))]
    #[case("2fa-app", Some("F"))]
    #[case("123", None)]
    fn default_prefix_is_the_first_letter(#[case] project: &str, #[case] want: Option<&str>) {
        assert_eq!(default_prefix(project).as_deref(), want);
    }

    #[test]
    fn a_configured_prefix_wins_over_the_project_name() {
        assert_eq!(resolve_prefix("at", Some("rtok")).unwrap(), "AT");
        assert_eq!(resolve_prefix("", Some("rtok")).unwrap(), "R");
        assert!(resolve_prefix("", Some("123")).is_err());
        assert!(resolve_prefix("", None).is_err());
        assert!(resolve_prefix("R2", Some("rtok")).is_err());
    }

    #[test]
    fn statuses_round_trip_and_take_host_words() {
        for status in Status::ALL {
            assert_eq!(status.as_str().parse::<Status>().unwrap(), status);
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, format!("\"{status}\""));
        }
        for (word, want) in [
            ("pending", Status::Open),
            ("in_progress", Status::InProgress),
            ("In-Progress", Status::InProgress),
            ("completed", Status::Done),
            ("not_planned", Status::Closed),
        ] {
            assert_eq!(word.parse::<Status>().unwrap(), want, "{word}");
        }
        assert!("blocked".parse::<Status>().is_err());
        assert!(Status::Open.is_active() && Status::InProgress.is_active());
        assert!(!Status::Done.is_active() && !Status::Closed.is_active());
    }

    #[test]
    fn a_task_round_trips_through_json_without_empty_optionals() {
        let task = Task {
            status: Status::InProgress,
            created_at: 1,
            updated_at: 2,
            ..Task::open("R3".parse().unwrap(), "Ship it", "", 0)
        };
        let json = serde_json::to_string(&task).unwrap();
        assert!(
            !json.contains("parent")
                && !json.contains("external")
                && !json.contains("priority")
                && !json.contains("assignee")
                && !json.contains("blocked_by")
                && !json.contains("conflicts_with")
                && !json.contains("parallel"),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<Task>(&json).unwrap(), task);
    }
}
