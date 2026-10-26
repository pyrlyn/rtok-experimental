// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T385.4: `[proxy.batch] parse_results` through a real proxy against a mock upstream. A
//! fetched Anthropic or OpenAI results stream becomes one `usage` row per succeeded request,
//! a malformed line is skipped, and the body reaches the client byte for byte either way.

use std::time::Duration;

use httpmock::prelude::*;
use rtok::store::{Store, UsageRow};

use crate::common;
use common::proxy::proxy_server;

const SESSION: &str = "batch-results";

/// Two succeeded Anthropic requests around an errored one, a torn line and plain garbage.
const ANTHROPIC_JSONL: &str = concat!(
    r#"{"custom_id":"a","result":{"type":"succeeded","message":{"model":"claude-sonnet-5-5","usage":{"input_tokens":10,"cache_creation_input_tokens":3,"cache_read_input_tokens":4,"output_tokens":5}}}}"#,
    "\n",
    r#"{"custom_id":"b","result":{"type":"errored","error":{"type":"error","error":{"type":"overloaded_error"}}}}"#,
    "\n",
    r#"{"custom_id":"c","result":{"type":"succeeded","mess"#,
    "\n",
    "not json at all\n",
    r#"{"custom_id":"d","result":{"type":"succeeded","message":{"model":"claude-haiku-5","usage":{"input_tokens":1,"output_tokens":2}}}}"#,
    "\n",
);

/// A Chat Completions line (cached tokens inside `prompt_tokens`), a Responses line, an
/// expired request and a failed one.
const OPENAI_JSONL: &str = concat!(
    r#"{"id":"r1","custom_id":"a","error":null,"response":{"status_code":200,"request_id":"x","body":{"object":"chat.completion","model":"gpt-5","usage":{"prompt_tokens":100,"completion_tokens":7,"total_tokens":107,"prompt_tokens_details":{"cached_tokens":40}}}}}"#,
    "\n",
    r#"{"id":"r2","custom_id":"b","error":null,"response":{"status_code":200,"request_id":"y","body":{"object":"response","model":"gpt-5-mini","usage":{"input_tokens":9,"output_tokens":2}}}}"#,
    "\n",
    r#"{"id":"r3","custom_id":"c","response":null,"error":{"code":"batch_expired","message":"expired"}}"#,
    "\n",
    r#"{"id":"r4","custom_id":"d","error":null,"response":{"status_code":500,"request_id":"z","body":{"error":{}}}}"#,
    "\n",
);

fn serve(server: &MockServer, path: &str, status: u16, body: &str) {
    server.mock(|when, then| {
        when.method(GET).path(path);
        then.status(status)
            .header("content-type", "application/x-jsonl")
            .body(body);
    });
}

async fn fetch(addr: &str, path: &str) -> (reqwest::StatusCode, String) {
    let resp = reqwest::Client::new()
        .get(format!("http://{addr}{path}"))
        .header("x-rtok-session", SESSION)
        .send()
        .await
        .expect("request through the proxy");
    (resp.status(), resp.text().await.expect("body"))
}

/// The rows are written after the body was forwarded: wait for `n` of them, sorted so a case
/// does not depend on insertion order.
async fn usage_rows(store: &Store, n: usize) -> Vec<UsageRow> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let mut rows = store.usage_rows(SESSION).expect("usage rows");
        if rows.len() >= n {
            rows.sort_by_key(|r| (r.api.clone(), r.input));
            return rows;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{n} usage rows never appeared, have {}",
            rows.len()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// For the cases that expect no row: let the post-response bookkeeping finish first.
async fn settle(store: &Store, kind: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while store.count_kind(kind).expect("count") < 1 {
        assert!(tokio::time::Instant::now() < deadline, "no {kind} row");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
}

fn legs(r: &UsageRow) -> (&str, Option<&str>, [i64; 4]) {
    (
        r.api.as_str(),
        r.model.as_deref(),
        [r.input, r.cache_create, r.cache_read, r.output],
    )
}

#[tokio::test]
async fn anthropic_results_become_usage_rows_and_skip_bad_lines() {
    let up = MockServer::start();
    serve(
        &up,
        "/v1/messages/batches/msgbatch_1/results",
        200,
        ANTHROPIC_JSONL,
    );
    let (addr, state, task) = proxy_server("batch-results-anthropic", |cfg| {
        cfg.proxy.upstream = up.base_url();
        cfg.proxy.batch.parse_results = true;
    })
    .await;
    let (status, body) = fetch(&addr, "/v1/messages/batches/msgbatch_1/results").await;
    assert!(status.is_success());
    assert_eq!(
        body, ANTHROPIC_JSONL,
        "the results stream is forwarded as is"
    );

    // The calls row lands after the usage rows, so wait for it instead of reading it at once.
    settle(&state.store, "api_request:batch").await;
    let rows = usage_rows(&state.store, 2).await;
    assert_eq!(
        rows.iter().map(legs).collect::<Vec<_>>(),
        vec![
            ("anthropic", Some("claude-haiku-5"), [1, 0, 0, 2]),
            ("anthropic", Some("claude-sonnet-5-5"), [10, 3, 4, 5]),
        ]
    );
    assert_eq!(state.store.count_kind("api_request:batch").unwrap(), 1);
    task.abort();
}

#[tokio::test]
async fn openai_file_content_with_results_becomes_usage_rows_and_a_batch_call() {
    let up = MockServer::start();
    serve(&up, "/v1/files/file-out/content", 200, OPENAI_JSONL);
    let (addr, state, task) = proxy_server("batch-results-openai", |cfg| {
        cfg.proxy.openai_upstream = up.base_url();
        cfg.proxy.upstream = up.base_url();
        cfg.proxy.batch.parse_results = true;
    })
    .await;
    let (status, body) = fetch(&addr, "/v1/files/file-out/content").await;
    assert!(status.is_success());
    assert_eq!(body, OPENAI_JSONL);

    // The calls row lands after the usage rows, so wait for it instead of reading it at once.
    settle(&state.store, "api_request:batch").await;
    let rows = usage_rows(&state.store, 2).await;
    assert_eq!(
        rows.iter().map(legs).collect::<Vec<_>>(),
        vec![
            // Cached tokens come out of `prompt_tokens`, so the counters stay disjoint.
            ("openai_chat", Some("gpt-5"), [60, 0, 40, 7]),
            ("openai_responses", Some("gpt-5-mini"), [9, 0, 0, 2]),
        ]
    );
    // The download was a Files-lane call until its lines said it was a Batch output.
    assert_eq!(state.store.count_kind("api_request:batch").unwrap(), 1);
    assert_eq!(state.store.count_kind("api_request:files").unwrap(), 0);
    task.abort();
}

#[tokio::test]
async fn an_ordinary_file_download_stays_a_files_call_without_rows() {
    let up = MockServer::start();
    serve(&up, "/v1/files/file-doc/content", 200, "just some text\n");
    let (addr, state, task) = proxy_server("batch-results-plain-file", |cfg| {
        cfg.proxy.upstream = up.base_url();
        cfg.proxy.openai_upstream = up.base_url();
        cfg.proxy.batch.parse_results = true;
    })
    .await;
    fetch(&addr, "/v1/files/file-doc/content").await;
    settle(&state.store, "api_request:files").await;
    assert!(state.store.usage_rows(SESSION).unwrap().is_empty());
    task.abort();
}

#[tokio::test]
async fn results_are_not_read_when_the_flag_is_off_or_the_fetch_failed() {
    let up = MockServer::start();
    serve(&up, "/v1/messages/batches/ok/results", 200, ANTHROPIC_JSONL);
    serve(
        &up,
        "/v1/messages/batches/gone/results",
        404,
        ANTHROPIC_JSONL,
    );
    let (off, off_state, off_task) = proxy_server("batch-results-off", |cfg| {
        cfg.proxy.upstream = up.base_url();
    })
    .await;
    let (_, body) = fetch(&off, "/v1/messages/batches/ok/results").await;
    assert_eq!(body, ANTHROPIC_JSONL);
    settle(&off_state.store, "api_request:batch").await;
    assert!(off_state.store.usage_rows(SESSION).unwrap().is_empty());
    off_task.abort();

    let (on, on_state, on_task) = proxy_server("batch-results-404", |cfg| {
        cfg.proxy.upstream = up.base_url();
        cfg.proxy.batch.parse_results = true;
    })
    .await;
    let (status, _) = fetch(&on, "/v1/messages/batches/gone/results").await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
    settle(&on_state.store, "api_request:batch").await;
    assert!(on_state.store.usage_rows(SESSION).unwrap().is_empty());
    on_task.abort();
}

#[tokio::test]
async fn create_and_poll_calls_are_tagged_but_carry_no_usage() {
    let up = MockServer::start();
    up.mock(|when, then| {
        when.path_includes("/v1/messages/batches");
        then.status(200)
            .body(r#"{"id":"msgbatch_1","processing_status":"in_progress"}"#);
    });
    let (addr, state, task) = proxy_server("batch-results-create", |cfg| {
        cfg.proxy.upstream = up.base_url();
        cfg.proxy.batch.parse_results = true;
    })
    .await;
    let client = reqwest::Client::new();
    for req in [
        client
            .post(format!("http://{addr}/v1/messages/batches"))
            .body(r#"{"requests":[]}"#)
            .header("x-rtok-session", SESSION),
        client
            .get(format!("http://{addr}/v1/messages/batches/msgbatch_1"))
            .header("x-rtok-session", SESSION),
        client
            .get(format!("http://{addr}/v1/messages/batches"))
            .header("x-rtok-session", SESSION),
        client
            .post(format!(
                "http://{addr}/v1/messages/batches/msgbatch_1/cancel"
            ))
            .header("x-rtok-session", SESSION),
    ] {
        assert!(req.send().await.expect("send").status().is_success());
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while state.store.count_kind("api_request:batch").unwrap() < 4 {
        assert!(tokio::time::Instant::now() < deadline, "calls not tagged");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(state.store.usage_rows(SESSION).unwrap().is_empty());
    task.abort();
}

/// Wait until `n` calls of `kind` are recorded, then let the usage writes behind them finish.
async fn settle_calls(store: &Store, kind: &str, n: i64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while store.count_kind(kind).expect("count") < n {
        assert!(
            tokio::time::Instant::now() < deadline,
            "fewer than {n} {kind} rows"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test]
async fn downloading_the_same_results_twice_counts_them_once() {
    let up = MockServer::start();
    serve(
        &up,
        "/v1/messages/batches/msgbatch_1/results",
        200,
        ANTHROPIC_JSONL,
    );
    serve(
        &up,
        "/v1/messages/batches/msgbatch_2/results",
        200,
        ANTHROPIC_JSONL,
    );
    serve(&up, "/v1/files/file_1/content", 200, OPENAI_JSONL);
    let (addr, state, task) = proxy_server("batch-results-replay", |cfg| {
        cfg.proxy.openai_upstream = up.base_url();
        cfg.proxy.upstream = up.base_url();
        cfg.proxy.batch.parse_results = true;
    })
    .await;

    for path in [
        "/v1/messages/batches/msgbatch_1/results",
        "/v1/files/file_1/content",
    ] {
        fetch(&addr, path).await;
        fetch(&addr, path).await;
    }
    settle_calls(&state.store, "api_request:batch", 4).await;
    // Two succeeded Anthropic lines and two usable OpenAI lines, however often they are fetched.
    assert_eq!(usage_rows(&state.store, 4).await.len(), 4);

    // The same bytes under another batch id are another job's results, not a replay.
    fetch(&addr, "/v1/messages/batches/msgbatch_2/results").await;
    settle_calls(&state.store, "api_request:batch", 5).await;
    assert_eq!(usage_rows(&state.store, 6).await.len(), 6);
    task.abort();
}
