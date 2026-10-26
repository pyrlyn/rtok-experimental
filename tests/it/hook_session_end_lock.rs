// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T958: SessionEnd releases the git lock that names the agent that just ended,
//! and no other lock.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::common;

use common::git::{self, repo};

fn hook(
    home: &Path,
    db: &Path,
    event: &str,
    session: &str,
    cwd: &Path,
    agent_id: Option<&str>,
) -> std::process::Output {
    let mut payload = serde_json::json!({
        "hook_event_name": event,
        "session_id": session,
        "cwd": cwd,
        "reason": "other",
    });
    if let Some(id) = agent_id {
        payload["agent_id"] = serde_json::Value::String(id.into());
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .args(["hook", event])
        .current_dir(cwd)
        .env("HOME", home)
        .env("RTOK_HOME", home)
        .env("RTOK_CORE_DB_PATH", db)
        .env("RTOK_PLUGINS_GRAPH_AUTO_ADD_PROJECTS", "false")
        .env_remove("RTOK_AGENT_ID")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("rtok spawns");
    use std::io::Write as _;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    child.wait_with_output().expect("rtok runs")
}

fn agents(db: &Path, session: &str) -> Vec<rtok::store::AgentDetail> {
    rtok::store::Store::open(db)
        .unwrap()
        .agents_of_sessions(&[session.to_string()])
        .unwrap()
}

fn lock(repo: &Path, worktree: &Path, agent: &str) {
    let reason = format!("Grok / grok-4.7 | t958 | 2026-10-22 | agent {agent}");
    git::run(
        repo,
        &[
            "worktree",
            "lock",
            "--reason",
            &reason,
            worktree.to_str().unwrap(),
        ],
    );
}

fn locked(worktree: &Path) -> bool {
    let gitdir = std::fs::read_to_string(worktree.join(".git")).unwrap();
    let admin = PathBuf::from(gitdir.trim().strip_prefix("gitdir:").unwrap().trim());
    admin.join("locked").is_file()
}

/// SessionEnd for A unlocks A's worktree and leaves B's lock. gc's dry run then
/// plans to remove A's merged, clean, idle worktree.
#[rstest::rstest]
fn session_end_unlocks_that_agents_worktree_and_gc_drops_it(#[values(false, true)] busy: bool) {
    let tmp = rtok::testutil::tmp_dir("t958-gc");
    let home = tmp.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let db = home.join("rtok.db");
    let work = repo(&tmp);
    let add = |name: &str| {
        git::run(
            &work,
            &["worktree", "add", "-q", "-b", name, &format!("../{name}")],
        );
        tmp.join(name).canonicalize().unwrap()
    };
    let wt_a = add("wt-a");
    let wt_b = add("wt-b");

    let end = hook(&home, &db, "SessionStart", "t958-a", &wt_a, None);
    assert!(
        end.status.success(),
        "{}",
        String::from_utf8_lossy(&end.stderr)
    );
    let end = hook(&home, &db, "SessionStart", "t958-b", &wt_b, None);
    assert!(
        end.status.success(),
        "{}",
        String::from_utf8_lossy(&end.stderr)
    );
    let id = |session: &str| {
        agents(&db, session)
            .into_iter()
            .find(|a| a.parent_id.is_none())
            .unwrap()
            .id
    };
    let (a, b) = (id("t958-a"), id("t958-b"));
    lock(&work, &wt_a, &a);
    lock(&work, &wt_b, &b);
    assert!(locked(&wt_a) && locked(&wt_b));

    let holder = busy.then(|| common::hold_store_writer(&db, Duration::from_millis(600)));
    let end = hook(&home, &db, "SessionEnd", "t958-a", &wt_a, None);
    assert!(
        end.status.success(),
        "{}",
        String::from_utf8_lossy(&end.stderr)
    );
    if let Some(holder) = holder {
        holder.join().unwrap();
    }
    // A busy store hands SessionEnd to a detached child; the hook can exit before it unlocks.
    let deadline = Instant::now() + common::scaled(Duration::from_secs(10));
    while locked(&wt_a) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        !locked(&wt_a),
        "A's lock is released: {}",
        String::from_utf8_lossy(&end.stderr)
    );
    assert!(locked(&wt_b), "B's lock stays");

    let out = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .args(["worktree", "gc", "--json", "--idle", "0h"])
        .current_dir(&work)
        .env("HOME", &home)
        .env("RTOK_HOME", &home)
        .env("RTOK_CORE_DB_PATH", &db)
        .env_remove("RTOK_AGENT_ID")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
    let row = |name: &str| {
        rows.iter()
            .find(|r| r["path"].as_str().unwrap().ends_with(name))
            .unwrap_or_else(|| panic!("{name} missing in {rows:?}"))
            .clone()
    };
    assert_eq!(row("wt-a")["action"], "remove");
    assert_eq!(row("wt-a")["note"], "merged, clean and idle");
    assert_eq!(row("wt-b")["action"], "keep");
    assert!(wt_a.is_dir(), "a dry run removes nothing");
}

/// A sub-agent's SessionEnd in the parent's worktree leaves the parent's lock.
#[test]
fn a_subagents_session_end_leaves_the_parents_lock() {
    let tmp = rtok::testutil::tmp_dir("t958-sub");
    let home = tmp.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let db = home.join("rtok.db");
    let work = repo(&tmp);
    git::run(
        &work,
        &["worktree", "add", "-q", "-b", "wt-parent", "../wt-parent"],
    );
    let wt = tmp.join("wt-parent").canonicalize().unwrap();

    let start = hook(&home, &db, "SessionStart", "t958-parent", &wt, None);
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );
    let parent = agents(&db, "t958-parent")
        .into_iter()
        .find(|a| a.parent_id.is_none())
        .unwrap();
    lock(&work, &wt, &parent.id);

    let end = hook(&home, &db, "SessionEnd", "t958-parent", &wt, Some("sub-1"));
    assert!(
        end.status.success(),
        "{}",
        String::from_utf8_lossy(&end.stderr)
    );
    assert!(locked(&wt), "the parent's lock stays");
    let rows = agents(&db, "t958-parent");
    let parent = rows.iter().find(|a| a.parent_id.is_none()).unwrap();
    let child = rows.iter().find(|a| a.parent_id.is_some()).unwrap();
    assert!(parent.ended_at.is_none(), "the parent is still live");
    assert!(child.ended_at.is_some(), "the sub-agent ended");
}
