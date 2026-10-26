// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T102: lossless round-trip — every plugin that writes an archive row gives back exactly
//! what it archived through `rtok expand <id>`. One walk over all of them with fixtures
//! that include CRLF, non-UTF-8 and an empty body, and a completeness test that fails when
//! a shortening plugin is added without a driver (or a "never archives" plugin starts
//! writing rows). Scope is `src/plugins/`; the hook and MCP surface wraps
//! (`hooks/mod.rs`, `mcp/wrap.rs`) share the same `put_archive` → `expand` mechanism and
//! pin their own round trips in `tests/mcp_wrap.rs` and `tests/archive_rewrite.rs`.

use rtok_plugin_sdk::{Ctx, Plugin, PostToolUse, WireRequest};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const COVERED: &[&str] = &[
    "archive",
    "cmd",
    "graph",
    "guard",
    "json_tree",
    "memory",
    "read",
    "toon",
];
const NEVER: &[&str] = &["compress", "docs", "inject", "measure", "proxy"];

/// The fixture body per case: 50 lines so a capping plugin has something to drop, carrying
/// the property under test. `empty` stays empty on purpose.
fn fixture(kind: &str) -> Vec<u8> {
    let text =
        |f: fn(usize) -> String| -> Vec<u8> { (0..50).map(f).collect::<String>().into_bytes() };
    match kind {
        "plain" => text(|i| format!("line {i}\n")),
        "crlf" => text(|i| format!("line {i}\r\n")),
        "non-utf8" => {
            let mut v = b"\xff\xfe line 0\n".to_vec();
            v.extend(text(|i| format!("line {}\n", i + 1)));
            v
        }
        "empty" => Vec::new(),
        other => panic!("unknown fixture: {other}"),
    }
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rtok-t102-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn runtime(name: &str) -> (rtok::plugin::Runtime, PathBuf) {
    let dir = tmp(name);
    let mut cx = rtok::plugin::Runtime::in_memory(format!("t102-{name}")).unwrap();
    cx.config.core.archive_dir = dir.join("archive");
    cx.config.proxy.mode = "compress".into();
    cx.config.plugins.archive.min_tokens = 1;
    cx.config.plugins.archive.keep_turns = 0;
    cx.config.plugins.toon.enabled = true;
    cx.config.plugins.toon.min_rows = 3;
    cx.config.plugins.graph.max_tokens = 1;
    cx.config.plugins.read.allow_paths.push(dir.clone());
    (cx, dir)
}

/// `rtok expand <id>`'s own fetch must return the archived bytes, byte for byte.
fn round_trip(cx: &rtok::plugin::Runtime, who: &str, id: &str, want: &[u8]) -> usize {
    let got = rtok::expand::fetch(cx, id)
        .unwrap()
        .unwrap_or_else(|| panic!("{who}: expand {id} lost the body"));
    assert_eq!(got, want, "{who}: expand {id} is not the archived bytes");
    1
}

fn last_ref_id(cx: &rtok::plugin::Runtime, plugin: &str) -> Option<String> {
    cx.store
        .list_measurements(plugin)
        .unwrap()
        .last()
        .and_then(|r| r.ref_id.clone())
}

/// `cmd`: `emit_filtered` archives the raw output when it shortened something (T160) and
/// the Measurement row names the id; a body nothing was dropped from stores nothing.
fn cmd(kind: &str, body: &[u8]) -> usize {
    let dir = tmp(&format!("cmd-{kind}"));
    let cfg = rtok::testutil::config_in(&dir);
    rtok::plugins::cmd::run::emit_filtered(&cfg, &["fixture".into()], body, 0, None);
    let cx = rtok::plugin::Runtime::open(cfg, format!("t102-cmd-{kind}")).unwrap();
    let n = match last_ref_id(&cx, "cmd") {
        Some(ref_id) => {
            let id = ref_id.split_once(':').map_or(ref_id.as_str(), |(_, id)| id);
            round_trip(&cx, "cmd", id, body)
        }
        None => 0,
    };
    let _ = fs::remove_dir_all(&dir);
    n
}

/// `read`: the re-read cache archives every body it remembers, bytes untouched.
fn read(kind: &str, body: &[u8]) -> usize {
    let (cx, dir) = runtime(&format!("read-{kind}"));
    let id = rtok::plugins::read::cache::remember(&Ctx::new(&cx), &format!("full:{kind}"), body)
        .unwrap();
    let n = round_trip(&cx, "read", &id, body);
    let _ = fs::remove_dir_all(&dir);
    n
}

/// `guard`: the PostToolUse cache archives the tool's payload so a repeat can deny with a
/// pointer at it. `post_tool` answers `None` and records no Measurement; the id is the
/// archive it wrote — the one file under the archive dir on a fresh runtime.
fn guard(kind: &str, body: &[u8]) -> usize {
    let Ok(text) = std::str::from_utf8(body) else {
        return 0;
    };
    let (cx, dir) = runtime(&format!("guard-{kind}"));
    // Absolute: since T556 a relative path without a cwd has no guard identity, so nothing
    // would be archived.
    let path = dir.join(format!("{kind}.txt"));
    let input = json!({ "file_path": path.to_string_lossy() });
    let response = Value::String(text.to_string());
    let _ = rtok::plugins::guard::GuardPlugin.post_tool(
        &PostToolUse {
            tool_name: "Read",
            tool_input: &input,
            tool_response: &response,
        },
        &Ctx::new(&cx),
    );
    let ids: Vec<String> = fs::read_dir(dir.join("archive"))
        .map(|rd| {
            rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    let n = match ids.first() {
        Some(id) => round_trip(&cx, "guard", id, body),
        None => 0,
    };
    let _ = fs::remove_dir_all(&dir);
    n
}

/// `json_tree`: a nested tree is replaced by a pointer plus its fold; the archived
/// original is the pretty JSON it replaced. Non-UTF-8 and empty fixtures cannot be a tree.
fn json_tree(kind: &str, body: &[u8]) -> usize {
    let Ok(text) = std::str::from_utf8(body) else {
        return 0;
    };
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 3 {
        return 0;
    }
    let fill = json!({"type": "SOLID", "color": "#C0FFEE"});
    let mut children = Vec::new();
    for (i, line) in lines.iter().take(3).enumerate() {
        children.push(json!({
            "id": format!("n-{i}"),
            "name": format!("Node {i}"),
            "type": "FRAME",
            "fills": fill,
            "note": line,
        }));
    }
    let mut root = json!({
        "id": "root",
        "name": "Tree",
        "type": "FRAME",
        "children": children,
    });
    let mut pad = String::new();
    while serde_json::to_vec(&root).unwrap().len() < 256 {
        pad.push_str("pad-");
        root["pad"] = Value::String(pad.clone());
    }
    let original = serde_json::to_string_pretty(&root).unwrap();
    let (mut cx, dir) = runtime(&format!("json-tree-{kind}"));
    cx.config.plugins.json_tree.enabled = true;
    let mut wire = json!({"messages": [{
        "role": "user",
        "content": [{
            "type": "tool_result",
            "tool_use_id": format!("t-{kind}"),
            "content": original,
        }],
    }]});
    let ms = rtok::plugins::json_tree::JsonTreePlugin.proxy_filter(
        &mut WireRequest::new(&rtok::proxy::anthropic::ANTHROPIC, &mut wire),
        &Ctx::new(&cx),
    );
    let id = ms
        .first()
        .and_then(|m| m.ref_id.clone())
        .unwrap_or_else(|| panic!("json_tree left the {kind} tree whole"));
    let n = round_trip(&cx, "json_tree", &id, original.as_bytes());
    let _ = fs::remove_dir_all(&dir);
    n
}

/// `toon`: a tabular block is replaced by a pointer plus its encoding; the archived
/// original is the pretty JSON it replaced. JSON strings are UTF-8 by construction, so
/// the non-UTF-8 fixture cannot reach this path.
fn toon(kind: &str, body: &[u8]) -> usize {
    let Ok(text) = std::str::from_utf8(body) else {
        return 0;
    };
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 3 {
        return 0;
    }
    let table = Value::Array(
        lines
            .iter()
            .enumerate()
            .map(|(i, l)| json!({ "a": i, "b": l, "c": i * 3 }))
            .collect(),
    );
    let original = serde_json::to_string_pretty(&table).unwrap();
    let (cx, dir) = runtime(&format!("toon-{kind}"));
    let mut wire = json!({"messages": [{
        "role": "user",
        "content": [{
            "type": "tool_result",
            "tool_use_id": format!("t-{kind}"),
            "content": original,
        }],
    }]});
    let ms = rtok::plugins::toon::ToonPlugin.proxy_filter(
        &mut WireRequest::new(&rtok::proxy::anthropic::ANTHROPIC, &mut wire),
        &Ctx::new(&cx),
    );
    let id = ms
        .first()
        .and_then(|m| m.ref_id.clone())
        .unwrap_or_else(|| panic!("toon left the {kind} table whole"));
    let n = round_trip(&cx, "toon", &id, original.as_bytes());
    let _ = fs::remove_dir_all(&dir);
    n
}

/// `archive`: an old tool result is replaced by head/tail plus its archive id; the
/// archived original is the result text (JSON strings are UTF-8, as for `toon`).
fn archive(kind: &str, body: &[u8]) -> usize {
    let Ok(text) = std::str::from_utf8(body) else {
        return 0;
    };
    let (cx, dir) = runtime(&format!("archive-{kind}"));
    let mut wire = json!({"messages": [
        {"role": "user", "content": [{"type": "text", "text": "prompt"}]},
        {"role": "assistant", "content": [{"type": "text", "text": "working"}]},
        {"role": "user", "content": [{
            "type": "tool_result",
            "tool_use_id": format!("t-{kind}"),
            "content": text,
        }]},
    ]});
    let ms = rtok::plugins::archive::ArchivePlugin.proxy_filter(
        &mut WireRequest::new(&rtok::proxy::anthropic::ANTHROPIC, &mut wire),
        &Ctx::new(&cx),
    );
    let n = match ms.first().and_then(|m| m.ref_id.clone()) {
        Some(id) => round_trip(&cx, "archive", &id, body),
        None => 0,
    };
    let _ = fs::remove_dir_all(&dir);
    n
}

/// `graph`: `outline` caps its answer and archives the uncapped text — which is exactly
/// what `read`'s `map` mode returns for the same file.
fn graph(kind: &str, body: &[u8]) -> usize {
    let (cx, dir) = runtime(&format!("graph-{kind}"));
    let file = dir.join("fixture.txt");
    fs::write(&file, body).unwrap();
    let path = file.to_string_lossy().into_owned();
    // Two runtimes on purpose: `read`'s re-read cache would answer the call inside
    // `outline` with a delta ("unchanged id …"), and the delta is what would be
    // archived. A separate store reads the original; the fresh one archives it.
    let mut probe = rtok::plugin::Runtime::in_memory("t102-graph-probe").unwrap();
    probe.config.core.archive_dir = dir.join("probe-archive");
    probe.config.plugins.read.allow_paths.push(dir.clone());
    let Ok(original) = rtok::plugins::read::read(&Ctx::new(&probe), &path, "map", None) else {
        // `read` refuses non-UTF-8 files, so no text ever reaches this plugin's cap.
        let _ = fs::remove_dir_all(&dir);
        return 0;
    };
    rtok::plugins::graph::outline(&Ctx::new(&cx), &path).unwrap();
    let id = match last_ref_id(&cx, "graph") {
        Some(id) => id,
        None => {
            let _ = fs::remove_dir_all(&dir);
            return 0;
        }
    };
    let n = round_trip(&cx, "graph", &id, original.as_bytes());
    let _ = fs::remove_dir_all(&dir);
    n
}

/// `memory`: `build_brief` (T130) archives the pointer digest it composes for the
/// `SubagentStart` hook and the `handoff` MCP tool — the ledger's touched-path list plus
/// the fixed read/expand instructions, not the touched file's own body, so `body` only
/// gates which fixtures apply (same as `guard`/`toon`) and the round trip is against the
/// archived digest reconstructed from the returned brief minus its own trailer, with a
/// budget generous enough that nothing gets capped.
fn memory(kind: &str, body: &[u8]) -> usize {
    let Ok(text) = std::str::from_utf8(body) else {
        return 0;
    };
    if text.trim().is_empty() {
        return 0;
    }
    let (cx, dir) = runtime(&format!("memory-{kind}"));
    let ctx = Ctx::new(&cx);
    let stdin = json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Read",
        "tool_input": {"file_path": format!("/fixture-{kind}.rs")},
    });
    // T202: `calls.name` carries the hook event (matches real `hooks::dispatch`), since
    // `handoff::ledger`'s query now filters on it.
    let call_id = cx.record_call("hook", "hook", Some("PreToolUse")).unwrap();
    cx.store
        .insert_call_io(
            call_id,
            Some(&serde_json::to_vec(&stdin).unwrap()),
            None,
            65536,
            None,
        )
        .unwrap();
    let brief = rtok::plugins::memory::handoff::build_brief(&ctx, 100_000, "")
        .unwrap_or_else(|| panic!("memory: build_brief offered nothing for {kind}"));
    let id = last_ref_id(&cx, "memory")
        .unwrap_or_else(|| panic!("memory: no archive id recorded for {kind}"));
    let shown = rtok::names::short_id(&id);
    let trailer = format!("\n[rtok {shown} · expand: rtok expand {shown}]");
    let full = brief
        .strip_suffix(&trailer)
        .unwrap_or_else(|| panic!("memory: brief for {kind} does not end with its own trailer"));
    let n = round_trip(&cx, "memory", &id, full.as_bytes());
    let _ = fs::remove_dir_all(&dir);
    n
}

type Driver = fn(&str, &[u8]) -> usize;

const WALK: &[(&str, Driver)] = &[
    ("archive", archive),
    ("cmd", cmd),
    ("graph", graph),
    ("guard", guard),
    ("memory", memory),
    ("json_tree", json_tree),
    ("read", read),
    ("toon", toon),
];

/// The walk: shorten a fixture, take the id, `expand` it, compare bytes — for every
/// plugin that writes an archive row and every fixture its input shape can carry.
#[test]
fn every_archive_row_round_trips_through_expand() {
    for (who, run) in WALK {
        let mut archived = 0;
        for kind in ["plain", "crlf", "non-utf8", "empty"] {
            archived += run(kind, &fixture(kind));
        }
        assert!(
            archived > 0,
            "{who} archived nothing across the fixtures — its driver is broken or the plugin stopped shortening"
        );
    }
}

/// The completeness half of the Check: a plugin added to the catalogue must be classified
/// here — with a fixture driver, or filed under NEVER and checked against its source.
fn source_archives(id: &str) -> bool {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/plugins");
    let mut files = Vec::new();
    let dir = root.join(id);
    if dir.is_dir() {
        collect_rs(&dir, &mut files);
    } else if root.join(format!("{id}.rs")).is_file() {
        files.push(root.join(format!("{id}.rs")));
    }
    files.iter().any(|p| {
        let src = fs::read_to_string(p).unwrap();
        let production = src.split("mod tests").next().unwrap();
        production.contains("put_archive(")
    })
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn every_shortening_plugin_has_a_fixture() {
    let mut classified: Vec<&str> = [COVERED, NEVER].concat();
    classified.sort_unstable();
    let mut catalogue: Vec<&str> = rtok::config::CATALOGUE.iter().map(|(id, _)| *id).collect();
    catalogue.sort_unstable();
    assert_eq!(
        classified, catalogue,
        "a plugin was added: give it a fixture driver or file it under NEVER"
    );
    for id in COVERED {
        assert!(
            source_archives(id),
            "{id} writes no archive row outside its tests — fix the classification"
        );
    }
    for id in NEVER {
        assert!(
            !source_archives(id),
            "{id} now writes an archive row — give it a fixture driver"
        );
    }
}
