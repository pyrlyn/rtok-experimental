// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T290 (D34): one table over every host in `src/agents/*`. The same session walk runs per host
//! with fakes only: a scratch `$HOME`, a scratch repository, `rtok hook` stdin for a host that
//! can carry hooks and the MCP process's own row for one that cannot, never a real agent. Every
//! host must come out with the same worktree path rule, lock format and list row.
//!
//! Post-create scripts (T289.3) are covered in `agents_post_create.rs`; here adoption goes through
//! MCP `worktree_adopt` only.

use crate::common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use common::git::{repo, run};
use rtok::agents::{self, link};

/// `rtok <args>` under `home` as the agent `agent`, with `stdin`; `(success, stdout, stderr)`.
fn rtok(
    home: &Path,
    cwd: &Path,
    agent: Option<&str>,
    args: &[&str],
    stdin: &str,
) -> (bool, String) {
    use std::io::Write as _;
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rtok"));
    match agent {
        Some(id) => cmd.env("RTOK_AGENT_ID", id),
        None => cmd.env_remove("RTOK_AGENT_ID"),
    };
    let mut child = cmd
        .current_dir(cwd)
        .env("HOME", home)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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
    (out.status.success(), text(&out.stdout) + &text(&out.stderr))
}

/// What one host's walk produced, with everything host-specific (ids, the scratch path, the
/// host's own name) replaced, so the walks can be compared.
#[derive(Debug, PartialEq, Eq)]
struct Shape {
    path: String,
    branch: String,
    rows: Vec<serde_json::Value>,
    inbox: String,
}

fn walk(host: &str) -> Result<Shape, String> {
    let tmp = rtok::testutil::tmp_dir(&format!("agents-worktrees-{host}"));
    let work = repo(&tmp);
    std::fs::create_dir_all(tmp.join(".rtok")).unwrap();
    let store = rtok::store::Store::open(&tmp.join(".rtok/rtok.db")).unwrap();
    // A worktree the host made in its own pool, for `worktree_adopt`.
    let made = tmp.join(".cursor/worktrees/rtok/abc");
    run(
        &work,
        &["worktree", "add", "-q", "--detach", made.to_str().unwrap()],
    );
    let made = made.canonicalize().unwrap();
    // A second agent in another directory, so no cwd link can mix the two up.
    let other_cwd = tmp.join("elsewhere");
    std::fs::create_dir_all(&other_cwd).unwrap();
    let claude = store.host_id("claude").unwrap().unwrap();
    let peer = store
        .register_agent(claude, "sess-peer", None, other_cwd.to_str(), None)
        .unwrap();

    // The session start: a hook payload where the host can carry hooks; otherwise the MCP
    // process registers its own row.
    let start = || {
        if link::hookless(host) {
            return;
        }
        let payload = serde_json::json!({
            "hook_event_name": "SessionStart",
            "session_id": format!("sess-{host}"),
            "cwd": work,
            "source": "startup",
        });
        let (ok, out) = rtok(
            &tmp,
            &work,
            None,
            &["hook", "SessionStart", "--host", host],
            &payload.to_string(),
        );
        assert!(ok, "hook for {host}: {out}");
    };
    let send =
        serde_json::json!({"to": &peer[..8], "text": format!("ping from {host}")}).to_string();
    let adopt = serde_json::json!({"path": made, "task": "t2"}).to_string();
    let remove_adopted = serde_json::json!({"path": made}).to_string();
    let calls = [
        ("whoami", "{}"),
        ("worktree_add", r#"{"task":"t1","slug":"x"}"#),
        ("worktree_adopt", adopt.as_str()),
        ("worktree_list", "{}"),
        ("agent_send", send.as_str()),
        ("worktree_remove", r#"{"task":"t1"}"#),
        ("worktree_remove", remove_adopted.as_str()),
    ];
    let got = common::mcp::session_as(host, &tmp, &work, start, &calls);
    for (i, (is_err, text)) in got.iter().enumerate() {
        if *is_err {
            return Err(format!(
                "{host}: call {} ({}) failed: {text}",
                i, calls[i].0
            ));
        }
    }
    let json = |i: usize| serde_json::from_str::<serde_json::Value>(&got[i].1).unwrap();
    let me = json(0)["id"]
        .as_str()
        .ok_or("whoami has no id")?
        .to_string();
    let added = json(1);
    if added["agent"] != me {
        return Err(format!(
            "{host}: worktree_add bound {} not {me}",
            added["agent"]
        ));
    }
    let path = PathBuf::from(added["path"].as_str().unwrap());
    let adopted = json(2);
    if adopted["origin"] != "cursor" || adopted["locked"] != false {
        return Err(format!("{host}: adopt gave {adopted}"));
    }
    let rows = json(3);
    let mut shaped = Vec::new();
    for name in ["rtok-t1", "abc"] {
        let mut row = rows
            .as_array()
            .unwrap()
            .iter()
            .find(|r| Path::new(r["path"].as_str().unwrap()).ends_with(name))
            .ok_or(format!("{host}: worktree_list lacks {name}"))?
            .clone();
        if row["agent"]["id"] != me || row["agent"]["state"] != "live" {
            return Err(format!("{host}: {name} row names {}", row["agent"]));
        }
        // The host's own name is the only thing allowed to differ.
        let object = row.as_object_mut().unwrap();
        for key in [
            "path",
            "modified_unix",
            "source_bytes",
            "cache_bytes",
            "session",
        ] {
            object.remove(key);
        }
        object["agent"] = serde_json::json!({"state": "live"});
        // An adopted Cursor-pool worktree has no lock, so no owner either.
        if name == "rtok-t1" {
            if !object["owner"].as_str().is_some_and(|o| !o.is_empty()) {
                return Err(format!("{host}: lock names no owner: {row}"));
            }
            object["owner"] = "<owner>".into();
        }
        shaped.push(row);
    }
    if path.exists() || made.exists() || !run(&work, &["branch", "--list", "t1-x"]).is_empty() {
        return Err(format!(
            "{host}: removal left {} or its branch",
            path.display()
        ));
    }

    // The peer reads what the session sent, then answers it.
    let (ok, inbox) = rtok(&tmp, &work, Some(&peer), &["agents", "inbox", &peer], "");
    if !ok || !inbox.contains(&format!("ping from {host}")) {
        return Err(format!("{host}: peer inbox: {inbox}"));
    }
    let (ok, out) = rtok(
        &tmp,
        &work,
        Some(&peer),
        &["agents", "send", &me[..8], "pong"],
        "",
    );
    let (read, mine) = rtok(&tmp, &work, Some(&me), &["agents", "inbox", &me], "");
    if !ok || !read || !mine.contains("pong") {
        return Err(format!("{host}: reply {out} / inbox {mine}"));
    }
    Ok(Shape {
        // The root comes from `$HOME` as spelled (`/var/…` on macOS, not `/private/var/…`);
        // fs-kit: the server reports paths without the `\\?\` prefix std adds on Windows.
        path: path
            .strip_prefix(&tmp)
            .or_else(|_| path.strip_prefix(fs_kit::path::canonicalize(&tmp).unwrap()))
            .unwrap()
            .display()
            .to_string()
            .replace('\\', "/"),
        branch: added["branch"].as_str().unwrap().into(),
        rows: shaped,
        inbox: "framed".into(),
    })
}

#[test]
fn every_host_gets_the_same_worktree_rule_lock_and_row() {
    // Six walks at a time: each spawns several processes, and CI runs other binaries beside this.
    let mut shapes: Vec<(&str, Result<Shape, String>)> = Vec::new();
    for chunk in agents::HOSTS.chunks(6) {
        std::thread::scope(|s| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|h| (*h, s.spawn(move || walk(h))))
                .collect();
            for (h, t) in handles {
                shapes.push((
                    h,
                    t.join().unwrap_or_else(|_| Err(format!("{h}: panicked"))),
                ));
            }
        });
    }
    let failed: Vec<&str> = shapes
        .iter()
        .filter_map(|(_, r)| r.as_ref().err().map(String::as_str))
        .collect();
    assert!(failed.is_empty(), "{}", failed.join("\n"));
    let (first, want) = (shapes[0].0, shapes[0].1.as_ref().unwrap());
    for (host, shape) in &shapes[1..] {
        assert_eq!(shape.as_ref().unwrap(), want, "{host} differs from {first}");
    }
    // T660.4: under the main checkout (`<tmp>/rtok`), not under `$HOME`.
    assert_eq!(want.path, "rtok/.rtok/worktrees/rtok-t1");
    assert_eq!(want.branch, "t1-x");
}

/// `docs/agents-and-worktrees.md` lists every host once, and its Hooks column says what the code
/// says: a host whose variants all lack hooks registers its agent through MCP.
#[test]
fn the_doc_names_every_host_and_whether_it_has_hooks() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/agents-and-worktrees.md");
    let doc = std::fs::read_to_string(path).expect("docs/agents-and-worktrees.md");
    for host in agents::HOSTS {
        let row = format!("| `{host}` | ");
        let mut rows = doc.lines().filter(|l| l.starts_with(&row));
        let line = rows.next().unwrap_or_else(|| panic!("no row for {host}"));
        assert!(rows.next().is_none(), "two rows for {host}");
        let hooks = line[row.len()..].split(" | ").next().unwrap();
        let want = if link::hookless(host) { "no" } else { "yes" };
        assert_eq!(hooks, want, "{host}: Hooks column");
    }
}
