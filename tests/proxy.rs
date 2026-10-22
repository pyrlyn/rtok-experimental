// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T5.0: httpmock upstream harness for Anthropic and OpenAI wires.
//!
//! Cargo compiles this as `tests/proxy.rs` (not `tests/proxy/mod.rs`) so
//! `cargo test proxy_mock` is a real test filter. Point rtok at
//! [`MockUpstream::base_url`] via `proxy.upstream` / `proxy.openai_upstream`.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use httpmock::Mock;
use httpmock::prelude::*;

const ANTHROPIC_MESSAGES_BODY: &[u8] =
    include_bytes!("fixtures/proxy/anthropic_messages_body.json");
const ANTHROPIC_MESSAGES_STREAM: &[u8] =
    include_bytes!("fixtures/proxy/anthropic_messages_stream.json");
const OPENAI_CHAT_BODY: &[u8] = include_bytes!("fixtures/proxy/openai_chat_body.json");
const OPENAI_CHAT_STREAM: &[u8] = include_bytes!("fixtures/proxy/openai_chat_stream.json");
const OPENAI_RESPONSES_BODY: &[u8] = include_bytes!("fixtures/proxy/openai_responses_body.json");
const OPENAI_RESPONSES_STREAM: &[u8] =
    include_bytes!("fixtures/proxy/openai_responses_stream.json");
const GEMINI_GENERATE_BODY: &[u8] = include_bytes!("fixtures/proxy/gemini_generate_body.json");
const GEMINI_GENERATE_STREAM: &[u8] = include_bytes!("fixtures/proxy/gemini_generate_stream.json");

/// One mock route serving a fixture body. T5.1+ reuse this for passthrough checks.
pub struct MockUpstream {
    pub server: &'static MockServer,
    mock: Mock<'static>,
    fixture: &'static [u8],
}

impl MockUpstream {
    pub fn anthropic_messages_body() -> Self {
        Self::mount("/v1/messages", ANTHROPIC_MESSAGES_BODY, "application/json")
    }
    pub fn anthropic_messages_stream() -> Self {
        Self::mount(
            "/v1/messages",
            ANTHROPIC_MESSAGES_STREAM,
            "text/event-stream",
        )
    }
    pub fn openai_chat_body() -> Self {
        Self::mount("/v1/chat/completions", OPENAI_CHAT_BODY, "application/json")
    }
    pub fn openai_chat_stream() -> Self {
        Self::mount(
            "/v1/chat/completions",
            OPENAI_CHAT_STREAM,
            "text/event-stream",
        )
    }
    pub fn openai_responses_body() -> Self {
        Self::mount("/v1/responses", OPENAI_RESPONSES_BODY, "application/json")
    }
    pub fn openai_responses_stream() -> Self {
        Self::mount(
            "/v1/responses",
            OPENAI_RESPONSES_STREAM,
            "text/event-stream",
        )
    }
    pub fn gemini_generate_body() -> Self {
        Self::mount(
            "/v1beta/models/gemini-2.0-flash:generateContent",
            GEMINI_GENERATE_BODY,
            "application/json",
        )
    }
    pub fn gemini_generate_stream() -> Self {
        Self::mount(
            "/v1beta/models/gemini-2.0-flash:streamGenerateContent",
            GEMINI_GENERATE_STREAM,
            "text/event-stream",
        )
    }

    fn mount(path: &str, fixture: &'static [u8], content_type: &str) -> Self {
        let server = Box::leak(Box::new(MockServer::start()));
        let mock = server.mock(|when, then| {
            when.method(POST).path(path);
            then.status(200)
                .header("content-type", content_type)
                .body(fixture);
        });
        Self {
            server,
            mock,
            fixture,
        }
    }

    pub fn base_url(&self) -> String {
        self.server.base_url()
    }

    pub fn assert_passthrough_bytes(&self, got: &[u8]) {
        assert_eq!(got, self.fixture, "response bytes must match the fixture");
    }

    pub fn assert_upstream_called_once(&self) {
        self.mock.assert();
    }

    pub fn assert_upstream_hits(&self, n: usize) {
        self.mock.assert_calls(n);
    }
}

fn post(up: &MockUpstream, path: &str) -> Vec<u8> {
    let host = up.server.host();
    let port = up.server.port();
    let mut stream = TcpStream::connect((host.as_str(), port)).expect("connect mock");
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let body = br#"{"model":"test"}"#;
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(req.as_bytes()).expect("headers");
    stream.write_all(body).expect("body");
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).expect("read");
    let sep = b"\r\n\r\n";
    let at = buf
        .windows(4)
        .position(|w| w == sep)
        .expect("HTTP header end");
    buf[at + 4..].to_vec()
}

fn roundtrip(up: MockUpstream, path: &str) {
    let got = post(&up, path);
    up.assert_passthrough_bytes(&got);
    up.assert_upstream_called_once();
}

#[test]
fn proxy_mock_anthropic_messages_body() {
    roundtrip(MockUpstream::anthropic_messages_body(), "/v1/messages");
}
#[test]
fn proxy_mock_anthropic_messages_stream() {
    roundtrip(MockUpstream::anthropic_messages_stream(), "/v1/messages");
}
#[test]
fn proxy_mock_openai_chat_body() {
    roundtrip(MockUpstream::openai_chat_body(), "/v1/chat/completions");
}
#[test]
fn proxy_mock_openai_chat_stream() {
    roundtrip(MockUpstream::openai_chat_stream(), "/v1/chat/completions");
}
#[test]
fn proxy_mock_openai_responses_body() {
    roundtrip(MockUpstream::openai_responses_body(), "/v1/responses");
}
#[test]
fn proxy_mock_openai_responses_stream() {
    roundtrip(MockUpstream::openai_responses_stream(), "/v1/responses");
}

// ── T5.1: passthrough proxy — identical bytes plus usage/calls/call_io/tokens rows ──

use std::sync::Arc;

use rtok::config::Config;
use rtok::proxy::{ProxyState, app};
use rtok::store::Store;
use rtok::store::UsageRow;

mod common;
use common::proxy::{Server, proxy_server};

const T51_MODEL: &str = "claude-sonnet-4-20250514";
const T51_SESSION: &str = "sess-t51";

fn t51_request() -> Vec<u8> {
    format!(
        r#"{{"model":"{T51_MODEL}","max_tokens":8,"messages":[{{"role":"user","content":"hi"}}],"metadata":{{"user_id":"{T51_SESSION}"}}}}"#
    )
    .into_bytes()
}

async fn t51_server(label: &str, up: &MockUpstream, mode: &str) -> Server {
    proxy_server(&format!("t51-{label}"), |cfg| {
        cfg.proxy.upstream = up.base_url();
        cfg.proxy.mode = mode.to_string();
    })
    .await
}

async fn t51_post(addr: &str, body: Vec<u8>) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("http://{addr}/v1/messages"))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("request through the proxy")
}

/// The recorder writes rows after the body was forwarded, one insert at a time; poll
/// `probe` until it yields, failing after a deadline generous enough for slow CI runners.
async fn eventually<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(v) = probe() {
            return v;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what} never appeared"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn t51_usage_n(state: &Store, session: &str, n: usize) -> Vec<UsageRow> {
    eventually(&format!("usage row for {session}"), || {
        Some(state.usage_rows(session).expect("usage read")).filter(|rows| rows.len() >= n)
    })
    .await
}

/// `finish` writes the provider `tokens` row after `usage`: wait for it, don't race it.
async fn t51_tokens(state: &Store, n: i64) {
    eventually(&format!("{n} tokens rows"), || {
        (state.count_tokens().expect("tokens") >= n).then_some(())
    })
    .await
}

async fn t51_usage(state: &Store, session: &str) -> Vec<UsageRow> {
    t51_usage_n(state, session, 1).await
}

#[tokio::test]
async fn proxy_passthrough_body_records_usage_rows() {
    let up = MockUpstream::anthropic_messages_body();
    let (addr, state, task) = t51_server("body", &up, "passthrough").await;
    let resp = t51_post(&addr, t51_request()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let bytes = resp.bytes().await.expect("response body");
    up.assert_passthrough_bytes(&bytes);
    up.assert_upstream_called_once();

    let rows = t51_usage(&state.store, T51_SESSION).await;
    assert_eq!(rows.len(), 1, "exactly one usage row");
    let u = &rows[0];
    assert_eq!(
        (u.input, u.cache_create, u.cache_read, u.output),
        (10, 0, 0, 2),
        "usage row carries the four provider counters"
    );
    assert_eq!(u.model.as_deref(), Some(T51_MODEL));
    let call_id = u.call_id.expect("usage.call_id points at the calls row") as i32;
    assert_eq!(
        state
            .store
            .model_slug_of_call(call_id)
            .expect("slug")
            .as_deref(),
        Some(T51_MODEL),
        "models.slug must equal the request model"
    );
    assert_eq!(state.store.count_kind("api_request").expect("calls"), 1);
    assert_eq!(state.store.count_call_io().expect("call_io"), 1);
    t51_tokens(&state.store, 1).await;
    assert_eq!(state.store.count_tokens().expect("tokens"), 1);
    task.abort();
}

/// A client that negotiates gzip must not cost us the usage row: this build links reqwest
/// without its decompression features, so the upstream request has to ask for `identity` —
/// and for nothing else: the client's own `gzip, deflate, br` used to be forwarded next to
/// it, and upstream picked gzip.
#[tokio::test]
async fn upstream_request_asks_for_identity_encoding() {
    let server = httpmock::MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/messages")
            .is_true(|req: &httpmock::HttpMockRequest| {
                let values: Vec<String> = req
                    .headers()
                    .get_all("accept-encoding")
                    .iter()
                    .map(|v| v.to_str().unwrap_or("?").to_string())
                    .collect();
                values == ["identity"]
            });
        then.status(200)
            .header("content-type", "application/json")
            .body(MockUpstream::anthropic_messages_body().fixture);
    });
    let dir = std::env::temp_dir().join(format!("rtok-proxy-identity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cfg = Config::load_from(&dir).expect("config");
    cfg.proxy.upstream = server.base_url();
    let state = Arc::new(ProxyState::new(&cfg).expect("proxy state"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr").to_string();
    let task = tokio::spawn(axum::serve(listener, app(state.clone())).into_future());

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/v1/messages"))
        .header("content-type", "application/json")
        .header("accept-encoding", "gzip, deflate, br")
        .body(t51_request())
        .send()
        .await
        .expect("request through the proxy");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    mock.assert();
    let rows = t51_usage_n(&state.store, T51_SESSION, 1).await;
    assert_eq!(
        rows.len(),
        1,
        "usage recorded despite the client's gzip offer"
    );
    task.abort();
}

#[tokio::test]
async fn proxy_passthrough_stream_is_byte_identical_and_records_usage() {
    let up = MockUpstream::anthropic_messages_stream();
    let (addr, state, task) = t51_server("stream", &up, "passthrough").await;
    let resp = t51_post(&addr, t51_request()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|c| c.contains("text/event-stream"))
            .unwrap_or(false),
        "SSE content-type must pass through"
    );
    let bytes = resp.bytes().await.expect("response body");
    up.assert_passthrough_bytes(&bytes);
    up.assert_upstream_called_once();

    let rows = t51_usage(&state.store, T51_SESSION).await;
    assert_eq!(rows.len(), 1, "exactly one usage row");
    let u = &rows[0];
    // The fixture's message_start carries no usage; the final message_delta has output only.
    assert_eq!(
        (u.input, u.cache_create, u.cache_read, u.output),
        (0, 0, 0, 2),
        "usage merged from SSE message_delta"
    );
    assert_eq!(state.store.count_kind("api_request").expect("calls"), 1);
    assert_eq!(state.store.count_call_io().expect("call_io"), 1);
    task.abort();
}

#[tokio::test]
async fn proxy_health_reports_ok_and_mode() {
    let up = MockUpstream::anthropic_messages_body();
    let (addr, _state, task) = t51_server("health", &up, "passthrough").await;
    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .expect("health");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body = resp.text().await.expect("health body");
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v,
        serde_json::json!({"ok": true, "mode": "passthrough", "enabled": true, "recording": true})
    );
    task.abort();
}

// ── T205: request bookkeeping must not block the tokio runtime ──

/// ~48 MB of space-separated words: representative of the tokenizer work `compress`
/// does over a live blob (unlike one giant unbroken string, which parses/scans faster).
/// Bigger than the card's "~20 MB" so the pre-fix synchronous section dominates the
/// window measured below, instead of being lost next to the upload and the forward.
fn t205_big_text() -> String {
    let word = "token ";
    word.repeat(48 * 1024 * 1024 / word.len())
}

/// Before T205, `handle` ran the request's JSON parse, tokenizer estimates and store
/// writes synchronously on the tokio worker. A single large `compress`-mode request
/// (parsed + walked by the `compress` plugin's `proxy_filter`) pinned that worker for
/// the whole synchronous section, so a concurrent `/health` request had to wait behind
/// it. With the bookkeeping moved to `spawn_blocking`, `/health` stays fast throughout.
///
/// "Fast" is relative to the same run, not a wall-clock bound (T421): a fixed 250 ms
/// flaked at 302 ms on a loaded macOS runner. The pre-fix stall makes one poll wait out
/// the whole shaping, over 90% of the time the big request takes to reach upstream; a
/// healthy proxy keeps every poll to a few percent of it, under 40% even with the host
/// CPUs oversubscribed eight times. Load stretches both sides, so a two-thirds bound on
/// the ratio keeps the margin that a fixed bound loses.
#[tokio::test]
async fn health_stays_fast_while_a_large_request_is_recorded() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/v1/messages");
        then.status(200)
            .header("content-type", "application/json")
            // Keeps the big request in flight long enough to poll `/health` several
            // times against it, on the mock-upstream side rather than a sleep in the
            // proxy itself.
            .delay(Duration::from_millis(300))
            .body(r#"{"type":"message","usage":{"input_tokens":1,"output_tokens":1}}"#);
    });

    let (addr, _state, task) = proxy_server("t205-health", |cfg| {
        cfg.proxy.upstream = server.base_url();
        cfg.proxy.mode = "compress".to_string();
        // This latency fixture deliberately exceeds the normal 8 MiB request limit.
        cfg.proxy.max_body_bytes = (64 * 1024 * 1024).try_into().unwrap();
        cfg.plugins.compress.enabled = true;
    })
    .await;

    let body = serde_json::json!({
        "model": T51_MODEL,
        "max_tokens": 8,
        "messages": [{"role": "user", "content": [{"type": "text", "text": t205_big_text()}]}],
        "metadata": {"user_id": "sess-t205-health"},
    });
    let body_bytes = serde_json::to_vec(&body).expect("request json");

    let client = reqwest::Client::new();
    let big_addr = addr.clone();
    let window_start = std::time::Instant::now();
    let big_req = tokio::spawn(async move { t51_post(&big_addr, body_bytes).await });

    let mut checks = 0;
    let mut slowest = Duration::ZERO;
    loop {
        let t0 = std::time::Instant::now();
        let health = client
            .get(format!("http://{addr}/health"))
            .send()
            .await
            .expect("health");
        slowest = slowest.max(t0.elapsed());
        assert_eq!(health.status(), reqwest::StatusCode::OK);
        checks += 1;
        // Stop once upstream has the request: the pre-fix stall is the shaping before the
        // forward. Dropping the 48 MB buffers after the reply costs ~100 ms on its own and
        // would stretch the window without saying anything about T205.
        if big_req.is_finished() || mock.calls_async().await > 0 {
            break;
        }
    }
    let window = window_start.elapsed();
    assert!(
        checks >= 2,
        "the big request must still overlap a /health poll"
    );
    assert!(
        slowest < window * 2 / 3,
        "/health took {slowest:?} of the {window:?} the large request needed to reach \
         upstream ({checks} polls): it waited on the recording"
    );

    let big_resp = big_req.await.expect("big request task");
    assert_eq!(big_resp.status(), reqwest::StatusCode::OK);
    mock.assert_calls(1);
    task.abort();
}

// ── T11.2: OpenAI Chat Completions wire — own upstream, usage, stream_options ──

const T112_SESSION: &str = "sess-t112";
const T112_MODEL: &str = "gpt-4o";

/// Points `proxy.openai_upstream` (not `proxy.upstream`) at the mock, so a request that
/// reaches the fixture proves the OpenAI wire picked the OpenAI upstream.
async fn openai_server(label: &str, up: &MockUpstream, mode: &str) -> Server {
    proxy_server(&format!("t11-{label}"), |cfg| {
        cfg.proxy.upstream = "http://127.0.0.1:1".to_string(); // Anthropic upstream must go unused
        cfg.proxy.openai_upstream = up.base_url();
        cfg.proxy.mode = mode.to_string();
    })
    .await
}

async fn openai_post(addr: &str, path: &str, body: Vec<u8>) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("http://{addr}{path}"))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("request through the proxy")
}

async fn t112_post(addr: &str, body: serde_json::Value) -> reqwest::Response {
    openai_post(
        addr,
        "/v1/chat/completions",
        serde_json::to_vec(&body).expect("request json"),
    )
    .await
}

#[tokio::test]
async fn proxy_openai_chat_body_records_usage_with_cached_tokens() {
    let up = MockUpstream::openai_chat_body();
    let (addr, state, task) = openai_server("chat-body", &up, "passthrough").await;
    let resp = t112_post(
        &addr,
        serde_json::json!({"model": T112_MODEL, "user": T112_SESSION,
                           "messages":[{"role":"user","content":"hi"}]}),
    )
    .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    up.assert_passthrough_bytes(&resp.bytes().await.expect("response body"));
    up.assert_upstream_called_once();

    let rows = t51_usage(&state.store, T112_SESSION).await;
    assert_eq!(rows.len(), 1, "exactly one usage row");
    let u = &rows[0];
    assert_eq!(
        (u.input, u.cache_create, u.cache_read, u.output),
        (3, 0, 7, 2),
        "cache_read comes from prompt_tokens_details.cached_tokens; no cache_create on this wire"
    );
    assert_eq!(u.model.as_deref(), Some(T112_MODEL));
    task.abort();
}

#[tokio::test]
async fn proxy_openai_chat_stream_is_byte_identical_and_adds_include_usage() {
    let up = MockUpstream::openai_chat_stream();
    let (addr, state, task) = openai_server("chat-stream", &up, "passthrough").await;
    let resp = t112_post(
        &addr,
        serde_json::json!({"model": T112_MODEL, "user": T112_SESSION, "stream": true,
                           "messages":[{"role":"user","content":"hi"}]}),
    )
    .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    up.assert_passthrough_bytes(&resp.bytes().await.expect("response body"));

    let rows = t51_usage(&state.store, T112_SESSION).await;
    let u = &rows[0];
    assert_eq!(
        (u.input, u.cache_create, u.cache_read, u.output),
        (3, 0, 7, 2),
        "usage decoded from the final SSE chunk"
    );
    // The one byte-level change passthrough makes: the request now opts into stream usage.
    let sent = state
        .store
        .call_io_request(u.call_id.expect("call id") as i32)
        .expect("call_io")
        .expect("request");
    let sent: serde_json::Value = serde_json::from_slice(&sent).expect("json");
    assert_eq!(
        sent["stream_options"]["include_usage"],
        serde_json::json!(true)
    );
    assert_eq!(
        sent["messages"][0]["content"], "hi",
        "nothing else rewritten"
    );
    task.abort();
}

// ── T11.3: OpenAI Responses wire — own upstream, body/SSE usage, server-side history ──

const T113_SESSION: &str = "sess-t113";
const T113_MODEL: &str = "gpt-4.1";
const T113_COLLIDING_CALL: &str = "call-shared-across-wires";
const T113_NEW_CALL: &str = "call-responses-only";

fn t113_previous_response_request() -> Vec<u8> {
    let output = (1..=400)
        .map(|i| format!("old line {i}: some tool output with words"))
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::to_vec(&serde_json::json!({
        "model": T113_MODEL,
        "user": T113_SESSION,
        "previous_response_id": "resp_previous",
        "input": [
            {"type": "function_call_output", "call_id": "call-old", "output": output},
            {"role": "user", "content": "one"},
            {"role": "user", "content": "two"},
            {"role": "user", "content": "three"}
        ]
    }))
    .expect("request json")
}

fn t113_regular_noncanonical_request() -> Vec<u8> {
    let output = (1..=400)
        .map(|i| format!("old line {i}: some tool output with words"))
        .collect::<Vec<_>>()
        .join("\n");
    let output = serde_json::to_string(&output).expect("output json");
    format!(
        r#"{{  "user" : "{T113_SESSION}", "input" : [
          {{ "type" : "function_call_output", "call_id" : "{T113_COLLIDING_CALL}", "output" : {output} }},
          {{ "type" : "function_call_output", "call_id" : "{T113_NEW_CALL}", "output" : {output} }},
          {{ "role" : "user", "content" : "one" }},
          {{ "role" : "user", "content" : "two" }},
          {{ "role" : "user", "content" : "three" }}
        ], "model" : "{T113_MODEL}" }}"#
    )
    .into_bytes()
}

#[tokio::test]
async fn proxy_openai_responses_body_records_usage_without_rewriting_previous_response() {
    let up = MockUpstream::openai_responses_body();
    let (addr, state, task) = openai_server("responses-body", &up, "compress").await;
    let request = t113_previous_response_request();
    let resp = openai_post(&addr, "/v1/responses", request.clone()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    up.assert_passthrough_bytes(&resp.bytes().await.expect("response body"));
    up.assert_upstream_called_once();

    let rows = t51_usage(&state.store, T113_SESSION).await;
    let u = &rows[0];
    assert_eq!(
        (u.input, u.cache_create, u.cache_read, u.output),
        (3, 0, 7, 2),
        "cache_read comes from input_tokens_details.cached_tokens; no cache_create on this wire"
    );
    assert_eq!(u.model.as_deref(), Some(T113_MODEL));
    let sent = state
        .store
        .call_io_request(u.call_id.expect("call id") as i32)
        .expect("call_io")
        .expect("request");
    assert_eq!(
        sent, request,
        "previous_response_id must produce zero rewrites"
    );
    assert_eq!(
        state
            .store
            .measurement_count("archive")
            .expect("measurements"),
        0
    );
    task.abort();
}

#[tokio::test]
async fn proxy_openai_responses_compress_is_byte_exact_and_ignores_archive_decisions() {
    let up = MockUpstream::openai_responses_body();
    let (addr, state, task) = openai_server("responses-compress", &up, "compress").await;
    let archive_dir =
        std::env::temp_dir().join(format!("rtok-proxy-t113-collision-{}", std::process::id()));
    let archive_id = state
        .store
        .put_archive("other-session", b"unrelated output", &archive_dir)
        .expect("seed archive");
    let collision_pointer = "[archived unrelated]";
    state
        .store
        .put_archive_decision(
            T113_COLLIDING_CALL,
            &archive_id,
            "other-session",
            collision_pointer,
        )
        .expect("seed colliding archive decision");

    let request = t113_regular_noncanonical_request();
    let resp = openai_post(&addr, "/v1/responses", request.clone()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    up.assert_passthrough_bytes(&resp.bytes().await.expect("response body"));

    let rows = t51_usage(&state.store, T113_SESSION).await;
    let sent = state
        .store
        .call_io_request(rows[0].call_id.expect("call id") as i32)
        .expect("call_io")
        .expect("request");
    assert_eq!(
        sent, request,
        "T11.3 must preserve noncanonical Responses JSON byte-for-byte"
    );
    assert_eq!(
        state
            .store
            .archive_decision("other-session", T113_COLLIDING_CALL)
            .expect("colliding decision")
            .expect("seeded decision")
            .pointer,
        collision_pointer,
        "the seeded decision still belongs to its own session"
    );
    assert!(
        state
            .store
            .archive_decision(T113_SESSION, T113_COLLIDING_CALL)
            .expect("lookup")
            .is_none(),
        "a decision from another API/session must not be visible here"
    );
    assert!(
        state
            .store
            .archive_decision(T113_SESSION, T113_NEW_CALL)
            .expect("new decision")
            .is_none(),
        "T11.3 must not create Responses archive decisions"
    );
    assert_eq!(
        state
            .store
            .measurement_count("archive")
            .expect("measurements"),
        0
    );
    task.abort();
}

#[tokio::test]
async fn proxy_openai_responses_stream_is_byte_identical_and_records_usage() {
    let up = MockUpstream::openai_responses_stream();
    let (addr, state, task) = openai_server("responses-stream", &up, "passthrough").await;
    let request = serde_json::to_vec(&serde_json::json!({
        "model": T113_MODEL, "user": T113_SESSION, "stream": true, "input": "hi"
    }))
    .expect("request json");
    let resp = openai_post(&addr, "/v1/responses", request.clone()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    up.assert_passthrough_bytes(&resp.bytes().await.expect("response body"));
    up.assert_upstream_called_once();

    let rows = t51_usage(&state.store, T113_SESSION).await;
    let u = &rows[0];
    assert_eq!(
        (u.input, u.cache_create, u.cache_read, u.output),
        (3, 0, 7, 2),
        "usage decoded from response.completed, cache_read from cached_tokens"
    );
    let sent = state
        .store
        .call_io_request(u.call_id.expect("call id") as i32)
        .expect("call_io")
        .expect("request");
    assert_eq!(
        sent, request,
        "Responses requests need no passthrough shaping"
    );
    task.abort();
}

// ── T51.3: Gemini wire — own upstream, usage with cached tokens, SSE passthrough ──

const T513_SESSION: &str = "sess-t513";
const T513_MODEL: &str = "gemini-2.0-flash";
const T513_GENERATE: &str = "/v1beta/models/gemini-2.0-flash:generateContent";
const T513_STREAM: &str = "/v1beta/models/gemini-2.0-flash:streamGenerateContent";

/// Points `proxy.gemini_upstream` (not `proxy.upstream`) at the mock, so a request
/// that reaches the fixture proves the Gemini wire picked the Gemini upstream.
/// Gemini carries no session or model in the body: the session rides the
/// `x-rtok-session` header, the model comes from the path.
async fn gemini_server(
    label: &str,
    up: &MockUpstream,
    mode: &str,
) -> (
    String,
    Arc<ProxyState>,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let dir = std::env::temp_dir().join(format!("rtok-proxy-t513-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cfg = Config::load_from(&dir).expect("config");
    cfg.proxy.upstream = "http://127.0.0.1:1".to_string(); // Anthropic upstream must go unused
    cfg.proxy.gemini_upstream = up.base_url();
    cfg.proxy.mode = mode.to_string();
    let state = Arc::new(ProxyState::new(&cfg).expect("proxy state"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr").to_string();
    let task = tokio::spawn(axum::serve(listener, app(state.clone())).into_future());
    (addr, state, task)
}

async fn gemini_post(addr: &str, path: &str, body: serde_json::Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("http://{addr}{path}"))
        .header("content-type", "application/json")
        .header("x-rtok-session", T513_SESSION)
        .body(serde_json::to_vec(&body).expect("request json"))
        .send()
        .await
        .expect("request through the proxy")
}

#[tokio::test]
async fn proxy_gemini_body_records_usage_with_cached_tokens_and_path_model() {
    let up = MockUpstream::gemini_generate_body();
    let (addr, state, task) = gemini_server("generate-body", &up, "passthrough").await;
    let resp = gemini_post(
        &addr,
        T513_GENERATE,
        serde_json::json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]}),
    )
    .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    up.assert_passthrough_bytes(&resp.bytes().await.expect("response body"));
    up.assert_upstream_called_once();

    let rows = t51_usage(&state.store, T513_SESSION).await;
    assert_eq!(rows.len(), 1, "exactly one usage row");
    let u = &rows[0];
    assert_eq!(
        (u.input, u.cache_create, u.cache_read, u.output),
        (5, 0, 7, 3),
        "cache_read comes from cachedContentTokenCount; no cache_create on this wire"
    );
    assert_eq!(u.model.as_deref(), Some(T513_MODEL));
    assert_eq!(u.api, "gemini");
    assert_eq!(state.store.count_kind("api_request").expect("calls"), 1);
    assert_eq!(state.store.count_call_io().expect("call_io"), 1);
    t51_tokens(&state.store, 1).await;
    assert_eq!(state.store.count_tokens().expect("tokens"), 1);
    task.abort();
}

#[tokio::test]
async fn proxy_gemini_stream_is_byte_identical_and_records_usage() {
    let up = MockUpstream::gemini_generate_stream();
    let (addr, state, task) = gemini_server("generate-stream", &up, "passthrough").await;
    let resp = gemini_post(
        &addr,
        T513_STREAM,
        serde_json::json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]}),
    )
    .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|c| c.contains("text/event-stream"))
            .unwrap_or(false),
        "SSE content-type must pass through"
    );
    up.assert_passthrough_bytes(&resp.bytes().await.expect("response body"));
    up.assert_upstream_called_once();

    let rows = t51_usage(&state.store, T513_SESSION).await;
    assert_eq!(rows.len(), 1, "exactly one usage row");
    let u = &rows[0];
    assert_eq!(
        (u.input, u.cache_create, u.cache_read, u.output),
        (5, 0, 7, 3),
        "usage decoded from the final SSE chunk"
    );
    assert_eq!(u.model.as_deref(), Some(T513_MODEL));
    task.abort();
}

const T53_SESSION: &str = "sess-t53";
/// Six user turns, each carrying one 400-line tool_result (well above `archive.min_tokens`).
fn t53_request() -> Vec<u8> {
    let mut messages = Vec::new();
    for t in 1..=6 {
        let text = (1..=400)
            .map(|i| format!("t{t} line {i}: some shell output with words"))
            .collect::<Vec<_>>()
            .join("\n");
        messages.push(serde_json::json!({"role":"user","content":[
            {"type":"tool_result","tool_use_id":format!("tu-{t}"),"content":text}]}));
        messages.push(serde_json::json!({"role":"assistant","content":[
            {"type":"tool_use","id":format!("tu-{}", t + 1),"name":"Bash","input":{}}]}));
    }
    serde_json::to_vec(&serde_json::json!({
        "model": T51_MODEL, "max_tokens": 8, "system": "sys", "tools": [{"name": "Bash"}],
        "messages": messages, "metadata": {"user_id": T53_SESSION}
    }))
    .expect("request json")
}

#[tokio::test]
async fn proxy_compress_rewrites_old_tool_results_identically() {
    let up = MockUpstream::anthropic_messages_body();
    let (addr, state, task) = t51_server("compress", &up, "compress").await;
    for _ in 0..2 {
        let resp = t51_post(&addr, t53_request()).await;
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        up.assert_passthrough_bytes(&resp.bytes().await.expect("body"));
    }
    let rows = t51_usage_n(&state.store, T53_SESSION, 2).await;
    let sent: Vec<String> = rows
        .iter()
        .map(|u| {
            let id = u.call_id.expect("call id") as i32;
            let bytes = state
                .store
                .call_io_request(id)
                .expect("call_io")
                .expect("request");
            String::from_utf8(bytes).expect("utf-8")
        })
        .collect();
    assert_eq!(
        sent[0], sent[1],
        "same request twice → byte-identical upstream bodies"
    );
    let body: serde_json::Value = serde_json::from_str(&sent[0]).expect("json");
    let contents: Vec<&str> = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "user")
        .map(|m| m["content"][0]["content"].as_str().unwrap())
        .collect();
    assert_eq!(contents.len(), 6);
    assert!(contents[0].starts_with("[archived ") && contents[1].starts_with("[archived "));
    for (i, c) in contents.iter().enumerate().skip(2) {
        assert!(
            c.starts_with(&format!("t{} line 1:", i + 1)),
            "turn {} untouched",
            i + 1
        );
        assert!(!c.contains("[archived"));
    }
    assert_eq!(body["system"], "sys");
    assert_eq!(body["tools"][0]["name"], "Bash");
    assert_eq!(state.store.count_kind("api_request").expect("calls"), 2);
    assert_eq!(
        state.store.count_kind("plugin_run").expect("plugin runs"),
        2
    );
    assert_eq!(
        state
            .store
            .measurement_count("archive")
            .expect("measurements"),
        4
    );
    task.abort();
}

/// T55.11: `rtok expand <id>` runs under its own session, not the proxy session that
/// wrote the decision — the freeze is keyed by archive id, so the next forwarded request
/// carries the original block for the expanded id while other pointers stay.
#[tokio::test]
async fn proxy_expand_freezes_the_pointer_for_the_next_request() {
    let up = MockUpstream::anthropic_messages_body();
    let label = "expand-freeze";
    let (addr, state, task) = t51_server(label, &up, "compress").await;
    // `usage_rows` is newest first, so the latest request is always `rows[0]`.
    let sent_request = |n: usize| {
        let state = state.clone();
        async move {
            let rows = t51_usage_n(&state.store, T53_SESSION, n).await;
            let bytes = state
                .store
                .call_io_request(rows[0].call_id.expect("call id") as i32)
                .expect("call_io")
                .expect("request");
            String::from_utf8(bytes).expect("utf-8")
        }
    };
    let resp = t51_post(&addr, t53_request()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let first = sent_request(1).await;
    let contents = tool_result_contents(&first);
    let pointer = contents[0].as_str();
    assert!(pointer.starts_with("[archived "), "{pointer}");
    let id: String = pointer
        .split("expand(")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .expect("expand id")
        .to_string();
    // The CLI expand surface: its own session, the same store.
    let dir = std::env::temp_dir().join(format!("rtok-proxy-t51-{label}-{}", std::process::id()));
    let cfg = Config::load_from(&dir).expect("config");
    let cx = rtok::plugin::Runtime::open(cfg, "expand").expect("expand runtime");
    assert_eq!(
        rtok::expand::fetch(&cx, &id).unwrap().unwrap(),
        contents_original_bytes(1),
        "expand returns the archived original"
    );
    let resp = t51_post(&addr, t53_request()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let second = sent_request(2).await;
    let contents = tool_result_contents(&second);
    assert!(
        contents[0].starts_with("t1 line 1:"),
        "expanded id goes to upstream whole: {}",
        contents[0]
    );
    assert!(
        contents[1].starts_with("[archived "),
        "the un-expanded turn-2 pointer stays"
    );
    task.abort();
}

fn tool_result_contents(body: &str) -> Vec<String> {
    let body: serde_json::Value = serde_json::from_str(body).expect("json");
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "user")
        .map(|m| m["content"][0]["content"].as_str().unwrap().to_string())
        .collect()
}

/// The original turn-`t` payload of [`t53_request`], as archived bytes.
fn contents_original_bytes(t: usize) -> Vec<u8> {
    (1..=400)
        .map(|i| format!("t{t} line {i}: some shell output with words"))
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

const T114_SESSION: &str = "sess-t114";
const ANTHROPIC_6TURNS: &[u8] = include_bytes!("fixtures/proxy/anthropic_messages_6turns.json");
const OPENAI_CHAT_6TURNS: &[u8] = include_bytes!("fixtures/proxy/openai_chat_6turns.json");
const OPENAI_RESPONSES_6TURNS: &[u8] =
    include_bytes!("fixtures/proxy/openai_responses_6turns.json");

struct T114Case {
    label: &'static str,
    path: &'static str,
    fixture: &'static [u8],
    openai: bool,
}

fn t114_cases() -> [T114Case; 3] {
    [
        T114Case {
            label: "anthropic",
            path: "/v1/messages",
            fixture: ANTHROPIC_6TURNS,
            openai: false,
        },
        T114Case {
            label: "chat",
            path: "/v1/chat/completions",
            fixture: OPENAI_CHAT_6TURNS,
            openai: true,
        },
        T114Case {
            label: "responses",
            path: "/v1/responses",
            fixture: OPENAI_RESPONSES_6TURNS,
            openai: true,
        },
    ]
}

fn t114_upstream(path: &str) -> MockUpstream {
    match path {
        "/v1/messages" => MockUpstream::anthropic_messages_body(),
        "/v1/chat/completions" => MockUpstream::openai_chat_body(),
        "/v1/responses" => MockUpstream::openai_responses_body(),
        _ => unreachable!(),
    }
}

fn t114_texts<'a>(path: &str, body: &'a serde_json::Value) -> Vec<&'a str> {
    match path {
        "/v1/messages" => body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "user")
            .map(|m| m["content"][0]["content"].as_str().unwrap())
            .collect(),
        "/v1/chat/completions" => body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "tool")
            .map(|m| m["content"].as_str().unwrap())
            .collect(),
        "/v1/responses" => body["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["type"] == "function_call_output")
            .map(|item| item["output"].as_str().unwrap())
            .collect(),
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn proxy_compress_archives_six_turns_on_each_wire() {
    for case in t114_cases() {
        let up = t114_upstream(case.path);
        let (addr, state, task) = if case.openai {
            openai_server(&format!("t114-{}", case.label), &up, "compress").await
        } else {
            t51_server(&format!("t114-{}", case.label), &up, "compress").await
        };
        let request = serde_json::to_vec(
            &serde_json::from_slice::<serde_json::Value>(case.fixture).expect(case.label),
        )
        .expect(case.label);
        for _ in 0..2 {
            let resp = openai_post(&addr, case.path, request.clone()).await;
            assert_eq!(resp.status(), reqwest::StatusCode::OK, "{}", case.label);
            up.assert_passthrough_bytes(&resp.bytes().await.expect("body"));
        }
        let rows = t51_usage_n(&state.store, T114_SESSION, 2).await;
        let sent: Vec<Vec<u8>> = rows
            .iter()
            .map(|u| {
                state
                    .store
                    .call_io_request(u.call_id.expect("call id") as i32)
                    .expect("call_io")
                    .expect("request")
            })
            .collect();
        assert_eq!(sent[0], sent[1], "{} byte-identical", case.label);
        let pos = sent[0]
            .windows(10)
            .position(|window| window == b"[archived ")
            .expect(case.label);
        assert_eq!(&request[..pos], &sent[0][..pos], "{} prefix", case.label);
        let body: serde_json::Value = serde_json::from_slice(&sent[0]).expect("json");
        let contents = t114_texts(case.path, &body);
        assert_eq!(contents.len(), 6, "{}", case.label);
        assert!(
            contents[0].starts_with("[archived ") && contents[1].starts_with("[archived "),
            "{}",
            case.label
        );
        for (i, c) in contents.iter().enumerate().skip(2) {
            assert!(
                c.starts_with(&format!("t{} line 1:", i + 1)),
                "{} turn {}",
                case.label,
                i + 1
            );
        }
        assert_eq!(
            state
                .store
                .measurement_count("archive")
                .expect("measurements"),
            4,
            "{}",
            case.label
        );
        task.abort();
    }
}

// ── T51.2: Anthropic native context editing — opt-in platform path ──

/// The platform path, armed: the proxy adds `context_management` + the beta header,
/// records which path the request took, and `archive` stands down (no double-shrink).
#[tokio::test]
async fn proxy_anthropic_context_edits_arm_platform_path() {
    let server = MockServer::start();
    // Only answers when the beta header is present: a call through proves forwarding.
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/messages")
            .header("anthropic-beta", "context-management-2025-06-27");
        then.status(200)
            .header("content-type", "application/json")
            .body(ANTHROPIC_MESSAGES_BODY);
    });
    let dir = std::env::temp_dir().join(format!("rtok-proxy-t512-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cfg = Config::load_from(&dir).expect("config");
    cfg.proxy.upstream = server.base_url();
    cfg.proxy.mode = "compress".to_string();
    cfg.proxy.context_management = true;
    let state = Arc::new(ProxyState::new(&cfg).expect("proxy state"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr").to_string();
    let task = tokio::spawn(axum::serve(listener, app(state.clone())).into_future());

    let resp = t51_post(&addr, t53_request()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resp.bytes().await.expect("body").as_ref(),
        ANTHROPIC_MESSAGES_BODY,
        "response bytes match the fixture"
    );
    mock.assert();

    let rows = t51_usage(&state.store, T53_SESSION).await;
    assert_eq!(rows.len(), 1, "exactly one usage row");
    assert_eq!(
        (
            rows[0].input,
            rows[0].cache_create,
            rows[0].cache_read,
            rows[0].output
        ),
        (10, 0, 0, 2)
    );
    let sent = state
        .store
        .call_io_request(rows[0].call_id.expect("call id") as i32)
        .expect("call_io")
        .expect("request");
    let body: serde_json::Value = serde_json::from_slice(&sent).expect("json");
    assert_eq!(
        body["context_management"],
        serde_json::json!({"edits": [{"type": "clear_tool_uses_20250919"}]}),
        "the platform-path field the proxy added"
    );
    assert!(
        !String::from_utf8_lossy(&sent).contains("[archived "),
        "archive stands down while the platform clears"
    );
    assert_eq!(
        state
            .store
            .measurement_count("archive")
            .expect("measurements"),
        0
    );
    let kinds: Vec<String> = state
        .store
        .list_measurements("proxy")
        .expect("measurements")
        .into_iter()
        .map(|r| r.kind)
        .collect();
    assert_eq!(
        kinds,
        ["context_management"],
        "one row naming the path this request took"
    );
    task.abort();
}

// ── proxy/core.enabled=false → plain reverse proxy (listener stays up) ──

// Global live ring is process-wide; parallel tests that clear()/assert it must serialize.
static LIVE_RING_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn plain_server(
    label: &str,
    up: &MockUpstream,
    mode: &str,
    proxy_enabled: bool,
    core_enabled: bool,
) -> (
    String,
    Arc<ProxyState>,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let dir = std::env::temp_dir().join(format!("rtok-proxy-plain-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cfg = Config::load_from(&dir).expect("config");
    cfg.proxy.upstream = up.base_url();
    cfg.proxy.mode = mode.to_string();
    cfg.proxy.enabled = proxy_enabled;
    cfg.core.enabled = core_enabled;
    // Asserts archive pointers; see `proxy_server`.
    cfg.plugins.compress.enabled = false;
    let state = Arc::new(ProxyState::new(&cfg).expect("proxy state"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr").to_string();
    let task = tokio::spawn(axum::serve(listener, app(state.clone())).into_future());
    (addr, state, task)
}

async fn assert_plain_forward(label: &str, proxy_enabled: bool, core_enabled: bool) {
    let _live_guard = LIVE_RING_TEST_LOCK.lock().await;
    rtok::proxy::live::clear();
    let up = MockUpstream::anthropic_messages_body();
    // mode=compress would rewrite if business logic ran; plain must ignore it.
    let (addr, state, task) =
        plain_server(label, &up, "compress", proxy_enabled, core_enabled).await;

    let health = reqwest::Client::new()
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .expect("health");
    assert_eq!(health.status(), reqwest::StatusCode::OK, "{label}");
    let hv: serde_json::Value =
        serde_json::from_str(&health.text().await.expect("health body")).expect("health json");
    assert_eq!(hv["ok"], true, "{label}");
    assert_eq!(hv["mode"], "passthrough", "{label}");
    assert_eq!(hv["enabled"], false, "{label}");
    assert_eq!(hv["recording"], false, "{label}");

    let resp = t51_post(&addr, t53_request()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "{label}");
    up.assert_passthrough_bytes(&resp.bytes().await.expect("body"));
    up.assert_upstream_called_once();

    // Give any accidental recorder task a moment; plain must leave the DB empty.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        state.store.count_kind("api_request").expect("calls"),
        0,
        "{label}: no calls rows"
    );
    assert_eq!(
        state.store.usage_rows(T53_SESSION).expect("usage").len(),
        0,
        "{label}: no usage rows"
    );
    assert_eq!(
        state
            .store
            .measurement_count("archive")
            .expect("measurements"),
        0,
        "{label}: no archive measurements"
    );

    for _ in 0..200 {
        if !rtok::proxy::live::snapshot().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let live = rtok::proxy::live::snapshot();
    assert!(!live.is_empty(), "{label}: live ring must show traffic");
    assert_eq!(live[0].path, "/v1/messages", "{label}");
    assert_eq!(live[0].status, 200, "{label}");
    let mut live_http: Vec<serde_json::Value> = Vec::new();
    for _ in 0..200 {
        let live_body = reqwest::Client::new()
            .get(format!("http://{addr}/live"))
            .send()
            .await
            .expect("live")
            .text()
            .await
            .expect("live body");
        live_http = serde_json::from_str(&live_body).expect("live json");
        if !live_http.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(!live_http.is_empty(), "{label} /live");

    let (mut cfg, _) = rtok::testutil::config(label);
    cfg.proxy.enabled = proxy_enabled;
    cfg.core.enabled = core_enabled;
    let snap = rtok::web::model::Model::new(&cfg, Some(&state.store)).snapshot();
    assert!(
        !snap.usage.alerts.is_empty(),
        "{label}: persistent alert until re-enabled"
    );
    assert!(
        snap.calls.iter().any(|c| c.kind == "live_passthrough"),
        "{label}: Calls page shows live passthrough rows"
    );
    task.abort();
}

#[tokio::test]
async fn proxy_disabled_is_plain_forward_with_no_bookkeeping() {
    assert_plain_forward("proxy-off", false, true).await;
}

#[tokio::test]
async fn core_disabled_is_plain_forward_with_no_bookkeeping() {
    assert_plain_forward("core-off", true, false).await;
}

/// `[plugins.proxy] enabled = false` is documented as the usage-capture switch. Leaving it
/// out of `plain()` made it inert: bodies kept being written with the plugin shown as off.
#[tokio::test]
async fn plugin_switch_off_is_plain_forward() {
    let dir = std::env::temp_dir().join(format!("rtok-proxy-plugin-off-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cfg = Config::load_from(&dir).expect("config");
    cfg.plugins.proxy.enabled = false;
    let state = ProxyState::new(&cfg).expect("proxy state");
    assert!(state.plain(), "the plugin switch must stop recording");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn enabled_compress_still_archives_when_flags_on() {
    let _live_guard = LIVE_RING_TEST_LOCK.lock().await;
    rtok::proxy::live::clear();
    let up = MockUpstream::anthropic_messages_body();
    let (addr, state, task) = plain_server("both-on", &up, "compress", true, true).await;
    let health = reqwest::Client::new()
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .expect("health");
    let hv: serde_json::Value =
        serde_json::from_str(&health.text().await.expect("health body")).expect("health json");
    assert_eq!(hv["ok"], true);
    assert_eq!(hv["mode"], "compress");
    assert_eq!(hv["enabled"], true);
    assert_eq!(hv["recording"], true);

    let resp = t51_post(&addr, t53_request()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    up.assert_passthrough_bytes(&resp.bytes().await.expect("body"));

    let rows = t51_usage(&state.store, T53_SESSION).await;
    assert_eq!(rows.len(), 1);
    let sent = state
        .store
        .call_io_request(rows[0].call_id.expect("call id") as i32)
        .expect("call_io")
        .expect("request");
    let body: serde_json::Value = serde_json::from_slice(&sent).expect("json");
    let contents: Vec<&str> = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "user")
        .map(|m| m["content"][0]["content"].as_str().unwrap())
        .collect();
    assert!(
        contents[0].starts_with("[archived "),
        "compress still rewrites when enabled"
    );
    assert!(
        state
            .store
            .measurement_count("archive")
            .expect("measurements")
            > 0
    );
    let (cfg, _) = rtok::testutil::config("both-on");
    let snap = rtok::web::model::Model::new(&cfg, Some(&state.store)).snapshot();
    assert!(snap.usage.alerts.is_empty(), "no alert when enabled");
    assert!(
        snap.calls.iter().all(|c| c.kind != "live_passthrough"),
        "no live rows when recording"
    );
    task.abort();
}

// ── T45.2: every request owes one usage row — cache hits and upstream errors ──

/// The second identical request is served from the semantic cache: upstream sees one
/// hit, but both requests leave a usage row with the cached body's counters.
#[tokio::test]
async fn proxy_cache_hit_records_usage_and_request_bytes() {
    let up = MockUpstream::anthropic_messages_body();
    let dir = std::env::temp_dir().join(format!("rtok-proxy-t452-hit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cfg = Config::load_from(&dir).expect("config");
    cfg.proxy.upstream = up.base_url();
    cfg.plugins.proxy.semantic_cache.enabled = true;
    let state = Arc::new(ProxyState::new(&cfg).expect("proxy state"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr").to_string();
    let task = tokio::spawn(axum::serve(listener, app(state.clone())).into_future());

    let body = t51_request();
    let resp = t51_post(&addr, body.clone()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    up.assert_passthrough_bytes(&resp.bytes().await.expect("body"));
    // The recorder task caches the first response; wait for its row before re-posting.
    t51_usage(&state.store, T51_SESSION).await;
    let resp = t51_post(&addr, body.clone()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    up.assert_passthrough_bytes(&resp.bytes().await.expect("body"));
    up.assert_upstream_called_once();

    let rows = t51_usage_n(&state.store, T51_SESSION, 2).await;
    assert_eq!(rows.len(), 2, "miss and hit each leave one usage row");
    for u in &rows {
        assert_eq!(
            (u.input, u.cache_create, u.cache_read, u.output),
            (10, 0, 0, 2),
            "hit usage comes from the cached body's counters"
        );
        assert_eq!(u.model.as_deref(), Some(T51_MODEL));
        u.call_id.expect("usage.call_id points at the calls row");
    }
    // Newest first: rows[0] is the cache hit.
    let hit_req = state
        .store
        .call_io_request(rows[0].call_id.expect("call id") as i32)
        .expect("call_io")
        .expect("request");
    assert_eq!(hit_req, body, "hit call_io keeps the request bytes");
    t51_tokens(&state.store, 2).await;
    assert_eq!(state.store.count_tokens().expect("tokens"), 2);
    assert_eq!(
        state
            .store
            .measurement_count("proxy")
            .expect("measurements"),
        1
    );
    // T653: the hit keeps its usage row above, yet only the upstream request is billed.
    let billed = state.store.otel_token_totals().expect("token totals");
    assert_eq!(
        billed
            .iter()
            .map(|t| (t.input, t.output))
            .collect::<Vec<_>>(),
        vec![(10, 2)],
        "cache hit must not count as provider-billed"
    );
    task.abort();
}

/// A proxy with the semantic cache on, pointed at `upstream`.
async fn t323_server(label: &str, upstream: String) -> Server {
    proxy_server(&format!("t323-{label}"), |cfg| {
        cfg.proxy.upstream = upstream;
        cfg.plugins.proxy.semantic_cache.enabled = true;
    })
    .await
}

/// T323: an upstream that promises a long JSON body, sends a prefix and hangs up. Binds
/// an ephemeral port; the counter is how many requests reached it.
fn t323_truncating_upstream() -> (String, Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fake upstream");
    let url = format!("http://{}", listener.local_addr().expect("local addr"));
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut s) = conn else { break };
            s.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let mut req = Vec::new();
            let mut chunk = [0u8; 4096];
            // Request head, then as many body bytes as its content-length announces.
            while let Ok(n @ 1..) = s.read(&mut chunk) {
                req.extend_from_slice(&chunk[..n]);
                let Some(at) = req.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let head = String::from_utf8_lossy(&req[..at]).to_lowercase();
                let want = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if req.len() >= at + 4 + want {
                    break;
                }
            }
            seen.fetch_add(1, Ordering::SeqCst);
            let prefix = &ANTHROPIC_MESSAGES_BODY[..40];
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                ANTHROPIC_MESSAGES_BODY.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(prefix);
        }
    });
    (url, hits)
}

/// T323: a body cut off mid-stream (upstream hang-up) is not a response — the next
/// identical request must reach upstream again, not be served the cut-off JSON.
#[tokio::test]
async fn proxy_cache_never_stores_a_truncated_body() {
    let (url, hits) = t323_truncating_upstream();
    let (addr, state, task) = t323_server("trunc", url).await;
    let first = t51_post(&addr, t51_request()).await;
    assert_eq!(first.status(), reqwest::StatusCode::OK);
    assert!(first.bytes().await.is_err(), "the client sees the cut");
    t51_usage(&state.store, T51_SESSION).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let second = t51_post(&addr, t51_request()).await;
    let _ = second.bytes().await;
    assert_eq!(
        hits.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the second request must reach upstream, not the cache"
    );
    task.abort();
}

/// T323: the cache key carries the caller (`x-api-key`, `authorization`) and the headers
/// that change the response (`anthropic-version`, `anthropic-beta`): the same body from
/// another identity is a miss, from the same identity a hit.
#[tokio::test]
async fn proxy_cache_is_keyed_by_caller_identity() {
    let up = MockUpstream::anthropic_messages_body();
    let (addr, state, task) = t323_server("caller", up.base_url()).await;
    let post = |headers: &'static [(&'static str, &'static str)]| {
        let addr = addr.clone();
        async move {
            let mut rb = reqwest::Client::new()
                .post(format!("http://{addr}/v1/messages"))
                .header("content-type", "application/json")
                .body(t51_request());
            for (k, v) in headers {
                rb = rb.header(*k, *v);
            }
            rb.send()
                .await
                .expect("request")
                .bytes()
                .await
                .expect("body");
        }
    };
    let cases: [&'static [(&'static str, &'static str)]; 5] = [
        &[("x-api-key", "key-a")],
        &[("x-api-key", "key-b")],
        &[("authorization", "Bearer tok-c")],
        &[
            ("x-api-key", "key-a"),
            ("anthropic-beta", "files-api-2025-04-14"),
        ],
        &[("x-api-key", "key-a"), ("anthropic-version", "2023-06-01")],
    ];
    for (i, headers) in cases.iter().enumerate() {
        post(headers).await;
        t51_usage_n(&state.store, T51_SESSION, i + 1).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    up.assert_upstream_hits(cases.len());
    // The same identity again is served from the cache.
    post(cases[0]).await;
    up.assert_upstream_hits(cases.len());
    task.abort();
}

/// Upstream connection refused → 502, but the request still leaves its `calls` row
/// and a minimal all-zero usage row (no counters known, so no `tokens` row).
#[tokio::test]
async fn proxy_upstream_error_still_records_usage_row() {
    let dir = std::env::temp_dir().join(format!("rtok-proxy-t452-err-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cfg = Config::load_from(&dir).expect("config");
    cfg.proxy.upstream = "http://127.0.0.1:1".to_string(); // nothing listens: refused
    let state = Arc::new(ProxyState::new(&cfg).expect("proxy state"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr").to_string();
    let task = tokio::spawn(axum::serve(listener, app(state.clone())).into_future());

    let resp = t51_post(&addr, t51_request()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_GATEWAY);
    let rows = t51_usage(&state.store, T51_SESSION).await;
    assert_eq!(rows.len(), 1, "exactly one usage row despite the error");
    let u = &rows[0];
    assert_eq!(
        (u.input, u.cache_create, u.cache_read, u.output),
        (0, 0, 0, 0),
        "no response means no counters"
    );
    u.call_id.expect("usage.call_id points at the calls row");
    assert_eq!(state.store.count_kind("api_request").expect("calls"), 1);
    assert_eq!(
        state.store.count_tokens().expect("tokens"),
        0,
        "no provider counters, no tokens row"
    );
    task.abort();
}

// ── T51.1: live-zone blob shrinking — pointered from turn 2, byte-stable, expandable ──

const T511_SESSION: &str = "sess-t511";

/// A 400-row JSON dump: a blob candidate (parses as JSON, not prose), far above
/// `archive.min_tokens`, and identical bytes every time the same turn is re-sent.
fn t511_blob(turn: usize) -> String {
    let rows: Vec<serde_json::Value> = (1..=400)
        .map(|i| {
            serde_json::json!({"id": i, "turn": turn, "name": format!("entry-{i}"),
                "note": "some payload the model already read"})
        })
        .collect();
    serde_json::to_string(&serde_json::json!({"rows": rows})).expect("blob json")
}

/// Six user turns, each carrying one blob in a non-`tool_result` position: Anthropic
/// `text` blocks, Chat plain string content. No tool results at all, so every `archive`
/// measurement the run writes has to have come from the live-blob pass.
fn t511_request(openai: bool) -> Vec<u8> {
    let mut messages = Vec::new();
    for t in 1..=6 {
        let blob = t511_blob(t);
        messages.push(if openai {
            serde_json::json!({"role": "user", "content": blob})
        } else {
            serde_json::json!({"role": "user", "content": [{"type": "text", "text": blob}]})
        });
        messages.push(serde_json::json!({"role": "assistant", "content": "ok"}));
    }
    let body = if openai {
        serde_json::json!({"model": T112_MODEL, "user": T511_SESSION, "messages": messages})
    } else {
        serde_json::json!({"model": T51_MODEL, "max_tokens": 8, "messages": messages,
            "metadata": {"user_id": T511_SESSION}})
    };
    serde_json::to_vec(&body).expect("request json")
}

fn t511_texts(openai: bool, body: &serde_json::Value) -> Vec<&str> {
    body["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter(|m| m["role"] == "user")
        .map(|m| {
            if openai {
                m["content"].as_str().expect("string content")
            } else {
                m["content"][0]["text"].as_str().expect("text block")
            }
        })
        .collect()
}

async fn t511_server(label: &str, up: &MockUpstream, openai: bool) -> Server {
    let base = up.base_url();
    proxy_server(&format!("t511-{label}"), move |cfg| {
        cfg.proxy.mode = "compress".to_string();
        if openai {
            cfg.proxy.upstream = "http://127.0.0.1:1".to_string();
            cfg.proxy.openai_upstream = base;
        } else {
            cfg.proxy.upstream = base;
        }
        cfg.plugins.archive.live_blobs = true;
    })
    .await
}

/// T51.1 acceptance. With the opt-in flag on, a six-turn request whose user blocks carry a
/// large JSON dump leaves the proxy pointered from turn 2 up, byte-identical on a replay
/// (so the prompt cache still hits), with the two working-edge turns untouched and every
/// archived original recoverable by its measurement's `ref_id` — what `expand <id>` reads.
#[tokio::test]
async fn proxy_compress_shrinks_live_blobs_on_both_wires() {
    for (label, path, openai) in [
        ("anthropic", "/v1/messages", false),
        ("chat", "/v1/chat/completions", true),
    ] {
        let up = if openai {
            MockUpstream::openai_chat_body()
        } else {
            MockUpstream::anthropic_messages_body()
        };
        let (addr, state, task) = t511_server(label, &up, openai).await;
        let request = t511_request(openai);
        for _ in 0..2 {
            let resp = openai_post(&addr, path, request.clone()).await;
            assert_eq!(resp.status(), reqwest::StatusCode::OK, "{label}");
            up.assert_passthrough_bytes(&resp.bytes().await.expect("body"));
        }
        let rows = t51_usage_n(&state.store, T511_SESSION, 2).await;
        let sent: Vec<Vec<u8>> = rows
            .iter()
            .map(|u| {
                state
                    .store
                    .call_io_request(u.call_id.expect("call id") as i32)
                    .expect("call_io")
                    .expect("request")
            })
            .collect();
        assert_eq!(
            sent[0], sent[1],
            "{label}: replay is byte-identical upstream"
        );
        assert!(
            sent[0].len() < request.len() / 2,
            "{label}: the blobs actually left the request"
        );
        let body: serde_json::Value = serde_json::from_slice(&sent[0]).expect("json");
        let texts = t511_texts(openai, &body);
        assert_eq!(texts.len(), 6, "{label}");
        for (i, text) in texts.iter().enumerate() {
            // Oldest user message first, so turn = 5 - i; the pass keeps turns 0 and 1.
            if i < 4 {
                assert!(
                    text.starts_with("[archived "),
                    "{label}: turn {} pointered",
                    5 - i
                );
            } else {
                assert_eq!(*text, t511_blob(i + 1), "{label}: working edge stays whole");
            }
        }
        let ms = state
            .store
            .list_measurements("archive")
            .expect("measurements");
        assert!(
            ms.iter().all(|m| m.kind == "live_blob"),
            "{label}: no tool results in this fixture, so nothing but blobs is measured"
        );
        assert_eq!(
            ms.len(),
            8,
            "{label}: four blobs, measured on both requests"
        );
        let mut ids: Vec<&str> = ms
            .iter()
            .map(|m| m.ref_id.as_deref().expect("ref id"))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let mut recovered: Vec<String> = ids
            .iter()
            .map(|id| {
                let bytes = state
                    .store
                    .get_archive(id, None)
                    .expect("archive read")
                    .expect("payload");
                String::from_utf8(bytes).expect("utf-8")
            })
            .collect();
        recovered.sort();
        let mut originals: Vec<String> = (1usize..=4).map(t511_blob).collect();
        originals.sort();
        assert_eq!(
            recovered, originals,
            "{label}: every pointer expands to its original blob, byte for byte"
        );
        task.abort();
    }
}

const T612_SESSION: &str = "sess-t612";

fn t612_skill_body() -> String {
    let lines: Vec<String> = (1..=400)
        .map(|i| format!("pixel line {i}: widget docs and examples for the skill body"))
        .collect();
    format!(
        "Base directory for this skill: /s/pixel\n\n# Pixel\n{}",
        lines.join("\n")
    )
}

fn t612_request() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "model": T51_MODEL,
        "max_tokens": 8,
        "metadata": {"user_id": T612_SESSION},
        "messages": [
            {"role":"user","content":[
                {"type":"tool_result","tool_use_id":"toolu_skill","content":"Launching skill: pixel"},
                {"type":"text","text": t612_skill_body()}
            ]},
            {"role":"assistant","content":"ok"},
            {"role":"user","content":[{"type":"text","text":"mid"}]},
            {"role":"assistant","content":"ok"},
            {"role":"user","content":[{"type":"text","text":"now"}]}
        ]
    }))
    .expect("request json")
}

/// T61.2: a 3-turn request carrying a skill body archives it on turn
/// `keep_turns + 1` (keep_turns = 2 → oldest of three), byte-identically on
/// replay, `kind = "skill"`, `expand` recovers the original.
#[tokio::test]
async fn proxy_compress_archives_skill_bodies_outside_keep_turns() {
    let up = MockUpstream::anthropic_messages_body();
    let (addr, state, task) = proxy_server("t612-skill", |cfg| {
        cfg.proxy.mode = "compress".to_string();
        cfg.proxy.upstream = up.base_url();
        cfg.plugins.archive.keep_turns = 2;
        // T61.2 ships skill archiving opt-in; this case is what the opt-in buys.
        cfg.plugins.archive.skills = true;
    })
    .await;
    let request = t612_request();
    for _ in 0..2 {
        let resp = t51_post(&addr, request.clone()).await;
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        up.assert_passthrough_bytes(&resp.bytes().await.expect("body"));
    }
    let rows = t51_usage_n(&state.store, T612_SESSION, 2).await;
    let sent: Vec<Vec<u8>> = rows
        .iter()
        .map(|u| {
            state
                .store
                .call_io_request(u.call_id.expect("call id") as i32)
                .expect("call_io")
                .expect("request")
        })
        .collect();
    assert_eq!(sent[0], sent[1], "replay is byte-identical");
    let body: serde_json::Value = serde_json::from_slice(&sent[0]).expect("json");
    let pointer = body["messages"][0]["content"][1]["text"]
        .as_str()
        .expect("skill text");
    assert!(
        pointer.starts_with("[archived ")
            && pointer.contains("skill pixel")
            && pointer.contains("expand("),
        "{pointer:.120}"
    );
    assert_eq!(
        body["messages"][0]["content"][0]["content"].as_str(),
        Some("Launching skill: pixel"),
        "the Launching skill result stays"
    );
    assert_eq!(
        body["messages"][2]["content"][0]["text"].as_str(),
        Some("mid"),
        "turns inside keep_turns stay whole"
    );
    let ms = state
        .store
        .list_measurements("archive")
        .expect("measurements");
    assert!(ms.iter().all(|m| m.kind == "skill"), "{ms:?}");
    assert_eq!(ms.len(), 2, "one measurement per request, not a re-archive");
    assert_eq!(ms[0].ref_id, ms[1].ref_id);
    let id = ms[0].ref_id.as_deref().expect("ref id");
    let recovered = state
        .store
        .get_archive(id, None)
        .expect("read")
        .expect("payload");
    assert_eq!(
        String::from_utf8(recovered).expect("utf-8"),
        t612_skill_body()
    );
    task.abort();
}

// ── T59.5: opt-in tools[] description rewrite ──

const T595_DESC: &str = "Short one. This second sentence is far too long to keep under the cap.";

#[tokio::test]
async fn proxy_tools_rewrite_anthropic_drops_denied_call_still_forwards() {
    let up = MockUpstream::anthropic_messages_body();
    let (addr, state, task) = proxy_server("t595-a", |cfg| {
        cfg.proxy.upstream = up.base_url();
        cfg.proxy.tools_rewrite.enabled = true;
        cfg.proxy.tools_rewrite.max_description_tokens = 5;
        cfg.proxy.tools_rewrite.deny = vec!["drop_me".into()];
    })
    .await;
    let body = serde_json::json!({
        "model": T51_MODEL,
        "max_tokens": 8,
        "tools": [
            {"name": "Bash", "description": T595_DESC, "input_schema": {"type": "object"}},
            {"name": "drop_me", "description": "Gone.", "input_schema": {"type": "object"}},
        ],
        "messages": [
            {"role": "assistant", "content": [{"type": "tool_use", "id": "t1", "name": "drop_me", "input": {}}]},
            {"role": "user", "content": "hi"},
        ],
        "metadata": {"user_id": "sess-t595-a"},
    });
    let resp = t51_post(&addr, serde_json::to_vec(&body).unwrap()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let rows = t51_usage(&state.store, "sess-t595-a").await;
    let sent: serde_json::Value = serde_json::from_slice(
        &state
            .store
            .call_io_request(rows[0].call_id.expect("call") as i32)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(sent["tools"].as_array().unwrap().len(), 1);
    assert_eq!(sent["tools"][0]["name"], "Bash");
    assert_eq!(sent["tools"][0]["description"], "Short one.");
    assert_eq!(
        sent["tools"][0]["input_schema"],
        serde_json::json!({"type": "object"})
    );
    assert_eq!(sent["messages"][0]["content"][0]["name"], "drop_me");
    let kinds: Vec<_> = state
        .store
        .list_measurements("proxy")
        .unwrap()
        .into_iter()
        .map(|m| m.kind)
        .collect();
    assert_eq!(kinds, ["tools_rewrite"]);
    task.abort();
}

/// T686: with `compact_schema` the upstream gets the compact catalog, and the measurement's
/// `ref_id` expands to the full original `tools[]`.
#[tokio::test]
async fn proxy_tools_rewrite_compact_schema_archives_the_original_catalog() {
    let up = MockUpstream::anthropic_messages_body();
    let (addr, state, task) = proxy_server("t686-a", |cfg| {
        cfg.proxy.upstream = up.base_url();
        cfg.proxy.tools_rewrite.enabled = true;
        cfg.proxy.tools_rewrite.compact_schema = true;
    })
    .await;
    let mut body: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/mcp_catalog_github.json")).unwrap();
    let original_tools = body["tools"].clone();
    body["model"] = T51_MODEL.into();
    body["max_tokens"] = 8.into();
    body["metadata"] = serde_json::json!({"user_id": "sess-t686-a"});
    let resp = t51_post(&addr, serde_json::to_vec(&body).unwrap()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let rows = t51_usage(&state.store, "sess-t686-a").await;
    let sent: serde_json::Value = serde_json::from_slice(
        &state
            .store
            .call_io_request(rows[0].call_id.expect("call") as i32)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let (before, after) = (
        original_tools.to_string().len(),
        sent["tools"].to_string().len(),
    );
    assert!(after * 10 < before * 7, "{before} -> {after}");
    let ms = state.store.list_measurements("proxy").unwrap();
    assert_eq!(ms.len(), 1);
    let id = ms[0].ref_id.as_deref().expect("archive id");
    let recovered: serde_json::Value =
        serde_json::from_slice(&state.store.get_archive(id, None).unwrap().expect("payload"))
            .unwrap();
    assert_eq!(recovered, original_tools);
    task.abort();
}

#[tokio::test]
async fn proxy_tools_rewrite_openai_chat_truncates_function_description() {
    let up = MockUpstream::openai_chat_body();
    let (addr, state, task) = proxy_server("t595-o", |cfg| {
        cfg.proxy.upstream = "http://127.0.0.1:1".into();
        cfg.proxy.openai_upstream = up.base_url();
        cfg.proxy.tools_rewrite.enabled = true;
        cfg.proxy.tools_rewrite.max_description_tokens = 5;
    })
    .await;
    let params = serde_json::json!({"type": "object"});
    let resp = t112_post(
        &addr,
        serde_json::json!({
            "model": T112_MODEL,
            "user": "sess-t595-o",
            "tools": [{
                "type": "function",
                "function": {"name": "search", "description": T595_DESC, "parameters": params},
            }],
            "messages": [{"role": "user", "content": "hi"}],
        }),
    )
    .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let rows = t51_usage(&state.store, "sess-t595-o").await;
    let sent: serde_json::Value = serde_json::from_slice(
        &state
            .store
            .call_io_request(rows[0].call_id.expect("call") as i32)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(sent["tools"][0]["function"]["description"], "Short one.");
    assert_eq!(sent["tools"][0]["function"]["parameters"], params);
    let kinds: Vec<_> = state
        .store
        .list_measurements("proxy")
        .unwrap()
        .into_iter()
        .map(|m| m.kind)
        .collect();
    assert_eq!(kinds, ["tools_rewrite"]);
    task.abort();
}
