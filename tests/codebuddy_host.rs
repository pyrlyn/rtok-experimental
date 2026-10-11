// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok agents install codebuddy` (T560.2) against a fake `codebuddy` on PATH that logs
//! `<CODEBUDDY_CONFIG_DIR>|<argv>` per call and keeps `plugins/installed_plugins.json` the way
//! the real CLI records an install. Unix only: the fake is a shell script.
#![cfg(unix)]

mod common;
use common::agents::{raw_without_claude, rtok, tmp, write_cfg};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn fake_codebuddy(home: &Path) {
    let dir = home.join(".fake-bin");
    fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("codebuddy");
    fs::write(
        &bin,
        r#"#!/bin/sh
[ "$1" = --version ] && { echo "1.0.0 (fake codebuddy)"; exit 0; }
printf '%s|%s\n' "${CODEBUDDY_CONFIG_DIR:-}" "$*" >> "$HOME/codebuddy.log"
d="${CODEBUDDY_CONFIG_DIR:-$HOME/.codebuddy}"
case "$*" in
  "plugin install rtok@rtok -s user") mkdir -p "$d/plugins"
    printf '{"version":2,"plugins":{"rtok@rtok":[{"scope":"user"}]}}' > "$d/plugins/installed_plugins.json" ;;
  "plugin uninstall rtok@rtok -s user -y") rm -f "$d/plugins/installed_plugins.json" ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
}

fn log(home: &Path) -> Vec<String> {
    fs::read_to_string(home.join("codebuddy.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

/// One variant end to end: `flag` picks it, `dir` is its config directory, `env` the
/// `CODEBUDDY_CONFIG_DIR` the CLI must see (empty for the default CLI directory).
fn lifecycle(name: &str, flag: &str, sub: &str, wants_env: bool) {
    let home = tmp(name);
    let cfg = write_cfg(&home);
    fake_codebuddy(&home);
    let dir = home.join(sub);
    let (settings, mcp) = (dir.join("settings.json"), dir.join(".mcp.json"));
    let env = if wants_env {
        dir.display().to_string()
    } else {
        String::new()
    };
    let install = ["agents", "install", "codebuddy", flag, "--yes"];

    // A plain install first: own hooks and MCP go in, beside a foreign key we must keep.
    fs::write(&settings, r#"{"model":"mine"}"#).unwrap();
    let plain = rtok(&["agents", "install", "codebuddy", flag], &cfg, &home);
    assert!(
        !plain.contains("codebuddy plugin"),
        "no --yes, no offer: {plain}"
    );
    assert!(text(&settings).contains("hook PreToolUse --host codebuddy"));
    assert!(text(&mcp).contains("\"rtok\""));

    // Dry-run names both calls and runs nothing.
    let mut dry = install.to_vec();
    dry.push("--dry-run");
    let out = rtok(&dry, &cfg, &home);
    assert!(out.contains("codebuddy plugin marketplace add"), "{out}");
    assert!(out.contains("plugins/codebuddy --name rtok"), "{out}");
    assert!(
        out.contains("codebuddy plugin install rtok@rtok -s user"),
        "{out}"
    );
    assert_eq!(out.contains("CODEBUDDY_CONFIG_DIR="), wants_env, "{out}");
    assert!(log(&home).is_empty(), "dry-run runs nothing");

    // `--yes`: marketplace add, install; the singleton takes our hooks back, MCP and the
    // foreign key stay.
    let out = rtok(&install, &cfg, &home);
    assert!(out.contains("+ plugin plugins/codebuddy → rtok"), "{out}");
    let calls = log(&home);
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(
        calls[0].starts_with(&format!("{env}|plugin marketplace add ")),
        "{calls:?}"
    );
    assert!(
        calls[0].ends_with("plugins/codebuddy --name rtok"),
        "{calls:?}"
    );
    assert_eq!(calls[1], format!("{env}|plugin install rtok@rtok -s user"));
    assert!(!text(&settings).contains("rtok"), "{}", text(&settings));
    assert!(text(&settings).contains("\"model\""));
    assert!(text(&mcp).contains("\"rtok\""), "D33: MCP stays");

    // A repeat is already installed and makes no CLI call.
    let again = rtok(&install, &cfg, &home);
    assert!(again.contains("already installed"), "{again}");
    assert_eq!(log(&home).len(), 2);

    // Remove uninstalls and takes the MCP entry out.
    let gone = rtok(&["agents", "remove", "codebuddy"], &cfg, &home);
    assert!(gone.contains("- plugin rtok"), "{gone}");
    let calls = log(&home);
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert_eq!(
        calls[2],
        format!("{env}|plugin uninstall rtok@rtok -s user -y")
    );
    assert!(!dir.join("plugins/installed_plugins.json").exists());
    assert!(!text(&mcp).contains("\"rtok\""));
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn the_cli_installs_the_plugin_through_codebuddy_and_remove_uninstalls() {
    lifecycle("codebuddy-cli", "--cli", ".codebuddy", false);
}

#[test]
fn workbuddy_passes_its_directory_as_codebuddy_config_dir() {
    lifecycle("codebuddy-desktop", "--desktop", ".workbuddy", true);
}

/// A CLI that cannot install (the stub of `raw_without_claude` fails everything past
/// `--version`) keeps the offer open and the own files are written instead.
#[test]
fn a_failing_codebuddy_falls_back_to_own_files() {
    let home = tmp("codebuddy-fail");
    let cfg = write_cfg(&home);
    let out = raw_without_claude(
        &["agents", "install", "codebuddy", "--desktop", "--yes"],
        &cfg,
        &home,
    );
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    let out = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.contains("codebuddy failed"), "{out}{err}");
    let settings = text(&home.join(".workbuddy/settings.json"));
    assert!(
        settings.contains("hook PreToolUse --host codebuddy"),
        "{settings}"
    );
    assert!(text(&home.join(".workbuddy/.mcp.json")).contains("\"rtok\""));
    let _ = fs::remove_dir_all(&home);
}
