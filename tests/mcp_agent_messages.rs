// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T287 PR 2: MCP `agent_send` / `agent_inbox` between two real `rtok mcp` sessions. Each
//! session is linked to its agent by cwd (T283.1), so the sender and the inbox owner are never
//! arguments.

mod common;

use std::path::PathBuf;

use common::mcp::session;
use rtok::store::Store;

struct Fixture {
    home: PathBuf,
    store: Store,
    cwds: [PathBuf; 2],
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let home = rtok::testutil::tmp_dir(tag);
        std::fs::create_dir_all(home.join(".rtok")).unwrap();
        let store = Store::open(&home.join(".rtok/rtok.db")).unwrap();
        let cwd = |name: &str| {
            let dir = home.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            dir.canonicalize().unwrap()
        };
        let cwds = [cwd("a"), cwd("b")];
        Self { home, store, cwds }
    }

    /// The agent of session `i`: the hooks register (or refresh) its row.
    fn agent(&self, i: usize) -> String {
        let claude = self.store.host_id("claude").unwrap().unwrap();
        let cwd = self.cwds[i].to_str();
        let session = format!("sess-{i}");
        self.store
            .register_agent(claude, &session, None, cwd, None)
            .unwrap()
    }

    fn run(&self, i: usize, calls: &[(&str, &str)]) -> Vec<(bool, String)> {
        let hooks = || drop((self.agent(0), self.agent(1)));
        session(&self.home, &self.cwds[i], hooks, calls)
    }

    /// Session 0 whose hooks refresh only its own row: a hook event revives an ended agent, so
    /// an ended recipient has to stay out of them.
    fn run_alone(&self, calls: &[(&str, &str)]) -> Vec<(bool, String)> {
        session(&self.home, &self.cwds[0], || drop(self.agent(0)), calls)
    }
}

fn short(id: &str) -> &str {
    &id[..8]
}

#[test]
fn a_message_one_agent_sends_reaches_the_other_framed_and_once() {
    let fx = Fixture::new("mcp-messages");
    let (a, b) = (fx.agent(0), fx.agent(1));
    let send = format!(
        r#"{{"to":"{}","text":"build is green\nship it"}}"#,
        short(&b)
    );
    let sent = fx.run(0, &[("agent_send", &send)]);
    assert!(!sent[0].0, "{}", sent[0].1);

    let read = fx.run(1, &[("agent_inbox", "{}"), ("agent_inbox", "{}")]);
    let (first, second) = (&read[0], &read[1]);
    assert!(!first.0, "{}", first.1);
    let frame = &first.1;
    assert!(
        frame.starts_with(&format!("[rtok message #1 from {} (claude)", short(&a))),
        "{frame}"
    );
    assert!(frame.contains("> build is green\n> ship it\n"), "{frame}");
    assert_eq!(second.1, "no messages");
}

#[test]
fn the_inbox_is_the_session_s_own_and_a_limit_leaves_the_rest_unread() {
    let fx = Fixture::new("mcp-messages-limit");
    let (a, b) = (fx.agent(0), fx.agent(1));
    for text in ["one", "two", "three"] {
        fx.store.send_message(Some(&a), &b, text).unwrap();
    }
    let got = fx.run(
        1,
        &[
            ("agent_inbox", r#"{"limit":2}"#),
            ("agent_inbox", "{}"),
            ("agent_inbox", "{}"),
        ],
    );
    assert!(got[0].1.contains("> one") && got[0].1.contains("> two"));
    assert!(!got[0].1.contains("> three"), "{}", got[0].1);
    assert!(got[1].1.contains("> three"), "{}", got[1].1);
    assert_eq!(got[2].1, "no messages");
    // A read never touches the other agent's queue.
    assert!(fx.store.inbox(&a, true, false).unwrap().is_empty());
}

#[test]
fn a_send_to_an_unknown_ended_or_oversized_target_is_refused() {
    let fx = Fixture::new("mcp-messages-refused");
    let b = fx.agent(1);
    let big = "x".repeat(5000);
    let to_b = |text: &str| format!(r#"{{"to":"{}","text":"{text}"}}"#, short(&b));
    let calls = [
        ("agent_send", r#"{"to":"ffffffff","text":"hi"}"#.to_string()),
        ("agent_send", to_b(&big)),
        ("agent_send", to_b("")),
        ("agent_send", r#"{"text":"no recipient"}"#.to_string()),
    ];
    let calls: Vec<(&str, &str)> = calls.iter().map(|(n, a)| (*n, a.as_str())).collect();
    let got = fx.run(0, &calls);
    assert!(got.iter().all(|(is_err, _)| *is_err), "{got:?}");
    assert!(got[0].1.contains("agent ffffffff"), "{}", got[0].1);
    assert!(got[1].1.contains("cap is 4096"), "{}", got[1].1);
    assert!(got[3].1.contains("missing `to`"), "{}", got[3].1);

    fx.store.end_agent(&b, 1).unwrap();
    let ended = fx.run_alone(&[("agent_send", &to_b("hi"))]);
    assert!(ended[0].0 && ended[0].1.contains("ended"), "{ended:?}");
    assert!(fx.store.inbox(&b, false, false).unwrap().is_empty());
}

#[test]
fn a_task_tag_travels_through_the_tools_and_filters_the_inbox() {
    let fx = Fixture::new("mcp-messages-task");
    let b = fx.agent(1);
    let send =
        |text: &str, task: &str| format!(r#"{{"to":"{}","text":"{text}"{task}}}"#, short(&b));
    let sent = fx.run(
        0,
        &[
            ("agent_send", &send("on twelve", r#","task":"a12""#)),
            ("agent_send", &send("on nothing", "")),
            ("agent_send", &send("bad", r#","task":"nope""#)),
        ],
    );
    assert!(!sent[0].0 && !sent[1].0, "{sent:?}");
    assert!(sent[2].0, "a bad task id is refused: {sent:?}");

    let got = fx.run(
        1,
        &[("agent_inbox", r#"{"task":"A12"}"#), ("agent_inbox", "{}")],
    );
    assert!(
        got[0].1.contains("\nTask: A12\n> on twelve\n"),
        "{}",
        got[0].1
    );
    assert!(!got[0].1.contains("on nothing"), "{}", got[0].1);
    assert!(
        got[1].1.contains("> on nothing") && !got[1].1.contains("on twelve"),
        "{}",
        got[1].1
    );
}
