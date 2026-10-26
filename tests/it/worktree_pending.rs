// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T289.5: `rtok worktree adopt` from a host's post-create script when no single live agent of
//! the pool's host works in the repository parks the claim; the next `adopt` or `SessionStart`
//! inside the worktree completes it, and `gc` leaves it alone meanwhile. Fakes only.

use crate::common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::git::{repo, run};

struct Scene {
    home: PathBuf,
    work: PathBuf,
    made: PathBuf,
    store: rtok::store::Store,
}

/// A repository and a branch-bearing worktree in Cursor's pool, as the host leaves it for its
/// post-create script.
fn scene(name: &str) -> Scene {
    let home = rtok::testutil::tmp_dir(name);
    let work = repo(&home);
    std::fs::create_dir_all(home.join(".rtok")).unwrap();
    let store = rtok::store::Store::open(&home.join(".rtok/rtok.db")).unwrap();
    let made = home.join(".cursor/worktrees/rtok/abc");
    run(
        &work,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "t7-park",
            made.to_str().unwrap(),
        ],
    );
    // The store keeps paths without the Windows `\\?\` prefix that std adds.
    let made = fs_kit::path::canonicalize(&made).unwrap();
    Scene {
        home,
        work,
        made,
        store,
    }
}

fn rtok(s: &Scene, cwd: &Path, args: &[&str], stdin: &str) -> (bool, String, String) {
    use std::io::Write as _;
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .env_remove("RTOK_AGENT_ID")
        .env("HOME", &s.home)
        .current_dir(cwd)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("rtok spawns");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    (out.status.success(), text(&out.stdout), text(&out.stderr))
}

/// The post-create script's call: no agent named, run from the new worktree.
fn adopt_unattended(s: &Scene) -> serde_json::Value {
    let (ok, out, err) = rtok(s, &s.made, &["worktree", "adopt", "--json"], "");
    assert!(ok, "{out}{err}");
    serde_json::from_str(&out).unwrap()
}

fn start_session(s: &Scene, session: &str, cwd: &Path) -> String {
    let payload = serde_json::json!({
        "hook_event_name": "SessionStart",
        "session_id": session,
        "cwd": cwd,
        "source": "startup",
    });
    let args = ["hook", "SessionStart", "--host", "cursor"];
    let (ok, out, err) = rtok(s, &s.work, &args, &payload.to_string());
    assert!(ok, "{out}{err}");
    out
}

fn parked(s: &Scene) -> Vec<(String, String)> {
    s.store.pending_worktrees().unwrap()
}

fn seed_cursor_agents(s: &Scene, n: usize) {
    let cursor = s.store.host_id("cursor").unwrap().unwrap();
    for i in 0..n {
        let cwd = s.work.to_str();
        s.store
            .register_agent(cursor, &format!("sess-{i}"), None, cwd, None)
            .unwrap();
    }
}

#[test]
fn no_live_agent_parks_the_claim_without_a_lock() {
    let s = scene("pending-zero");
    let done = adopt_unattended(&s);
    assert_eq!(done["origin"], "cursor");
    assert_eq!(done["task"], "t7");
    assert_eq!(done["locked"], false);
    assert_eq!(done["pending"], true);
    assert_eq!(parked(&s), [(s.made.to_string_lossy().into(), "t7".into())]);
    assert!(s.store.open_worktree_claims().unwrap().is_empty());
    assert!(!run(&s.work, &["worktree", "list", "--porcelain"]).contains("locked"));
}

#[test]
fn two_live_agents_park_the_claim_too() {
    let s = scene("pending-two");
    seed_cursor_agents(&s, 2);
    assert_eq!(adopt_unattended(&s)["pending"], true);
    assert_eq!(parked(&s).len(), 1);
    assert!(s.store.open_worktree_claims().unwrap().is_empty());
}

#[test]
fn one_live_agent_still_binds_directly() {
    let s = scene("pending-one");
    seed_cursor_agents(&s, 1);
    let done = adopt_unattended(&s);
    assert!(done.get("pending").is_none(), "{done}");
    assert!(parked(&s).is_empty());
    assert_eq!(s.store.open_worktree_claims().unwrap().len(), 1);
}

#[test]
fn session_start_inside_the_worktree_completes_it() {
    let s = scene("pending-hook");
    adopt_unattended(&s);
    // A session elsewhere in the repository takes nothing.
    start_session(&s, "sess-main", &s.work);
    assert_eq!(parked(&s).len(), 1);

    let nested = s.made.join("sub");
    std::fs::create_dir_all(&nested).unwrap();
    let out = start_session(&s, "sess-wt", &nested);
    let id = out
        .split("(full: ")
        .nth(1)
        .and_then(|t| t.split(')').next())
        .unwrap_or_else(|| panic!("no agent id in {out}"));
    assert!(parked(&s).is_empty());
    assert_eq!(
        s.store.open_worktree_claims().unwrap(),
        [(s.made.to_string_lossy().into(), id.into())]
    );
}

#[test]
fn an_explicit_adopt_completes_it() {
    let s = scene("pending-adopt");
    adopt_unattended(&s);
    let cursor = s.store.host_id("cursor").unwrap().unwrap();
    let agent = s
        .store
        .register_agent(cursor, "sess-a", None, s.made.to_str(), None)
        .unwrap();
    let args = ["worktree", "adopt", "--agent", agent.as_str()];
    let (ok, out, err) = rtok(&s, &s.made, &args, "");
    assert!(ok, "{out}{err}");
    assert!(parked(&s).is_empty());
    assert_eq!(s.store.open_worktree_claims().unwrap().len(), 1);
}

#[test]
fn gc_keeps_a_pending_worktree_that_it_would_otherwise_remove() {
    let s = scene("pending-gc");
    let gc = || {
        let (ok, out, err) = rtok(
            &s,
            &s.work,
            &["worktree", "gc", "--idle", "0h", "--json"],
            "",
        );
        assert!(ok, "{out}{err}");
        let rows: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
        let row = rows
            .into_iter()
            // git prints `C:/...` on Windows; compare the paths, not the spelling.
            .find(|r| {
                r["path"]
                    .as_str()
                    .and_then(|p| fs_kit::path::canonicalize(p).ok())
                    .is_some_and(|p| p == s.made)
            })
            .expect("the worktree is listed");
        (
            row["action"].as_str().unwrap().to_string(),
            row["note"].to_string(),
        )
    };
    assert_eq!(gc().0, "remove");
    adopt_unattended(&s);
    let (action, note) = gc();
    assert_eq!(action, "keep", "{note}");
    assert!(note.contains("pending claim"), "{note}");
}
