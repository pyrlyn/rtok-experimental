// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T385.9.1: the P28 compressor client against a mock upstream — one stored summary per archive
//! id and version, reuse, the must-keep guard, an upstream failure, and the call reaching the
//! `internal` lane of a real proxy. No live network.

use httpmock::prelude::*;
use rtok::config::Config;
use rtok::plugins::compress::llm::{Auth, Compressor, Outcome};
use rtok::store::Store;
use serde_json::json;

use crate::it::common;
use common::compress_fixture::{log, reply, summary_with};
use common::proxy::proxy_server;

struct Fixture {
    store: Store,
    cfg: Config,
    id: String,
    text: String,
}

fn fixture(tag: &str, url: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("rtok-compress-llm-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cfg = Config::load_from(&dir).expect("config");
    cfg.core.archive_dir = dir.join("archive");
    cfg.plugins.compress.llm.enabled = true;
    cfg.plugins.compress.llm.url = url.to_string();
    let store = Store::open_in_memory().expect("store");
    let text = log();
    let id = store
        .put_archive("s", text.as_bytes(), &cfg.core.archive_dir)
        .expect("archive");
    Fixture {
        store,
        cfg,
        id,
        text,
    }
}

fn auth() -> Auth {
    Auth::new(vec![
        ("x-api-key".into(), "test-key".into()),
        ("anthropic-beta".into(), "oauth-test".into()),
    ])
    .expect("a credential")
}

#[test]
fn a_flag_that_is_off_builds_no_client() {
    let mut cfg = Config::default();
    assert!(!cfg.plugins.compress.llm.enabled);
    assert!(Compressor::new(&cfg).expect("new").is_none());
    cfg.plugins.compress.llm.enabled = true;
    assert!(Compressor::new(&cfg).expect("new").is_some());
}

#[tokio::test]
async fn a_summary_is_stored_once_per_archive_id_and_version_and_read_back() {
    let up = MockServer::start();
    let fx = fixture("store", &up.base_url());
    let summary = summary_with(&fx.text, None);
    let mock = up.mock(|when, then| {
        when.method(POST)
            .path("/v1/messages")
            .header("x-rtok-lane", "internal")
            .header("x-api-key", "test-key")
            .header("anthropic-beta", "oauth-test")
            .header("anthropic-version", "2023-06-01")
            .body_includes("posting.rs:142:23")
            .body_includes("claude-haiku-4-5");
        then.status(200).json_body(reply(&summary));
    });
    let c = Compressor::new(&fx.cfg).expect("new").expect("on");

    assert_eq!(c.run(&fx.store, &auth(), &fx.id).await, Outcome::Stored);
    assert_eq!(
        fx.store
            .archive_summary(&fx.id, c.version())
            .expect("read")
            .as_deref(),
        Some(summary.as_str())
    );
    assert_eq!(c.run(&fx.store, &auth(), &fx.id).await, Outcome::Cached);
    mock.assert_calls(1);
}

#[tokio::test]
async fn a_new_version_is_summarised_again() {
    let up = MockServer::start();
    let mut fx = fixture("version", &up.base_url());
    let summary = summary_with(&fx.text, None);
    let mock = up.mock(|when, then| {
        when.method(POST).path("/v1/messages");
        then.status(200).json_body(reply(&summary));
    });
    let c = Compressor::new(&fx.cfg).expect("new").expect("on");
    assert_eq!(c.run(&fx.store, &auth(), &fx.id).await, Outcome::Stored);
    fx.cfg.plugins.compress.llm.model = "other-model".into();
    let c2 = Compressor::new(&fx.cfg).expect("new").expect("on");
    assert_eq!(c2.run(&fx.store, &auth(), &fx.id).await, Outcome::Stored);
    mock.assert_calls(2);
}

#[tokio::test]
async fn a_summary_that_drops_a_must_keep_span_is_rejected_and_not_stored() {
    let up = MockServer::start();
    let fx = fixture("guard", &up.base_url());
    let lossy = summary_with(&fx.text, Some("crates/ledger/src/posting.rs:142:23"));
    let mock = up.mock(|when, then| {
        when.method(POST).path("/v1/messages");
        then.status(200).json_body(reply(&lossy));
    });
    let c = Compressor::new(&fx.cfg).expect("new").expect("on");
    assert_eq!(
        c.run(&fx.store, &auth(), &fx.id).await,
        Outcome::Rejected(vec!["crates/ledger/src/posting.rs:142:23".into()])
    );
    mock.assert_calls(1);
    assert_eq!(fx.store.archive_summary(&fx.id, c.version()).unwrap(), None);
}

#[tokio::test]
async fn an_upstream_error_stores_nothing_and_is_not_fatal() {
    let up = MockServer::start();
    let fx = fixture("error", &up.base_url());
    up.mock(|when, then| {
        when.method(POST).path("/v1/messages");
        then.status(529).body("overloaded");
    });
    let c = Compressor::new(&fx.cfg).expect("new").expect("on");
    assert!(matches!(
        c.run(&fx.store, &auth(), &fx.id).await,
        Outcome::Failed(_)
    ));
    assert_eq!(fx.store.archive_summary(&fx.id, c.version()).unwrap(), None);

    // A reply cut off at max_tokens is a failure too, not a short summary.
    let cut = MockServer::start();
    cut.mock(|when, then| {
        when.method(POST).path("/v1/messages");
        then.status(200).json_body(
            json!({"content": [{"type": "text", "text": "x"}], "stop_reason": "max_tokens"}),
        );
    });
    let mut cfg = fx.cfg.clone();
    cfg.plugins.compress.llm.url = cut.base_url();
    let c = Compressor::new(&cfg).expect("new").expect("on");
    assert!(matches!(
        c.run(&fx.store, &auth(), &fx.id).await,
        Outcome::Failed(_)
    ));
}

#[tokio::test]
async fn results_not_worth_a_call_never_reach_the_model() {
    let up = MockServer::start();
    let mock = up.mock(|when, then| {
        when.method(POST).path("/v1/messages");
        then.status(200).json_body(reply("never"));
    });
    let mut fx = fixture("skip", &up.base_url());
    let diff = format!("diff --git a/x b/x\n@@ -1 +1 @@\n{}", "-a\n+b\n".repeat(80));
    let diff_id = fx
        .store
        .put_archive("s", diff.as_bytes(), &fx.cfg.core.archive_dir)
        .unwrap();
    fx.cfg.plugins.compress.llm.max_input_bytes = 100_000;
    let c = Compressor::new(&fx.cfg).expect("new").expect("on");
    assert_eq!(
        c.run(&fx.store, &auth(), &diff_id).await,
        Outcome::Skipped("diff or source")
    );
    assert_eq!(
        c.run(&fx.store, &auth(), "no-such-archive-id").await,
        Outcome::Skipped("archive payload missing")
    );
    fx.cfg.plugins.compress.llm.max_input_bytes = 50;
    let c = Compressor::new(&fx.cfg).expect("new").expect("on");
    assert_eq!(
        c.run(&fx.store, &auth(), &fx.id).await,
        Outcome::Skipped("too large")
    );
    fx.cfg.plugins.compress.llm.max_input_bytes = 100_000;
    fx.cfg.plugins.compress.llm.max_spans = 2;
    let c = Compressor::new(&fx.cfg).expect("new").expect("on");
    assert_eq!(
        c.run(&fx.store, &auth(), &fx.id).await,
        Outcome::Skipped("too many must-keep spans")
    );
    mock.assert_calls(0);
}

#[tokio::test]
async fn the_call_goes_through_the_proxy_internal_lane() {
    let up = MockServer::start();
    let seen = up.mock(|when, then| {
        when.method(POST)
            .path("/v1/messages")
            // The marker is for rtok: the provider never sees it.
            .is_true(|req| req.headers().get("x-rtok-lane").is_none());
        then.status(200)
            .header("content-type", "application/json")
            .json_body(reply("placeholder"));
    });
    let base = up.base_url();
    let (addr, state, task) = proxy_server("compress-llm-lane", |cfg| {
        cfg.proxy.upstream = base;
    })
    .await;
    let fx = fixture("lane", &format!("http://{addr}"));
    // The proxy's reply is the mock's; the guard then rejects it, which is fine here: the
    // point is that the request was made as an internal-lane call and recorded as one.
    let c = Compressor::new(&fx.cfg).expect("new").expect("on");
    let _ = c.run(&fx.store, &auth(), &fx.id).await;
    seen.assert_calls(1);
    assert_eq!(state.store.count_kind("api_request:internal").unwrap(), 1);
    assert_eq!(state.store.count_kind("api_request").unwrap(), 0);
    task.abort();
}
