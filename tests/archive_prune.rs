// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T968: `rtok archive prune` and the `rtok info` warning, through the binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-t968-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn rtok(args: &[&str], home: &Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_rtok"))
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

/// Two 600 KiB bodies archived by a session that was last active a month ago, under a
/// `max_archive_mb = 1` limit. Returns the archive dir.
fn seed(home: &Path) -> PathBuf {
    fs::write(home.join("config.toml"), "[core]\nmax_archive_mb = 1\n").unwrap();
    let cfg = rtok::config::Config::load_from(home).expect("config");
    let store = rtok::store::Store::open(&cfg.core.db_path).expect("store");
    store
        .upsert_session("idle", None, None, None, Some("proxy"))
        .unwrap();
    let call = store
        .insert_call("idle", "proxy", "api_request", None, None, None, None, None)
        .unwrap();
    store.set_call_ts(call, 1).unwrap();
    for tag in *b"ab" {
        store
            .put_archive("idle", &vec![tag; 600 * 1024], &cfg.core.archive_dir)
            .unwrap();
    }
    cfg.core.archive_dir
}

fn bodies(dir: &Path) -> usize {
    fs::read_dir(dir).unwrap().count()
}

#[test]
fn info_warns_over_the_limit_and_prune_dry_run_then_real_brings_it_under() {
    let home = home("prune");
    let archive = seed(&home);
    assert_eq!(bodies(&archive), 2);

    let info = rtok(&["info"], &home);
    assert!(
        info.contains("warning archive") && info.contains("core.max_archive_mb"),
        "{info}"
    );

    let dry = rtok(&["archive", "prune", "--dry-run"], &home);
    assert!(dry.contains("would delete 1 bodies"), "{dry}");
    assert_eq!(bodies(&archive), 2, "a dry run deletes nothing");

    let real = rtok(&["archive", "prune"], &home);
    assert!(real.contains("deleted 1 bodies"), "{real}");
    assert!(real.contains("freed"), "{real}");
    assert_eq!(bodies(&archive), 1);
    assert!(!rtok(&["info"], &home).contains("warning"));
    let _ = fs::remove_dir_all(&home);
}
