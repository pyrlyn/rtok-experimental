// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T71.4: skill listing overhead measured through the proxy `call_io` path.

use std::sync::Arc;

use httpmock::prelude::*;
use rtok::config::Config;
use rtok::measure::skills_listing::measure;
use rtok::proxy::{ProxyState, app};

#[tokio::test]
async fn proxy_capture_measures_skill_listing_overhead() {
    let body = include_bytes!("../fixtures/proxy/skills_listing_request.json");
    let fixture = measure(body).expect("fixture carries a skills block");
    assert_eq!(fixture.count, 3);

    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST).path("/v1/messages");
        then.status(200)
            .header("content-type", "application/json")
            .body(include_str!(
                "../fixtures/proxy/anthropic_messages_body.json"
            ));
    });

    let dir = std::env::temp_dir().join(format!("rtok-skill-listing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cfg = Config::load_from(&dir).expect("config");
    cfg.proxy.upstream = server.base_url();
    let state = Arc::new(ProxyState::new(&cfg).expect("proxy state"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let task = tokio::spawn(axum::serve(listener, app(state.clone())).into_future());

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/v1/messages"))
        .header("content-type", "application/json")
        .body(body.to_vec())
        .send()
        .await
        .expect("proxy request");
    assert!(resp.status().is_success(), "status {}", resp.status());

    for _ in 0..20 {
        if state.store.count_call_io().unwrap_or(0) > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(state.store.count_call_io().expect("call_io"), 1);
    let call_id = state.store.calls_after(0, 1).expect("calls")[0].id;
    let req = state
        .store
        .call_io_request(call_id)
        .expect("read")
        .expect("request bytes");
    let wire = measure(&req).expect("stored request");
    assert_eq!(wire.count, fixture.count);
    assert_eq!(
        wire.framing_bytes_per_skill,
        fixture.framing_bytes_per_skill
    );

    task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}
