// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T351: roots the path guard accepts beyond cwd and `allow_paths`, for `rtok mcp` only.
//!
//! Agents work in a task worktree while the host launched the server in the main checkout,
//! so a path in a sibling worktree of the cwd's repository is allowed, and so is every
//! `file://` root of the client's `roots/list` answer (`mcp.rs`) with the worktrees of its
//! repository (T435). Scratchpads and every
//! other directory stay outside; `plugins.read.allow_paths` remains the manual escape hatch.
//! Nothing here runs unless `rtok mcp` called [`enable`], so hooks never spawn git.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// A linked worktree can be added after the server started, so the listing is re-read
/// after this long, and only when a path failed the cheap checks first.
const WORKTREE_TTL: Duration = Duration::from_secs(30);

static ENABLED: AtomicBool = AtomicBool::new(false);
static DENY_SECRETS: AtomicBool = AtomicBool::new(false);
static CLIENT: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
/// Per cwd: when the listing was read, and its worktree paths.
type Listing = (Instant, Vec<PathBuf>);

static WORKTREES: LazyLock<Mutex<HashMap<PathBuf, Listing>>> = LazyLock::new(Default::default);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Called once by `rtok mcp`; turns the extra roots on for this process.
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

/// Called by `rtok mcp --http`: refuse credential files (see [`is_secret`]) in this process.
pub fn deny_secrets() {
    DENY_SECRETS.store(true, Ordering::Relaxed);
}

/// Whether credential files are refused in this process.
pub(crate) fn secrets_denied() -> bool {
    DENY_SECRETS.load(Ordering::Relaxed)
}

pub(crate) use crate::fs::is_secret;

/// The directories of the client's latest `roots/list` answer (replaces the previous one).
pub fn set_client_roots(roots: Vec<PathBuf>) {
    *lock(&CLIENT) = roots;
}

/// The client's latest `roots/list` directories, for the project-dir fallback.
pub(crate) fn client_roots() -> Vec<PathBuf> {
    lock(&CLIENT).clone()
}

/// Client roots plus the worktrees of the repositories of `cwd` and of every client root;
/// empty unless [`enable`]d.
pub(crate) fn dynamic(cwd: &Path) -> Vec<PathBuf> {
    if !ENABLED.load(Ordering::Relaxed) {
        return Vec::new();
    }
    let client = lock(&CLIENT).clone();
    let mut roots = client.clone();
    // T435: Claude.app's Code tab can hand a session a server whose cwd is another project,
    // so the session's own repository is known only as a client root.
    for dir in std::iter::once(cwd).chain(client.iter().map(PathBuf::as_path)) {
        roots.extend(worktrees(dir));
    }
    roots
}

thread_local! {
    /// The directory the current tool call's relative paths resolve against, when the server
    /// knows the caller works somewhere other than its own cwd (T923). Per thread, so two
    /// concurrent calls never see each other's root.
    static CALLER: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Restores the previous caller root when the tool call ends.
pub(crate) struct CallerGuard(Option<PathBuf>);

impl Drop for CallerGuard {
    fn drop(&mut self) {
        CALLER.with(|c| *c.borrow_mut() = self.0.take());
    }
}

/// The caller's root for this call, when [`enter_caller`] set one.
pub(crate) fn caller_root() -> Option<PathBuf> {
    CALLER.with(|c| c.borrow().clone())
}

/// Whether `dir`'s repository has a linked worktree besides `dir` itself, so a relative path
/// answered from `dir` could have been meant for another checkout.
pub(crate) fn has_other_worktrees(dir: &Path) -> bool {
    worktrees(dir).len() > 1
}

/// The base of relative paths: the caller's root for this call, else the process cwd.
pub(crate) fn cwd() -> std::io::Result<PathBuf> {
    match caller_root() {
        Some(dir) => Ok(dir),
        None => std::env::current_dir(),
    }
}

/// Makes `dir` the base of relative paths until the guard drops, if the path guard would
/// accept it from a server running in `server_cwd`: inside it, or inside a worktree of its
/// repository or a client root's. A caller root comes from a hook-registered agent row, which
/// is untrusted, so anything else (a missing directory, `/`, an unrelated project) is ignored.
pub(crate) fn enter_caller(server_cwd: &Path, dir: &Path) -> Option<CallerGuard> {
    let dir = fs_kit::path::canonicalize(dir)
        .ok()
        .filter(|d| d.is_dir())?;
    let canon = |p: &Path| fs_kit::path::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let client = lock(&CLIENT).clone();
    let known = std::iter::once(server_cwd)
        .chain(client.iter().map(PathBuf::as_path))
        .flat_map(|d| std::iter::once(d.to_path_buf()).chain(worktrees(d)));
    // `/` and `$HOME` (Claude.app launches the server there) would admit every directory.
    let home = std::env::home_dir();
    if !known
        .map(|r| canon(&r))
        .filter(|r| !crate::fs::is_unwalkable_root(r, home.as_deref()))
        .any(|r| dir.starts_with(&r))
    {
        return None;
    }
    Some(CallerGuard(CALLER.with(|c| c.borrow_mut().replace(dir))))
}

/// T972: ` (moved? it exists at <path>)` for a missing absolute `path` when one of its
/// directories is named like a worktree or client root this server knows and the same file
/// is there, else empty. Agents keep reading old paths after a worktree root moves (the
/// `_worktrees/` → `.rtok/worktrees/` move cost ten failed reads in one day).
pub(crate) fn moved_hint(cwd: &Path, path: &Path) -> String {
    if !path.is_absolute() {
        return String::new();
    }
    let trees = dynamic(cwd);
    let parts: Vec<_> = path.components().collect();
    let found = parts.iter().enumerate().find_map(|(i, part)| {
        let rest: PathBuf = parts[i + 1..].iter().collect();
        trees
            .iter()
            .filter(|t| t.file_name() == Some(part.as_os_str()))
            .map(|t| t.join(&rest))
            .find(|p| p != path && p.is_file())
    });
    found.map_or_else(String::new, |p| {
        format!(" (moved? it exists at {})", p.display())
    })
}

fn worktrees(cwd: &Path) -> Vec<PathBuf> {
    if let Some((at, list)) = lock(&WORKTREES).get(cwd)
        && at.elapsed() < WORKTREE_TTL
    {
        return list.clone();
    }
    // Fail soft: no git, not a repository, or a broken listing grants nothing.
    let list: Vec<PathBuf> = crate::worktree::git::list(cwd)
        .map(|records| records.into_iter().map(|r| r.path).collect())
        .unwrap_or_default();
    lock(&WORKTREES).insert(cwd.to_path_buf(), (Instant::now(), list.clone()));
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .expect("git")
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    /// Main checkout + linked worktree + an unrelated dir, all canonical (macOS `/var`).
    fn repo(tag: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("rtok-roots-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (main, wt, other) = (base.join("main"), base.join("wt"), base.join("other"));
        for d in [&main, &other] {
            std::fs::create_dir_all(d).unwrap();
        }
        let base = fs_kit::path::canonicalize(&base).unwrap();
        let (main, other) = (base.join("main"), base.join("other"));
        git(&main, &["init", "-q"]);
        std::fs::write(main.join("a.txt"), "a\n").unwrap();
        git(&main, &["add", "."]);
        git(&main, &["commit", "-q", "-m", "init"]);
        git(
            &main,
            &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "w"],
        );
        (base.clone(), main, base.join("wt"), other)
    }

    #[test]
    fn worktrees_of_the_repo_are_listed_and_other_dirs_are_not() {
        let (base, main, wt, other) = repo("list");
        let listed: Vec<PathBuf> = worktrees(&main)
            .iter()
            .map(|p| fs_kit::path::canonicalize(p).unwrap())
            .collect();
        assert!(listed.contains(&wt) && listed.contains(&main), "{listed:?}");
        assert!(!listed.contains(&other));
        // Not a repository: nothing, no error.
        assert!(worktrees(&other).is_empty());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn guard_accepts_sibling_worktree_and_client_roots() {
        let (base, main, wt, other) = repo("guard");
        let scratch = base.join("scratchpad");
        std::fs::create_dir_all(&scratch).unwrap();
        let files = [wt.join("a.txt"), other.join("b.txt"), scratch.join("c.txt")];
        for f in &files[1..] {
            std::fs::write(f, "x\n").unwrap();
        }
        let ok = |p: &Path| super::super::resolve(&main, p, &[]).is_ok();
        enable();
        assert!(ok(&files[0]), "sibling worktree");
        assert!(
            !ok(&files[1]) && !ok(&files[2]),
            "unrelated dir, scratchpad"
        );
        set_client_roots(vec![other.clone(), base.join("gone")]);
        assert!(ok(&files[1]), "client root");
        assert!(!ok(&files[2]), "scratchpad stays outside");
        let err = super::super::resolve(&main, &files[2], &[]).unwrap_err();
        assert!(err.to_string().contains("path outside cwd"), "{err}");
        // T435: the process cwd is another project; this repository is only a client root.
        set_client_roots(vec![main.clone()]);
        assert!(
            super::super::resolve(&other, &files[0], &[]).is_ok(),
            "worktree of a client root's repository"
        );
        assert!(super::super::resolve(&other, &files[2], &[]).is_err());
        set_client_roots(Vec::new());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_missing_path_under_an_old_worktree_root_names_where_the_file_is_now() {
        let (base, main, wt, _) = repo("moved");
        // The old root sat under the server's cwd, so the guard let the read through.
        let old = main.join("_worktrees").join("wt").join("a.txt");
        assert_eq!(moved_hint(&main, &old), "", "only `rtok mcp` looks");
        enable();
        let hint = format!(" (moved? it exists at {})", wt.join("a.txt").display());
        assert_eq!(moved_hint(&main, &old), hint);
        let gone = main.join("_worktrees").join("wt").join("gone.txt");
        assert_eq!(
            moved_hint(&main, &gone),
            "",
            "no such file in the worktree either"
        );
        assert_eq!(moved_hint(&main, &wt.join("gone.txt")), "");
        let (rt, dir) = crate::testutil::runtime("moved-read");
        let err = crate::plugins::read::read_with(
            &rtok_plugin_sdk::Ctx::new(&rt),
            &crate::fs::HostFs,
            &main,
            old.to_str().unwrap(),
            "full",
            None,
        )
        .unwrap_err();
        assert!(format!("{err:#}").ends_with(&hint), "{err:#}");
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn relative_paths_follow_the_caller_worktree_not_the_server_checkout() {
        use rtok_plugin_sdk::Ctx;
        let (base, main, wt, other) = repo("caller");
        std::fs::write(wt.join("a.txt"), "edited in the worktree\n").unwrap();
        let (rt, dir) = crate::plugins::read::tests::cx("callerroot");
        let cx = Ctx::new(&rt);
        // The server runs in `main`; the caller works in `wt`.
        let guard = enter_caller(&main, &wt).expect("sibling worktree is accepted");
        assert_eq!(cwd().unwrap(), wt);
        let read = crate::plugins::read::read(&cx, "a.txt", "full", None).unwrap();
        assert!(read.contains("edited in the worktree"), "{read}");
        let hits = crate::plugins::read::search::search(&cx, "edited", "", None).unwrap();
        assert!(hits.contains("a.txt:1:"), "{hits}");
        let tree = crate::plugins::read::search::tree(&cx, ".", None).unwrap();
        assert!(tree.contains("a.txt"), "{tree}");
        drop(guard);
        assert_eq!(cwd().unwrap(), std::env::current_dir().unwrap());
        let _ = std::fs::remove_dir_all(base);
        let _ = std::fs::remove_dir_all(dir);
        let _ = other;
    }

    #[test]
    fn a_caller_root_outside_the_server_project_is_refused() {
        let (base, main, _wt, other) = repo("callerbad");
        // A hook-registered cwd is untrusted: an unrelated directory, `/` and a missing
        // path must not become the base of relative reads.
        for dir in [other.as_path(), Path::new("/"), &base.join("gone")] {
            assert!(enter_caller(&main, dir).is_none(), "{}", dir.display());
        }
        let sub = main.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        assert!(
            enter_caller(&main, &sub).is_some(),
            "subdirectory of the project"
        );
        let _ = std::fs::remove_dir_all(base);
    }
}
