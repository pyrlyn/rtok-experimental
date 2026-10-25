// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T978: Claude Code's workspace trust for the folder `rtok doctor` runs in.
//!
//! A folder whose trust dialog was never accepted shows "This workspace isn't trusted", and
//! there Claude Code and the desktop Code tab skip the project's hooks and MCP servers, so rtok
//! silently does nothing. Claude Code keys the trust on the git repository root (a worktree on
//! its main checkout's root) and, outside a repository, on the folder it started in, covering
//! its subfolders; a session started in the home folder keeps the trust in memory only
//! (`research.md` §48).
//!
//! Read-only: trusting a folder is the user's security decision, so the finding is never
//! fixable. Anything this cannot read or parse is no finding.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::hooks::{Probes, Problem};
use super::probe::PathKind;
use crate::config::Config;

/// The one finding: Claude Code has not trusted the folder `doctor` runs in, or none.
pub fn check(cfg: &Config, p: &Probes) -> Vec<Problem> {
    let Some(cwd) = p.env.cwd().filter(|c| !c.as_os_str().is_empty()) else {
        return Vec::new();
    };
    // Claude Code never writes the home folder's trust to disk, so its absence means nothing.
    if p.env.home().is_some_and(|h| same(p, &h, &cwd)) {
        return Vec::new();
    }
    let source = &cfg.doctor.claude_json;
    let Some(projects) =
        p.fs.read(source)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|v| v.get("projects").cloned())
            .filter(Value::is_object)
    else {
        return Vec::new();
    };
    let accepted = |dir: &Path| {
        [dir.to_path_buf(), p.fs.canonical(dir)].iter().any(|d| {
            let want = slashes(d);
            projects.as_object().is_some_and(|o| {
                o.iter().any(|(k, v)| {
                    slashes(Path::new(k)) == want
                        && v.get("hasTrustDialogAccepted") == Some(&Value::Bool(true))
                })
            })
        })
    };
    let (root, trusted) = match repo_root(p, &cwd) {
        Some(root) => {
            let ok = accepted(&root);
            (root, ok)
        }
        None => (cwd.clone(), cwd.ancestors().any(accepted)),
    };
    if trusted {
        return Vec::new();
    }
    let worktree = if root == cwd {
        String::new()
    } else {
        format!(", whose trust {} takes", cwd.display())
    };
    vec![Problem {
        kind: "untrusted-folder",
        agent: "claude",
        source: source.display().to_string(),
        path: format!("projects[\"{}\"].hasTrustDialogAccepted", slashes(&root)),
        event: String::new(),
        matcher: None,
        command: String::new(),
        detail: format!(
            "Claude Code does not trust {}{worktree}, so it skips rtok's hooks and MCP there; trust it by running `claude` in that folder and accepting the dialog, or by accepting it in the desktop app",
            root.display()
        ),
        fixable: false,
        group: None,
        keep: false,
    }]
}

/// The folder Claude Code keys a repository's trust on: the main checkout's root for a linked
/// worktree, the repository's own root otherwise (a submodule included). `None` outside one.
fn repo_root(p: &Probes, cwd: &Path) -> Option<PathBuf> {
    for dir in cwd.ancestors() {
        let dot = dir.join(".git");
        match p.fs.kind(&dot) {
            PathKind::Dir => return Some(dir.to_path_buf()),
            PathKind::File { .. } => return Some(linked_main(p, dir, &dot).unwrap_or(dir.into())),
            _ => {}
        }
    }
    None
}

/// A `.git` file names the repository's git dir; a linked worktree's has a `commondir` that
/// leads to the main checkout's `.git`. Anything unexpected keeps `dir` itself.
fn linked_main(p: &Probes, dir: &Path, dot: &Path) -> Option<PathBuf> {
    let raw = p.fs.read(dot).ok()?;
    let gitdir = dir.join(raw.trim().strip_prefix("gitdir:")?.trim());
    let common = p.fs.read(&gitdir.join("commondir")).ok()?;
    let common = normal(&gitdir.join(common.trim()));
    (common.file_name()? == ".git").then(|| common.parent().map(Path::to_path_buf))?
}

/// `path` without `.` and `..` parts, by text: `commondir` is usually `../..`.
fn normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn same(p: &Probes, a: &Path, b: &Path) -> bool {
    a == b || p.fs.canonical(a) == p.fs.canonical(b)
}

/// Claude Code writes the key with `/` on every OS.
fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use crate::doctor::probe::{Env, Fs, Which};
    use std::collections::BTreeMap;
    use std::io;

    #[derive(Default)]
    struct Mock {
        files: BTreeMap<PathBuf, String>,
        dirs: Vec<PathBuf>,
        cwd: PathBuf,
    }

    impl Fs for Mock {
        fn read(&self, path: &Path) -> io::Result<String> {
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
        fn kind(&self, path: &Path) -> PathKind {
            if self.dirs.iter().any(|d| d == path) {
                PathKind::Dir
            } else if self.files.contains_key(path) {
                PathKind::File { executable: false }
            } else {
                PathKind::Missing
            }
        }
        fn canonical(&self, path: &Path) -> PathBuf {
            path.to_path_buf()
        }
    }

    impl Env for Mock {
        fn var(&self, _: &str) -> Option<String> {
            None
        }
        fn home(&self) -> Option<PathBuf> {
            Some("/h".into())
        }
        fn cwd(&self) -> Option<PathBuf> {
            Some(self.cwd.clone())
        }
    }

    impl Which for Mock {
        fn find(&self, _: &str) -> Option<PathBuf> {
            None
        }
    }

    fn mock(cwd: &str, trusted: &[(&str, bool)]) -> Mock {
        let projects: serde_json::Map<String, Value> = trusted
            .iter()
            .map(|(k, v)| {
                (
                    (*k).into(),
                    serde_json::json!({ "hasTrustDialogAccepted": v }),
                )
            })
            .collect();
        let mut m = Mock {
            cwd: cwd.into(),
            ..Mock::default()
        };
        m.files.insert(
            "/h/.claude.json".into(),
            serde_json::json!({ "projects": projects }).to_string(),
        );
        m
    }

    fn run(m: &Mock) -> Vec<Problem> {
        let mut cfg = Config::default();
        cfg.doctor.claude_json = "/h/.claude.json".into();
        check(
            &cfg,
            &Probes {
                fs: m,
                env: m,
                which: m,
            },
        )
    }

    #[test]
    fn a_trusted_repository_is_no_finding_from_any_subfolder() {
        let mut m = mock("/r/src/deep", &[("/r", true)]);
        m.dirs.push("/r/.git".into());
        assert!(run(&m).is_empty());
    }

    #[test]
    fn an_untrusted_repository_names_its_root_and_how_to_trust_it() {
        let mut m = mock("/r/src", &[("/r", false), ("/", true)]);
        m.dirs.push("/r/.git".into());
        let found = run(&m);
        assert_eq!(found.len(), 1, "{found:?}");
        let x = &found[0];
        assert_eq!(
            (x.kind, x.agent, x.fixable),
            ("untrusted-folder", "claude", false)
        );
        assert_eq!(x.path, r#"projects["/r"].hasTrustDialogAccepted"#);
        assert!(
            x.detail.contains("does not trust /r") && x.detail.contains("run"),
            "{x:?}"
        );
    }

    #[test]
    fn a_linked_worktree_uses_the_main_checkouts_trust() {
        let mut m = mock("/w/t1/src", &[("/r", true)]);
        m.dirs.push("/r/.git".into());
        m.files
            .insert("/w/t1/.git".into(), "gitdir: /r/.git/worktrees/t1\n".into());
        m.files
            .insert("/r/.git/worktrees/t1/commondir".into(), "../..\n".into());
        assert!(run(&m).is_empty());
        let mut untrusted = mock("/w/t1", &[("/w/t1", true)]);
        untrusted.files = m.files.clone();
        untrusted.files.insert(
            "/h/.claude.json".into(),
            r#"{"projects":{"/w/t1":{"hasTrustDialogAccepted":true}}}"#.into(),
        );
        let found = run(&untrusted);
        assert_eq!(found[0].path, r#"projects["/r"].hasTrustDialogAccepted"#);
    }

    #[test]
    fn outside_a_repository_a_trusted_parent_covers_the_folder() {
        assert!(run(&mock("/p/a/b", &[("/p", true)])).is_empty());
        assert_eq!(
            run(&mock("/p/a/b", &[("/q", true)]))[0].kind,
            "untrusted-folder"
        );
    }

    #[test]
    fn missing_or_broken_config_and_the_home_folder_are_no_finding() {
        let mut m = mock("/p", &[]);
        m.files.clear();
        assert!(run(&m).is_empty());
        m.files.insert("/h/.claude.json".into(), "{ nope".into());
        assert!(run(&m).is_empty());
        assert!(run(&mock("/h", &[])).is_empty());
    }
}
