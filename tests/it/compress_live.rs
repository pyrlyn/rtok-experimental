// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T385.9.2: the P28 summaries in the live zone, through the real proxy against mock upstreams
//! (one for the agent, one for the compressor). With the flag off the proxy forwards what it
//! always did; with it on a request never waits for the compressor, the view of an old result
//! turns into the stored summary once, and stays byte-stable from then on.

use std::time::{Duration, Instant};

use httpmock::prelude::*;
use rtok::config::Config;
use rtok::plugins::compress::llm;
use rtok::proxy::ProxyState;
use serde_json::{Value, json};

use crate::it::common;
use common::compress_fixture::{log, reply, summary_with};
use common::proxy::{Sink, proxy_server};

const SESSION: &str = "sess-compress-live";

/// The conversation after `n` tool calls, every result the same build log.
fn request(n: usize) -> Vec<u8> {
    let mut messages = vec![json!({"role":"user","content":"start the task"})];
    for t in 1..=n {
        messages.push(json!({"role":"assistant","content":[
            {"type":"tool_use","id":format!("tu-{t}"),"name":"Bash","input":{}}]}));
        messages.push(json!({"role":"user","content":[
            {"type":"tool_result","tool_use_id":format!("tu-{t}"),"content":log()}]}));
    }
    serde_json::to_vec(&json!({
        "model": "claude-test", "max_tokens": 8, "system": "You are a coding agent.",
        "messages": messages, "metadata": {"user_id": SESSION}
    }))
    .expect("request json")
}

fn configure(cfg: &mut Config, agent: &str, compressor: Option<&str>) {
    cfg.proxy.upstream = agent.to_string();
    cfg.proxy.openai_upstream = agent.to_string();
    cfg.proxy.gemini_upstream = agent.to_string();
    cfg.proxy.mode = "compress".to_string();
    cfg.plugins.compress.enabled = true;
    cfg.plugins.archive.keep_turns = 1;
    if let Some(url) = compressor {
        cfg.plugins.compress.llm.enabled = true;
        cfg.plugins.compress.llm.url = url.to_string();
    }
}

async fn send(addr: &str, n: usize) -> Duration {
    send_with(addr, n, "x-api-key", "test-key").await
}

async fn send_with(addr: &str, n: usize, header: &str, value: &str) -> Duration {
    let start = Instant::now();
    let status = reqwest::Client::new()
        .post(format!("http://{addr}/v1/messages"))
        .header("content-type", "application/json")
        .header(header, value)
        .body(request(n))
        .send()
        .await
        .expect("request through the proxy")
        .status();
    assert!(status.is_success(), "turn {n}: {status}");
    start.elapsed()
}

fn sent(up: &Sink, turn: usize) -> Value {
    serde_json::from_slice(&up.all()[turn - 1].0).expect("forwarded json")
}

/// The text of the first tool result of a forwarded body.
fn first_result(body: &Value) -> String {
    body["messages"][2]["content"][0]["content"]
        .as_str()
        .expect("string result")
        .to_string()
}

fn archive_id(view: &str) -> String {
    let at = view.find("expand(").expect("expand pointer") + "expand(".len();
    view[at..].split(')').next().expect("id").to_string()
}

async fn stored(state: &ProxyState, cfg_model: &str, id: &str) -> Option<String> {
    let version = llm::version(cfg_model);
    for _ in 0..100 {
        if let Some(s) = state.store.archive_summary(id, &version).expect("read") {
            return Some(s);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    None
}

fn model() -> String {
    Config::default().plugins.compress.llm.model
}

#[tokio::test]
async fn flag_off_keeps_the_extractive_view_and_makes_no_model_call() {
    let (agent, compressor) = (Sink::new(), MockServer::start());
    let calls = compressor.mock(|when, then| {
        when.any_request();
        then.status(200).json_body(reply("x"));
    });
    let (addr, state, task) = proxy_server("compress-live-off", |cfg| {
        configure(cfg, &agent.base(), None);
        // The compressor is reachable but not asked for.
        cfg.plugins.compress.llm.url = compressor.base_url();
    })
    .await;
    for n in 1..=3 {
        send(&addr, n).await;
    }
    task.abort();
    assert!(first_result(&sent(&agent, 3)).contains("narrative:"));
    calls.assert_calls(0);
    assert_eq!(state.store.count_kind("api_request:internal").unwrap(), 0);
}

/// A compressor that cannot answer leaves the proxy exactly where the flag off leaves it.
#[tokio::test]
async fn a_failing_compressor_leaves_the_bytes_of_the_flag_off_run() {
    let run = |tag: &'static str, compressor: Option<String>| async move {
        let agent = Sink::new();
        let (addr, _state, task) = proxy_server(tag, |cfg| {
            configure(cfg, &agent.base(), compressor.as_deref());
        })
        .await;
        for n in 1..=3 {
            send(&addr, n).await;
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        task.abort();
        (1..=3)
            .map(|t| agent.all()[t - 1].0.clone())
            .collect::<Vec<_>>()
    };
    let broken = MockServer::start();
    broken.mock(|when, then| {
        when.any_request();
        then.status(500).body("boom");
    });
    let off = run("compress-live-eq-off", None).await;
    let on = run("compress-live-eq-on", Some(broken.base_url())).await;
    assert_eq!(off, on);
}

#[tokio::test]
async fn the_summary_reaches_the_next_turn_and_stays_byte_stable() {
    let (agent, compressor) = (Sink::new(), MockServer::start());
    let text = log();
    let summary = summary_with(&text, None);
    let calls = compressor.mock(|when, then| {
        when.method(POST).path("/v1/messages");
        then.status(200).json_body(reply(&summary));
    });
    let mut archive_dir = std::path::PathBuf::new();
    let (addr, state, task) = proxy_server("compress-live-on", |cfg| {
        configure(cfg, &agent.base(), Some(&compressor.base_url()));
        archive_dir = cfg.core.archive_dir.clone();
    })
    .await;
    send(&addr, 1).await;
    send(&addr, 2).await;
    // Turn 2 is the first time the result is outside the live zone: extractive, summary queued.
    let early = first_result(&sent(&agent, 2));
    assert!(early.contains("narrative:"), "{early}");
    let short = archive_id(&early);
    assert_eq!(short.len(), 8, "{short}");
    let hits = state
        .store
        .archive_ids_with_prefix(&short, 2)
        .expect("prefix");
    assert_eq!(hits.len(), 1, "{hits:?}");
    let id = hits[0].clone();
    assert!(stored(&state, &model(), &id).await.is_some(), "no summary");

    send(&addr, 3).await;
    send(&addr, 4).await;
    task.abort();
    let (third, fourth) = (sent(&agent, 3), sent(&agent, 4));
    let view = first_result(&third);
    assert!(view.contains(&format!("summary: {summary}")), "{view}");
    assert!(view.contains(&format!("expand({short})")), "{view}");
    assert!(!view.contains(&id), "{view}");
    // The prefix of turn 4 up to the live edge is turn 3's, byte for byte.
    let edge = third["messages"].as_array().expect("messages").len() - 2;
    for i in 0..edge {
        assert_eq!(third["messages"][i], fourth["messages"][i], "message {i}");
    }
    // One job per archive id: the second and third result are the same bytes, so the same id.
    calls.assert_calls(1);
    // The summary is a view; the archive still holds the original.
    let original = state
        .store
        .get_archive(&id, Some(&archive_dir))
        .expect("archive read")
        .expect("archived original");
    assert_eq!(String::from_utf8(original).expect("utf8"), text);
}

#[tokio::test]
async fn a_slow_compressor_never_delays_a_request() {
    let (agent, compressor) = (Sink::new(), MockServer::start());
    let summary = summary_with(&log(), None);
    compressor.mock(|when, then| {
        when.method(POST).path("/v1/messages");
        then.status(200)
            .json_body(reply(&summary))
            .delay(Duration::from_secs(4));
    });
    let (addr, _state, task) = proxy_server("compress-live-slow", |cfg| {
        configure(cfg, &agent.base(), Some(&compressor.base_url()));
    })
    .await;
    send(&addr, 1).await;
    let queued = send(&addr, 2).await;
    let after = send(&addr, 3).await;
    task.abort();
    assert!(queued < Duration::from_secs(2), "turn 2 took {queued:?}");
    assert!(after < Duration::from_secs(2), "turn 3 took {after:?}");
    // Still waiting on the model: the view is the extractive one.
    assert!(first_result(&sent(&agent, 3)).contains("narrative:"));
}

/// The same conversation on the OpenAI Chat Completions wire.
fn openai_request(n: usize) -> Vec<u8> {
    let mut messages = vec![json!({"role":"user","content":"start the task"})];
    for t in 1..=n {
        messages.push(json!({"role":"assistant","content":null,"tool_calls":[
            {"id":format!("tc-{t}"),"type":"function","function":{"name":"Bash","arguments":"{}"}}]}));
        messages.push(json!({"role":"tool","tool_call_id":format!("tc-{t}"),"content":log()}));
        // A turn is one user message, so each result needs one after it to age out.
        messages.push(json!({"role":"user","content":"continue"}));
    }
    serde_json::to_vec(&json!({
        "model": "gpt-test", "messages": messages, "user": SESSION
    }))
    .expect("request json")
}

async fn send_openai(addr: &str, n: usize) {
    let status = reqwest::Client::new()
        .post(format!("http://{addr}/v1/chat/completions"))
        .header("content-type", "application/json")
        .header("authorization", "Bearer sk-openai-test")
        .body(openai_request(n))
        .send()
        .await
        .expect("request through the proxy")
        .status();
    assert!(status.is_success(), "turn {n}: {status}");
}

/// T657: the compressor speaks the Anthropic Messages wire, so a credential from another
/// provider's agent turn must not be sent there.
#[tokio::test]
async fn an_openai_credential_is_never_sent_to_the_anthropic_compressor() {
    let (agent, compressor) = (Sink::new(), MockServer::start());
    let leaked = compressor.mock(|when, then| {
        when.any_request()
            .header("authorization", "Bearer sk-openai-test");
        then.status(200).json_body(reply("x"));
    });
    let any = compressor.mock(|when, then| {
        when.any_request();
        then.status(200).json_body(reply("x"));
    });
    let (addr, _state, task) = proxy_server("compress-live-openai-cred", |cfg| {
        configure(cfg, &agent.base(), Some(&compressor.base_url()));
    })
    .await;
    for n in 1..=3 {
        send_openai(&addr, n).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    task.abort();
    assert_eq!(agent.all().len(), 3, "the agent requests still go through");
    leaked.assert_calls(0);
    any.assert_calls(0);
}

/// Control for the test above: an Anthropic OAuth bearer still reaches the compressor.
#[tokio::test]
async fn an_anthropic_bearer_credential_still_reaches_the_compressor() {
    let (agent, compressor) = (Sink::new(), MockServer::start());
    let summary = summary_with(&log(), None);
    let calls = compressor.mock(|when, then| {
        when.method(POST)
            .path("/v1/messages")
            .header("authorization", "Bearer oauth-test");
        then.status(200).json_body(reply(&summary));
    });
    let (addr, state, task) = proxy_server("compress-live-anthropic-bearer", |cfg| {
        configure(cfg, &agent.base(), Some(&compressor.base_url()));
    })
    .await;
    send_with(&addr, 1, "authorization", "Bearer oauth-test").await;
    send_with(&addr, 2, "authorization", "Bearer oauth-test").await;
    let short = archive_id(&first_result(&sent(&agent, 2)));
    let hits = state
        .store
        .archive_ids_with_prefix(&short, 2)
        .expect("prefix");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(
        stored(&state, &model(), &hits[0]).await.is_some(),
        "no summary"
    );
    task.abort();
    calls.assert_calls(1);
}
