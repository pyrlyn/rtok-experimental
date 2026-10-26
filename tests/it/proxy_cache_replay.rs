// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T385.6: a replay of one growing agent conversation through the real proxy, every rewrite
//! switched on, against a mock upstream. The provider's prompt cache reads a request back to
//! front: tools, then system, then messages. So a turn keeps its cache hit only if everything
//! the previous turn had already finished rewriting reaches upstream byte for byte the same.
//! The edge the model is still working on (the last `keep_turns` turns) may change as it ages
//! out of the live zone; nothing before it may.

use rtok::config::Config;
use serde_json::{Value, json};

use crate::common;
use common::proxy::{Sink, proxy_server};

const SESSION: &str = "sess-cache-replay";
const KEEP_TURNS: usize = 3;
const TURNS: usize = 9;
/// Well over `proxy.tools_rewrite.max_description_tokens`, so the rewrite has work to do.
const LONG_DESCRIPTION: &str = "Runs a shell command and returns its output. It accepts a working directory, a timeout and an environment, streams long output back in chunks, and reports the exit status separately. Prefer it over ad hoc scripts, and keep each command short and idempotent whenever possible.";

/// One tool result. Every third is a JSON array (what `toon` rewrites); the rest is prose,
/// with terminal colour codes on the even ones (what the noise strip removes).
fn tool_output(t: usize) -> String {
    if t.is_multiple_of(3) {
        let rows: Vec<Value> = (1..=80)
            .map(|i| json!({"id": i, "name": format!("item-{t}-{i}"), "state": "ok"}))
            .collect();
        return serde_json::to_string(&rows).expect("rows");
    }
    (1..=300)
        .map(|i| {
            if t.is_multiple_of(2) {
                format!("\u{1b}[32mt{t} line {i}\u{1b}[0m: some shell output with words")
            } else {
                format!("t{t} line {i}: some shell output with words")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the client sends on turn `n`: the whole history so far, as agents do.
fn request(n: usize) -> Vec<u8> {
    let mut messages = vec![json!({"role":"user","content":"start the task"})];
    for t in 1..=n {
        messages.push(json!({"role":"assistant","content":[
            {"type":"tool_use","id":format!("tu-{t}"),"name":"Bash","input":{}}]}));
        messages.push(json!({"role":"user","content":[
            {"type":"tool_result","tool_use_id":format!("tu-{t}"),"content":tool_output(t)}]}));
    }
    serde_json::to_vec(&json!({
        "model": "claude-test", "max_tokens": 8, "system": "You are a coding agent.",
        "tools": [{"name": "Bash", "description": LONG_DESCRIPTION,
                   "input_schema": {"type": "object"}}],
        "messages": messages, "metadata": {"user_id": SESSION}
    }))
    .expect("request json")
}

fn configure(cfg: &mut Config, base: &str, context_management: bool) {
    // Every provider upstream points at the mock, so no case can reach a real API.
    cfg.proxy.upstream = base.to_string();
    cfg.proxy.openai_upstream = base.to_string();
    cfg.proxy.gemini_upstream = base.to_string();
    cfg.proxy.mode = "compress".to_string();
    cfg.proxy.tools_rewrite.enabled = true;
    cfg.proxy.context_management = context_management;
    cfg.plugins.toon.enabled = true;
    cfg.plugins.compress.enabled = true;
    cfg.plugins.archive.keep_turns = KEEP_TURNS as u32;
}

/// Index of the first message still inside the live zone: the `KEEP_TURNS`-th user message
/// counted from the end. Everything before it was outside the zone on that turn.
fn live_edge(messages: &[Value]) -> usize {
    messages
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, m)| m["role"] == "user")
        .nth(KEEP_TURNS - 1)
        .map_or(0, |(i, _)| i)
}

/// Replays `TURNS` turns and returns what upstream saw, parsed, one body per turn.
async fn replay(tag: &str, context_management: bool) -> Vec<Value> {
    let up = Sink::new();
    let (addr, _state, task) = proxy_server(tag, |cfg| {
        configure(cfg, &up.base(), context_management);
    })
    .await;
    let client = reqwest::Client::new();
    for n in 1..=TURNS {
        let status = client
            .post(format!("http://{addr}/v1/messages"))
            .header("content-type", "application/json")
            .body(request(n))
            .send()
            .await
            .expect("request through the proxy")
            .status();
        assert!(status.is_success(), "turn {n}: {status}");
    }
    task.abort();
    let seen = up.all();
    assert_eq!(seen.len(), TURNS, "one upstream request per turn");
    seen.into_iter()
        .map(|(body, _)| serde_json::from_slice(&body).expect("forwarded json"))
        .collect()
}

fn bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).expect("json")
}

/// Turn after turn, `tools`, `system` and every other header-level field are identical and
/// the messages before the previous turn's live edge are byte-identical.
fn assert_prefix_is_stable(sent: &[Value]) {
    for (n, pair) in sent.windows(2).enumerate() {
        let (prev, next) = (&pair[0], &pair[1]);
        let turn = n + 2;
        let mut prev_head = prev.clone();
        let mut next_head = next.clone();
        prev_head["messages"] = Value::Null;
        next_head["messages"] = Value::Null;
        assert_eq!(
            bytes(&prev_head),
            bytes(&next_head),
            "turn {turn}: tools/system/top-level fields moved"
        );
        let (prev, next) = (prev["messages"].as_array(), next["messages"].as_array());
        let (prev, next) = (prev.expect("messages"), next.expect("messages"));
        let edge = live_edge(prev);
        for i in 0..edge {
            assert_eq!(
                bytes(&prev[i]),
                bytes(&next[i]),
                "turn {turn}: message {i} changed after it left the live zone"
            );
        }
    }
}

fn text_of(message: &Value) -> String {
    match &message["content"][0]["content"] {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[tokio::test]
async fn agent_prefix_stays_byte_stable_across_turns_with_rewrites_on() {
    let sent = replay("cache-replay-archive", false).await;
    assert_prefix_is_stable(&sent);

    // Not vacuous: the rewrites really ran. The oldest result is a summary by the last turn,
    // while the newest is whole, and the long tool description was shortened.
    let last = &sent[TURNS - 1];
    let messages = last["messages"].as_array().expect("messages");
    let oldest = text_of(&messages[2]);
    assert!(
        oldest.len() < tool_output(1).len() / 4,
        "oldest: {oldest:.300}"
    );
    assert_eq!(
        text_of(&messages[messages.len() - 1]),
        tool_output(TURNS),
        "newest result must stay whole"
    );
    let original: Value = serde_json::from_slice(&request(TURNS)).expect("json");
    assert_ne!(last["messages"], original["messages"]);
    assert!(
        last["tools"][0]["description"]
            .as_str()
            .expect("description")
            .len()
            < LONG_DESCRIPTION.len()
    );
}

/// With context editing armed `archive` stands down and the platform clears old tool uses;
/// the request still must not move between turns.
#[tokio::test]
async fn agent_prefix_stays_byte_stable_with_context_management_armed() {
    let sent = replay("cache-replay-context", true).await;
    assert_prefix_is_stable(&sent);
    assert!(
        sent[TURNS - 1]["context_management"].is_object(),
        "context management was not armed"
    );
}
