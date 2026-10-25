// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Qwen Code installer (`rtok agents install qwen`, plan T413.2).
//!
//! Qwen Code is a Gemini CLI fork, but only the MCP entry stayed Gemini's shape.
//! `[setup.qwen] dir`/`settings.json` (default `~/.qwen/settings.json`, `QWEN_HOME`
//! moves the directory) holds `mcpServers.<name> = {command, args}`. Gemini's own
//! `register_mcp` hardcodes `--host gemini`, so a path override would stamp the
//! wrong host; this module calls [`super::register_stdio_mcp`], the shared writer
//! for that JSON.
//!
//! Hooks did not stay Gemini's. Qwen's events are Claude's names (`PreToolUse`,
//! not `BeforeTool`) and `timeout` is seconds, not milliseconds, so a Gemini
//! hook writer would install events Qwen skips. The command is Claude's too
//! (`rtok hook <event>`, no `--host`): Qwen's stdin already uses
//! `hook_event_name` / `tool_name`, and `--host qwen` would no longer match
//! [`super::claude::is_ours`]. The rows are the `qwen` entries of [`hook_events`].
//! `plugins/qwen` is the D21 hooks unit behind `--yes` (`qwen extensions link`
//! loads `hooks/hooks.json`). That file uses bare `rtok hook <event>` (rtok on
//! PATH). `run` writes the PATH resolver into `settings.json` instead, because
//! that file is generated per machine; both stay on the `qwen` rows of
//! [`hook_events`]. MCP stays in `settings.json` on every install: Qwen's
//! settings file wins over an extension server of the same name.

use std::fs;
use std::path::PathBuf;

use super::hook_events;
use super::{Agent, Kind, Mode, Support, Variant, apply};
use crate::config::Config;
use anyhow::Result;
use rtok_agent_sdk::{edit_json, object_at};

const NAME: &str = "rtok";
const HOST: &str = "qwen";
const PLUGIN_SRC: &str = "plugins/qwen";

/// Qwen Code: one `settings.json`. The desktop app has no documented config path
/// of its own, so there is no desktop variant.
pub struct Qwen;

static VARIANTS: [Variant; 1] = [Variant {
    kind: Kind::Cli,
    name: "Qwen Code",
    bins: &["qwen"],
    apps: &[],
}];

/// `<dir>/settings.json`.
pub fn settings_path(cfg: &Config) -> PathBuf {
    cfg.setup.qwen.dir.join("settings.json")
}

/// Apply, dry-run, or remove rtok's `hooks.<Event>[]` entries. Foreign entries survive.
/// `timeout` is seconds: Qwen reads a command-hook value of 1000 or more as legacy
/// milliseconds, so Gemini's `timeout_s * 1000` would mean a different delay.
pub fn run(cfg: &Config, remove: bool) -> Result<String> {
    let (a, path) = (apply(cfg), settings_path(cfg));
    let timeout = cfg.setup.hook_timeout_s;
    edit_json(&a, &path, |root| {
        let entries = hook_events::entries(HOST);
        if remove {
            super::claude::strip_ours(
                &a,
                &path,
                root.get_mut("hooks"),
                &entries,
                "timeout",
                timeout,
            )
        } else {
            super::claude::insert_ours(
                object_at(root, "hooks"),
                &entries,
                &super::rtok_hook_bin(),
                "timeout",
                timeout,
            )
        }
    })
}

/// `mcpServers.rtok = {command, args}` — the stdio shape Qwen's MCP page shows.
pub fn register_mcp(cfg: &Config) -> Result<String> {
    super::register_stdio_mcp(cfg, &settings_path(cfg), HOST)
}

/// Drop `mcpServers.rtok` from `settings.json`, unless the user edited it (T246.2).
pub fn unregister_mcp(cfg: &Config) -> Result<String> {
    super::unregister_stdio_mcp(cfg, &settings_path(cfg), HOST)
}

/// `<dir>/extensions` — where `qwen extensions link` puts each extension.
fn extensions_dir(cfg: &Config) -> PathBuf {
    cfg.setup.qwen.dir.join("extensions")
}

/// True while some `extensions/*/qwen-extension.json` names rtok. The link's
/// folder name is not pinned by the docs, so the scan does not assume one.
fn plugin_installed(cfg: &Config) -> bool {
    let Ok(entries) = fs::read_dir(extensions_dir(cfg)) else {
        return false;
    };
    entries
        .flatten()
        .any(|e| super::manifest_names(&e.path(), "qwen-extension.json", NAME))
}

/// One `qwen …` call. `QWEN_HOME` only when `dir` is not the default, so a
/// relocated `[setup.qwen] dir` is the directory the CLI actually links into.
fn qwen_cli(cfg: &Config, args: &[&str]) -> std::result::Result<(), String> {
    let dir = cfg.setup.qwen.dir.as_path();
    let default = super::home_dir().join(".qwen");
    let env = (dir != default).then_some(("QWEN_HOME", dir));
    super::run_cli(cfg, "qwen", args, env)
}

/// Offer, install, or uninstall `plugins/qwen` through `qwen extensions`. Behind
/// `--yes`: rtok never writes `~/.qwen/extensions/` itself. A missing `qwen`
/// keeps the offer open; the settings-file hooks and MCP still go in.
fn plugin(cfg: &Config, remove: bool) -> Result<String> {
    super::offer_plugin(
        cfg,
        remove,
        plugin_installed(cfg),
        super::PluginOffer {
            bin: "qwen",
            name: NAME,
            src_rel: PLUGIN_SRC,
            install_verb: &["extensions", "link"],
            uninstall_verb: &["extensions", "uninstall"],
        },
        |args| qwen_cli(cfg, args),
    )
}

impl Agent for Qwen {
    super::agent_identity!(HOST);

    fn support(&self, _kind: Kind, module: &str) -> Support {
        match module {
            "hooks" | "mcp" => Support::Yes,
            "plugin" => Support::Flag("--yes"),
            "proxy" => Support::No(
                "Qwen Code's model.baseUrl is written by the model picker; modelProviders entries carry their own baseUrl, which setup does not edit",
            ),
            _ => Support::No("unknown module"),
        }
    }

    fn plugin_surfaces(&self) -> &'static [rtok_plugin_sdk::Surface] {
        &[rtok_plugin_sdk::Surface::Hook]
    }

    fn files(&self, cfg: &Config, _kind: Kind) -> Vec<PathBuf> {
        vec![settings_path(cfg)]
    }

    fn installed(&self, cfg: &Config, _kind: Kind) -> Vec<&'static str> {
        let text = super::read(&settings_path(cfg));
        let mut out = Vec::new();
        if text.contains(" hook PreToolUse") {
            out.push("hooks");
        }
        if super::mcp::has_entry(&settings_path(cfg), "mcpServers", NAME) {
            out.push("mcp");
        }
        if plugin_installed(cfg) {
            out.push("plugin");
        }
        out
    }

    fn apply(&self, cfg: &Config, _kind: Kind, mode: Mode) -> Result<Vec<String>> {
        // D21: while the extension is linked its hooks/hooks.json already serves,
        // so settings.json hook entries go instead of coming. MCP is the T275/D33
        // exception: settings.json always gets mcpServers.rtok, and that file wins
        // over an extension server of the same name.
        super::d21_plugin_apply(
            cfg,
            mode,
            super::D21Plugin {
                offer: plugin,
                plugin_installed,
                run,
                register_mcp,
                unregister_mcp,
            },
            super::no_extra,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtok_agent_sdk::NO_CHANGES;
    use serde_json::{Value, json};
    use std::fs;

    fn cfg(name: &str, dry: bool) -> (Config, PathBuf) {
        super::super::test_scratch_cfg("qwen", name, "settings.json", dry, |c, path| {
            c.setup.qwen.dir = path.parent().unwrap().to_path_buf();
        })
    }

    #[test]
    fn dry_run_names_hooks_and_mcp_and_writes_nothing() {
        let (c, path) = cfg("dry", true);
        let lines = Qwen.apply(&c, Kind::Cli, Mode::Install).unwrap();
        let out = lines.join("\n");
        assert!(out.contains("+ PreToolUse Bash"), "{out}");
        assert!(out.contains("10 additions"), "{out}");
        assert!(out.contains("mcpServers.rtok: "), "{out}");
        assert!(out.contains("mcp --host qwen"), "{out}");
        assert!(!path.exists());
        assert!(Qwen.installed(&c, Kind::Cli).is_empty());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn apply_is_idempotent_and_remove_keeps_foreign() {
        let (c, path) = cfg("apply", false);
        fs::write(
            &path,
            json!({
                "hooks": {"Notification": [{"hooks": [{"type": "command", "command": "echo hi"}]}]},
                "mcpServers": {"other": {"command": "npx", "args": ["x"]}},
                "env": {"KEEP": "1"}
            })
            .to_string(),
        )
        .unwrap();
        let lines = Qwen.apply(&c, Kind::Cli, Mode::Install).unwrap();
        assert!(
            lines.iter().any(|l| l.contains("10 additions")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.starts_with("mcpServers.rtok: ")),
            "{lines:?}"
        );
        assert!(
            Qwen.apply(&c, Kind::Cli, Mode::Install)
                .unwrap()
                .iter()
                .all(|l| l == NO_CHANGES)
        );

        let root: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let before = &root["hooks"]["PreToolUse"][0];
        assert_eq!(before["matcher"], "Bash");
        let cmd = before["hooks"][0]["command"].as_str().unwrap();
        // Unix settings get the PATH resolver, whose marker is `exec rtok hook <event>`.
        assert!(cmd.contains("hook PreToolUse"), "{cmd}");
        assert!(!cmd.contains("--host"), "{cmd}");
        assert_eq!(before["hooks"][0]["timeout"], 5);
        assert!(root["hooks"]["Notification"].is_array());
        assert_eq!(root["mcpServers"]["rtok"]["args"][2], "qwen");
        assert_eq!(root["mcpServers"]["other"]["command"], "npx");
        assert_eq!(root["env"]["KEEP"], "1");
        assert_eq!(Qwen.installed(&c, Kind::Cli), ["hooks", "mcp"]);

        Qwen.apply(&c, Kind::Cli, Mode::Remove).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("rtok hook"), "{raw}");
        let root: Value = serde_json::from_str(&raw).unwrap();
        assert!(root["hooks"]["Notification"].is_array());
        assert!(root["mcpServers"]["rtok"].is_null());
        assert_eq!(root["mcpServers"]["other"]["command"], "npx");
        assert_eq!(root["env"]["KEEP"], "1");
        assert!(Qwen.installed(&c, Kind::Cli).is_empty());
        assert!(
            Qwen.apply(&c, Kind::Cli, Mode::Remove)
                .unwrap()
                .iter()
                .all(|l| l == NO_CHANGES)
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// While the extension is linked, settings.json loses rtok's hooks and keeps
    /// `mcpServers.rtok` (D21 for hooks, D33 for MCP).
    #[test]
    fn linked_extension_strips_settings_hooks_and_keeps_mcp() {
        let (c, path) = cfg("linked", false);
        Qwen.apply(&c, Kind::Cli, Mode::Install).unwrap();
        let manifest = extensions_dir(&c).join("rtok/qwen-extension.json");
        fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        fs::write(&manifest, r#"{"name":"rtok"}"#).unwrap();
        assert!(plugin_installed(&c));
        Qwen.apply(&c, Kind::Cli, Mode::Install).unwrap();
        let root: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(root["hooks"]["PreToolUse"].is_null(), "{root}");
        assert_eq!(root["mcpServers"]["rtok"]["args"][2], "qwen");
        assert_eq!(Qwen.installed(&c, Kind::Cli), ["mcp", "plugin"]);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// The extension file is the bare PATH spelling of the same `(event, matcher)`
    /// rows `run` writes. Settings.json's resolver is not copied into the package.
    #[test]
    fn extension_hooks_follow_the_event_table() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("plugins/qwen/hooks/hooks.json");
        let file: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let hooks = file["hooks"].as_object().unwrap();
        let mut got = Vec::new();
        for (event, arr) in hooks {
            for entry in arr.as_array().unwrap() {
                let matcher = entry["matcher"].as_str().unwrap_or("");
                let hook = &entry["hooks"][0];
                assert_eq!(hook["command"], format!("rtok hook {event}"));
                assert_eq!(hook["timeout"], 5);
                assert_eq!(hook["type"], "command");
                got.push((event.as_str(), matcher));
            }
        }
        // serde_json maps iterate by key, not by the file's event order.
        let mut want: Vec<(&str, &str)> = hook_events::entries(HOST);
        got.sort();
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn support_matches_hooks_mcp_yes_proxy_no_plugin_flag() {
        assert!(matches!(Qwen.support(Kind::Cli, "hooks"), Support::Yes));
        assert!(matches!(Qwen.support(Kind::Cli, "mcp"), Support::Yes));
        assert!(matches!(Qwen.support(Kind::Cli, "proxy"), Support::No(_)));
        assert!(matches!(
            Qwen.support(Kind::Cli, "plugin"),
            Support::Flag("--yes")
        ));
    }
}
