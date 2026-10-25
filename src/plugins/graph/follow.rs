// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T329.8: follow the references of a project (`project::refs::discover`) into other projects.
//! Each referenced directory is registered with origin `reference`, linked `auto` from the
//! project that names it, and followed in turn up to `reference_depth` levels. `max_auto_projects`
//! caps the registry rows references may add. Both limits are reported, never silent.
//!
//! The registry work is quick and runs first; indexing the new projects is [`index_new`], which
//! the callers run off the hook path (`rtok mcp` on a thread, `rtok graph index` after its own
//! index). An auto link whose reference vanished from the manifests is dropped and its project
//! kept; a link the user removed (`unlinked`) is not re-created and not followed; a manual link
//! is never touched, because [`Store::link_projects`] and [`Store::drop_auto_links`] leave it be.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;

use crate::config::Graph;
use crate::plugin::{Ctx, Runtime};
use crate::project::refs::discover;
use crate::store::{LinkKind, Origin, Project, Store, is_locked, retry_locked};

/// Same bound as a worktree claim (T934): one `busy_timeout` is not enough when graph
/// indexing holds the writer, and a warning on every MCP start was the 53-line flood.
const LOCK_ATTEMPTS: u32 = 8;
const LOCK_PAUSE: Duration = Duration::from_millis(250);

/// Why a branch of references was not followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    Depth { project: String, limit: u32 },
    Cap { dir: String, limit: u32 },
}

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stop::Depth { project, limit } => write!(
                f,
                "reference_depth {limit} reached at {project}: its references are not followed"
            ),
            Stop::Cap { dir, limit } => {
                write!(f, "max_auto_projects {limit} reached: {dir} was not added")
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct Followed {
    /// Projects this run added to the registry; they still need an index.
    pub registered: Vec<Project>,
    pub linked: usize,
    pub dropped: usize,
    pub stops: Vec<Stop>,
    pub warnings: Vec<String>,
}

/// Walk the references of the project registered for `root`. Nothing happens when the feature is
/// off or `root` is not in the registry (T329.6 puts it there).
pub fn follow(store: &Store, cfg: &Graph, root: &Path) -> Result<Followed> {
    let mut out = Followed::default();
    let Some(start) = store
        .project_by_root(root)?
        .filter(|_| cfg.auto_link_references)
    else {
        return Ok(out);
    };
    let mut auto = store
        .projects()?
        .iter()
        .filter(|p| p.origin == Origin::Reference)
        .count();
    let mut seen = HashSet::from([start.id]);
    let mut level = vec![start];
    let mut depth = 0;
    while !level.is_empty() {
        let active = store.project_links()?;
        let mut next = Vec::new();
        for p in level {
            let found = discover(Path::new(&p.root));
            if depth >= cfg.reference_depth {
                if !found.refs.is_empty() {
                    let project = p.display_name().to_string();
                    out.stops.push(Stop::Depth {
                        project,
                        limit: cfg.reference_depth,
                    });
                }
                continue;
            }
            let unread = found.warnings.iter().any(|w| w.contains(": not read:"));
            out.warnings.extend(
                found
                    .warnings
                    .iter()
                    .map(|w| format!("{}: {w}", p.display_name())),
            );
            let mut keep = Vec::new();
            for r in &found.refs {
                let target = match store.project_by_root(&r.dir)? {
                    Some(t) => t,
                    None if auto >= cfg.max_auto_projects as usize => {
                        let dir = r.dir.display().to_string();
                        out.stops.push(Stop::Cap {
                            dir,
                            limit: cfg.max_auto_projects,
                        });
                        continue;
                    }
                    None => {
                        auto += 1;
                        let t = store.register_project(&r.dir, Origin::Reference)?;
                        out.registered.push(t.clone());
                        t
                    }
                };
                if target.id == p.id {
                    continue;
                }
                let made = store.link_projects(p.id, target.id, LinkKind::Auto, Some(&r.reason))?;
                out.linked += usize::from(made);
                keep.push(target.id);
                // A link the user removed neither comes back nor leads anywhere.
                let live = made || active.iter().any(|l| l.from == p.id && l.to == target.id);
                if live && seen.insert(target.id) {
                    next.push(target);
                }
            }
            // A manifest that could not be read says nothing about what it references.
            if !unread {
                out.dropped += store.drop_auto_links(p.id, &keep)?;
            }
        }
        level = next;
        depth += 1;
    }
    Ok(out)
}

/// Index the projects a [`follow`] added. One failing project is logged and skipped.
pub fn index_new(rt: &Runtime, followed: &Followed) {
    for p in &followed.registered {
        if let Err(e) = super::index::run(&Ctx::new(rt), Path::new(&p.root), false) {
            let msg = format!("indexing reference {} failed: {e:#}", p.root);
            crate::log::append(&rt.config, "warn", "graph", "references", &msg);
        }
    }
}

/// [`follow`] with the limits and failures logged; what `rtok mcp` and `rtok graph index` call.
/// Fails open: a registry error is logged and nothing is returned. A locked store is retried
/// and, if it stays locked, skipped without a warning — the next MCP start tries again.
pub fn refresh(rt: &Runtime, root: &Path) -> Followed {
    refresh_with(rt, root, LOCK_ATTEMPTS, LOCK_PAUSE)
}

fn refresh_with(rt: &Runtime, root: &Path, attempts: u32, pause: Duration) -> Followed {
    let cfg = &rt.config.plugins.graph;
    match retry_locked(attempts, pause, || {
        rt.store.drop_toolless_session_projects()?;
        follow(&rt.store, cfg, root).map_err(Into::into)
    }) {
        Ok(f) => {
            for line in f
                .stops
                .iter()
                .map(ToString::to_string)
                .chain(f.warnings.iter().cloned())
            {
                crate::log::append(&rt.config, "warn", "graph", "references", &line);
            }
            f
        }
        Err(e) if is_locked(&e) => Followed::default(),
        Err(e) => {
            let msg = format!("following references of {} failed: {e:#}", root.display());
            crate::log::append(&rt.config, "warn", "graph", "references", &msg);
            Followed::default()
        }
    }
}

/// What `rtok graph index` does after its own index: follow, index what was added, and say what
/// changed and where a limit stopped.
pub fn report(rt: &Runtime, root: &Path) {
    use crate::ui::style;
    let f = refresh(rt, root);
    index_new(rt, &f);
    if f.registered.is_empty() && f.linked == 0 && f.dropped == 0 {
        return;
    }
    let line = format!(
        "references: {} projects added, {} links made, {} dropped",
        f.registered.len(),
        f.linked,
        f.dropped
    );
    crate::log::stdout_ln(&style::success_op("link", &line));
    for stop in &f.stops {
        crate::log::stdout_ln(&style::warn(&stop.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::tmp_dir;
    use std::path::PathBuf;

    struct Fx {
        base: PathBuf,
        store: Store,
        cfg: Graph,
    }

    impl Fx {
        fn new() -> Self {
            Fx {
                base: tmp_dir("follow"),
                store: Store::open_in_memory().unwrap(),
                cfg: Graph::default(),
            }
        }

        /// A directory `name` whose Cargo.toml depends by path on each of `deps`.
        fn proj(&self, name: &str, deps: &[&str]) -> PathBuf {
            let dir = self.base.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            let body: String = deps
                .iter()
                .map(|d| format!("{d} = {{ path = \"../{d}\" }}\n"))
                .collect();
            std::fs::write(dir.join("Cargo.toml"), format!("[dependencies]\n{body}")).unwrap();
            dir
        }

        fn add(&self, name: &str) -> i32 {
            self.store
                .register_project(&self.base.join(name), Origin::Manual)
                .unwrap()
                .id
        }

        fn follow(&self, name: &str) -> Followed {
            follow(&self.store, &self.cfg, &self.base.join(name)).unwrap()
        }

        /// `from->to:kind` of every active link, by directory name.
        fn links(&self) -> Vec<String> {
            let names: std::collections::HashMap<i32, String> = self
                .store
                .projects()
                .unwrap()
                .into_iter()
                .map(|p| (p.id, p.display_name().to_string()))
                .collect();
            self.store
                .project_links()
                .unwrap()
                .iter()
                .map(|l| format!("{}->{}:{}", names[&l.from], names[&l.to], l.kind.as_str()))
                .collect()
        }
    }

    #[test]
    fn a_registers_and_links_b_and_c_and_not_d_and_a_second_run_changes_nothing() {
        let fx = Fx::new();
        fx.proj("A", &["B", "C"]);
        fx.proj("B", &[]);
        fx.proj("C", &[]);
        fx.proj("D", &[]);
        fx.add("A");
        fx.add("D");
        let f = fx.follow("A");
        assert_eq!(f.registered.len(), 2);
        assert!(f.registered.iter().all(|p| p.origin == Origin::Reference));
        assert_eq!(fx.links(), ["A->B:auto", "A->C:auto"]);
        let again = fx.follow("A");
        assert_eq!(
            (again.registered.len(), again.linked, again.dropped),
            (0, 0, 0)
        );
    }

    #[test]
    fn the_depth_limit_stops_and_says_so() {
        let mut fx = Fx::new();
        fx.proj("A", &["B"]);
        fx.proj("B", &["C"]);
        fx.proj("C", &["D"]);
        fx.proj("D", &[]);
        fx.add("A");
        fx.cfg.reference_depth = 1;
        let f = fx.follow("A");
        assert_eq!(fx.links(), ["A->B:auto"]);
        assert_eq!(
            f.stops,
            [Stop::Depth {
                project: "B".into(),
                limit: 1
            }]
        );
        assert_eq!(
            f.stops[0].to_string(),
            "reference_depth 1 reached at B: its references are not followed"
        );
        fx.cfg.reference_depth = 3;
        let f = fx.follow("A");
        assert_eq!(fx.links(), ["A->B:auto", "B->C:auto", "C->D:auto"]);
        assert!(f.stops.is_empty());
        fx.cfg.reference_depth = 0;
        assert_eq!(fx.follow("A").stops.len(), 1);
    }

    #[test]
    fn the_project_cap_is_reported_and_existing_projects_are_still_linked() {
        let mut fx = Fx::new();
        fx.proj("A", &["B", "C", "D"]);
        for n in ["B", "C", "D"] {
            fx.proj(n, &[]);
        }
        fx.add("A");
        fx.add("D");
        fx.cfg.max_auto_projects = 1;
        let f = fx.follow("A");
        assert_eq!(f.registered.len(), 1);
        assert_eq!(f.stops.len(), 1);
        assert!(matches!(&f.stops[0], Stop::Cap { limit: 1, .. }));
        assert_eq!(fx.links().len(), 2, "B or C, and the already known D");
    }

    #[test]
    fn a_removed_dependency_drops_the_auto_link_and_keeps_the_project() {
        let fx = Fx::new();
        fx.proj("A", &["B"]);
        fx.proj("B", &[]);
        fx.add("A");
        fx.follow("A");
        fx.proj("A", &[]);
        let f = fx.follow("A");
        assert_eq!(f.dropped, 1);
        assert!(fx.links().is_empty());
        assert!(
            fx.store
                .project_by_root(&fx.base.join("B"))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn a_link_the_user_removed_is_not_recreated_or_followed() {
        let fx = Fx::new();
        fx.proj("A", &["B"]);
        fx.proj("B", &[]);
        fx.add("A");
        fx.follow("A");
        let (a, b) = (fx.add("A"), fx.add("B"));
        assert!(fx.store.unlink_projects(a, b).unwrap());
        fx.proj("B", &["C"]);
        fx.proj("C", &[]);
        let f = fx.follow("A");
        assert_eq!(f.linked, 0);
        assert!(
            f.registered.is_empty(),
            "B is not walked, so C is not found"
        );
        assert!(fx.links().is_empty());
    }

    #[test]
    fn a_manual_link_is_never_dropped_or_changed() {
        let fx = Fx::new();
        fx.proj("A", &["B"]);
        fx.proj("B", &[]);
        fx.proj("X", &[]);
        let (a, x) = (fx.add("A"), fx.add("X"));
        let b = fx.add("B");
        fx.store
            .link_projects(a, x, LinkKind::Manual, None)
            .unwrap();
        fx.store
            .link_projects(a, b, LinkKind::Manual, None)
            .unwrap();
        fx.follow("A");
        fx.proj("A", &[]);
        fx.follow("A");
        assert_eq!(fx.links(), ["A->X:manual", "A->B:manual"]);
    }

    #[test]
    fn a_cycle_ends_and_an_unreadable_manifest_drops_nothing() {
        let fx = Fx::new();
        fx.proj("A", &["B"]);
        fx.proj("B", &["A"]);
        fx.add("A");
        fx.follow("A");
        assert_eq!(fx.links(), ["A->B:auto", "B->A:auto"]);
        std::fs::write(fx.base.join("A/Cargo.toml"), "[dependencies\n").unwrap();
        let f = fx.follow("A");
        assert_eq!(f.dropped, 0);
        assert_eq!(f.warnings.len(), 1, "{:?}", f.warnings);
        assert_eq!(fx.links().len(), 2);
    }

    #[test]
    fn nothing_happens_when_off_or_when_the_root_is_not_registered() {
        let mut fx = Fx::new();
        fx.proj("A", &["B"]);
        fx.proj("B", &[]);
        assert_eq!(fx.follow("A").registered.len(), 0);
        fx.add("A");
        fx.cfg.auto_link_references = false;
        assert_eq!(fx.follow("A").registered.len(), 0);
        assert!(fx.links().is_empty());
    }

    fn locked_refresh(tag: &str) -> (crate::plugin::Runtime, PathBuf, PathBuf) {
        use crate::store::LockWait;
        let dir = tmp_dir(tag);
        let db = dir.join("rtok.db");
        let mut cfg = crate::config::Config::default();
        cfg.rebase_paths(&dir);
        cfg.core.db_path = db.clone();
        cfg.log.path = dir.join("rtok.log");
        let wait = LockWait {
            busy: Duration::from_millis(20),
            ..LockWait::STEADY
        };
        let rt = crate::plugin::Runtime::open_with(cfg, "t940", wait).unwrap();
        (rt, dir, db)
    }

    fn cargo_dep(dir: &Path, name: &str, deps: &[&str]) -> PathBuf {
        let proj = dir.join(name);
        std::fs::create_dir_all(&proj).unwrap();
        let body: String = deps
            .iter()
            .map(|d| format!("{d} = {{ path = \"../{d}\" }}\n"))
            .collect();
        std::fs::write(proj.join("Cargo.toml"), format!("[dependencies]\n{body}")).unwrap();
        proj
    }

    #[test]
    fn a_locked_references_write_waits_out_a_lock_that_ends() {
        use crate::store::{HeldWrite, hold_write_lock};
        let (rt, dir, db) = locked_refresh("follow-lock-ends");
        let a = cargo_dep(&dir, "A", &["B"]);
        cargo_dep(&dir, "B", &[]);
        rt.store.register_project(&a, Origin::Manual).unwrap();
        let holder = hold_write_lock(&db, HeldWrite::Immediate, Duration::from_millis(300));
        let f = refresh_with(&rt, &a, 100, Duration::from_millis(20));
        holder.join().unwrap();
        assert_eq!(f.registered.len(), 1, "{:?}", f.registered);
        assert!(rt.store.project_by_root(&dir.join("B")).unwrap().is_some());
    }

    #[test]
    fn a_locked_references_write_does_not_warn_when_the_lock_stays() {
        use crate::store::{HeldWrite, hold_write_lock};
        let (rt, dir, db) = locked_refresh("follow-lock-stays");
        let a = cargo_dep(&dir, "A", &["B"]);
        cargo_dep(&dir, "B", &[]);
        rt.store.register_project(&a, Origin::Manual).unwrap();
        let holder = hold_write_lock(&db, HeldWrite::Immediate, Duration::from_millis(600));
        let f = refresh_with(&rt, &a, 2, Duration::from_millis(5));
        holder.join().unwrap();
        assert!(f.registered.is_empty(), "{:?}", f.registered);
        let log = std::fs::read_to_string(&rt.config.log.path).unwrap_or_default();
        assert!(
            !log.contains("database is locked"),
            "locked write must back off without warning: {log}"
        );
    }
}
