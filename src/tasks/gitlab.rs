// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The `gitlab` adapter (T441 §7): one issue per task through the REST API v4, on gitlab.com
//! or a self-hosted instance. The issue carries the `rtok` label and `rtok:R12` and its title
//! starts with the id, as on GitHub. Status rides on `status::` labels: scoped (one at a time)
//! on Premium, plain on Free, so the adapter removes the others itself. Done and closed close
//! the issue; REST has no close reason, so `status::done` or `status::wont-do` tells them
//! apart. A subtask's issue gets a `relates_to` link to its parent's, since the parent-child
//! hierarchy is GraphQL only (`research.md` §35.3).

use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::Method;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::{Value, json};

use super::adapter::{Filter, Taken, TaskAdapter};
use super::meta;
use super::remote::{
    Http, LABEL, id_label, issue_title, label_id, max_with_prefix, secs, task_title,
};
use super::{ExternalRef, NewTask, Status, Task, TaskId};

pub const IN_PROGRESS: &str = "status::in-progress";
pub const DONE: &str = "status::done";
pub const WONT_DO: &str = "status::wont-do";

pub struct GitlabAdapter {
    http: Http,
    /// URL-encoded path (`group%2Fname`) or numeric id, as request paths take it.
    project: String,
}

#[derive(Deserialize)]
struct Issue {
    /// The per-project number people see; the global `id` is not needed by REST calls.
    iid: u64,
    project_id: u64,
    web_url: String,
    title: String,
    #[serde(default)]
    description: Option<String>,
    /// `opened` or `closed`.
    state: String,
    #[serde(default)]
    labels: Vec<String>,
    created_at: String,
    updated_at: String,
}

impl Issue {
    fn task_id(&self) -> Option<TaskId> {
        self.labels.iter().find_map(|l| label_id(l))
    }

    fn has(&self, label: &str) -> bool {
        self.labels.iter().any(|l| l.eq_ignore_ascii_case(label))
    }

    fn into_task(self) -> Option<Task> {
        let id = self.task_id()?;
        let status = match self.state.as_str() {
            "closed" if self.has(WONT_DO) => Status::Closed,
            "closed" => Status::Done,
            _ if self.has(IN_PROGRESS) => Status::InProgress,
            _ => Status::Open,
        };
        let meta = meta::read_meta(&self.labels);
        Some(Task {
            title: task_title(&id, &self.title),
            description: self.description.unwrap_or_default().trim().to_string(),
            status,
            parent: id.parent(),
            created_at: secs(&self.created_at),
            updated_at: secs(&self.updated_at),
            external: Some(ExternalRef {
                adapter: "gitlab".into(),
                number: self.iid,
                url: self.web_url,
                node_id: None,
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

/// The checked `[tasks.gitlab] url`. Plain https only: every request carries the token.
pub fn instance(url: &str) -> Result<url::Url> {
    let u = url::Url::parse(url.trim())
        .with_context(|| format!("gitlab tasks: [tasks.gitlab] url {url:?} is not a URL"))?;
    if u.scheme() != "https"
        || u.host_str().is_none()
        || u.query().is_some()
        || u.fragment().is_some()
        || !u.username().is_empty()
    {
        bail!("gitlab tasks: [tasks.gitlab] url {url:?} must be a plain https:// base URL");
    }
    Ok(u)
}

/// The REST v4 root of `instance`, which may sit below a path (`https://host/gitlab`).
pub fn api_base(instance: &url::Url) -> String {
    format!("{}/api/v4", instance.as_str().trim_end_matches('/'))
}

/// `[tasks.gitlab] project`, else the origin's path when `key`
/// ([`crate::project::project_key`]) is on `instance`'s host; URL-encoded for request paths.
/// Checked, since it goes into every request path.
pub fn project(configured: &str, instance: &url::Url, key: &str) -> Result<String> {
    let path = match configured.trim().trim_matches('/') {
        "" => origin_path(instance, key).with_context(|| {
            format!(
                "gitlab tasks: origin {key} is not on {}; set [tasks.gitlab] project",
                instance.host_str().unwrap_or_default()
            )
        })?,
        p => p,
    };
    if !path.is_empty() && path.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(path.to_string());
    }
    let ok = |s: &str| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 2 || !parts.iter().all(|p| ok(p)) {
        bail!("gitlab tasks: project {path:?} is not group/name or a numeric id");
    }
    Ok(parts.join("%2F"))
}

/// The project path of an origin key on `instance`'s host. An https remote of an instance
/// under a path repeats that path; an ssh remote does not.
fn origin_path<'a>(instance: &url::Url, key: &'a str) -> Option<&'a str> {
    let (host, path) = key.split_once('/')?;
    // The key keeps an ssh or https port after the host; glab and the API ignore it.
    let name = host.split(':').next()?;
    if !name.eq_ignore_ascii_case(instance.host_str()?) {
        return None;
    }
    let below = instance.path().trim_matches('/').to_ascii_lowercase();
    if below.is_empty() {
        return Some(path);
    }
    Some(
        path.strip_prefix(below.as_str())
            .and_then(|p| p.strip_prefix('/'))
            .unwrap_or(path),
    )
}

impl GitlabAdapter {
    /// `base` is [`api_base`] (a mock server in tests), `project` what [`project`] returns;
    /// `gap` paces writes ([`super::remote::WRITE_GAP`]).
    pub fn new(base: &str, project: &str, token: &str, gap: Duration) -> Result<Self> {
        let mut headers = HeaderMap::new();
        // Bearer takes personal, project and group tokens as well as glab's OAuth token.
        let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))
            .context("gitlab tasks: the token is not a valid header value")?;
        auth.set_sensitive(true);
        headers.insert(AUTHORIZATION, auth);
        Ok(Self {
            http: Http::new("gitlab", base, headers, gap)?,
            project: project.to_string(),
        })
    }

    fn issues_path(&self) -> String {
        format!("/projects/{}/issues", self.project)
    }

    /// rtok's issues with `label`; `state` is `opened` or `all`.
    fn issues(&self, label: &str, state: &str) -> Result<Vec<Issue>> {
        let query = [("labels", label), ("state", state), ("per_page", "100")];
        self.http.get_all(&self.issues_path(), &query)
    }

    /// The issue labelled `id`, open or closed: a label lookup, which sees writes at once.
    fn find(&self, id: &TaskId) -> Result<Option<Issue>> {
        Ok(self
            .issues(&id_label(id), "all")?
            .into_iter()
            .find(|i| i.task_id().as_ref() == Some(id)))
    }

    fn shown(&self) -> String {
        self.project.replace("%2F", "/")
    }
}

fn is_status(label: &str) -> bool {
    label.eq_ignore_ascii_case(IN_PROGRESS)
        || label.eq_ignore_ascii_case(DONE)
        || label.eq_ignore_ascii_case(WONT_DO)
}

impl TaskAdapter for GitlabAdapter {
    fn name(&self) -> &'static str {
        "gitlab"
    }

    fn create(&self, task: &NewTask, id: &TaskId) -> Result<Task> {
        if task.title.trim().is_empty() {
            bail!("gitlab tasks: a task needs a title");
        }
        // The counter is per machine: another machine may already have issued this id.
        if self.find(id)?.is_some() {
            return Err(Taken(id.clone()).into());
        }
        let parent = match id.parent() {
            Some(p) => Some(self.find(&p)?.with_context(|| {
                format!("gitlab tasks: no parent task {p} in {}", self.shown())
            })?),
            None => None,
        };
        let label = id_label(id);
        let body = json!({
            "title": issue_title(id, &task.title),
            "description": task.description,
            "labels": format!("{LABEL},{label}"),
        });
        let issue: Issue = self.http.send(Method::POST, &self.issues_path(), &body)?;
        // GitLab ignores labels from a Guest, and an issue without its label is invisible
        // to every later call.
        if issue.task_id().as_ref() != Some(id) {
            bail!(
                "gitlab tasks: {} was created without the {label} label (labels need more than the Guest role in {}); label or close it by hand",
                issue.web_url,
                self.shown()
            );
        }
        if let Some(p) = parent {
            let path = format!("{}/{}/links", self.issues_path(), p.iid);
            let link = json!({
                "target_project_id": issue.project_id,
                "target_issue_iid": issue.iid,
                "link_type": "relates_to",
            });
            // The id already names the parent for rtok; the link only helps people browsing
            // GitLab, so a failed link must not report the stored task as not created.
            if let Err(e) = self.http.send::<Value>(Method::POST, &path, &link) {
                log::warn!("gitlab tasks: {id} is not linked to {}: {e:#}", p.web_url);
            }
        }
        issue
            .into_task()
            .context("gitlab tasks: the new issue has no task id")
    }

    fn list(&self, filter: &Filter) -> Result<Vec<Task>> {
        let state = if filter.wants_finished() {
            "all"
        } else {
            "opened"
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
            .with_context(|| format!("gitlab tasks: no task {id} in {}", self.shown()))?;
        let target = match status {
            Status::Open => None,
            Status::InProgress => Some(IN_PROGRESS),
            Status::Done => Some(DONE),
            Status::Closed => Some(WONT_DO),
        };
        // Removing every other status label is what scoped labels do on Premium; on Free
        // they are plain labels and would pile up.
        let remove: Vec<&str> = [IN_PROGRESS, DONE, WONT_DO]
            .into_iter()
            .filter(|l| Some(*l) != target)
            .collect();
        let mut body = json!({ "remove_labels": remove.join(",") });
        if let Some(t) = target {
            body["add_labels"] = json!(t);
        }
        match (status.is_active(), issue.state == "closed") {
            (true, true) => body["state_event"] = json!("reopen"),
            (false, false) => body["state_event"] = json!("close"),
            _ => {}
        }
        let path = format!("{}/{}", self.issues_path(), issue.iid);
        let updated: Issue = self.http.send(Method::PUT, &path, &body)?;
        updated
            .into_task()
            .with_context(|| format!("gitlab tasks: {id} lost its label"))
    }

    /// Assignee, blockers and priority, plus the status labels. Not compare-and-set: two
    /// callers can both PUT, and the last write wins. `claim` refuses a live foreign
    /// assignee before this runs.
    fn save(&self, task: &Task) -> Result<Task> {
        let issue = self
            .find(&task.id)?
            .with_context(|| format!("gitlab tasks: no task {} in {}", task.id, self.shown()))?;
        let status_label = match task.status {
            Status::Open => None,
            Status::InProgress => Some(IN_PROGRESS),
            Status::Done => Some(DONE),
            Status::Closed => Some(WONT_DO),
        };
        let mut desired: Vec<String> = issue
            .labels
            .iter()
            .filter(|label| !meta::is_managed(label) && !is_status(label))
            .cloned()
            .collect();
        if let Some(label) = status_label {
            desired.push(label.to_string());
        }
        desired.extend(meta::managed_for(task));
        let remove: Vec<&str> = issue
            .labels
            .iter()
            .filter(|label| !desired.iter().any(|want| want == *label))
            .map(String::as_str)
            .collect();
        let add: Vec<&str> = desired
            .iter()
            .filter(|label| !issue.labels.iter().any(|have| have == *label))
            .map(String::as_str)
            .collect();
        let mut body = json!({});
        if !remove.is_empty() {
            body["remove_labels"] = json!(remove.join(","));
        }
        if !add.is_empty() {
            body["add_labels"] = json!(add.join(","));
        }
        match (task.status.is_active(), issue.state == "closed") {
            (true, true) => body["state_event"] = json!("reopen"),
            (false, false) => body["state_event"] = json!("close"),
            _ => {}
        }
        let path = format!("{}/{}", self.issues_path(), issue.iid);
        let updated: Issue = if body.as_object().is_some_and(|o| o.is_empty()) {
            issue
        } else {
            self.http.send(Method::PUT, &path, &body)?
        };
        updated
            .into_task()
            .with_context(|| format!("gitlab tasks: {} lost its label", task.id))
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
}

#[cfg(test)]
mod tests {
    use super::super::adapter::set_status;
    use super::super::run::Project;
    use super::*;
    use crate::store::Store;
    use httpmock::MockServer;

    const ISSUES: &str = "/projects/me%2Fapp/issues";

    fn id(s: &str) -> TaskId {
        s.parse().unwrap()
    }

    /// An issue as REST v4 returns it, trimmed to the fields the adapter reads.
    fn issue(iid: u64, task: &str, state: &str, extra: &[&str]) -> Value {
        let mut labels = vec!["rtok".to_string(), format!("rtok:{task}")];
        labels.extend(extra.iter().map(|l| l.to_string()));
        json!({
            "id": iid * 100,
            "iid": iid,
            "project_id": 42,
            "web_url": format!("https://gitlab.com/me/app/-/issues/{iid}"),
            "title": format!("{task}. Task {iid}"),
            "description": "Why.\n",
            "state": state,
            "labels": labels,
            "issue_type": "issue",
            "created_at": "2026-10-07T10:00:00.000Z",
            "updated_at": "2026-10-07T11:00:00.000Z",
        })
    }

    fn adapter(server: &MockServer) -> GitlabAdapter {
        GitlabAdapter::new(&server.base_url(), "me%2Fapp", "test-token", Duration::ZERO).unwrap()
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

    fn new(title: &str) -> NewTask {
        NewTask {
            title: title.into(),
            description: String::new(),
            parent: None,
        }
    }

    #[test]
    fn create_labels_the_issue_and_links_it_to_its_parent() {
        let server = MockServer::start();
        by_label(&server, "rtok:A2.1", "all", json!([]));
        by_label(
            &server,
            "rtok:A2",
            "all",
            json!([issue(5, "A2", "opened", &[])]),
        );
        let post = server.mock(|when, then| {
            when.method("POST").path(ISSUES).json_body(json!({
                "title": "A2.1. Leaf",
                "description": "Done means tested.",
                "labels": "rtok,rtok:A2.1",
            }));
            then.status(201).json_body(issue(9, "A2.1", "opened", &[]));
        });
        let link = server.mock(|when, then| {
            when.method("POST")
                .path(format!("{ISSUES}/5/links"))
                .json_body(json!({
                    "target_project_id": 42,
                    "target_issue_iid": 9,
                    "link_type": "relates_to",
                }));
            then.status(201)
                .json_body(json!({"link_type": "relates_to"}));
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
        assert_eq!(task.description, "Why.");
        assert_eq!(task.parent, Some(id("A2")));
        assert_eq!(task.status, Status::Open);
        assert_eq!(task.created_at, 1_791_367_200);
        let x = task.external.unwrap();
        assert_eq!((x.adapter.as_str(), x.number), ("gitlab", 9));
        assert_eq!(x.url, "https://gitlab.com/me/app/-/issues/9");
    }

    #[test]
    fn a_failed_parent_link_still_returns_the_created_task() {
        let server = MockServer::start();
        by_label(&server, "rtok:A1.1", "all", json!([]));
        by_label(
            &server,
            "rtok:A1",
            "all",
            json!([issue(1, "A1", "opened", &[])]),
        );
        server.mock(|when, then| {
            when.method("POST").path(ISSUES);
            then.status(201).json_body(issue(2, "A1.1", "opened", &[]));
        });
        server.mock(|when, then| {
            when.method("POST").path(format!("{ISSUES}/1/links"));
            then.status(403)
                .json_body(json!({"message": "403 Forbidden"}));
        });
        let new = NewTask {
            parent: Some(id("A1")),
            ..new("Leaf")
        };
        let task = adapter(&server).create(&new, &id("A1.1")).unwrap();
        assert_eq!(task.external.unwrap().number, 2);
    }

    #[test]
    fn create_refuses_a_taken_id_a_missing_parent_and_a_dropped_label() {
        let server = MockServer::start();
        let gl = adapter(&server);
        by_label(
            &server,
            "rtok:A1",
            "all",
            json!([issue(1, "A1", "closed", &[DONE])]),
        );
        let err = gl.create(&new("Again"), &id("A1")).unwrap_err();
        assert!(err.downcast_ref::<Taken>().is_some(), "{err}");

        by_label(&server, "rtok:A3.1", "all", json!([]));
        by_label(&server, "rtok:A3", "all", json!([]));
        let err = gl.create(&new("Orphan"), &id("A3.1")).unwrap_err();
        assert!(
            err.to_string().contains("no parent task A3 in me/app"),
            "{err}"
        );

        assert!(gl.create(&new("  "), &id("A5")).is_err(), "no title");

        by_label(&server, "rtok:A4", "all", json!([]));
        server.mock(|when, then| {
            when.method("POST").path(ISSUES);
            let mut bare = issue(12, "A4", "opened", &[]);
            bare["labels"] = json!([]);
            then.status(201).json_body(bare);
        });
        let err = gl.create(&new("Unlabelled"), &id("A4")).unwrap_err();
        assert!(err.to_string().contains("Guest role"), "{err}");
    }

    #[test]
    fn list_reads_rtok_issues_and_tells_done_from_wont_do() {
        let server = MockServer::start();
        let mut stranger = issue(4, "A4", "opened", &[]);
        stranger["labels"] = json!(["rtok"]);
        let open = by_label(
            &server,
            "rtok",
            "opened",
            json!([
                issue(2, "A2", "opened", &[IN_PROGRESS]),
                stranger,
                issue(1, "A1", "opened", &["bug"])
            ]),
        );
        by_label(
            &server,
            "rtok",
            "all",
            json!([
                issue(1, "A1", "opened", &[]),
                issue(7, "A1.1", "closed", &[WONT_DO]),
                issue(8, "B9", "closed", &[]),
                issue(9, "B10", "closed", &[DONE])
            ]),
        );
        let gl = adapter(&server);
        let plan = gl.list(&Filter::default()).unwrap();
        let ids: Vec<String> = plan.iter().map(|t| t.id.to_string()).collect();
        assert_eq!(ids, ["A1", "A2"]);
        assert_eq!(plan[1].status, Status::InProgress);
        open.assert();
        let closed = gl
            .list(&Filter {
                statuses: vec![Status::Closed],
                ..Filter::default()
            })
            .unwrap();
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].id, id("A1.1"));
        let done = gl
            .list(&Filter {
                statuses: vec![Status::Done],
                ..Filter::default()
            })
            .unwrap();
        let ids: Vec<String> = done.iter().map(|t| t.id.to_string()).collect();
        assert_eq!(ids, ["B9", "B10"], "closed without a status label is done");
        assert_eq!(gl.max_id("a").unwrap(), Some(id("A1.1")));
        assert_eq!(gl.max_id("B").unwrap(), Some(id("B10")));
        assert_eq!(gl.max_id("Q").unwrap(), None);
    }

    #[test]
    fn status_swaps_the_labels_and_closes_or_reopens() {
        let server = MockServer::start();
        let gl = adapter(&server);
        by_label(
            &server,
            "rtok:A1",
            "all",
            json!([issue(1, "A1", "opened", &["bug"])]),
        );
        by_label(&server, "rtok", "opened", json!([]));
        by_label(
            &server,
            "rtok:A2",
            "all",
            json!([issue(2, "A2", "closed", &[DONE])]),
        );
        let started = server.mock(|when, then| {
            when.method("PUT")
                .path(format!("{ISSUES}/1"))
                .json_body(json!({
                    "add_labels": IN_PROGRESS,
                    "remove_labels": "status::done,status::wont-do",
                }));
            then.status(200)
                .json_body(issue(1, "A1", "opened", &["bug", IN_PROGRESS]));
        });
        let done = server.mock(|when, then| {
            when.method("PUT")
                .path(format!("{ISSUES}/1"))
                .json_body(json!({
                    "add_labels": DONE,
                    "remove_labels": "status::in-progress,status::wont-do",
                    "state_event": "close",
                }));
            then.status(200)
                .json_body(issue(1, "A1", "closed", &["bug", DONE]));
        });
        let reopened = server.mock(|when, then| {
            when.method("PUT")
                .path(format!("{ISSUES}/2"))
                .json_body(json!({
                    "remove_labels": "status::in-progress,status::done,status::wont-do",
                    "state_event": "reopen",
                }));
            then.status(200).json_body(issue(2, "A2", "opened", &[]));
        });
        let wont = server.mock(|when, then| {
            when.method("PUT")
                .path(format!("{ISSUES}/2"))
                .json_body(json!({
                    "add_labels": WONT_DO,
                    "remove_labels": "status::in-progress,status::done",
                }));
            then.status(200)
                .json_body(issue(2, "A2", "closed", &[WONT_DO]));
        });

        let t = set_status(&gl, &id("A1"), Status::InProgress, false).unwrap();
        assert_eq!(t.status, Status::InProgress);
        let t = set_status(&gl, &id("A1"), Status::Done, false).unwrap();
        assert_eq!(t.status, Status::Done);
        let t = set_status(&gl, &id("A2"), Status::Open, false).unwrap();
        assert_eq!(t.status, Status::Open);
        // A2 still reads as closed here (the mock does not change), so won't-do only moves
        // the label and sends no state event.
        let t = set_status(&gl, &id("A2"), Status::Closed, false).unwrap();
        assert_eq!(t.status, Status::Closed);
        for m in [started, done, reopened, wont] {
            m.assert();
        }
        by_label(&server, "rtok:A9", "all", json!([]));
        let err = gl.write_status(&id("A9"), Status::Done).unwrap_err();
        assert!(err.to_string().contains("no task A9 in me/app"), "{err}");
    }

    #[test]
    fn an_id_another_machine_took_is_skipped_on_create() {
        let server = MockServer::start();
        by_label(
            &server,
            "rtok",
            "all",
            json!([issue(1, "A1", "opened", &[])]),
        );
        // Another machine created A2 after this one's seed listed the project.
        by_label(
            &server,
            "rtok:A2",
            "all",
            json!([issue(2, "A2", "opened", &[])]),
        );
        by_label(&server, "rtok:A3", "all", json!([]));
        server.mock(|when, then| {
            when.method("POST")
                .path(ISSUES)
                .json_body_includes(r#"{"labels":"rtok,rtok:A3"}"#);
            then.status(201).json_body(issue(3, "A3", "opened", &[]));
        });
        let project = Project::with_adapter(
            "gitlab.com/me/app".into(),
            std::env::temp_dir(),
            "A".into(),
            Box::new(adapter(&server)),
        );
        let store = Store::open_in_memory().unwrap();
        let task = project.create(&store, &new("Mine")).unwrap();
        assert_eq!(task.id, id("A3"));
    }

    #[test]
    fn the_instance_must_be_https_and_the_project_comes_from_config_or_origin() {
        let gl = instance("https://gitlab.com").unwrap();
        assert_eq!(api_base(&gl), "https://gitlab.com/api/v4");
        let sub = instance(" https://Git.Example.com/gitlab/ ").unwrap();
        assert_eq!(api_base(&sub), "https://git.example.com/gitlab/api/v4");
        for bad in [
            "http://gitlab.com",
            "gitlab.com",
            "https://gitlab.com/?x=1",
            "https://me@gitlab.com",
        ] {
            assert!(instance(bad).is_err(), "{bad}");
        }

        assert_eq!(project("", &gl, "gitlab.com/me/app").unwrap(), "me%2Fapp");
        assert_eq!(
            project("", &gl, "gitlab.com/grp/sub/app").unwrap(),
            "grp%2Fsub%2Fapp"
        );
        assert_eq!(
            project("/org/x.rs/", &gl, "github.com/a/b").unwrap(),
            "org%2Fx.rs"
        );
        assert_eq!(project("1234", &gl, "").unwrap(), "1234");
        let err = project("", &gl, "github.com/me/app").unwrap_err();
        assert!(
            err.to_string().contains("set [tasks.gitlab] project"),
            "{err}"
        );
        // An https remote repeats the instance's path; ssh ones and ports do not matter.
        assert_eq!(
            project("", &sub, "git.example.com/gitlab/me/app").unwrap(),
            "me%2Fapp"
        );
        assert_eq!(
            project("", &sub, "git.example.com:2222/me/app").unwrap(),
            "me%2Fapp"
        );
        for bad in [
            "app",
            "../etc/x",
            "me/app?x=1",
            "me//app",
            "me/app#x",
            "me%2Fapp",
        ] {
            assert!(project(bad, &gl, "").is_err(), "{bad}");
        }
    }

    #[test]
    fn save_adds_the_owner_and_keeps_a_user_label() {
        let server = MockServer::start();
        let gl = adapter(&server);
        by_label(
            &server,
            "rtok:A1",
            "all",
            json!([issue(1, "A1", "opened", &["bug"])]),
        );
        let put = server.mock(|when, then| {
            when.method("PUT")
                .path(format!("{ISSUES}/1"))
                .json_body(json!({
                    "add_labels": "status::in-progress,rtok:owner:agent-1",
                }));
            then.status(200).json_body(issue(
                1,
                "A1",
                "opened",
                &["bug", IN_PROGRESS, "rtok:owner:agent-1"],
            ));
        });
        let mut task = gl.get(&id("A1")).unwrap().unwrap();
        task.assignee = Some("agent-1".into());
        task.status = Status::InProgress;
        let saved = gl.save(&task).unwrap();
        put.assert();
        assert_eq!(saved.assignee.as_deref(), Some("agent-1"));
        assert_eq!(saved.status, Status::InProgress);
    }

    #[test]
    fn save_stores_conflict_and_parallel_markers_and_reads_them_back() {
        let server = MockServer::start();
        let gl = adapter(&server);
        by_label(
            &server,
            "rtok:A1",
            "all",
            json!([issue(
                1,
                "A1",
                "opened",
                &["bug", "rtok:conflicts:A9", "rtok:parallel"]
            )]),
        );
        let put = server.mock(|when, then| {
            when.method("PUT")
                .path(format!("{ISSUES}/1"))
                .json_body(json!({
                    "remove_labels": "rtok:conflicts:A9,rtok:parallel",
                    "add_labels": "rtok:conflicts:A2,rtok:conflicts:A3",
                }));
            then.status(200).json_body(issue(
                1,
                "A1",
                "opened",
                &["bug", "rtok:conflicts:A2", "rtok:conflicts:A3"],
            ));
        });
        let mut task = gl.get(&id("A1")).unwrap().unwrap();
        assert_eq!(task.conflicts_with, vec![id("A9")]);
        assert!(task.parallel);
        task.conflicts_with = vec![id("A2"), id("A3")];
        task.parallel = false;
        let saved = gl.save(&task).unwrap();
        put.assert();
        assert_eq!(saved.conflicts_with, vec![id("A2"), id("A3")]);
        assert!(!saved.parallel);
        assert!(gl.stores_markers());
    }

    #[test]
    fn task_conflict_writes_both_issues_and_task_parallel_sets_the_label() {
        let server = MockServer::start();
        for (n, task) in [(1, "A1"), (2, "A2")] {
            by_label(
                &server,
                &format!("rtok:{task}"),
                "all",
                json!([issue(n, task, "opened", &["bug"])]),
            );
        }
        let edge = |n: u64, task: &str, peer: &str| {
            let label = format!("rtok:conflicts:{peer}");
            server.mock(|when, then| {
                when.method("PUT")
                    .path(format!("{ISSUES}/{n}"))
                    .json_body(json!({"add_labels": label}));
                then.status(200)
                    .json_body(issue(n, task, "opened", &["bug", &label]));
            })
        };
        let (first, second) = (edge(1, "A1", "A2"), edge(2, "A2", "A1"));
        let project = Project::with_adapter(
            "gitlab.com/me/app".into(),
            std::env::temp_dir(),
            "A".into(),
            Box::new(adapter(&server)),
        );
        let task = project.conflict(&id("A1"), &id("A2"), false).unwrap();
        first.assert();
        second.assert();
        assert_eq!(task.conflicts_with, vec![id("A2")]);

        let parallel = server.mock(|when, then| {
            when.method("PUT")
                .path(format!("{ISSUES}/1"))
                .json_body(json!({"add_labels": "rtok:parallel"}));
            then.status(200)
                .json_body(issue(1, "A1", "opened", &["bug", "rtok:parallel"]));
        });
        assert!(project.set_parallel(&id("A1"), true).unwrap().parallel);
        parallel.assert();
    }
}
