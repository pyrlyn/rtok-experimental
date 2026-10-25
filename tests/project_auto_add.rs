// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T329.6: a directory rtok sees in use joins the project registry when
//! `[plugins.graph] auto_add_projects` is on, and only then. One scratch `$HOME` per case, real
//! `rtok` processes, no real agent.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use common::git::{repo, run};
use rtok::store::{Origin, Project, Store};

/// A scratch home (with a config turning the key off when `on` is false) and a project
/// directory inside it.
fn scene(name: &str, on: bool) -> (PathBuf, PathBuf) {
    // `fs_kit::path`, as the store spells roots: no `\\?\` prefix on Windows.
    let tmp = fs_kit::path::canonicalize(rtok::testutil::tmp_dir(name)).unwrap();
    std::fs::create_dir_all(tmp.join(".rtok")).unwrap();
    if !on {
        let off = "[plugins.graph]\nauto_add_projects = false\n";
        std::fs::write(tmp.join(".rtok/config.toml"), off).unwrap();
    }
    let proj = tmp.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    (tmp, proj)
}

fn store(home: &Path) -> Store {
    Store::open(&home.join(".rtok/rtok.db")).unwrap()
}

fn projects(home: &Path) -> Vec<Project> {
    store(home).projects().unwrap()
}

fn rtok(home: &Path, cwd: &Path, args: &[&str], stdin: &str) -> std::process::Output {
    use std::io::Write as _;
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .current_dir(cwd)
        .env("HOME", home)
        .env_remove("RTOK_AGENT_ID")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("rtok spawns");
    let mut pipe = child.stdin.take().unwrap();
    pipe.write_all(stdin.as_bytes()).unwrap();
    drop(pipe);
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// A root as the store spells it: `/` separators on every OS.
fn spelled(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// The only registered project: its root and origin, or nothing registered.
fn only(home: &Path, on: bool) -> Option<(String, Origin)> {
    let got = projects(home);
    if !on {
        assert!(got.is_empty(), "{got:?}");
        return None;
    }
    assert_eq!(got.len(), 1, "{got:?}");
    Some((got[0].root.clone(), got[0].origin))
}

#[test]
fn a_session_that_ran_no_tool_does_not_register_a_project() {
    let (home, proj) = scene("auto-add-toolless", true);
    let start = serde_json::json!({
        "hook_event_name": "SessionStart",
        "session_id": "sess-auto-add",
        "cwd": proj,
        "source": "startup",
    });
    rtok(&home, &proj, &["hook", "SessionStart"], &start.to_string());
    let end = serde_json::json!({
        "hook_event_name": "SessionEnd",
        "session_id": "sess-auto-add",
        "cwd": proj,
    });
    rtok(&home, &proj, &["hook", "SessionEnd"], &end.to_string());
    assert!(projects(&home).is_empty());
}

#[test]
fn a_session_that_ran_a_tool_registers_the_project_only_when_on() {
    for on in [true, false] {
        let (home, proj) = scene(&format!("auto-add-tool-{on}"), on);
        let start = serde_json::json!({
            "hook_event_name": "SessionStart",
            "session_id": "sess-auto-add",
            "cwd": proj,
            "source": "startup",
        });
        rtok(&home, &proj, &["hook", "SessionStart"], &start.to_string());
        let tool = serde_json::json!({
            "hook_event_name": "PreToolUse",
            "session_id": "sess-auto-add",
            "cwd": proj,
            "tool_name": "Read",
            "tool_input": {"file_path": proj.join("x.rs")},
        });
        rtok(&home, &proj, &["hook", "PreToolUse"], &tool.to_string());
        let want = on.then(|| (spelled(&proj), Origin::Session));
        assert_eq!(only(&home, on), want);
    }
}

#[test]
fn a_graph_mcp_call_registers_its_root_only_when_on() {
    for on in [true, false] {
        let (home, proj) = scene(&format!("auto-add-mcp-{on}"), on);
        let call = [("symbol", r#"{"name":"nothing"}"#)];
        common::mcp::session(&home, &proj, || {}, &call);
        let want = on.then(|| (spelled(&proj), Origin::Mcp));
        assert_eq!(only(&home, on), want);
    }
}

/// `worktree add` and `worktree adopt` register the worktree, named by its branch; off, neither.
#[test]
fn worktrees_made_or_adopted_register_named_by_branch_only_when_on() {
    for on in [true, false] {
        let (home, _) = scene(&format!("auto-add-worktree-{on}"), on);
        let work = repo(&home);
        let args = ["worktree", "add", "t5", "x", "--owner", "me"];
        let out = rtok(&home, &work, &args, "");
        let added = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());

        let made = home.join(".kilo/worktrees/t7-y");
        let dir = made.to_str().unwrap();
        run(&work, &["worktree", "add", "-q", "-b", "t7-y", dir]);
        let db = store(&home);
        let claude = db.host_id("claude").unwrap().unwrap();
        let me = db
            .register_agent(claude, "sess-wt", None, None, None)
            .unwrap();
        drop(db);
        rtok(&home, &made, &["worktree", "adopt", "--agent", &me], "");

        let got = projects(&home);
        if !on {
            assert!(got.is_empty(), "{got:?}");
            continue;
        }
        let mut named: Vec<_> = got
            .iter()
            .map(|p| (p.origin, p.display_name().to_string()))
            .collect();
        named.sort_by(|a, b| a.1.cmp(&b.1));
        let want = [(Origin::Worktree, "t5-x"), (Origin::Worktree, "t7-y")];
        assert_eq!(named, want.map(|(o, n)| (o, n.to_string())));
        let roots: Vec<_> = got.iter().map(|p| PathBuf::from(&p.root)).collect();
        assert!(
            roots.contains(&fs_kit::path::canonicalize(&added).unwrap()),
            "{roots:?}"
        );
    }
}
