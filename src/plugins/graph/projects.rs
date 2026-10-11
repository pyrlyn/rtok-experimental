// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok graph projects` — the project registry (T329.1) from the command line, with each
//! project's index status (T329.2) and the links between projects (T329.3). The `project`
//! argument on the other graph commands comes later.

use std::path::Path;

use anyhow::{Result, bail};
use serde::Serialize;

use rtok_plugin_sdk::Ctx;

use super::capability::{self, Capability};
use super::health::score::{self, Score};
use super::{health, index, status};
use crate::plugin::Runtime;
use crate::render::{Col, table};
use crate::store::{LinkKind, Origin, Project, Store};

/// `graph status` numbers for one project; absent for a missing root, which has nothing
/// readable to count.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ProjectIndex {
    rows: i64,
    files: i64,
    pending: usize,
    watch: String,
    pub(super) indexed_at: Option<i64>,
}

/// One outgoing link of a project.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ProjectLink {
    pub to: i32,
    pub name: String,
    pub(super) kind: LinkKind,
    pub(super) reason: Option<String>,
}

/// One registry row as `graph projects` prints it and the `/ws` snapshot carries it.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ProjectRow {
    pub id: i32,
    pub name: String,
    pub(super) root: String,
    pub(super) origin: Origin,
    pub selected: bool,
    pub missing: bool,
    pub state: &'static str,
    created_at: i64,
    last_used_at: i64,
    pub(super) index: Option<ProjectIndex>,
    /// Which graph mode works here, as the last process to answer for it recorded (T329.11);
    /// absent until a request under `lsp` or `auto` has checked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) backend: Option<Capability>,
    /// What the health check raised for this project (T329.17); absent while nothing is wrong.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    alerts: Vec<health::Alert>,
    /// The 0 to 100 health score with its components, reasons and fixes (T329.19).
    pub health: Score,
    /// The lowest score in this project's scope (itself and what it links to); absent while
    /// every member is still on its first index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_health: Option<u8>,
    pub links: Vec<ProjectLink>,
}

impl ProjectRow {
    /// A row with only what the TUI paints, for render tests that must not index a project.
    #[cfg(test)]
    pub(crate) fn fixture(id: i32, name: &str, health: Score, scope_health: Option<u8>) -> Self {
        Self {
            id,
            name: name.into(),
            root: format!("/{name}"),
            origin: Origin::Manual,
            selected: false,
            missing: health.level == score::Level::Missing,
            state: "indexed",
            created_at: 0,
            last_used_at: 0,
            index: None,
            backend: None,
            alerts: Vec::new(),
            health,
            scope_health,
            links: Vec::new(),
        }
    }
}

fn link_rows(store: &Store, from: i32) -> Result<Vec<ProjectLink>> {
    let mut out = Vec::new();
    for l in store
        .project_links()?
        .into_iter()
        .filter(|l| l.from == from)
    {
        let name = store
            .project(l.to)?
            .map_or_else(|| l.to.to_string(), |p| p.display_name().to_string());
        out.push(ProjectLink {
            to: l.to,
            name,
            kind: l.kind,
            reason: l.reason,
        });
    }
    Ok(out)
}

pub(super) fn row(rt: &Runtime, p: Project) -> Result<ProjectRow> {
    let cx = &Ctx::new(rt);
    let missing = p.missing();
    let status = if missing {
        None
    } else {
        Some(status::collect(cx, Path::new(&p.root))?)
    };
    let health = status
        .as_ref()
        .map_or_else(|| score::missing(&p), |st| score::of(rt, &p, st));
    let scope_health = health::reach(&rt.store, &p)
        .ok()
        .and_then(|reached| score::scope_lowest(rt, &reached, &health));
    let index = status.map(|s| ProjectIndex {
        rows: s.rows,
        files: s.files,
        pending: s.pending.len(),
        watch: s.watch,
        indexed_at: s.indexed_at,
    });
    let state = match &index {
        None => "missing",
        Some(i) if i.indexed_at.is_none() && i.rows == 0 => "not indexed",
        Some(i) if i.pending > 0 => "stale",
        Some(_) => "ok",
    };
    let backend = (!missing)
        .then(|| capability::mirrored(cx, Path::new(&p.root)))
        .flatten();
    let alerts = health::mirrored(cx, &p.root);
    Ok(ProjectRow {
        id: p.id,
        name: p.display_name().to_string(),
        root: p.root,
        origin: p.origin,
        selected: p.selected,
        missing,
        state,
        created_at: p.created_at,
        last_used_at: p.last_used_at,
        index,
        backend,
        alerts,
        health,
        scope_health,
        links: link_rows(&rt.store, p.id)?,
    })
}

fn render(rows: &[ProjectRow], json: bool) -> Result<String> {
    if json {
        return Ok(serde_json::to_string_pretty(rows)? + "\n");
    }
    if rows.is_empty() {
        return Ok("no projects; `rtok graph projects add <path>`\n".into());
    }
    let cols = [
        Col::left(1),
        Col::right(2),
        Col::left(4),
        Col::left(6),
        Col::left(5),
        Col::right(4),
        Col::right(5),
        Col::right(7),
        Col::right(5),
        Col::right(6),
        Col::right(5),
        Col::left(4),
    ];
    let head = [
        "", "id", "name", "origin", "state", "rows", "files", "pending", "links", "health",
        "scope", "root",
    ];
    let mut cells = vec![head.map(String::from).to_vec()];
    for r in rows {
        let n = |f: fn(&ProjectIndex) -> String| r.index.as_ref().map_or("-".into(), f);
        cells.push(vec![
            if r.selected { "*" } else { "" }.into(),
            r.id.to_string(),
            r.name.clone(),
            r.origin.as_str().into(),
            r.state.into(),
            n(|i| i.rows.to_string()),
            n(|i| i.files.to_string()),
            n(|i| i.pending.to_string()),
            r.links.len().to_string(),
            r.health.score.map_or("-".into(), |n| n.to_string()),
            r.scope_health.map_or("-".into(), |n| n.to_string()),
            r.root.clone(),
        ]);
    }
    // `table` pads every column, so the free-text root would end each line in spaces.
    let text = table(&cols, &cells);
    Ok(text
        .lines()
        .map(|l| format!("{}\n", l.trim_end()))
        .collect())
}

pub fn rows(rt: &Runtime) -> Result<Vec<ProjectRow>> {
    rt.store
        .projects()?
        .into_iter()
        .map(|p| row(rt, p))
        .collect()
}

fn list(rt: &Runtime, json: bool) -> Result<String> {
    render(&rows(rt)?, json)
}

/// `<id|path>`: a number naming a known project is its id, anything else is a directory.
pub(super) fn resolve(store: &Store, target: &str) -> Result<Project> {
    let by_id = target
        .parse::<i32>()
        .ok()
        .and_then(|id| store.project(id).transpose());
    match by_id {
        Some(p) => Ok(p?),
        None => match store.project_by_root(Path::new(target))? {
            Some(p) => Ok(p),
            None => bail!("no project `{target}`; see `rtok graph projects`"),
        },
    }
}

/// `<to>` and the project it is linked from: `--from`, else the selected project (or the
/// working directory's, which `resolve_project` selects when nothing is).
fn pair(rt: &Runtime, to: &str, from: Option<&str>) -> Result<(Project, Project)> {
    let from = match from {
        Some(f) => resolve(&rt.store, f)?,
        None => match rt.store.resolve_project(&std::env::current_dir()?)?.project {
            Some(p) => p,
            None => {
                bail!("no selected project; pass --from <project> or `rtok graph projects select`")
            }
        },
    };
    Ok((from, resolve(&rt.store, to)?))
}

fn link(rt: &Runtime, from: &Project, to: &Project, reason: Option<&str>) -> Result<bool> {
    rt.store
        .link_projects(from.id, to.id, LinkKind::Manual, reason)
        .map_err(Into::into)
}

fn changes(done: &[(&Project, &Project, bool)], yes: &str, no: &str, json: bool) -> Result<String> {
    if json {
        let rows: Vec<_> = done
            .iter()
            .map(|(a, b, c)| serde_json::json!({ "from": a.id, "to": b.id, "changed": c }))
            .collect();
        return Ok(serde_json::to_string_pretty(&rows)? + "\n");
    }
    Ok(done
        .iter()
        .map(|(a, b, c)| {
            let (a, b) = (a.display_name(), b.display_name());
            if *c {
                format!("{yes} {a} -> {b}\n")
            } else {
                format!("{a} -> {b} {no}\n")
            }
        })
        .collect())
}

/// One project as `list` shows it, so every action answers in the shape the user reads.
fn one(rt: &Runtime, p: Project, json: bool) -> Result<String> {
    let r = row(rt, p)?;
    if json {
        return Ok(serde_json::to_string_pretty(&r)? + "\n");
    }
    render(&[r], false)
}

pub fn run(rt: &Runtime, action: Action, json: bool) -> Result<String> {
    match action {
        Action::List => list(rt, json),
        Action::Add(path) => {
            if !path.is_dir() {
                bail!("{} is not a directory", path.display());
            }
            one(rt, rt.store.register_project(&path, Origin::Manual)?, json)
        }
        Action::Select(target) => {
            let p = resolve(&rt.store, &target)?;
            if p.missing() {
                bail!(
                    "{} no longer exists; remove it or restore the directory",
                    p.root
                );
            }
            rt.store.select_project(p.id)?;
            let p = rt.store.project(p.id)?.unwrap_or(p);
            one(rt, p, json)
        }
        Action::Link {
            to,
            from,
            both,
            reason,
        } => {
            let (a, b) = pair(rt, &to, from.as_deref())?;
            for p in [&a, &b] {
                if p.missing() {
                    bail!("{} no longer exists; it cannot be linked", p.root);
                }
            }
            let mut done = vec![(&a, &b, link(rt, &a, &b, reason.as_deref())?)];
            if both {
                done.push((&b, &a, link(rt, &b, &a, reason.as_deref())?));
            }
            // Linking a project that was never indexed starts indexing it, so the scope answers
            // from it right away.
            if row(rt, b.clone())?.state == "not indexed" {
                index::run(&Ctx::new(rt), Path::new(&b.root), false)?;
            }
            changes(&done, "linked", "already linked", json)
        }
        Action::Unlink { to, from, both } => {
            let (a, b) = pair(rt, &to, from.as_deref())?;
            let mut done = vec![(&a, &b, rt.store.unlink_projects(a.id, b.id)?)];
            if both {
                done.push((&b, &a, rt.store.unlink_projects(b.id, a.id)?));
            }
            changes(&done, "unlinked", "was not linked", json)
        }
        Action::Remove(target) => {
            let p = resolve(&rt.store, &target)?;
            rt.store.remove_project(p.id)?;
            if json {
                return Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "removed": p.id,
                    "root": p.root,
                }))? + "\n");
            }
            Ok(format!(
                "removed {} (index rows dropped, files untouched)\n",
                p.root
            ))
        }
    }
}

/// What `Remove` or `Unlink` would drop, with nothing written (T416.4). Any other action is
/// refused: only the two that delete rows have a preview.
pub fn preview(rt: &Runtime, action: Action, json: bool) -> Result<String> {
    match action {
        Action::Unlink { to, from, both } => {
            let (a, b) = pair(rt, &to, from.as_deref())?;
            let links = rt.store.project_links()?;
            let linked =
                |x: &Project, y: &Project| links.iter().any(|l| l.from == x.id && l.to == y.id);
            let mut done = vec![(&a, &b, linked(&a, &b))];
            if both {
                done.push((&b, &a, linked(&b, &a)));
            }
            changes(&done, "would unlink", "is not linked", json)
        }
        Action::Remove(target) => {
            let p = resolve(&rt.store, &target)?;
            let links = rt.store.project_links()?;
            let out = links.iter().filter(|l| l.from == p.id).count();
            let incoming = links.iter().filter(|l| l.to == p.id).count();
            let index = row(rt, p.clone())?.index;
            let (rows, files) = index.as_ref().map_or((0, 0), |i| (i.rows, i.files));
            if json {
                return Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "would_remove": p.id,
                    "root": p.root,
                    "index_rows": rows,
                    "indexed_files": files,
                    "links_out": out,
                    "links_in": incoming,
                }))? + "\n");
            }
            Ok(format!(
                "would remove {} (id {})\n  index rows: {rows} over {files} files\n  links: {out} out, {incoming} in\n  files untouched; nothing removed (dry run)\n",
                p.root, p.id
            ))
        }
        _ => bail!("only `remove` and `unlink` have a dry run"),
    }
}

pub enum Action {
    Link {
        to: String,
        from: Option<String>,
        both: bool,
        reason: Option<String>,
    },
    Unlink {
        to: String,
        from: Option<String>,
        both: bool,
    },
    List,
    Add(std::path::PathBuf),
    Select(String),
    Remove(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_registered_says_how_to_add() {
        let rows: Vec<ProjectRow> = Vec::new();
        assert!(render(&rows, false).unwrap().contains("graph projects add"));
        assert_eq!(render(&rows, true).unwrap(), "[]\n");
    }

    #[test]
    fn resolve_prefers_a_known_id_then_a_path() {
        let store = Store::open_in_memory().unwrap();
        let dir = std::env::temp_dir().join(format!("rtok-gp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = store.register_project(&dir, Origin::Manual).unwrap();
        assert_eq!(resolve(&store, &p.id.to_string()).unwrap().id, p.id);
        assert_eq!(resolve(&store, dir.to_str().unwrap()).unwrap().id, p.id);
        assert!(resolve(&store, "9999").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T329.11: a process that never asked the server still shows what another decided.
    #[test]
    fn rows_carry_the_mirrored_capability_record() {
        let (mut c, dir) = crate::testutil::config("t32911-rows");
        c.plugins.graph.backend = "auto".into();
        let rt = Runtime::open(c, "t32911-rows").unwrap();
        let root = dir.join("proj");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("go.mod"), "module x\n").unwrap();
        rt.store.register_project(&root, Origin::Manual).unwrap();
        let none = serde_json::to_value(rows(&rt).unwrap()).unwrap();
        assert!(none[0].get("backend").is_none(), "{none}");

        let cx = Ctx::new(&rt);
        let out = super::super::lsp_or_tags(
            &cx,
            &root,
            "symbol",
            &[],
            || panic!("no server for go"),
            || Ok("tags answer".into()),
        );
        assert_eq!(out.unwrap(), "(tags)\ntags answer");
        let shown = serde_json::to_value(rows(&rt).unwrap()).unwrap();
        let rec = &shown[0]["backend"];
        assert_eq!(rec["backend"], "tags");
        assert_eq!(rec["language"], "go");
        assert_eq!(rec["server"], false);
        assert_eq!(rec["config"], "auto");
        assert!(rec["checked_at"].as_i64().unwrap() > 0, "{rec}");
        assert!(rec["next_probe_at"].as_i64().unwrap() > rec["checked_at"].as_i64().unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }
}
