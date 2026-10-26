// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T90 + D21: the Antigravity plugin tree is one unit — a manifest and one MCP server, no hooks.

use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

/// Read and parse one file from the bundled Antigravity plugin.
fn read(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("plugins/antigravity")
        .join(name);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The manifest uses Antigravity's documented portable plugin-name format.
#[test]
fn manifest_name_matches_the_documented_pattern() {
    let name = read("plugin.json")["name"].as_str().unwrap().to_owned();
    assert_eq!(name, "rtok");
    // Antigravity docs: `^[a-zA-Z0-9-_]+$`.
    assert!(
        name.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    );
}

/// The plugin exposes exactly one MCP server that starts `rtok mcp`.
#[test]
fn mcp_config_is_exactly_rtok() {
    assert_eq!(
        read("mcp_config.json"),
        json!({"mcpServers": {"rtok": {"command": "rtok", "args": ["mcp"]}}})
    );
}

/// Antigravity hooks cannot rewrite tool input or add context, so the plugin ships none (T90).
#[test]
fn plugin_ships_no_hooks() {
    let hooks = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins/antigravity/hooks.json");
    assert!(!hooks.exists(), "{}", hooks.display());
}
