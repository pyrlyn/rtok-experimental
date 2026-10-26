// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T242: re-running the installer brings what rtok already wrote up to date — the files
//! change where they are stale and stay byte-identical where they are current.
//!
//! Check: seed a host file the way an older rtok left it, run the command, assert the file
//! now holds what the current binary writes (foreign keys and hooks kept) with one backup of
//! the seed; run it again and assert the bytes and the backups do not move.

use crate::common;

use common::agents::{
    backups, claude_log, codex_log, json, raw_without_claude, rtok, rtok_without_claude, tmp,
    write_cfg,
};
use std::fs;

/// What an older install left in `~/.claude/settings.json`: a hook on a versioned store path
/// with another timeout, a `PostToolUse` matcher rtok no longer uses, and a `Stop` hook rtok
/// no longer installs sitting beside a foreign one.
const STALE_SETTINGS: &str = r#"{"theme":"dark","hooks":{
  "PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"/old/store/rtok/v0.1.0/rtok hook PreToolUse","timeout":1}]}],
  "PostToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"rtok hook PostToolUse"}]}],
  "Stop":[{"hooks":[{"type":"command","command":"rtok hook Stop"},{"type":"command","command":"notify-send done"}]}]
}}"#;

#[test]
fn install_rewrites_stale_claude_hooks_and_a_rerun_changes_nothing() {
    let home = tmp("update-stale-hooks");
    let cfg = write_cfg(&home);
    let settings = home.join(".claude/settings.json");
    fs::write(&settings, STALE_SETTINGS).unwrap();

    let out = rtok_without_claude(&["agents", "install", "claude", "--cli"], &cfg, &home);
    let after = fs::read_to_string(&settings).unwrap();
    assert_ne!(after, STALE_SETTINGS, "{out}");
    assert!(!after.contains("/old/store"), "{after}");

    let root = json(&settings);
    let hooks = &root["hooks"];
    // Every fresh hook shares one command shape (bare `rtok`, the T174 PATH-resolving form,
    // or the absolute `rtok.exe` on Windows) — only the event name inside it differs.
    let fresh = hooks["SessionEnd"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    let pre = &hooks["PreToolUse"][0]["hooks"][0];
    assert_eq!(
        pre["command"],
        fresh.replace("SessionEnd", "PreToolUse"),
        "{after}"
    );
    // The timeout every freshly added hook got is the one the stale hook now carries.
    assert_eq!(
        pre["timeout"],
        hooks["SessionEnd"][0]["hooks"][0]["timeout"]
    );
    assert_ne!(pre["timeout"], 1);
    let post = hooks["PostToolUse"].as_array().unwrap();
    assert_eq!(post.len(), 1, "one rtok hook per event: {after}");
    assert_eq!(post[0]["matcher"], "*");
    assert_eq!(
        hooks["Stop"][0]["hooks"],
        serde_json::json!([{"type":"command","command":"notify-send done"}])
    );
    assert_eq!(root["theme"], "dark");

    let baks = backups(&settings);
    assert_eq!(baks.len(), 1, "{baks:?}");
    assert_eq!(fs::read_to_string(&baks[0]).unwrap(), STALE_SETTINGS);

    // Without `claude` on PATH the plugin offer line stays open on every run, so the block
    // never says `already installed`; the hooks themselves must report nothing and the file
    // must keep its bytes.
    let again = rtok_without_claude(&["agents", "install", "claude", "--cli"], &cfg, &home);
    assert!(!again.contains(" hook "), "{again}");
    assert_eq!(fs::read_to_string(&settings).unwrap(), after);
}

/// `mcp_config.json` as an older rtok left it: the server on a versioned store path, beside a
/// foreign server that must survive.
const STALE_WINDSURF: &str = r#"{"mcpServers":{
  "rtok":{"command":"/old/store/rtok/v0.1.0/rtok","args":["mcp"]},
  "other":{"command":"other-mcp","args":[]}
}}"#;

/// The command install writes now: bare `rtok`, or the absolute `rtok.exe` on Windows when
/// `rtok` is not on PATH — never the stale store path the seeds carry.
fn is_current_rtok(cmd: &serde_json::Value) -> bool {
    let cmd = cmd.as_str().unwrap_or("");
    !cmd.contains("/old/store") && (cmd == "rtok" || cmd.ends_with("rtok.exe"))
}

fn windsurf(home: &std::path::Path) -> std::path::PathBuf {
    home.join(".codeium/windsurf/mcp_config.json")
}

/// T242.2: `agents update claude` rewrites stale hooks and a stale `mcpServers.rtok` command
/// in both of Claude Code's files; foreign entries stay.
#[test]
fn update_rewrites_stale_claude_hooks_and_mcp() {
    let home = tmp("update-claude");
    let cfg = write_cfg(&home);
    let settings = home.join(".claude/settings.json");
    let claude_json = home.join(".claude.json");
    fs::write(&settings, STALE_SETTINGS).unwrap();
    let stale_mcp = r#"{"mcpServers":{"rtok":{"type":"stdio","command":"/old/store/rtok/v0.1.0/rtok","args":["mcp"]},"other":{"command":"x"}}}"#;
    fs::write(&claude_json, stale_mcp).unwrap();

    let out = rtok_without_claude(&["agents", "update", "claude", "--cli"], &cfg, &home);
    assert!(out.contains("~ PreToolUse Bash "), "{out}");
    assert!(out.contains("mcpServers.rtok: "), "{out}");
    let mcp = json(&claude_json);
    assert!(
        is_current_rtok(&mcp["mcpServers"]["rtok"]["command"]),
        "{mcp}"
    );
    assert_eq!(mcp["mcpServers"]["other"]["command"], "x");
    assert!(
        !fs::read_to_string(&settings)
            .unwrap()
            .contains("/old/store")
    );
    assert_eq!(
        fs::read_to_string(&backups(&claude_json)[0]).unwrap(),
        stale_mcp
    );
}

/// T242.2: a stale host changes once; the second update says `already current`, keeps the
/// bytes and takes no second backup.
#[test]
fn update_rewrites_once_then_is_already_current() {
    let home = tmp("update-windsurf");
    let cfg = write_cfg(&home);
    let path = windsurf(&home);
    fs::write(&path, STALE_WINDSURF).unwrap();

    let out = rtok(&["agents", "update", "windsurf"], &cfg, &home);
    let after = fs::read_to_string(&path).unwrap();
    assert_ne!(after, STALE_WINDSURF, "{out}");
    let root = json(&path);
    assert!(
        is_current_rtok(&root["mcpServers"]["rtok"]["command"]),
        "{after}"
    );
    // T283.2: an entry written before `--host` gains it on update.
    assert_eq!(
        root["mcpServers"]["rtok"]["args"],
        serde_json::json!(["mcp", "--host", "windsurf"])
    );
    assert_eq!(root["mcpServers"]["other"]["command"], "other-mcp");
    assert_eq!(backups(&path).len(), 1);

    let again = rtok(&["agents", "update", "windsurf"], &cfg, &home);
    assert!(again.contains("Windsurf — already current"), "{again}");
    assert_eq!(fs::read_to_string(&path).unwrap(), after);
    assert_eq!(backups(&path).len(), 1);
}

/// T242.2: with no host named, only hosts that already carry rtok are touched — the Claude
/// and Cursor dirs exist but get no file, the Windsurf entry is refreshed.
#[test]
fn update_without_a_host_touches_only_installed_hosts() {
    let home = tmp("update-all");
    let cfg = write_cfg(&home);
    fs::write(windsurf(&home), STALE_WINDSURF).unwrap();

    let out = rtok(&["agents", "update"], &cfg, &home);
    assert!(out.contains("Windsurf"), "{out}");
    assert!(!out.contains("Claude Code"), "{out}");
    assert!(!home.join(".claude/settings.json").exists());
    assert!(!home.join(".claude.json").exists());
    assert!(!home.join(".cursor/hooks.json").exists());
    assert!(is_current_rtok(
        &json(&windsurf(&home))["mcpServers"]["rtok"]["command"]
    ));
}

/// T242.2: a named host with nothing of rtok is skipped, not installed into; with no
/// installed host at all, update says so and writes nothing.
#[test]
fn update_skips_a_host_rtok_is_not_in() {
    let home = tmp("update-none");
    let cfg = write_cfg(&home);
    let out = rtok(&["agents", "update", "windsurf"], &cfg, &home);
    assert!(out.contains("Windsurf — not installed"), "{out}");
    assert!(out.contains("rtok agents install windsurf"), "{out}");
    assert!(!windsurf(&home).exists());

    let none = rtok(&["agents", "update"], &cfg, &home);
    assert!(none.contains("nothing to update"), "{none}");
}

/// T242.2: `--dry-run` reports the rewrite and leaves the stale file and no backup.
#[test]
fn update_dry_run_writes_nothing() {
    let home = tmp("update-dry");
    let cfg = write_cfg(&home);
    let path = windsurf(&home);
    fs::write(&path, STALE_WINDSURF).unwrap();
    let out = rtok(&["agents", "update", "windsurf", "--dry-run"], &cfg, &home);
    assert!(out.contains("mcpServers.rtok"), "{out}");
    assert_eq!(fs::read_to_string(&path).unwrap(), STALE_WINDSURF);
    assert!(backups(&path).is_empty());
}

/// Claude Code's record of `rtok@rtok` installed from the GitHub marketplace, as an older
/// plugin version left it (T242.3).
fn seed_claude_plugin(home: &std::path::Path) -> std::path::PathBuf {
    let dir = home.join(".claude/plugins");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("known_marketplaces.json"),
        r#"{"rtok":{"source":{"source":"github","repo":"pyrlyn/rtok"}}}"#,
    )
    .unwrap();
    let record = dir.join("installed_plugins.json");
    fs::write(&record, OLD_PLUGIN_RECORD).unwrap();
    record
}

const OLD_PLUGIN_RECORD: &str =
    r#"{"version":2,"plugins":{"rtok@rtok":[{"scope":"user","version":"0.1.0"}]}}"#;

/// T242.3: an installed plugin is updated in place — marketplace refresh, then `plugin
/// update` — never uninstalled; Claude's record changes once, and a second update finds
/// nothing new and says `already current`.
#[test]
fn update_runs_claude_plugin_update_in_place() {
    let home = tmp("update-plugin");
    let cfg = write_cfg(&home);
    let record = seed_claude_plugin(&home);

    let out = rtok(&["agents", "update", "claude", "--cli"], &cfg, &home);
    assert!(out.contains("~ plugin rtok@rtok updated"), "{out}");
    assert_eq!(
        claude_log(&home),
        "plugin marketplace update rtok\nplugin update rtok@rtok\n"
    );
    let after = fs::read_to_string(&record).unwrap();
    assert_ne!(after, OLD_PLUGIN_RECORD);

    let again = rtok(&["agents", "update", "claude", "--cli"], &cfg, &home);
    assert!(again.contains("Claude Code — already current"), "{again}");
    assert!(!claude_log(&home).contains("uninstall"));
    assert_eq!(fs::read_to_string(&record).unwrap(), after);
}

/// T242.3: when `plugin update` fails, update falls back to a reinstall — uninstall, then
/// install — and says why.
#[test]
fn update_reinstalls_the_claude_plugin_when_update_fails() {
    let home = tmp("update-plugin-fail");
    let cfg = write_cfg(&home);
    let record = seed_claude_plugin(&home);
    fs::write(home.join("fake-claude-fail-update"), "").unwrap();

    // T279 PR 3: the fallback reinstall now runs through the same `plugin_against` path a
    // fresh install takes (`+ plugin <src> → rtok@rtok`), not a bespoke "reinstalled" line.
    let out = rtok(&["agents", "update", "claude", "--cli"], &cfg, &home);
    assert!(
        out.contains("+ plugin") && out.contains("rtok@rtok") && out.contains("(update failed"),
        "{out}"
    );
    assert_eq!(
        claude_log(&home),
        "plugin marketplace update rtok\nplugin update rtok@rtok\n\
         plugin uninstall rtok@rtok\nplugin install rtok@rtok\n"
    );
    assert_ne!(fs::read_to_string(&record).unwrap(), OLD_PLUGIN_RECORD);
}

/// T242.3: plain `install` over an installed plugin stays the no-op it was — updating is
/// `agents update`'s job.
#[test]
fn install_leaves_an_installed_claude_plugin_alone() {
    let home = tmp("install-plugin-noop");
    let cfg = write_cfg(&home);
    let record = seed_claude_plugin(&home);
    rtok(&["agents", "install", "claude", "--cli"], &cfg, &home);
    assert_eq!(claude_log(&home), "");
    assert_eq!(fs::read_to_string(&record).unwrap(), OLD_PLUGIN_RECORD);
}

/// Codex's own record of `rtok@rtok` enabled from the GitHub marketplace, beside a foreign
/// table that must survive (T242.4).
const CODEX_WITH_PLUGIN: &str = "model = \"o3\"\n\n[marketplaces.rtok]\nsource_type = \"git\"\n\
source = \"https://github.com/pyrlyn/rtok.git\"\n\n[plugins.\"rtok@rtok\"]\nenabled = true\n";

fn seed_codex_plugin(home: &std::path::Path) -> std::path::PathBuf {
    let path = home.join(".codex/config.toml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, CODEX_WITH_PLUGIN).unwrap();
    path
}

/// T242.4: Codex has no `plugin update`, so an enabled plugin is upgraded in place through
/// `marketplace upgrade` — never removed; the installed cache changes once, and a second
/// update finds nothing new and says `already current`. `config.toml` keeps every table.
#[test]
fn update_upgrades_the_codex_marketplace_in_place() {
    let home = tmp("update-codex-plugin");
    let cfg = write_cfg(&home);
    let config = seed_codex_plugin(&home);

    let out = rtok(&["agents", "update", "codex"], &cfg, &home);
    assert!(out.contains("~ plugin rtok@rtok updated"), "{out}");
    assert_eq!(codex_log(&home), "plugin marketplace upgrade rtok\n");
    let cache = home.join(".codex/plugins/cache/rtok/rtok/0.0.1/.mcp.json");
    assert_eq!(fs::read_to_string(&cache).unwrap().trim(), "upgraded");

    let again = rtok(&["agents", "update", "codex"], &cfg, &home);
    assert!(again.contains("Codex — already current"), "{again}");
    assert!(!codex_log(&home).contains("remove"));
    let doc = fs::read_to_string(&config).unwrap();
    for table in [
        "model = \"o3\"",
        "[marketplaces.rtok]",
        "[plugins.\"rtok@rtok\"]",
    ] {
        assert!(doc.contains(table), "{table} lost: {doc}");
    }
}

/// T242.4: a failed upgrade means a broken snapshot, so update reinstalls the whole chain —
/// plugin and marketplace out, then back in from GitHub — and says why.
#[test]
fn update_reinstalls_the_codex_plugin_when_upgrade_fails() {
    let home = tmp("update-codex-fail");
    let cfg = write_cfg(&home);
    let config = seed_codex_plugin(&home);
    fs::write(home.join("fake-codex-fail-upgrade"), "").unwrap();

    let out = rtok(&["agents", "update", "codex"], &cfg, &home);
    assert!(
        out.contains("~ plugin rtok@rtok reinstalled (update failed"),
        "{out}"
    );
    assert_eq!(
        codex_log(&home),
        "plugin marketplace upgrade rtok\nplugin remove rtok@rtok\n\
         plugin marketplace remove rtok\nplugin marketplace add pyrlyn/rtok\n\
         plugin add rtok@rtok\n"
    );
    let doc = fs::read_to_string(&config).unwrap();
    assert!(doc.contains("[marketplaces.rtok]"), "{doc}");
    assert!(doc.contains("[plugins.\"rtok@rtok\"]"), "{doc}");
}

/// T242.4: plain `install` over an enabled Codex plugin stays a no-op.
#[test]
fn install_leaves_an_enabled_codex_plugin_alone() {
    let home = tmp("install-codex-noop");
    let cfg = write_cfg(&home);
    seed_codex_plugin(&home);
    rtok(&["agents", "install", "codex"], &cfg, &home);
    assert_eq!(codex_log(&home), "");
}

/// T436.4: the per-host bar of `agents install` and `agents update` draws nothing when stderr
/// is a pipe — no carriage return, no escape sequence, no bar text — on either stream.
#[test]
fn install_and_update_draw_no_bar_on_a_pipe() {
    let home = tmp("update-pipe-bar");
    let cfg = write_cfg(&home);
    for args in [
        ["agents", "install", "claude,cursor", "--cli"],
        ["agents", "update", "claude,cursor", "--cli"],
    ] {
        let out = raw_without_claude(&args, &cfg, &home);
        assert!(out.status.success(), "{args:?}: {out:?}");
        for stream in [&out.stdout, &out.stderr] {
            let text = String::from_utf8_lossy(stream);
            assert!(
                !text.contains(['\r', '\u{1b}']) && !text.contains("updating hosts"),
                "{args:?}: {text:?}"
            );
        }
    }
}
