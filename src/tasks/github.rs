// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The `github` adapter (T441 §7): one issue per task through the REST API. The issue carries
//! the `rtok` label and `rtok:R12`, its title starts with the id, a subtask is a sub-issue of
//! its parent's issue, `rtok:in-progress` marks work under way, and done or closed close the
//! issue as `completed` or `not_planned` (§8). With `[tasks.github] project` set, the issue
//! also joins that Projects v2 board and its Status follows the task (`github_project.rs`).

use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::Method;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::{Value, json};

use super::adapter::{Filter, Stray, Taken, TaskAdapter};
use super::github_project::ProjectSync;
use super::meta;
use super::remote::{
    Http, LABEL, id_label, issue_title, label_id, max_with_prefix, secs, task_title, title_id,
};
use super::{ExternalRef, NewTask, Status, Task, TaskId};

/// The public GitHub REST API.
pub const API: &str = "https://api.github.com";

/// The label `in-progress` sets; open without it is `open`.
pub const IN_PROGRESS: &str = "rtok:in-progress";

/// GitHub rejects a label name longer than this. `rtok:owner:` plus a UUID is 47.
const LABEL_MAX: usize = 50;

pub struct GithubAdapter {
    http: Http,
    repo: String,
    /// `[tasks.github] project`: the Projects v2 board whose Status follows the task.
    project: Option<ProjectSync>,
}

#[derive(Deserialize)]
struct Issue {
    /// The database id the sub-issues API takes, not the number.
    id: u64,
    number: u64,
    #[serde(default)]
    node_id: Option<String>,
    html_url: String,
    title: String,
    #[serde(default)]
    body: Option<String>,
    state: String,
    #[serde(default)]
    state_reason: Option<String>,
    #[serde(default)]
    labels: Vec<Label>,
    created_at: String,
    updated_at: String,
    /// Set on pull requests, which share the issues list and numbering.
    #[serde(default)]
    pull_request: Option<Value>,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

impl Issue {
    fn task_id(&self) -> Option<TaskId> {
        self.labels.iter().find_map(|l| label_id(&l.name))
    }

    fn into_task(self) -> Option<Task> {
        let id = self.task_id()?;
        let status = match (self.state.as_str(), self.state_reason.as_deref()) {
            ("closed", Some("not_planned" | "duplicate")) => Status::Closed,
            ("closed", _) => Status::Done,
            _ if self.labels.iter().any(|l| l.name == IN_PROGRESS) => Status::InProgress,
            _ => Status::Open,
        };
        let names: Vec<&str> = self.labels.iter().map(|l| l.name.as_str()).collect();
        let meta = meta::read_meta(&names);
        Some(Task {
            title: task_title(&id, &self.title),
            description: self.body.unwrap_or_default().trim().to_string(),
            status,
            parent: id.parent(),
            created_at: secs(&self.created_at),
            updated_at: secs(&self.updated_at),
            external: Some(ExternalRef {
                adapter: "github".into(),
                number: self.number,
                url: self.html_url,
                node_id: self.node_id,
            }),
            priority: meta.priority,
            assignee: meta.assignee,
            blocked_by: meta.blocked_by,
            conflicts_with: meta.conflicts_with,
            parallel: meta.parallel,
            id,
        })
    }
}

/// `[tasks.github] repo`, else the `owner/name` of a `github.com` origin (`key` is
/// [`crate::project::project_key`]). Checked, since it goes into every request path.
pub fn repo(configured: &str, key: &str) -> Result<String> {
    let repo = if configured.is_empty() {
        key.strip_prefix("github.com/").with_context(|| {
            format!("github tasks: origin {key} is not on github.com; set [tasks.github] repo")
        })?
    } else {
        configured
    };
    let ok = |s: &str| {
        !s.is_empty()
            && s != ".."
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    match repo.split_once('/') {
        Some((owner, name)) if ok(owner) && ok(name) => Ok(repo.to_string()),
        _ => bail!("github tasks: repo {repo:?} is not owner/name"),
    }
}

impl GithubAdapter {
    /// `base` is [`API`] (a mock server in tests); `gap` paces writes
    /// ([`super::remote::WRITE_GAP`]).
    pub fn new(base: &str, repo: &str, token: &str, gap: Duration) -> Result<Self> {
        let mut headers = HeaderMap::new();
        let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))
            .context("github tasks: the token is not a valid header value")?;
        auth.set_sensitive(true);
        headers.insert(AUTHORIZATION, auth);
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "x-github-api-version",
            HeaderValue::from_static("2022-11-28"),
        );
        Ok(Self {
            http: Http::new("github", base, headers, gap)?,
            repo: repo.to_string(),
            project: None,
        })
    }

    /// Mirror each task's status into the Status field of Projects v2 project `number`
    /// (owned by the repo's owner); 0 keeps the adapter on issues only.
    pub fn with_project(mut self, number: u64) -> Self {
        self.project = (number != 0).then(|| ProjectSync::new(&self.repo, number));
        self
    }

    fn sync_project(&self, id: &TaskId, issue: &Issue, status: Status) {
        if let Some(p) = &self.project {
            p.sync(&self.http, id, issue.node_id.as_deref(), status);
        }
    }

    /// rtok's issues with `label`, pull requests left out; `state` is `open` or `all`.
    fn issues(&self, label: &str, state: &str) -> Result<Vec<Issue>> {
        let path = format!("/repos/{}/issues", self.repo);
        let query = [("labels", label), ("state", state), ("per_page", "100")];
        let mut issues: Vec<Issue> = self.http.get_all(&path, &query)?;
        issues.retain(|i| i.pull_request.is_none());
        Ok(issues)
    }

    /// The issue labelled `id`, open or closed. A label lookup, not the search API: search
    /// allows 30 requests a minute and lags behind writes (`research.md` §35.4).
    fn find(&self, id: &TaskId) -> Result<Option<Issue>> {
        Ok(self
            .issues(&id_label(id), "all")?
            .into_iter()
            .find(|i| i.task_id().as_ref() == Some(id)))
    }

    /// The `save` write. `text` also rewrites title and body: the exporter (T441.13.5) owns
    /// them, while a claim must not clobber what people edited on the issue.
    pub(super) fn patch(&self, task: &Task, text: bool) -> Result<Task> {
        let issue = self
            .find(&task.id)?
            .with_context(|| format!("github tasks: no task {} in {}", task.id, self.repo))?;
        for label in meta::managed_for(task) {
            if label.len() > LABEL_MAX {
                bail!("github tasks: label {label:?} is longer than {LABEL_MAX} characters");
            }
        }
        let names: Vec<String> = issue.labels.iter().map(|l| l.name.clone()).collect();
        let mut labels = meta::merge_labels(&names, task);
        labels.retain(|label| label != IN_PROGRESS);
        if task.status == Status::InProgress {
            labels.push(IN_PROGRESS.to_string());
        }
        let mut body = json!({
            "labels": labels,
            "state": if task.status.is_active() { "open" } else { "closed" },
        });
        if text {
            body["title"] = json!(issue_title(&task.id, &task.title));
            body["body"] = json!(task.description);
        }
        let reason = match task.status {
            Status::Done => Some("completed"),
            Status::Closed => Some("not_planned"),
            _ if issue.state == "closed" => Some("reopened"),
            _ => None,
        };
        if let Some(reason) = reason {
            body["state_reason"] = json!(reason);
        }
        let path = format!("/repos/{}/issues/{}", self.repo, issue.number);
        let updated: Issue = self.http.send(Method::PATCH, &path, &body)?;
        self.sync_project(&task.id, &updated, task.status);
        updated
            .into_task()
            .with_context(|| format!("github tasks: {} lost its label", task.id))
    }
}

impl TaskAdapter for GithubAdapter {
    fn name(&self) -> &'static str {
        "github"
    }

    fn create(&self, task: &NewTask, id: &TaskId) -> Result<Task> {
        if task.title.trim().is_empty() {
            bail!("github tasks: a task needs a title");
        }
        // The counter is per machine: another machine may already have issued this id.
        if self.find(id)?.is_some() {
            return Err(Taken(id.clone()).into());
        }
        let parent =
            match id.parent() {
                Some(p) => Some(self.find(&p)?.with_context(|| {
                    format!("github tasks: no parent task {p} in {}", self.repo)
                })?),
                None => None,
            };
        let label = id_label(id);
        let body = json!({
            "title": issue_title(id, &task.title),
            "body": task.description,
            "labels": [LABEL, label],
        });
        let issue: Issue =
            self.http
                .send(Method::POST, &format!("/repos/{}/issues", self.repo), &body)?;
        // GitHub drops labels silently for users without push access, and an issue without
        // its label is invisible to every later call.
        if issue.task_id().as_ref() != Some(id) {
            bail!(
                "github tasks: {} was created without the {label} label (labels need push access to {}); label or close it by hand",
                issue.html_url,
                self.repo
            );
        }
        if let Some(p) = parent {
            let path = format!("/repos/{}/issues/{}/sub_issues", self.repo, p.number);
            // The id already names the parent for rtok; the link only helps people browsing
            // GitHub, so a failed link must not report the stored task as not created.
            if let Err(e) =
                self.http
                    .send::<Value>(Method::POST, &path, &json!({"sub_issue_id": issue.id}))
            {
                log::warn!(
                    "github tasks: {id} is not linked under {}: {e:#}",
                    p.html_url
                );
            }
        }
        self.sync_project(id, &issue, Status::Open);
        issue
            .into_task()
            .context("github tasks: the new issue has no task id")
    }

    fn list(&self, filter: &Filter) -> Result<Vec<Task>> {
        let state = if filter.wants_finished() {
            "all"
        } else {
            "open"
        };
        let issues = self.issues(LABEL, state)?;
        Ok(filter.select(issues.into_iter().filter_map(Issue::into_task)))
    }

    fn get(&self, id: &TaskId) -> Result<Option<Task>> {
        Ok(self.find(id)?.and_then(Issue::into_task))
    }

    fn write_status(&self, id: &TaskId, status: Status) -> Result<Task> {
        let issue = self
            .find(id)?
            .with_context(|| format!("github tasks: no task {id} in {}", self.repo))?;
        let mut labels: Vec<&str> = issue
            .labels
            .iter()
            .map(|l| l.name.as_str())
            .filter(|n| *n != IN_PROGRESS)
            .collect();
        if status == Status::InProgress {
            labels.push(IN_PROGRESS);
        }
        let mut body = json!({
            "labels": labels,
            "state": if status.is_active() { "open" } else { "closed" },
        });
        let reason = match status {
            Status::Done => Some("completed"),
            Status::Closed => Some("not_planned"),
            _ if issue.state == "closed" => Some("reopened"),
            _ => None,
        };
        if let Some(r) = reason {
            body["state_reason"] = json!(r);
        }
        let path = format!("/repos/{}/issues/{}", self.repo, issue.number);
        let updated: Issue = self.http.send(Method::PATCH, &path, &body)?;
        self.sync_project(id, &updated, status);
        updated
            .into_task()
            .with_context(|| format!("github tasks: {id} lost its label"))
    }

    /// Assignee, blockers and priority. Not compare-and-set: two callers can both PATCH, and
    /// the last write wins. `claim` refuses a live foreign assignee before this runs.
    fn save(&self, task: &Task) -> Result<Task> {
        self.patch(task, false)
    }

    fn stores_markers(&self) -> bool {
        true
    }

    fn max_id(&self, prefix: &str) -> Result<Option<TaskId>> {
        let issues = self.issues(LABEL, "all")?;
        Ok(max_with_prefix(
            issues.iter().filter_map(Issue::task_id),
            prefix,
        ))
    }

    fn unlabelled(&self) -> Result<Vec<Stray>> {
        Ok(self
            .issues(LABEL, "all")?
            .into_iter()
            .filter(|i| i.task_id().is_none())
            .map(|i| Stray {
                id: title_id(&i.title),
                title: i.title,
                url: i.html_url,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::super::adapter::set_status;
    use super::super::run::Project;
    use super::*;
    use crate::store::Store;
    use httpmock::MockServer;

    const ISSUES: &str = "/repos/me/app/issues";

    fn id(s: &str) -> TaskId {
        s.parse().unwrap()
    }

    /// An issue as the REST API returns it, trimmed to the fields the adapter reads.
    fn issue(number: u64, task: &str, state: &str, reason: Option<&str>, extra: &[&str]) -> Value {
        let mut labels = vec![
            json!({"id": 1, "name": "rtok"}),
            json!({"name": format!("rtok:{task}")}),
        ];
        labels.extend(extra.iter().map(|n| json!({"name": n})));
        json!({
            "id": number * 100,
            "node_id": format!("I_kw{number}"),
            "number": number,
            "html_url": format!("https://github.com/me/app/issues/{number}"),
            "title": format!("{task}. Task {number}"),
            "body": "Why.\n",
            "state": state,
            "state_reason": reason,
            "labels": labels,
            "created_at": "2026-10-07T10:00:00Z",
            "updated_at": "2026-10-07T11:00:00Z",
        })
    }

    fn adapter(server: &MockServer) -> GithubAdapter {
        GithubAdapter::new(&server.base_url(), "me/app", "test-token", Duration::ZERO).unwrap()
    }

    fn by_label<'a>(
        server: &'a MockServer,
        label: &str,
        state: &str,
        body: Value,
    ) -> httpmock::Mock<'a> {
        server.mock(|when, then| {
            when.method("GET")
                .path(ISSUES)
                .query_param("labels", label)
                .query_param("state", state)
                .header("authorization", "Bearer test-token");
            then.status(200).json_body(body);
        })
    }

    #[test]
    fn create_labels_the_issue_and_links_it_under_its_parent() {
        let server = MockServer::start();
        by_label(&server, "rtok:A2.1", "all", json!([]));
        by_label(
            &server,
            "rtok:A2",
            "all",
            json!([issue(5, "A2", "open", None, &[])]),
        );
        let post = server.mock(|when, then| {
            when.method("POST").path(ISSUES).json_body(json!({
                "title": "A2.1. Leaf",
                "body": "Done means tested.",
                "labels": ["rtok", "rtok:A2.1"],
            }));
            then.status(201)
                .json_body(issue(9, "A2.1", "open", None, &[]));
        });
        let link = server.mock(|when, then| {
            when.method("POST")
                .path(format!("{ISSUES}/5/sub_issues"))
                .json_body(json!({"sub_issue_id": 900}));
            then.status(201)
                .json_body(issue(5, "A2", "open", None, &[]));
        });
        let new = NewTask {
            title: " Leaf ".into(),
            description: "Done means tested.".into(),
            parent: Some(id("A2")),
        };
        let task = adapter(&server).create(&new, &id("A2.1")).unwrap();
        post.assert();
        link.assert();
        assert_eq!(task.title, "Task 9", "read back from the issue title");
        assert_eq!(task.parent, Some(id("A2")));
        assert_eq!(task.status, Status::Open);
        assert_eq!(task.created_at, 1_791_367_200);
        let x = task.external.unwrap();
        assert_eq!((x.number, x.node_id.as_deref()), (9, Some("I_kw9")));
        assert_eq!(x.url, "https://github.com/me/app/issues/9");
    }

    #[test]
    fn create_refuses_a_taken_id_a_missing_parent_and_a_dropped_label() {
        let server = MockServer::start();
        let gh = adapter(&server);
        by_label(
            &server,
            "rtok:A1",
            "all",
            json!([issue(1, "A1", "closed", Some("completed"), &[])]),
        );
        let err = gh.create(&new("Again"), &id("A1")).unwrap_err();
        assert!(err.downcast_ref::<Taken>().is_some(), "{err}");

        by_label(&server, "rtok:A3.1", "all", json!([]));
        by_label(&server, "rtok:A3", "all", json!([]));
        let err = gh.create(&new("Orphan"), &id("A3.1")).unwrap_err();
        assert!(err.to_string().contains("no parent task A3"), "{err}");

        by_label(&server, "rtok:A4", "all", json!([]));
        server.mock(|when, then| {
            when.method("POST").path(ISSUES);
            let mut bare = issue(12, "A4", "open", None, &[]);
            bare["labels"] = json!([]);
            then.status(201).json_body(bare);
        });
        let err = gh.create(&new("Unlabelled"), &id("A4")).unwrap_err();
        assert!(err.to_string().contains("push access"), "{err}");
    }

    fn new(title: &str) -> NewTask {
        NewTask {
            title: title.into(),
            description: String::new(),
            parent: None,
        }
    }

    #[test]
    fn list_reads_rtok_issues_and_skips_pull_requests_and_strangers() {
        let server = MockServer::start();
        let mut pr = issue(3, "A3", "open", None, &[]);
        pr["pull_request"] = json!({"url": "x"});
        let mut stranger = issue(4, "A4", "open", None, &[]);
        stranger["labels"] = json!([{"name": "rtok"}]);
        let open = by_label(
            &server,
            "rtok",
            "open",
            json!([
                issue(2, "A2", "open", None, &[IN_PROGRESS]),
                pr,
                stranger,
                issue(1, "A1", "open", None, &[])
            ]),
        );
        by_label(
            &server,
            "rtok",
            "all",
            json!([
                issue(1, "A1", "open", None, &[]),
                issue(7, "A1.1", "closed", Some("not_planned"), &[]),
                issue(8, "B9", "closed", None, &[])
            ]),
        );
        let gh = adapter(&server);
        let plan = gh.list(&Filter::default()).unwrap();
        let ids: Vec<String> = plan.iter().map(|t| t.id.to_string()).collect();
        assert_eq!(ids, ["A1", "A2"]);
        assert_eq!(plan[1].status, Status::InProgress);
        open.assert();
        let closed = gh
            .list(&Filter {
                statuses: vec![Status::Closed],
                ..Filter::default()
            })
            .unwrap();
        assert_eq!(closed[0].id, id("A1.1"));
        assert_eq!(gh.max_id("a").unwrap(), Some(id("A1.1")));
        assert_eq!(gh.max_id("B").unwrap(), Some(id("B9")));
        assert_eq!(gh.max_id("Q").unwrap(), None);
    }

    #[test]
    fn status_moves_the_label_and_closes_done_and_wont_do() {
        let server = MockServer::start();
        let gh = adapter(&server);
        by_label(
            &server,
            "rtok:A1",
            "all",
            json!([issue(1, "A1", "open", None, &["bug"])]),
        );
        by_label(&server, "rtok", "open", json!([]));
        by_label(
            &server,
            "rtok:A2",
            "all",
            json!([issue(2, "A2", "closed", Some("completed"), &[IN_PROGRESS])]),
        );
        let started = server.mock(|when, then| {
            when.method("PATCH")
                .path(format!("{ISSUES}/1"))
                .json_body(json!({
                    "labels": ["rtok", "rtok:A1", "bug", IN_PROGRESS],
                    "state": "open",
                }));
            then.status(200)
                .json_body(issue(1, "A1", "open", None, &["bug", IN_PROGRESS]));
        });
        let done = server.mock(|when, then| {
            when.method("PATCH")
                .path(format!("{ISSUES}/1"))
                .json_body_includes(r#"{"state":"closed","state_reason":"completed"}"#);
            then.status(200)
                .json_body(issue(1, "A1", "closed", Some("completed"), &["bug"]));
        });
        let reopened = server.mock(|when, then| {
            when.method("PATCH")
                .path(format!("{ISSUES}/2"))
                .json_body(json!({
                    "labels": ["rtok", "rtok:A2"],
                    "state": "open",
                    "state_reason": "reopened",
                }));
            then.status(200)
                .json_body(issue(2, "A2", "open", Some("reopened"), &[]));
        });
        let wont = server.mock(|when, then| {
            when.method("PATCH")
                .path(format!("{ISSUES}/2"))
                .json_body_includes(r#"{"state":"closed","state_reason":"not_planned"}"#);
            then.status(200)
                .json_body(issue(2, "A2", "closed", Some("not_planned"), &[]));
        });

        let t = set_status(&gh, &id("A1"), Status::InProgress, false).unwrap();
        assert_eq!(t.status, Status::InProgress);
        let t = set_status(&gh, &id("A1"), Status::Done, false).unwrap();
        assert_eq!(t.status, Status::Done);
        assert_eq!(
            set_status(&gh, &id("A2"), Status::Open, false)
                .unwrap()
                .status,
            Status::Open
        );
        let t = set_status(&gh, &id("A2"), Status::Closed, false).unwrap();
        assert_eq!(t.status, Status::Closed);
        for m in [started, done, reopened, wont] {
            m.assert();
        }
        by_label(&server, "rtok:A9", "all", json!([]));
        assert!(gh.write_status(&id("A9"), Status::Done).is_err());
    }

    #[test]
    fn an_id_another_machine_took_is_skipped_on_create() {
        let server = MockServer::start();
        by_label(
            &server,
            "rtok",
            "all",
            json!([issue(1, "A1", "open", None, &[])]),
        );
        // Another machine created A2 after this one's seed listed the repository.
        by_label(
            &server,
            "rtok:A2",
            "all",
            json!([issue(2, "A2", "open", None, &[])]),
        );
        by_label(&server, "rtok:A3", "all", json!([]));
        server.mock(|when, then| {
            when.method("POST")
                .path(ISSUES)
                .json_body_includes(r#"{"labels":["rtok","rtok:A3"]}"#);
            then.status(201)
                .json_body(issue(3, "A3", "open", None, &[]));
        });
        let project = Project::with_adapter(
            "github.com/me/app".into(),
            std::env::temp_dir(),
            "A".into(),
            Box::new(adapter(&server)),
        );
        let store = Store::open_in_memory().unwrap();
        let task = project.create(&store, &new("Mine")).unwrap();
        assert_eq!(task.id, id("A3"));
    }

    #[test]
    fn the_repo_comes_from_config_or_a_github_origin() {
        assert_eq!(repo("", "github.com/me/app").unwrap(), "me/app");
        assert_eq!(repo("org/x.rs", "gitlab.com/a/b").unwrap(), "org/x.rs");
        assert!(repo("", "gitlab.com/a/b").is_err());
        assert!(repo("../etc", "").is_err());
        assert!(repo("me/app?x=1", "").is_err());
        assert!(repo("me/app/extra", "").is_err());
    }

    #[test]
    fn save_merges_claim_labels_and_keeps_the_users() {
        let server = MockServer::start();
        let gh = adapter(&server);
        by_label(
            &server,
            "rtok:A1",
            "all",
            json!([issue(1, "A1", "open", None, &["bug", "rtok:p:0"])]),
        );
        let patch = server.mock(|when, then| {
            when.method("PATCH")
                .path(format!("{ISSUES}/1"))
                .json_body(json!({
                    "labels": [
                        "rtok",
                        "rtok:A1",
                        "bug",
                        "rtok:owner:agent-1",
                        "rtok:needs:A2",
                        "rtok:p:0",
                        IN_PROGRESS,
                    ],
                    "state": "open",
                }));
            then.status(200).json_body(issue(
                1,
                "A1",
                "open",
                None,
                &[
                    "bug",
                    "rtok:owner:agent-1",
                    "rtok:needs:A2",
                    "rtok:p:0",
                    IN_PROGRESS,
                ],
            ));
        });
        let mut task = gh.get(&id("A1")).unwrap().unwrap();
        assert_eq!(task.priority, 0);
        task.assignee = Some("agent-1".into());
        task.blocked_by = vec![id("A2")];
        task.status = Status::InProgress;
        let saved = gh.save(&task).unwrap();
        patch.assert();
        assert_eq!(saved.assignee.as_deref(), Some("agent-1"));
        assert_eq!(saved.blocked_by, vec![id("A2")]);
        assert_eq!(saved.priority, 0);
        assert_eq!(saved.status, Status::InProgress);
    }

    #[test]
    fn save_stores_conflict_and_parallel_markers_and_reads_them_back() {
        let server = MockServer::start();
        let gh = adapter(&server);
        by_label(
            &server,
            "rtok:A1",
            "all",
            json!([issue(1, "A1", "open", None, &["bug", "rtok:conflicts:A9"])]),
        );
        let patch = server.mock(|when, then| {
            when.method("PATCH")
                .path(format!("{ISSUES}/1"))
                .json_body(json!({
                    "labels": [
                        "rtok",
                        "rtok:A1",
                        "bug",
                        "rtok:conflicts:A2",
                        "rtok:conflicts:A3",
                        "rtok:parallel",
                    ],
                    "state": "open",
                }));
            then.status(200).json_body(issue(
                1,
                "A1",
                "open",
                None,
                &[
                    "bug",
                    "rtok:conflicts:A2",
                    "rtok:conflicts:A3",
                    "rtok:parallel",
                ],
            ));
        });
        let mut task = gh.get(&id("A1")).unwrap().unwrap();
        assert_eq!(task.conflicts_with, vec![id("A9")]);
        assert!(!task.parallel);
        task.conflicts_with = vec![id("A2"), id("A3")];
        task.parallel = true;
        let saved = gh.save(&task).unwrap();
        patch.assert();
        assert_eq!(saved.conflicts_with, vec![id("A2"), id("A3")]);
        assert!(saved.parallel);
        assert!(gh.stores_markers());
    }

    #[test]
    fn task_conflict_writes_both_issues_and_task_parallel_sets_the_label() {
        let server = MockServer::start();
        for (n, task) in [(1, "A1"), (2, "A2")] {
            by_label(
                &server,
                &format!("rtok:{task}"),
                "all",
                json!([issue(n, task, "open", None, &["bug"])]),
            );
        }
        let edge = |n: u64, task: &str, peer: &str| {
            let label = format!("rtok:conflicts:{peer}");
            let labels = json!(["rtok", format!("rtok:{task}"), "bug", label.clone()]);
            server.mock(|when, then| {
                when.method("PATCH")
                    .path(format!("{ISSUES}/{n}"))
                    .json_body(json!({"labels": labels, "state": "open"}));
                then.status(200)
                    .json_body(issue(n, task, "open", None, &["bug", &label]));
            })
        };
        let (first, second) = (edge(1, "A1", "A2"), edge(2, "A2", "A1"));
        let project = Project::with_adapter(
            "github.com/me/app".into(),
            std::env::temp_dir(),
            "A".into(),
            Box::new(adapter(&server)),
        );
        let task = project.conflict(&id("A1"), &id("A2"), false).unwrap();
        first.assert();
        second.assert();
        assert_eq!(task.conflicts_with, vec![id("A2")]);

        let parallel = server.mock(|when, then| {
            when.method("PATCH").path(format!("{ISSUES}/1")).json_body(
                json!({"labels": ["rtok", "rtok:A1", "bug", "rtok:parallel"], "state": "open"}),
            );
            then.status(200)
                .json_body(issue(1, "A1", "open", None, &["bug", "rtok:parallel"]));
        });
        assert!(project.set_parallel(&id("A1"), true).unwrap().parallel);
        parallel.assert();
    }
}
