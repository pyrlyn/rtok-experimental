// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T631: the production `build.rs` in a scratch package. The embedded git sha must follow HEAD
//! across incremental builds, in an ordinary checkout and in a linked worktree, where `.git` is
//! a file and the refs live in the common directory.

use std::path::Path;
use std::process::Command;

use crate::it::common;

use common::git::run;

const MAIN_RS: &str = "fn main() { println!(\"{}\", env!(\"RTOK_GIT_SHA\")); }\n";
const MANIFEST: &str =
    "[package]\nname = \"probe\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n";

fn package(dir: &Path) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), MANIFEST).unwrap();
    std::fs::write(dir.join("src/main.rs"), MAIN_RS).unwrap();
    let build = Path::new(env!("CARGO_MANIFEST_DIR")).join("build.rs");
    std::fs::copy(build, dir.join("build.rs")).unwrap();
    run(dir, &["init", "-q"]);
    run(dir, &["add", "."]);
    run(dir, &["commit", "-q", "-m", "probe"]);
}

fn embedded(pkg: &Path, target: &Path) -> String {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = Command::new(cargo)
        .args(["run", "-q"])
        .current_dir(pkg)
        .env("CARGO_TARGET_DIR", target)
        .env_remove("RTOK_WEB_EMBED")
        .output()
        .expect("cargo runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn head(dir: &Path) -> String {
    run(dir, &["rev-parse", "--short=9", "HEAD"])
        .trim()
        .to_string()
}

fn advance(dir: &Path) {
    run(dir, &["commit", "-q", "--allow-empty", "-m", "next"]);
}

#[test]
fn the_sha_follows_commits_in_an_ordinary_checkout() {
    let tmp = rtok::testutil::tmp_dir("build-sha");
    let (pkg, target) = (tmp.join("pkg"), tmp.join("target"));
    package(&pkg);
    assert_eq!(embedded(&pkg, &target), head(&pkg));
    advance(&pkg);
    assert_eq!(embedded(&pkg, &target), head(&pkg));
}

#[test]
fn the_sha_follows_commits_and_detached_checkouts_in_a_linked_worktree() {
    let tmp = rtok::testutil::tmp_dir("build-sha-wt");
    let (main, wt, target) = (tmp.join("main"), tmp.join("wt"), tmp.join("target"));
    package(&main);
    run(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            wt.to_str().unwrap(),
        ],
    );
    let first = head(&wt);
    assert_eq!(embedded(&wt, &target), first);
    advance(&wt);
    assert_eq!(embedded(&wt, &target), head(&wt), "a commit on the branch");
    run(&wt, &["checkout", "-q", "--detach", &first]);
    assert_eq!(embedded(&wt, &target), first, "a detached checkout");
}
