// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! What `rtok task …` does (T441.5), shaped so the MCP `task_*` tools (T441.6) call the same
//! functions and print the same JSON: open the project's adapter, allocate ids from the store,
//! pick the next task, and write `[tasks]` into `.rtok.toml`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use super::adapter::{Filter, Taken, TaskAdapter, set_status};
use super::disk::DiskAdapter;
use super::github::{self, GithubAdapter};
use super::gitlab::{self, GitlabAdapter};
use super::ready::{self, active_blocker, find_cycles, holder_is_stale, select_ready};
use super::remote;
use super::{
    ClaimConflict, ClaimOutcome, NewTask, PRIORITY_MAX, ReadyItem, Status, Task, TaskId,
    check_prefix, resolve_prefix,
};
use crate::config::layers::git_root;
use crate::store::Store;

/// The adapters `[tasks] adapter` accepts; the config validator checks the same set.
pub use crate::config::TASK_ADAPTERS as ADAPTERS;

/// Creates tried when the id keeps turning out taken, before the last error is the answer.
const TAKEN_RETRIES: usize = 3;

/// One project's tasks: where they are stored, the counter they are numbered from and the
/// prefix new ids get.
pub struct Project {
    /// The counter key, [`crate::project::project_key`].
    pub key: String,
    /// The checkout's root: `.rtok.toml` and the disk adapter's directory live here.
    pub root: PathBuf,
    pub prefix: String,
    adapter: Box<dyn TaskAdapter>,
}

/// `rtok task show`: the task and the ids of its subtasks.
#[derive(Debug, Serialize)]
pub struct Shown {
    #[serde(flatten)]
    pub task: Task,
    pub subtasks: Vec<TaskId>,
}

impl Project {
    /// The project `cwd` belongs to, with the adapter `[tasks]` names.
    pub fn open(cfg: &crate::config::Tasks, cwd: &Path) -> Result<Self> {
        let root = git_root(cwd).context("rtok task: not inside a git checkout")?;
        let key = crate::project::project_key(cwd)
            .context("rtok task: cannot tell which project this checkout is")?;
        let name = crate::project::project_name(cwd);
        let prefix = resolve_prefix(&cfg.prefix, name.as_deref())?;
        let adapter: Box<dyn TaskAdapter> = match cfg.adapter.as_str() {
            "disk" => Box::new(DiskAdapter::new(root.join(&cfg.disk.dir))),
            "github" => {
                let repo = github::repo(&cfg.github.repo, &key)?;
                let token = remote::token(
                    "github",
                    &["GH_TOKEN", "GITHUB_TOKEN"],
                    &["gh", "auth", "token"],
                )?;
                Box::new(
                    GithubAdapter::new(github::API, &repo, &token, remote::WRITE_GAP)?
                        .with_project(cfg.github.project.into()),
                )
            }
            "gitlab" => {
                let instance = gitlab::instance(&cfg.gitlab.url)?;
                let project = gitlab::project(&cfg.gitlab.project, &instance, &key)?;
                let host = instance.host_str().unwrap_or_default();
                let token = remote::token(
                    "gitlab",
                    &["GITLAB_TOKEN", "GITLAB_ACCESS_TOKEN", "GL_TOKEN"],
                    &["glab", "config", "get", "token", "--host", host],
                )?;
                Box::new(GitlabAdapter::new(
                    &gitlab::api_base(&instance),
                    &project,
                    &token,
                    remote::WRITE_GAP,
                )?)
            }
            other => bail!("rtok task: unknown [tasks] adapter {other:?}"),
        };
        Ok(Self::with_adapter(key, root, prefix, adapter))
    }

    pub fn with_adapter(
        key: String,
        root: PathBuf,
        prefix: String,
        adapter: Box<dyn TaskAdapter>,
    ) -> Self {
        Self {
            key,
            root,
            prefix,
            adapter,
        }
    }

    pub fn adapter(&self) -> &dyn TaskAdapter {
        self.adapter.as_ref()
    }

    /// Store a new task under the next free id. The counter is first raised past every id
    /// the adapter already holds, so tasks that arrived without this machine's counter (a
    /// pull, another checkout's clone) are never numbered over.
    pub fn create(&self, store: &Store, new: &NewTask) -> Result<Task> {
        if let Some(p) = &new.parent
            && self.adapter.get(p)?.is_none()
        {
            bail!("rtok task: no parent task {p}");
        }
        let parent = new.parent.as_ref();
        self.seed(store, parent)?;
        for _ in 1..TAKEN_RETRIES {
            let id = store.allocate_task_id(&self.key, &self.prefix, parent)?;
            match self.adapter.create(new, &id) {
                // Another machine's counter got there between the seed and the create (a
                // remote adapter's own check); seeding again jumps past everything it made.
                Err(e) if e.downcast_ref::<Taken>().is_some() => self.seed(store, parent)?,
                done => return done,
            };
        }
        let id = store.allocate_task_id(&self.key, &self.prefix, parent)?;
        self.adapter.create(new, &id)
    }

    /// Raise the counter for `parent` (`None`: top level) to the highest number in use.
    /// Every prefix counts: the counter belongs to the project, not to its current prefix.
    pub fn seed(&self, store: &Store, parent: Option<&TaskId>) -> Result<u32> {
        let tasks = self.adapter.list(&Filter {
            all: true,
            parent: parent.cloned(),
            ..Filter::default()
        })?;
        let depth = parent.map_or(0, |p| p.path().len());
        let max = tasks
            .iter()
            .filter_map(|t| t.id.path().get(depth).copied())
            .max()
            .unwrap_or(0);
        store
            .seed_task_counter(&self.key, parent, max)
            .map_err(Into::into)
    }

    /// The task, or an error naming the id.
    pub fn get(&self, id: &TaskId) -> Result<Task> {
        self.adapter
            .get(id)?
            .with_context(|| format!("no task {id}"))
    }

    /// `status <id> [<status>]`: set the status when given, else read it. Finishing a task
    /// drops its claim row, so SessionStart stops naming it.
    pub fn status(
        &self,
        store: &Store,
        id: &TaskId,
        status: Option<Status>,
        force: bool,
    ) -> Result<Task> {
        match status {
            Some(s) => {
                let task = set_status(self.adapter(), id, s, force)?;
                if !s.is_active() {
                    store.clear_task_claim(&self.key, &id.to_string())?;
                }
                Ok(task)
            }
            None => self.get(id),
        }
    }

    pub fn show(&self, id: &TaskId) -> Result<Shown> {
        let task = self.get(id)?;
        let subtasks = self
            .adapter
            .list(&Filter {
                all: true,
                parent: Some(id.clone()),
                ..Filter::default()
            })?
            .into_iter()
            .map(|t| t.id)
            .collect();
        Ok(Shown { task, subtasks })
    }

    /// Tasks that can be claimed, highest priority first (`0` before `2`), then by id.
    pub fn ready(&self, store: &Store) -> Result<Vec<ReadyItem>> {
        let _guard = self.adapter.claim_guard()?;
        self.ready_unlocked(store)
    }

    fn ready_unlocked(&self, store: &Store) -> Result<Vec<ReadyItem>> {
        let tasks = self.list_all()?;
        let now = ready::unix_now();
        Ok(select_ready(&tasks, |agent| {
            match store.agent_last_seen(agent) {
                Ok(seen) => holder_is_stale(Ok(seen), now),
                Err(_) => false,
            }
        }))
    }

    /// The first ready task. With no `blocked_by` edges anywhere, that is still the lowest
    /// open task that has no active subtask.
    pub fn next(&self, store: &Store) -> Result<Option<Task>> {
        Ok(self.ready(store)?.into_iter().next().map(|item| item.task))
    }

    /// Claim `id`, or the first ready task when `id` is `None`. A task this agent already
    /// holds comes back with `changed: false` and the file or issue is not rewritten.
    pub fn claim(&self, store: &Store, id: Option<&TaskId>, agent: &str) -> Result<ClaimOutcome> {
        let _guard = self.adapter.claim_guard()?;
        match id {
            Some(id) => self.claim_held(store, id, agent),
            None => {
                let ready = self.ready_unlocked(store)?;
                let mut last = None;
                for item in ready {
                    match self.claim_held(store, &item.task.id, agent) {
                        Err(e) if e.downcast_ref::<ClaimConflict>().is_some() => last = Some(e),
                        other => return other,
                    }
                }
                match last {
                    Some(e) => Err(e),
                    None => bail!("no ready task"),
                }
            }
        }
    }

    fn claim_held(&self, store: &Store, id: &TaskId, agent: &str) -> Result<ClaimOutcome> {
        let mut task = self.get(id)?;
        if !task.status.is_active() {
            return Err(ClaimConflict::NotClaimable {
                id: id.clone(),
                status: task.status,
            }
            .into());
        }
        if task.assignee.as_deref() == Some(agent) && task.status == Status::InProgress {
            store.upsert_task_claim(&self.key, &id.to_string(), agent, &task.title)?;
            return Ok(ClaimOutcome {
                task,
                changed: false,
            });
        }
        let all = self.list_all()?;
        if let Some(by) = active_blocker(&task, &all) {
            return Err(ClaimConflict::Blocked {
                id: id.clone(),
                by: by.clone(),
            }
            .into());
        }
        if let Some(holder) = task.assignee.clone().filter(|s| !s.is_empty())
            && holder != agent
            && !self.holder_stale(store, &holder)
        {
            return Err(ClaimConflict::Already {
                id: id.clone(),
                assignee: holder,
            }
            .into());
        }
        task.assignee = Some(agent.to_string());
        task.status = Status::InProgress;
        let task = self.adapter.save(&task)?;
        store.upsert_task_claim(&self.key, &task.id.to_string(), agent, &task.title)?;
        Ok(ClaimOutcome {
            task,
            changed: true,
        })
    }

    /// Clear the assignee and set an in-progress task back to open. Only the holder, unless
    /// `force`.
    pub fn release(&self, store: &Store, id: &TaskId, agent: &str, force: bool) -> Result<Task> {
        let _guard = self.adapter.claim_guard()?;
        let mut task = self.get(id)?;
        if !task.status.is_active() {
            bail!("{id} is {} and cannot be released", task.status);
        }
        if let Some(holder) = task.assignee.as_deref().filter(|s| !s.is_empty())
            && holder != agent
            && !force
        {
            bail!("{id} is claimed by {holder}; pass --force");
        }
        let changed = task.assignee.is_some() || task.status != Status::Open;
        task.assignee = None;
        if task.status == Status::InProgress {
            task.status = Status::Open;
        }
        let task = if changed {
            self.adapter.save(&task)?
        } else {
            task
        };
        store.clear_task_claim(&self.key, &id.to_string())?;
        Ok(task)
    }

    /// `id` waits on `blocker`. A self-edge, a missing blocker or a cycle writes nothing.
    /// An edge that is already there is left as it is.
    pub fn block(&self, id: &TaskId, blocker: &TaskId) -> Result<Task> {
        if id == blocker {
            bail!("{id} cannot depend on itself");
        }
        let _guard = self.adapter.claim_guard()?;
        let mut task = self.get(id)?;
        if self.adapter.get(blocker)?.is_none() {
            bail!("no task {blocker}");
        }
        if task.blocked_by.iter().any(|have| have == blocker) {
            return Ok(task);
        }
        task.blocked_by.push(blocker.clone());
        task.blocked_by.sort();
        let mut graph = self.list_all()?;
        if let Some(slot) = graph.iter_mut().find(|t| t.id == task.id) {
            *slot = task.clone();
        } else {
            graph.push(task.clone());
        }
        if let Some(cycle) = find_cycles(&graph).into_iter().next() {
            let shown = cycle
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" -> ");
            bail!("{id} would cycle: {shown}");
        }
        self.adapter.save(&task)
    }

    /// `id` and `other` touch the same files (`remove`: they no longer do). Symmetric: both tasks
    /// are written, and an edge that is already as asked is left alone. A self-edge, a missing
    /// task or an adapter that cannot store markers writes nothing, except that `remove` clears
    /// `id`'s side of an edge whose peer is gone, which would otherwise need a hand edit.
    pub fn conflict(&self, id: &TaskId, other: &TaskId, remove: bool) -> Result<Task> {
        if id == other {
            bail!("{id} cannot conflict with itself");
        }
        self.need_markers()?;
        let _guard = self.adapter.claim_guard()?;
        let mut task = self.get(id)?;
        let mut peer = match self.adapter.get(other)? {
            Some(peer) => peer,
            None if remove => {
                let Some(at) = task.conflicts_with.iter().position(|have| have == other) else {
                    return Ok(task);
                };
                task.conflicts_with.remove(at);
                return self.adapter.save(&task);
            }
            None => bail!("no task {other}"),
        };
        let edit = |t: &mut Task, with: &TaskId| {
            let at = t.conflicts_with.iter().position(|have| have == with);
            match (remove, at) {
                (true, Some(at)) => {
                    t.conflicts_with.remove(at);
                    true
                }
                (false, None) => {
                    t.conflicts_with.push(with.clone());
                    t.conflicts_with.sort();
                    true
                }
                _ => false,
            }
        };
        let (task_changed, peer_changed) = (edit(&mut task, other), edit(&mut peer, id));
        if peer_changed {
            self.adapter.save(&peer)?;
        }
        if task_changed {
            task = self.adapter.save(&task)?;
        }
        Ok(task)
    }

    /// Mark `id` safe to run beside any other task, or (`on` false) take the mark off.
    pub fn set_parallel(&self, id: &TaskId, on: bool) -> Result<Task> {
        self.need_markers()?;
        let _guard = self.adapter.claim_guard()?;
        let mut task = self.get(id)?;
        if task.parallel == on {
            return Ok(task);
        }
        task.parallel = on;
        self.adapter.save(&task)
    }

    fn need_markers(&self) -> Result<()> {
        if !self.adapter.stores_markers() {
            bail!(
                "{} tasks cannot store conflict or parallel markers yet: use the disk adapter",
                self.adapter.name()
            );
        }
        Ok(())
    }

    /// `0` is highest, [`PRIORITY_MAX`] is lowest. The same level is not rewritten.
    pub fn set_priority(&self, id: &TaskId, level: u8) -> Result<Task> {
        if level > PRIORITY_MAX {
            bail!("priority {level} is outside 0–{PRIORITY_MAX}");
        }
        let _guard = self.adapter.claim_guard()?;
        let mut task = self.get(id)?;
        if task.priority == level {
            return Ok(task);
        }
        task.priority = level;
        self.adapter.save(&task)
    }

    fn list_all(&self) -> Result<Vec<Task>> {
        self.adapter.list(&Filter {
            all: true,
            ..Filter::default()
        })
    }

    /// A store error counts as not stale, so a live claim is not taken because the lookup
    /// failed.
    fn holder_stale(&self, store: &Store, agent: &str) -> bool {
        match store.agent_last_seen(agent) {
            Ok(seen) => holder_is_stale(Ok(seen), ready::unix_now()),
            Err(_) => false,
        }
    }
}

/// `rtok task init`: write `[tasks] adapter` and `prefix` into the checkout's `.rtok.toml`,
/// keeping every other line. Returns the file.
pub fn init(cwd: &Path, adapter: Option<&str>, prefix: Option<&str>) -> Result<PathBuf> {
    if let Some(a) = adapter
        && !ADAPTERS.contains(&a)
    {
        bail!(
            "rtok task init: --adapter must be one of {}",
            ADAPTERS.join(", ")
        );
    }
    if let Some(p) = prefix {
        check_prefix(p)?;
    }
    let root = git_root(cwd).context("rtok task init: not inside a git checkout")?;
    let path = root.join(".rtok.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| path.display().to_string()),
    };
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("{}: not valid TOML", path.display()))?;
    let tasks = doc
        .entry("tasks")
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .with_context(|| format!("{}: `tasks` is not a table", path.display()))?;
    if let Some(a) = adapter {
        tasks["adapter"] = toml_edit::value(a);
    } else if !tasks.contains_key("adapter") {
        tasks["adapter"] = toml_edit::value("disk");
    }
    if let Some(p) = prefix {
        tasks["prefix"] = toml_edit::value(p.to_ascii_uppercase());
    }
    crate::config::write_file(&path, &doc.to_string())?;
    Ok(path)
}

/// The agent a claim binds to: `--agent`, else `RTOK_AGENT_ID`. Neither means the caller
/// has to say who is claiming.
pub fn require_agent(store: &Store, flag: Option<&str>) -> Result<String> {
    match crate::worktree::claim::caller(Some(store), flag)? {
        Some(agent) => Ok(agent.id),
        None => bail!("no agent to bind: pass --agent or set RTOK_AGENT_ID"),
    }
}

/// `rtok task ready`: one line per task, `(stale)` when the assignee can be replaced.
pub fn ready_text(items: &[ReadyItem]) -> String {
    if items.is_empty() {
        return "no ready task\n".into();
    }
    let mut out = String::new();
    for item in items {
        out.push_str(&format!("{}  {}", item.task.id, item.task.title));
        if item.stale {
            out.push_str("  (stale)");
        }
        if item.task.parallel {
            out.push_str("  [parallel]");
        }
        if !item.conflicts_in_progress.is_empty() {
            out.push_str(&format!(
                "  (conflicts with {} in progress)",
                ids(&item.conflicts_in_progress)
            ));
        }
        out.push('\n');
    }
    out
}

/// `list`'s filter from what a caller passed: status names, `all`, a parent id.
pub fn filter(statuses: &[String], all: bool, parent: Option<&str>) -> Result<Filter> {
    Ok(Filter {
        statuses: statuses
            .iter()
            .map(|s| s.parse::<Status>())
            .collect::<Result<_>>()?,
        all,
        parent: parent.map(str::parse::<TaskId>).transpose()?,
    })
}

/// Comma-separated ids.
fn ids(list: &[TaskId]) -> String {
    list.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The list line's tail: `  [parallel]` and `  [conflicts: A3, A4]`, nothing for a plain task.
fn markers(t: &Task) -> String {
    let mut out = String::new();
    if t.parallel {
        out.push_str("  [parallel]");
    }
    if !t.conflicts_with.is_empty() {
        out.push_str(&format!("  [conflicts: {}]", ids(&t.conflicts_with)));
    }
    out
}

/// `rtok task list`: one line per task, subtasks indented under their parent.
pub fn table(tasks: &[Task]) -> String {
    let width = tasks
        .iter()
        .map(|t| t.id.to_string().len() + 2 * (t.id.path().len() - 1))
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for t in tasks {
        let id = format!("{}{}", "  ".repeat(t.id.path().len() - 1), t.id);
        out.push_str(&format!(
            "{id:<width$}  {:<11}  {}{}\n",
            t.status.as_str(),
            t.title,
            markers(t)
        ));
    }
    out
}

/// `rtok task show`: the fields, then the description.
pub fn details(shown: &Shown) -> String {
    let t = &shown.task;
    let mut out = format!("{}  {}\nstatus: {}\n", t.id, t.title, t.status);
    if let Some(p) = &t.parent {
        out.push_str(&format!("parent: {p}\n"));
    }
    if t.priority != super::DEFAULT_PRIORITY {
        out.push_str(&format!("priority: {}\n", t.priority));
    }
    if let Some(agent) = &t.assignee {
        out.push_str(&format!("assignee: {agent}\n"));
    }
    if !t.blocked_by.is_empty() {
        let ids: Vec<String> = t.blocked_by.iter().map(ToString::to_string).collect();
        out.push_str(&format!("blocked by: {}\n", ids.join(", ")));
    }
    if !t.conflicts_with.is_empty() {
        out.push_str(&format!("conflicts with: {}\n", ids(&t.conflicts_with)));
    }
    if t.parallel {
        out.push_str("parallel: yes\n");
    }
    if !shown.subtasks.is_empty() {
        let ids: Vec<String> = shown.subtasks.iter().map(ToString::to_string).collect();
        out.push_str(&format!("subtasks: {}\n", ids.join(", ")));
    }
    if let Some(x) = &t.external {
        out.push_str(&format!("link: {}\n", x.url));
    }
    if !t.description.is_empty() {
        out.push_str(&format!("\n{}\n", t.description));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkout(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rtok-taskrun-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(
            dir.join(".git/config"),
            "[remote \"origin\"]\n\turl = git@github.com:me/airtalk.git\n",
        )
        .unwrap();
        dir
    }

    fn new(title: &str, parent: Option<&str>) -> NewTask {
        NewTask {
            title: title.into(),
            description: String::new(),
            parent: parent.map(|p| p.parse().unwrap()),
        }
    }

    #[test]
    fn create_numbers_past_tasks_already_on_disk_and_next_picks_a_leaf() {
        let dir = checkout("create");
        let store = Store::open_in_memory().unwrap();
        let project = Project::open(&crate::config::Tasks::default(), &dir).unwrap();
        assert_eq!(project.prefix, "A");
        assert_eq!(project.key, "github.com/me/airtalk");

        // A task that came in with a pull: the counter never saw it.
        let pulled: TaskId = "A7".parse().unwrap();
        project
            .adapter()
            .create(&new("Pulled", None), &pulled)
            .unwrap();
        let t = project.create(&store, &new("First", None)).unwrap();
        assert_eq!(t.id.to_string(), "A8");
        let sub = project.create(&store, &new("Leaf", Some("A7"))).unwrap();
        assert_eq!(sub.id.to_string(), "A7.1");
        assert!(project.create(&store, &new("Orphan", Some("A99"))).is_err());

        assert_eq!(
            project.next(&store).unwrap().unwrap().id.to_string(),
            "A7.1"
        );
        let shown = project.show(&pulled).unwrap();
        assert_eq!(shown.subtasks, vec![sub.id.clone()]);
        assert!(details(&shown).contains("subtasks: A7.1"));
        let listed = project.adapter().list(&Filter::default()).unwrap();
        assert_eq!(
            table(&listed),
            "A7      open         Pulled\n  A7.1  open         Leaf\nA8      open         First\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn init_writes_tasks_and_keeps_the_rest_of_the_file() {
        let dir = checkout("init");
        std::fs::write(dir.join(".rtok.toml"), "# mine\n[proxy]\nport = 9999\n").unwrap();
        let path = init(&dir, None, Some("at")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# mine\n[proxy]\nport = 9999\n"), "{text}");
        assert!(
            text.contains("[tasks]\nadapter = \"disk\"\nprefix = \"AT\"\n"),
            "{text}"
        );
        init(&dir, Some("github"), None).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("adapter = \"github\"") && text.contains("prefix = \"AT\""));
        assert!(init(&dir, Some("jira"), None).is_err());
        assert!(init(&dir, None, Some("A1")).is_err());
        assert!(crate::config::validate::issues(&path).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gitlab_needs_an_origin_on_its_host_and_an_https_url() {
        let dir = checkout("remote");
        let mut cfg = crate::config::Tasks {
            adapter: "gitlab".into(),
            ..crate::config::Tasks::default()
        };
        // Both fail before any token lookup or request.
        let err = Project::open(&cfg, &dir).err().unwrap();
        assert!(
            err.to_string().contains("set [tasks.gitlab] project"),
            "{err}"
        );
        cfg.gitlab.url = "http://gitlab.example.com".into();
        let err = Project::open(&cfg, &dir).err().unwrap();
        assert!(err.to_string().contains("https://"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod claim_tests {
    use std::sync::Arc;

    use super::*;
    use crate::tasks::disk::DiskAdapter;

    fn id(s: &str) -> TaskId {
        s.parse().unwrap()
    }

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rtok-claim-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn project(path: &Path) -> Project {
        Project::with_adapter(
            "proj".into(),
            path.to_path_buf(),
            "R".into(),
            Box::new(DiskAdapter::new(path.to_path_buf())),
        )
    }

    fn agent(store: &Store, session: &str, seen: i64) -> String {
        let host = store.host_id("claude").unwrap().unwrap();
        let id = store
            .register_agent(host, session, None, None, None)
            .unwrap();
        store.set_agent_last_seen(&id, seen).unwrap();
        id
    }

    fn bytes(path: &Path) -> Vec<u8> {
        std::fs::read(path).unwrap()
    }

    #[test]
    fn two_claims_one_winner_until_the_holder_is_stale() {
        let path = dir("race");
        let store = Arc::new(Store::open_in_memory().unwrap());
        let now = ready::unix_now();
        let a = agent(&store, "a", now);
        let b = agent(&store, "b", now);
        project(&path)
            .adapter()
            .create(
                &NewTask {
                    title: "Ship".into(),
                    description: String::new(),
                    parent: None,
                },
                &id("R1"),
            )
            .unwrap();

        let path_b = path.clone();
        let store_b = Arc::clone(&store);
        let b2 = b.clone();
        let winner = std::thread::scope(|scope| {
            let left = scope.spawn(|| project(&path).claim(&store, Some(&id("R1")), &a));
            let right = scope.spawn(|| project(&path_b).claim(&store_b, Some(&id("R1")), &b2));
            let left = left.join().unwrap();
            let right = right.join().unwrap();
            match (left, right) {
                (Ok(ok), Err(err)) | (Err(err), Ok(ok)) => {
                    assert!(ok.changed, "the winner rewrote the file");
                    assert!(err.downcast_ref::<ClaimConflict>().is_some(), "{err:#}");
                    ok
                }
                other => panic!("expected one claim and one conflict, got {other:?}"),
            }
        });
        let holder = winner.task.assignee.clone().unwrap();
        assert!(holder == a || holder == b, "{holder}");

        let file = std::fs::read_dir(&path)
            .unwrap()
            .find_map(|e| {
                let p = e.unwrap().path();
                p.extension().is_some_and(|x| x == "md").then_some(p)
            })
            .unwrap();
        let before = bytes(&file);
        let again = project(&path)
            .claim(&store, Some(&id("R1")), &holder)
            .unwrap();
        assert!(!again.changed);
        assert_eq!(bytes(&file), before, "a re-claim does not rewrite the file");

        store
            .set_agent_last_seen(&holder, now - crate::tasks::STALE_SECS - 5)
            .unwrap();
        let other = if holder == a { b.as_str() } else { a.as_str() };
        let taken = project(&path)
            .claim(&store, Some(&id("R1")), other)
            .unwrap();
        assert!(taken.changed, "a stale claim can be taken");
        assert_eq!(taken.task.assignee.as_deref(), Some(other));
        let _ = std::fs::remove_dir_all(&path);
    }

    /// An adapter that implements only what the trait requires: it stores no markers.
    struct Bare(DiskAdapter);

    impl TaskAdapter for Bare {
        fn name(&self) -> &'static str {
            "bare"
        }
        fn create(&self, task: &NewTask, id: &TaskId) -> Result<Task> {
            self.0.create(task, id)
        }
        fn list(&self, filter: &Filter) -> Result<Vec<Task>> {
            self.0.list(filter)
        }
        fn get(&self, id: &TaskId) -> Result<Option<Task>> {
            self.0.get(id)
        }
        fn write_status(&self, id: &TaskId, status: Status) -> Result<Task> {
            self.0.write_status(id, status)
        }
        fn max_id(&self, prefix: &str) -> Result<Option<TaskId>> {
            self.0.max_id(prefix)
        }
    }

    #[test]
    fn markers_are_symmetric_idempotent_and_refused_where_they_cannot_be_kept() {
        let path = dir("markers");
        let p = project(&path);
        for n in ["R1", "R2"] {
            p.adapter()
                .create(
                    &NewTask {
                        title: n.into(),
                        description: String::new(),
                        parent: None,
                    },
                    &id(n),
                )
                .unwrap();
        }
        let edge = p.conflict(&id("R1"), &id("R2"), false).unwrap();
        assert_eq!(edge.conflicts_with, vec![id("R2")]);
        assert_eq!(p.get(&id("R2")).unwrap().conflicts_with, vec![id("R1")]);

        let file = std::fs::read_dir(&path)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|f| f.file_name().unwrap().to_str().unwrap().starts_with("R1 "))
            .unwrap();
        let before = bytes(&file);
        p.conflict(&id("R2"), &id("R1"), false).unwrap();
        assert_eq!(
            bytes(&file),
            before,
            "an edge already there is not rewritten"
        );
        assert!(p.conflict(&id("R1"), &id("R1"), false).is_err());
        assert!(p.conflict(&id("R1"), &id("R9"), false).is_err());

        assert!(p.set_parallel(&id("R1"), true).unwrap().parallel);
        let before = bytes(&file);
        p.set_parallel(&id("R1"), true).unwrap();
        assert_eq!(bytes(&file), before);
        assert!(!p.set_parallel(&id("R1"), false).unwrap().parallel);

        let cleared = p.conflict(&id("R1"), &id("R2"), true).unwrap();
        assert!(cleared.conflicts_with.is_empty());
        assert!(p.get(&id("R2")).unwrap().conflicts_with.is_empty());

        let bare = Project::with_adapter(
            "proj".into(),
            path.clone(),
            "R".into(),
            Box::new(Bare(DiskAdapter::new(path.clone()))),
        );
        let err = bare.conflict(&id("R1"), &id("R2"), false).unwrap_err();
        assert!(err.to_string().contains("bare tasks cannot store"), "{err}");
        assert!(bare.set_parallel(&id("R1"), true).is_err());
        assert!(bare.get(&id("R2")).unwrap().conflicts_with.is_empty());

        // A peer whose file is gone leaves a stale edge that `--remove` must still clear.
        p.conflict(&id("R1"), &id("R2"), false).unwrap();
        std::fs::remove_file(path.join(format!("R2 - {}.md", "r2"))).unwrap();
        assert!(p.conflict(&id("R1"), &id("R2"), false).is_err());
        assert!(
            p.conflict(&id("R1"), &id("R2"), true)
                .unwrap()
                .conflicts_with
                .is_empty()
        );
        assert!(p.get(&id("R1")).unwrap().conflicts_with.is_empty());
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_missing_agent_row_can_be_taken_and_a_block_writes_nothing_on_a_cycle() {
        let path = dir("block");
        let store = Store::open_in_memory().unwrap();
        let now = ready::unix_now();
        let live = agent(&store, "live", now);
        let p = project(&path);
        for (task_id, title) in [("R1", "Parent"), ("R1.1", "Child"), ("R2", "Other")] {
            p.adapter()
                .create(
                    &NewTask {
                        title: title.into(),
                        description: String::new(),
                        parent: None,
                    },
                    &id(task_id),
                )
                .unwrap();
        }
        let first = p.claim(&store, Some(&id("R1")), "not-registered").unwrap();
        assert!(first.changed);
        let stolen = p.claim(&store, Some(&id("R1")), &live).unwrap();
        assert!(stolen.changed, "no agents row is stale");

        p.release(&store, &id("R1"), &live, false).unwrap();
        let child = std::fs::read_dir(&path)
            .unwrap()
            .find_map(|e| {
                let p = e.unwrap().path();
                p.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("R1.1")
                    .then_some(p)
            })
            .unwrap();
        let before = bytes(&child);
        let err = p.block(&id("R1.1"), &id("R1")).unwrap_err();
        assert!(err.to_string().contains("cycle"), "{err}");
        assert_eq!(bytes(&child), before, "a cycle writes nothing");
        assert!(p.block(&id("R1"), &id("R1")).is_err());
        assert!(p.block(&id("R1"), &id("R9")).is_err());

        p.block(&id("R2"), &id("R1")).unwrap();
        let again = bytes(
            &std::fs::read_dir(&path)
                .unwrap()
                .find_map(|e| {
                    let p = e.unwrap().path();
                    p.file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .starts_with("R2 ")
                        .then_some(p)
                })
                .unwrap(),
        );
        p.block(&id("R2"), &id("R1")).unwrap();
        let after = std::fs::read(
            std::fs::read_dir(&path)
                .unwrap()
                .find_map(|e| {
                    let p = e.unwrap().path();
                    p.file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .starts_with("R2 ")
                        .then_some(p)
                })
                .unwrap(),
        )
        .unwrap();
        assert_eq!(again, after, "a duplicate edge is not rewritten");

        let err = p.claim(&store, Some(&id("R2")), &live).unwrap_err();
        assert!(err.to_string().contains("R2 is blocked by R1"), "{err}");
        let ready = p.ready(&store).unwrap();
        assert!(ready.iter().all(|item| item.task.id != id("R2")));
        let next = p.claim(&store, None, &live).unwrap();
        assert_ne!(next.task.id, id("R2"));
        p.status(&store, &id("R1"), Some(Status::Done), true)
            .unwrap();
        assert!(
            store
                .latest_task_claim("proj", &live)
                .unwrap()
                .is_none_or(|row| row.task_id != "R1")
        );
        let _ = std::fs::remove_dir_all(&path);
    }
}
