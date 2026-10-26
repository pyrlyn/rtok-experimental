// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T441.3: task ids stay unique and dense when many processes allocate from one store at once.
//! Threads would share one connection behind the store's mutex and prove nothing, so the test
//! binary re-runs itself as separate processes.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use rtok::store::Store;

const PROCS: u32 = 8;
const PER_PROC: u32 = 25;
const DB_ENV: &str = "RTOK_TEST_TASK_ALLOC_DB";

/// Child mode: allocate [`PER_PROC`] ids from the store the parent names, one per line.
/// Without the variable (a normal test run) there is nothing to do.
#[test]
fn allocate_child() {
    let Some(db) = std::env::var_os(DB_ENV) else {
        return;
    };
    let store = Store::open(&PathBuf::from(db)).unwrap();
    for _ in 0..PER_PROC {
        let id = store.allocate_task_id("github.com/x/y", "R", None).unwrap();
        println!("id={id}");
    }
}

#[test]
fn parallel_processes_get_unique_dense_ids() {
    let dir = std::env::temp_dir().join(format!("rtok-task-alloc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("rtok.db");
    // Migrated once up front, so the children race on allocation rather than on the schema.
    Store::open(&db).unwrap();

    let children: Vec<_> = (0..PROCS)
        .map(|_| {
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "it::task_allocator::allocate_child",
                    "--exact",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(DB_ENV, &db)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();

    let mut ids = Vec::new();
    for child in children {
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "child failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).unwrap();
        ids.extend(
            stdout
                .lines()
                // libtest prints `test allocate_child ... ` without a newline first.
                .filter_map(|l| l.rsplit_once("id=R").map(|(_, n)| n))
                .map(|n| n.parse::<u32>().unwrap()),
        );
    }
    let total = PROCS * PER_PROC;
    assert_eq!(ids.len(), total as usize, "every allocation reported");
    let unique: BTreeSet<u32> = ids.into_iter().collect();
    assert_eq!(unique, (1..=total).collect(), "unique and dense");
    let _ = std::fs::remove_dir_all(&dir);
}
