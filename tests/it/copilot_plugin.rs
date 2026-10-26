// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T116 + D21: the Copilot CLI plugin tree carries a legacy `plugin.json` manifest and
//! Copilot's camelCase hooks only (T275/D33 dropped its MCP server) — and `rtok agents
//! install copilot --yes` drives `copilot plugin install <resolved plugins/copilot>` through
//! the `copilot` CLI (the fake shim's log), taking rtok's own hooks/rtok.json back while
//! `mcp-config.json`'s `mcpServers.rtok` is written independently, plugin installed or not.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use crate::it::common;
use common::agents::{fake_copilot, rtok, tmp, write_cfg};

fn read(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("plugins/copilot")
        .join(name);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn manifest_is_rtok_with_the_hooks_component_path_and_no_mcp() {
    let m = read("plugin.json");
    assert_eq!(m["name"], "rtok");
    assert_eq!(m["hooks"], "hooks/hooks.json");
    assert!(m.get("mcpServers").is_none(), "{m}");
    assert!(
        !PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("plugins/copilot/.mcp.json")
            .exists(),
        "T275/D33: the plugin no longer ships an MCP server"
    );
}

/// The tree's hooks file is exactly what `~/.copilot/hooks/rtok.json` writes — one shape,
/// two surfaces (D21), never a drifted copy.
#[test]
fn hooks_are_the_installers_doc_with_the_rtok_resolver() {
    assert_eq!(
        read("hooks/hooks.json"),
        rtok::agents::copilot::hooks_doc("rtok", 5)
    );
}

#[test]
fn the_plugin_installs_through_the_copilot_cli_and_remove_uninstalls() {
    let home = tmp("copilot-plugin");
    let cfg = write_cfg(&home);
    fake_copilot(&home);
    let log = home.join("copilot.log");
    let marker = home.join(".copilot/installed-plugins/_direct/x/plugin.json");

    // Dry-run names the documented local install and runs nothing.
    let dry = rtok(
        &["agents", "install", "copilot", "--yes", "--dry-run"],
        &cfg,
        &home,
    );
    assert!(dry.contains("copilot plugin install"), "{dry}");
    assert!(dry.contains("plugins/copilot"), "{dry}");
    assert!(!log.exists(), "dry-run runs nothing");

    // `--yes` installs through the CLI; D21 (hooks only) — the plugin is the hooks unit, so
    // rtok's own hooks/rtok.json never comes beside it. MCP is independent of it (T275/D33):
    // `mcp-config.json`'s `mcpServers.rtok` is written on this same run regardless.
    let first = rtok(&["agents", "install", "copilot", "--yes"], &cfg, &home);
    assert!(first.contains("+ plugin plugins/copilot → rtok"), "{first}");
    assert!(fs::read_to_string(&log).unwrap().contains("plugin install"));
    assert!(marker.is_file(), "the shim installed the plugin");
    assert!(
        !home.join(".copilot/hooks/rtok.json").exists(),
        "D21: no second hooks set: {first}"
    );
    assert!(
        fs::read_to_string(home.join(".copilot/mcp-config.json"))
            .unwrap_or_default()
            .contains("rtok"),
        "T275/D33: mcp-config.json gets rtok's entry even with the plugin installed"
    );

    // A repeat is a no-op with no second CLI call, and the mcp entry is kept, not stripped.
    let again = rtok(&["agents", "install", "copilot", "--yes"], &cfg, &home);
    assert!(again.contains("already installed"), "{again}");
    let calls = fs::read_to_string(&log).unwrap();
    assert_eq!(
        calls
            .lines()
            .filter(|l| l.contains("plugin install"))
            .count(),
        1,
        "{calls}"
    );
    assert!(
        fs::read_to_string(home.join(".copilot/mcp-config.json"))
            .unwrap_or_default()
            .contains("rtok"),
        "a repeat install must not strip the mcp entry"
    );

    // Remove uninstalls by the manifest's `name` and takes the mcp entry out too.
    let removed = rtok(&["agents", "remove", "copilot"], &cfg, &home);
    assert!(removed.contains("- plugin rtok"), "{removed}");
    assert!(!marker.exists());
    assert!(
        !fs::read_to_string(home.join(".copilot/mcp-config.json"))
            .unwrap_or_default()
            .contains("rtok"),
        "remove takes the mcp entry out too"
    );
    let _ = fs::remove_dir_all(&home);
}

/// Every hook's `field` line fails open; only `sessionStart`'s fallback carries the note.
fn assert_hooks_fail_open(doc: &Value, field: &str, run: &dyn Fn(&str) -> (bool, String)) {
    for (event, hook) in doc["hooks"].as_object().unwrap() {
        let (ok, stdout) = run(hook[0][field].as_str().unwrap());
        assert!(ok, "{event}: not fail-open");
        if event == "sessionStart" {
            assert!(stdout.contains("additionalContext"), "{stdout}");
            assert!(stdout.contains("ketch install pyrlyn/rtok"), "{stdout}");
        } else {
            assert_eq!(stdout.trim(), "", "{event}");
        }
    }
}

/// T250.2: `bash` hooks resolve `rtok` from PATH, then `~/.ketch/bin/rtok`, else fail open.
#[cfg(unix)]
#[test]
fn bash_hooks_resolve_rtok_then_ketch_then_fail_open_silently() {
    let sh = common::HookShell::new("copilot-resolver-sh");
    let run = |bash: &str| sh.run(bash);
    let doc = read("hooks/hooks.json");
    assert_hooks_fail_open(&doc, "bash", &run);
    // sessionStart's fallback is Copilot's flat shape, never Claude's hookSpecificOutput.
    let start = doc["hooks"]["sessionStart"][0]["bash"].as_str().unwrap();
    let v: Value = serde_json::from_str(&run(start).1).unwrap();
    assert!(v.get("hookSpecificOutput").is_none(), "{v}");
    // With `~/.ketch/bin/rtok` present, the resolved fallback actually runs it.
    sh.install_fake_ketch_rtok(common::KETCH_ECHO);
    let (ok, stdout) = run(start);
    assert!(ok);
    assert_eq!(stdout, "ketch SessionStart");
}

/// Windows twin for Copilot's `powershell` field; runs only in Windows CI.
#[cfg(windows)]
#[test]
fn powershell_hooks_resolve_rtok_then_ketch_then_fail_open_silently() {
    use std::process::{Command, Stdio};

    let home = tmp("copilot-resolver-ps");
    let rtok_home = tmp("copilot-resolver-ps-rtok-home");
    let empty_path = home.join("empty-path");
    fs::create_dir_all(&empty_path).unwrap();
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let powershell = PathBuf::from(root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let run = |ps: &str| -> (bool, String) {
        let out = Command::new(&powershell)
            .args(["-NoProfile", "-NonInteractive", "-Command", ps])
            .envs([
                ("USERPROFILE", home.as_path()),
                ("PATH", empty_path.as_path()),
                ("RTOK_HOME", rtok_home.as_path()),
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    let doc = read("hooks/hooks.json");
    assert_hooks_fail_open(&doc, "powershell", &run);
    // Copy the real rtok.exe into %USERPROFILE%\.ketch\bin and confirm it actually runs.
    let ketch_bin = home.join(".ketch").join("bin");
    fs::create_dir_all(&ketch_bin).unwrap();
    fs::copy(
        crate::it::common::bin_path("rtok"),
        ketch_bin.join("rtok.exe"),
    )
    .unwrap();
    let field = &doc["hooks"]["sessionStart"][0]["powershell"];
    let (ok, stdout) = run(field.as_str().unwrap());
    assert!(ok);
    assert!(
        !stdout.contains("rtok is not installed"),
        "the real rtok should have run: {stdout}"
    );
    let _ = fs::remove_dir_all(&home);
    let _ = fs::remove_dir_all(&rtok_home);
}
