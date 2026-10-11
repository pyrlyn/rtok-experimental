// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T385.10: model routing on the `bulk` and `internal` lanes through a real proxy against a mock
//! upstream. The agent lane is never routed, a rule missing its quality gate does nothing, the
//! ledger names the model that served the call, and `stats --price` costs each lane at it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use httpmock::HttpMockRequest;
use httpmock::prelude::*;
use rtok::config::{Config, RouteRule};
use rtok::measure::stats::{Report, attach_costs};
use rtok::proxy::ProxyState;
use serde_json::Value;

mod common;
use common::proxy::proxy_server;

const CHAT: &str = "/v1/chat/completions";
const BIG: &str = "gpt-5";
const SMALL: &str = "gpt-5-mini";
/// A fixed answer, whatever model is asked: only the model's price then differs between runs.
const USAGE: &str =
    r#""usage":{"prompt_tokens":1000000,"completion_tokens":100000,"total_tokens":1100000}"#;

/// An upstream that records the `model` of every request it receives.
fn upstream() -> (MockServer, Arc<Mutex<Vec<String>>>) {
    let server = MockServer::start();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    server.mock(move |when, then| {
        when.is_true(move |req: &HttpMockRequest| {
            let body: Value = serde_json::from_slice(&req.body_vec()).expect("json");
            sink.lock()
                .expect("sink")
                .push(body["model"].as_str().expect("model").to_string());
            true
        });
        then.status(200)
            .header("content-type", "application/json")
            .body(format!(r#"{{"id":"x",{USAGE}}}"#));
    });
    (server, seen)
}

fn rule(from: &[&str], to: &str, gate: &str) -> RouteRule {
    RouteRule {
        from: from.iter().map(|s| s.to_string()).collect(),
        to: to.into(),
        gate: gate.into(),
        ..RouteRule::default()
    }
}

async fn send(addr: &str, lane: Option<&str>, session: &str) {
    let mut req = reqwest::Client::new()
        .post(format!("http://{addr}{CHAT}"))
        .header("content-type", "application/json")
        .body(format!(
            r#"{{"model":"{BIG}","user":"{session}","messages":[]}}"#
        ));
    if let Some(lane) = lane {
        req = req.header("x-rtok-lane", lane);
    }
    req.send()
        .await
        .expect("through the proxy")
        .bytes()
        .await
        .expect("body");
}

/// Wait until `n` usage rows exist; the proxy writes them after the response is streamed.
async fn usage_rows(state: &ProxyState, n: usize) {
    for _ in 0..100 {
        if state.store.usage_by_lane_model_tier().expect("usage").len() >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no usage rows");
}

fn lane_models(state: &ProxyState) -> Vec<(String, String)> {
    state
        .store
        .usage_by_lane_model_tier()
        .expect("usage")
        .into_iter()
        .map(|r| (r.kind.unwrap_or_default(), r.usage.model))
        .collect()
}

fn routing_on(cfg: &mut Config, base: &str) {
    cfg.proxy.openai_upstream = base.to_string();
    cfg.proxy.upstream = base.to_string();
    cfg.proxy.routing.enabled = true;
    cfg.proxy.routing.bulk = rule(&[BIG], SMALL, "bench/bulk.md");
    cfg.proxy.routing.internal = rule(&["gpt-*"], SMALL, "bench/internal.md");
}

#[tokio::test]
async fn bulk_and_internal_follow_their_rule_and_the_agent_lane_never_does() {
    let (up, seen) = upstream();
    let base = up.base_url();
    let (addr, state, task) = proxy_server("routing-lanes", |cfg| routing_on(cfg, &base)).await;
    send(&addr, Some("bulk"), "s-bulk").await;
    send(&addr, Some("internal"), "s-internal").await;
    send(&addr, None, "s-agent").await;
    send(&addr, Some("embeddings"), "s-other").await;
    usage_rows(&state, 4).await;
    assert_eq!(*seen.lock().unwrap(), [SMALL, SMALL, BIG, BIG]);
    // The ledger names the model that served each call, per lane.
    let mut served = lane_models(&state);
    served.sort();
    assert_eq!(
        served,
        [
            ("api_request".to_string(), BIG.to_string()),
            ("api_request:bulk".to_string(), SMALL.to_string()),
            ("api_request:embeddings".to_string(), BIG.to_string()),
            ("api_request:internal".to_string(), SMALL.to_string()),
        ]
    );
    task.abort();
}

#[tokio::test]
async fn a_routed_call_is_marked_logged_and_recorded_on_the_serving_model() {
    let (up, _seen) = upstream();
    let base = up.base_url();
    let (addr, state, task) = proxy_server("routing-ledger", |cfg| {
        routing_on(cfg, &base);
        cfg.proxy.routing.bulk.provider = "gateway".into();
    })
    .await;
    send(&addr, Some("bulk"), "s-bulk").await;
    usage_rows(&state, 1).await;
    let kinds: Vec<_> = state
        .store
        .list_measurements("proxy")
        .unwrap()
        .into_iter()
        .map(|m| (m.kind, m.before_bytes == m.after_bytes))
        .collect();
    // Same size before and after: the row marks the call, it claims no token saving.
    assert_eq!(kinds, [("model_route".to_string(), true)]);
    let logs = state.store.logs_after(0, 100).unwrap();
    assert!(
        logs.iter()
            .any(|l| l.message == format!("model route on the bulk lane: {BIG} -> {SMALL}")),
        "{:?}",
        logs.iter().map(|l| &l.message).collect::<Vec<_>>()
    );
    let call = &state.store.calls_after(0, 10).unwrap()[0];
    assert_eq!(call.kind, "api_request:bulk");
    task.abort();
}

#[tokio::test]
async fn nothing_is_routed_while_off_or_without_the_gate() {
    for (tag, tune) in [
        (
            "routing-off",
            (|cfg: &mut Config| cfg.proxy.routing.enabled = false) as fn(&mut Config),
        ),
        ("routing-no-gate", |cfg: &mut Config| {
            cfg.proxy.routing.bulk.gate = String::new();
            cfg.proxy.routing.internal.gate = "  ".into();
        }),
        ("routing-no-from", |cfg: &mut Config| {
            cfg.proxy.routing.bulk.from.clear();
            cfg.proxy.routing.internal.from.clear();
        }),
    ] {
        let (up, seen) = upstream();
        let base = up.base_url();
        let (addr, _state, task) = proxy_server(tag, |cfg| {
            routing_on(cfg, &base);
            tune(cfg);
        })
        .await;
        send(&addr, Some("bulk"), "s1").await;
        send(&addr, Some("internal"), "s2").await;
        assert_eq!(*seen.lock().unwrap(), [BIG, BIG], "{tag}");
        task.abort();
    }
}

/// The same fixed workload, once without and once with routing; the lane costs come from the
/// `usage` rows the proxy wrote.
async fn lane_dollars(tag: &str, routed: bool) -> Report {
    let (up, _seen) = upstream();
    let base = up.base_url();
    let (addr, state, task) = proxy_server(tag, |cfg| {
        routing_on(cfg, &base);
        cfg.proxy.routing.enabled = routed;
    })
    .await;
    send(&addr, Some("bulk"), "s-bulk").await;
    send(&addr, Some("internal"), "s-internal").await;
    send(&addr, None, "s-agent").await;
    usage_rows(&state, 3).await;
    let mut report = Report::default();
    attach_costs(&mut report, &state.store, &Config::default().stats.prices).expect("costs");
    task.abort();
    report
}

#[tokio::test]
async fn stats_price_shows_dollars_per_lane_before_and_after() {
    let before = lane_dollars("routing-before", false).await.cost.unwrap();
    let after = lane_dollars("routing-after", true).await.cost.unwrap();
    // 1 MTok in + 0.1 MTok out per call, at the shipped $/MTok rows (gpt-5 vs gpt-5-mini).
    let cost = |model: &str| {
        let p = &Config::default().stats.prices[model];
        p.input + 0.1 * p.output
    };
    let (big, small) = (cost(BIG), cost(SMALL));
    assert!(small < big);
    let round = |v: f64| (v * 1e6).round() / 1e6;
    for lane in ["agent", "bulk", "internal"] {
        assert_eq!(before.lanes[lane].cost, round(big), "{lane} before");
    }
    // Routed lanes drop to the small model's price; the agent lane is where it was.
    assert_eq!(after.lanes["agent"].cost, round(big));
    assert_eq!(after.lanes["bulk"].cost, round(small));
    assert_eq!(after.lanes["internal"].cost, round(small));
    assert!(after.total_cost < before.total_cost);
    assert!(!after.lanes.values().any(|l| l.partial));
}
