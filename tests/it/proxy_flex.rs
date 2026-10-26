// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T385.5: Flex `service_tier` on the bulk and internal lanes through a real proxy against a
//! scripted mock upstream. The omit / force / respect matrix, the agent lane and the Anthropic
//! wire left alone, and each `429` policy: `none`, `backoff` and `default`.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use rtok::config::Config;
use serde_json::Value;

use crate::common;
use common::proxy::proxy_server;

const CHAT: &str = "/v1/chat/completions";
/// Pretty-printed with odd spacing, so any re-serialisation shows.
const BODY: &str = "{ \"model\" : \"gpt-test\",\n  \"user\": \"sess-x\", \"messages\": [] }";

type Seen = Arc<Mutex<Vec<Vec<u8>>>>;

/// Answers with the scripted statuses in order, the last one repeating, and keeps every body.
#[derive(Clone)]
struct Upstream {
    addr: String,
    seen: Seen,
}

type Script = (Vec<u16>, Seen, Option<&'static str>);

async fn answer(
    State((script, seen, retry_after)): State<Script>,
    body: Bytes,
) -> impl IntoResponse {
    let mut seen = seen.lock().expect("seen");
    let n = seen.len();
    seen.push(body.to_vec());
    let status = *script.get(n).or(script.last()).unwrap_or(&200);
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    if let (429, Some(v)) = (status, retry_after) {
        headers.insert("retry-after", HeaderValue::from_static(v));
    }
    (StatusCode::from_u16(status).expect("status"), headers, "{}")
}

async fn upstream(script: &[u16]) -> Upstream {
    upstream_asking(script, None).await
}

/// Every `429` it sends carries `Retry-After: <retry_after>`.
async fn upstream_asking(script: &[u16], retry_after: Option<&'static str>) -> Upstream {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let app =
        Router::new()
            .fallback(answer)
            .with_state((script.to_vec(), seen.clone(), retry_after));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = format!("http://{}", listener.local_addr().expect("addr"));
    tokio::spawn(axum::serve(listener, app).into_future());
    Upstream { addr, seen }
}

impl Upstream {
    fn bodies(&self) -> Vec<String> {
        let seen = self.seen.lock().expect("seen");
        seen.iter()
            .map(|b| String::from_utf8(b.clone()).expect("utf8"))
            .collect()
    }
}

/// `tune` runs after the upstream and a fast backoff are set.
async fn proxy(
    tag: &str,
    up: &Upstream,
    tune: impl FnOnce(&mut Config),
) -> (String, tokio::task::JoinHandle<std::io::Result<()>>) {
    let base = up.addr.clone();
    let (addr, _state, task) = proxy_server(tag, |cfg| {
        // Both: the OpenAI wires default to the real API, which a test must never reach.
        cfg.proxy.openai_upstream = base.clone();
        cfg.proxy.upstream = base;
        cfg.proxy.lanes.bulk.flex = true;
        cfg.proxy.lanes.internal.flex = true;
        cfg.proxy.flex.backoff_ms = 1;
        tune(cfg);
    })
    .await;
    (addr, task)
}

async fn post(addr: &str, path: &str, lane: Option<&str>, body: &str) -> u16 {
    let mut req = reqwest::Client::new()
        .post(format!("http://{addr}{path}"))
        .header("content-type", "application/json")
        .body(body.to_string());
    if let Some(lane) = lane {
        req = req.header("x-rtok-lane", lane);
    }
    req.send()
        .await
        .expect("through the proxy")
        .status()
        .as_u16()
}

fn json(s: &str) -> Value {
    serde_json::from_str(s).expect("json")
}

/// `body` with `service_tier` removed, for "differs only in the tier" checks.
fn without_tier(s: &str) -> Value {
    let mut v = json(s);
    v.as_object_mut().expect("object").remove("service_tier");
    v
}

#[tokio::test]
async fn flex_is_set_on_bulk_and_internal_only() {
    let up = upstream(&[200]).await;
    let (addr, task) = proxy("flex-lanes", &up, |_| {}).await;
    for lane in [Some("bulk"), Some("internal")] {
        assert_eq!(post(&addr, CHAT, lane, BODY).await, 200);
    }
    assert_eq!(
        post(&addr, "/lane/bulk/v1/responses", None, BODY).await,
        200
    );
    for (i, sent) in up.bodies().iter().enumerate() {
        assert_eq!(json(sent)["service_tier"], "flex", "request {i}");
        // Only the field was added: the client's own bytes follow it unchanged.
        assert!(sent.ends_with(&BODY[1..]), "request {i}: {sent}");
    }
    // The agent lane (no header, or an explicit one) is never opened silently.
    for lane in [None, Some("agent")] {
        assert_eq!(post(&addr, CHAT, lane, BODY).await, 200);
        assert_eq!(up.bodies().last().expect("sent"), BODY);
    }
    task.abort();
}

#[tokio::test]
async fn a_lane_without_the_switch_and_other_wires_are_untouched() {
    let up = upstream(&[200]).await;
    let (addr, task) = proxy("flex-off", &up, |cfg| {
        cfg.proxy.lanes.bulk.flex = false;
        cfg.proxy.lanes.internal.flex = true;
    })
    .await;
    assert_eq!(post(&addr, CHAT, Some("bulk"), BODY).await, 200);
    assert_eq!(up.bodies().last().expect("sent"), BODY);
    // Anthropic has no Flex tier, whatever the lane says.
    let anthropic = r#"{"model":"claude-test","messages":[]}"#;
    assert_eq!(
        post(&addr, "/v1/messages", Some("internal"), anthropic).await,
        200
    );
    assert_eq!(up.bodies().last().expect("sent"), anthropic);
    // Neither is a body the proxy cannot parse as a JSON object.
    assert_eq!(post(&addr, CHAT, Some("internal"), "[1]").await, 200);
    assert_eq!(up.bodies().last().expect("sent"), "[1]");
    task.abort();
}

#[tokio::test]
async fn a_client_tier_is_respected_unless_forced() {
    let up = upstream(&[200]).await;
    let (addr, task) = proxy("flex-respect", &up, |_| {}).await;
    let (forcing, ftask) = proxy("flex-force", &up, |cfg| cfg.proxy.flex.force = true).await;
    for tier in ["auto", "default", "priority", "flex"] {
        let body = format!(r#"{{"model":"m","service_tier":"{tier}"}}"#);
        assert_eq!(post(&addr, CHAT, Some("bulk"), &body).await, 200);
        assert_eq!(up.bodies().last().expect("sent"), &body, "{tier} respected");
    }
    let body = r#"{"model":"m","service_tier":"priority","n":1}"#;
    assert_eq!(post(&forcing, CHAT, Some("bulk"), body).await, 200);
    let sent = up.bodies().pop().expect("sent");
    assert_eq!(json(&sent)["service_tier"], "flex");
    assert_eq!(without_tier(&sent), without_tier(body));
    task.abort();
    ftask.abort();
}

#[tokio::test]
async fn policy_none_hands_the_429_to_the_client() {
    let up = upstream(&[429, 200]).await;
    let (addr, task) = proxy("flex-none", &up, |_| {}).await;
    assert_eq!(post(&addr, CHAT, Some("bulk"), BODY).await, 429);
    assert_eq!(up.bodies().len(), 1);
    task.abort();
}

#[tokio::test]
async fn backoff_retries_on_flex_until_it_succeeds_or_runs_out() {
    let up = upstream(&[429, 429, 200]).await;
    let (addr, task) = proxy("flex-backoff", &up, |cfg| {
        cfg.proxy.flex.on_429 = "backoff".to_string();
    })
    .await;
    assert_eq!(post(&addr, CHAT, Some("bulk"), BODY).await, 200);
    let sent = up.bodies();
    assert_eq!(sent.len(), 3);
    assert!(
        sent.iter()
            .all(|b| b == &sent[0] && json(b)["service_tier"] == "flex")
    );
    task.abort();

    let never = upstream(&[429]).await;
    let (addr, task) = proxy("flex-backoff-out", &never, |cfg| {
        cfg.proxy.flex.on_429 = "backoff".to_string();
        cfg.proxy.flex.retries = 2;
    })
    .await;
    assert_eq!(post(&addr, CHAT, Some("bulk"), BODY).await, 429);
    assert_eq!(never.bodies().len(), 3, "one try and two retries");
    task.abort();
}

#[tokio::test]
async fn default_retries_once_on_auto_and_changes_only_the_tier() {
    let up = upstream(&[429, 200]).await;
    let (addr, task) = proxy("flex-default", &up, |cfg| {
        cfg.proxy.flex.on_429 = "default".to_string();
    })
    .await;
    assert_eq!(post(&addr, CHAT, Some("internal"), BODY).await, 200);
    let sent = up.bodies();
    assert_eq!(sent.len(), 2);
    assert_eq!(json(&sent[0])["service_tier"], "flex");
    assert_eq!(json(&sent[1])["service_tier"], "auto");
    assert_eq!(sent[1], sent[0].replace("flex", "auto"));
    assert!(sent[1].ends_with(&BODY[1..]));
    task.abort();

    // A second 429 on standard processing is a real limit: it is not retried again.
    let limited = upstream(&[429]).await;
    let (addr, task) = proxy("flex-default-limit", &limited, |cfg| {
        cfg.proxy.flex.on_429 = "default".to_string();
    })
    .await;
    assert_eq!(post(&addr, CHAT, Some("bulk"), BODY).await, 429);
    assert_eq!(limited.bodies().len(), 2);
    task.abort();
}

#[tokio::test]
async fn a_forced_over_a_client_tier_falls_back_to_it_and_a_client_flex_is_not_retried() {
    let up = upstream(&[429, 200]).await;
    let (addr, task) = proxy("flex-force-default", &up, |cfg| {
        cfg.proxy.flex.force = true;
        cfg.proxy.flex.on_429 = "default".to_string();
    })
    .await;
    let body = r#"{"model":"m","service_tier":"priority"}"#;
    assert_eq!(post(&addr, CHAT, Some("bulk"), body).await, 200);
    let tiers: Vec<_> = up
        .bodies()
        .iter()
        .map(|b| json(b)["service_tier"].clone())
        .collect();
    assert_eq!(tiers, ["flex", "priority"]);
    task.abort();

    // `auto` is for a request that named no tier at all.
    let none = upstream(&[429, 200]).await;
    let (addr, task) = proxy("flex-force-none", &none, |cfg| {
        cfg.proxy.flex.force = true;
        cfg.proxy.flex.on_429 = "default".to_string();
    })
    .await;
    assert_eq!(
        post(&addr, CHAT, Some("bulk"), r#"{"model":"m"}"#).await,
        200
    );
    let tiers: Vec<_> = none
        .bodies()
        .iter()
        .map(|b| json(b)["service_tier"].clone())
        .collect();
    assert_eq!(tiers, ["flex", "auto"]);
    task.abort();

    // The client chose Flex itself, so its 429 is its own to handle.
    let own = upstream(&[429, 200]).await;
    let (addr, task) = proxy("flex-client-owned", &own, |cfg| {
        cfg.proxy.flex.on_429 = "default".to_string();
    })
    .await;
    let body = r#"{"model":"m","service_tier":"flex"}"#;
    assert_eq!(post(&addr, CHAT, Some("bulk"), body).await, 429);
    assert_eq!(own.bodies(), [body]);
    task.abort();
}

#[tokio::test]
async fn a_retry_after_over_the_cap_is_not_waited_for() {
    // An hour asked for: holding the client for it is not an option, so `backoff` hands the
    // 429 over at once and `default` goes straight to its one fallback try.
    let up = upstream_asking(&[429], Some("3600")).await;
    let (addr, task) = proxy("flex-ra-backoff", &up, |cfg| {
        cfg.proxy.flex.on_429 = "backoff".to_string();
    })
    .await;
    assert_eq!(post(&addr, CHAT, Some("bulk"), BODY).await, 429);
    assert_eq!(up.bodies().len(), 1);
    task.abort();

    let up = upstream_asking(&[429, 200], Some("3600")).await;
    let (addr, task) = proxy("flex-ra-default", &up, |cfg| {
        cfg.proxy.flex.on_429 = "default".to_string();
    })
    .await;
    assert_eq!(post(&addr, CHAT, Some("bulk"), BODY).await, 200);
    assert_eq!(up.bodies().len(), 2);
    task.abort();

    // A date is not read: our own (1 ms) delay applies and the retry happens.
    let up = upstream_asking(&[429, 200], Some("Wed, 21 Oct 2099 07:28:00 GMT")).await;
    let (addr, task) = proxy("flex-ra-date", &up, |cfg| {
        cfg.proxy.flex.on_429 = "backoff".to_string();
    })
    .await;
    assert_eq!(post(&addr, CHAT, Some("bulk"), BODY).await, 200);
    assert_eq!(up.bodies().len(), 2);
    task.abort();
}
