// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T244 + D21: after `rtok agents install` of every host into one home — run twice, so an
//! append shows — no agent surface sees rtok twice: at most one rtok MCP server, and at most
//! one rtok hook per host event and matcher. A surface counts everything it loads: its own
//! config files (from `agents info --json`), the installed plugin (its hook files under
//! `plugins/`), and the files it imports from another surface (`cross_loads`). Checked once
//! with the Claude Code plugin path (fake `claude` on PATH) and once with the settings-file
//! path (no `claude`), which Grok then imports.
#![cfg(unix)]

mod common;

use common::agents::{rtok, rtok_without_claude, tmp, write_cfg};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The tree under `plugins/` a host links, and whether it ships an MCP server: the OpenCode
/// plugin (shared by Kilo and MiMo Code) filters bash output and carries none; OMP links Pi's extension,
/// which leaves `registerTool` off there (the tools come from `mcp.json`). Cline's plugin
/// (T95: the per-event hook links) is hooks only, and both `mcpServers.rtok` files (CLI + VS
/// Code extension, T96.1) are meant to carry it at once, so neither is a duplicate of the
/// plugin. The Claude plugin carries hooks and the scout sub-agent only (T275): its
/// `.mcp.json` is gone, so it never counts here either. Copilot's, Cursor's, Codex's,
/// ZCode's, Kimi's and Grok's plugins lost their `.mcp.json` / `mcpServers` the same way
/// (T275/D33) — `mcp.servers.rtok` (or the host's equivalent) is now written by setup itself,
/// plugin linked or not. VS Code links the Claude tree, not a `plugins/vscode` folder, so it
/// walks that one too. Gemini is the T275/D33 exception: its extension keeps `mcpServers.rtok`
/// alongside the always-written `settings.json` entry, so it stays on the default arm below.
fn plugin_tree(host: &str) -> (&str, bool) {
    match host {
        "opencode" | "kilo" | "mimo" => ("opencode", false),
        "omp" => ("pi", false),
        "cline" => ("cline", false),
        "claude" => ("claude", false),
        "copilot" => ("copilot", false),
        "vscode" => ("claude", false),
        "cursor" => ("cursor", false),
        "codex" => ("codex", false),
        "zcode" => ("zcode", false),
        "kimi" => ("kimi", false),
        "grok" => ("grok", false),
        // T275.2: the Command Code tree is files the installer links, never a bundle the host
        // loads; its `scripts/mcp.sh` is a by-hand command, so it serves no MCP by itself.
        "commandcode" => ("commandcode", false),
        _ => (host, true),
    }
}

/// What a surface loads besides its own (host, kind) config — `(host, kind, with plugin)`:
/// the desktop app's Code tab still runs Claude Code, so it loads Claude Code's files and
/// plugin too (T243) — dropping that cross-load would also drop the Code-tab hook check. Since
/// T275 both surfaces always carry their own `mcpServers.rtok` entry independently, so this
/// cross-load used to look like the Code tab seeing rtok's MCP server twice; it doesn't,
/// because Claude merges same-name servers across its own config sources (decision rule 2,
/// `merges_same_name`) — the two entries are both named `rtok` and collapse into one live
/// server. Grok imports Claude's settings-file hooks and `mcpServers` through `[compat.claude]`
/// (on by default), not Claude's plugin.
fn cross_loads(host: &str, kind: &str) -> &'static [(&'static str, &'static str, bool)] {
    match (host, kind) {
        ("claude", "desktop") => &[("claude", "cli", true)],
        ("grok", "cli") => &[("claude", "cli", false)],
        _ => &[],
    }
}

/// Whether `host`'s `kind` surface merges MCP server entries that share a name across all the
/// config sources it reads, instead of running one process per source (T275 decision rule 2):
/// true for Claude CLI and Claude Desktop, so a name seen from two sources (e.g. the desktop
/// surface's own file plus the Claude Code file it cross-loads) still counts as one server.
/// Gemini CLI is the D33 exception: its extension and `settings.json` both declare `rtok` by
/// that same real name (`walk_plugin` records the plugin's marker as `rtok` too, not a
/// synthetic per-host name, exactly because Gemini's case must merge like Claude's does), and
/// `settings.json` wins over the extension's same-name server.
fn merges_same_name(host: &str, kind: &str) -> bool {
    matches!(
        (host, kind),
        ("claude", "cli") | ("claude", "desktop") | ("gemini", "cli")
    )
}

/// VS Code and VS Code Insiders are two apps behind one variant, each with its own
/// `mcp.json`: every config file is a surface of its own there.
fn one_surface_per_file(host: &str) -> bool {
    host == "vscode"
}

#[derive(Default)]
struct Seen {
    /// MCP server name (the parent key of its entry, always `rtok` here, including a
    /// plugin's own marker from `walk_plugin`) → sources that install a server under that
    /// name.
    mcp: BTreeMap<String, Vec<String>>,
    /// `event|matcher` → sources.
    hooks: BTreeMap<String, Vec<String>>,
}

fn is_rtok(bin: &str) -> bool {
    let name = Path::new(bin.trim_matches('"')).file_name();
    name.is_some_and(|n| n.eq_ignore_ascii_case("rtok") || n.eq_ignore_ascii_case("rtok.exe"))
}

/// The rtok event of `<rtok> hook <Event> …`, or of `command: rtok, args: [hook, Event]`.
fn hook_event(argv: &[&str]) -> Option<String> {
    let joined = argv.join(" ");
    let at = joined.find(" hook ")?;
    let bin = joined[..at].rsplit(['&', ';', '|']).next()?.trim();
    is_rtok(bin).then(|| {
        joined[at + 6..]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .into()
    })
}

/// Walk one parsed config. `event` is the host's own event key (`hooks.<event>` or an
/// `event` field), `matcher` the nearest `matcher`; one hook entry counts once even when it
/// spells its command twice (Copilot's `bash` + `powershell`). `name` is the JSON key by which
/// the current value was reached from its parent object — for an MCP server entry that is the
/// server's name (`mcpServers.<name>`), whatever the surrounding nesting looks like, since it is
/// always overwritten by the very next hop before the entry itself is examined.
fn walk(
    v: &Value,
    src: &str,
    event: Option<&str>,
    matcher: &str,
    name: Option<&str>,
    seen: &mut Seen,
) {
    match v {
        Value::Object(map) => {
            let event = map.get("event").and_then(Value::as_str).or(event);
            let matcher = map
                .get("matcher")
                .and_then(Value::as_str)
                .unwrap_or(matcher);
            let args = map.get("args").and_then(Value::as_array);
            let args: Vec<&str> = args
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let mut argvs: Vec<Vec<&str>> = map
                .values()
                .filter_map(Value::as_str)
                .map(|s| s.split_whitespace().collect())
                .collect();
            match map.get("command") {
                Some(Value::String(c)) => argvs.push([vec![c.as_str()], args.clone()].concat()),
                Some(Value::Array(a)) => argvs.push(a.iter().filter_map(Value::as_str).collect()),
                _ => {}
            }
            if argvs
                .iter()
                .any(|a| a.first().is_some_and(|b| is_rtok(b)) && a.get(1) == Some(&"mcp"))
            {
                seen.mcp
                    .entry(name.unwrap_or("rtok").into())
                    .or_default()
                    .push(src.into());
            }
            let hits: BTreeSet<String> = argvs.iter().filter_map(|a| hook_event(a)).collect();
            for e in hits {
                let key = format!("{}|{matcher}", event.unwrap_or(&e));
                seen.hooks.entry(key).or_default().push(src.into());
            }
            for (k, x) in map {
                match (k.as_str(), x) {
                    ("hooks", Value::Object(events)) => {
                        for (e, y) in events {
                            walk(y, src, Some(e), matcher, Some(e.as_str()), seen);
                        }
                    }
                    (_, Value::Object(_) | Value::Array(_)) => {
                        walk(x, src, event, matcher, Some(k.as_str()), seen)
                    }
                    _ => {}
                }
            }
        }
        Value::Array(a) => a
            .iter()
            .for_each(|x| walk(x, src, event, matcher, name, seen)),
        _ => {}
    }
}

fn toml_to_json(item: &toml_edit::Item) -> Value {
    use toml_edit::{Item, Value as T};
    fn val(v: &T) -> Value {
        match v {
            T::String(s) => Value::String(s.value().clone()),
            T::Array(a) => Value::Array(a.iter().map(val).collect()),
            T::InlineTable(t) => t.iter().map(|(k, v)| (k.to_string(), val(v))).collect(),
            _ => Value::Null,
        }
    }
    match item {
        Item::Value(v) => val(v),
        Item::Table(t) => t
            .iter()
            .map(|(k, v)| (k.to_string(), toml_to_json(v)))
            .collect(),
        Item::ArrayOfTables(a) => Value::Array(
            a.iter()
                .map(|t| {
                    t.iter()
                        .map(|(k, v)| (k.to_string(), toml_to_json(v)))
                        .collect()
                })
                .collect(),
        ),
        Item::None => Value::Null,
    }
}

fn read_config(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    if path.extension().is_some_and(|e| e == "toml") {
        let doc: toml_edit::DocumentMut = text.parse().unwrap_or_default();
        toml_to_json(doc.as_item())
    } else {
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }
}

fn walk_file(path: &Path, seen: &mut Seen) {
    walk(
        &read_config(path),
        &path.display().to_string(),
        None,
        "",
        None,
        seen,
    );
}

/// The hook entries an installed plugin ships: every JSON file in its tree.
fn walk_plugin(host: &str, seen: &mut Seen) {
    let (tree, mcp) = plugin_tree(host);
    if mcp {
        // Every host with `mcp: true` here ships its plugin's MCP server under the real name
        // `rtok` (never anything else), so the marker uses that same name rather than a
        // synthetic `plugin_<host>` placeholder — required for Gemini to merge with its own
        // `settings.json` entry through `merges_same_name`, and harmless everywhere else since
        // only `len()` (only used when merging) is sensitive to the name at all.
        seen.mcp
            .entry("rtok".to_string())
            .or_default()
            .push(format!("plugin {host}"));
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("plugins")
        .join(tree);
    let mut hooks = Seen::default();
    let mut dirs = vec![root];
    while let Some(dir) = dirs.pop() {
        for p in std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
        {
            if p.is_dir() && !p.ends_with("node_modules") {
                dirs.push(p);
            } else if p.extension().is_some_and(|e| e == "json") {
                walk_file(&p, &mut hooks);
            }
        }
    }
    for (k, v) in hooks.hooks {
        seen.hooks
            .entry(k)
            .or_default()
            .extend(v.into_iter().map(|_| format!("plugin {host}")));
    }
}

/// One variant as `agents info --json` reports it: its config files and whether the
/// host's plugin is installed there.
fn variant(
    infos: &BTreeMap<String, Vec<Value>>,
    host: &str,
    kind: &str,
) -> Option<(Vec<String>, bool)> {
    let v = infos.get(host)?.iter().find(|v| v["kind"] == kind)?;
    let files = v["config"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string);
    let plugin = v["modules"]
        .as_array()?
        .iter()
        .any(|m| m["name"] == "plugin" && m["state"] == "installed");
    Some((files.collect(), plugin))
}

/// What each surface of `host`'s `kind` variant loads: the variant itself first (its own
/// files, cross-loaded hosts' files, and the plugin), then one entry per file for hosts
/// that read each file as a surface of its own.
fn surfaces(infos: &BTreeMap<String, Vec<Value>>, host: &str, kind: &str) -> Vec<(String, Seen)> {
    let mut surfaces: Vec<(String, Seen)> = vec![(kind.into(), Seen::default())];
    let own = std::iter::once((host, kind, true));
    for (h, k, with_plugin) in own.chain(cross_loads(host, kind).iter().copied()) {
        let Some((files, plugin)) = variant(infos, h, k) else {
            continue;
        };
        for f in &files {
            if h == host && one_surface_per_file(host) {
                let mut seen = Seen::default();
                walk_file(Path::new(f), &mut seen);
                surfaces.push((format!("{kind} {f}"), seen));
            } else {
                walk_file(Path::new(f), &mut surfaces[0].1);
            }
        }
        if plugin && with_plugin {
            walk_plugin(h, &mut surfaces[0].1);
        }
    }
    surfaces
}

fn duplicates(run: impl Fn(&[&str]) -> String) -> Vec<String> {
    let list: Vec<Value> = serde_json::from_str(&run(&["agents", "list", "--json"])).unwrap();
    let hosts: BTreeSet<&str> = list.iter().filter_map(|v| v["host"].as_str()).collect();
    assert!(hosts.len() > 10, "{list:?}");
    for _ in 0..2 {
        for host in &hosts {
            run(&["agents", "install", host, "--yes"]);
        }
    }
    let infos: BTreeMap<String, Vec<Value>> = hosts
        .iter()
        .map(|h| {
            let out = run(&["agents", "info", h, "--json"]);
            let v = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{h}: {e}: {out}"));
            (h.to_string(), v)
        })
        .collect();
    let mut bad = Vec::new();
    for (host, variants) in &infos {
        for kind in variants.iter().filter_map(|v| v["kind"].as_str()) {
            for (name, seen) in &surfaces(&infos, host, kind) {
                // A surface that merges same-name servers only sees one live process per
                // distinct name (T275 decision rule 2); elsewhere every source that installs
                // one is a separate process, so count sources as before.
                let mcp_count = if merges_same_name(host, kind) {
                    seen.mcp.len()
                } else {
                    seen.mcp.values().map(Vec::len).sum::<usize>()
                };
                if mcp_count > 1 {
                    bad.push(format!(
                        "{host} {name}: rtok MCP servers from {:?}",
                        seen.mcp
                    ));
                }
                for (key, srcs) in seen.hooks.iter().filter(|(_, s)| s.len() > 1) {
                    bad.push(format!("{host} {name}: {key} fires rtok from {srcs:?}"));
                }
            }
        }
    }
    bad
}

#[test]
fn no_surface_sees_rtok_twice_with_the_claude_plugin() {
    let home = tmp("singleton-plugin");
    let cfg = write_cfg(&home);
    let bad = duplicates(|args| rtok(args, &cfg, &home));
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn no_surface_sees_rtok_twice_with_claude_settings_files() {
    let home = tmp("singleton-files");
    let cfg = write_cfg(&home);
    let bad = duplicates(|args| rtok_without_claude(args, &cfg, &home));
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
