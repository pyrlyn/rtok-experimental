// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T284 PR 2: MCP `agents_list`, `agent_show` and `agent_status_set` in a real `rtok mcp`
//! session linked to its agent by cwd (T283.1).

use crate::common;

use common::mcp::session;
use rtok::store::Store;
use serde_json::Value;

#[test]
fn an_agent_sees_every_agent_and_sets_only_its_own_status() {
    let home = rtok::testutil::tmp_dir("mcp-agents");
    std::fs::create_dir_all(home.join(".rtok")).unwrap();
    let store = Store::open(&home.join(".rtok/rtok.db")).unwrap();
    let claude = store.host_id("claude").unwrap().unwrap();
    std::fs::create_dir_all(home.join("a")).unwrap();
    let cwd = home.join("a").canonicalize().unwrap();
    let register = |session: &str, cwd: Option<&str>| {
        store
            .register_agent(claude, session, None, cwd, None)
            .unwrap()
    };
    let other = register("sess-other", Some("/elsewhere"));
    let me = register("sess-me", cwd.to_str());
    store.claim_worktree("/wt/rtok-t9", &me, "t9").unwrap();
    store.send_message(Some(&other), &me, "ping").unwrap();

    let too_long = format!(r#"{{"text":"{}"}}"#, "x".repeat(121));
    let show = format!(r#"{{"id":"{}"}}"#, &me[..8]);
    let calls = [
        ("agent_status_set", r#"{"text":"fixing T284"}"#),
        ("agent_status_set", too_long.as_str()),
        ("agent_show", show.as_str()),
        ("agents_list", "{}"),
        ("agent_show", r#"{"id":"ffffffff"}"#),
        ("agent_status_set", r#"{"text":" "}"#),
    ];
    let hooks = || drop(register("sess-me", cwd.to_str()));
    let got = session(&home, &cwd, hooks, &calls);

    assert_eq!(got[0], (false, "status: fixing T284".to_string()));
    assert!(got[1].0 && got[1].1.contains("at most 120"), "{:?}", got[1]);
    let shown: Value = serde_json::from_str(&got[2].1).unwrap();
    assert_eq!(shown["id"], me);
    assert_eq!(shown["status_text"], "fixing T284");
    assert_eq!(shown["worktrees"], serde_json::json!(["/wt/rtok-t9"]));
    assert_eq!(
        (&shown["worktree"], &shown["unread"]),
        (&"rtok-t9".into(), &1.into())
    );
    let listed: Value = serde_json::from_str(&got[3].1).unwrap();
    assert!(listed.is_array(), "{}", got[3].1);
    assert!(
        got[4].0 && got[4].1.contains("no agent id starts with"),
        "{:?}",
        got[4]
    );
    assert_eq!(got[5], (false, "status cleared".to_string()));
    // Only the caller's own row was written.
    assert_eq!(
        store.agent_detail(&other).unwrap().unwrap().status_text,
        None
    );
}
