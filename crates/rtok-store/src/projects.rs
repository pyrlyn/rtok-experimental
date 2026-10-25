// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T329.1: `projects` — the graph's project registry. A project is a directory rtok indexes
//! as one unit, identified by its canonical root; the symbol index stays keyed by that root,
//! so switching projects never re-indexes or mixes rows. No CLI, links or page yet.

use std::collections::HashSet;
use std::path::Path;

use crate::Result;
use diesel::prelude::*;
use serde::Serialize;

use super::Store;
use super::schema::{calls, extractor, file_rank, projects, sessions, symbol_stale, symbols};
use super::unixepoch;

/// Host events that mean a session used a tool. SessionStart/SessionEnd alone do not:
/// probe apps fire only those and must not keep a registry row.
const TOOL_HOOKS: &[&str] = &["PreToolUse", "PostToolUse", "AfterMCPExecution"];

/// Canonical absolute path as one string: the index key of a root, and the match key of a
/// file for `mark_symbols_stale` (T8.3). Rows are scoped to it so one store holds many repos.
pub fn canon_root(p: &Path) -> String {
    fs_kit::path::canonicalize(p)
        .unwrap_or_else(|_| p.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
}

/// How a project got into the registry (T329 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Manual,
    Session,
    Worktree,
    Mcp,
    Reference,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Session => "session",
            Self::Worktree => "worktree",
            Self::Mcp => "mcp",
            Self::Reference => "reference",
        }
    }

    // The column has a CHECK for these five, so an unknown value means a hand-edited db;
    // `manual` keeps such a row listable rather than failing the whole registry.
    fn parse(s: &str) -> Self {
        match s {
            "session" => Self::Session,
            "worktree" => Self::Worktree,
            "mcp" => Self::Mcp,
            "reference" => Self::Reference,
            _ => Self::Manual,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Project {
    pub id: i32,
    pub root: String,
    /// The user's rename; `None` shows the directory name.
    pub name: Option<String>,
    pub origin: Origin,
    pub created_at: i64,
    pub last_used_at: i64,
    pub selected: bool,
}

impl Project {
    pub fn display_name(&self) -> &str {
        self.name
            .as_deref()
            .or_else(|| Path::new(&self.root).file_name().and_then(|n| n.to_str()))
            .unwrap_or(&self.root)
    }

    /// The root no longer exists. Read from the filesystem each time, never stored, so a
    /// directory that comes back stops being missing without a write.
    pub fn missing(&self) -> bool {
        !Path::new(&self.root).is_dir()
    }
}

type Row = (i32, String, Option<String>, String, i64, i64, i32);

impl From<Row> for Project {
    fn from((id, root, name, origin, created_at, last_used_at, selected): Row) -> Self {
        Self {
            id,
            root,
            name,
            origin: Origin::parse(&origin),
            created_at,
            last_used_at,
            selected: selected == 1,
        }
    }
}

/// The project to answer for, and whether it replaced a stored selection that is gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub project: Option<Project>,
    /// A stored selection was missing and the working directory's project took its place.
    pub fell_back: bool,
}

impl Store {
    /// Register `path` (canonicalised), or only refresh last-used when its root is already
    /// known: two spellings of one directory are one project and the first origin stands.
    pub fn register_project(&self, path: &Path, origin: Origin) -> Result<Project> {
        let root = canon_root(path);
        let mut conn = self.lock()?;
        diesel::insert_into(projects::table)
            .values((
                projects::root.eq(&root),
                projects::origin.eq(origin.as_str()),
            ))
            .on_conflict(projects::root)
            .do_update()
            .set(projects::last_used_at.eq(unixepoch().assume_not_null()))
            .execute(&mut *conn)?;
        Ok(projects::table
            .filter(projects::root.eq(&root))
            .first::<Row>(&mut *conn)?
            .into())
    }

    /// T329.6: register a directory rtok saw in use (`[plugins.graph] auto_add_projects`). The
    /// filesystem root, `$HOME` and a path that is not a directory are skipped, as no index may
    /// use them as a root. `name` (a worktree's branch) labels only the row this call created,
    /// so a rename or a manual add of the same root is never overwritten.
    pub fn auto_add_project(&self, path: &Path, origin: Origin, name: Option<&str>) -> Result<()> {
        if !path.is_dir() || crate::is_unwalkable_root(path, std::env::home_dir().as_deref()) {
            return Ok(());
        }
        let project = self.register_project(path, origin)?;
        // Without a name there is nothing to set: the update would write NULL over NULL, and
        // every SessionStart paid a write lock for it.
        if project.origin == origin
            && project.name.is_none()
            && name.is_some_and(|n| !n.trim().is_empty())
        {
            self.rename_project(project.id, name)?;
        }
        Ok(())
    }

    /// Drop session-origin projects whose sessions never recorded a tool. Probe apps
    /// register on SessionStart, then `graph/references` keeps hitting a locked write.
    pub fn drop_toolless_session_projects(&self) -> Result<u64> {
        let used = self.tool_session_roots()?;
        let ids: Vec<i32> = self
            .projects()?
            .into_iter()
            .filter(|p| p.origin == Origin::Session && !used.contains(&p.root))
            .map(|p| p.id)
            .collect();
        let mut n = 0;
        for id in ids {
            if self.remove_project(id)? {
                n += 1;
            }
        }
        Ok(n)
    }

    fn tool_session_roots(&self) -> Result<HashSet<String>> {
        let mut conn = self.lock()?;
        let cwds: Vec<Option<String>> = sessions::table
            .inner_join(calls::table)
            .filter(calls::name.eq_any(TOOL_HOOKS).or(calls::surface.eq("mcp")))
            .select(sessions::cwd)
            .distinct()
            .load(&mut *conn)?;
        Ok(cwds
            .into_iter()
            .flatten()
            .map(|c| canon_root(Path::new(&c)))
            .collect())
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        let mut conn = self.lock()?;
        let rows: Vec<Row> = projects::table.order(projects::id).load(&mut *conn)?;
        Ok(rows.into_iter().map(Project::from).collect())
    }

    pub fn project(&self, id: i32) -> Result<Option<Project>> {
        let mut conn = self.lock()?;
        let row: Option<Row> = projects::table.find(id).first(&mut *conn).optional()?;
        Ok(row.map(Project::from))
    }

    pub fn project_by_root(&self, path: &Path) -> Result<Option<Project>> {
        let mut conn = self.lock()?;
        let row: Option<Row> = projects::table
            .filter(projects::root.eq(canon_root(path)))
            .first(&mut *conn)
            .optional()?;
        Ok(row.map(Project::from))
    }

    /// Set (or, with `None` or a blank name, clear) the display name. False for an unknown id.
    pub fn rename_project(&self, id: i32, name: Option<&str>) -> Result<bool> {
        let name = name.map(str::trim).filter(|n| !n.is_empty());
        let mut conn = self.lock()?;
        let n = diesel::update(projects::table.find(id))
            .set(projects::name.eq(name))
            .execute(&mut *conn)?;
        Ok(n == 1)
    }

    pub fn selected_project(&self) -> Result<Option<Project>> {
        let mut conn = self.lock()?;
        let row: Option<Row> = projects::table
            .filter(projects::selected.eq(1))
            .first(&mut *conn)
            .optional()?;
        Ok(row.map(Project::from))
    }

    /// Make `id` the one selected project. False for an unknown id, leaving the selection alone.
    pub fn select_project(&self, id: i32) -> Result<bool> {
        let mut conn = self.lock()?;
        Ok(
            conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
                if diesel::select(diesel::dsl::exists(projects::table.find(id))).get_result(conn)? {
                    diesel::update(projects::table.filter(projects::selected.eq(1)))
                        .set(projects::selected.eq(0))
                        .execute(conn)?;
                    diesel::update(projects::table.find(id))
                        .set((
                            projects::selected.eq(1),
                            projects::last_used_at.eq(unixepoch().assume_not_null()),
                        ))
                        .execute(conn)?;
                    return Ok(true);
                }
                Ok(false)
            })?,
        )
    }

    /// The project the page (and, later, the CLI) answers for. A stored selection stands while
    /// its root exists. Without one, or when it is gone, the project registered for `cwd` is
    /// selected, if there is one; `fell_back` then tells the caller to show a notice.
    pub fn resolve_project(&self, cwd: &Path) -> Result<Resolved> {
        let stored = self.selected_project()?;
        if let Some(p) = stored.as_ref().filter(|p| !p.missing()) {
            return Ok(Resolved {
                project: Some(p.clone()),
                fell_back: false,
            });
        }
        let Some(here) = self.project_by_root(cwd)?.filter(|p| !p.missing()) else {
            return Ok(Resolved {
                project: stored,
                fell_back: false,
            });
        };
        self.select_project(here.id)?;
        Ok(Resolved {
            fell_back: stored.is_some(),
            project: self.project(here.id)?,
        })
    }

    /// Drop the project's index rows and its registry row; its files are never touched.
    /// False for an unknown id.
    pub fn remove_project(&self, id: i32) -> Result<bool> {
        let mut conn = self.lock()?;
        Ok(
            conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
                let root: Option<String> = projects::table
                    .find(id)
                    .select(projects::root)
                    .first(conn)
                    .optional()?;
                let Some(root) = root else {
                    return Ok(false);
                };
                diesel::delete(symbols::table.filter(symbols::root.eq(&root))).execute(conn)?;
                diesel::delete(extractor::table.filter(extractor::root.eq(&root))).execute(conn)?;
                diesel::delete(file_rank::table.find(&root)).execute(conn)?;
                diesel::delete(symbol_stale::table.filter(symbol_stale::root.eq(&root)))
                    .execute(conn)?;
                diesel::delete(projects::table.find(id)).execute(conn)?;
                Ok(true)
            })?,
        )
    }
}

#[cfg(test)]
pub(super) mod fixture {
    use std::path::PathBuf;

    /// A scratch directory tree that removes itself.
    pub struct Dir(pub PathBuf);

    impl Dir {
        pub fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("rtok-proj-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        pub fn sub(&self, name: &str) -> PathBuf {
            let p = self.0.join(name);
            std::fs::create_dir_all(&p).unwrap();
            p
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::Dir;
    use super::*;

    fn index_row(store: &Store, root: &str) {
        let row = rtok_plugin_sdk::SymbolRow::new("main", "function", 1, true, 1, "");
        for path in ["a.rs", "stale.rs"] {
            store
                .replace_symbols(root, path, "sha", (0, 0), std::slice::from_ref(&row))
                .unwrap();
        }
        store.set_extractor_fingerprint(root, "fp").unwrap();
        store.mark_symbols_stale_in(root, "stale.rs").unwrap();
    }

    // `/` is the filesystem root only on Unix.
    #[cfg(unix)]
    #[test]
    fn auto_add_skips_unwalkable_roots_and_never_renames_a_known_project() {
        let dir = Dir::new("auto-add");
        let store = Store::open_in_memory().unwrap();
        store
            .auto_add_project(Path::new("/"), Origin::Session, None)
            .unwrap();
        store
            .auto_add_project(&dir.0.join("gone"), Origin::Session, None)
            .unwrap();
        assert!(store.projects().unwrap().is_empty());

        let manual = store
            .register_project(&dir.sub("a"), Origin::Manual)
            .unwrap();
        store
            .auto_add_project(&dir.sub("a"), Origin::Worktree, Some("br"))
            .unwrap();
        assert_eq!(store.project(manual.id).unwrap().unwrap().name, None);

        store
            .auto_add_project(&dir.sub("b"), Origin::Worktree, Some("br"))
            .unwrap();
        let b = store.project_by_root(&dir.sub("b")).unwrap().unwrap();
        assert_eq!((b.origin, b.display_name()), (Origin::Worktree, "br"));
        store.rename_project(b.id, Some("mine")).unwrap();
        store
            .auto_add_project(&dir.sub("b"), Origin::Worktree, Some("br"))
            .unwrap();
        assert_eq!(
            store.project(b.id).unwrap().unwrap().name.as_deref(),
            Some("mine")
        );
    }

    // Windows needs a privilege to create a symlink, and `std::os::unix` does not exist there.
    #[cfg(unix)]
    #[test]
    fn spellings_of_one_directory_are_one_project() {
        let dir = Dir::new("dedup");
        let real = dir.sub("real");
        std::os::unix::fs::symlink(&real, dir.0.join("link")).unwrap();
        let store = Store::open_in_memory().unwrap();
        let first = store.register_project(&real, Origin::Session).unwrap();
        let dotted = dir.0.join("real/../real");
        let by_dots = store.register_project(&dotted, Origin::Manual).unwrap();
        let by_link = store
            .register_project(&dir.0.join("link"), Origin::Reference)
            .unwrap();
        assert_eq!((by_dots.id, by_link.id), (first.id, first.id));
        assert_eq!(by_link.origin, Origin::Session, "the first origin stands");
        assert_eq!(store.projects().unwrap().len(), 1);
        assert_eq!(first.display_name(), "real");
    }

    #[test]
    fn one_project_is_selected_and_an_unknown_id_changes_nothing() {
        let dir = Dir::new("select");
        let store = Store::open_in_memory().unwrap();
        let a = store
            .register_project(&dir.sub("a"), Origin::Manual)
            .unwrap();
        let b = store
            .register_project(&dir.sub("b"), Origin::Manual)
            .unwrap();
        assert_eq!(store.selected_project().unwrap(), None);
        assert!(store.select_project(a.id).unwrap());
        assert!(store.select_project(b.id).unwrap());
        assert!(!store.select_project(9999).unwrap());
        let selected: Vec<i32> = store
            .projects()
            .unwrap()
            .iter()
            .filter(|p| p.selected)
            .map(|p| p.id)
            .collect();
        assert_eq!(selected, [b.id]);
    }

    #[test]
    fn rename_sets_and_clears_the_display_name() {
        let dir = Dir::new("rename");
        let store = Store::open_in_memory().unwrap();
        let p = store
            .register_project(&dir.sub("a"), Origin::Manual)
            .unwrap();
        assert!(store.rename_project(p.id, Some("  api  ")).unwrap());
        assert_eq!(store.project(p.id).unwrap().unwrap().display_name(), "api");
        assert!(store.rename_project(p.id, Some("   ")).unwrap());
        assert_eq!(store.project(p.id).unwrap().unwrap().display_name(), "a");
        assert!(!store.rename_project(9999, Some("x")).unwrap());
    }

    #[test]
    fn a_deleted_root_is_missing_and_stays_listed() {
        let dir = Dir::new("missing");
        let store = Store::open_in_memory().unwrap();
        let root = dir.sub("gone");
        let p = store.register_project(&root, Origin::Manual).unwrap();
        assert!(!p.missing());
        std::fs::remove_dir_all(&root).unwrap();
        assert!(store.project(p.id).unwrap().unwrap().missing());
        assert_eq!(store.projects().unwrap().len(), 1);
    }

    #[test]
    fn a_missing_selection_falls_back_to_the_working_directory() {
        let dir = Dir::new("resolve");
        let store = Store::open_in_memory().unwrap();
        let (here, gone) = (dir.sub("here"), dir.sub("gone"));
        let h = store.register_project(&here, Origin::Manual).unwrap();
        let g = store.register_project(&gone, Origin::Manual).unwrap();

        let none = store.resolve_project(&dir.sub("elsewhere")).unwrap();
        assert_eq!(
            none,
            Resolved {
                project: None,
                fell_back: false
            }
        );
        let first = store.resolve_project(&here).unwrap();
        assert_eq!(first.project.map(|p| p.id), Some(h.id));
        assert!(
            !first.fell_back,
            "nothing was stored, so nothing was replaced"
        );

        store.select_project(g.id).unwrap();
        assert_eq!(
            store.resolve_project(&here).unwrap().project.unwrap().id,
            g.id
        );
        std::fs::remove_dir_all(&gone).unwrap();
        let back = store.resolve_project(&here).unwrap();
        assert_eq!(
            (back.project.map(|p| p.id), back.fell_back),
            (Some(h.id), true)
        );
        assert_eq!(store.selected_project().unwrap().unwrap().id, h.id);

        let lost = store.resolve_project(&dir.sub("elsewhere")).unwrap();
        assert_eq!(
            lost.project.map(|p| p.id),
            Some(h.id),
            "a live selection stands"
        );
    }

    #[test]
    fn remove_drops_only_that_projects_index() {
        let dir = Dir::new("remove");
        let store = Store::open_in_memory().unwrap();
        let a = store
            .register_project(&dir.sub("a"), Origin::Manual)
            .unwrap();
        let b = store
            .register_project(&dir.sub("b"), Origin::Manual)
            .unwrap();
        index_row(&store, &a.root);
        index_row(&store, &b.root);
        store.file_rank_put(&a.root, "{}").unwrap();
        store.file_rank_put(&b.root, "{}").unwrap();
        store.select_project(a.id).unwrap();
        assert!(store.remove_project(a.id).unwrap());
        assert!(!store.remove_project(a.id).unwrap());
        assert_eq!(store.symbol_count(&a.root).unwrap(), 0);
        assert_eq!(store.symbol_count(&b.root).unwrap(), 1);
        assert!(store.symbol_stale_paths(&a.root).unwrap().is_empty());
        assert_eq!(store.symbol_stale_paths(&b.root).unwrap(), ["stale.rs"]);
        assert!(store.extractor_fingerprint(&a.root).unwrap().is_none());
        assert!(store.extractor_fingerprint(&b.root).unwrap().is_some());
        assert!(store.file_rank_get(&a.root).unwrap().is_none());
        assert!(store.file_rank_get(&b.root).unwrap().is_some());
        assert_eq!(store.selected_project().unwrap(), None);
        assert!(dir.0.join("a").is_dir(), "the files are never touched");
    }

    #[test]
    fn session_projects_with_no_tool_are_dropped() {
        let dir = Dir::new("toolless-session");
        let store = Store::open_in_memory().unwrap();
        let probe = store
            .register_project(&dir.sub("Probe"), Origin::Session)
            .unwrap();
        let github = store
            .register_project(&dir.sub("GitHub"), Origin::Session)
            .unwrap();
        let used = store
            .register_project(&dir.sub("app"), Origin::Session)
            .unwrap();
        let mcp = store
            .register_project(&dir.sub("mcp-root"), Origin::Session)
            .unwrap();
        let manual = store
            .register_project(&dir.sub("manual"), Origin::Manual)
            .unwrap();
        let reference = store
            .register_project(&dir.sub("dep"), Origin::Reference)
            .unwrap();

        store
            .upsert_session("probe", None, None, Some(&probe.root), Some("hook"))
            .unwrap();
        store
            .insert_call(
                "probe",
                "hook",
                "hook",
                None,
                None,
                None,
                None,
                Some("SessionStart"),
            )
            .unwrap();
        store
            .insert_call(
                "probe",
                "hook",
                "hook",
                None,
                None,
                None,
                None,
                Some("SessionEnd"),
            )
            .unwrap();

        store
            .upsert_session("gh", None, None, Some(&github.root), Some("hook"))
            .unwrap();
        store
            .insert_call(
                "gh",
                "hook",
                "hook",
                None,
                None,
                None,
                None,
                Some("SessionStart"),
            )
            .unwrap();

        store
            .upsert_session("app", None, None, Some(&used.root), Some("hook"))
            .unwrap();
        store
            .insert_call(
                "app",
                "hook",
                "hook",
                None,
                None,
                None,
                None,
                Some("PostToolUse"),
            )
            .unwrap();

        store
            .upsert_session("mcp", None, None, Some(&mcp.root), Some("mcp"))
            .unwrap();
        store
            .insert_call(
                "mcp",
                "mcp",
                "symbol",
                None,
                None,
                None,
                None,
                Some("symbol"),
            )
            .unwrap();

        assert_eq!(store.drop_toolless_session_projects().unwrap(), 2);
        let roots: Vec<_> = store
            .projects()
            .unwrap()
            .into_iter()
            .map(|p| p.root)
            .collect();
        assert!(roots.contains(&used.root), "{roots:?}");
        assert!(roots.contains(&mcp.root), "{roots:?}");
        assert!(roots.contains(&manual.root), "{roots:?}");
        assert!(roots.contains(&reference.root), "{roots:?}");
        assert!(!roots.contains(&probe.root), "{roots:?}");
        assert!(!roots.contains(&github.root), "{roots:?}");
        assert_eq!(store.drop_toolless_session_projects().unwrap(), 0);
    }
}
