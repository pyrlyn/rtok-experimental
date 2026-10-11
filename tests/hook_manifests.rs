// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T390, T390.1: every plugin's hook registration matches the one table in
//! `rtok::agents::hook_events` for its host. A `hooks/hooks.json` (Claude's shape), Devin's
//! top-level `hooks.json`, Kimi's `kimi.plugin.json` and the script hooks of Command Code and
//! Cline (which take their event from their link name, so the table is what they are held to)
//! must each register exactly the events the table lists, each running the table's
//! `rtok hook <event>`. Installers read their lists from the same table, so a manifest cannot
//! drift from one: this test is what failed to exist when Cursor's manifest silently lacked two
//! events.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use rtok::agents::hook_events::{HOOK_EVENTS, for_host};
use rtok::hooks::types::HookInput;
use serde_json::Value;

fn plugins() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins")
}

/// Every string below `v`, whatever key holds it (`command`, `bash`, `commandWindows`, …).
fn strings<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
    match v {
        Value::String(s) => out.push(s),
        Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
        Value::Object(o) => o.values().for_each(|x| strings(x, out)),
        _ => {}
    }
}

/// The rtok events an entry runs: the word after a `hook` token (`rtok hook X`, `rtok-hook X`),
/// or ZCode's launcher argument (`"args": ["X"]`). Entries naming neither (Claude's worktree
/// scripts) are not rtok hook events and yield nothing.
fn rtok_events(entry: &Value) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut all = Vec::new();
    strings(entry, &mut all);
    for s in all {
        let words: Vec<&str> = s.split_whitespace().collect();
        for pair in words.windows(2) {
            let name = pair[1].trim_end_matches(';');
            // `command -v rtok-hook >/dev/null` also has a word after `rtok-hook`.
            if pair[0].ends_with("hook") && name.chars().all(|c| c.is_ascii_alphanumeric()) {
                found.insert(name.to_string());
            }
        }
    }
    let mut stack = vec![entry];
    while let Some(v) = stack.pop() {
        match v {
            Value::Object(o) => {
                if let Some(Value::Array(args)) = o.get("args")
                    && let Some(Value::String(first)) = args.first()
                {
                    found.insert(first.clone());
                }
                stack.extend(o.values());
            }
            Value::Array(a) => stack.extend(a),
            _ => {}
        }
    }
    found
}

/// A plugin's hook entries as `(host event, entry)`, whatever shape its manifest has: Claude's
/// `hooks/hooks.json` (`{"hooks": {event: [..]}}`), Devin's `hooks.json` (the same without the
/// wrapper) or Kimi's `kimi.plugin.json` (`{"hooks": [{event, ..}]}`). `None` when the plugin
/// has no such manifest (a script hook, or no hooks at all).
fn manifest_entries(dir: &Path) -> Option<(PathBuf, Vec<(String, Value)>)> {
    let read = |rel: &str| {
        let path = dir.join(rel);
        let doc: Value = serde_json::from_str(&fs::read_to_string(&path).ok()?).unwrap();
        Some((path, doc))
    };
    let by_event = |obj: &Value| {
        obj.as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    };
    if let Some((path, doc)) = read("hooks/hooks.json") {
        return Some((path, by_event(&doc["hooks"])));
    }
    if let Some((path, doc)) = read("hooks.json") {
        return Some((path, by_event(&doc)));
    }
    let (path, doc) = read("kimi.plugin.json")?;
    let entries = doc["hooks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| (h["event"].as_str().unwrap().to_string(), h.clone()))
        .collect();
    Some((path, entries))
}

/// The event `rtok hook <event> --host <host>` resolves to for a script-hook host, or `None`
/// when the dispatcher would treat it as a no-op.
fn dispatched(host: &str, event: &str) -> Option<String> {
    let mut input = HookInput::default();
    match host {
        "cline" => input.adapt_cline(event),
        "commandcode" => input.adapt_commandcode(event, None),
        other => panic!("{other} is not a script-hook host this test knows"),
    }
    let name = input.hook_event_name;
    (!name.is_empty() && name != "Noop").then_some(name)
}

#[test]
fn every_plugin_manifest_matches_the_event_table() {
    let mut seen_hosts = BTreeSet::new();
    for dir in fs::read_dir(plugins()).unwrap().flatten() {
        let host = dir.file_name().to_string_lossy().into_owned();
        let Some((manifest, entries)) = manifest_entries(&dir.path()) else {
            continue;
        };
        let mut actual = BTreeSet::new();
        for (event, entry) in &entries {
            for rtok_event in rtok_events(entry) {
                actual.insert((event.clone(), rtok_event));
            }
        }
        let want: BTreeSet<(String, String)> = for_host(&host)
            .map(|e| (e.host_event.to_string(), e.rtok_event.to_string()))
            .collect();
        assert_eq!(
            actual,
            want,
            "{}: manifest and hook_events::HOOK_EVENTS disagree (left: manifest, right: table)",
            manifest.display()
        );
        seen_hosts.insert(host);
    }
    // A host dropped from `plugins/` would otherwise pass by having nothing to compare.
    for host in ["claude", "codebuddy"] {
        assert!(
            seen_hosts.contains(host),
            "plugins/{host} has no hooks manifest"
        );
    }
    for host in ["cline", "commandcode"] {
        let script = plugins().join(host).join("hooks/rtok-hook");
        let text =
            fs::read_to_string(&script).unwrap_or_else(|e| panic!("{}: {e}", script.display()));
        assert!(
            text.contains(&format!("hook \"$event\" --host {host}")),
            "{}: runs `rtok hook <event> --host {host}` with the event from its own name",
            script.display()
        );
        for e in for_host(host) {
            let event = dispatched(host, e.rtok_event);
            assert!(
                event.is_some(),
                "{host}: `rtok hook {}` is a no-op for the dispatcher",
                e.rtok_event
            );
            // Command Code's adapter passes the name through, so it must already be Claude's.
            assert!(
                host != "commandcode"
                    || ["PreToolUse", "PostToolUse", "SessionStart", "SessionEnd"]
                        .contains(&e.rtok_event),
                "{host}: {} is not a Claude event",
                e.rtok_event
            );
        }
        seen_hosts.insert(host.to_string());
    }
    for e in HOOK_EVENTS {
        assert!(
            seen_hosts.contains(e.host),
            "hook_events names host {} with no manifest or script hook under plugins/{}",
            e.host,
            e.host
        );
    }
}

#[test]
fn table_has_no_duplicate_rows_and_rtok_events_are_known() {
    let mut seen = BTreeSet::new();
    for e in HOOK_EVENTS {
        assert!(
            seen.insert((e.host, e.host_event, e.rtok_event, e.matcher)),
            "duplicate row {} {} {} {:?}",
            e.host,
            e.host_event,
            e.rtok_event,
            e.matcher
        );
    }
    assert!(
        for_host("cursor").all(|e| e.host_event != "subagentStart"),
        "Cursor's subagentStart output has no context field (research.md §23)"
    );
    // T390.1: its output is only `{continue, user_message}` (https://cursor.com/docs/hooks,
    // checked 2026-10-04), so a hook there reaches nobody; `sessionStart` registers the agent.
    let manifest = fs::read_to_string(plugins().join("cursor/hooks/hooks.json")).unwrap();
    assert!(
        for_host("cursor").all(|e| e.host_event != "beforeSubmitPrompt")
            && !manifest.contains("beforeSubmitPrompt"),
        "Cursor's beforeSubmitPrompt output reaches no model: it must stay unregistered"
    );
}
