// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T298: proxy replay bench with a saving floor for `archive`/`toon`/`compress`. Sends a
//! realistic multi-turn request through the real proxy against a fake upstream, then checks
//! the saving end-to-end (see [`end_to_end`]): per tool_use_id, ORIGINAL fixture content vs.
//! that id's content in the body upstream received — summing Measurement rows per plugin
//! instead double-counts a block `archive` pointed at and `compress` shrank further (T298
//! review); [`print_table`] prints that for reference only.
//!
//! Fixture (`fixtures/proxy/messages.json`, 10 turns, realistic timestamps/paths/`ERROR`
//! lines). `keep_turns` (4, default) makes turns 1-6 eligible: tu-1 a CI log, tu-2 an `ls -la`
//! listing, tu-3 a postmortem, all old/large enough to archive and shrink under `compress`.
//! tu-4 passes through untouched; tu-5/tu-6 are small JSON arrays (`toon`); tu-7..tu-10 stay
//! live. Measured 2026-09-27: 91.4% end-to-end (10,054 → 867 est. tokens); floor = measured
//! − 5. Known gap: an `archive` row's `after_bytes` is the intermediate pointer once
//! `compress` shrinks it further; `assert_terminal_bytes` skips those (floor unaffected).

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use httpmock::Method::POST;
use httpmock::MockServer;
use serde_json::Value;

use rtok::config::Config;
use rtok::plugin::Runtime;
use rtok::proxy::ProxyState;
use rtok::store::{MeasRow, Store};
use rtok::tokens::Class;

use crate::it::common;
use common::proxy::{proxy_home_dir, proxy_server};

/// The proxy's store, forwarded bytes, and a runtime on the same store (`estimate`/`expand`).
type Run = (Arc<ProxyState>, Vec<u8>, Runtime);

const FIXTURE: &[u8] = include_bytes!("../fixtures/proxy/messages.json");
const UPSTREAM_BODY: &[u8] = include_bytes!("../fixtures/proxy/anthropic_messages_body.json");
const SESSION: &str = "sess-t298-bench";
const PASSTHROUGH_ID: &str = "tu-4";
const PASSTHROUGH_TEXT: &str = "No matches found.";
/// Measured 2026-09-27 (see module header): end-to-end saving 91.4%.
const FLOOR_PCT: f64 = 86.4;

/// A minimal, valid Anthropic response — enough to log a usage row; never asserted on.
fn mount_upstream() -> &'static MockServer {
    let server = Box::leak(Box::new(MockServer::start()));
    server.mock(|when, then| {
        when.method(POST).path("/v1/messages");
        then.status(200).body(UPSTREAM_BODY);
    });
    server
}

/// The recorder task writes its row after the body is forwarded; poll briefly.
async fn usage_row(store: &Store, session: &str) -> rtok::store::UsageRow {
    for _ in 0..400 {
        let rows = store.usage_rows(session).expect("usage read");
        if let Some(r) = rows.into_iter().next() {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("usage row for {session} never appeared");
}

/// Sends [`FIXTURE`] through the proxy (compress mode); `extra` layers the mutation config.
async fn run(dir_tag: &str, extra: impl FnOnce(&mut Config)) -> Run {
    let up = mount_upstream();
    let (addr, state, task) = proxy_server(dir_tag, |cfg| {
        cfg.proxy.upstream = up.base_url();
        cfg.proxy.mode = "compress".to_string();
        cfg.plugins.compress.enabled = true;
        extra(cfg);
    })
    .await;
    let url = format!("http://{addr}/v1/messages");
    let post = reqwest::Client::new().post(url).body(FIXTURE.to_vec());
    let resp = post.send().await.expect("request through the proxy");
    assert!(resp.status().is_success(), "proxy must forward the fixture");
    let row = usage_row(&state.store, SESSION).await;
    let call_id = row.call_id.expect("usage.call_id points at the calls row") as i32;
    let sent = state
        .store
        .call_io_request(call_id)
        .unwrap()
        .expect("bytes");
    task.abort();
    let cfg = Config::load_from(&proxy_home_dir(dir_tag)).expect("config");
    let cx = Runtime::open(cfg, "expand").expect("expand runtime");
    (state, sent, cx)
}

/// `tool_use_id -> content`, pristine or rewritten (always a plain string either way).
fn tool_results(body: &[u8]) -> HashMap<String, String> {
    let v: Value = serde_json::from_slice(body).expect("json");
    let mut out = HashMap::new();
    for m in v["messages"].as_array().unwrap() {
        if m["role"] != "user" {
            continue;
        }
        for block in m["content"].as_array().unwrap() {
            if block["type"] != "tool_result" {
                continue;
            }
            let id = block["tool_use_id"].as_str().unwrap().to_string();
            out.insert(id, block["content"].as_str().unwrap().to_string());
        }
    }
    out
}

fn pct_saved(before: i64, after: i64) -> f64 {
    (before - after) as f64 * 100.0 / before.max(1) as f64
}

fn print_row(plugin: &str, calls: usize, before: i64, after: i64) {
    let pct = pct_saved(before, after);
    println!("{plugin:<8} {calls:>6} {before:>11} {after:>10} {pct:>7.1}%");
}

/// Informational only (`--nocapture`): per-plugin Measurement rows, which double-count a
/// chained archive→compress block if summed across plugins — see the module header.
fn print_table(store: &Store) {
    println!("plugin    calls  est_before est_after   saving");
    for plugin in ["archive", "toon", "compress"] {
        let rows = store.list_measurements(plugin).unwrap();
        let b: i64 = rows.iter().map(|x| i64::from(x.est_before)).sum();
        let a: i64 = rows.iter().map(|x| i64::from(x.est_after)).sum();
        print_row(plugin, rows.len(), b, a);
    }
}

/// The saving the floor checks: `before`/`after` are each tool_use_id's ORIGINAL fixture
/// content vs. its content in the body upstream received, both estimated the way every
/// plugin does (`Runtime::estimate`, `Class::Code`) — one before/after pair per id, not per
/// plugin row, so a chained archive→compress block is never counted twice.
fn end_to_end(cx: &Runtime, sent: &HashMap<String, String>) -> (i64, i64) {
    let (mut before, mut after) = (0i64, 0i64);
    for (id, orig) in tool_results(FIXTURE) {
        before += i64::from(cx.estimate(&orig, Class::Code));
        let now = sent
            .get(&id)
            .unwrap_or_else(|| panic!("{id} missing from the sent body"));
        after += i64::from(cx.estimate(now, Class::Code));
    }
    (before, after)
}

/// Every `plugin` row not in `skip` must match what upstream received.
/// A full archive id is printed as its 8-character prefix (T909); the row keeps the full id.
fn assert_terminal_bytes(rows: &[MeasRow], plugin: &str, skip: &HashSet<String>, sent: &[&String]) {
    for r in rows {
        let Some(ref_id) = &r.ref_id else { continue };
        if skip.contains(ref_id) {
            continue;
        }
        let needle = printed_archive_id(ref_id);
        let content = sent.iter().find(|c| c.contains(needle));
        let content = content.unwrap_or_else(|| panic!("{plugin} {ref_id}: not sent"));
        assert_eq!(content.len() as i64, r.after_bytes, "{plugin}: bytes");
    }
}

/// The id a pointer prints. Full sha256 archive ids become 8 hex chars; anything else
/// (a toon ref, a prefixed id) is matched as stored.
fn printed_archive_id(ref_id: &str) -> &str {
    if ref_id.len() >= 64 && ref_id.bytes().all(|b| b.is_ascii_hexdigit()) {
        rtok::names::short_id(ref_id)
    } else {
        ref_id
    }
}

#[tokio::test]
async fn proxy_session_saves_over_the_floor() {
    let (state, sent, cx) = run("t298-bench", |_| {}).await;
    let store = &state.store;
    let sent_results = tool_results(&sent);

    // (c) short body passes through byte-identical, though it is outside the live zone.
    let passthrough = sent_results.get(PASSTHROUGH_ID).map(String::as_str);
    assert_eq!(passthrough, Some(PASSTHROUGH_TEXT), "passthrough changed");

    for plugin in ["archive", "toon", "compress"] {
        let rows = store.list_measurements(plugin).unwrap();
        assert!(!rows.is_empty(), "no {plugin} rows");
        assert!(
            rows.iter().any(|m| m.est_after < m.est_before),
            "{plugin}: no shrink"
        );
    }

    print_table(store);
    let (before, after) = end_to_end(&cx, &sent_results);
    let pct = pct_saved(before, after);
    println!("end-to-end {before:>7} {after:>6} {pct:>6.1}%");
    assert!(pct >= FLOOR_PCT, "{pct:.1}% below floor {FLOOR_PCT}%");

    // (e) terminal bytes vs. what upstream received (see the module header's known gap).
    let compress_rows = store.list_measurements("compress").unwrap();
    let cids: HashSet<String> = compress_rows
        .iter()
        .filter_map(|r| r.ref_id.clone())
        .collect();
    let sent_values: Vec<&String> = sent_results.values().collect();
    let toon_rows = store.list_measurements("toon").unwrap();
    let archive_rows = store.list_measurements("archive").unwrap();
    assert_terminal_bytes(&toon_rows, "toon", &HashSet::new(), &sent_values);
    assert_terminal_bytes(&compress_rows, "compress", &HashSet::new(), &sent_values);
    assert_terminal_bytes(&archive_rows, "archive", &cids, &sent_values);

    // (d) every ref_id this run produced expands back to one of the original blocks.
    let original_bytes: HashSet<Vec<u8>> = tool_results(FIXTURE)
        .into_values()
        .map(String::into_bytes)
        .collect();
    let all_rows = archive_rows.iter().chain(&toon_rows).chain(&compress_rows);
    let ref_ids: HashSet<&String> = all_rows.filter_map(|r| r.ref_id.as_ref()).collect();
    assert!(!ref_ids.is_empty(), "no ref_id to expand");
    for id in ref_ids {
        let bytes = rtok::expand::fetch(&cx, id).unwrap();
        let bytes = bytes.unwrap_or_else(|| panic!("expand {id}: empty"));
        assert!(original_bytes.contains(&bytes), "expand {id}: wrong bytes");
    }
}

/// Mutation check: disabling `archive` silences `compress` too (no pointer to summarise), so
/// the total must fall back under the floor — `toon` alone can't hold it up.
#[tokio::test]
async fn disabling_archive_plugin_drops_the_total_below_the_floor() {
    let (state, sent, cx) = run("t298-bench-mut", |cfg| cfg.plugins.archive.enabled = false).await;
    let store = &state.store;
    for plugin in ["archive", "compress"] {
        assert_eq!(
            store.measurement_count(plugin).unwrap(),
            0,
            "{plugin}: not silent"
        );
    }
    print_table(store);
    let (before, after) = end_to_end(&cx, &tool_results(&sent));
    let pct = pct_saved(before, after);
    assert!(pct < FLOOR_PCT, "should drop below floor, got {pct:.1}%");
}
