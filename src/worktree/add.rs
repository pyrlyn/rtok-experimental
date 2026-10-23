// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok worktree add` (T158): the one creation path — one location, one name, one
//! owner. [`plan`] decides everything from paths and strings; [`run`] is one fetch and
//! one `git worktree add`.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use rtok_config::root::{ProjectRoot, contain, resolved};

use super::{Owner, git};
use crate::store::Store;

#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub path: PathBuf,
    pub branch: String,
    pub task: String,
    /// The lock reason, `<owner> | <task-id> | <date>[ | agent <uuid>]` — what
    /// [`Owner::parse`] reads back.
    pub reason: String,
}

/// Lower-cased, then `[a-z0-9][a-z0-9._-]*` minus what git refuses in a ref name.
fn ident(what: &str, raw: &str) -> Result<String> {
    let id = raw.to_ascii_lowercase();
    let allowed = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c);
    let ok = id.starts_with(|c: char| c.is_ascii_alphanumeric())
        && id.chars().all(allowed)
        && !id.contains("..")
        && !id.ends_with('.')
        && !id.ends_with(".lock");
    ensure!(
        ok,
        "{what} `{raw}` must match [a-z0-9][a-z0-9._-]* and be a valid branch name"
    );
    Ok(id)
}

/// Agents pass `T504 read-hook-same-path` the way the branch is named: one argument. The first
/// token is the task id, the rest the slug; spaces in a slug become dashes, a separate `slug`
/// is joined after the rest, and a bare id is unchanged.
fn split_id_slug<'a>(task: &'a str, slug: Option<&str>) -> (&'a str, Option<String>) {
    let task = task.trim();
    let (id, rest) = task.split_once(char::is_whitespace).unwrap_or((task, ""));
    let dashed = |s: &str| {
        let d = s.split_whitespace().collect::<Vec<_>>().join("-");
        (!d.is_empty()).then_some(d)
    };
    let slug = match (dashed(rest), slug.and_then(dashed)) {
        (Some(rest), Some(slug)) => Some(format!("{rest}-{slug}")),
        (rest, slug) => rest.or(slug),
    };
    (id, slug)
}

/// The directory `root` names for the repository whose main checkout is `main`. A relative
/// root is under `main`, never the process cwd: a worktree added from inside another one still
/// lands beside its siblings. `~` was expanded when the config loaded.
pub fn landing(main: &Path, root: &Path) -> PathBuf {
    if in_project(root) {
        main.join(root)
    } else {
        root.to_path_buf()
    }
}

/// Whether `root` names a place inside the project. Not `is_relative`: on Windows `\x` has no
/// drive, so it is "relative", yet it names the drive root, not a directory of the project.
fn in_project(root: &Path) -> bool {
    !root.has_root() && !matches!(root.components().next(), Some(Component::Prefix(_)))
}

/// `temp` lists the directories the OS may purge; a worktree under one of them loses
/// its files while git keeps the (locked, never pruned) record.
pub fn plan(
    main: &Path,
    root: &Path,
    temp: &[PathBuf],
    (task, slug): (&str, Option<&str>),
    (owner, agent): (&str, Option<&str>),
    date: &str,
) -> Result<Plan> {
    let (task, slug) = split_id_slug(task, slug);
    let task = ident("task id", task)?;
    let branch = match slug {
        Some(slug) => format!("{task}-{}", ident("slug", &slug)?),
        None => task.clone(),
    };
    let reason = Owner {
        owner: owner.into(),
        task: task.clone(),
        date: date.into(),
        agent: agent.map(Into::into),
    }
    .reason();
    // ASCII: `git worktree list --porcelain` C-quotes anything else.
    let round_trips = Owner::parse(&reason).is_some_and(|o| o.owner == owner);
    ensure!(
        owner.is_ascii() && round_trips && !owner.contains(['\n', '\r']),
        "--owner `{owner}` must be non-empty ASCII without ` | `, e.g. \"Claude Code / sonnet\""
    );
    ensure!(
        !root.as_os_str().is_empty(),
        "`[worktree] root` is empty; set it or drop the key for .rtok/worktrees"
    );
    let base = landing(main, root);
    let (landing, checkout) = (resolved(&base), resolved(main));
    // A repository that itself lives under the temp directory (a test fixture, a scratch
    // clone) is no worse off with its worktrees beside it.
    let purgeable = |t: &PathBuf| {
        crate::fs::path_starts_with(&landing, t) && !crate::fs::path_starts_with(&checkout, t)
    };
    if let Some(tmp) = temp.iter().map(|t| resolved(t)).find(purgeable) {
        bail!(
            "worktree root {} is under the temporary directory {}; set `[worktree] root`",
            root.display(),
            tmp.display()
        );
    }
    let repo = main
        .file_name()
        .context("the main checkout has no directory name")?;
    let name = format!("{}-{task}", repo.to_string_lossy());
    let path = base.join(&name);
    if in_project(root) {
        // A relative root is a promise to stay in the project; `../x` would break it.
        contain(main, &root.join(&name)).with_context(|| {
            format!(
                "worktree path {} is outside the main checkout {}; use an absolute `[worktree] root`",
                path.display(),
                main.display()
            )
        })?;
    }
    Ok(Plan {
        path,
        branch,
        task,
        reason,
    })
}

/// What a retry of an add that already landed gets back: the plan of the worktree that is there.
/// `Ok(None)` when nothing is at the path; an error when something else is — another task's
/// directory, a worktree someone else holds, or a directory git does not know.
fn existing(
    worktrees: &[super::Record],
    plan: &Plan,
    (owner, agent): (&str, Option<&str>),
    store: Option<&Store>,
) -> Result<Option<Plan>> {
    if !plan.path.exists() {
        return Ok(None);
    }
    let clash = |why: String| {
        anyhow::anyhow!(
            "{} already exists: one worktree per task{why}",
            plan.path.display()
        )
    };
    let here = crate::fs::canon(&plan.path);
    let record = worktrees
        .iter()
        .skip(1)
        .find(|r| crate::fs::same_path(&crate::fs::canon(&r.path), &here))
        .ok_or_else(|| clash(String::new()))?;
    let ours = record.owner().is_some_and(|o| o.task == plan.task);
    // An agent-less caller has no id to match a lock that names one, so it never reuses it.
    let held = super::claim::held_against(record, (owner, agent.unwrap_or("")), store);
    if let Some(held) = held {
        return Err(clash(format!(" ({})", held.locked())));
    }
    if !ours {
        return Err(clash(String::new()));
    }
    Ok(Some(Plan {
        branch: record.branch.clone().unwrap_or_else(|| plan.branch.clone()),
        path: plan.path.clone(),
        task: plan.task.clone(),
        reason: plan.reason.clone(),
    }))
}

/// A created worktree with what undoes it, or the one a retry found already there.
pub enum Added<'a> {
    New(Plan, Undo<'a>),
    Existing(Plan),
}

/// The rule [`ensure_ignored`] adds.
const IGNORE_RULE: &str = ".rtok/";

/// Makes git ignore `<main>/.rtok/`, so worktrees inside it never show as untracked files of
/// main. A rule already in effect is left alone; else `.gitignore` gets it when the project
/// has one (the rest kept byte-for-byte), else the clone-local `info/exclude`.
pub fn ensure_ignored(main: &Path) -> Result<()> {
    if git::is_ignored(main, IGNORE_RULE) {
        return Ok(());
    }
    let gitignore = main.join(".gitignore");
    let file = if gitignore.is_file() {
        gitignore
    } else {
        let git_dir = ProjectRoot::find(main).map_or_else(|| main.join(".git"), |r| r.git_dir);
        let info = git_dir.join("info");
        std::fs::create_dir_all(&info)
            .with_context(|| format!("cannot create {}", info.display()))?;
        info.join("exclude")
    };
    append_rule(&file)
}

fn append_rule(file: &Path) -> Result<()> {
    let mut bytes = match std::fs::read(file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", file.display())),
    };
    if bytes.last().is_some_and(|b| *b != b'\n') {
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(IGNORE_RULE.as_bytes());
    bytes.push(b'\n');
    std::fs::write(file, bytes).with_context(|| format!("cannot write {}", file.display()))
}

/// `YYYY-MM-DD`, UTC: the lock reason's date.
pub(super) fn today() -> Result<String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?;
    Ok(crate::log::stamp(now.as_secs())[..10].to_string())
}

/// The two effects a healthy test machine cannot cause: a nearly full volume, and a
/// `git worktree add` that dies half-way.
#[derive(Clone, Copy)]
pub struct Seams {
    /// Free bytes on the volume that holds an existing path.
    pub free_bytes: fn(&Path) -> std::io::Result<u64>,
    /// `git worktree add` for `(repo, path, branch, lock reason, base)`.
    pub create: fn(&Path, &Path, &str, &str, &str) -> Result<()>,
}

impl Seams {
    pub const HOST: Self = Self {
        free_bytes: rtok_sys::free_bytes,
        create: git::add_locked,
    };
}

/// Refuses when the volume that would hold `path` has less than `min_mib` MiB free. `path` does
/// not exist yet, so the nearest ancestor that does stands for its volume. A reading that fails
/// lets the add go on: an unknown is no reason to block work.
fn ensure_room(seams: &Seams, path: &Path, min_mib: u32) -> Result<()> {
    let Some(on) = path
        .ancestors()
        .find(|p| p.exists())
        .filter(|_| min_mib > 0)
    else {
        return Ok(());
    };
    let free = match (seams.free_bytes)(on) {
        Ok(free) => free >> 20,
        Err(e) => {
            crate::log::stderr_ln(&format!(
                "warning: free space of {} unknown: {e}",
                on.display()
            ));
            return Ok(());
        }
    };
    ensure!(
        free >= u64::from(min_mib),
        "only {free} MiB free for {}, below `[worktree] min_free_mib` = {min_mib}; free disk space or lower it (0 turns the check off)",
        path.display()
    );
    Ok(())
}

/// What a failed add takes back: the branch, the worktree and the claim this call made, in
/// reverse order, unless [`Undo::keep`] says the add went through. A branch or directory that
/// was there before the call is never recorded, so it stays.
#[derive(Default)]
pub struct Undo<'a> {
    steps: Vec<Step<'a>>,
}

enum Step<'a> {
    Branch { repo: PathBuf, name: String },
    Worktree { repo: PathBuf, path: PathBuf },
    Claim { store: &'a Store, path: String },
}

impl<'a> Undo<'a> {
    /// Records a claim in the store, to be released on a failure.
    pub fn claim(&mut self, store: &'a Store, path: &Path) {
        let path = crate::fs::canon(path).to_string_lossy().into_owned();
        self.steps.push(Step::Claim { store, path });
    }

    /// The add went through; nothing is taken back.
    pub fn keep(mut self) {
        self.steps.clear();
    }
}

impl Drop for Undo<'_> {
    fn drop(&mut self) {
        for step in self.steps.drain(..).rev() {
            let done = match &step {
                Step::Claim { store, path } => super::claim::release(store, path),
                Step::Worktree { repo, path } => {
                    // A lock outlives a failed add: `prune` leaves a locked record alone.
                    let _ = git::unlock(repo, path);
                    fs_kit::tree::remove_any(path)
                        .map_err(Into::into)
                        .and_then(|()| git::prune(repo))
                }
                Step::Branch { repo, name } if git::has_branch(repo, name) => {
                    git::delete_branch(repo, name)
                }
                Step::Branch { .. } => Ok(()),
            };
            if let Err(e) = done {
                crate::log::stderr_ln(&format!("warning: add not fully undone: {e:#}"));
            }
        }
    }
}

/// Creates the worktree and returns its plan; a retry of an add that already landed (the path
/// is this task's worktree, held by this owner) gets that worktree back and touches nothing. A
/// failed fetch is an error: branching from a stale base is how a finished task gets rebuilt on
/// old code. A failure here leaves nothing behind; the caller keeps the [`Undo`] until its own
/// steps are done.
pub fn run<'a>(
    cwd: &Path,
    wt: &crate::config::Worktree,
    seams: Seams,
    id: (&str, Option<&str>),
    owner: (&str, Option<&str>),
    store: Option<&Store>,
) -> Result<Added<'a>> {
    let worktrees = git::list(cwd)?;
    let main = &worktrees.first().context("git lists no worktree")?.path;
    let temp = [std::env::temp_dir(), "/tmp".into()];
    let plan = plan(main, &wt.root, &temp, id, owner, &today()?)?;
    if let Some(again) = existing(&worktrees, &plan, owner, store)? {
        return Ok(Added::Existing(again));
    }
    ensure_room(&seams, &plan.path, wt.min_free_mib)?;
    let base = git::default_base(main);
    git::fetch(main, &base)?;
    if crate::fs::path_starts_with(&resolved(&plan.path), &resolved(main)) {
        ensure_ignored(main)?;
    }
    if let Some(root) = plan.path.parent() {
        std::fs::create_dir_all(root)
            .with_context(|| format!("cannot create {}", root.display()))?;
    }
    // `plan` checked this before the fetch; another add of the same task may have made it since,
    // and the rollback below must never remove a directory this call did not create.
    ensure!(
        !plan.path.exists(),
        "{} already exists: one worktree per task",
        plan.path.display()
    );
    let mut undo = Undo::default();
    // Recorded before the call: git may die with the branch made and the directory half-filled.
    if !git::has_branch(main, &plan.branch) {
        undo.steps.push(Step::Branch {
            repo: main.clone(),
            name: plan.branch.clone(),
        });
    }
    undo.steps.push(Step::Worktree {
        repo: main.clone(),
        path: plan.path.clone(),
    });
    (seams.create)(main, &plan.path, &plan.branch, &plan.reason, &base)?;
    Ok(Added::New(plan, undo))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::tmp_dir;
    use rstest::rstest;

    const OWNER: &str = "Claude Code / sonnet";

    fn plan_in(main: &Path, root: &Path, id: (&str, Option<&str>), owner: &str) -> Result<Plan> {
        plan(
            main,
            root,
            &["/purged".into()],
            id,
            (owner, None),
            "2026-09-22",
        )
    }

    #[test]
    fn the_root_holds_the_worktree_and_the_names_follow_the_task() {
        let dir = tmp_dir("wt-add");
        let (main, root) = (dir.join("apps/rtok"), dir.join("home/.rtok/worktrees"));
        std::fs::create_dir_all(&main).unwrap();
        // T410: a `_worktrees/` beside the repository no longer pulls the worktree there.
        std::fs::create_dir_all(dir.join("_worktrees")).unwrap();
        let p = plan_in(&main, &root, ("T158", Some("Worktree-Add")), OWNER).unwrap();
        assert_eq!(p.path, root.join("rtok-t158"));
        assert_eq!(p.branch, "t158-worktree-add");
        assert_eq!(p.reason, "Claude Code / sonnet | t158 | 2026-09-22");
        let parsed = Owner::parse(&p.reason).unwrap();
        assert_eq!(
            (parsed.owner.as_str(), parsed.task.as_str()),
            (OWNER, "t158")
        );

        let empty = plan_in(&main, Path::new(""), ("t2", None), OWNER).unwrap_err();
        assert!(empty.to_string().contains("is empty"), "{empty}");
    }

    /// T523: agents name the branch in one argument; the first token is the id, the rest the slug.
    #[rstest]
    #[case::logged_t504("T504 read-hook-same-path", "t504", "t504-read-hook-same-path")]
    #[case::logged_m16_5("M16.5 test toml", "m16.5", "m16.5-test-toml")]
    #[case::logged_m16_3(
        "M16.3 push.rs through the config module",
        "m16.3",
        "m16.3-push.rs-through-the-config-module"
    )]
    #[case::bare_id("T504", "t504", "t504")]
    fn one_argument_is_an_id_plus_a_slug(
        #[case] arg: &str,
        #[case] task: &str,
        #[case] branch: &str,
    ) {
        let p = plan_in(Path::new("/r/rtok"), Path::new("/r/_w"), (arg, None), OWNER).unwrap();
        assert_eq!((p.task.as_str(), p.branch.as_str()), (task, branch));
    }

    /// T523: a retry gets its worktree back; a clash with another owner or task is still refused.
    #[rstest]
    #[case::same_task_and_owner(OWNER, "t504", true)]
    #[case::another_owner("Cursor / grok", "t504", false)]
    #[case::another_task(OWNER, "t505", false)]
    fn only_the_same_task_and_owner_gets_its_worktree_back(
        #[case] holder: &str,
        #[case] task: &str,
        #[case] again: bool,
    ) {
        let dir = tmp_dir("wt-retry");
        let (main, root) = (dir.join("apps/rtok"), dir.join("home/.rtok/worktrees"));
        std::fs::create_dir_all(&main).unwrap();
        let taken = root.join("rtok-t504");
        std::fs::create_dir_all(&taken).unwrap();
        let records = [
            super::super::Record {
                path: main.clone(),
                ..Default::default()
            },
            super::super::Record {
                path: taken.clone(),
                branch: Some("t504-read-hook".into()),
                locked: Some(format!("{holder} | {task} | 2026-10-24")),
                ..Default::default()
            },
        ];
        let plan = plan_in(&main, &root, ("T504 other-words", None), OWNER).unwrap();
        let got = existing(&records, &plan, (OWNER, None), None);
        if again {
            // The branch is the one that is there, not the spelling of this call.
            assert_eq!(got.unwrap().unwrap().branch, "t504-read-hook");
        } else {
            let err = got.unwrap_err().to_string();
            assert!(err.contains("one worktree per task"), "{err}");
        }
        let free = plan_in(&main, &root, ("t7", None), OWNER).unwrap();
        assert!(
            existing(&records, &free, (OWNER, None), None)
                .unwrap()
                .is_none()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn spaces_in_a_separate_slug_become_dashes_too() {
        let id = ("T504", Some("read hook same path"));
        let p = plan_in(Path::new("/r/rtok"), Path::new("/r/_w"), id, OWNER).unwrap();
        assert_eq!(p.branch, "t504-read-hook-same-path");
        let both = ("T504 read", Some("hook"));
        let p = plan_in(Path::new("/r/rtok"), Path::new("/r/_w"), both, OWNER).unwrap();
        assert_eq!(p.branch, "t504-read-hook");
    }

    /// The path of a task that already has one is still its plan: `run` decides whether the
    /// directory is that task's worktree, so a retry can be told from a clash.
    #[test]
    fn an_existing_path_is_planned_like_any_other() {
        let dir = tmp_dir("wt-existing");
        let (main, root) = (dir.join("apps/rtok"), dir.join("home/.rtok/worktrees"));
        std::fs::create_dir_all(&main).unwrap();
        std::fs::create_dir_all(root.join("rtok-t504")).unwrap();
        let p = plan_in(&main, &root, ("T504 read-hook-same-path", None), OWNER).unwrap();
        assert_eq!(p.path, root.join("rtok-t504"));
        assert_eq!(p.branch, "t504-read-hook-same-path");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T660.4: the default root is inside the project; relative roots start at the main
    /// checkout, absolute ones stay; a relative root may not climb out of the project.
    #[test]
    fn a_relative_root_is_under_the_main_checkout_and_may_not_leave_it() {
        let root = crate::config::Config::default().worktree.root;
        assert_eq!(root, Path::new(".rtok/worktrees"));
        let dir = tmp_dir("wt-add-relative");
        let main = dir.join("apps/rtok");
        std::fs::create_dir_all(&main).unwrap();
        let at = |root: &str| plan_in(&main, Path::new(root), ("t1", None), OWNER);
        assert_eq!(
            at(".rtok/worktrees").unwrap().path,
            main.join(".rtok/worktrees/rtok-t1")
        );
        assert_eq!(at("wt").unwrap().path, main.join("wt/rtok-t1"));
        let abs = dir.join("elsewhere");
        let abs_plan = plan_in(&main, &abs, ("t1", None), OWNER).unwrap();
        assert_eq!(abs_plan.path, abs.join("rtok-t1"));
        for escaping in ["../x", "a/../../x"] {
            let err = at(escaping).unwrap_err();
            assert!(
                format!("{err:#}").contains("outside the main checkout"),
                "{err:#}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A root with no drive (`\x`, `/x` on Windows) names the drive root, not the project.
    #[test]
    fn a_rooted_root_is_never_inside_the_project() {
        assert!(in_project(Path::new(".rtok/worktrees")) && in_project(Path::new("../x")));
        assert!(!in_project(Path::new("/r/_w")));
        assert_eq!(
            landing(Path::new("/r/rtok"), Path::new("/r/_w")),
            Path::new("/r/_w")
        );
        #[cfg(windows)]
        {
            assert!(!in_project(Path::new(r"\r\_w")));
            assert!(!in_project(Path::new(r"C:\r\_w")));
            assert!(!in_project(Path::new("C:x")));
        }
    }

    #[test]
    fn a_known_agent_is_the_fourth_field_of_the_lock() {
        let agent = "an-agent-id";
        let p = plan(
            Path::new("/r/rtok"),
            Path::new("/r/_w"),
            &[],
            ("t1", None),
            (OWNER, Some(agent)),
            "d",
        );
        let reason = p.unwrap().reason;
        assert_eq!(reason, format!("{OWNER} | t1 | d | agent {agent}"));
        assert_eq!(Owner::parse(&reason).unwrap().agent.as_deref(), Some(agent));
    }

    #[rstest]
    #[case::empty_id(("", None), OWNER, "task id")]
    #[case::path_in_id(("../t1", None), OWNER, "task id")]
    #[case::leading_dash(("-t1", None), OWNER, "task id")]
    #[case::ref_rule(("t1.lock", None), OWNER, "task id")]
    #[case::not_an_id_after_split(("../t1 rest", None), OWNER, "task id")]
    #[case::dash_id_after_split(("-t1 rest", None), OWNER, "task id")]
    #[case::bad_slug(("t1", Some("../evil")), OWNER, "slug")]
    #[case::bad_slug_in_the_argument(("t1 ../evil", None), OWNER, "slug")]
    #[case::no_owner(("t1", None), "", "--owner")]
    #[case::separator_in_owner(("t1", None), "a | b", "--owner")]
    #[case::non_ascii_owner(("t1", None), "Клод / sonnet", "--owner")]
    fn bad_input_is_refused_before_git_runs(
        #[case] id: (&str, Option<&str>),
        #[case] owner: &str,
        #[case] names: &str,
    ) {
        let err = plan_in(Path::new("/r/rtok"), Path::new("/r/_w"), id, owner).unwrap_err();
        assert!(err.to_string().contains(names), "{err}");
    }

    #[test]
    fn a_root_under_a_temporary_directory_is_refused_unless_the_repository_is_there_too() {
        let purged = Path::new("/purged/x");
        let err = plan_in(Path::new("/r/rtok"), purged, ("t1", None), OWNER).unwrap_err();
        assert!(err.to_string().contains("temporary directory"), "{err}");
        let beside = Path::new("/purged/session/_worktrees");
        let scratch = plan_in(
            Path::new("/purged/session/rtok"),
            beside,
            ("t1", None),
            OWNER,
        );
        assert_eq!(
            scratch.unwrap().path,
            Path::new("/purged/session/_worktrees/rtok-t1")
        );
    }

    /// T430: a missing child of a real directory does not keep `std::fs::canonicalize`'s `\\?\`.
    #[cfg(windows)]
    #[test]
    fn resolved_drops_the_verbatim_prefix() {
        let dir = tmp_dir("wt-resolved");
        let got = resolved(&dir.join("child"));
        let text = got.to_string_lossy();
        assert!(!text.starts_with(r"\\?\"), "{text}");
        assert!(got.ends_with("child"), "{got:?}");
    }
}
