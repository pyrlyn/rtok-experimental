// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Test-only helpers shared by the unit tests: a fresh temp dir per call and a `Config` whose
//! every on-disk path lives inside it, so no test writes to `~/.rtok` or into another test's files.

use crate::config::Config;
use crate::plugin::Runtime;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fresh, empty directory unique to this call. The pid keeps parallel test binaries apart and
/// the counter keeps parallel tests of one binary apart, even when they pass the same `tag`.
pub fn tmp_dir(tag: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rtok-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An absolute path from a Unix-style spelling: unchanged on Unix, `C:` plus backslashes on
/// Windows, where `/repo` has a root but no drive and is not absolute.
pub fn native_abs(unix: &str) -> String {
    if cfg!(windows) {
        format!("C:{}", unix.replace('/', "\\"))
    } else {
        unix.to_string()
    }
}

/// Default config with the DB, the archive and the log under a fresh [`tmp_dir`].
pub fn config(tag: &str) -> (Config, PathBuf) {
    let dir = tmp_dir(tag);
    (config_in(&dir), dir)
}

/// Default config with every on-disk path under `dir`. A test that sets only `db_path` left
/// `log.path` at its unexpanded default and wrote a literal `./~/.rtok/logs/` into the repo.
pub fn config_in(dir: &Path) -> Config {
    let mut c = Config::default();
    c.rebase_paths(dir);
    c.log.path = dir.join("rtok.log");
    c
}

/// [`Config::load_from`] `dir`, for a test that needs `dir/config.toml` itself — a TUI or web
/// `set` rewrites and reloads it. Every absolute path of [`config_in`] is written into the file
/// first (T252, T254): a reload would otherwise point each `~/x` back at this machine's real
/// `~/.claude*`, `~/.codex` and agent configs, and a snapshot would parse its session history.
pub fn config_file_in(dir: &Path) -> Config {
    // Fixtures need persisted paths, not the annotated reference file: revealing each path
    // in that template repeatedly scans all its documentation before applying the edits.
    let path = Config::user_path(dir, None);
    if !path.exists() {
        crate::config::write_file(&path, "").expect("empty fixture config");
    }
    let mut rebased = config_in(dir);
    let mut pairs: Vec<(String, String)> = Vec::new();
    // A list key (`plugins.read.allow_paths[0]`, `agents.usage.dirs.kilo[0]`) is one `set`
    // of a TOML array, not one per element.
    let mut lists: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for (key, path) in rebased.path_fields_mut() {
        if !path.is_absolute() {
            continue;
        }
        let path = path.to_string_lossy().into_owned();
        match key.split_once('[') {
            Some((list, _)) => lists.entry(list.to_owned()).or_default().push(path),
            None => pairs.push((key.into_owned(), path)),
        }
    }
    pairs.extend(
        lists
            .into_iter()
            .map(|(key, paths)| (key, serde_json::to_string(&paths).expect("paths as json"))),
    );
    let pairs: Vec<_> = pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    crate::config::validate::set_all_with(dir, None, &pairs, false).expect("config set");
    Config::load_from(dir).expect("config")
}

/// A [`Runtime`] over [`config`]; `tag` doubles as the session id.
pub fn runtime(tag: &str) -> (Runtime, PathBuf) {
    let (c, dir) = config(tag);
    (Runtime::open(c, tag).unwrap(), dir)
}

/// The layout git writes for a main checkout `repo` with one linked worktree `wt/repo-t1`
/// (`.git` file → `gitdir:` → admin dir → `commondir`), no git binary needed. Returns
/// `(main, worktree)`; both resolve to the project `repo` (T133 / T154).
pub fn worktree_layout(dir: &Path) -> (PathBuf, PathBuf) {
    let (main, wt) = (dir.join("repo"), dir.join("wt").join("repo-t1"));
    let admin = main.join(".git").join("worktrees").join("repo-t1");
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::create_dir_all(&wt).unwrap();
    std::fs::write(admin.join("commondir"), "../..\n").unwrap();
    std::fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
    (main, wt)
}

/// A linked worktree at `main/rel` as git writes it, without a git binary: `main/.git` is a
/// directory, the admin dir under `.git/worktrees` has a `commondir`, and the worktree's
/// `.git` file names the admin dir. T660.3: worktrees nested in the project they belong to.
pub fn linked_worktree(main: &Path, rel: &str) -> PathBuf {
    let wt = main.join(rel);
    let admin = main.join(".git/worktrees").join(wt.file_name().unwrap());
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::create_dir_all(&wt).unwrap();
    std::fs::write(admin.join("commondir"), "../..\n").unwrap();
    std::fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
    wt
}

/// A submodule at `parent/rel`: a `.git` file naming an admin dir with no `commondir`.
pub fn submodule(parent: &Path, rel: &str) -> PathBuf {
    let sub = parent.join(rel);
    let admin = parent.join(".git/modules").join(sub.file_name().unwrap());
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
    sub
}

/// T143: claim the otel hook-flush "queued" slot for a test, simulating another hook-spawned
/// flush already waiting behind the running one. Drop the guard to release it.
pub fn hold_otel_queue_slot(cx: &Runtime) -> impl Drop + use<> {
    crate::otel::export::try_queue(cx)
        .expect("queue lock")
        .expect("queued slot free for the test to claim")
}

/// In-memory path → bytes map for unit tests that must not touch the host disk (D29 / T56).
/// Prefer this over `tmp_dir` when the code under test only needs path/content/size.
/// Optional symlinks (`link` → `target`) let `ReadFs` / resolve twins cover escape cases.
#[derive(Default, Clone, Debug)]
pub struct Vfs {
    files: std::collections::BTreeMap<String, Vec<u8>>,
    symlinks: std::collections::BTreeMap<String, String>,
}

impl Vfs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn write(&mut self, path: impl Into<String>, bytes: impl AsRef<[u8]>) {
        self.files.insert(path.into(), bytes.as_ref().to_vec());
    }

    /// Record a symlink `link` → `target` (forward-slash keys). Used by ReadFs canonicalize.
    pub fn symlink(&mut self, link: impl Into<String>, target: impl Into<String>) {
        self.symlinks.insert(link.into(), target.into());
    }

    /// Target of a recorded symlink, if any.
    pub fn symlink_target(&self, path: &str) -> Option<&str> {
        self.symlinks.get(path).map(String::as_str)
    }

    /// Follow recorded symlinks (bounded) to the final key used for file reads.
    pub fn resolve_key(&self, path: &str) -> String {
        let mut key = path.to_string();
        for _ in 0..32 {
            match self.symlinks.get(&key) {
                Some(next) => key = next.clone(),
                None => break,
            }
        }
        key
    }

    pub fn read(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    pub fn len(&self, path: &str) -> Option<u64> {
        self.files.get(path).map(|b| b.len() as u64)
    }

    pub fn paths(&self) -> impl Iterator<Item = String> + '_ {
        self.files.keys().cloned()
    }

    pub fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path) || self.symlinks.contains_key(path)
    }

    /// UTF-8 body, or `None` if missing / not UTF-8.
    pub fn read_str(&self, path: &str) -> Option<&str> {
        self.read(path).and_then(|b| std::str::from_utf8(b).ok())
    }

    /// Paths under `prefix/` (or exact `prefix`), sorted — for drop-in / tree fixtures.
    pub fn paths_under(&self, prefix: &str) -> Vec<String> {
        let mut out: Vec<String> = if prefix.is_empty() {
            self.files.keys().cloned().collect()
        } else {
            let with_sep = if prefix.ends_with('/') {
                prefix.to_string()
            } else {
                format!("{prefix}/")
            };
            self.files
                .keys()
                .filter(|p| *p == prefix || p.starts_with(&with_sep))
                .cloned()
                .collect()
        };
        out.sort();
        out
    }

    /// Immediate child names under `dir` (files and inferred directories), sorted.
    /// Empty `dir` lists the Vfs root. Used by the T56.4 walk adapter.
    pub fn list_dir(&self, dir: &str) -> Vec<String> {
        let prefix = if dir.is_empty() {
            String::new()
        } else if dir.ends_with('/') {
            dir.to_string()
        } else {
            format!("{dir}/")
        };
        let mut names = std::collections::BTreeSet::new();
        for path in self.files.keys() {
            let rest = if prefix.is_empty() {
                path.as_str()
            } else if let Some(r) = path.strip_prefix(&prefix) {
                r
            } else {
                continue;
            };
            if rest.is_empty() {
                continue;
            }
            if let Some(name) = rest.split('/').next()
                && !name.is_empty()
            {
                names.insert(name.to_string());
            }
        }
        names.into_iter().collect()
    }

    /// File and/or directory metadata for `path`. Directories are inferred from prefixes.
    /// A recorded symlink counts as a file entry (len 0) so canonicalize can see it.
    pub fn meta(&self, path: &str) -> Option<VfsMeta> {
        let is_file = self.files.contains_key(path) || self.symlinks.contains_key(path);
        let is_dir = !self.list_dir(path).is_empty();
        if !is_file && !is_dir {
            return None;
        }
        Some(VfsMeta {
            len: self.files.get(path).map(|b| b.len() as u64).unwrap_or(0),
            is_file,
            is_dir,
        })
    }
}

/// Metadata returned by [`Vfs::meta`] (T56.4 walk adapter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VfsMeta {
    pub len: u64,
    pub is_file: bool,
    pub is_dir: bool,
}

/// Seed the two-API fixture the store and stats table tests share: one proxy
/// session, two api_request calls, usage for `anthropic` and `openai_chat`.
pub fn seed_two_apis(store: &crate::store::Store) {
    store
        .upsert_session("s1", None, None, None, Some("proxy"))
        .unwrap();
    let id1 = store
        .insert_call(
            "s1",
            "proxy",
            "api_request",
            None,
            None,
            None,
            None,
            Some("/v1/messages"),
        )
        .unwrap();
    let id2 = store
        .insert_call(
            "s1",
            "proxy",
            "api_request",
            None,
            None,
            None,
            None,
            Some("/v1/chat/completions"),
        )
        .unwrap();
    store
        .insert_usage("s1", Some("m"), "anthropic", 10, 1, 2, 3, id1)
        .unwrap();
    store
        .insert_usage("s1", Some("m"), "openai_chat", 20, 0, 5, 4, id2)
        .unwrap();
}

#[cfg(test)]
mod tests {
    #[test]
    fn same_tag_gives_distinct_dirs() {
        let (a, b) = (super::tmp_dir("same"), super::tmp_dir("same"));
        assert_ne!(a, b);
        assert!(a.is_dir() && b.is_dir());
        let (c, dir) = super::config("paths");
        assert!(c.core.db_path.starts_with(&dir) && c.log.path.starts_with(&dir));
    }

    /// T254: no path of either helper leaves `dir` (relative ones name project files), so a
    /// new `~` field nobody redirected fails here instead of reading this machine's home.
    #[test]
    fn every_config_path_stays_in_dir() {
        let dir = super::tmp_dir("paths-in-dir");
        for mut c in [
            super::config_in(&dir),
            super::config_file_in(&dir),
            crate::config::Config::load_from(&dir).unwrap(),
        ] {
            for (key, path) in c.path_fields_mut() {
                let shown = path.display();
                assert!(
                    path.is_relative() || path.starts_with(&dir),
                    "{key} = {shown}"
                );
            }
        }
    }

    #[test]
    fn config_fixture_preserves_existing_settings_and_rebased_paths_on_reload() {
        let dir = super::tmp_dir("config-reload").join("nested");
        super::config_file_in(&dir);
        crate::config::validate::set(&dir, "tui.tab", "plugins", false).unwrap();
        let cfg = super::config_file_in(&dir);
        assert_eq!(cfg.tui.tab, "plugins");
        let reloaded = crate::config::Config::load_from(&dir).unwrap();
        assert_eq!(cfg, reloaded);
    }

    /// T255: nextest's `test-home` setup script moves `HOME` under `target/`, so a path that
    /// slips past `Config` lands there instead of in this machine's real home. A plain
    /// `cargo test` runs no setup script and is not checked.
    #[test]
    fn nextest_runs_under_the_test_home() {
        if std::env::var_os("NEXTEST").is_none() {
            return;
        }
        let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME"));
        assert!(home.ends_with("target/test-home"), "{}", home.display());
        #[cfg(windows)]
        assert_eq!(std::env::var_os("USERPROFILE"), Some(home.into_os_string()));
    }

    /// T644: the dirs tests leave behind go to a per-checkout dir nextest empties each run,
    /// never into this machine's `$TMPDIR`. Short on Unix, or `hook.sock` under it would be
    /// past `sun_path`. Outside the checkout on every OS, or git discovery from a test's
    /// scratch dir would find this repo.
    #[test]
    fn nextest_runs_under_the_test_tmp() {
        if std::env::var_os("NEXTEST").is_none() {
            return;
        }
        let tmp = std::env::temp_dir();
        let name = tmp.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("rtok-test-"), "{}", tmp.display());
        assert!(
            !tmp.starts_with(env!("CARGO_MANIFEST_DIR")),
            "{}",
            tmp.display()
        );
        if cfg!(unix) {
            assert_eq!(tmp.parent(), Some(std::path::Path::new("/tmp")));
        }
        assert!(tmp.is_dir());
    }

    #[test]
    fn vfs_symlink_resolve_key_and_exists() {
        let mut v = super::Vfs::new();
        v.write("/outside/secret", b"x");
        v.symlink("cwd/escape", "/outside/secret");
        assert!(v.exists("cwd/escape"));
        assert_eq!(v.symlink_target("cwd/escape"), Some("/outside/secret"));
        assert_eq!(v.resolve_key("cwd/escape"), "/outside/secret");
        assert!(v.meta("cwd/escape").is_some_and(|m| m.is_file));
    }

    #[test]
    fn vfs_round_trips_bytes_and_len() {
        let mut v = super::Vfs::new();
        v.write("a.txt", b"hi");
        assert_eq!(v.read("a.txt"), Some(b"hi".as_slice()));
        assert_eq!(v.read_str("a.txt"), Some("hi"));
        assert_eq!(v.len("a.txt"), Some(2));
        assert!(v.exists("a.txt"));
        assert_eq!(v.paths().collect::<Vec<_>>(), vec!["a.txt".to_string()]);
        v.write("rules.d/b.toml", b"x");
        v.write("rules.d/a.toml", b"y");
        assert_eq!(
            v.paths_under("rules.d"),
            vec!["rules.d/a.toml".to_string(), "rules.d/b.toml".to_string()]
        );
    }

    #[test]
    fn vfs_read_str_rejects_non_utf8_and_missing() {
        let mut v = super::Vfs::new();
        v.write("bin", [0x80, 0x81]);
        assert!(v.exists("bin"));
        assert_eq!(v.read_str("bin"), None);
        assert_eq!(v.read_str("nope"), None);
        assert!(!v.exists("nope"));
    }

    #[test]
    fn vfs_paths_under_trailing_slash_and_exact() {
        let mut v = super::Vfs::new();
        v.write("rules.d/x.toml", b"1");
        v.write("rules.d", b"not-a-dir-key"); // exact prefix key
        v.write("other/y.toml", b"2");
        assert_eq!(
            v.paths_under("rules.d/"),
            vec!["rules.d/x.toml".to_string()]
        );
        let under = v.paths_under("rules.d");
        assert!(under.contains(&"rules.d".to_string()));
        assert!(under.contains(&"rules.d/x.toml".to_string()));
        assert!(!under.iter().any(|p| p.starts_with("other")));
    }

    #[test]
    fn vfs_overwrite_replaces_bytes_and_len() {
        let mut v = super::Vfs::new();
        v.write("a.txt", b"hi");
        v.write("a.txt", b"hello");
        assert_eq!(v.read_str("a.txt"), Some("hello"));
        assert_eq!(v.len("a.txt"), Some(5));
    }

    #[test]
    fn vfs_list_dir_and_meta_infer_directories() {
        let mut v = super::Vfs::new();
        v.write("nest/a/f.txt", b"x");
        v.write("nest/hit.rs", b"y");
        assert_eq!(v.list_dir(""), vec!["nest".to_string()]);
        assert_eq!(
            v.list_dir("nest"),
            vec!["a".to_string(), "hit.rs".to_string()]
        );
        assert_eq!(v.list_dir("nest/a"), vec!["f.txt".to_string()]);
        let nest = v.meta("nest").unwrap();
        assert!(nest.is_dir && !nest.is_file && nest.len == 0);
        let hit = v.meta("nest/hit.rs").unwrap();
        assert!(hit.is_file && !hit.is_dir && hit.len == 1);
        assert!(v.meta("missing").is_none());
    }
}
