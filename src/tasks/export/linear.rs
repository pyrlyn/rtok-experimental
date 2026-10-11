// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The Linear exporter (T441.13.6). One GraphQL endpoint, a personal API key, and the same
//! label vocabulary as the GitHub adapter (`rtok`, `rtok:<id>`, owner, needs, priority and the
//! markers), so a task reads back from a Linear issue the way it does from a GitHub one. The
//! status is Linear's workflow state, picked by its type (`research.md` §35.5).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::Method;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{Action, Exporter, Kind};
use crate::tasks::meta;
use crate::tasks::remote::{Http, LABEL, id_label, issue_title, label_id, task_title};
use crate::tasks::{ExternalRef, Status, Task, TaskId};

pub const API: &str = "https://api.linear.app/graphql";

/// Issues per page. Linear caps one query at 10,000 complexity points and a connection costs
/// its page size times its children, so 50 issues with 50 labels each stay near 3,000.
const PAGE: u32 = 50;

const TEAM: &str = "query Team($key: String!) { teams(filter: {key: {eq: $key}}) { nodes { id states(first: 100) { nodes { id type position } } } } }";
const LABELS: &str = "query Labels($team: ID!, $after: String) { issueLabels(first: 100, after: $after, filter: {or: [{team: {id: {eq: $team}}}, {team: {null: true}}]}) { nodes { id name } pageInfo { hasNextPage endCursor } } }";
// Archived issues stay in the listing, or a re-run would create their task a second time.
const ISSUES: &str = "query Issues($key: String!, $label: String!, $after: String, $first: Int!) { issues(first: $first, after: $after, includeArchived: true, filter: {team: {key: {eq: $key}}, labels: {name: {eq: $label}}}) { nodes { id number url title description state { type } labels(first: 50) { nodes { name } } } pageInfo { hasNextPage endCursor } } }";
const LABEL_CREATE: &str = "mutation LabelCreate($input: IssueLabelCreateInput!) { issueLabelCreate(input: $input) { success issueLabel { id name } } }";
const ISSUE_CREATE: &str = "mutation IssueCreate($input: IssueCreateInput!) { issueCreate(input: $input) { success issue { id number url } } }";
const ISSUE_UPDATE: &str = "mutation IssueUpdate($id: String!, $input: IssueUpdateInput!) { issueUpdate(id: $id, input: $input) { success issue { id number url } } }";

#[derive(Deserialize)]
struct Reply<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GraphqlError>,
}

#[derive(Deserialize)]
struct GraphqlError {
    message: String,
}

#[derive(Deserialize)]
struct Page<T> {
    nodes: Vec<T>,
    #[serde(default, rename = "pageInfo")]
    page_info: PageInfo,
}

#[derive(Deserialize, Default)]
struct PageInfo {
    #[serde(default, rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(default, rename = "endCursor")]
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
}

#[derive(Deserialize)]
struct TeamData {
    teams: Nodes<TeamNode>,
}

#[derive(Deserialize)]
struct TeamNode {
    id: String,
    states: Nodes<StateNode>,
}

#[derive(Deserialize)]
struct StateNode {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    position: f64,
}

#[derive(Deserialize)]
struct LabelsData {
    #[serde(rename = "issueLabels")]
    labels: Page<LabelNode>,
}

#[derive(Deserialize)]
struct LabelNode {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct LabelPayload {
    #[serde(rename = "issueLabelCreate")]
    create: LabelCreated,
}

#[derive(Deserialize)]
struct LabelCreated {
    success: bool,
    #[serde(rename = "issueLabel")]
    label: Option<LabelNode>,
}

#[derive(Deserialize)]
struct IssuesData {
    issues: Page<IssueNode>,
}

#[derive(Deserialize)]
struct IssueNode {
    id: String,
    number: f64,
    url: String,
    title: String,
    description: Option<String>,
    state: StateType,
    labels: Nodes<LabelName>,
}

#[derive(Deserialize)]
struct StateType {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct LabelName {
    name: String,
}

#[derive(Deserialize)]
struct IssuePayload {
    #[serde(alias = "issueCreate", alias = "issueUpdate")]
    payload: Written,
}

#[derive(Deserialize)]
struct Written {
    success: bool,
    issue: Option<Written0>,
}

#[derive(Deserialize)]
struct Written0 {
    id: String,
    url: String,
}

/// What the exporter must remember of an issue to write to it later.
#[derive(Clone)]
struct Held {
    uuid: String,
    labels: Vec<String>,
}

struct Team {
    id: String,
    states: Vec<StateNode>,
}

pub struct LinearExporter {
    http: Http,
    url: String,
    key: String,
    team: RefCell<Option<Team>>,
    /// Label name to id, for the team's labels and the workspace's; filled on the first write.
    labels: RefCell<Option<BTreeMap<String, String>>>,
    issues: RefCell<BTreeMap<TaskId, Held>>,
}

impl LinearExporter {
    /// `url` is [`API`] (a mock server in tests); `key` is a personal API key and `team` the
    /// team's short key.
    pub fn new(url: &str, team: &str, key: &str, gap: Duration) -> Result<Self> {
        // Linear takes a personal key bare; `Bearer` is for OAuth tokens only.
        let mut auth = HeaderValue::from_str(key)
            .context("linear: the API key is not a valid header value")?;
        auth.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, auth);
        Ok(Self {
            http: Http::new("linear", url, headers, gap)?,
            url: url.to_string(),
            key: team.to_string(),
            team: RefCell::new(None),
            labels: RefCell::new(None),
            issues: RefCell::new(BTreeMap::new()),
        })
    }

    /// One query or mutation. A GraphQL failure can arrive as HTTP 200, so `errors` is read
    /// before `data` is trusted.
    fn gql<T: DeserializeOwned>(&self, query: &str, variables: Value) -> Result<T> {
        let body = json!({"query": query, "variables": variables});
        let reply: Reply<T> = self.http.send(Method::POST, "", &body)?;
        if !reply.errors.is_empty() {
            let messages: Vec<&str> = reply.errors.iter().map(|e| e.message.as_str()).collect();
            bail!("linear: POST {}: {}", self.url, messages.join("; "));
        }
        reply
            .data
            .with_context(|| format!("linear: POST {}: the answer has no data", self.url))
    }

    fn team_id_and_state(&self, status: Status) -> Result<(String, String)> {
        if self.team.borrow().is_none() {
            let data: TeamData = self.gql(TEAM, json!({"key": self.key}))?;
            let node = data
                .teams
                .nodes
                .into_iter()
                .next()
                .with_context(|| format!("linear: no team with key {:?}", self.key))?;
            *self.team.borrow_mut() = Some(Team {
                id: node.id,
                states: node.states.nodes,
            });
        }
        let team = self.team.borrow();
        let team = team.as_ref().expect("filled above");
        // Types in order of preference: a team that has no `unstarted` state keeps new work
        // in `backlog`.
        let wanted: &[&str] = match status {
            Status::Open => &["unstarted", "backlog"],
            Status::InProgress => &["started"],
            Status::Done => &["completed"],
            Status::Closed => &["canceled"],
        };
        let state = wanted.iter().find_map(|kind| {
            team.states
                .iter()
                .filter(|s| s.kind == *kind)
                .min_by(|a, b| a.position.total_cmp(&b.position))
        });
        match state {
            Some(s) => Ok((team.id.clone(), s.id.clone())),
            None => bail!(
                "linear: team {} has no workflow state of type {}",
                self.key,
                wanted.join(" or ")
            ),
        }
    }

    /// The ids of labels `names`, creating the ones the team does not have yet. A label must
    /// exist before `labelIds` can name it.
    fn label_ids(&self, names: &[String], team: &str) -> Result<Vec<String>> {
        if self.labels.borrow().is_none() {
            let mut known = BTreeMap::new();
            let mut after = Value::Null;
            loop {
                let data: LabelsData = self.gql(LABELS, json!({"team": team, "after": after}))?;
                known.extend(data.labels.nodes.into_iter().map(|l| (l.name, l.id)));
                match data.labels.page_info {
                    PageInfo {
                        has_next_page: true,
                        end_cursor: Some(c),
                    } => after = json!(c),
                    _ => break,
                }
            }
            *self.labels.borrow_mut() = Some(known);
        }
        let mut ids = Vec::new();
        for name in names {
            let known = self
                .labels
                .borrow()
                .as_ref()
                .and_then(|m| m.get(name).cloned());
            let id = match known {
                Some(id) => id,
                None => {
                    let made: LabelPayload = self.gql(
                        LABEL_CREATE,
                        json!({"input": {"name": name, "teamId": team}}),
                    )?;
                    let label = made
                        .create
                        .label
                        .filter(|_| made.create.success)
                        .with_context(|| {
                            format!("linear: POST {}: label {name:?} was not created", self.url)
                        })?;
                    self.labels
                        .borrow_mut()
                        .as_mut()
                        .expect("filled above")
                        .insert(name.clone(), label.id.clone());
                    label.id
                }
            };
            ids.push(id);
        }
        Ok(ids)
    }

    fn written(&self, query: &str, variables: Value) -> Result<Written0> {
        let data: IssuePayload = self.gql(query, variables)?;
        data.payload
            .issue
            .filter(|_| data.payload.success)
            .with_context(|| format!("linear: POST {}: the write was not accepted", self.url))
    }
}

fn status_of(state_type: &str) -> Status {
    match state_type {
        "started" => Status::InProgress,
        "completed" => Status::Done,
        "canceled" | "duplicate" => Status::Closed,
        _ => Status::Open,
    }
}

impl Exporter for LinearExporter {
    fn name(&self) -> &'static str {
        "linear"
    }

    fn target(&self) -> String {
        self.key.clone()
    }

    fn remote(&self) -> Result<Vec<Task>> {
        let mut tasks = Vec::new();
        let mut held = BTreeMap::new();
        let mut after = Value::Null;
        loop {
            let data: IssuesData = self.gql(
                ISSUES,
                json!({"key": self.key, "label": LABEL, "after": after, "first": PAGE}),
            )?;
            for issue in data.issues.nodes {
                let names: Vec<String> = issue.labels.nodes.into_iter().map(|l| l.name).collect();
                // An issue carrying `rtok` but no `rtok:<id>` is not one of ours to touch.
                let Some(id) = names.iter().find_map(|n| label_id(n)) else {
                    continue;
                };
                let meta = meta::read_meta(&names);
                let task = Task {
                    status: status_of(&issue.state.kind),
                    priority: meta.priority,
                    assignee: meta.assignee,
                    blocked_by: meta.blocked_by,
                    conflicts_with: meta.conflicts_with,
                    parallel: meta.parallel,
                    external: Some(ExternalRef {
                        adapter: "linear".into(),
                        number: issue.number as u64,
                        url: issue.url,
                        node_id: Some(issue.id.clone()),
                    }),
                    ..Task::open(
                        id.clone(),
                        task_title(&id, &issue.title),
                        issue.description.unwrap_or_default(),
                        0,
                    )
                };
                held.entry(id).or_insert(Held {
                    uuid: issue.id,
                    labels: names,
                });
                tasks.push(task);
            }
            match data.issues.page_info {
                PageInfo {
                    has_next_page: true,
                    end_cursor: Some(c),
                } => after = json!(c),
                _ => break,
            }
        }
        *self.issues.borrow_mut() = held;
        Ok(tasks)
    }

    fn apply(&self, action: &Action, task: &Task) -> Result<Option<String>> {
        let (team, state) = self.team_id_and_state(task.status)?;
        let existing = match action.kind {
            Kind::Create => vec![LABEL.to_string(), id_label(&task.id)],
            _ => self
                .issues
                .borrow()
                .get(&task.id)
                .with_context(|| format!("linear: no issue for task {}", task.id))?
                .labels
                .clone(),
        };
        // `merge_labels` keeps a person's own labels and rewrites the ones rtok owns.
        let labels = meta::merge_labels(&existing, task);
        let label_ids = self.label_ids(&labels, &team)?;
        let mut input = json!({
            "title": issue_title(&task.id, &task.title),
            "description": task.description,
            "labelIds": label_ids,
        });
        let written = if action.kind == Kind::Create {
            input["teamId"] = json!(team);
            input["stateId"] = json!(state);
            if let Some(parent) = task.id.parent() {
                let held = self.issues.borrow();
                let parent = held.get(&parent).with_context(|| {
                    format!("linear: the parent {parent} of {} has no issue", task.id)
                })?;
                input["parentId"] = json!(parent.uuid);
            }
            self.written(ISSUE_CREATE, json!({"input": input}))?
        } else {
            let uuid = self.issues.borrow()[&task.id].uuid.clone();
            // The state moves only when the status differs: an issue a person parked in
            // another `unstarted` column stays there.
            if action.fields.contains(&"status") {
                input["stateId"] = json!(state);
            }
            self.written(ISSUE_UPDATE, json!({"id": uuid, "input": input}))?
        };
        self.issues.borrow_mut().insert(
            task.id.clone(),
            Held {
                uuid: written.id,
                labels,
            },
        );
        Ok(Some(written.url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::export::{self, Report};
    use httpmock::MockServer;

    fn id(s: &str) -> TaskId {
        s.parse().unwrap()
    }

    fn exporter(server: &MockServer) -> LinearExporter {
        LinearExporter::new(
            &server.url("/graphql"),
            "ENG",
            "lin_api_test",
            Duration::ZERO,
        )
        .unwrap()
    }

    fn local(task: &str, title: &str, status: Status) -> Task {
        Task {
            status,
            ..Task::open(id(task), title, "Why.", 0)
        }
    }

    fn summary(r: &Report) -> Vec<String> {
        r.actions
            .iter()
            .map(|a| format!("{:?} {}", a.kind, a.id))
            .collect()
    }

    /// A recorded answer: `{"data": ...}`.
    fn data(v: Value) -> Value {
        json!({"data": v})
    }

    /// An issue as the `Issues` query returns it.
    fn issue(n: u32, task: &str, title: &str, state: &str, extra: &[&str]) -> Value {
        let mut labels = vec![
            json!({"name": "rtok"}),
            json!({"name": format!("rtok:{task}")}),
        ];
        labels.extend(extra.iter().map(|l| json!({"name": l})));
        json!({
            "id": format!("uuid-{n}"),
            "number": n,
            "url": format!("https://linear.app/acme/issue/ENG-{n}"),
            "title": format!("{task}. {title}"),
            "description": "Why.",
            "state": {"type": state},
            "labels": {"nodes": labels},
        })
    }

    fn issues(nodes: Vec<Value>) -> Value {
        data(
            json!({"issues": {"nodes": nodes, "pageInfo": {"hasNextPage": false, "endCursor": null}}}),
        )
    }

    /// Answers the operation named `op`; `check` tests the request's variables.
    fn on<'a>(
        server: &'a MockServer,
        op: &'static str,
        check: impl Fn(&Value) -> bool + Send + Sync + 'static,
        answer: Value,
    ) -> httpmock::Mock<'a> {
        server.mock(|when, then| {
            when.method("POST")
                .path("/graphql")
                .header("authorization", "lin_api_test")
                .is_true(move |req| {
                    let body: Value = serde_json::from_slice(req.body_ref()).unwrap_or_default();
                    body["query"].as_str().is_some_and(|q| q.contains(op))
                        && check(&body["variables"])
                });
            then.status(200).json_body(answer);
        })
    }

    fn any(_: &Value) -> bool {
        true
    }

    /// The team lookup and the label list every write run starts with.
    fn setup(server: &MockServer) {
        on(
            server,
            "query Team",
            any,
            data(
                json!({"teams": {"nodes": [{"id": "team-1", "states": {"nodes": [
                    {"id": "s-backlog", "type": "backlog", "position": 0.0},
                    {"id": "s-todo", "type": "unstarted", "position": 1.0},
                    {"id": "s-todo2", "type": "unstarted", "position": 2.0},
                    {"id": "s-doing", "type": "started", "position": 3.0},
                    {"id": "s-done", "type": "completed", "position": 4.0},
                    {"id": "s-wont", "type": "canceled", "position": 5.0},
                ]}}]}}),
            ),
        );
        on(
            server,
            "query Labels",
            any,
            data(json!({"issueLabels": {
                "nodes": [{"id": "l-rtok", "name": "rtok"}, {"id": "l-bug", "name": "bug"}],
                "pageInfo": {"hasNextPage": false, "endCursor": null},
            }})),
        );
    }

    fn written(op: &str, n: u32) -> Value {
        data(json!({op: {"success": true, "issue": {
            "id": format!("uuid-{n}"),
            "number": n,
            "url": format!("https://linear.app/acme/issue/ENG-{n}"),
        }}}))
    }

    #[test]
    fn a_new_task_becomes_an_issue_and_a_second_run_sends_nothing() {
        let server = MockServer::start();
        setup(&server);
        let mut empty = on(&server, "query Issues", any, issues(vec![]));
        let label = on(
            &server,
            "mutation LabelCreate",
            |v| v["input"] == json!({"name": "rtok:R1", "teamId": "team-1"}),
            data(
                json!({"issueLabelCreate": {"success": true, "issueLabel": {"id": "l-r1", "name": "rtok:R1"}}}),
            ),
        );
        let create = on(
            &server,
            "mutation IssueCreate",
            |v| {
                v["input"]
                    == json!({
                        "title": "R1. Ship",
                        "description": "Why.",
                        "labelIds": ["l-rtok", "l-r1"],
                        "teamId": "team-1",
                        "stateId": "s-todo",
                    })
            },
            written("issueCreate", 7),
        );
        let tasks = [local("R1", "Ship", Status::Open)];
        let first = export::run(&exporter(&server), &tasks, false).unwrap();
        assert_eq!(summary(&first), ["Create R1"]);
        assert_eq!(
            first.actions[0].url.as_deref(),
            Some("https://linear.app/acme/issue/ENG-7")
        );
        label.assert();
        create.assert();
        empty.delete();

        on(
            &server,
            "query Issues",
            any,
            issues(vec![issue(7, "R1", "Ship", "unstarted", &[])]),
        );
        let again = export::run(&exporter(&server), &tasks, false).unwrap();
        assert!(again.actions.is_empty(), "{again:?}");
        assert_eq!(again.unchanged, 1);
        create.assert_calls(1);
    }

    #[test]
    fn drift_is_updated_and_done_and_closed_tasks_move_to_their_state_type() {
        let server = MockServer::start();
        setup(&server);
        on(
            &server,
            "query Issues",
            any,
            issues(vec![
                issue(1, "R1", "Old title", "backlog", &["bug"]),
                issue(2, "R2", "Two", "started", &[]),
                issue(3, "R3", "Three", "unstarted", &[]),
                issue(4, "R4", "Four", "completed", &[]),
            ]),
        );
        on(
            &server,
            "mutation LabelCreate",
            any,
            data(
                json!({"issueLabelCreate": {"success": true, "issueLabel": {"id": "l-x", "name": "x"}}}),
            ),
        );
        // The retitle leaves the state alone: `backlog` already reads as open.
        let retitle = on(
            &server,
            "mutation IssueUpdate",
            |v| {
                v["id"] == "uuid-1"
                    && v["input"]["title"] == "R1. New title"
                    && v["input"].get("stateId").is_none()
                    && v["input"]["labelIds"] == json!(["l-rtok", "l-x", "l-bug"])
            },
            written("issueUpdate", 1),
        );
        let done = on(
            &server,
            "mutation IssueUpdate",
            |v| v["id"] == "uuid-2" && v["input"]["stateId"] == "s-done",
            written("issueUpdate", 2),
        );
        let wont = on(
            &server,
            "mutation IssueUpdate",
            |v| v["id"] == "uuid-3" && v["input"]["stateId"] == "s-wont",
            written("issueUpdate", 3),
        );
        let reopen = on(
            &server,
            "mutation IssueUpdate",
            |v| v["id"] == "uuid-4" && v["input"]["stateId"] == "s-todo",
            written("issueUpdate", 4),
        );
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
    fn a_subtask_hangs_under_its_parent_and_a_finished_parent_is_created_done() {
        let server = MockServer::start();
        setup(&server);
        on(&server, "query Issues", any, issues(vec![]));
        on(
            &server,
            "mutation LabelCreate",
            any,
            data(
                json!({"issueLabelCreate": {"success": true, "issueLabel": {"id": "l-x", "name": "x"}}}),
            ),
        );
        let parent = on(
            &server,
            "mutation IssueCreate",
            |v| v["input"]["title"] == "R1. Big" && v["input"]["stateId"] == "s-done",
            written("issueCreate", 1),
        );
        let child = on(
            &server,
            "mutation IssueCreate",
            |v| {
                v["input"]["title"] == "R1.1. Small"
                    && v["input"]["parentId"] == "uuid-1"
                    && v["input"]["stateId"] == "s-doing"
            },
            written("issueCreate", 2),
        );
        let mut doing = local("R1.1", "Small", Status::InProgress);
        doing.parent = Some(id("R1"));
        let tasks = [doing, local("R1", "Big", Status::Done)];
        let report = export::run(&exporter(&server), &tasks, false).unwrap();
        assert_eq!(summary(&report), ["Create R1", "Create R1.1"]);
        parent.assert();
        child.assert();
    }

    #[test]
    fn dry_run_lists_the_writes_and_sends_none() {
        let server = MockServer::start();
        on(
            &server,
            "query Issues",
            any,
            issues(vec![issue(1, "R1", "Old", "unstarted", &[])]),
        );
        let write = server.mock(|when, then| {
            when.is_true(|req| {
                let body = String::from_utf8_lossy(req.body_ref()).into_owned();
                body.contains("mutation") || body.contains("query Team")
            });
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
    fn a_graphql_error_in_a_200_names_linear_and_the_url() {
        let server = MockServer::start();
        on(
            &server,
            "query Issues",
            any,
            json!({"errors": [{"message": "Argument Validation Error", "path": ["issues"]}]}),
        );
        let err = export::run(&exporter(&server), &[], false).unwrap_err();
        let text = format!("{err:#}");
        assert!(
            text.contains("linear")
                && text.contains("/graphql")
                && text.contains("Argument Validation Error")
                && !text.contains("lin_api_test"),
            "{text}"
        );
    }

    #[test]
    fn a_rate_limit_arrives_as_a_400_and_says_so() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/graphql");
            then.status(400).json_body(json!({"errors": [{
                "message": "Rate limit exceeded",
                "extensions": {"code": "RATELIMITED"},
            }]}));
        });
        let err = export::run(&exporter(&server), &[], false).unwrap_err();
        let text = format!("{err:#}");
        assert!(
            text.contains("Rate limit exceeded") && text.contains("linear"),
            "{text}"
        );
    }

    #[test]
    fn a_team_without_a_state_for_the_status_is_named() {
        let server = MockServer::start();
        on(&server, "query Issues", any, issues(vec![]));
        on(
            &server,
            "query Team",
            any,
            data(
                json!({"teams": {"nodes": [{"id": "team-1", "states": {"nodes": [
                    {"id": "s-done", "type": "completed", "position": 0.0},
                ]}}]}}),
            ),
        );
        let err = export::run(
            &exporter(&server),
            &[local("R1", "Go", Status::Open)],
            false,
        )
        .unwrap_err();
        let text = format!("{err:#}");
        assert!(
            text.contains("linear export: create R1 failed after 0 of 1 writes")
                && text.contains("no workflow state of type unstarted or backlog"),
            "{text}"
        );
    }
}
