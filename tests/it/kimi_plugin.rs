// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T86 + D21: the Kimi host plugin is one manifest, a singleton, CLI+Desktop.
//!
//! Check: `plugins/kimi/kimi.plugin.json` carries the same nine hooks the installer
//! writes (pinned by `agents::kimi::tests::plugin_manifest_matches_the_installer`);
//! it ships no `mcpServers` of its own (T275/D33: `rtok agents install kimi` writes
//! `mcpServers.rtok` into the host's own config, independent of the plugin);
//! `rtok agents install kimi` with a seeded `plugins/managed/rtok/kimi.plugin.json`
//! strips its own hook tables and reports `plugin`; remove leaves the managed copy alone.

use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!["CARGO_MANIFEST_DIR"]).join("plugins/kimi")
}

fn manifest() -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(root().join("kimi.plugin.json")).unwrap()).unwrap()
}

#[test]
fn d21_single_manifest_carries_hooks_only_no_mcp() {
    let m = manifest();
    assert_eq!(m["name"], "rtok");
    let hooks = m["hooks"].as_array().expect("hooks array");
    assert_eq!(hooks.len(), 9, "same nine events the installer writes");
    for h in hooks {
        let cmd = h["command"].as_str().unwrap_or("");
        assert!(cmd.starts_with("rtok hook "), "{h}");
        assert_eq!(h["timeout"], 5);
    }
    // T275/D33: MCP is independent of the plugin — the manifest carries none.
    assert!(m.get("mcpServers").is_none(), "{m}");
}

#[test]
fn d21_no_duplicate_call_paths() {
    let m = manifest();
    for h in m["hooks"].as_array().unwrap() {
        let cmd = h["command"].as_str().unwrap_or("");
        assert!(
            !cmd.contains("rtok read") && !cmd.contains("rtok search"),
            "hooks must not duplicate MCP read/search: {cmd}"
        );
    }
}

#[test]
fn d21_plugin_tree_has_no_launcher_scripts() {
    // Unlike Cursor (scripts/mcp.sh), Kimi starts `rtok mcp` directly and there is
    // nothing to fail open with a ketch hint — rtok must be on PATH (README says so).
    assert!(
        !root().join("scripts").exists(),
        "no scripts/ in plugins/kimi"
    );
    assert!(root().join("kimi.plugin.json").is_file());
}
