// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok worktree list` (T151): what every worktree costs on disk, split into source and
//! tagged build cache, plus the orphans git cannot see. Read-only — it deletes nothing.

use std::collections::{BTreeSet, HashSet};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::{Entry, inventory_with, par_map};
use crate::render::{Col, duration, table};
use change_preview::human_bytes;

/// First line of a valid `CACHEDIR.TAG` (<https://bford.info/cachedir/>); cargo writes one
/// into `target/`. The tag, not a directory name, is what makes bytes "cache".
const CACHEDIR_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

pub fn is_cache_dir(dir: &Path) -> bool {
    std::fs::read(dir.join("CACHEDIR.TAG")).is_ok_and(|tag| tag.starts_with(CACHEDIR_SIGNATURE))
}

/// Logical bytes: an APFS clone or a hard link counts in full, so the sum over worktrees
/// can exceed what deleting them would free.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Usage {
    pub source: u64,
    pub cache: u64,
    pub modified: Option<SystemTime>,
    /// Every tagged cache root under the worktree; `cache` is their sum.
    pub caches: Vec<Cache>,
    /// A directory, entry or mtime could not be read (T656): sizes and `modified` are lower
    /// bounds, since the newest file may be the one the walk missed.
    pub incomplete: bool,
}

/// One tagged cache root: the unit `rtok worktree clean` (T152) deletes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cache {
    pub path: PathBuf,
    pub bytes: u64,
    pub modified: Option<SystemTime>,
    /// [`Usage::incomplete`] for this cache root alone.
    pub incomplete: bool,
}

pub fn usage(dir: &Path) -> Usage {
    let mut total = Usage::default();
    let _ = walk(dir, true, false, None, &mut total, None, &|_| false);
    total
}

/// [`usage`] without the tagged cache roots (T936): they hold nearly every file of a built
/// worktree, so skipping them turns a minutes-long walk into one over the sources. Cache bytes
/// are left at zero and `modified` is the newest source file, as for [`newest_until`].
pub fn usage_sources(dir: &Path) -> Usage {
    let mut total = Usage::default();
    let _ = walk(dir, true, true, None, &mut total, None, &|_| false);
    total
}

/// [`usage`] that stops at `deadline`; the flag says the walk was cut short, so the result is
/// a lower bound. A cut walk's mtimes are incomplete: a newer file may be the one not visited,
/// so a caller that decides "idle" from them must keep the cache.
pub fn usage_until(dir: &Path, deadline: std::time::Instant) -> (Usage, bool) {
    let mut total = Usage::default();
    let cut = walk(dir, true, false, None, &mut total, Some(deadline), &|_| {
        false
    });
    (total, cut.is_break())
}

/// The newest mtime of `dir`'s source files — tagged cache roots left out (T948), since a
/// cargo or rust-analyzer build rewrites `target/` without anyone touching the work — except
/// that the walk ends at the first file whose mtime satisfies `enough`. `gc` only asks whether
/// a worktree was touched within its idle window, which one recent file answers; `enough` must
/// hold for every later mtime too, so that it holds for the newest exactly when it holds for
/// this answer. `None` when the walk could not read everything and found no file that settles it.
pub fn newest_until(dir: &Path, enough: impl Fn(SystemTime) -> bool) -> Option<SystemTime> {
    let mut total = Usage::default();
    let settled = walk(dir, true, true, None, &mut total, None, &enough).is_break();
    total.modified.filter(|_| settled || !total.incomplete)
}

/// Records a read the walk could not make. A path gone mid-walk is not missing evidence:
/// nothing is there to be newer.
fn missed(total: &mut Usage, cache: Option<usize>, err: Option<&std::io::Error>) {
    if err.is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
        return;
    }
    total.incomplete = true;
    if let Some(i) = cache {
        total.caches[i].incomplete = true;
    }
}

/// `cache` indexes `total.caches` once the walk is inside a tagged root. The listing in hand
/// answers "is there a `.git` or a `CACHEDIR.TAG` here", so most directories cost one
/// `read_dir` and no probes; only a name that is present gets checked on disk. `sources_only`
/// skips every tagged cache root.
fn walk(
    dir: &Path,
    root: bool,
    sources_only: bool,
    cache: Option<usize>,
    total: &mut Usage,
    deadline: Option<std::time::Instant>,
    enough: &dyn Fn(SystemTime) -> bool,
) -> ControlFlow<()> {
    // `enough` only sees files that have an mtime, so a tree of directories would never stop.
    if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
        return ControlFlow::Break(());
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            missed(total, cache, Some(&e));
            return ControlFlow::Continue(());
        }
    };
    let mut entries_ok = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => entries_ok.push(entry),
            Err(e) => missed(total, cache, Some(&e)),
        }
    }
    let entries = entries_ok;
    let (mut git, mut tag) = (false, false);
    for entry in &entries {
        let name = entry.file_name();
        git |= name == ".git";
        tag |= name == "CACHEDIR.TAG";
    }
    // A nested checkout (another worktree, a submodule) is its own row.
    if !root && git && dir.join(".git").exists() {
        return ControlFlow::Continue(());
    }
    let tagged = cache.is_none() && tag && is_cache_dir(dir);
    if tagged && sources_only {
        return ControlFlow::Continue(());
    }
    let cache = cache.or_else(|| {
        tagged.then(|| {
            let root = Cache {
                path: dir.to_path_buf(),
                bytes: 0,
                modified: None,
                incomplete: false,
            };
            total.caches.push(root);
            total.caches.len() - 1
        })
    });
    for entry in entries {
        // Neither the listing's file type nor `DirEntry::metadata` follows symlinks: a link
        // costs its own length.
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(e) => {
                missed(total, cache, Some(&e));
                continue;
            }
        };
        if kind.is_dir() {
            walk(
                &entry.path(),
                false,
                sources_only,
                cache,
                total,
                deadline,
                enough,
            )?;
            continue;
        }
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(e) => {
                missed(total, cache, Some(&e));
                continue;
            }
        };
        let modified = meta.modified().ok();
        if modified.is_none() {
            missed(total, cache, None);
        }
        match cache {
            Some(i) => {
                let root = &mut total.caches[i];
                root.bytes += meta.len();
                root.modified = root.modified.max(modified);
                total.cache += meta.len();
            }
            None => total.source += meta.len(),
        }
        total.modified = total.modified.max(modified);
        if modified.is_some_and(enough) {
            return ControlFlow::Break(());
        }
    }
    ControlFlow::Continue(())
}

/// Directories that look like a linked worktree but that git does not list: their `.git`
/// *file* names a `…/worktrees/<id>` admin directory that no longer exists — the
/// repository moved, or the record was pruned. Looked for next to the known worktrees
/// and in `<main>/.claude/worktrees`.
/// `listed` is the path of every worktree git lists, the main checkout first.
pub fn orphans(listed: &[PathBuf]) -> Vec<PathBuf> {
    let real = |p: &Path| crate::fs::canon(p);
    let known: HashSet<PathBuf> = listed.iter().map(|p| real(p)).collect();
    let mut parents: BTreeSet<PathBuf> = listed
        .iter()
        .skip(1)
        .filter_map(|p| p.parent().map(real))
        .collect();
    if let Some(main) = listed.first() {
        parents.insert(real(&main.join(".claude/worktrees")));
    }
    let children = parents
        .iter()
        .flat_map(|p| std::fs::read_dir(p).into_iter().flatten().flatten());
    children
        .map(|child| real(&child.path()))
        .filter(|dir| !known.contains(dir) && is_orphan(dir))
        .collect()
}

fn is_orphan(dir: &Path) -> bool {
    // A `.git` directory (a repository of its own) fails to read as a file.
    let Ok(dot_git) = std::fs::read_to_string(dir.join(".git")) else {
        return false;
    };
    let Some(admin) = dot_git.trim().strip_prefix("gitdir: ") else {
        return false;
    };
    // A relative `gitdir:` (`worktree.useRelativePaths`) resolves against the worktree.
    let admin = dir.join(admin);
    admin.parent().is_some_and(|p| p.ends_with("worktrees")) && !admin.exists()
}

#[derive(Debug, Serialize)]
pub struct Row {
    pub path: PathBuf,
    pub branch: Option<String>,
    /// From the lock reason; `None` with `locked` means the owner is unknown.
    pub owner: Option<String>,
    pub locked: bool,
    /// T150's state, or `orphan`.
    pub state: &'static str,
    /// Which tool made the worktree (T289): `main`, a host's pool, or `other`.
    pub origin: &'static str,
    /// The rtok agent (T282) the worktree is bound to: the lock's, else a claim row (T285).
    pub agent: Option<Bound>,
    /// The newest session the hooks saw working here (T154); never set on the main checkout.
    pub session: Option<Seen>,
    pub source_bytes: u64,
    /// `None` when the build caches were not walked (T936); see [`rows_sized`].
    pub cache_bytes: Option<u64>,
    pub modified_unix: Option<u64>,
}

/// T285: a bound agent as the store knows it. `host` is `None` and `state` `unknown` when
/// the store has no such agent (a lost store; the lock survives it).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Bound {
    pub id: String,
    pub host: Option<String>,
    /// `live`, `idle`, `ended` or `unknown`.
    pub state: &'static str,
}

impl Bound {
    fn new(id: String) -> Self {
        Self {
            id,
            host: None,
            state: "unknown",
        }
    }
}

/// Fill [`Row::agent`] from the store: an unlocked worktree's open claim row, then every
/// bound agent's host and state — `live` within `[agents] idle`, else `ended` or `idle`.
pub fn bind(rows: &mut [Row], store: &crate::store::Store, idle: &str) -> anyhow::Result<()> {
    let real = |p: &Path| crate::fs::canon(p);
    let claims = store.open_worktree_claims()?;
    let live: HashSet<String> = store.live_agents(idle)?.into_iter().map(|a| a.id).collect();
    for row in rows.iter_mut().filter(|r| r.state != "main") {
        if row.agent.is_none() && !row.locked {
            let dir = real(&row.path);
            let claim = claims
                .iter()
                .find(|(p, _)| crate::fs::same_path(&real(Path::new(p)), &dir));
            row.agent = claim.map(|(_, id)| Bound::new(id.clone()));
        }
        let Some(bound) = row.agent.as_mut() else {
            continue;
        };
        let Some(detail) = store.agent_detail(&bound.id)? else {
            continue;
        };
        bound.host = Some(detail.host);
        bound.state = match detail.ended_at {
            _ if live.contains(&bound.id) => "live",
            Some(_) => "ended",
            None => "idle",
        };
    }
    Ok(())
}

/// Ownership without agent discipline: every hook upserts `sessions.cwd`, so the store knows
/// who worked in a worktree even when nobody wrote a lock reason. The lock reason still wins.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Seen {
    pub session: String,
    pub host: Option<String>,
    pub seen_unix: i64,
    /// No `SessionEnd` recorded — the host may simply never send one.
    pub live: bool,
}

impl Row {
    fn new(path: PathBuf, state: &'static str, sized: bool) -> Self {
        let used = if sized {
            usage(&path)
        } else {
            usage_sources(&path)
        };
        let unix = |t: SystemTime| t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs());
        let origin = if state == "main" {
            "main"
        } else {
            super::origin::of(&path)
        };
        Self {
            path,
            branch: None,
            owner: None,
            locked: false,
            state,
            origin,
            agent: None,
            session: None,
            source_bytes: used.source,
            cache_bytes: sized.then_some(used.cache),
            modified_unix: used.modified.and_then(unix),
        }
    }
}

/// Fill [`Row::session`] from the store's sessions: for each linked worktree the newest
/// session whose `cwd` is the worktree or a directory under it (both canonicalised, so
/// `/tmp` and `/private/tmp` agree). The main checkout belongs to nobody.
pub fn attribute(rows: &mut [Row], sessions: &[crate::store::SessionSeen]) {
    let real = |p: &Path| crate::fs::canon(p);
    let seen: Vec<(PathBuf, &crate::store::SessionSeen)> = sessions
        .iter()
        .map(|s| (real(Path::new(&s.cwd)), s))
        .collect();
    for row in rows.iter_mut().filter(|r| r.state != "main") {
        let dir = real(&row.path);
        row.session = seen
            .iter()
            .filter(|(cwd, _)| crate::fs::path_starts_with(cwd, &dir))
            .map(|(_, s)| *s)
            .max_by(|a, b| a.last_seen.cmp(&b.last_seen).then(b.id.cmp(&a.id)))
            .map(|s| Seen {
                session: s.id.clone(),
                host: s.host.clone(),
                seen_unix: s.last_seen,
                live: s.ended_at.is_none(),
            });
    }
}

/// Every worktree of the repository `cwd` belongs to, then the orphans. The build caches are not
/// walked (T936): their bytes are the slow part and most callers never show them.
pub fn rows(cwd: &Path) -> anyhow::Result<Vec<Row>> {
    rows_with(cwd, false)
}

/// [`rows`] with every tagged build cache walked and its bytes filled in.
pub fn rows_sized(cwd: &Path) -> anyhow::Result<Vec<Row>> {
    rows_with(cwd, true)
}

fn rows_with(cwd: &Path, sized: bool) -> anyhow::Result<Vec<Row>> {
    let listed = inventory_with(cwd, |e| {
        let owner = e.record.owner();
        Row {
            branch: e.record.branch.clone(),
            agent: owner.as_ref().and_then(|o| o.agent.clone()).map(Bound::new),
            owner: owner.map(|o| o.owner),
            locked: e.record.locked.is_some(),
            ..Row::new(e.record.path.clone(), e.state.label(), sized)
        }
    })?;
    let (entries, mut rows): (Vec<Entry>, Vec<Row>) = listed.into_iter().unzip();
    let listed: Vec<PathBuf> = entries.iter().map(|e| e.record.path.clone()).collect();
    let orphans = orphans(&listed);
    rows.extend(par_map(&orphans, |_, p| {
        Row::new(p.clone(), "orphan", sized)
    }));
    Ok(rows)
}

/// What the store knows about `rows`: T154's inferred session per worktree, then T285's bound
/// agent. The listing must not depend on the store, so without one (or on a store error) the
/// rows stay unattributed. Shared by `rtok worktree list` and MCP `worktree_list`. The caller
/// scans first and opens the store after: a store opened under a worktree's own directory
/// would otherwise show up in that worktree's scan.
pub fn attribute_with_store(rows: &mut [Row], store: Option<&crate::store::Store>, idle: &str) {
    if let Some(store) = store
        && let Ok(seen) = store.sessions_by_cwd()
    {
        attribute(rows, &seen);
        let _ = bind(rows, store, idle);
    }
}

pub fn to_table(rows: &[Row], now: SystemTime) -> String {
    let now = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let dash = || "-".to_string();
    let mut lines = vec![
        [
            "path",
            "branch",
            "owner",
            "agent",
            "agent state",
            "state",
            "origin",
            "seen",
            "modified",
            "source",
            "cache",
        ]
        .map(String::from)
        .to_vec(),
    ];
    lines.extend(rows.iter().map(|r| {
        let owner = match (&r.owner, r.locked) {
            (Some(owner), _) => owner.clone(),
            (None, true) => "locked, owner unknown".into(),
            (None, false) => dash(),
        };
        let id8 = |id: &str| id.chars().take(8).collect::<String>();
        let host = |h: &Option<String>| h.clone().unwrap_or_else(|| "?".into());
        let (agent, agent_state) = match (&r.agent, &r.session) {
            (Some(a), _) => (format!("{} {}", id8(&a.id), host(&a.host)), a.state.into()),
            (None, Some(s)) => (
                format!("seen {} {}", host(&s.host), id8(&s.session)),
                dash(),
            ),
            (None, None) => (dash(), dash()),
        };
        let ago = |t: i64| duration(now as i64 - t);
        let seen = r.session.as_ref().map(|s| ago(s.seen_unix));
        let age = r.modified_unix.map(|t| ago(t as i64));
        vec![
            r.path.display().to_string(),
            r.branch.clone().unwrap_or_else(dash),
            owner,
            agent,
            agent_state,
            r.state.into(),
            r.origin.into(),
            seen.unwrap_or_else(dash),
            age.unwrap_or_else(dash),
            human_bytes(r.source_bytes),
            r.cache_bytes.map_or_else(dash, human_bytes),
        ]
    }));
    let cols = [0; 9].map(Col::left).into_iter();
    let cols: Vec<Col> = cols.chain([Col::right(0), Col::right(0)]).collect();
    let source: u64 = rows.iter().map(|r| r.source_bytes).sum();
    let cache = if rows.iter().all(|r| r.cache_bytes.is_some()) {
        let cache: u64 = rows.iter().filter_map(|r| r.cache_bytes).sum();
        format!("{} build cache", human_bytes(cache))
    } else {
        "build cache not sized (--sizes)".into()
    };
    format!(
        "{}\n{} worktrees: {} source, {cache} (logical bytes; clones and hard links count in full)\n",
        table(&cols, &lines),
        rows.len(),
        human_bytes(source),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::tmp_dir;
    use std::fs::{create_dir_all, write};

    #[test]
    fn only_a_tagged_directory_is_cache_and_a_nested_checkout_is_not_counted() {
        let dir = tmp_dir("wt-usage");
        for sub in ["src", "target/debug", "vendor/target", "nested/src"] {
            create_dir_all(dir.join(sub)).unwrap();
        }
        write(dir.join("src/a.rs"), [0; 10]).unwrap();
        write(dir.join("target/CACHEDIR.TAG"), CACHEDIR_SIGNATURE).unwrap();
        write(dir.join("target/debug/bin"), [0; 1000]).unwrap();
        // Same name, no tag (and a tag with the wrong signature): source, never cache.
        write(dir.join("vendor/target/lib"), [0; 100]).unwrap();
        write(dir.join("vendor/CACHEDIR.TAG"), "not a signature").unwrap();
        write(dir.join("nested/.git"), "gitdir: /elsewhere").unwrap();
        write(dir.join("nested/src/big"), [0; 5000]).unwrap();

        let used = usage(&dir);
        assert_eq!(used.cache, 1000 + CACHEDIR_SIGNATURE.len() as u64);
        assert_eq!(used.source, 10 + 100 + "not a signature".len() as u64);
        assert!(used.modified.is_some());
        let [root] = used.caches.as_slice() else {
            panic!("one cache root, got {:?}", used.caches);
        };
        assert_eq!(
            (root.path.as_path(), root.bytes),
            (dir.join("target").as_path(), used.cache)
        );
        // The root's newest file is at most as new as the worktree's newest file.
        assert!(root.modified.is_some() && root.modified <= used.modified);
        assert_eq!(usage(&dir.join("missing")), Usage::default());
    }

    /// T936: the unsized walk reports the same source bytes and never opens a tagged cache.
    #[test]
    fn usage_sources_skips_tagged_caches() {
        let dir = tmp_dir("wt-t936-sources");
        create_dir_all(dir.join("src")).unwrap();
        create_dir_all(dir.join("target/debug")).unwrap();
        write(dir.join("src/a.rs"), [0; 10]).unwrap();
        write(dir.join("target/CACHEDIR.TAG"), CACHEDIR_SIGNATURE).unwrap();
        write(dir.join("target/debug/bin"), [0; 1000]).unwrap();

        let (full, quick) = (usage(&dir), usage_sources(&dir));
        assert_eq!(quick.source, full.source);
        assert_eq!((quick.cache, quick.caches.len()), (0, 0));
        assert!(full.cache > 0);
    }

    /// T948: a build that rewrites `target/` is not work, so gc's idle age reads source files
    /// only; `usage` (what `worktree list` and `clean` show) still counts the cache's mtimes.
    #[test]
    fn newest_until_leaves_tagged_caches_out() {
        let dir = tmp_dir("wt-t948-sources");
        create_dir_all(dir.join("target/debug")).unwrap();
        write(dir.join("a.rs"), b"x").unwrap();
        write(dir.join("target/CACHEDIR.TAG"), CACHEDIR_SIGNATURE).unwrap();
        write(dir.join("target/debug/bin"), b"x").unwrap();
        let old = UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        let new = UNIX_EPOCH + std::time::Duration::from_secs(2_000);
        let touch = |p: PathBuf, t| {
            let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
            f.set_modified(t).unwrap();
        };
        touch(dir.join("a.rs"), old);
        touch(dir.join("target/CACHEDIR.TAG"), new);
        touch(dir.join("target/debug/bin"), new);
        // Nothing is ever enough: the whole walk.
        let sources = newest_until(&dir, |_| false);
        let used = usage(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(sources, Some(old));
        assert_eq!(used.modified, Some(new));
        assert_eq!(used.caches[0].modified, Some(new));
    }

    /// `gc`'s question: the walk stops at the first file young enough, and that file
    /// answers it the same as the newest one would.
    #[test]
    fn newest_until_stops_at_the_first_file_that_is_enough() {
        let dir = tmp_dir("wt-newest");
        for name in ["a", "b", "c"] {
            write(dir.join(name), b"x").unwrap();
        }
        let seen = std::cell::Cell::new(0);
        let found = newest_until(&dir, |_| {
            seen.set(seen.get() + 1);
            true
        });
        assert_eq!(seen.get(), 1, "one recent file ends the walk");
        assert!(found.is_some() && found <= usage(&dir).modified);
        assert_eq!(newest_until(&dir.join("missing"), |_| true), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T656: a directory the walk cannot open may hold the newest file, so the walk says it
    /// is incomplete and `newest_until` reports no age unless a recent file already settled it.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_directory_makes_the_walk_incomplete() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp_dir("wt-t656-unreadable");
        write(dir.join("old"), b"x").unwrap();
        let hidden = dir.join("hidden");
        std::fs::create_dir(&hidden).unwrap();
        write(hidden.join("new"), b"x").unwrap();
        assert!(!usage(&dir).incomplete);
        std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o000)).unwrap();
        let blocked = std::fs::read_dir(&hidden).is_err(); // root ignores the mode bits
        let used = usage(&dir);
        let newest = newest_until(&dir, |_| false);
        let settled = newest_until(&dir, |_| true);
        std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        if blocked {
            assert!(used.incomplete);
            assert_eq!(newest, None, "an incomplete walk has no newest mtime");
            assert!(settled.is_some(), "a file that settles it still answers");
        }
    }

    #[test]
    fn the_table_names_an_unknown_owner_and_totals_both_kinds_of_bytes() {
        let row = |state, owner: Option<&str>, locked, session: Option<Seen>, cache| Row {
            path: "/w/x".into(),
            branch: Some("t1".into()),
            owner: owner.map(Into::into),
            locked,
            state,
            origin: "other",
            agent: None,
            session,
            source_bytes: 1024,
            cache_bytes: cache,
            modified_unix: Some(1_000),
        };
        let seen = Seen {
            session: "b1e2c3d4-0000-4000-8000-000000000001".into(),
            host: Some("claude".into()),
            seen_unix: 1_000 + 3 * 86_400,
            live: true,
        };
        let rows = [
            row(
                "merged",
                Some("Cursor / grok"),
                true,
                Some(seen.clone()),
                Some(2048),
            ),
            row("dirty", None, true, None, Some(0)),
            row("unmerged", None, false, Some(seen), Some(0)),
            row("orphan", None, false, None, Some(0)),
            Row {
                agent: Some(Bound {
                    id: "0193ab12-0000-7000-8000-000000000000".into(),
                    host: Some("claude".into()),
                    state: "live",
                }),
                ..row("unmerged", Some("claude / sonnet"), true, None, Some(0))
            },
        ];
        let now = UNIX_EPOCH + std::time::Duration::from_secs(1_000 + 3 * 86_400 + 4 * 3_600);
        insta::assert_snapshot!(to_table(&rows, now), @"
        path branch owner                 agent                agent state state    origin seen  modified source  cache
        /w/x t1     Cursor / grok         seen claude b1e2c3d4 -           merged   other  4h00m 3d04h    1.0 KB 2.0 KB
        /w/x t1     locked, owner unknown -                    -           dirty    other  -     3d04h    1.0 KB    0 B
        /w/x t1     -                     seen claude b1e2c3d4 -           unmerged other  4h00m 3d04h    1.0 KB    0 B
        /w/x t1     -                     -                    -           orphan   other  -     3d04h    1.0 KB    0 B
        /w/x t1     claude / sonnet       0193ab12 claude      live        unmerged other  -     3d04h    1.0 KB    0 B

        5 worktrees: 5.0 KB source, 2.0 KB build cache (logical bytes; clones and hard links count in full)
        ");
    }

    /// T936: a row whose caches were not walked prints `-` and the footer says why.
    #[test]
    fn an_unsized_row_has_no_cache_bytes_in_the_table() {
        let row = Row::new("/w/x".into(), "dirty", false);
        let table = to_table(&[row], UNIX_EPOCH);
        assert!(table.contains("build cache not sized (--sizes)"), "{table}");
        assert!(table.lines().nth(1).unwrap().ends_with(" -"), "{table}");
    }

    /// T154: the newest session at or under a linked worktree wins; the main checkout and a
    /// worktree nobody visited stay unattributed.
    #[test]
    fn attribute_picks_the_newest_session_under_each_linked_worktree() {
        use crate::store::SessionSeen;
        let dir = tmp_dir("wt-attribute");
        for sub in ["work", "wt-a/src", "wt-b"] {
            create_dir_all(dir.join(sub)).unwrap();
        }
        let session = |id: &str, cwd: PathBuf, last_seen, ended_at| SessionSeen {
            id: id.into(),
            host: Some("claude".into()),
            cwd: cwd.to_string_lossy().into_owned(),
            last_seen,
            ended_at,
        };
        let sessions = [
            session("old", dir.join("wt-a"), 100, Some(150)),
            session("new", dir.join("wt-a/src"), 200, None),
            session("main", dir.join("work"), 300, None),
        ];
        let mut rows: Vec<Row> = [("work", "main"), ("wt-a", "dirty"), ("wt-b", "merged")]
            .into_iter()
            .map(|(name, state)| Row::new(dir.join(name), state, false))
            .collect();
        attribute(&mut rows, &sessions);
        assert_eq!(rows[0].session, None, "the main checkout belongs to nobody");
        assert_eq!(
            rows[1].session,
            Some(Seen {
                session: "new".into(),
                host: Some("claude".into()),
                seen_unix: 200,
                live: true,
            })
        );
        assert_eq!(rows[2].session, None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
