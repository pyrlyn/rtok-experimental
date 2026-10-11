// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The GitHub Issues exporter (T441.13.5). It drives the `github` task adapter, so labels,
//! titles, sub-issue links and the close reasons are exactly what that adapter writes and
//! reads: an exported repository can later be adopted as the project's task store.

use anyhow::Result;

use super::{Action, Exporter, Kind, drifted};
use crate::tasks::adapter::{Filter, TaskAdapter};
use crate::tasks::github::GithubAdapter;
use crate::tasks::{NewTask, Task};

pub struct GithubExporter {
    adapter: GithubAdapter,
    repo: String,
}

impl GithubExporter {
    pub fn new(adapter: GithubAdapter, repo: &str) -> Self {
        Self {
            adapter,
            repo: repo.to_string(),
        }
    }
}

impl Exporter for GithubExporter {
    fn name(&self) -> &'static str {
        "github"
    }

    fn target(&self) -> String {
        self.repo.clone()
    }

    fn remote(&self) -> Result<Vec<Task>> {
        self.adapter.list(&Filter {
            all: true,
            ..Filter::default()
        })
    }

    fn apply(&self, action: &Action, task: &Task) -> Result<Option<String>> {
        let issue = match action.kind {
            Kind::Create => {
                let new = NewTask {
                    title: task.title.clone(),
                    description: task.description.clone(),
                    parent: task.parent.clone(),
                };
                let created = self.adapter.create(&new, &task.id)?;
                // `create` opens a bare issue; labels and a finished state need one more write.
                if drifted(task, &created, true).is_empty() {
                    created
                } else {
                    self.adapter.patch(task, true)?
                }
            }
            Kind::Update | Kind::Close | Kind::Reopen => self.adapter.patch(task, true)?,
        };
        Ok(issue.external.map(|x| x.url))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use httpmock::MockServer;
    use serde_json::{Value, json};

    use super::*;
    use crate::tasks::export::{self, Report};
    use crate::tasks::{Status, TaskId};

    const ISSUES: &str = "/repos/me/app/issues";

    fn id(s: &str) -> TaskId {
        s.parse().unwrap()
    }

    fn exporter(server: &MockServer) -> GithubExporter {
        let gh = GithubAdapter::new(&server.base_url(), "me/app", "tok", Duration::ZERO).unwrap();
        GithubExporter::new(gh, "me/app")
    }

    /// An issue as the REST API returns it, trimmed to what the adapter reads.
    fn issue(number: u64, task: &str, title: &str, state: &str, reason: Option<&str>) -> Value {
        json!({
            "id": number * 100,
            "node_id": format!("I_kw{number}"),
            "number": number,
            "html_url": format!("https://github.com/me/app/issues/{number}"),
            "title": format!("{task}. {title}"),
            "body": "Why.",
            "state": state,
            "state_reason": reason,
            "labels": [{"name": "rtok"}, {"name": format!("rtok:{task}")}],
            "created_at": "2026-10-07T10:00:00Z",
            "updated_at": "2026-10-07T11:00:00Z",
        })
    }

    fn local(task: &str, title: &str, status: Status) -> Task {
        Task {
            status,
            ..Task::open(id(task), title, "Why.", 0)
        }
    }

    fn listing<'a>(server: &'a MockServer, issues: Value) -> httpmock::Mock<'a> {
        server.mock(|when, then| {
            when.method("GET")
                .path(ISSUES)
                .query_param("labels", "rtok")
                .query_param("state", "all")
                .header("authorization", "Bearer tok");
            then.status(200).json_body(issues);
        })
    }

    fn by_label<'a>(server: &'a MockServer, label: &str, issues: Value) -> httpmock::Mock<'a> {
        server.mock(|when, then| {
            when.method("GET")
                .path(ISSUES)
                .query_param("labels", label)
                .query_param("state", "all");
            then.status(200).json_body(issues);
        })
    }

    fn summary(r: &Report) -> Vec<String> {
        r.actions
            .iter()
            .map(|a| format!("{:?} {}", a.kind, a.id))
            .collect()
    }

    #[test]
    fn a_new_task_becomes_an_issue_and_a_second_run_sends_nothing() {
        let server = MockServer::start();
        let tasks = [local("R1", "Ship", Status::Open)];

        let mut empty = listing(&server, json!([]));
        by_label(&server, "rtok:R1", json!([]));
        let post = server.mock(|when, then| {
            when.method("POST").path(ISSUES).json_body(json!({
                "title": "R1. Ship",
                "body": "Why.",
                "labels": ["rtok", "rtok:R1"],
            }));
            then.status(201)
                .json_body(issue(7, "R1", "Ship", "open", None));
        });
        let first = export::run(&exporter(&server), &tasks, false).unwrap();
        assert_eq!(summary(&first), ["Create R1"]);
        assert_eq!(
            first.actions[0].url.as_deref(),
            Some("https://github.com/me/app/issues/7")
        );
        post.assert();
        empty.delete();

        listing(&server, json!([issue(7, "R1", "Ship", "open", None)]));
        let again = export::run(&exporter(&server), &tasks, false).unwrap();
        assert!(again.actions.is_empty(), "{again:?}");
        assert_eq!(again.unchanged, 1);
        post.assert_calls(1);
    }

    #[test]
    fn drift_is_patched_and_done_and_closed_tasks_close_with_their_reason() {
        let server = MockServer::start();
        listing(
            &server,
            json!([
                issue(1, "R1", "Old title", "open", None),
                issue(2, "R2", "Two", "open", None),
                issue(3, "R3", "Three", "open", None),
                issue(4, "R4", "Four", "closed", Some("completed")),
            ]),
        );
        for (n, task) in [(1, "R1"), (2, "R2"), (3, "R3")] {
            by_label(
                &server,
                &format!("rtok:{task}"),
                json!([issue(n, task, "x", "open", None)]),
            );
        }
        by_label(
            &server,
            "rtok:R4",
            json!([issue(4, "R4", "Four", "closed", Some("completed"))]),
        );
        let retitle = server.mock(|when, then| {
            when.method("PATCH")
                .path(format!("{ISSUES}/1"))
                .json_body(json!({
                    "title": "R1. New title",
                    "body": "Why.",
                    "labels": ["rtok", "rtok:R1"],
                    "state": "open",
                }));
            then.status(200)
                .json_body(issue(1, "R1", "New title", "open", None));
        });
        let done = server.mock(|when, then| {
            when.method("PATCH")
                .path(format!("{ISSUES}/2"))
                .json_body_includes(r#"{"state":"closed","state_reason":"completed"}"#);
            then.status(200)
                .json_body(issue(2, "R2", "Two", "closed", Some("completed")));
        });
        let wont = server.mock(|when, then| {
            when.method("PATCH")
                .path(format!("{ISSUES}/3"))
                .json_body_includes(r#"{"state":"closed","state_reason":"not_planned"}"#);
            then.status(200)
                .json_body(issue(3, "R3", "Three", "closed", Some("not_planned")));
        });
        let reopen = server.mock(|when, then| {
            when.method("PATCH")
                .path(format!("{ISSUES}/4"))
                .json_body_includes(r#"{"state":"open","state_reason":"reopened"}"#);
            then.status(200)
                .json_body(issue(4, "R4", "Four", "open", None));
        });
        let tasks = [
            local("R1", "New title", Status::Open),
            local("R2", "Two", Status::Done),
            local("R3", "Three", Status::Closed),
            local("R4", "Four", Status::Open),
            local("R9", "Old history", Status::Done),
        ];
        let report = export::run(&exporter(&server), &tasks, false).unwrap();
        assert_eq!(
            summary(&report),
            ["Update R1", "Close R2", "Close R3", "Reopen R4"]
        );
        assert_eq!(report.skipped, 1, "R9 is finished and has no issue");
        for m in [retitle, done, wont, reopen] {
            m.assert();
        }
    }

    #[test]
    fn dry_run_lists_the_writes_and_sends_none() {
        let server = MockServer::start();
        listing(&server, json!([issue(1, "R1", "Old", "open", None)]));
        let write = server.mock(|when, then| {
            when.is_true(|req| req.method().as_str() != "GET");
            then.status(500);
        });
        let tasks = [
            local("R1", "New", Status::Open),
            local("R2", "Fresh", Status::Open),
        ];
        let report = export::run(&exporter(&server), &tasks, true).unwrap();
        assert!(report.dry_run);
        assert_eq!(summary(&report), ["Update R1", "Create R2"]);
        assert_eq!(report.actions[0].fields, ["title"]);
        write.assert_calls(0);
    }

    #[test]
    fn an_in_progress_task_is_created_then_labelled() {
        let server = MockServer::start();
        listing(&server, json!([]));
        // The same label lookup is empty before the POST and finds the issue after it.
        let posted = Arc::new(AtomicBool::new(false));
        let flag = posted.clone();
        server.mock(|when, then| {
            when.method("POST").path(ISSUES).is_true(move |req| {
                flag.store(req.method().as_str() == "POST", Ordering::SeqCst);
                true
            });
            then.status(201)
                .json_body(issue(5, "R1", "Go", "open", None));
        });
        for (after, issues) in [
            (false, json!([])),
            (true, json!([issue(5, "R1", "Go", "open", None)])),
        ] {
            let seen = posted.clone();
            server.mock(|when, then| {
                when.method("GET")
                    .path(ISSUES)
                    .query_param("labels", "rtok:R1")
                    .is_true(move |_| seen.load(Ordering::SeqCst) == after);
                then.status(200).json_body(issues);
            });
        }
        let patch = server.mock(|when, then| {
            when.method("PATCH")
                .path(format!("{ISSUES}/5"))
                .json_body_includes(r#"{"labels":["rtok","rtok:R1","rtok:owner:a1","rtok:needs:R2","rtok:p:0","rtok:in-progress"]}"#);
            then.status(200).json_body(issue(5, "R1", "Go", "open", None));
        });
        let mut task = local("R1", "Go", Status::InProgress);
        task.assignee = Some("a1".into());
        task.blocked_by = vec![id("R2")];
        task.priority = 0;
        let report = export::run(&exporter(&server), &[task], false).unwrap();
        assert_eq!(summary(&report), ["Create R1"]);
        patch.assert();
    }

    #[test]
    fn a_failed_write_names_the_backend_the_task_and_how_far_it_got() {
        let server = MockServer::start();
        listing(&server, json!([]));
        by_label(&server, "rtok:R1", json!([]));
        server.mock(|when, then| {
            when.method("POST").path(ISSUES);
            then.status(422)
                .json_body(json!({"message": "Validation Failed"}));
        });
        let err = export::run(
            &exporter(&server),
            &[local("R1", "Go", Status::Open)],
            false,
        )
        .unwrap_err();
        let text = format!("{err:#}");
        assert!(
            text.contains("github export: create R1 failed after 0 of 1 writes")
                && text.contains(ISSUES)
                && text.contains("Validation Failed"),
            "{text}"
        );
        assert!(!text.contains("tok\""), "no token in errors");
    }
}
