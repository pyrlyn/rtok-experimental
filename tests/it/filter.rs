// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T10.2: `rtok filter --cmd` reads stdin; OpenCode plugin mock.

use crate::it::common;

use rstest::rstest;
use std::io::Write;
use std::process::{Command, Stdio};

#[rstest]
fn printf_git_status_returns_filtered_text() {
    let home = std::env::temp_dir().join(format!("rtok-filter-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();

    let bin = crate::it::common::bin_path("rtok");
    let mut child = Command::new(bin)
        .args(["filter", "--cmd", "git status"])
        .env("RTOK_HOME", &home)
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        stdin
            .write_all(
                b"On branch main\nChanges not staged for commit:\n  (use \"git add <file>...\" to update what will be committed)\n  (use \"git restore <file>...\" to discard changes in working directory)\n\tmodified:   src/lib.rs\n\tmodified:   src/main.rs\n\tmodified:   src/plugins/cmd/run.rs\n\nUntracked files:\n  (use \"git add <file>...\" to include in what will be committed)\n\tnotes.txt\n\nno changes added to commit (use \"git add\" and/or \"git commit -a\")\n",
            )
            .unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("On branch main"), "{s}");
    assert!(
        s.contains(" M src/{lib.rs,main.rs} src/plugins/cmd/run.rs"),
        "{s}"
    );
    assert!(!s.contains("Changes not staged"), "{s}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "host plugin tests run on Linux only"
)]
fn opencode_plugin_unit_test_with_api_mock() {
    common::vitest("plugins/opencode/rtok.test.ts", &[]);
}

/// T360: one invalid UTF-8 byte used to empty the whole result.
#[rstest]
#[case::default(&["filter"][..])]
#[case::stdin_flag(&["filter", "--stdin"][..])]
fn invalid_utf8_byte_keeps_the_rest_of_stdin(#[case] args: &[&str]) {
    let home = std::env::temp_dir().join(format!(
        "rtok-filter-utf8-{}-{}",
        std::process::id(),
        args.len()
    ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();

    let mut child = Command::new(crate::it::common::bin_path("rtok"))
        .args(args)
        .env("RTOK_HOME", &home)
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"a\xffb\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("a\u{FFFD}b"), "{s:?}");
    let _ = std::fs::remove_dir_all(&home);
}
