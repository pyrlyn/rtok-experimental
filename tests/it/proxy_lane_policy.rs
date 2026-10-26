// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T385.2: the per-lane policy table through a real proxy against a mock upstream, with every
//! rewrite switch on globally. Bulk, batch and the other non-agent lanes are forwarded byte
//! for byte; the agent lane is rewritten exactly as it was before lanes; a lane switch opens
//! one rewrite at a time.

use std::time::Duration;

use httpmock::prelude::*;
use rtok::config::Config;
use serde_json::{Value, json};

use crate::it::common;
use common::proxy::{Sink, proxy_server};

const SESSION: &str = "sess-lane-policy";
/// Well over `proxy.tools_rewrite.max_description_tokens` (60), so the rewrite has work to do.
const LONG_DESCRIPTION: &str = "Runs a shell command and returns its output. It accepts a working directory, a timeout and an environment, streams long output back in chunks, and reports the exit status separately. Prefer it over ad hoc scripts, and keep each command short and idempotent whenever possible.";

/// Six user turns of a 400-line tool result (past `archive.min_tokens`), a long tool
/// description for `tools_rewrite`, and a pretty-printed body so any re-serialisation shows.
fn request() -> Vec<u8> {
    let mut messages = Vec::new();
    for t in 1..=6 {
        let text = (1..=400)
            .map(|i| format!("t{t} line {i}: some shell output with words"))
            .collect::<Vec<_>>()
            .join("\n");
        messages.push(json!({"role":"user","content":[
            {"type":"tool_result","tool_use_id":format!("tu-{t}"),"content":text}]}));
        messages.push(json!({"role":"assistant","content":[
            {"type":"tool_use","id":format!("tu-{}", t + 1),"name":"Bash","input":{}}]}));
    }
    serde_json::to_vec_pretty(&json!({
        "model": "claude-test", "max_tokens": 8, "system": "sys",
        "tools": [{"name": "Bash", "description": LONG_DESCRIPTION,
                   "input_schema": {"type": "object"}}],
        "messages": messages, "metadata": {"user_id": SESSION}
    }))
    .expect("request json")
}

/// Every global rewrite switch on: what the agent lane has always been subject to.
fn all_switches_on(cfg: &mut Config, base: &str) {
    // Every provider upstream points at the mock, so no case can reach a real API.
    cfg.proxy.upstream = base.to_string();
    cfg.proxy.openai_upstream = base.to_string();
    cfg.proxy.gemini_upstream = base.to_string();
    cfg.proxy.mode = "compress".to_string();
    cfg.proxy.tools_rewrite.enabled = true;
    cfg.proxy.context_management = true;
    cfg.plugins.toon.enabled = true;
}

async fn post(addr: &str, path: &str, lane: Option<&str>, body: Vec<u8>) -> reqwest::StatusCode {
    let mut req = reqwest::Client::new()
        .post(format!("http://{addr}{path}"))
        .header("content-type", "application/json")
        .body(body);
    if let Some(lane) = lane {
        req = req.header("x-rtok-lane", lane);
    }
    req.send()
        .await
        .expect("request through the proxy")
        .status()
}

#[tokio::test]
async fn non_agent_lanes_are_forwarded_byte_for_byte_in_compress_mode() {
    let up = Sink::new();
    let (addr, _state, task) = proxy_server("lane-policy-bytes", |cfg| {
        all_switches_on(cfg, &up.base());
    })
    .await;
    let body = request();
    let cases = [
        ("/v1/messages", Some("bulk")),
        ("/lane/internal/v1/messages", None),
        ("/v1/messages/count_tokens", None),
        ("/v1/embeddings", None),
        ("/v1/messages/batches", None),
        // A marker never opens a path that names a pass-through lane.
        ("/v1/messages/batches", Some("bulk")),
        ("/v1/files", None),
    ];
    for (path, lane) in cases {
        assert!(post(&addr, path, lane, body.clone()).await.is_success());
        let (sent, beta) = up.last();
        assert_eq!(sent, body, "{path} {lane:?} was rewritten");
        assert!(!beta, "{path} {lane:?} carries the context-management beta");
    }
    task.abort();
}

#[tokio::test]
async fn agent_lane_is_rewritten_as_before_lanes_existed() {
    let before = Sink::new();
    let (addr, _s, task) = proxy_server("lane-policy-agent-off", |cfg| {
        all_switches_on(cfg, &before.base());
        // Lanes off: every request is the agent lane, which is the pre-T385 behaviour.
        cfg.proxy.lanes.enabled = false;
    })
    .await;
    assert!(
        post(&addr, "/v1/messages", None, request())
            .await
            .is_success()
    );
    let (reference, reference_beta) = before.last();
    task.abort();

    let up = Sink::new();
    let (addr, _s, task) = proxy_server("lane-policy-agent-on", |cfg| {
        all_switches_on(cfg, &up.base());
    })
    .await;
    for lane in [None, Some("agent")] {
        assert!(
            post(&addr, "/v1/messages", lane, request())
                .await
                .is_success()
        );
        let (sent, beta) = up.last();
        assert_ne!(sent, request(), "the agent lane must still be rewritten");
        assert_eq!(sent, reference, "agent bytes drifted ({lane:?})");
        assert_eq!(beta, reference_beta);
        assert!(beta, "the agent lane must still arm context management");
    }
    let sent: Value = serde_json::from_slice(&up.last().0).expect("json");
    assert!(sent["context_management"].is_object());
    assert!(
        sent["tools"][0]["description"]
            .as_str()
            .expect("description")
            .len()
            < LONG_DESCRIPTION.len()
    );
    task.abort();
}

#[tokio::test]
async fn a_lane_switch_opens_exactly_one_rewrite() {
    let up = Sink::new();
    let (addr, _s, task) = proxy_server("lane-policy-opt-in", |cfg| {
        all_switches_on(cfg, &up.base());
        cfg.proxy.lanes.bulk.tools_rewrite = true;
        cfg.proxy.lanes.internal.context_management = true;
    })
    .await;
    let body = request();

    assert!(
        post(&addr, "/v1/messages", Some("bulk"), body.clone())
            .await
            .is_success()
    );
    let (sent, beta) = up.last();
    let sent: Value = serde_json::from_slice(&sent).expect("json");
    assert!(!beta && sent.get("context_management").is_none());
    assert!(
        sent["tools"][0]["description"]
            .as_str()
            .expect("description")
            .len()
            < LONG_DESCRIPTION.len(),
        "bulk opted in to tools_rewrite"
    );
    assert!(
        !sent["messages"][0]["content"][0]["content"]
            .as_str()
            .expect("tool result")
            .starts_with("[archived "),
        "bulk did not opt in to compress"
    );

    assert!(
        post(&addr, "/v1/messages", Some("internal"), body.clone())
            .await
            .is_success()
    );
    let (sent, beta) = up.last();
    let sent: Value = serde_json::from_slice(&sent).expect("json");
    assert!(beta && sent["context_management"].is_object());
    assert_eq!(sent["tools"][0]["description"], LONG_DESCRIPTION);
    task.abort();
}

fn tabular_request() -> Vec<u8> {
    let table = serde_json::to_string(
        &(1..=8)
            .map(|i| json!({"a": i, "b": i, "c": i}))
            .collect::<Vec<_>>(),
    )
    .expect("table");
    let mut messages = Vec::new();
    for t in 1..=6 {
        messages.push(json!({"role":"user","content":[
            {"type":"tool_result","tool_use_id":format!("tu-{t}"),"content":table}]}));
        messages.push(json!({"role":"assistant","content":[
            {"type":"tool_use","id":format!("tu-{}", t + 1),"name":"Bash","input":{}}]}));
    }
    serde_json::to_vec(&json!({
        "model": "claude-test", "max_tokens": 8, "messages": messages,
        "metadata": {"user_id": "sess-lane-toon"}
    }))
    .expect("request json")
}

#[tokio::test]
async fn toon_needs_its_own_lane_switch_inside_a_compressing_lane() {
    let up = Sink::new();
    let (addr, state, task) = proxy_server("lane-policy-toon", |cfg| {
        all_switches_on(cfg, &up.base());
        cfg.proxy.lanes.bulk.compress = true;
        cfg.proxy.lanes.internal.compress = true;
        cfg.proxy.lanes.internal.toon = true;
    })
    .await;
    assert!(
        post(&addr, "/v1/messages", Some("bulk"), tabular_request())
            .await
            .is_success()
    );
    assert_eq!(state.store.measurement_count("toon").expect("count"), 0);
    assert!(
        post(&addr, "/v1/messages", Some("internal"), tabular_request())
            .await
            .is_success()
    );
    assert!(state.store.measurement_count("toon").expect("count") > 0);
    task.abort();
}

#[tokio::test]
async fn the_semantic_cache_serves_the_agent_lane_only_by_default() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/v1/messages");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"type":"message","usage":{"input_tokens":1,"output_tokens":2}}"#);
    });
    let (addr, state, task) = proxy_server("lane-policy-cache", |cfg| {
        cfg.proxy.upstream = server.base_url();
        cfg.plugins.proxy.semantic_cache.enabled = true;
    })
    .await;
    // The proxy fills the cache after the reply has streamed, just before the usage row, so a
    // usage row is the sign that a fill (if any) is done; without the wait the next call races it.
    let recorded = |n: i64| {
        let store = &state.store;
        async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            while store.count_usage().expect("usage rows") < n {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "{n} usage rows never appeared"
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    };
    let body = br#"{"model":"claude-test","messages":[{"role":"user","content":"hi"}]}"#.to_vec();
    // Bulk neither reads nor fills the cache, so both calls reach upstream.
    for n in 1..=2 {
        assert!(
            post(&addr, "/v1/messages", Some("bulk"), body.clone())
                .await
                .is_success()
        );
        recorded(n).await;
    }
    mock.assert_calls(2);
    // The agent lane starts cold even though bulk sent the same prompt, then hits.
    assert!(
        post(&addr, "/v1/messages", None, body.clone())
            .await
            .is_success()
    );
    recorded(3).await;
    assert!(
        post(&addr, "/v1/messages", None, body.clone())
            .await
            .is_success()
    );
    mock.assert_calls(3);
    task.abort();
}

/// T533: the cache key names the upstream request target. Two cache-enabled lanes that send
/// the same body and credentials to different upstreams must each reach their own upstream,
/// and a replay to one target still hits.
#[tokio::test]
async fn cached_responses_are_scoped_to_the_upstream_target() {
    let (a, b) = (MockServer::start(), MockServer::start());
    fn mock_for<'a>(server: &'a MockServer, answer: &'static str) -> httpmock::Mock<'a> {
        server.mock(move |when, then| {
            when.method(POST).path("/v1/messages");
            then.status(200)
                .header("content-type", "application/json")
                .body(answer);
        })
    }
    let ma = mock_for(
        &a,
        r#"{"type":"message","from":"a","usage":{"input_tokens":1,"output_tokens":2}}"#,
    );
    let mb = mock_for(
        &b,
        r#"{"type":"message","from":"b","usage":{"input_tokens":1,"output_tokens":2}}"#,
    );
    let (addr, state, task) = proxy_server("lane-policy-cache-target", |cfg| {
        cfg.proxy.upstream = a.base_url();
        cfg.proxy.lanes.internal.upstream = b.base_url();
        cfg.proxy.lanes.internal.semantic_cache = true;
        cfg.plugins.proxy.semantic_cache.enabled = true;
    })
    .await;
    let body = br#"{"model":"claude-test","messages":[{"role":"user","content":"hi"}]}"#.to_vec();
    let text = |lane: Option<&'static str>| {
        let (addr, body) = (addr.clone(), body.clone());
        async move {
            let mut req = reqwest::Client::new()
                .post(format!("http://{addr}/v1/messages"))
                .header("content-type", "application/json")
                .body(body);
            if let Some(lane) = lane {
                req = req.header("x-rtok-lane", lane);
            }
            req.send()
                .await
                .expect("request")
                .text()
                .await
                .expect("body")
        }
    };
    let wait_usage = |n: i64| {
        let store = &state.store;
        async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            while store.count_usage().expect("usage rows") < n {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "usage {n} never appeared"
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    };
    assert!(text(None).await.contains(r#""from":"a""#));
    wait_usage(1).await;
    assert!(
        text(Some("internal")).await.contains(r#""from":"b""#),
        "the warm entry for upstream A must not answer a request bound for B"
    );
    wait_usage(2).await;
    ma.assert_calls(1);
    mb.assert_calls(1);
    assert!(text(None).await.contains(r#""from":"a""#));
    assert!(text(Some("internal")).await.contains(r#""from":"b""#));
    ma.assert_calls(1);
    mb.assert_calls(1);
    task.abort();
}

/// T584: a streamed reply that ends inside a multibyte character still records the usage and
/// tier its complete `message_start` event carried, and the client gets the bytes unchanged.
#[tokio::test]
async fn a_stream_cut_inside_a_character_keeps_its_earlier_usage() {
    let start = json!({"type": "message_start", "message": {"usage": {
        "input_tokens": 100, "output_tokens": 1, "service_tier": "standard"}}});
    let mut stream = format!("event: message_start\ndata: {start}\n\n").into_bytes();
    stream.extend_from_slice(b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"\xc3");
    let server = MockServer::start();
    let reply = stream.clone();
    server.mock(|when, then| {
        when.method(POST).path("/v1/messages");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(reply);
    });
    let (addr, state, task) = proxy_server("lane-policy-utf8-tail", |cfg| {
        cfg.proxy.upstream = server.base_url();
    })
    .await;
    let body = json!({"model": "claude-test", "max_tokens": 8, "metadata": {"user_id": SESSION},
        "messages": [{"role": "user", "content": "hi"}]});
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/v1/messages"))
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .expect("request through the proxy");
    assert_eq!(
        resp.bytes().await.expect("body").as_ref(),
        stream.as_slice()
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let rows = loop {
        let rows = state.store.usage_rows(SESSION).expect("usage rows");
        if !rows.is_empty() {
            break rows;
        }
        assert!(tokio::time::Instant::now() < deadline, "no usage row");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert_eq!((rows[0].input, rows[0].output), (100, 1), "{rows:?}");
    task.abort();
}

#[tokio::test]
async fn a_lane_timeout_bounds_only_that_lane() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST).path("/v1/messages");
        then.status(200)
            .delay(Duration::from_millis(2500))
            .body("{}");
    });
    let (addr, _s, task) = proxy_server("lane-policy-timeout", |cfg| {
        cfg.proxy.upstream = server.base_url();
        cfg.proxy.lanes.bulk.timeout_s = 1;
    })
    .await;
    let body = br#"{"model":"claude-test","messages":[]}"#.to_vec();
    assert_eq!(
        post(&addr, "/v1/messages", Some("bulk"), body.clone()).await,
        reqwest::StatusCode::BAD_GATEWAY
    );
    assert!(post(&addr, "/v1/messages", None, body).await.is_success());
    task.abort();
}
