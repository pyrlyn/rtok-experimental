// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `git` against scratch repositories (T150, T290): fixed identity, no signing, default branch `main`.

use std::path::Path;
use std::process::Command;

pub fn run(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .expect("git runs");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "git {args:?}: {err}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn commit(dir: &Path, file: &str) {
    std::fs::write(dir.join(file), file).unwrap();
    run(dir, &["add", file]);
    run(dir, &["commit", "-q", "-m", file]);
}

/// A bare origin, a clone named `rtok` with one pushed commit, and the `_worktrees` root, all
/// under `tmp`; the clone's canonical path.
pub fn repo(tmp: &Path) -> std::path::PathBuf {
    run(tmp, &["init", "-q", "--bare", "origin.git"]);
    run(tmp, &["clone", "-q", "origin.git", "rtok"]);
    let work = tmp.join("rtok");
    commit(&work, "a.txt");
    run(&work, &["push", "-q", "-u", "origin", "main"]);
    run(&work, &["remote", "set-head", "origin", "main"]);
    std::fs::create_dir_all(tmp.join("_worktrees")).unwrap();
    work.canonicalize().unwrap()
}
