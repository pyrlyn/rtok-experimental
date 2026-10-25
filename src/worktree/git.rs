// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The only place the worktree inventory spawns git.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Output;

use anyhow::{Context, Result, bail};
use rtok_sys::proc::ProgressRunner;

use super::{Record, parse_porcelain};

/// `label` is the spinner text of a call a person waits for; `None` is a quick read.
fn spawn(dir: &Path, label: Option<&str>, args: &[&str]) -> Result<Output> {
    let argv = [OsStr::new("-C"), dir.as_os_str()]
        .into_iter()
        .chain(args.iter().map(OsStr::new));
    match label {
        Some(label) => ProgressRunner::new(label, "git", argv),
        None => ProgressRunner::quiet("git", argv),
    }
    .run()
    .with_context(|| format!("cannot run git {}", args.join(" ")))
}

fn git(dir: &Path, args: &[&str]) -> Result<Output> {
    spawn(dir, None, args)
}

fn checked(out: Output, args: &[&str]) -> Result<Output> {
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("git {}: {}", args.join(" "), err.trim());
    }
    Ok(out)
}

fn git_ok(dir: &Path, args: &[&str]) -> Result<Output> {
    checked(git(dir, args)?, args)
}

/// [`git_ok`] with a spinner: a fetch or a checkout can take seconds on a large repository.
fn git_slow(dir: &Path, label: &str, args: &[&str]) -> Result<Output> {
    checked(spawn(dir, Some(label), args)?, args)
}

fn stdout(dir: &Path, args: &[&str]) -> Result<String> {
    let out = git_ok(dir, args)?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

pub fn list(repo: &Path) -> Result<Vec<Record>> {
    let out = git_ok(repo, &["worktree", "list", "--porcelain", "-z"])?;
    Ok(parse_porcelain(&out.stdout))
}

/// Modified, staged or untracked files; ignored files (build output) do not count.
pub fn is_dirty(worktree: &Path) -> Result<bool> {
    Ok(!stdout(worktree, &["status", "--porcelain"])?.is_empty())
}

/// Whether git ignores `path` (relative to `worktree`): exit 0 of `check-ignore`.
pub fn is_ignored(worktree: &Path, path: &str) -> bool {
    git(worktree, &["check-ignore", "-q", "--", path]).is_ok_and(|o| o.status.success())
}

/// The remote's default branch, `origin/main` when `origin/HEAD` is not set.
pub fn default_base(repo: &Path) -> String {
    stdout(
        repo,
        &["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"],
    )
    .ok()
    .filter(|base| !base.is_empty())
    .unwrap_or_else(|| "origin/main".into())
}

/// The tree `rev` points at.
pub fn tree(repo: &Path, rev: &str) -> Result<String> {
    stdout(repo, &["rev-parse", &format!("{rev}^{{tree}}")])
}

/// Squash-aware: `rev` is merged when merging it into `base` would produce exactly
/// `base`'s tree. `git branch --merged` cannot see this — a squash merge leaves the
/// branch's own commits outside the base's history. `base_tree` is [`tree`] of `base`,
/// resolved once for a whole inventory.
pub fn is_merged(repo: &Path, base: &str, base_tree: Option<&str>, rev: &str) -> Result<bool> {
    let merge = git(repo, &["merge-tree", "--write-tree", base, rev])?;
    match merge.status.code() {
        Some(0) => {}
        // Conflicts: either real divergence, or the base moved on next to the same lines.
        Some(1) => return is_patch_equivalent(repo, base, rev),
        _ => bail!(
            "git merge-tree: {}",
            String::from_utf8_lossy(&merge.stderr).trim()
        ),
    }
    let merged_tree = String::from_utf8_lossy(&merge.stdout);
    let base_tree = base_tree.with_context(|| format!("no tree for {base}"))?;
    Ok(merged_tree.lines().next() == Some(base_tree))
}

/// The second merged signal: the branch's whole diff as one commit already has a
/// patch-equivalent commit in `base` — a squash merge that `base` later edited around,
/// which [`is_merged`]'s trial merge reads as a conflict. Writes one unreferenced
/// commit object, as the trial merge writes a tree.
fn is_patch_equivalent(repo: &Path, base: &str, rev: &str) -> Result<bool> {
    let fork = stdout(repo, &["merge-base", base, rev])?;
    let tree = format!("{rev}^{{tree}}");
    // An identity of its own: `commit-tree` fails where `user.name` is not configured.
    let id = ["-c", "user.name=rtok", "-c", "user.email=rtok@localhost"];
    let squash = ["commit-tree", &tree, "-p", &fork, "-m", "squash"];
    let squashed = stdout(repo, &[&id[..], &squash[..]].concat())?;
    Ok(stdout(repo, &["cherry", base, &squashed])?.starts_with('-'))
}

/// `rev` has commits `base` does not reach: a task branch that did work, never a fresh branch
/// still at the base, which [`is_merged`] reads as merged too.
pub fn has_own_commits(repo: &Path, base: &str, rev: &str) -> Result<bool> {
    let count = stdout(repo, &["rev-list", "--count", &format!("{base}..{rev}")])?;
    Ok(count.parse::<u64>().context("git rev-list --count")? > 0)
}

fn path_arg(path: &Path) -> Result<&str> {
    path.to_str().context("worktree path is not UTF-8")
}

pub fn lock(repo: &Path, worktree: &Path, reason: &str) -> Result<()> {
    let path = path_arg(worktree)?;
    git_ok(repo, &["worktree", "lock", "--reason", reason, path]).map(drop)
}

pub fn unlock(repo: &Path, worktree: &Path) -> Result<()> {
    git_ok(repo, &["worktree", "unlock", path_arg(worktree)?]).map(drop)
}

/// T958: release the lock that names `agent` on the worktree `cwd` is in.
/// One `git worktree unlock`, and only then. A lock that names someone else, or
/// no agent, stays. Missing checkout or lock is not an error: SessionEnd fails open.
pub fn unlock_ended(cwd: &Path, agent: &str) -> Result<()> {
    let Some((root, admin)) = super::locked_by(cwd, agent)
        .and_then(|root| super::checkout_of(&root).map(|(_, admin)| (root, admin)))
    else {
        return Ok(());
    };
    // A linked worktree's `commondir` points at the main `.git`. Unlock from that
    // repository; the main checkout's own lock has no `commondir`.
    let repo = std::fs::read_to_string(admin.join("commondir"))
        .ok()
        .and_then(|common| {
            let git_dir = admin.join(common.trim());
            git_dir.parent().map(std::path::Path::to_path_buf)
        })
        .unwrap_or_else(|| root.clone());
    unlock(&repo, &root)
}

/// `git worktree move`, never forced: git refuses a locked worktree and one with submodules.
pub fn move_to(repo: &Path, from: &Path, to: &Path) -> Result<()> {
    let name = from.file_name().unwrap_or_default().to_string_lossy();
    let label = format!("moving worktree {name}…");
    let args = ["worktree", "move", path_arg(from)?, path_arg(to)?];
    git_slow(repo, &label, &args).map(drop)
}

/// Never `--force`: git refuses a worktree with modified or untracked files. For a
/// record whose directory is gone this drops that one record.
pub fn remove(repo: &Path, worktree: &Path) -> Result<()> {
    let name = worktree.file_name().unwrap_or_default().to_string_lossy();
    let label = format!("removing worktree {name}…");
    git_slow(repo, &label, &["worktree", "remove", path_arg(worktree)?]).map(drop)
}

/// `-D`: the caller has established "merged" squash-aware, which `-d` cannot see.
pub fn delete_branch(repo: &Path, branch: &str) -> Result<()> {
    git_ok(repo, &["branch", "-D", "--", branch]).map(drop)
}

/// Whether the local branch `branch` exists.
pub fn has_branch(repo: &Path, branch: &str) -> bool {
    let name = format!("refs/heads/{branch}");
    git(repo, &["show-ref", "--verify", "-q", &name]).is_ok_and(|out| out.status.success())
}

/// Drops the records of worktrees whose directory is gone; a locked one stays.
pub fn prune(repo: &Path) -> Result<()> {
    git_ok(repo, &["worktree", "prune"]).map(drop)
}

/// From the local remote-tracking ref: no network on this path.
pub fn has_remote_branch(repo: &Path, branch: &str) -> bool {
    let name = format!("refs/remotes/origin/{branch}");
    git(repo, &["show-ref", "--verify", "-q", &name]).is_ok_and(|out| out.status.success())
}

/// `origin/<x>` → `git fetch origin <x>`; a base that is not on `origin` is left as is.
pub fn fetch(repo: &Path, base: &str) -> Result<()> {
    let Some(branch) = base.strip_prefix("origin/") else {
        return Ok(());
    };
    let label = format!("fetching {branch}…");
    git_slow(repo, &label, &["fetch", "--quiet", "origin", branch]).map(drop)
}

/// The paths `git add -A` would stage, deletions left out: a deleted file cannot leak.
/// `--porcelain -z` keeps names raw; a rename's or copy's second field is its source.
pub fn pending_paths(worktree: &Path) -> Result<Vec<String>> {
    let out = git_ok(
        worktree,
        &["status", "--porcelain", "-z", "--untracked-files=all"],
    )?;
    let mut fields = out.stdout.split(|b| *b == 0).filter(|f| f.len() > 3);
    let mut paths = Vec::new();
    while let Some(field) = fields.next() {
        let (xy, path) = field.split_at(3);
        if matches!(xy[0], b'R' | b'C') {
            fields.next();
        }
        if !xy[..2].contains(&b'D') {
            paths.push(String::from_utf8_lossy(path).into_owned());
        }
    }
    Ok(paths)
}

/// `git add -A` and one commit with `message`; the repository's own hooks and identity apply.
pub fn commit_all(worktree: &Path, message: &str) -> Result<()> {
    git_ok(worktree, &["add", "-A"])?;
    git_slow(
        worktree,
        "committing…",
        &["commit", "--quiet", "-m", message],
    )
    .map(drop)
}

/// `git push -u origin <branch>`: the upstream is set so a later plain `git push` lands here.
pub fn push(worktree: &Path, branch: &str) -> Result<()> {
    let label = format!("pushing {branch}…");
    git_slow(
        worktree,
        &label,
        &["push", "--quiet", "-u", "origin", branch],
    )
    .map(drop)
}

/// One locked worktree on a new branch with no upstream: a bare `git push` from it
/// cannot reach the default branch.
pub fn add_locked(repo: &Path, path: &Path, branch: &str, reason: &str, base: &str) -> Result<()> {
    let args = ["worktree", "add", "--quiet", "--lock", "--reason", reason];
    let tail = ["--no-track", "-b", branch, path_arg(path)?, base];
    git_slow(repo, "creating worktree…", &[&args[..], &tail[..]].concat()).map(drop)
}

/// A plain worktree on `branch`, no lock: what a host makes without rtok. An existing branch
/// is checked out rather than recreated.
pub fn add_plain(repo: &Path, path: &Path, branch: &str) -> Result<()> {
    let path = path_arg(path)?;
    let args = if has_branch(repo, branch) {
        ["worktree", "add", "--quiet", path, branch].to_vec()
    } else {
        ["worktree", "add", "--quiet", "-b", branch, path].to_vec()
    };
    git_slow(repo, "creating worktree…", &args).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::tmp_dir;

    #[test]
    fn a_failing_call_carries_gits_own_message() {
        let dir = tmp_dir("wt-git-fail");
        let err = git_ok(&dir, &["rev-parse", "--git-dir"])
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("git rev-parse --git-dir: "), "{err}");
        assert!(!is_dirty(&dir).unwrap_or(false));
    }

    #[test]
    fn a_labelled_call_returns_the_same_output_off_a_terminal() {
        let dir = tmp_dir("wt-git-slow");
        let quiet = git_ok(&dir, &["--version"]).unwrap();
        let slow = git_slow(&dir, "checking git…", &["--version"]).unwrap();
        assert_eq!((slow.stdout, slow.stderr), (quiet.stdout, quiet.stderr));
    }
}
