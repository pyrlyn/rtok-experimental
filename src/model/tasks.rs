// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The Tasks page (T441.13.7): `rtok task list --all` for the project `rtok web` or `rtok tui`
//! was started in, read through the same [`Project`] and [`filter`] the CLI and the MCP tool
//! use, so a board card is a row of that list and nothing is listed a second time (D27).
//! Read-only: no function here writes a task.

use std::path::Path;
use std::time::Duration;

use schemars::JsonSchema;
use serde::Serialize;

use super::Background;
use crate::config::{Config, layers};
use crate::tasks::run::{Project, filter, table};
use crate::tasks::{Status, Task};

/// Finished tasks shown per column. A project that has run for months holds hundreds of done
/// tasks, and the frame goes to every connected page every two seconds.
pub const FINISHED_SHOWN: usize = 50;

/// How long a snapshot reuses the last read of the disk adapter: the files are local, and an
/// agent that just changed a status should see it on the board soon.
const DISK_TTL: Duration = Duration::from_secs(5);

/// How long it reuses a GitHub or GitLab read. An open tui or web tab refreshes this whether or
/// not the Tasks page is showing, and every refresh is API requests against the host's rate limit.
const REMOTE_TTL: Duration = Duration::from_secs(120);

/// Where the Tasks page stands. `Unconfigured` is an ordinary case (the project never ran
/// `rtok task init`), not a failure, so the page shows how to start instead of an error.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TasksState {
    /// The first read has not landed yet.
    #[default]
    Reading,
    /// No `[tasks]` key in any config layer.
    Unconfigured,
    /// The adapter would not open or list; [`TasksPage::text`] says why.
    Failed,
    Ready,
}

/// The board's columns, the four statuses of [`Status`] under their `rtok task` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TaskColumn {
    Open,
    InProgress,
    Done,
    Closed,
}

impl From<Status> for TaskColumn {
    fn from(s: Status) -> Self {
        match s {
            Status::Open => Self::Open,
            Status::InProgress => Self::InProgress,
            Status::Done => Self::Done,
            Status::Closed => Self::Closed,
        }
    }
}

/// The issue a task lives in on GitHub or GitLab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct TaskLink {
    pub adapter: String,
    pub number: u64,
    /// Always `http(s)`: an issue host's reply is untrusted, and the page turns this into a link.
    pub url: String,
}

/// One task as the board draws it: the fields of `rtok task list --json` except the description,
/// which can be pages long and has no place on a card (`rtok task show` has it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct TaskCard {
    pub id: String,
    pub title: String,
    pub status: TaskColumn,
    pub parent: Option<String>,
    /// `0` is highest. Unlike the CLI's JSON, the default is carried too: a card always has one.
    pub priority: u8,
    pub assignee: Option<String>,
    /// The tasks this one waits for (`task_dep`), shown as "needs".
    pub blocked_by: Vec<String>,
    pub conflicts_with: Vec<String>,
    pub parallel: bool,
    pub external: Option<TaskLink>,
    /// Unix seconds.
    pub updated_at: i64,
}

impl From<&Task> for TaskCard {
    fn from(t: &Task) -> Self {
        let ids = |list: &[crate::tasks::TaskId]| list.iter().map(ToString::to_string).collect();
        Self {
            id: t.id.to_string(),
            title: t.title.clone(),
            status: t.status.into(),
            parent: t.parent.as_ref().map(ToString::to_string),
            priority: t.priority,
            assignee: t.assignee.clone(),
            blocked_by: ids(&t.blocked_by),
            conflicts_with: ids(&t.conflicts_with),
            parallel: t.parallel,
            external: t
                .external
                .as_ref()
                .filter(|x| x.url.starts_with("https://") || x.url.starts_with("http://"))
                .map(|x| TaskLink {
                    adapter: x.adapter.clone(),
                    number: x.number,
                    url: x.url.clone(),
                }),
            updated_at: t.updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct TaskBoard {
    /// `disk`, `github` or `gitlab`.
    pub adapter: String,
    /// Every active task, and the newest [`FINISHED_SHOWN`] of each finished status, by id.
    pub tasks: Vec<TaskCard>,
    /// Finished tasks left off `tasks`.
    pub hidden: u32,
}

/// The Tasks page. `text` is the `rtok task list` screen for the tui and the SPA's status line;
/// `board` is the same rows as data for the SPA. Both come from one list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct TasksPage {
    pub state: TasksState,
    pub text: String,
    pub board: Option<TaskBoard>,
}

impl TasksPage {
    fn say(state: TasksState, text: impl Into<String>) -> Self {
        Self {
            state,
            text: text.into(),
            board: None,
        }
    }
}

/// The page for the process's working directory, read off the tick (see [`Background`]): a cold
/// or stale entry never blocks it, and the tick shows the last page, or "reading tasks…".
pub fn page(cfg: &Config) -> TasksPage {
    static TASKS: Background<TasksPage> = Background::new();
    let ttl = if cfg.tasks.adapter == "disk" {
        DISK_TTL
    } else {
        REMOTE_TTL
    };
    let home = cfg.home.clone();
    let fallback = cfg.clone();
    TASKS
        .get(ttl, move || {
            // Re-read the layers each time: `.rtok.toml` is edited by `rtok task init` while the
            // page is open, and the page must follow it without a restart.
            // `load_from` would write a missing `config.toml`: a read-only page creates nothing.
            let cfg = layers::load(&home, None, None).unwrap_or(fallback);
            match std::env::current_dir() {
                Ok(cwd) => read(&cfg, &cwd, configured(&cfg)),
                Err(e) => TasksPage::say(
                    TasksState::Failed,
                    format!("tasks: cannot read the current directory: {e}\n"),
                ),
            }
        })
        .unwrap_or_else(|| TasksPage::say(TasksState::Reading, "reading tasks…\n"))
}

/// Whether any config layer above the built-in defaults sets a `[tasks]` key. The defaults alone
/// would open the disk adapter in a project that never asked for tasks.
fn configured(cfg: &Config) -> bool {
    let fig = layers::figment(&cfg.home, None, None);
    layers::sourced(&fig)
        .iter()
        .any(|(key, _, source)| key.starts_with("tasks.") && source != "default")
}

/// The list `rtok task list --all` prints for the project at `cwd`, as a page.
pub fn read(cfg: &Config, cwd: &Path, configured: bool) -> TasksPage {
    if !configured {
        return TasksPage::say(
            TasksState::Unconfigured,
            "no [tasks] section: run `rtok task init` in the project\n",
        );
    }
    let listed = Project::open(&cfg.tasks, cwd).and_then(|project| {
        project
            .adapter()
            .list(&filter(&[], true, None)?)
            .map(|tasks| (project.adapter().name(), tasks))
    });
    match listed {
        Ok((adapter, tasks)) => ready(adapter, tasks),
        Err(e) => TasksPage::say(TasksState::Failed, format!("tasks did not answer: {e:#}\n")),
    }
}

fn ready(adapter: &str, mut tasks: Vec<Task>) -> TasksPage {
    let total = tasks.len();
    // Newest first, so the cap keeps what moved last.
    tasks.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
    let mut finished = [0_usize; 2];
    tasks.retain(|t| {
        let slot = match t.status {
            Status::Done => 0,
            Status::Closed => 1,
            Status::Open | Status::InProgress => return true,
        };
        finished[slot] += 1;
        finished[slot] <= FINISHED_SHOWN
    });
    tasks.sort_by(|a, b| a.id.cmp(&b.id));
    let hidden = u32::try_from(total - tasks.len()).unwrap_or(u32::MAX);

    let mut text = table(&tasks);
    if text.is_empty() {
        text = "no tasks yet: rtok task create \"<title>\"\n".into();
    }
    if hidden > 0 {
        text.push_str(&format!("{hidden} older done or closed tasks not shown\n"));
    }
    TasksPage {
        state: TasksState::Ready,
        text,
        board: Some(TaskBoard {
            adapter: adapter.into(),
            tasks: tasks.iter().map(TaskCard::from).collect(),
            hidden,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::{ExternalRef, NewTask};

    fn checkout(name: &str) -> std::path::PathBuf {
        let dir = crate::testutil::tmp_dir(name);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(
            dir.join(".git/config"),
            "[remote \"origin\"]\n\turl = git@github.com:me/airtalk.git\n",
        )
        .unwrap();
        dir
    }

    fn task(id: &str, status: Status, updated_at: i64) -> Task {
        Task {
            status,
            updated_at,
            ..Task::open(id.parse().unwrap(), format!("title {id}"), "a long body", 0)
        }
    }

    /// The Check of T441.13.7: the page lists a fixture project's tasks by column, with the
    /// markers, and agrees with what the CLI's list returns.
    #[test]
    fn the_board_lists_a_fixture_project_the_way_task_list_does() {
        let cfg = crate::testutil::config_in(&crate::testutil::tmp_dir("tasks-page-home"));
        let dir = checkout("tasks-page-board");
        let project = Project::open(&cfg.tasks, &dir).unwrap();
        let disk = project.adapter();
        for (id, parent) in [
            ("A1", None),
            ("A1.1", Some("A1")),
            ("A2", None),
            ("A3", None),
        ] {
            let new = NewTask {
                title: format!("title {id}"),
                description: "a long body".into(),
                parent: parent.map(|p| p.parse().unwrap()),
            };
            disk.create(&new, &id.parse().unwrap()).unwrap();
        }
        let get = |id: &str| disk.get(&id.parse().unwrap()).unwrap().unwrap();
        disk.save(&Task {
            status: Status::InProgress,
            assignee: Some("agent-1".into()),
            ..get("A1")
        })
        .unwrap();
        disk.save(&Task {
            blocked_by: vec!["A2".parse().unwrap()],
            conflicts_with: vec!["A3".parse().unwrap()],
            parallel: true,
            priority: 0,
            ..get("A1.1")
        })
        .unwrap();
        disk.save(&Task {
            status: Status::Done,
            ..get("A2")
        })
        .unwrap();

        let page = read(&cfg, &dir, true);
        assert_eq!(page.state, TasksState::Ready, "{}", page.text);
        let board = page.board.as_ref().unwrap();
        assert_eq!(board.adapter, "disk");
        let cli = disk.list(&filter(&[], true, None).unwrap()).unwrap();
        assert_eq!(
            board.tasks.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
            cli.iter().map(|t| t.id.to_string()).collect::<Vec<_>>(),
            "the same tasks in the same order"
        );
        assert_eq!(page.text, table(&cli));
        let card = |id: &str| board.tasks.iter().find(|c| c.id == id).unwrap();
        assert_eq!(card("A1").status, TaskColumn::InProgress);
        assert_eq!(card("A1").assignee.as_deref(), Some("agent-1"));
        assert_eq!(card("A2").status, TaskColumn::Done);
        assert_eq!(card("A3").status, TaskColumn::Open);
        let c = card("A1.1");
        assert_eq!(c.parent.as_deref(), Some("A1"));
        assert_eq!(c.blocked_by, ["A2"]);
        assert_eq!(c.conflicts_with, ["A3"]);
        assert!(c.parallel && c.priority == 0);
        let wire = serde_json::to_value(&page).unwrap();
        assert_eq!(wire["state"], "ready");
        assert!(
            !wire.to_string().contains("a long body"),
            "a card carries no description"
        );
        assert!(
            wire["board"]["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["status"] == "in-progress"),
        );
    }

    /// The Done-when of T441.13.7: no `[tasks]` section is an empty state, and nothing is read.
    #[test]
    fn a_project_without_tasks_gets_the_empty_state() {
        let cfg = crate::testutil::config_in(&crate::testutil::tmp_dir("tasks-page-none"));
        let dir = checkout("tasks-page-none-repo");
        let page = read(&cfg, &dir, false);
        assert_eq!(page.state, TasksState::Unconfigured);
        assert!(page.board.is_none());
        assert!(page.text.contains("rtok task init"), "{}", page.text);
        assert!(!dir.join("tasks").exists(), "reading creates nothing");
    }

    #[test]
    fn a_layer_that_sets_tasks_makes_the_project_configured() {
        let home = crate::testutil::tmp_dir("tasks-page-layer");
        let mut cfg = crate::testutil::config_in(&home);
        // `config_in` leaves `home` empty; the layers are read from there.
        cfg.home = home.clone();
        assert!(!configured(&cfg));
        std::fs::write(home.join("config.toml"), "[tasks]\nprefix = \"T\"\n").unwrap();
        assert!(configured(&cfg));
    }

    #[test]
    fn a_failed_adapter_is_the_pages_text_not_a_failed_snapshot() {
        let cfg = crate::testutil::config_in(&crate::testutil::tmp_dir("tasks-page-fail"));
        let outside = crate::testutil::tmp_dir("tasks-page-nogit");
        let page = read(&cfg, &outside, true);
        assert_eq!(page.state, TasksState::Failed);
        assert!(page.board.is_none());
        assert!(
            page.text.starts_with("tasks did not answer"),
            "{}",
            page.text
        );
    }

    #[test]
    fn the_cap_keeps_the_newest_finished_tasks_and_every_active_one() {
        let mut tasks: Vec<Task> = (1..=60)
            .map(|n| task(&format!("A{n}"), Status::Done, i64::from(n)))
            .collect();
        tasks.push(task("A61", Status::Open, 0));
        tasks.push(task("A62", Status::Closed, 1));
        let page = ready("disk", tasks);
        let board = page.board.unwrap();
        let done: Vec<&str> = board
            .tasks
            .iter()
            .filter(|c| c.status == TaskColumn::Done)
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(done.len(), FINISHED_SHOWN);
        assert!(
            done.contains(&"A60") && !done.contains(&"A10"),
            "newest kept"
        );
        assert!(board.tasks.iter().any(|c| c.id == "A61"), "active kept");
        assert!(
            board.tasks.iter().any(|c| c.id == "A62"),
            "closed has its own cap"
        );
        assert_eq!(board.hidden, 10);
        assert!(
            page.text.contains("10 older done or closed"),
            "{}",
            page.text
        );
    }

    #[test]
    fn only_an_http_issue_link_reaches_the_page() {
        let link = |url: &str| {
            let mut t = task("A1", Status::Open, 0);
            t.external = Some(ExternalRef {
                adapter: "github".into(),
                number: 1,
                url: url.into(),
                node_id: None,
            });
            TaskCard::from(&t).external
        };
        assert_eq!(
            link("https://github.com/me/airtalk/issues/1").map(|l| l.number),
            Some(1)
        );
        assert_eq!(link("javascript:alert(1)"), None);
        assert_eq!(link("data:text/html,x"), None);
    }

    #[test]
    fn an_empty_project_says_how_to_add_a_task() {
        let page = ready("disk", Vec::new());
        assert_eq!(page.state, TasksState::Ready);
        assert!(page.text.contains("rtok task create"), "{}", page.text);
    }
}
