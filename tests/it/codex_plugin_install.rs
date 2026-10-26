// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T140 + D21: `rtok agents install codex` offers rtok's Codex plugin (`plugins/codex`) from
//! the GitHub marketplace `pyrlyn/rtok`, enabled by default once `codex` is on PATH — no flag
//! needed, mirroring T139's Claude flow (a fake `codex` first on PATH records the calls).
//! Codex's docs at https://developers.openai.com/plugins/build/plugins name only `marketplace
//! add|list|upgrade|remove`, but `codex plugin add --help` on the installed CLI (codex-cli
//! 0.155.1) shows real `codex plugin add|remove` subcommands that enable/disable one plugin,
//! verified empirically in a scratch `CODEX_HOME`; the offer runs `codex plugin marketplace
//! add pyrlyn/rtok && codex plugin add rtok@rtok`. A `codex` that fails the
//! offer (`raw_without_claude` leaves only stub CLIs that fail past `--version`) keeps the
//! offer open instead of failing the install, and the file-based hooks/MCP surfaces still go
//! in.
#![cfg(unix)]

use crate::it::common;

use common::agents::{codex_log, rtok, rtok_without_claude, tmp, write_cfg};
use std::fs;

#[test]
fn dry_run_offers_the_codex_commands_and_touches_nothing() {
    let home = tmp("codex-plugin-dry");
    let cfg = write_cfg(&home);
    let out = rtok_without_claude(&["agents", "install", "codex", "--dry-run"], &cfg, &home);
    assert!(out.contains("offer plugins/codex"), "{out}");
    assert!(
        out.contains("codex plugin marketplace add pyrlyn/rtok"),
        "{out}"
    );
    assert!(out.contains("codex plugin add rtok@rtok"), "{out}");
    assert!(out.contains("ketch install pyrlyn/rtok"), "{out}");
    assert!(
        !home.join(".codex/config.toml").exists(),
        "a dry run writes nothing"
    );
    let _ = fs::remove_dir_all(&home);
}

/// A fake `codex` on PATH (`rtok`, unlike `rtok_without_claude`, keeps the whole real PATH plus
/// the fakes) records the calls: install goes straight through the plugin, no `--yes` needed,
/// and while it is installed the plugin is the only call path for hooks (D21). MCP is
/// independent of it (T275/D33): `[mcp_servers.rtok]` is written on the same run regardless,
/// and taken out again on remove. Mirrors `claude_plugin.rs`'s
/// `installs_the_plugin_by_default_as_the_only_call_path_and_remove_uninstalls`.
#[test]
fn installs_the_plugin_by_default_as_the_only_call_path_and_remove_uninstalls() {
    let home = tmp("codex-plugin-default");
    let cfg = write_cfg(&home);
    let config_path = home.join(".codex/config.toml");

    let plain = rtok(&["agents", "install", "codex"], &cfg, &home);
    assert!(
        plain.contains("+ plugin plugins/codex → rtok@rtok"),
        "{plain}"
    );
    let log = codex_log(&home);
    let calls: Vec<&str> = log.lines().collect();
    assert_eq!(calls.len(), 2, "{log}");
    assert_eq!(calls[0], "plugin marketplace add pyrlyn/rtok");
    assert_eq!(calls[1], "plugin add rtok@rtok");
    let config = fs::read_to_string(&config_path).unwrap_or_default();
    assert!(
        config.contains("[mcp_servers.rtok]"),
        "T275/D33: config.toml gets rtok's mcp entry even with the plugin installed: {config}"
    );

    let again = rtok(&["agents", "install", "codex"], &cfg, &home);
    assert!(again.contains("already installed"), "{again}");
    assert_eq!(codex_log(&home).lines().count(), 2, "no second install");
    assert!(
        fs::read_to_string(&config_path)
            .unwrap_or_default()
            .contains("[mcp_servers.rtok]"),
        "a repeat install must not strip the mcp entry"
    );

    let removed = rtok(&["agents", "remove", "codex"], &cfg, &home);
    assert!(removed.contains("- plugin rtok@rtok"), "{removed}");
    let log = codex_log(&home);
    let tail: Vec<&str> = log.lines().skip(2).collect();
    assert_eq!(
        tail,
        ["plugin remove rtok@rtok", "plugin marketplace remove rtok"]
    );
    assert!(
        !fs::read_to_string(&config_path)
            .unwrap_or_default()
            .contains("[mcp_servers.rtok]"),
        "remove takes the mcp entry out too"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn a_failing_codex_keeps_the_offer_open_and_the_file_surfaces_still_go_in() {
    let home = tmp("codex-plugin-missing");
    let cfg = write_cfg(&home);
    let out = rtok_without_claude(&["agents", "install", "codex"], &cfg, &home);
    assert!(
        out.contains("codex failed: fake host: unsupported"),
        "{out}"
    );
    let config = fs::read_to_string(home.join(".codex/config.toml")).unwrap_or_default();
    assert!(
        config.contains("[mcp_servers.rtok]"),
        "D21 fallback: the plugin offer failed, so the file surfaces should still land: {config}"
    );
    assert!(!config.contains("[plugins."), "{config}");
    let _ = fs::remove_dir_all(&home);
}
