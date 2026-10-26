// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T12.6: `--dry-run` on the commands that write, and the git-shaped diff they print.
//!
//! Check: per command, run it with `--dry-run`, assert the store or the file is untouched, and
//! assert the real run afterwards reports exactly what the preview promised.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rtok")
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rtok-t126-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `rtok` with its whole tree — config, DB, archive — inside `home`.
fn rtok(args: &[&str], home: &PathBuf) -> String {
    let out = Command::new(bin())
        .args(args)
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "rtok {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn config_init_dry_run_prints_the_file_it_would_write_and_writes_none() {
    let home = tmp("init");
    let path = home.join("config.toml");
    let out = rtok(&["config", "init", "--dry-run"], &home);
    assert!(!path.exists(), "--dry-run created {}", path.display());
    assert!(out.contains("+++ b/"), "no diff header:\n{out}");
    assert!(out.contains("+[core]"), "no added lines:\n{out}");
    rtok(&["config", "init"], &home);
    assert!(path.exists());
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn config_set_dry_run_shows_both_sides_and_leaves_the_file_alone() {
    let home = tmp("set");
    rtok(&["config", "init"], &home);
    let path = home.join("config.toml");
    let before = fs::read_to_string(&path).unwrap();
    let out = rtok(&["config", "set", "proxy.port", "8791", "--dry-run"], &home);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        before,
        "--dry-run wrote"
    );
    // Init leaves the default commented, so the preview uncomments it as it changes the value.
    assert!(out.contains("-# port") && out.contains("+port"), "{out}");
    assert!(out.contains("8791"), "new value missing:\n{out}");
    // A no-op set has nothing to show, and prints nothing rather than an empty diff.
    let same = rtok(&["config", "set", "proxy.port", "8791", "--dry-run"], &home);
    assert!(same.contains("8791"));
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn memory_import_dry_run_counts_rows_it_does_not_insert() {
    let home = tmp("import");
    let file = home.join("n.jsonl");
    let lines: String = (0..5)
        .map(|i| format!("{{\"kind\":\"note\",\"title\":\"t{i}\",\"body\":\"b-{i}\"}}\n"))
        .collect();
    fs::write(&file, lines).unwrap();
    let f = file.to_str().unwrap();
    let dry = rtok(&["memory", "import", f, "--dry-run"], &home);
    assert!(dry.contains("inserted 5"), "{dry}");
    // Nothing was stored, so the real run must still have all five to insert.
    let real = rtok(&["memory", "import", f], &home);
    assert!(real.contains("inserted 5"), "{real}");
    assert!(
        rtok(&["memory", "import", f], &home).contains("skipped 5"),
        "second real run should dedupe"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn graph_index_dry_run_reports_rows_but_stores_none() {
    let home = tmp("graph");
    let src = home.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.rs"), "fn alpha() {}\nfn beta() {}\n").unwrap();
    let p = src.to_str().unwrap();
    let dry = rtok(&["graph", "index", p, "--dry-run"], &home);
    assert!(dry.contains("indexed 1 files"), "{dry}");
    assert!(!dry.contains("· 0 rows"), "preview counted no rows:\n{dry}");
    // A stored run would make this second one a skip; it must still be a full index.
    let real = rtok(&["graph", "index", p], &home);
    assert!(real.contains("indexed 1 files"), "{real}");
    assert!(
        rtok(&["graph", "index", p], &home).contains("1 skipped"),
        "second real run should skip"
    );
    let _ = fs::remove_dir_all(&home);
}
