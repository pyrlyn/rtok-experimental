// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T634: `tools/pypi-publish.sh` against synthetic distribution files and fake `git`, `cargo`,
//! `node` and `twine` executables that only log. A `--dry-run` must never upload, whatever else
//! is on the command line, and must keep naming the target it was asked about.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

const VERSION: &str = "1.2.3";
const TEST_URL: &str = "https://test.pypi.org/legacy/";
const FILES: [&str; 4] = [
    "rtok_cli-1.2.3-py3-none-macosx_11_0_arm64.whl",
    "rtok_cli-1.2.3-py3-none-manylinux_2_17_x86_64.whl",
    "rtok_cli-1.2.3-py3-none-win_amd64.whl",
    "rtok_cli-1.2.3.tar.gz",
];

fn fake(bin: &Path, name: &str, body: &str) {
    let path = bin.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The fixture root, with the dist files already in place, and the log twine writes to.
fn fixture(files: &[&str]) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = rtok::testutil::tmp_dir("pypi-publish")
        .canonicalize()
        .unwrap();
    let (bin, dist) = (root.join("bin"), root.join("target/pypi/dist"));
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&dist).unwrap();
    for f in files {
        std::fs::write(dist.join(f), "x").unwrap();
    }
    let log = root.join("twine.log");
    fake(&bin, "git", &format!("echo '{}'", root.display()));
    fake(&bin, "cargo", "echo '{}'");
    // Reads its stdin so the fake cargo upstream of it is not killed by SIGPIPE.
    fake(&bin, "node", &format!("cat >/dev/null; echo {VERSION}"));
    fake(
        &bin,
        "twine",
        &format!("echo \"$*\" >> '{}'", log.display()),
    );
    (root, log)
}

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    twine: Vec<String>,
}

fn publish(root: &Path, log: &Path, args: &[&str]) -> Run {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/pypi-publish.sh");
    let path = format!("{}:/usr/bin:/bin", root.join("bin").display());
    let out = Command::new("bash")
        .arg(script)
        .args(args)
        .env("PATH", path)
        .output()
        .expect("bash runs");
    Run {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        twine: std::fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect(),
    }
}

fn uploads(run: &Run) -> Vec<&String> {
    run.twine
        .iter()
        .filter(|l| l.starts_with("upload"))
        .collect()
}

#[test]
fn a_dry_run_never_uploads_in_any_flag_order() {
    for args in [
        &["--dry-run"][..],
        &["--dry-run", "--testpypi"],
        &["--testpypi", "--dry-run"],
        &["--dry-run", "--dry-run", "--testpypi"],
        &["--testpypi", "--dry-run", "--testpypi"],
    ] {
        let (root, log) = fixture(&FILES);
        let run = publish(&root, &log, args);
        assert_eq!(run.code, Some(0), "{args:?}: {}", run.stderr);
        assert!(
            uploads(&run).is_empty(),
            "{args:?} uploaded: {:?}",
            run.twine
        );
        assert!(run.twine.iter().any(|l| l.starts_with("check")), "{args:?}");
        assert!(
            run.stdout.contains("nothing uploaded"),
            "{args:?}: {}",
            run.stdout
        );
    }
}

#[test]
fn a_dry_run_names_the_target_it_was_asked_about() {
    let (root, log) = fixture(&FILES);
    let test = publish(&root, &log, &["--testpypi", "--dry-run"]);
    assert!(test.stdout.contains("TestPyPI"), "{}", test.stdout);
    let prod = publish(&root, &log, &["--dry-run"]);
    assert!(
        prod.stdout.contains("PyPI") && !prod.stdout.contains("TestPyPI"),
        "{}",
        prod.stdout
    );
}

#[test]
fn a_real_upload_goes_to_the_chosen_repository_only() {
    let (root, log) = fixture(&FILES);
    let test = publish(&root, &log, &["--testpypi"]);
    assert_eq!(test.code, Some(0), "{}", test.stderr);
    let seen = uploads(&test);
    assert_eq!(seen.len(), 1, "{:?}", test.twine);
    assert!(seen[0].contains(TEST_URL), "{seen:?}");

    let (root, log) = fixture(&FILES);
    let prod = publish(&root, &log, &[]);
    assert_eq!(prod.code, Some(0), "{}", prod.stderr);
    let seen = uploads(&prod);
    assert_eq!(seen.len(), 1, "{:?}", prod.twine);
    assert!(!seen[0].contains("--repository-url"), "{seen:?}");
}

#[test]
fn a_failed_validation_never_uploads_and_a_pypi_upload_needs_every_wheel() {
    // A wheel of another version.
    let (root, log) = fixture(&[
        FILES[0],
        FILES[1],
        FILES[2],
        FILES[3],
        "rtok_cli-9.9.9.tar.gz",
    ]);
    for args in [&["--testpypi"][..], &["--dry-run", "--testpypi"], &[]] {
        let run = publish(&root, &log, args);
        assert_eq!(run.code, Some(1), "{args:?}");
        assert!(run.twine.is_empty(), "{args:?}: {:?}", run.twine);
    }
    // The Windows wheel is missing: only PyPI itself refuses.
    let (root, log) = fixture(&[FILES[0], FILES[1], FILES[3]]);
    let prod = publish(&root, &log, &[]);
    assert_eq!(prod.code, Some(1));
    assert!(prod.twine.is_empty(), "{:?}", prod.twine);
    let test = publish(&root, &log, &["--testpypi"]);
    assert_eq!(test.code, Some(0), "{}", test.stderr);
    assert_eq!(uploads(&test).len(), 1);
}
