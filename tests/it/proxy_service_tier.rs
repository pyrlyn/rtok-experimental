// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T385.12.2: the proxy records the tier the provider says it served a request on, from the
//! response (a Flex request can come back on another tier), and `usage_by_model_tier` lists
//! Flex usage under `<model>@flex`.

use std::time::Duration;

use httpmock::prelude::*;
use rtok::store::Store;

use crate::it::common;
use common::proxy::proxy_server;

const CHAT: &str = "/v1/chat/completions";
const USAGE: &str = r#""usage":{"prompt_tokens":100,"completion_tokens":10,"total_tokens":110}"#;

async fn used(store: &Store, model: &str) -> bool {
    for _ in 0..100 {
        if store
            .usage_by_model_tier()
            .expect("usage")
            .iter()
            .any(|u| u.model == model)
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

async fn send(addr: &str, lane: &str, model: &str) {
    let body = format!(r#"{{"model":"{model}","user":"s-{model}","messages":[]}}"#);
    reqwest::Client::new()
        .post(format!("http://{addr}{CHAT}"))
        .header("content-type", "application/json")
        .header("x-rtok-lane", lane)
        .body(body)
        .send()
        .await
        .expect("through the proxy")
        .bytes()
        .await
        .expect("body");
}

#[tokio::test]
async fn the_response_tier_prices_flex_usage_and_a_fallback_stays_standard() {
    let up = MockServer::start();
    // The upstream answers the model named in the body with the tier of that name's suffix.
    for (model, tier) in [("gpt-flexed", "flex"), ("gpt-fell-back", "default")] {
        up.mock(|when, then| {
            when.method(POST).path(CHAT).body_includes(model);
            then.status(200)
                .header("content-type", "application/json")
                .body(format!(r#"{{"id":"x","service_tier":"{tier}",{USAGE}}}"#));
        });
    }
    let base = up.base_url();
    let (addr, state, task) = proxy_server("service-tier", |cfg| {
        cfg.proxy.openai_upstream = base.clone();
        cfg.proxy.upstream = base;
    })
    .await;
    send(&addr, "bulk", "gpt-flexed").await;
    send(&addr, "bulk", "gpt-fell-back").await;
    assert!(used(&state.store, "gpt-flexed@flex").await);
    assert!(used(&state.store, "gpt-fell-back").await);
    let lanes = state.store.usage_by_lane_tier().expect("lanes");
    let tiers: Vec<_> = lanes
        .iter()
        .map(|l| (l.kind.as_str(), l.tier.as_deref()))
        .collect();
    assert_eq!(
        tiers,
        [
            ("api_request:bulk", Some("default")),
            ("api_request:bulk", Some("flex"))
        ]
    );
    task.abort();
}
