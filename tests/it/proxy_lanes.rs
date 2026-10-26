// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T385.1: the lane classifier through a real proxy against a mock upstream. Each lane path
//! gets its `calls.kind` tag, `/v1/messages` stays an agent turn, and the lane marker never
//! reaches the provider.

use std::time::Duration;

use httpmock::HttpMockRequest;
use httpmock::prelude::*;
use rtok::store::Store;

use crate::common;
use common::proxy::proxy_server;

const BODY: &str = r#"{"model":"m","messages":[]}"#;

/// Upstream that answers anything but fails the hop when the lane marker leaks through.
fn upstream() -> (MockServer, String) {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.is_true(|req: &HttpMockRequest| req.headers().get("x-rtok-lane").is_none());
        then.status(200).body("{}");
    });
    let base = server.base_url();
    (server, base)
}

async fn send(addr: &str, path: &str, header: Option<&str>) -> reqwest::StatusCode {
    let mut req = reqwest::Client::new()
        .post(format!("http://{addr}{path}"))
        .body(BODY);
    if let Some(v) = header {
        req = req.header("x-rtok-lane", v);
    }
    req.send()
        .await
        .expect("request through the proxy")
        .status()
}

/// `finish` and `record` write after the body was forwarded: wait for the row.
async fn kind_count(store: &Store, kind: &str, n: i64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while store.count_kind(kind).expect("count") < n {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no {kind} row appeared"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn each_lane_path_is_tagged_and_the_marker_never_reaches_upstream() {
    let (_up, base) = upstream();
    let (addr, state, task) = proxy_server("lanes", |cfg| {
        cfg.proxy.upstream = base.clone();
        cfg.proxy.openai_upstream = base;
    })
    .await;
    let cases = [
        ("/v1/messages", None, "api_request"),
        ("/v1/messages/batches", None, "api_request:batch"),
        ("/v1/batches", None, "api_request:batch"),
        ("/v1/files", None, "api_request:files"),
        ("/v1/embeddings", None, "api_request:embeddings"),
        ("/v1/models", None, "api_request:meta"),
        ("/v1/messages/count_tokens", None, "api_request:meta"),
        ("/v1/messages", Some("bulk"), "api_request:bulk"),
        ("/lane/internal/v1/messages", None, "api_request:internal"),
        // A path that names its lane beats the marker.
        ("/v1/messages/batches", Some("bulk"), "api_request:batch"),
    ];
    for (path, header, _) in cases {
        let status = send(&addr, path, header).await;
        assert_eq!(status, reqwest::StatusCode::OK, "{path} {header:?}");
    }
    for kind in [
        "api_request",
        "api_request:batch",
        "api_request:files",
        "api_request:embeddings",
        "api_request:meta",
        "api_request:bulk",
        "api_request:internal",
    ] {
        let expected = cases.iter().filter(|(_, _, k)| *k == kind).count() as i64;
        kind_count(&state.store, kind, expected).await;
        assert_eq!(
            state.store.count_kind(kind).expect("count"),
            expected,
            "{kind}"
        );
    }
    task.abort();
}

#[tokio::test]
async fn lane_prefix_is_stripped_before_forwarding() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/v1/messages").body(BODY);
        then.status(200).body("{}");
    });
    let (addr, _state, task) = proxy_server("lanes-prefix", |cfg| {
        cfg.proxy.upstream = server.base_url();
    })
    .await;
    send(&addr, "/lane/bulk/v1/messages", None).await;
    mock.assert();
    task.abort();
}

#[tokio::test]
async fn lanes_off_tags_nothing_and_forwards_the_marker_as_sent() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/messages")
            .header("x-rtok-lane", "bulk");
        then.status(200).body("{}");
    });
    let (addr, state, task) = proxy_server("lanes-off", |cfg| {
        cfg.proxy.upstream = server.base_url();
        cfg.proxy.lanes.enabled = false;
    })
    .await;
    send(&addr, "/v1/messages", Some("bulk")).await;
    mock.assert();
    kind_count(&state.store, "api_request", 1).await;
    assert_eq!(
        state.store.count_kind("api_request:bulk").expect("count"),
        0
    );
    task.abort();
}
