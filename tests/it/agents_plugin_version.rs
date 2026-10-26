// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T382: the `plugin` row of `rtok agents list|info` names the installed plugin's version and
//! source, says so when it differs from the running rtok, and `--json` carries
//! `plugin_version` / `plugin_source` (absent when unknown). Fake `claude` on a temp `HOME`;
//! the version comes from Claude's own `installed_plugins.json`, the lookup `agents outdated`
//! uses.

use crate::it::common;

use common::agents::{rtok, tmp, write_cfg};
use std::fs;
use std::path::Path;

const RTOK: &str = env!("CARGO_PKG_VERSION");

fn record(home: &Path, entry: &str) {
    let plugins = home.join(".claude/plugins");
    fs::create_dir_all(&plugins).unwrap();
    fs::write(
        plugins.join("installed_plugins.json"),
        format!(r#"{{"version":2,"plugins":{{"rtok@rtok":[{entry}]}}}}"#),
    )
    .unwrap();
}

/// The `plugin` line of the first (CLI) Claude Code block, mark and padding included.
fn plugin_line(home: &Path) -> String {
    let cfg = write_cfg(home);
    let out = rtok(&["agents", "info", "claude"], &cfg, home);
    out.lines()
        .find(|l| l.contains(" plugin "))
        .unwrap_or_else(|| panic!("no plugin row:\n{out}"))
        .trim()
        .to_string()
}

fn row(home: &Path) -> serde_json::Value {
    let cfg = write_cfg(home);
    let rows: serde_json::Value =
        serde_json::from_str(&rtok(&["agents", "info", "claude", "--json"], &cfg, home)).unwrap();
    rows[0].clone()
}

#[test]
fn a_plugin_at_the_running_version_shows_it_with_its_source() {
    let home = tmp("pv-current");
    record(&home, &format!(r#"{{"scope":"user","version":"{RTOK}"}}"#));
    assert_eq!(
        plugin_line(&home),
        format!("✓ plugin  installed {RTOK} (github)")
    );
    let r = row(&home);
    assert_eq!(r["plugin_version"], RTOK);
    assert_eq!(r["plugin_source"], "github");
}

#[test]
fn an_older_plugin_is_flagged_with_the_update_command() {
    let home = tmp("pv-older");
    record(&home, r#"{"scope":"user","version":"0.14.0"}"#);
    assert_eq!(
        plugin_line(&home),
        format!("✓ plugin  installed 0.14.0 (github), rtok is {RTOK} — rtok agents update claude")
    );
    assert_eq!(row(&home)["plugin_version"], "0.14.0");
}

#[test]
fn a_newer_plugin_is_flagged_without_an_update_hint() {
    let home = tmp("pv-newer");
    record(&home, r#"{"scope":"user","version":"99.0.0"}"#);
    assert_eq!(
        plugin_line(&home),
        format!("✓ plugin  installed 99.0.0 (github), rtok is {RTOK}")
    );
}

#[test]
fn an_install_with_no_version_anywhere_says_legacy_and_leaves_the_json_fields_out() {
    let home = tmp("pv-legacy");
    record(&home, r#"{"scope":"user"}"#);
    assert_eq!(
        plugin_line(&home),
        "✓ plugin  installed (legacy, no version)"
    );
    let r = row(&home);
    assert!(r.get("plugin_version").is_none(), "{r}");
    assert!(r.get("plugin_source").is_none(), "{r}");
}

#[test]
fn no_plugin_leaves_the_row_and_the_json_fields_alone() {
    let home = tmp("pv-none");
    assert!(plugin_line(&home).starts_with("✗ plugin  not installed"));
    let r = row(&home);
    assert!(r.get("plugin_version").is_none(), "{r}");
}
