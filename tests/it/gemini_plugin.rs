// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T118.3 + D21: the Gemini CLI extension tree carries `gemini-extension.json` with an
//! embedded MCP server, plus Gemini's own `hooks/hooks.json` — and `rtok agents install
//! gemini --yes` drives `gemini extensions link <resolved plugins/gemini>` through the
//! `gemini` CLI (the fake shim's log), taking rtok's own `settings.json` hook entries back
//! (hooks only, D21). Gemini is the T275/D33 exception for MCP: `settings.json` always gets
//! `mcpServers.rtok` too, alongside the extension's own — Gemini's `settings.json` wins over
//! a same-name extension server, so the two merge into one live process.

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::common;
use common::agents::{fake_gemini, rtok, tmp, write_cfg};

fn read(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("plugins/gemini")
        .join(name);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn manifest_is_rtok_with_no_trust_field() {
    let m = read("gemini-extension.json");
    assert_eq!(m["name"], "rtok");
    assert!(m["version"].is_string());
    assert!(m["description"].is_string());
    assert_eq!(
        m["mcpServers"]["rtok"],
        json!({"command": "rtok", "args": ["mcp"]})
    );
    assert!(m.get("trust").is_none(), "trust is unsupported: {m}");
}

/// The tree's hooks file is built from the same event table `settings.json` merges from
/// (D21) — one map, two surfaces, never a drifted copy.
#[test]
fn hooks_are_the_installers_doc_with_the_bare_rtok_command() {
    assert_eq!(
        read("hooks/hooks.json"),
        rtok::agents::gemini::hooks_doc("rtok", 5)
    );
}

#[test]
fn hooks_carry_no_matcher_on_every_event() {
    let doc = read("hooks/hooks.json");
    for (_event, entries) in doc["hooks"].as_object().unwrap() {
        for entry in entries.as_array().unwrap() {
            assert!(entry.get("matcher").is_none(), "{entry}");
        }
    }
}

#[test]
fn the_extension_installs_through_the_gemini_cli_and_remove_uninstalls() {
    let home = tmp("gemini-plugin");
    let cfg = write_cfg(&home);
    fake_gemini(&home);
    let log = home.join("gemini.log");
    let marker = home.join(".gemini/extensions/rtok/gemini-extension.json");

    // Dry-run names the documented local link and runs nothing.
    let dry = rtok(
        &["agents", "install", "gemini", "--yes", "--dry-run"],
        &cfg,
        &home,
    );
    assert!(dry.contains("gemini extensions link"), "{dry}");
    assert!(dry.contains("plugins/gemini"), "{dry}");
    assert!(!log.exists(), "dry-run runs nothing");

    // `--yes` links through the CLI; D21 (hooks only) — the extension is the hooks unit, so
    // rtok's own settings.json hook entries never come beside it. MCP is the T275/D33
    // exception: `mcpServers.rtok` is written into settings.json on this same run regardless.
    let first = rtok(&["agents", "install", "gemini", "--yes"], &cfg, &home);
    assert!(first.contains("+ plugin plugins/gemini → rtok"), "{first}");
    assert!(
        fs::read_to_string(&log)
            .unwrap()
            .contains("extensions link")
    );
    assert!(marker.is_file(), "the shim linked the extension");
    let settings = fs::read_to_string(home.join(".gemini/settings.json")).unwrap_or_default();
    assert!(
        !settings.contains("hook PreToolUse"),
        "D21: no second hooks set: {settings}"
    );
    assert!(
        settings.contains("\"rtok\""),
        "T275/D33: settings.json gets rtok's mcp entry even with the extension linked"
    );

    // A repeat is a no-op with no second CLI call, and the mcp entry is kept, not stripped.
    let again = rtok(&["agents", "install", "gemini", "--yes"], &cfg, &home);
    assert!(again.contains("already installed"), "{again}");
    let calls = fs::read_to_string(&log).unwrap();
    assert_eq!(
        calls
            .lines()
            .filter(|l| l.contains("extensions link"))
            .count(),
        1,
        "{calls}"
    );
    assert!(
        fs::read_to_string(home.join(".gemini/settings.json"))
            .unwrap_or_default()
            .contains("\"rtok\""),
        "a repeat install must not strip the mcp entry"
    );

    // Remove uninstalls by the manifest's `name` and takes the mcp entry out too.
    let removed = rtok(&["agents", "remove", "gemini"], &cfg, &home);
    assert!(removed.contains("- plugin rtok"), "{removed}");
    assert!(!marker.exists());
    assert!(
        !fs::read_to_string(home.join(".gemini/settings.json"))
            .unwrap_or_default()
            .contains("\"rtok\""),
        "remove takes the mcp entry out too"
    );
    let _ = fs::remove_dir_all(&home);
}
