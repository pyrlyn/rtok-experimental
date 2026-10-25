// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `search` + `tree` (plan T4.5): gitignore-aware regex hits and a size listing.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use ignore::WalkBuilder;
use regex::Regex;

use super::resolve;
use rtok_plugin_sdk::Ctx;

/// Walk results are rooted at `resolve`'s path (often `canonicalize(cwd)`), but
/// display used to strip the raw `current_dir()`. Whenever those disagree —
/// macOS `/tmp` → `/private/tmp`, or an `allow_paths` root outside cwd — every
/// hit became an absolute path. Prefer relative-to-canonical-cwd (so a search
/// of `src` still yields `src/…`), then relative-to-the-walk-root, then raw.
///
/// Strip `prefix` from `path` (T430: ASCII case and a `\\?\` prefix on Windows).
fn strip_prefix_ci(path: &Path, prefix: &Path) -> Option<PathBuf> {
    crate::fs::strip_prefix(path, prefix)
}

/// Canonical walk base: `fs_kit::path::canonicalize(cwd)`, or `cwd` when it does not
/// exist. `search`/`tree` compute this once per call (T59.2) — `display_rel`
/// takes the base instead of paying the syscalls per hit or row.
fn canonical_base_with(fs: &impl super::fs::ReadFs, cwd: &Path) -> PathBuf {
    fs.canonicalize(cwd).unwrap_or_else(|| cwd.to_path_buf())
}

fn canonical_base(cwd: &Path) -> PathBuf {
    canonical_base_with(&super::fs::HostFs, cwd)
}

fn display_rel(path: &Path, root: &Path, base: &Path) -> String {
    let rel = strip_prefix_ci(path, base)
        .or_else(|| strip_prefix_ci(path, root))
        .unwrap_or_else(|| path.to_path_buf());
    // A hit on the root itself strips to "": name the file instead of
    // printing `:1: snippet` with no path.
    if rel.as_os_str().is_empty() {
        return path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
    // One spelling on every OS (T83.16): `src\main.rs` on Windows missed every caller and test
    // looking for `src/main.rs`, and Windows takes `/` back in any path. Unix keeps `\`, a legal
    // file-name byte there.
    let shown = rel.display().to_string();
    if cfg!(windows) {
        shown.replace('\\', "/")
    } else {
        shown
    }
}

/// `WalkBuilder::hidden(false)` also descends into `.git/`; no tool wants object files,
/// packed refs or reflogs as hits (`search` returned `.git/logs/HEAD`, 2026-09-16).
///
/// A linked worktree below the walk root (one under `.rtok/worktrees`) is not walked into
/// either: its files belong to another checkout even when no ignore rule names it.
/// Submodules and nested clones stay in the walk.
pub(crate) fn skip_git(e: &ignore::DirEntry) -> bool {
    if e.file_name() == ".git" {
        return false;
    }
    let worktree = e.depth() > 0
        && e.file_type().is_some_and(|t| t.is_dir())
        && crate::fs::is_linked_worktree(e.path());
    !worktree
}

/// [`skip_git`], and with credential files refused in this process (`rtok mcp --http`, S4) no
/// credential file either: `read` refuses `.env`, so `search` must not print its lines.
pub(crate) fn walk_filter(deny_secrets: bool) -> impl Fn(&ignore::DirEntry) -> bool + Send + Sync {
    move |e| skip_git(e) && !(deny_secrets && super::roots::is_secret(e.path()))
}

/// The regular files a walk reaches whose bytes are valid UTF-8 text, as `(path, text)`, read one
/// at a time. T55.5: a file over `max_bytes` is skipped before it is read, so a multi-GB blob
/// cannot spike MCP memory. Shared by `search` and the graph text backend (T329.10).
pub(crate) fn text_files(
    walk: impl Iterator<Item = ignore::DirEntry>,
    max_bytes: u64,
) -> impl Iterator<Item = (PathBuf, String)> {
    text_files_with_policy(walk, max_bytes, super::roots::secrets_denied())
}

fn text_files_with_policy(
    walk: impl Iterator<Item = ignore::DirEntry>,
    max_bytes: u64,
    deny_secrets: bool,
) -> impl Iterator<Item = (PathBuf, String)> {
    walk.filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        // Graph's text backend supplies its own walk, so the policy must also guard reads.
        .filter(walk_filter(deny_secrets))
        .filter_map(move |e| {
            if fs::metadata(e.path()).ok()?.len() > max_bytes {
                return None;
            }
            let text = fs::read_to_string(e.path()).ok()?;
            Some((e.into_path(), text))
        })
}

/// `path:line: snippet` rows, at most `max` (default `plugins.read.search_max`).
pub fn search(cx: &Ctx, pattern: &str, path: &str, max: Option<u32>) -> Result<String> {
    let cfg = cx.plugin_config::<crate::config::Read>("read");
    let cwd = super::roots::cwd()?;
    // A pathless search from `$HOME` or `/` (those roots are refused) searches the client
    // root that lies in a repository, instead of failing on the launch cwd.
    let no_path = path.is_empty() || path == ".";
    let root = if no_path && crate::fs::is_unwalkable_root(&cwd, std::env::home_dir().as_deref()) {
        crate::project::project_dir(&cwd, &super::roots::client_roots())?
    } else {
        resolve(
            &cwd,
            Path::new(if path.is_empty() { "." } else { path }),
            &cfg.allow_paths,
        )?
    };
    // T263: the resolved root, so an explicit `path` still works from `/`.
    super::walk_root_ok(&root)?;
    let cap = max.unwrap_or(cfg.search_max).max(1) as usize;
    // Same grammar as `expand --grep`: an invalid regex searches literally
    // instead of erroring the whole call.
    let re = Regex::new(pattern).or_else(|_| Regex::new(&regex::escape(pattern)))?;
    let base = canonical_base(&cwd);
    let mut hits = Vec::new();
    let mut walk = WalkBuilder::new(&root);
    walk.hidden(false)
        .filter_entry(walk_filter(super::roots::secrets_denied()));
    let mut files = text_files(walk.build().filter_map(Result::ok), cfg.search_max_bytes);
    // Checked before the next file is read, so a full result set never costs another read.
    while hits.len() < cap {
        let Some((path, text)) = files.next() else {
            break;
        };
        let rel = display_rel(&path, &root, &base);
        for (i, line) in text.lines().enumerate() {
            if hits.len() >= cap {
                break;
            }
            if !re.is_match(line) {
                continue;
            }
            let mut snippet = line.trim().to_string();
            if snippet.chars().count() > 120 {
                snippet = snippet.chars().take(120).collect();
            }
            hits.push(format!("{rel}:{}: {snippet}", i + 1));
        }
    }
    super::cap_recording(cx, hits.join("\n"), "search_cap")
}

/// Compact listing `path size` down to `depth` (default `plugins.read.tree_depth`).
pub fn tree(cx: &Ctx, path: &str, depth: Option<u32>) -> Result<String> {
    let cfg = cx.plugin_config::<crate::config::Read>("read");
    let cwd = super::roots::cwd()?;
    let root = resolve(
        &cwd,
        Path::new(if path.is_empty() { "." } else { path }),
        &cfg.allow_paths,
    )?;
    // T263: as in `search`.
    super::walk_root_ok(&root)?;
    let depth = depth.unwrap_or(cfg.tree_depth).max(1) as usize;
    let base = canonical_base(&cwd);
    let mut rows = Vec::new();
    for entry in WalkBuilder::new(&root)
        .hidden(false)
        .filter_entry(skip_git)
        .max_depth(Some(depth))
        .build()
    {
        let Ok(entry) = entry else {
            continue;
        };
        let p = entry.path();
        if p == root {
            continue;
        }
        let rel = display_rel(p, &root, &base);
        let size = fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        rows.push(format!("{rel} {size}"));
    }
    rows.sort();
    super::cap_recording(cx, rows.join("\n"), "tree_cap")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtok_plugin_sdk::Ctx;
    fn cx(name: &str) -> crate::plugin::Runtime {
        crate::testutil::runtime(name).0
    }

    #[test]
    fn text_file_consumers_share_the_http_secret_policy() {
        let dir = crate::testutil::tmp_dir("text-file-secret-policy");
        for name in ["public.txt", ".env", "server.key", ".env.example"] {
            fs::write(dir.join(name), "needle\n").unwrap();
        }
        let names = |deny| {
            let walk = WalkBuilder::new(&dir).hidden(false).build().flatten();
            let mut names: Vec<_> = text_files_with_policy(walk, 1024, deny)
                .map(|(path, _)| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };
        assert_eq!(
            names(false),
            [".env", ".env.example", "public.txt", "server.key"]
        );
        assert_eq!(names(true), [".env.example", "public.txt"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn search_fn_main_finds_src_main() {
        let cx = cx("fnmain");
        let out = search(&Ctx::new(&cx), "fn main", ".", None).unwrap();
        assert!(out.contains("src/main.rs"), "{out}");
        let n = out.lines().count();
        assert!(n <= cx.config.plugins.read.search_max as usize, "{n}");
    }

    /// Audit 2026-10-19: over HTTP `read` refuses `.env`; a directory `search` used to print
    /// its lines anyway. The filter drops credential files only when they are refused.
    #[test]
    fn walk_filter_drops_credential_files_only_when_denied() {
        let dir = crate::testutil::tmp_dir("search-secrets");
        fs::create_dir_all(dir.join("k")).unwrap();
        fs::write(dir.join(".env"), "API_TOKEN=x\n").unwrap();
        fs::write(dir.join("k/server.pem"), "API_TOKEN=x\n").unwrap();
        fs::write(dir.join("a.txt"), "API_TOKEN=x\n").unwrap();
        let names = |deny: bool| {
            let mut walk = WalkBuilder::new(&dir);
            walk.hidden(false).filter_entry(walk_filter(deny));
            let mut got: Vec<String> = text_files(walk.build().filter_map(Result::ok), 1 << 20)
                .map(|(p, _)| {
                    p.strip_prefix(&dir)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                .collect();
            got.sort();
            got
        };
        assert_eq!(names(true), ["a.txt"]);
        assert_eq!(names(false), [".env", "a.txt", "k/server.pem"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_respects_max() {
        let cx = cx("max");
        let out = search(&Ctx::new(&cx), "the", ".", Some(3)).unwrap();
        assert!(out.lines().count() <= 3, "{out}");
    }

    #[test]
    fn display_rel_names_the_root_file_instead_of_empty() {
        let root = Path::new("/repo");
        let got = display_rel(Path::new("/repo"), root, Path::new("/repo"));
        assert_eq!(got, "repo");
    }

    /// Same grammar as `expand --grep`: `read(` is a literal search, not an error.
    #[test]
    fn invalid_regex_searches_literally() {
        let (rt, dir) = crate::plugins::read::tests::cx("litsearch");
        std::fs::write(dir.join("a.txt"), "read(x) literally\n").unwrap();
        let out = search(&Ctx::new(&rt), "read(", dir.to_str().unwrap(), None).unwrap();
        assert!(out.contains("a.txt:1:"), "{out}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    // WalkBuilder host-disk e2e — kept; Vfs twins use walk::WalkFs (T56.4).
    fn search_paths_stay_relative_for_allow_paths_root() {
        let (rt, dir) = crate::plugins::read::tests::cx("searchrel");
        let nested = dir.join("nest");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("hit.rs"), "fn needle() {}\n").unwrap();
        let out = search(&Ctx::new(&rt), "needle", nested.to_str().unwrap(), None).unwrap();
        assert!(
            out.contains("hit.rs:"),
            "expected path relative to allow_paths root, got {out}"
        );
        let abs = nested.to_string_lossy();
        assert!(
            !out.contains(abs.as_ref()),
            "must not echo the absolute search root: {out}"
        );
    }

    /// `.git/` is never walked: no hit from a reflog, no `.git/objects` rows in a tree.
    #[test]
    fn search_and_tree_skip_git_dir() {
        let (rt, dir) = crate::plugins::read::tests::cx("skipgit");
        let git = dir.join(".git").join("logs");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("HEAD"), "needle in reflog\n").unwrap();
        fs::write(dir.join("a.txt"), "needle in tree\n").unwrap();
        let cx = Ctx::new(&rt);
        let out = search(&cx, "needle", dir.to_str().unwrap(), None).unwrap();
        assert!(out.contains("a.txt:1:") && !out.contains(".git"), "{out}");
        let out = tree(&cx, dir.to_str().unwrap(), Some(3)).unwrap();
        assert!(out.contains("a.txt") && !out.contains(".git"), "{out}");
        let _ = fs::remove_dir_all(dir);
    }

    /// T660.3: a linked worktree nested in the tree (one under `.rtok/worktrees`) is another
    /// checkout's files: no hit and no row from it, unless it is the walk root itself. A
    /// submodule and a nested clone are still searched.
    #[test]
    fn search_and_tree_skip_a_nested_linked_worktree() {
        let (rt, dir) = crate::plugins::read::tests::cx("skipnested");
        let nested = crate::testutil::linked_worktree(&dir, ".rtok/worktrees/x");
        let sub = crate::testutil::submodule(&dir, "mods/sub");
        let clone = dir.join("deps/clone");
        fs::create_dir_all(clone.join(".git")).unwrap();
        fs::write(nested.join("n.txt"), "needle in nested\n").unwrap();
        fs::write(sub.join("s.txt"), "needle in submodule\n").unwrap();
        fs::write(clone.join("c.txt"), "needle in clone\n").unwrap();
        fs::write(dir.join(".rtok/k.txt"), "needle in rtok\n").unwrap();
        fs::write(dir.join("a.txt"), "needle in tree\n").unwrap();
        let cx = Ctx::new(&rt);
        let out = search(&cx, "needle", dir.to_str().unwrap(), None).unwrap();
        for kept in ["a.txt:1:", "k.txt:1:", "s.txt:1:", "c.txt:1:"] {
            assert!(out.contains(kept), "{kept} missing: {out}");
        }
        assert!(!out.contains("n.txt"), "{out}");
        let out = tree(&cx, dir.to_str().unwrap(), Some(5)).unwrap();
        for kept in ["a.txt", "k.txt", "s.txt", "c.txt"] {
            assert!(out.contains(kept), "{kept} missing: {out}");
        }
        assert!(!out.contains("n.txt"), "{out}");
        let out = search(&cx, "needle", nested.to_str().unwrap(), None).unwrap();
        assert!(out.contains("n.txt:1:"), "{out}");
        let _ = fs::remove_dir_all(dir);
    }

    /// Oversized output is archived like `read`, so `max`/`depth` cannot flood the context.
    #[test]
    fn search_output_is_capped_with_archive_id() {
        let (mut c, dir) = crate::testutil::config("searchcap");
        c.plugins.read.allow_paths = vec![dir.clone()];
        c.plugins.read.max_chars = 200;
        let rt = crate::plugin::Runtime::open(c, "searchcap").unwrap();
        let body = "needle line\n".repeat(100);
        fs::write(dir.join("big.txt"), body).unwrap();
        let out = search(&Ctx::new(&rt), "needle", dir.to_str().unwrap(), Some(100)).unwrap();
        assert!(out.contains("archived"), "{out}");
        assert!(out.chars().count() <= 200, "{}", out.chars().count());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn tree_paths_stay_relative_for_allow_paths_root() {
        let (rt, dir) = crate::plugins::read::tests::cx("treerel");
        let nested = dir.join("nest");
        fs::create_dir_all(nested.join("a")).unwrap();
        fs::write(nested.join("a").join("f.txt"), "x").unwrap();
        let out = tree(&Ctx::new(&rt), nested.to_str().unwrap(), Some(3)).unwrap();
        assert!(out.contains("a"), "{out}");
        let abs = nested.to_string_lossy();
        assert!(
            !out.contains(abs.as_ref()),
            "must not echo the absolute tree root: {out}"
        );
    }

    /// T55.1: when only ASCII case differs, hits stay relative (Windows residual).
    #[test]
    fn display_rel_strips_ascii_case_insensitive_prefix() {
        // Pure path logic — no host TempDir (plan VFS / T56).
        let path = Path::new(r"C:\Users\Me\Proj\src\a.rs");
        let root = Path::new(r"c:\users\me\proj");
        let cwd = Path::new(r"C:\Users\Me\Proj");
        let rel = display_rel(path, root, &canonical_base(cwd));
        if cfg!(windows) {
            assert_eq!(rel.replace('/', "\\"), r"src\a.rs");
        } else {
            // On Unix Path::strip_prefix is case-sensitive; mixed-case paths do not strip.
            assert!(rel.contains("a.rs"), "{rel}");
        }
    }

    /// T55.5: files over `search_max_bytes` are never loaded (WalkBuilder e2e; pure gate in Vfs above).
    #[test]
    fn search_skips_files_over_search_max_bytes() {
        let (mut c, dir) = crate::testutil::config("searchcapbytes");
        c.plugins.read.allow_paths = vec![dir.clone()];
        c.plugins.read.search_max_bytes = 64;
        let rt = crate::plugin::Runtime::open(c, "searchcapbytes").unwrap();
        fs::write(dir.join("small.txt"), "needle small\n").unwrap();
        let large = dir.join("large.txt");
        fs::write(&large, "needle large\n").unwrap();
        let f = fs::OpenOptions::new().write(true).open(&large).unwrap();
        f.set_len(128).unwrap();
        drop(f);
        let out = search(&Ctx::new(&rt), "needle", dir.to_str().unwrap(), None).unwrap();
        assert!(out.contains("small.txt"), "{out}");
        assert!(
            !out.contains("large.txt"),
            "oversized file must be skipped: {out}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// T56.4: thin wrappers over [`super::walk`] so twins share the real WalkFs adapter.
    fn search_hits_from_vfs(
        vfs: &crate::testutil::Vfs,
        pattern: &str,
        max_bytes: u64,
        max_hits: usize,
    ) -> Vec<String> {
        crate::plugins::read::walk::search_hits(vfs, "", pattern, max_bytes, max_hits)
    }

    fn tree_rows_from_vfs(
        vfs: &crate::testutil::Vfs,
        root_prefix: &str,
        max_depth: usize,
    ) -> Vec<String> {
        crate::plugins::read::walk::tree_rows(vfs, root_prefix, max_depth)
    }

    #[test]
    fn search_max_bytes_gate_uses_vfs_sizes() {
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write("ok.txt", b"needle small\n");
        vfs.write("big.txt", {
            let mut b = b"needle large\n".to_vec();
            b.resize(200, b'x');
            b
        });
        vfs.write("other.txt", b"no match\n");
        let hits = search_hits_from_vfs(&vfs, "needle", 64, 10);
        assert_eq!(hits, vec!["ok.txt:1: needle small".to_string()]);
    }

    /// T56.2: relative display path + hit formatting without host TempDir.
    #[test]
    fn vfs_search_hit_paths_stay_basename_relative() {
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write("nest/hit.rs", b"fn needle() {}\n");
        let hits = search_hits_from_vfs(&vfs, "needle", 1024, 10);
        assert_eq!(hits, vec!["nest/hit.rs:1: fn needle() {}".to_string()]);
    }

    #[test]
    fn vfs_search_respects_max_hits_across_files() {
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write("a.txt", b"needle\nneedle\n");
        vfs.write("b.txt", b"needle\n");
        let hits = search_hits_from_vfs(&vfs, "needle", 1024, 2);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(hits.iter().all(|h| h.contains("needle")), "{hits:?}");
    }

    #[test]
    fn vfs_search_skips_non_utf8_bodies() {
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write("bin.dat", [0xff, 0xfe, 0x00]);
        vfs.write("ok.txt", b"needle here\n");
        let hits = search_hits_from_vfs(&vfs, "needle", 1024, 10);
        assert_eq!(hits, vec!["ok.txt:1: needle here".to_string()]);
    }

    #[test]
    fn vfs_search_empty_is_empty() {
        let vfs = crate::testutil::Vfs::new();
        assert!(search_hits_from_vfs(&vfs, "needle", 1024, 10).is_empty());
    }

    #[test]
    fn vfs_search_spaced_windows_style_keys() {
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write(r"C:/Users/Ivan Tuhai/proj/src/a.rs", b"fn needle() {}\n");
        let hits = search_hits_from_vfs(&vfs, "needle", 1024, 10);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].contains("Ivan Tuhai"), "{hits:?}");
        assert!(hits[0].contains("a.rs:1:"), "{hits:?}");
    }

    /// T56.2 twin of `search_and_tree_skip_git_dir` — keep disk e2e; Vfs skips `.git/` keys.
    #[test]
    fn search_and_tree_skip_git_dir_from_vfs() {
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write(".git/logs/HEAD", b"needle in reflog\n");
        vfs.write("a.txt", b"needle in tree\n");
        let hits = search_hits_from_vfs(&vfs, "needle", 1024, 10);
        assert_eq!(hits, vec!["a.txt:1: needle in tree".to_string()]);
        let rows = tree_rows_from_vfs(&vfs, "", 3);
        assert!(rows.iter().any(|r| r.starts_with("a.txt ")), "{rows:?}");
        assert!(
            rows.iter().all(|r| !r.contains(".git")),
            "tree must skip .git: {rows:?}"
        );
    }

    /// T56.2 twin of `tree_paths_stay_relative_for_allow_paths_root`.
    #[test]
    fn tree_paths_stay_relative_from_vfs() {
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write("nest/a/f.txt", b"x");
        let rows = tree_rows_from_vfs(&vfs, "nest", 3);
        assert!(
            rows.iter().any(|r| r.starts_with("a/f.txt ")),
            "expected path relative to walk root, got {rows:?}"
        );
        assert!(
            rows.iter().all(|r| !r.contains("nest/")),
            "must not echo the absolute/search-root prefix: {rows:?}"
        );
    }

    /// T56.2/T56.4 twin of `search_paths_stay_relative_for_allow_paths_root` via WalkFs root.
    #[test]
    fn search_paths_stay_relative_from_vfs() {
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write("nest/hit.rs", b"fn needle() {}\n");
        let hits = crate::plugins::read::walk::search_hits(&vfs, "nest", "needle", 1024, 10);
        assert!(
            hits.iter().any(|h| h.starts_with("hit.rs:")),
            "expected path relative to allow_paths root, got {hits:?}"
        );
        assert!(
            hits.iter().all(|h| !h.contains("nest/")),
            "must not echo the nested root: {hits:?}"
        );
    }

    /// T56.2 twin of `search_skips_files_over_search_max_bytes` (already covered by size gate; assert twin name).
    #[test]
    fn search_skips_files_over_search_max_bytes_from_vfs() {
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write("small.txt", b"needle small\n");
        vfs.write("large.txt", {
            let mut b = b"needle large\n".to_vec();
            b.resize(128, b'x');
            b
        });
        let hits = search_hits_from_vfs(&vfs, "needle", 64, 10);
        assert_eq!(hits, vec!["small.txt:1: needle small".to_string()]);
    }

    /// Pure path: display_rel with spaced profile (T55/T56 edge).
    #[test]
    fn display_rel_keeps_spaced_segment_when_prefix_matches() {
        let path = Path::new(r"C:\Users\Ivan Tuhai\proj\src\a.rs");
        let root = Path::new(r"C:\Users\Ivan Tuhai\proj");
        let cwd = Path::new(r"C:\Users\Ivan Tuhai\proj");
        let rel = display_rel(path, root, &canonical_base(cwd));
        assert!(rel.contains("a.rs"), "{rel}");
        // When strip works (Windows case fold or exact match), stay relative.
        if cfg!(windows) || path.starts_with(root) {
            assert!(
                !rel.contains("Ivan Tuhai") || rel.starts_with("src"),
                "{rel}"
            );
        }
    }

    /// T59.2: one canonicalization per call — `search`/`tree` resolve the base once
    /// on the adapter and per-row `display_rel` never touches the FS.
    #[test]
    fn display_rel_canonicalizes_base_once_per_call_from_vfs() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use crate::plugins::read::fs::ReadFs;

        struct CountingFs<'a> {
            fs: &'a crate::testutil::Vfs,
            calls: &'a AtomicUsize,
        }
        impl ReadFs for CountingFs<'_> {
            fn read_to_string(&self, path: &Path) -> std::io::Result<String> {
                ReadFs::read_to_string(self.fs, path)
            }
            fn canonicalize(&self, path: &Path) -> Option<PathBuf> {
                self.calls.fetch_add(1, Ordering::Relaxed);
                ReadFs::canonicalize(self.fs, path)
            }
        }

        let mut vfs = crate::testutil::Vfs::new();
        vfs.write("src/a.rs", b"fn main() {}\n");
        vfs.write("src/b.rs", b"fn other() {}\n");
        let calls = AtomicUsize::new(0);
        let cfs = CountingFs {
            fs: &vfs,
            calls: &calls,
        };

        // The call: the base is canonicalized exactly once…
        let base = canonical_base_with(&cfs, Path::new("src"));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(base, PathBuf::from("src"));

        // …then any number of rows display without another adapter call.
        assert_eq!(
            display_rel(Path::new("src/a.rs"), Path::new("src"), &base),
            "a.rs"
        );
        let _ = display_rel(Path::new("src/b.rs"), Path::new("src"), &base);
        let _ = display_rel(Path::new("elsewhere/x.rs"), Path::new("src"), &base);
        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "per-row display must not canonicalize"
        );
    }
}
