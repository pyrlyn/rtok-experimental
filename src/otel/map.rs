// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T16.4 (D19): ledger rows → GenAI-semconv spans, events and log records. The table is in
//! `PLAN.md` beside this file; nothing here reads the database or the network.

use serde_json::Value;

use super::otlp::{
    Attr, Event, Kind, LogRecord, Span, Status, b, i, s, seconds_to_ns, span_id, trace_id,
};
use crate::config::Otel;
use crate::store::models::{Call, LogRow, Session};
use crate::store::otel::CallDetail;

/// `invoke_agent {host}`: the root every call span hangs from. Sent once `ended_at` is set.
pub fn session_span(se: &Session, host: Option<&str>) -> Span {
    let host = host.unwrap_or("agent");
    let mut attrs = vec![
        s("gen_ai.operation.name", "invoke_agent"),
        s("gen_ai.agent.name", host),
        s("gen_ai.conversation.id", &se.id),
    ];
    for (k, v) in [
        ("rtok.project", &se.project),
        ("rtok.cwd", &se.cwd),
        ("rtok.source", &se.source),
    ] {
        if let Some(v) = v {
            attrs.push(s(k, v));
        }
    }
    let start = seconds_to_ns(se.started_at);
    Span {
        trace_id: trace_id(&se.id),
        span_id: span_id("session", &se.id),
        parent: None,
        name: format!("invoke_agent {host}"),
        kind: Kind::Internal,
        start_ns: start,
        end_ns: se.ended_at.map(seconds_to_ns).unwrap_or(start),
        attrs,
        events: Vec::new(),
        status: Status::Unset,
    }
}

/// One `calls` row → one span, shaped by its surface (hook / mcp / proxy).
pub fn call_span(call: &Call, d: &CallDetail, cfg: &Otel) -> Span {
    let start = seconds_to_ns(call.ts);
    let end = start + (call.ms.unwrap_or(0.0).max(0.0) * 1e6) as u64;
    let mut attrs = vec![
        s("gen_ai.conversation.id", &call.session_id),
        s("rtok.surface", &call.surface),
        s("rtok.kind", &call.kind),
        i("rtok.call.id", i64::from(call.id)),
    ];
    if let Some(h) = &d.host {
        attrs.push(s("rtok.host", h));
    }
    if let Some(p) = &call.plugin {
        attrs.push(s("rtok.plugin", p));
    }
    let io = d.io.as_ref();
    let req = io.and_then(|io| io.request_json.as_deref());
    let resp = io.and_then(|io| io.response_json.as_deref());
    let req_archive = io.and_then(|io| io.request_archive.as_deref());
    let resp_archive = io.and_then(|io| io.response_archive.as_deref());
    let mut kind = Kind::Internal;
    let name = match call.surface.as_str() {
        "hook" => {
            let event = call.name.clone().unwrap_or_default();
            let v: Value = req
                .and_then(|r| serde_json::from_str(r).ok())
                .unwrap_or(Value::Null);
            attrs.push(s("rtok.hook.event", &event));
            let tool = v["tool_name"].as_str().map(str::to_string);
            match (event.as_str(), tool) {
                ("PreToolUse" | "PostToolUse", Some(tool)) => {
                    attrs.push(s("gen_ai.operation.name", "execute_tool"));
                    attrs.push(s("gen_ai.tool.name", &tool));
                    if let Some(id) = v["tool_use_id"].as_str() {
                        attrs.push(s("gen_ai.tool.call.id", id));
                    }
                    content(
                        cfg,
                        &mut attrs,
                        "gen_ai.tool.call.arguments",
                        json_text(&v["tool_input"]),
                        req_archive,
                    );
                    if event == "PostToolUse" {
                        content(
                            cfg,
                            &mut attrs,
                            "gen_ai.tool.call.result",
                            json_text(&v["tool_response"]),
                            req_archive,
                        );
                    }
                    format!("execute_tool {tool}")
                }
                _ => {
                    if let Some(p) = v["prompt"].as_str() {
                        let msgs = serde_json::json!([{ "role": "user", "parts": [{ "type": "text", "content": p }] }]);
                        content(
                            cfg,
                            &mut attrs,
                            "gen_ai.input.messages",
                            Some(msgs.to_string()),
                            req_archive,
                        );
                    }
                    format!("hook {event}")
                }
            }
        }
        "mcp" => {
            let tool = call.name.clone().unwrap_or_default();
            attrs.push(s("gen_ai.operation.name", "execute_tool"));
            attrs.push(s("gen_ai.tool.name", &tool));
            content(
                cfg,
                &mut attrs,
                "gen_ai.tool.call.arguments",
                req.map(str::to_string),
                req_archive,
            );
            content(
                cfg,
                &mut attrs,
                "gen_ai.tool.call.result",
                resp.map(str::to_string),
                resp_archive,
            );
            format!("execute_tool {tool}")
        }
        "proxy" => {
            kind = Kind::Client;
            let model = d.model.clone().unwrap_or_else(|| "unknown".into());
            attrs.push(s("gen_ai.operation.name", "chat"));
            attrs.push(s("gen_ai.request.model", &model));
            if let Some(p) = &d.provider {
                attrs.push(s("gen_ai.provider.name", p));
            }
            if let Some(u) = &d.usage {
                attrs.push(s("rtok.api", &u.api));
                attrs.push(i("gen_ai.usage.input_tokens", u.input));
                attrs.push(i("gen_ai.usage.output_tokens", u.output));
                attrs.push(i("gen_ai.usage.cache_read.input_tokens", u.cache_read));
                attrs.push(i(
                    "gen_ai.usage.cache_creation.input_tokens",
                    u.cache_create,
                ));
            }
            content(
                cfg,
                &mut attrs,
                "gen_ai.input.messages",
                pick(req, &["messages", "input"]),
                req_archive,
            );
            content(
                cfg,
                &mut attrs,
                "gen_ai.output.messages",
                pick(resp, &["content", "choices", "output"]),
                resp_archive,
            );
            format!("chat {model}")
        }
        other => format!("{other} {}", call.name.clone().unwrap_or_default()),
    };
    let mut events: Vec<Event> = d
        .tokens
        .iter()
        .map(|t| {
            let mut a = vec![
                s("rtok.phase", &t.phase),
                s("rtok.source", &t.source),
                i("rtok.tokens", t.n_tokens),
            ];
            if let Some(p) = &t.plugin {
                a.push(s("rtok.plugin", p));
            }
            if let Some(n) = t.bytes {
                a.push(i("rtok.bytes", n));
            }
            Event {
                name: "rtok.plugin.run".into(),
                time_ns: seconds_to_ns(t.ts),
                attrs: a,
            }
        })
        .collect();
    events.extend(d.measurements.iter().map(|m| {
        let mut a = vec![
            s("rtok.plugin", &m.plugin),
            s("rtok.kind", &m.kind),
            i("rtok.before_bytes", m.before_bytes),
            i("rtok.after_bytes", m.after_bytes),
            i("rtok.est_before", i64::from(m.est_before)),
            i("rtok.est_after", i64::from(m.est_after)),
            i(
                "rtok.tokens.saved",
                i64::from(m.est_before) - i64::from(m.est_after),
            ),
        ];
        if let Some(r) = &m.ref_id {
            a.push(s("rtok.ref_id", r));
        }
        Event {
            name: "rtok.measurement".into(),
            time_ns: start,
            attrs: a,
        }
    }));
    let status = if call.ok != 0 {
        Status::Unset
    } else {
        attrs.push(s("error.type", "_OTHER"));
        Status::Error(call.error.clone().unwrap_or_else(|| "error".into()))
    };
    Span {
        trace_id: trace_id(&call.session_id),
        span_id: span_id("call", &call.id.to_string()),
        parent: Some(span_id("session", &call.session_id)),
        name,
        kind,
        start_ns: start,
        end_ns: end,
        attrs,
        events,
        status,
    }
}

/// One `logs` row → one log record on the session's trace.
pub fn log_record(row: &LogRow) -> LogRecord {
    let level = row.level.to_ascii_lowercase();
    let severity = match level.as_str() {
        "trace" => 1,
        "debug" => 5,
        "warn" | "warning" => 13,
        "error" => 17,
        _ => 9,
    };
    let mut attrs = vec![s("rtok.source", &row.source), s("rtok.name", &row.name)];
    if let Some(p) = &row.plugin {
        attrs.push(s("rtok.plugin", p));
    }
    if let Some(f) = &row.fields {
        attrs.push(s("rtok.fields", f));
    }
    LogRecord {
        time_ns: seconds_to_ns(row.ts),
        severity,
        severity_text: level.to_ascii_uppercase(),
        body: row.message.clone(),
        attrs,
        trace_id: row.session.as_deref().map(trace_id),
        span_id: row.call_id.map(|id| span_id("call", &id.to_string())),
    }
}

/// A JSON value as attribute text: strings verbatim, anything else serialised, null absent.
fn json_text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(t) => Some(t.clone()),
        other => Some(other.to_string()),
    }
}

/// The first of `keys` in a JSON body, else the whole body — the fullest record we hold.
fn pick(body: Option<&str>, keys: &[&str]) -> Option<String> {
    let body = body?;
    let v: Value = serde_json::from_str(body).ok()?;
    keys.iter()
        .find_map(|k| json_text(&v[*k]))
        .or_else(|| Some(body.to_string()))
}

/// Content attributes obey `content` and `content_bytes`; a cut names the archive id (D4).
fn content(
    cfg: &Otel,
    attrs: &mut Vec<Attr>,
    key: &str,
    text: Option<String>,
    archive: Option<&str>,
) {
    if !cfg.content {
        return;
    }
    if let Some(id) = archive {
        attrs.push(s("rtok.archive.id", id));
    }
    let Some(text) = text else { return };
    let cap = cfg.content_bytes as usize;
    if text.len() <= cap {
        attrs.push(s(key, text));
        return;
    }
    let mut cut = cap;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    attrs.push(s(key, &text[..cut]));
    attrs.push(b("rtok.content.truncated", true));
}

#[cfg(test)]
mod tests {
    use super::super::otlp::AttrValue;
    use super::*;
    use crate::store::models::CallIo;
    use crate::store::otel::{CallMeasurement, CallUsage};

    fn call(surface: &str, kind: &str, name: &str) -> Call {
        Call {
            id: 7,
            ts: 1_700_000_000,
            session_id: "s1".into(),
            host_id: None,
            provider_id: None,
            model_id: None,
            plugin: None,
            surface: surface.into(),
            kind: kind.into(),
            parent_id: None,
            name: Some(name.into()),
            ms: Some(25.0),
            ok: 1,
            error: None,
            service_tier: None,
        }
    }

    fn io(req: Option<&str>, resp: Option<&str>) -> CallIo {
        CallIo {
            call_id: 7,
            request_bytes: 0,
            response_bytes: 0,
            request_sha256: None,
            response_sha256: None,
            request_json: req.map(str::to_string),
            response_json: resp.map(str::to_string),
            request_archive: None,
            response_archive: None,
        }
    }

    fn attr<'a>(sp: &'a Span, key: &str) -> Option<&'a AttrValue> {
        sp.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn text<'a>(sp: &'a Span, key: &str) -> Option<&'a str> {
        match attr(sp, key) {
            Some(AttrValue::Str(t)) => Some(t),
            _ => None,
        }
    }

    #[test]
    fn content_requires_opt_in_even_with_a_generic_endpoint() {
        let cfg = Otel::default();
        assert!(
            cfg.resolve_with(|key| (key == "OTEL_EXPORTER_OTLP_ENDPOINT")
                .then(|| "http://localhost:4318".into()))
                .is_some()
        );
        for (surface, kind, name, request, response) in [
            (
                "hook",
                "hook",
                "UserPromptSubmit",
                r#"{"prompt":"private prompt"}"#,
                "",
            ),
            (
                "hook",
                "hook",
                "PostToolUse",
                r#"{"tool_name":"Read","tool_input":{"file_path":"private path"},"tool_response":"private result"}"#,
                "",
            ),
            (
                "mcp",
                "tool_call",
                "read",
                "private arguments",
                "private result",
            ),
            (
                "proxy",
                "api_request",
                "/v1/messages",
                r#"{"messages":[{"role":"user","content":"private prompt"}]}"#,
                r#"{"content":[{"text":"private result"}]}"#,
            ),
        ] {
            let detail = CallDetail {
                io: Some(io(Some(request), Some(response))),
                ..CallDetail::default()
            };
            let c = call(surface, kind, name);
            let span = call_span(&c, &detail, &cfg);
            assert!(span.attrs.iter().all(|(key, _)| !matches!(
                key.as_str(),
                "gen_ai.input.messages"
                    | "gen_ai.output.messages"
                    | "gen_ai.tool.call.arguments"
                    | "gen_ai.tool.call.result"
            )));
            let opted_in = Otel {
                content: true,
                ..cfg.clone()
            };
            let span = call_span(&c, &detail, &opted_in);
            assert!(span.attrs.iter().any(|(_, value)| matches!(value,
                AttrValue::Str(text) if text.contains("private"))));
        }
    }

    #[test]
    fn post_tool_use_is_an_execute_tool_span() {
        let req = r#"{"hook_event_name":"PostToolUse","tool_name":"Read","tool_use_id":"toolu_1","tool_input":{"file_path":"/x"},"tool_response":"body"}"#;
        let d = CallDetail {
            io: Some(io(Some(req), None)),
            ..CallDetail::default()
        };
        let cfg = Otel {
            content: true,
            ..Otel::default()
        };
        let sp = call_span(&call("hook", "hook", "PostToolUse"), &d, &cfg);
        assert_eq!(sp.name, "execute_tool Read");
        assert_eq!(sp.kind, Kind::Internal);
        assert_eq!(
            sp.parent.as_deref(),
            Some(span_id("session", "s1").as_str())
        );
        assert_eq!(sp.trace_id, trace_id("s1"));
        assert_eq!(sp.end_ns - sp.start_ns, 25_000_000);
        assert_eq!(text(&sp, "gen_ai.operation.name"), Some("execute_tool"));
        assert_eq!(text(&sp, "gen_ai.tool.name"), Some("Read"));
        assert_eq!(text(&sp, "gen_ai.tool.call.id"), Some("toolu_1"));
        assert_eq!(
            text(&sp, "gen_ai.tool.call.arguments"),
            Some(r#"{"file_path":"/x"}"#)
        );
        assert_eq!(text(&sp, "gen_ai.tool.call.result"), Some("body"));
        assert_eq!(text(&sp, "rtok.hook.event"), Some("PostToolUse"));
        let sp = call_span(
            &call("hook", "hook", "UserPromptSubmit"),
            &CallDetail {
                io: Some(io(Some(r#"{"prompt":"hi"}"#), None)),
                ..CallDetail::default()
            },
            &cfg,
        );
        assert_eq!(sp.name, "hook UserPromptSubmit");
        assert!(
            text(&sp, "gen_ai.input.messages")
                .unwrap()
                .contains("\"content\":\"hi\"")
        );
    }

    #[test]
    fn proxy_call_is_a_chat_span_with_usage() {
        let cfg = Otel {
            content: true,
            ..Otel::default()
        };
        let d = CallDetail {
            io: Some(io(
                Some(r#"{"model":"claude-x","messages":[{"role":"user","content":"q"}]}"#),
                Some(r#"{"content":[{"type":"text","text":"a"}]}"#),
            )),
            usage: Some(CallUsage {
                api: "anthropic".into(),
                input: 100,
                cache_create: 10,
                cache_read: 20,
                output: 30,
            }),
            measurements: vec![CallMeasurement {
                plugin: "proxy".into(),
                kind: "raw".into(),
                before_bytes: 10,
                after_bytes: 5,
                est_before: 3,
                est_after: 1,
                ref_id: None,
            }],
            provider: Some("anthropic".into()),
            model: Some("claude-x".into()),
            ..CallDetail::default()
        };
        let sp = call_span(&call("proxy", "api_request", "/v1/messages"), &d, &cfg);
        assert_eq!(sp.name, "chat claude-x");
        assert_eq!(sp.kind, Kind::Client);
        assert_eq!(text(&sp, "gen_ai.provider.name"), Some("anthropic"));
        assert_eq!(
            attr(&sp, "gen_ai.usage.input_tokens"),
            Some(&AttrValue::Int(100))
        );
        assert_eq!(
            attr(&sp, "gen_ai.usage.output_tokens"),
            Some(&AttrValue::Int(30))
        );
        assert_eq!(
            attr(&sp, "gen_ai.usage.cache_read.input_tokens"),
            Some(&AttrValue::Int(20))
        );
        assert_eq!(
            attr(&sp, "gen_ai.usage.cache_creation.input_tokens"),
            Some(&AttrValue::Int(10))
        );
        let parsed = |k: &str| serde_json::from_str::<Value>(text(&sp, k).unwrap()).unwrap();
        assert_eq!(
            parsed("gen_ai.input.messages"),
            serde_json::json!([{ "role": "user", "content": "q" }])
        );
        assert_eq!(
            parsed("gen_ai.output.messages"),
            serde_json::json!([{ "type": "text", "text": "a" }])
        );
        assert_eq!(sp.events.len(), 1);
        assert_eq!(sp.events[0].name, "rtok.measurement");
        assert!(sp.events[0].attrs.contains(&i("rtok.tokens.saved", 2)));
    }

    #[test]
    fn content_is_capped_with_the_archive_id_or_dropped() {
        let big = "x".repeat(100 * 1024);
        let mut d = CallDetail {
            io: Some(io(Some(&big), Some(&big))),
            ..CallDetail::default()
        };
        d.io.as_mut().unwrap().response_archive = Some("arc1".into());
        let cfg = Otel {
            content: true,
            content_bytes: 1024,
            ..Otel::default()
        };
        let sp = call_span(&call("mcp", "tool_call", "symbol"), &d, &cfg);
        assert_eq!(sp.name, "execute_tool symbol");
        assert_eq!(text(&sp, "gen_ai.tool.call.result").unwrap().len(), 1024);
        assert_eq!(text(&sp, "rtok.archive.id"), Some("arc1"));
        assert_eq!(
            attr(&sp, "rtok.content.truncated"),
            Some(&AttrValue::Bool(true))
        );
        let off = Otel {
            content: false,
            ..Otel::default()
        };
        let sp = call_span(&call("mcp", "tool_call", "symbol"), &d, &off);
        assert!(
            sp.attrs
                .iter()
                .all(|(k, _)| !k.starts_with("gen_ai.tool.call.") && k != "rtok.archive.id")
        );
        assert_eq!(text(&sp, "gen_ai.tool.name"), Some("symbol"));
    }

    #[test]
    fn failures_and_logs_carry_status_and_ids() {
        let mut c = call("hook", "hook", "PreToolUse");
        c.ok = 0;
        c.error = Some("boom".into());
        let sp = call_span(&c, &CallDetail::default(), &Otel::default());
        assert_eq!(sp.status, Status::Error("boom".into()));
        assert_eq!(text(&sp, "error.type"), Some("_OTHER"));
        let row = LogRow {
            id: 1,
            ts: 5,
            level: "warn".into(),
            source: "otel".into(),
            name: "flush".into(),
            session: Some("s1".into()),
            call_id: Some(7),
            plugin: None,
            message: "m".into(),
            fields: None,
        };
        let r = log_record(&row);
        assert_eq!((r.severity, r.severity_text.as_str()), (13, "WARN"));
        assert_eq!(r.trace_id.as_deref(), Some(trace_id("s1").as_str()));
        assert_eq!(r.span_id.as_deref(), Some(span_id("call", "7").as_str()));
        let se = Session {
            id: "s1".into(),
            host_id: None,
            project: Some("p".into()),
            cwd: None,
            source: None,
            started_at: 1,
            ended_at: Some(2),
        };
        let root = session_span(&se, Some("claude"));
        assert_eq!(root.name, "invoke_agent claude");
        assert_eq!(root.span_id, span_id("session", "s1"));
        assert_eq!(root.end_ns, seconds_to_ns(2));
    }
}
