// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T385.12.1: `rtok batch` through a real proxy against two mock providers. Each provider's
//! calls reach its own upstream, the JSONL goes out unchanged, and the ledger tags every hop
//! as a Batch-lane call. No real provider is contacted.

use std::time::Duration;

use httpmock::prelude::*;
use rtok::batch::{Api, Provider};

use crate::common;
use common::proxy::proxy_server;

const ANTHROPIC_LINES: &str = concat!(
    r#"{"custom_id":"a","params":{"model":"claude-haiku-4-5","max_tokens":1,"messages":[]}}"#,
    "\n",
    r#"{"custom_id":"b","params":{"model":"claude-haiku-4-5","max_tokens":2,"messages":[]}}"#,
    "\n",
);
const OPENAI_LINES: &str = concat!(
    r#"{"custom_id":"a","method":"POST","url":"/v1/chat/completions","body":{"model":"gpt-5"}}"#,
    "\n",
);
const RESULTS: &str = "{\"custom_id\":\"a\",\"result\":{}}\n";

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-batch-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

async fn calls_of_kind(store: &rtok::store::Store, kind: &str, n: i64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while store.count_kind(kind).unwrap() < n {
        assert!(tokio::time::Instant::now() < deadline, "no {kind} rows");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn an_anthropic_batch_is_created_polled_and_fetched_through_the_proxy() {
    let up = MockServer::start();
    let create = up.mock(|when, then| {
        when.method(POST)
            .path("/v1/messages/batches")
            .header("x-api-key", "k")
            .header("anthropic-version", "2023-06-01")
            .body(format!(
                "{{\"requests\":[{}]}}",
                ANTHROPIC_LINES.trim_end().replace('\n', ",")
            ));
        then.status(200).body(r#"{"id":"msgbatch_1"}"#);
    });
    up.mock(|when, then| {
        when.method(GET).path("/v1/messages/batches/msgbatch_1");
        then.status(200).body(r#"{"processing_status":"ended"}"#);
    });
    up.mock(|when, then| {
        when.method(GET)
            .path("/v1/messages/batches/msgbatch_1/results");
        then.status(200).body(RESULTS);
    });
    let (addr, state, task) = proxy_server("batch-cli-anthropic", |cfg| {
        cfg.proxy.upstream = up.base_url();
    })
    .await;
    let api = Api::new(&format!("http://{addr}"), Provider::Anthropic, "k").unwrap();

    let file = tmp("anthropic.jsonl");
    std::fs::write(&file, ANTHROPIC_LINES).unwrap();
    assert_eq!(api.submit(&file).await.unwrap(), r#"{"id":"msgbatch_1"}"#);
    create.assert();
    assert!(api.status("msgbatch_1").await.unwrap().contains("ended"));

    let out = tmp("anthropic-results.jsonl");
    let _ = std::fs::remove_file(&out);
    assert_eq!(
        api.fetch("msgbatch_1", &out).await.unwrap(),
        RESULTS.len() as u64
    );
    assert_eq!(std::fs::read_to_string(&out).unwrap(), RESULTS);
    // The results file is never overwritten, and an id cannot leave the URL path.
    assert!(api.fetch("msgbatch_1", &out).await.is_err());
    assert!(api.status("../x").await.is_err());

    calls_of_kind(&state.store, "api_request:batch", 3).await;
    task.abort();
}

#[tokio::test]
async fn an_openai_batch_reaches_the_openai_upstream_not_the_anthropic_one() {
    let anthropic = MockServer::start();
    let wrong = anthropic.mock(|when, then| {
        when.any_request();
        then.status(500);
    });
    let openai = MockServer::start();
    let upload = openai.mock(|when, then| {
        when.method(POST)
            .path("/v1/files")
            .header("authorization", "Bearer k")
            .body_includes("name=\"purpose\"\r\n\r\nbatch")
            .body_includes(OPENAI_LINES.trim_end());
        then.status(200).body(r#"{"id":"file-in"}"#);
    });
    let create = openai.mock(|when, then| {
        when.method(POST)
            .path("/v1/batches")
            .json_body(serde_json::json!({
                "input_file_id": "file-in",
                "endpoint": "/v1/chat/completions",
                "completion_window": "24h",
            }));
        then.status(200).body(r#"{"id":"batch_1"}"#);
    });
    let pending = openai.mock(|when, then| {
        when.method(GET).path("/v1/batches/batch_0");
        then.status(200)
            .body(r#"{"status":"in_progress","output_file_id":null}"#);
    });
    openai.mock(|when, then| {
        when.method(GET).path("/v1/batches/batch_1");
        then.status(200)
            .body(r#"{"status":"completed","output_file_id":"file-out"}"#);
    });
    openai.mock(|when, then| {
        when.method(GET).path("/v1/files/file-out/content");
        then.status(200).body(RESULTS);
    });
    let (addr, state, task) = proxy_server("batch-cli-openai", |cfg| {
        cfg.proxy.upstream = anthropic.base_url();
        cfg.proxy.openai_upstream = openai.base_url();
    })
    .await;
    let api = Api::new(&format!("http://{addr}/"), Provider::Openai, "k").unwrap();

    let file = tmp("openai.jsonl");
    std::fs::write(&file, OPENAI_LINES).unwrap();
    assert_eq!(api.submit(&file).await.unwrap(), r#"{"id":"batch_1"}"#);
    upload.assert();
    create.assert();

    let out = tmp("openai-results.jsonl");
    let _ = std::fs::remove_file(&out);
    api.fetch("batch_1", &out).await.unwrap();
    assert_eq!(std::fs::read_to_string(&out).unwrap(), RESULTS);

    // A batch without an output file says where it stands and leaves nothing behind.
    let missing = tmp("openai-missing.jsonl");
    let _ = std::fs::remove_file(&missing);
    let err = api.fetch("batch_0", &missing).await.unwrap_err();
    assert!(err.to_string().contains("in_progress"), "{err}");
    assert!(!missing.exists());
    pending.assert();

    assert_eq!(
        wrong.calls(),
        0,
        "an OpenAI call went to the Anthropic upstream"
    );
    calls_of_kind(&state.store, "api_request:files", 2).await;
    task.abort();
}

#[tokio::test]
async fn a_provider_error_is_reported_with_its_status() {
    let up = MockServer::start();
    up.mock(|when, then| {
        when.method(GET).path("/v1/messages/batches/msgbatch_9");
        then.status(404).body(r#"{"error":"not_found"}"#);
    });
    let (addr, _state, task) = proxy_server("batch-cli-404", |cfg| {
        cfg.proxy.upstream = up.base_url();
    })
    .await;
    let api = Api::new(&format!("http://{addr}"), Provider::Anthropic, "k").unwrap();
    let err = api.status("msgbatch_9").await.unwrap_err().to_string();
    assert!(
        err.starts_with("404 Not Found") && err.contains("not_found"),
        "{err}"
    );
    task.abort();
}
