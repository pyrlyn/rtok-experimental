// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T385.7: per-lane upstream and in-flight cap through a real proxy. The bulk lane goes to its
//! own upstream, which holds every request until the test lets it go — the slowest upstream
//! there is — so "the agent turn is not queued behind the burst" is shown by the agent turn
//! finishing while every bulk slot is still taken, not by comparing timings.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use httpmock::prelude::*;
use rtok::proxy::lane::Lane;
use tokio::sync::Semaphore;

use crate::it::common;
use common::proxy::proxy_server;

const BODY: &str = r#"{"model":"m","max_tokens":1,"messages":[{"role":"user","content":"hi"}]}"#;

/// An upstream that counts each request and answers it only once the test releases it.
struct Held {
    arrived: AtomicUsize,
    release: Semaphore,
}

impl Held {
    async fn start() -> (String, Arc<Held>) {
        let held = Arc::new(Held {
            arrived: AtomicUsize::new(0),
            release: Semaphore::new(0),
        });
        let app = Router::new()
            .fallback(|State(h): State<Arc<Held>>| async move {
                h.arrived.fetch_add(1, Ordering::SeqCst);
                h.release.acquire().await.expect("open").forget();
                ([(CONTENT_TYPE, "application/json")], "{}")
            })
            .with_state(held.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        tokio::spawn(axum::serve(listener, app).into_future());
        (base, held)
    }

    fn arrived(&self) -> usize {
        self.arrived.load(Ordering::SeqCst)
    }
}

/// `server` answers `{}` at once on every path.
fn instant(server: &MockServer) -> httpmock::Mock<'_> {
    server.mock(|_, then| {
        then.status(200)
            .header("content-type", "application/json")
            .body("{}");
    })
}

async fn post(addr: &str, path: &str, lane: Option<&str>) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .post(format!("http://{addr}{path}"))
        .header("content-type", "application/json")
        .body(BODY);
    if let Some(lane) = lane {
        req = req.header("x-rtok-lane", lane);
    }
    req.send().await.expect("proxy answers")
}

/// Polls observable state; panics instead of hanging when it never comes.
async fn until(what: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_turn_is_not_queued_behind_a_bulk_burst() {
    let agent_up = MockServer::start();
    let answered = instant(&agent_up);
    let (bulk_up, held) = Held::start().await;
    let (addr, state, _task) = proxy_server("lane-gate-burst", |cfg| {
        cfg.proxy.upstream = agent_up.base_url();
        cfg.proxy.lanes.bulk.upstream = bulk_up.clone();
        cfg.proxy.lanes.bulk.max_in_flight = 2;
        cfg.proxy.lanes.bulk.max_queued = 1;
    })
    .await;

    // Two bulk requests take both slots upstream, a third waits in line.
    let mut burst = Vec::new();
    for _ in 0..3 {
        let addr = addr.clone();
        burst.push(tokio::spawn(async move {
            post(&addr, "/v1/messages", Some("bulk")).await.status()
        }));
        if burst.len() <= 2 {
            let n = burst.len();
            until("a bulk slot upstream", || held.arrived() == n).await;
        }
    }
    until("a queued bulk request", || state.queued(Lane::Bulk) == 1).await;

    // The line is full: the next bulk request is told to come back and never goes upstream.
    let turned_away = post(&addr, "/v1/messages", Some("bulk")).await;
    assert_eq!(turned_away.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        turned_away
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok()),
        Some("1")
    );
    let err: serde_json::Value =
        serde_json::from_slice(&turned_away.bytes().await.expect("body")).expect("json error");
    assert_eq!(err["error"]["type"], "rate_limit_error");

    // Every bulk slot is still held: an agent turn answers anyway, from its own upstream.
    let agent = tokio::time::timeout(Duration::from_secs(30), post(&addr, "/v1/messages", None))
        .await
        .expect("the agent turn is not queued behind bulk");
    assert_eq!(agent.status(), reqwest::StatusCode::OK);
    // A Batch call marked bulk keeps the provider upstream, never the bulk lane's.
    let batch = post(&addr, "/v1/messages/batches", Some("bulk")).await;
    assert_eq!(batch.status(), reqwest::StatusCode::OK);
    assert_eq!((held.arrived(), state.queued(Lane::Bulk)), (2, 1));

    held.release.add_permits(3);
    for request in burst {
        assert_eq!(request.await.expect("join"), reqwest::StatusCode::OK);
    }
    assert_eq!(
        held.arrived(),
        3,
        "the turned-away request never went upstream"
    );
    assert_eq!(state.queued(Lane::Bulk), 0);
    // Only the agent turn and the Batch call reached the default upstream.
    assert_eq!(answered.calls(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn proxy_switched_off_skips_lane_caps() {
    let (bulk_up, held) = Held::start().await;
    let (addr, state, _task) = proxy_server("lane-gate-off", |cfg| {
        cfg.proxy.enabled = false;
        cfg.proxy.lanes.bulk.upstream = bulk_up.clone();
        cfg.proxy.lanes.bulk.max_in_flight = 1;
        cfg.proxy.lanes.bulk.max_queued = 0;
    })
    .await;

    let mut burst = Vec::new();
    for _ in 0..3 {
        let addr = addr.clone();
        burst.push(tokio::spawn(async move {
            post(&addr, "/v1/messages", Some("bulk")).await.status()
        }));
    }
    until("all three bulk requests upstream at once", || {
        held.arrived() == 3
    })
    .await;
    assert_eq!(state.queued(Lane::Bulk), 0);
    held.release.add_permits(3);
    for request in burst {
        assert_eq!(request.await.expect("join"), reqwest::StatusCode::OK);
    }
}

#[rstest::rstest]
#[case(false)]
#[case(true)]
#[tokio::test(flavor = "multi_thread")]
async fn global_cap_applies_without_lanes_and_in_plain_mode(#[case] plain: bool) {
    let (upstream, held) = Held::start().await;
    let (addr, _, _task) = proxy_server(&format!("global-gate-{plain}"), |cfg| {
        cfg.proxy.upstream = upstream;
        cfg.proxy.enabled = !plain;
        cfg.proxy.lanes.enabled = false;
        cfg.proxy.max_in_flight = 1.try_into().unwrap();
    })
    .await;
    let first_addr = addr.clone();
    let first = tokio::spawn(async move {
        post(&first_addr, "/v1/messages", None)
            .await
            .bytes()
            .await
            .unwrap()
    });
    until("global slot", || held.arrived() == 1).await;
    let refused = post(&addr, "/v1/messages", None).await;
    assert_eq!(refused.status(), 429);
    assert_eq!(refused.headers()["retry-after"], "1");
    assert_eq!(held.arrived(), 1);
    drop(refused);
    held.release.add_permits(1);
    assert_eq!(first.await.unwrap().as_ref(), b"{}");
}

#[tokio::test(flavor = "multi_thread")]
async fn request_body_limit_rejects_before_forwarding_and_response_stays_whole() {
    let upstream = MockServer::start();
    let mock = upstream.mock(|_, then| {
        then.status(200).body("x".repeat(8192));
    });
    let (addr, _, _task) = proxy_server("body-limits", |cfg| {
        cfg.proxy.upstream = upstream.base_url();
        cfg.proxy.max_body_bytes = 128.try_into().unwrap();
    })
    .await;
    let client = reqwest::Client::new();
    let refused = client
        .post(format!("http://{addr}/v1/messages"))
        .body("x".repeat(129))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 413);
    assert_eq!(mock.calls(), 0);
    let forwarded = post(&addr, "/v1/messages", None).await;
    assert_eq!(forwarded.status(), 200);
    assert_eq!(forwarded.bytes().await.unwrap().len(), 8192);
    assert_eq!(mock.calls(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn global_slot_lasts_through_streaming_and_releases_on_disconnect() {
    use futures_util::{StreamExt, stream};
    let upstream = Router::new().fallback(|| async {
        axum::body::Body::from_stream(
            stream::once(async {
                Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"first"))
            })
            .chain(stream::pending()),
        )
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let upstream_task = tokio::spawn(axum::serve(listener, upstream).into_future());
    let (addr, _, _task) = proxy_server("global-stream", |cfg| {
        cfg.proxy.upstream = base;
        cfg.proxy.enabled = false;
        cfg.proxy.lanes.enabled = false;
        cfg.proxy.max_in_flight = 1.try_into().unwrap();
    })
    .await;
    let mut first = post(&addr, "/v1/messages", None).await;
    assert_eq!(first.chunk().await.unwrap().unwrap().as_ref(), b"first");
    assert_eq!(post(&addr, "/v1/messages", None).await.status(), 429);
    drop(first);
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let response = post(&addr, "/v1/messages", None).await;
            if response.status() == 200 {
                break;
            }
            assert_eq!(response.status(), 429);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("disconnect frees the slot even when upstream sends no more chunks");
    upstream_task.abort();
}
