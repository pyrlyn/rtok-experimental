// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T16.5: the exporter against a mock collector — every row once, never twice, marks only on
//! 2xx, and the CLI does nothing without an endpoint.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use httpmock::prelude::*;
use rtok::otel::export::flush_blocking;
use rtok::plugin::{Measurement, Runtime};

use crate::common;

fn home(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-otel-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn ctx(dir: &Path, endpoint: &str) -> Runtime {
    let mut cfg = rtok::testutil::config_in(dir);
    cfg.otel.endpoint = endpoint.into();
    cfg.otel.headers = "x-key=k".into();
    cfg.otel.flush_secs = 2;
    let cx = Runtime::open(cfg, "s1").unwrap();
    cx.store
        .upsert_session("s1", None, Some("p"), Some("/w"), Some("startup"))
        .unwrap();
    cx
}

/// Three calls (hook, mcp, proxy) with io, one measurement, one log row.
fn seed(cx: &Runtime) {
    let s = &cx.store;
    let hook = s
        .insert_call(
            "s1",
            "hook",
            "hook",
            None,
            None,
            None,
            None,
            Some("PostToolUse"),
        )
        .unwrap();
    let req = br#"{"hook_event_name":"PostToolUse","tool_name":"Read","tool_use_id":"toolu_1","tool_input":{"file_path":"/x"},"tool_response":"body"}"#;
    s.insert_call_io(hook, Some(req), None, 65536, None)
        .unwrap();
    // `dispatch` closes a hook row with its `ms`; an open one is still running.
    s.set_call_ms(hook, 1.5).unwrap();
    let mcp = s
        .insert_call(
            "s1",
            "mcp",
            "tool_call",
            None,
            None,
            None,
            Some("graph"),
            Some("symbol"),
        )
        .unwrap();
    s.insert_call_io(
        mcp,
        Some(br#"{"name":"b"}"#),
        Some(b"chain.rs:4"),
        65536,
        None,
    )
    .unwrap();
    let (pid, mid) = s.upsert_model("anthropic", "claude-x").unwrap();
    let chat = s
        .insert_call(
            "s1",
            "proxy",
            "api_request",
            None,
            Some(pid),
            Some(mid),
            None,
            Some("/v1/messages"),
        )
        .unwrap();
    // `finish` sets `ms` and the usage once the response is through.
    s.set_call_ms(chat, 25.0).unwrap();
    s.insert_usage("s1", Some("claude-x"), "anthropic", 100, 10, 20, 30, chat)
        .unwrap();
    s.insert_measurement(
        "s1",
        &Measurement {
            plugin: "proxy",
            kind: "raw",
            before_bytes: 10,
            after_bytes: 5,
            est_before: 3,
            est_after: 1,
            ref_id: None,
            call_id: Some(chat),
        },
    )
    .unwrap();
    s.insert_log(
        "info",
        "hook",
        "dispatch",
        "hello",
        Some("s1"),
        Some(hook),
        None,
    )
    .unwrap();
}

#[test]
fn every_row_posts_once_and_the_marks_advance() {
    let server = MockServer::start();
    let traces = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/traces")
            .header("content-type", "application/json")
            .header("x-key", "k")
            .body_includes("execute_tool Read")
            .body_includes("execute_tool symbol")
            .body_includes("chat claude-x")
            .body_includes("gen_ai.usage.cache_read.input_tokens")
            .body_includes("rtok.measurement");
        then.status(200).body("{}");
    });
    let logs = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/logs")
            .body_includes(r#""stringValue":"hello""#);
        then.status(200).body("{}");
    });
    // Metrics are whole-table sums: posted on every flush, with no watermark (T16.7).
    let sums = server.mock(|when, then| {
        when.method(POST).path("/v1/metrics");
        then.status(200).body("{}");
    });
    let dir = home("ok");
    let cx = ctx(&dir, &server.base_url());
    seed(&cx);
    let r = flush_blocking(&cx);
    assert_eq!(r.error, None, "{r}");
    assert_eq!((r.enabled, r.spans, r.logs, r.posted), (true, 3, 1, 3));
    traces.assert_calls(1);
    logs.assert_calls(1);
    assert_eq!(cx.store.otel_mark("calls").unwrap(), 3);
    assert_eq!(cx.store.otel_mark("logs").unwrap(), 1);
    let r = flush_blocking(&cx);
    assert_eq!((r.spans, r.logs, r.posted), (0, 0, 1));
    traces.assert_calls(1);
    logs.assert_calls(1);
    sums.assert_calls(2);
    // An ended session ships its root span; a second flush does not repeat it.
    cx.store.end_session("s1", 1_700_000_000).unwrap();
    let root = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/traces")
            .body_includes("invoke_agent");
        then.status(200).body("{}");
    });
    assert_eq!(flush_blocking(&cx).spans, 1);
    assert_eq!(flush_blocking(&cx).spans, 0);
    root.assert_calls(1);
    assert_eq!(cx.store.otel_mark("sessions_seq").unwrap(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A second session ending in the watermark second still ships its root span once.
#[test]
fn session_in_watermark_second_posts_once() {
    let server = MockServer::start();
    let traces = server.mock(|when, then| {
        when.method(POST).path("/v1/traces");
        then.status(200).body("{}");
    });
    let logs = server.mock(|when, then| {
        when.method(POST).path("/v1/logs");
        then.status(200).body("{}");
    });
    let _metrics = server.mock(|when, then| {
        when.method(POST).path("/v1/metrics");
        then.status(200).body("{}");
    });
    let dir = home("tie");
    let cx = ctx(&dir, &server.base_url());
    seed(&cx);
    let r = flush_blocking(&cx);
    assert_eq!(r.error, None, "{r}");
    cx.store.end_session("s1", 1_700_000_000).unwrap();
    assert_eq!(flush_blocking(&cx).spans, 1);
    cx.store
        .upsert_session("s2", None, Some("p"), Some("/w"), Some("startup"))
        .unwrap();
    cx.store.end_session("s2", 1_700_000_000).unwrap();
    assert_eq!(flush_blocking(&cx).spans, 1);
    assert_eq!(flush_blocking(&cx).spans, 0);
    traces.assert_calls(3);
    logs.assert_calls(1);
    assert_eq!(cx.store.otel_mark("sessions_seq").unwrap(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// T592: a session that ends in an already-posted second must ship whatever its id sorts
/// like. Counting posted rows per second by id re-sent `first` and never sent `late` when
/// `late` sorted before it, so each case names the span it expects, not only a span count.
fn late_session_ships_once(tag: &str, first: &str, late: &str) {
    let server = MockServer::start();
    let session = |id: &str| {
        server.mock(|when, then| {
            when.method(POST)
                .path("/v1/traces")
                .body_includes("invoke_agent")
                .body_includes(format!("\"stringValue\":\"{id}\""));
            then.status(200).body("{}");
        })
    };
    let (first_posts, late_posts) = (session(first), session(late));
    for path in ["/v1/traces", "/v1/logs", "/v1/metrics"] {
        server.mock(|when, then| {
            when.method(POST).path(path);
            then.status(200).body("{}");
        });
    }
    let dir = home(tag);
    let cx = ctx(&dir, &server.base_url());
    for id in [first, late] {
        cx.store
            .upsert_session(id, None, Some("p"), Some("/w"), Some("startup"))
            .unwrap();
    }
    cx.store.end_session(first, 1_700_000_000).unwrap();
    let r = flush_blocking(&cx);
    assert_eq!((r.error, r.spans), (None, 1));
    first_posts.assert_calls(1);
    cx.store.end_session(late, 1_700_000_000).unwrap();
    assert_eq!(flush_blocking(&cx).spans, 1);
    late_posts.assert_calls(1);
    first_posts.assert_calls(1);
    assert_eq!(flush_blocking(&cx).spans, 0, "the next flush drains");
    // The mark is stored, so a fresh process picks up where this one stopped.
    drop(cx);
    let cx = ctx(&dir, &server.base_url());
    assert_eq!(flush_blocking(&cx).spans, 0);
    late_posts.assert_calls(1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_late_session_with_a_lower_id_ships_in_the_watermark_second() {
    late_session_ships_once("late-low", "z-first", "a-late");
}

#[test]
fn a_late_session_with_a_higher_id_ships_in_the_watermark_second() {
    late_session_ships_once("late-high", "a-first", "z-late");
}

/// A proxy call whose response is still streaming has its `calls` row but no `ms` or usage
/// yet: the batch stops in front of it — and of everything after it — and the mark stays
/// there, so the finished span ships on the next flush instead of a 0 ms one for good.
#[test]
fn an_in_flight_call_waits_for_its_finish() {
    let server = MockServer::start();
    let traces = server.mock(|when, then| {
        when.method(POST).path("/v1/traces");
        then.status(200).body("{}");
    });
    for path in ["/v1/logs", "/v1/metrics"] {
        server.mock(|when, then| {
            when.method(POST).path(path);
            then.status(200).body("{}");
        });
    }
    let dir = home("inflight");
    let cx = ctx(&dir, &server.base_url());
    seed(&cx);
    let s = &cx.store;
    let (pid, mid) = s.upsert_model("anthropic", "claude-x").unwrap();
    let open = s
        .insert_call(
            "s1",
            "proxy",
            "api_request",
            None,
            Some(pid),
            Some(mid),
            None,
            Some("/v1/messages"),
        )
        .unwrap();
    let later = s
        .insert_call("s1", "hook", "hook", None, None, None, None, Some("Stop"))
        .unwrap();
    s.set_call_ms(later, 2.0).unwrap();
    let r = flush_blocking(&cx);
    assert_eq!(r.error, None, "{r}");
    assert_eq!(
        r.spans, 3,
        "the seeded calls ship; the open one and its followers wait"
    );
    assert_eq!(s.otel_mark("calls").unwrap(), 3);
    s.set_call_ms(open, 40.0).unwrap();
    assert_eq!(flush_blocking(&cx).spans, 2, "finished: both ship");
    assert_eq!(s.otel_mark("calls").unwrap(), i64::from(later));
    assert_eq!(flush_blocking(&cx).spans, 0);
    traces.assert_calls(2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Traces 404 must not block logs and metrics or log an error every flush.
#[test]
fn a_traces_404_still_posts_logs_and_metrics() {
    let server = MockServer::start();
    // No /v1/traces mock: httpmock answers 404.
    let logs = server.mock(|when, then| {
        when.method(POST).path("/v1/logs");
        then.status(200).body("{}");
    });
    let metrics = server.mock(|when, then| {
        when.method(POST).path("/v1/metrics");
        then.status(200).body("{}");
    });
    let dir = home("t404");
    let cx = ctx(&dir, &server.base_url());
    seed(&cx);
    let r = flush_blocking(&cx);
    assert_eq!(r.error, None, "{r}");
    assert_eq!((r.spans, r.logs, r.points, r.posted), (0, 1, 8, 2));
    assert_eq!(r.skipped, ["traces"]);
    assert!(r.to_string().ends_with("not served: traces"), "{r}");
    logs.assert_calls(1);
    metrics.assert_calls(1);
    assert_eq!(cx.store.otel_mark("calls").unwrap(), 0);
    assert_eq!(cx.store.otel_mark("logs").unwrap(), 1);
    assert!(cx.store.last_log("otel").unwrap().is_none(), "no error row");
    flush_blocking(&cx);
    logs.assert_calls(1);
    metrics.assert_calls(2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A 500 on traces must not stall the other streams: logs and metrics still post, only
/// the traces mark stays for the retry, and the error is reported.
#[test]
fn traces_500_still_posts_logs_and_metrics() {
    let server = MockServer::start();
    let traces = server.mock(|when, then| {
        when.method(POST).path("/v1/traces");
        then.status(500).body("nope");
    });
    let logs = server.mock(|when, then| {
        when.method(POST).path("/v1/logs");
        then.status(200).body("{}");
    });
    let metrics = server.mock(|when, then| {
        when.method(POST).path("/v1/metrics");
        then.status(200).body("{}");
    });
    let dir = home("t500");
    let cx = ctx(&dir, &server.base_url());
    seed(&cx);
    let r = flush_blocking(&cx);
    let err = r.error.clone().expect("a 500 is an error");
    assert!(err.contains("/v1/traces: HTTP 500"), "{err}");
    assert_eq!((r.spans, r.logs, r.points, r.posted), (0, 1, 8, 2));
    assert!(r.skipped.is_empty(), "{r:?}");
    traces.assert_calls(1);
    logs.assert_calls(1);
    metrics.assert_calls(1);
    assert_eq!(cx.store.otel_mark("calls").unwrap(), 0);
    assert_eq!(cx.store.otel_mark("logs").unwrap(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failure_keeps_the_marks_and_is_logged() {
    let server = MockServer::start();
    let traces = server.mock(|when, then| {
        when.method(POST).path("/v1/traces");
        then.status(500).body("nope");
    });
    let dir = home("fail");
    let cx = ctx(&dir, &server.base_url());
    seed(&cx);
    let r = flush_blocking(&cx);
    let err = r.error.clone().expect("a 500 is an error");
    assert!(err.contains("/v1/traces: HTTP 500"), "{err}");
    assert_eq!(r.posted, 0);
    traces.assert_calls(1);
    assert_eq!(cx.store.otel_mark("calls").unwrap(), 0);
    assert_eq!(cx.store.otel_mark("logs").unwrap(), 0);
    let last = cx.store.last_log("otel").unwrap().expect("failure logged");
    assert_eq!(
        (last.level.as_str(), last.name.as_str()),
        ("error", "flush")
    );
    let (calls, logs) = cx.store.otel_pending().unwrap();
    assert_eq!((calls, logs), (3, 2));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A backend with no logs or metrics pipeline (Jaeger) answers 404: the stream is skipped,
/// its mark stays, nothing is logged — a logged error would be re-sent by every later flush —
/// and the traces still ship.
#[test]
fn a_404_stream_is_skipped_not_logged() {
    let server = MockServer::start();
    let traces = server.mock(|when, then| {
        when.method(POST).path("/v1/traces");
        then.status(200).body("{}");
    });
    // No /v1/logs or /v1/metrics mock: httpmock answers 404.
    let dir = home("nf");
    let cx = ctx(&dir, &server.base_url());
    seed(&cx);
    let r = flush_blocking(&cx);
    assert_eq!(r.error, None, "{r}");
    assert_eq!((r.spans, r.logs, r.points, r.posted), (3, 0, 0, 1));
    assert_eq!(r.skipped, ["logs", "metrics"]);
    assert!(r.to_string().ends_with("not served: logs, metrics"), "{r}");
    traces.assert_calls(1);
    assert_eq!(cx.store.otel_mark("calls").unwrap(), 3);
    assert_eq!(cx.store.otel_mark("logs").unwrap(), 0);
    assert!(cx.store.last_log("otel").unwrap().is_none(), "no error row");
    flush_blocking(&cx);
    let (calls, logs) = cx.store.otel_pending().unwrap();
    assert_eq!((calls, logs), (0, 1), "pending logs do not grow per flush");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_without_an_endpoint_does_nothing() {
    let dir = home("cli");
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_rtok"))
            .args(args)
            .env("RTOK_HOME", &dir)
            .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
            .env_remove("OTEL_EXPORTER_OTLP_HEADERS")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(run(&["otel", "flush"]).trim(), "otel: no endpoint");
    let status = run(&["otel", "status"]);
    assert!(status.starts_with("endpoint: none"), "{status}");
    assert!(status.contains("calls: mark 0 · 0 pending"), "{status}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// T16.6: `Stop` hands the flush to a child and returns; the trace lands without the hook
/// waiting for it, and the hook stays fast when the endpoint is unreachable.
#[test]
fn stop_hook_spawns_the_flush_and_stays_under_10ms() {
    let server = MockServer::start();
    let traces = server.mock(|when, then| {
        when.method(POST).path("/v1/traces");
        then.status(200).body("{}");
    });
    let dir = home("stop");
    // A config file the child will read, plus rows worth posting.
    {
        let cx = ctx(&dir, &server.base_url());
        seed(&cx);
    }
    let mut cfg = std::fs::read_to_string(dir.join("config.toml")).unwrap_or_default();
    cfg.push_str(&format!(
        "\n[otel]\nendpoint = \"{}\"\nflush_secs = 2\n",
        server.base_url()
    ));
    std::fs::write(dir.join("config.toml"), cfg).unwrap();

    let run = |event: &str, payload: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
            .args(["hook", event])
            .env("RTOK_HOME", &dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.as_bytes())
            .unwrap();
        let start = std::time::Instant::now();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
        start.elapsed()
    };
    run(
        "Stop",
        r#"{"session_id":"s1","hook_event_name":"Stop","reason":"end_turn"}"#,
    );
    // The child posts on its own; the hook did not wait for it. The deadline has to clear
    // `flush_secs` above with room to spare — at 40 × 50 ms it was 2 s against a 2 s flush
    // interval, so the post landed on the boundary and the test failed on a loaded runner
    // (ci run 34342891823). 400 matches the wait in `tests/proxy.rs`; a green run still breaks
    // out on the first poll that sees the call.
    for _ in 0..400 {
        if traces.calls() > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(traces.calls() >= 1, "the spawned child posted the trace");

    // SessionEnd closes the session so the root span can ship.
    run(
        "SessionEnd",
        r#"{"session_id":"s1","hook_event_name":"SessionEnd","reason":"clear"}"#,
    );
    // T83.15: the flush child may still hold the store; then SessionEnd defers to a child of
    // its own, so `ended_at` lands a moment later rather than before the hook returns.
    let cx = ctx(&dir, &server.base_url());
    let ended = || {
        cx.store
            .sessions_ended_after(1)
            .unwrap()
            .into_iter()
            .any(|s| s.id == "s1")
    };
    for _ in 0..200 {
        if ended() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(ended(), "SessionEnd set ended_at");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The hook path must not pay for an endpoint that never answers.
///
/// Measured against the same hook with no endpoint, not against a wall clock. nextest runs
/// this beside the rest of the suite, so the absolute bar was really measuring how loaded
/// the machine was: it failed at load avg 14–23 and on a two-core CI runner (p95 240 ms
/// against a 200 ms debug bar) while passing on every idle box. A spawn that is slow for
/// both configurations says nothing about the endpoint; only the gap does. Interleaved, so
/// whatever else the machine is doing lands on both.
#[test]
fn hooks_stay_fast_with_an_unreachable_endpoint() {
    let dirs = [home("slow"), home("slow-baseline")];
    for (dir, otel) in dirs.iter().zip([
        "\n[otel]\nendpoint = \"http://127.0.0.1:9\"\nflush_secs = 5\n",
        "",
    ]) {
        let mut cfg = std::fs::read_to_string(dir.join("config.toml")).unwrap_or_default();
        cfg.push_str(otel);
        std::fs::write(dir.join("config.toml"), cfg).unwrap();
    }
    let payload = r#"{"session_id":"s1","hook_event_name":"Stop","reason":"end_turn"}"#;
    let once = |dir: &Path| {
        let start = std::time::Instant::now();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
            .args(["hook", "Stop"])
            .env("RTOK_HOME", dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "hooks fail open");
        assert_eq!(out.stdout, b"{}");
        start.elapsed()
    };
    let n = if cfg!(debug_assertions) { 20 } else { 100 };
    // T74: one sample set decided the gate, and a load spike that lands on only one of
    // the two interleaved configs says nothing about the endpoint. Re-measure both to a
    // deadline: a load blip hits both sides of a retry, a real regression fails every
    // attempt and still fails here.
    let deadline = std::time::Instant::now()
        + if cfg!(debug_assertions) {
            std::time::Duration::from_secs(60)
        } else {
            std::time::Duration::from_secs(120)
        };
    let mut attempt = 0;
    loop {
        attempt += 1;
        let mut samples = [Vec::with_capacity(n), Vec::with_capacity(n)];
        once(&dirs[0]);
        once(&dirs[1]);
        for _ in 0..n {
            for (i, dir) in dirs.iter().enumerate() {
                samples[i].push(once(dir));
            }
        }
        samples.iter_mut().for_each(|s| s.sort());
        let (endpoint, baseline) = (common::p95(&samples[0]), common::p95(&samples[1]));
        // A closed port refuses at once, so the unreachable endpoint may cost a connect
        // attempt — never a timeout, and never the same order as the run itself.
        let bar = baseline + baseline.max(std::time::Duration::from_millis(10));
        if endpoint < bar {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "p95 {endpoint:?} with the endpoint vs {baseline:?} without it, bar {bar:?} — \
             over the bar in all {attempt} attempts"
        );
    }
    dirs.iter().for_each(|d| {
        let _ = std::fs::remove_dir_all(d);
    });
}

/// T16.7: the sums are whole-table aggregates — no watermark, repeated every flush.
#[test]
fn metrics_repeat_the_totals_every_flush() {
    let server = MockServer::start();
    let metrics = server.mock(|when, then| {
        when.method(POST)
            .path("/v1/metrics")
            .body_includes("rtok.tokens")
            .body_includes("rtok.tokens.saved")
            .body_includes("rtok.calls")
            .body_includes(r#""isMonotonic":true"#)
            // `rtok.tokens.saved` can go down (an `expand` row is a negative saving).
            .body_includes(r#""isMonotonic":false"#)
            .body_includes(r#""aggregationTemporality":2"#)
            .body_includes(r#""asInt":"100""#)
            .body_includes(r#""asInt":"2""#);
        then.status(200).body("{}");
    });
    server.mock(|when, then| {
        when.method(POST).path("/v1/traces");
        then.status(200).body("{}");
    });
    server.mock(|when, then| {
        when.method(POST).path("/v1/logs");
        then.status(200).body("{}");
    });
    let dir = home("metrics");
    let cx = ctx(&dir, &server.base_url());
    seed(&cx);
    let r = flush_blocking(&cx);
    assert_eq!(r.error, None, "{r}");
    // 4 token types × 1 model, 1 saved row, 3 call rows.
    assert_eq!(r.points, 8);
    metrics.assert_calls(1);
    // No watermark: a second flush posts the same totals again.
    let r = flush_blocking(&cx);
    assert_eq!((r.spans, r.logs, r.points), (0, 0, 8));
    metrics.assert_calls(2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// T16.9: two overlapping flushes against one store post each row once. The exclusive
/// lock serialises them; the second finds watermarks already advanced.
#[test]
fn concurrent_flushes_post_each_row_once() {
    let server = MockServer::start();
    let traces = server.mock(|when, then| {
        when.method(POST).path("/v1/traces");
        then.status(200)
            .body("{}")
            .delay(Duration::from_millis(250));
    });
    let logs = server.mock(|when, then| {
        when.method(POST).path("/v1/logs");
        then.status(200).body("{}");
    });
    let _metrics = server.mock(|when, then| {
        when.method(POST).path("/v1/metrics");
        then.status(200).body("{}");
    });
    let dir = home("race");
    // Two runtimes, one DB — the cross-process case (proxy + hook child).
    let cx_a = ctx(&dir, &server.base_url());
    seed(&cx_a);
    let cx_b = ctx(&dir, &server.base_url());

    let a = std::thread::spawn(move || flush_blocking(&cx_a));
    std::thread::sleep(Duration::from_millis(30));
    let b = std::thread::spawn(move || flush_blocking(&cx_b));
    let (ra, rb) = (a.join().unwrap(), b.join().unwrap());
    assert_eq!(ra.error, None, "{ra}");
    assert_eq!(rb.error, None, "{rb}");
    // Exactly one flusher posts the call spans; the other sees an empty batch.
    assert_eq!(
        ra.spans + rb.spans,
        3,
        "spans must not double: {ra:?} {rb:?}"
    );
    assert_eq!(ra.logs + rb.logs, 1, "logs must not double: {ra:?} {rb:?}");
    traces.assert_calls(1);
    logs.assert_calls(1);

    let cx = ctx(&dir, &server.base_url());
    assert_eq!(cx.store.otel_mark("calls").unwrap(), 3);
    assert_eq!(cx.store.otel_mark("logs").unwrap(), 1);
    let (pending_calls, pending_logs) = cx.store.otel_pending().unwrap();
    assert_eq!((pending_calls, pending_logs), (0, 0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// T143: a directory each hook-spawned flush child marks with `<pid>.run` for its whole
/// lifetime (`RTOK_OTEL_FLUSH_TRACE`, see `FlushTrace` in `src/otel/export.rs`), so these
/// tests can count concurrent flush children without querying the host process table — no
/// `ps`, no `kill`, ever, per the creator's hard rule. `home` already gives each test its own
/// scratch directory, so nesting the trace dir under it keeps parallel tests apart too.
#[cfg(unix)]
fn trace_dir(base: &Path) -> PathBuf {
    let dir = base.join("flush-trace");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Markers in `trace_dir` with extension `ext`: `run` (child alive), `done` (child exited),
/// `spawned` (written by the hook itself before it exits — T161).
#[cfg(unix)]
fn trace_files(dir: &Path, ext: &str) -> usize {
    std::fs::read_dir(dir)
        .map(|it| {
            it.filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|x| x == ext))
                .count()
        })
        .unwrap_or(0)
}

/// Flush children currently alive, per `trace_dir`.
#[cfg(unix)]
fn trace_count(dir: &Path) -> usize {
    trace_files(dir, "run")
}

/// T161: wait (bounded) until every spawned flush child has exited. An empty `run` set alone
/// is not enough — a child that was spawned but has not started yet has no marker either.
#[cfg(unix)]
fn trace_drained(dir: &Path, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if trace_count(dir) == 0 && trace_files(dir, "done") >= trace_files(dir, "spawned") {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// T143: `Stop`/`SessionEnd` hand a flush to a detached child on every event. Against a slow
/// collector, a burst of them used to pile up one blocked `rtok otel flush` process per event
/// (each one waiting forever on the exclusive flush lock). Fixed by coalescing: at most one
/// running + one queued hook-spawned flush at a time. Reproduces the pile-up on unfixed code
/// (this test fails there — see the task notes for the failing count) and proves the bound
/// holds after the fix, with every seeded row still posted exactly once. Counts concurrent
/// flush children via `trace_dir`/`RTOK_OTEL_FLUSH_TRACE`, never `ps` (see `trace_dir`'s doc).
#[cfg(unix)]
#[test]
fn hook_spawned_flushes_coalesce_to_at_most_two_processes() {
    let server = MockServer::start();
    let delay = Duration::from_millis(500);
    let traces = server.mock(|when, then| {
        when.method(POST).path("/v1/traces");
        then.status(200).body("{}").delay(delay);
    });
    server.mock(|when, then| {
        when.method(POST).path("/v1/logs");
        then.status(200).body("{}").delay(delay);
    });
    server.mock(|when, then| {
        when.method(POST).path("/v1/metrics");
        then.status(200).body("{}").delay(delay);
    });

    let dir = home("burst");
    {
        let cx = ctx(&dir, &server.base_url());
        seed(&cx);
    }
    let mut cfg = std::fs::read_to_string(dir.join("config.toml")).unwrap_or_default();
    cfg.push_str(&format!(
        "\n[otel]\nendpoint = \"{}\"\nflush_secs = 5\n",
        server.base_url()
    ));
    std::fs::write(dir.join("config.toml"), cfg).unwrap();

    let trace = trace_dir(&dir);
    let fire_stop = || {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
            .args(["hook", "Stop"])
            .env("RTOK_HOME", &dir)
            .env("RTOK_OTEL_FLUSH_TRACE", &trace)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(br#"{"session_id":"s1","hook_event_name":"Stop","reason":"end_turn"}"#)
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
    };

    let mut max_seen = 0usize;
    for _ in 0..20 {
        fire_stop();
        max_seen = max_seen.max(trace_count(&trace));
    }
    // Keep sampling a while: children spawned near the end of the burst are still alive.
    let sample_deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < sample_deadline {
        max_seen = max_seen.max(trace_count(&trace));
        std::thread::sleep(Duration::from_millis(20));
    }
    let result: Result<(), String> = if max_seen <= 2 {
        Ok(())
    } else {
        Err(format!(
            "at most 2 hook-spawned flush processes may be alive at once, saw {max_seen}"
        ))
    };

    // Drain: the mock collector answers every in-flight POST on its own delay and the
    // client's `flush_secs` timeout bounds the rest, so every child exits by itself —
    // wait (bounded) for the trace dir to empty instead of killing anything.
    assert!(
        trace_drained(&trace, Duration::from_secs(20)),
        "every flush child must exit on its own within the drain deadline"
    );

    let cx = ctx(&dir, &server.base_url());
    let (pending_calls, pending_logs) = cx.store.otel_pending().unwrap();
    assert_eq!(
        (pending_calls, pending_logs),
        (0, 0),
        "every seeded row must post exactly once, none lost"
    );
    assert!(
        traces.calls() >= 1,
        "at least one flush posted the seeded spans"
    );

    let _ = std::fs::remove_dir_all(&dir);
    result.unwrap();
}

/// T143: a manual `rtok otel flush` (no `--coalesce`) must keep flushing even while a
/// hook-spawned flush is already queued — only the hook path coalesces away.
#[test]
fn manual_flush_ignores_the_queued_slot() {
    let server = MockServer::start();
    for path in ["/v1/traces", "/v1/logs", "/v1/metrics"] {
        server.mock(|when, then| {
            when.method(POST).path(path);
            then.status(200).body("{}");
        });
    }
    let dir = home("manual-queued");
    let cx = ctx(&dir, &server.base_url());
    seed(&cx);
    let _slot = rtok::testutil::hold_otel_queue_slot(&cx);
    let r = flush_blocking(&cx);
    assert_eq!(r.error, None, "{r}");
    assert!(
        r.posted > 0,
        "the manual path must not be coalesced away: {r}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// T143: with the queued slot already held, the hook's pre-check must skip spawning a flush
/// process at all, and stay fast — but a bare wall-clock bound on a spawned subprocess is
/// exactly the load-flakiness `hooks_stay_fast_with_an_unreachable_endpoint` documents (a
/// freshly linked binary's first exec pays a one-time cost unrelated to this code: measured
/// directly, `Runtime::open` + `spawn_child` together take low single-digit milliseconds). A
/// warm-up call on the same binary absorbs that cost before the timed, queued-slot call; the
/// trace-dir count is the real behavioural proof, independent of timing (see `trace_dir`).
#[cfg(unix)]
#[test]
fn hook_skips_spawning_when_a_flush_is_already_queued() {
    let dir = home("skip-queue");
    let trace = trace_dir(&dir);

    let mut cfg = std::fs::read_to_string(dir.join("config.toml")).unwrap_or_default();
    cfg.push_str("\n[otel]\nendpoint = \"http://127.0.0.1:9\"\nflush_secs = 5\n");
    std::fs::write(dir.join("config.toml"), cfg).unwrap();

    let run_stop = || {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
            .args(["hook", "Stop"])
            .env("RTOK_HOME", &dir)
            .env("RTOK_OTEL_FLUSH_TRACE", &trace)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(br#"{"session_id":"s1","hook_event_name":"Stop","reason":"end_turn"}"#)
            .unwrap();
        let start = std::time::Instant::now();
        let out = child.wait_with_output().unwrap();
        (out, start.elapsed())
    };

    // Warm-up: no queued slot yet, so this spawns a real (unreachable-endpoint) flush child,
    // and also pays this binary's first-exec cost before the timed run below.
    let (warm, _) = run_stop();
    assert!(warm.status.success());
    assert_eq!(
        trace_files(&trace, "spawned"),
        1,
        "the warm-up hook spawns exactly one flush child"
    );
    // T161: wait for that child to exit, not merely for its `run` marker to be absent — on a
    // loaded runner it may not have started yet, and would then run into the timed check.
    assert!(
        trace_drained(&trace, Duration::from_secs(20)),
        "the warm-up flush child must exit on its own within the drain deadline"
    );

    let cx = ctx(&dir, "http://127.0.0.1:9");
    let slot = rtok::testutil::hold_otel_queue_slot(&cx);
    drop(cx);
    let (out, elapsed) = run_stop();
    assert!(out.status.success(), "hooks fail open");
    assert_eq!(out.stdout, b"{}");
    assert!(
        elapsed < Duration::from_secs(2),
        "the pre-check path must stay fast, took {elapsed:?}"
    );
    // The hook has exited, so any spawn it made is already marked: no timing involved.
    assert_eq!(
        trace_files(&trace, "spawned"),
        1,
        "queued lock held: no flush process should have been spawned at all"
    );
    assert_eq!(trace_count(&trace), 0, "no flush child is running");

    drop(slot);
    let _ = std::fs::remove_dir_all(&dir);
}

/// T53.4: `tools/otel-check.sh` seeds `RTOK_HOME` before `rtok otel flush`.
#[ignore]
#[test]
fn seed_fixture_ledger() {
    let home = std::env::var("RTOK_HOME").expect("RTOK_HOME");
    let dir = PathBuf::from(home);
    let mut cfg = rtok::testutil::config_in(&dir);
    cfg.core.db_path = dir.join("rtok.db");
    cfg.core.archive_dir = dir.join("archive");
    let cx = Runtime::open(cfg, "s1").unwrap();
    seed(&cx);
}
