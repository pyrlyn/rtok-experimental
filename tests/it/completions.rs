// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T53.2: `rtok completions <shell>` renders every supported shell and `rtok man`
//! prints a roff page. Snapshots stay out of here: the man page embeds the build
//! version (git sha), and one shell's completions are snapshotted in
//! `tests/trycmd/completions-bash.toml` instead.

use assert_cmd::Command;
use clap::ValueEnum;
use rtok::completions::Shell;

fn stdout(args: &[&str]) -> Vec<u8> {
    Command::cargo_bin("rtok")
        .unwrap()
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone()
}

#[test]
fn every_shell_renders_non_empty_completions() {
    for shell in Shell::value_variants() {
        let name = shell
            .to_possible_value()
            .expect("shell has a value")
            .get_name()
            .to_string();
        let out = stdout(&["completions", &name]);
        let text = String::from_utf8(out).expect("completions are utf-8");
        assert!(!text.trim().is_empty(), "{name} completions are empty");
        assert!(
            text.contains("rtok"),
            "{name} completions never name the binary"
        );
    }
}

#[test]
fn unknown_shell_is_refused() {
    Command::cargo_bin("rtok")
        .unwrap()
        .args(["completions", "tcsh"])
        .assert()
        .failure();
}

#[test]
fn man_prints_a_roff_page() {
    let text = String::from_utf8(stdout(&["man"])).expect("man is utf-8");
    assert!(text.contains(".TH"), "roff title header, got:\n{text}");
    assert!(
        text.to_lowercase().contains("rtok"),
        "the page names the binary"
    );
    assert!(
        text.contains("completions"),
        "the page documents the new command"
    );
}

/// T416.4: `--install --dry-run` and `--uninstall --dry-run` print the file change and touch
/// nothing; PowerShell's profile line shows as a diff too.
#[test]
fn dry_run_previews_install_and_uninstall_and_writes_nothing() {
    let home = rtok::testutil::tmp_dir("completions-dry-run");
    let run = |args: &[&str]| {
        let out = Command::cargo_bin("rtok")
            .unwrap()
            .env_clear()
            .env("HOME", &home)
            .env("RTOK_HOME", &home)
            .args(args)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        String::from_utf8(out).unwrap()
    };
    let installed = || std::fs::read_dir(&home).unwrap().count();
    let before = installed();

    let text = run(&["completions", "bash", "--install", "--dry-run"]);
    assert!(
        text.contains("+++ b/") && text.contains("bash-completion/completions/rtok"),
        "{text}"
    );
    let text = run(&["completions", "powershell", "--install", "--dry-run"]);
    assert!(
        text.contains("Microsoft.PowerShell_profile.ps1") && text.contains("# rtok completions"),
        "{text}"
    );
    assert_eq!(installed(), before, "a dry run creates nothing");

    run(&["completions", "bash", "--install"]);
    let script = home.join(".local/share/bash-completion/completions/rtok");
    let text = run(&["completions", "bash", "--uninstall", "--dry-run"]);
    assert!(text.contains("- ") && text.contains("1 file"), "{text}");
    assert!(script.exists(), "a dry-run uninstall keeps the script");
    let _ = std::fs::remove_dir_all(&home);
}
