// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Devin CLI + Desktop (`rtok agents install devin`, plan T89).
//!
//! Both surfaces read the same user files. Hooks live under the `"hooks"` key of
//! `config.json` (event names at the top of that object, each an array of
//! `{matcher?, hooks: [{type, command, timeout}]}` — the same groups as
//! `plugins/devin/hooks.json`, which has no wrapper key). MCP is `mcpServers.rtok`
//! in the sibling `mcp_config.json`.
//!
//! Devin does not document where `devin plugins install --local` records a plugin
//! (plugins overview, fetched 2026-09-25; this machine's `~/.config/devin/` has no
//! plugin store, and `devin plugins list` is a live command, not a file). There is
//! no marker `installed()` can honestly read, so it never reports `plugin`. Setup
//! still writes the user files — that is the path without the plugin — and prints
//! the install line behind `--yes`. It does not guess the store and does not strip
//! on a hunch (a guess either double-fires or deletes hooks the plugin still needs).

use std::path::{Path, PathBuf};

use anyhow::Result;
use rtok_agent_sdk::{NO_CHANGES, edit_json, object_at};
use serde_json::{Value, json};

use super::{Agent, Kind, Mode, Support, Variant, apply, hook_events};
use crate::config::Config;

const NAME: &str = "rtok";

/// `(event, matcher)` in Devin's names, from the `devin` rows of [`hook_events`]. Empty matcher
/// omits the field. Mapped from Claude's list: `Bash`/`Read` → `^exec$`/`^read$`, no `Skill`
/// (Devin has no such tool hook here), `PreCompact`+`PostCompact` → the one event
/// `PostCompaction`. The same list `plugins/devin/hooks.json` carries (T88).
fn entries() -> Vec<(&'static str, &'static str)> {
    hook_events::entries("devin")
}

/// CLI (`devin` on PATH) and Desktop (`Devin.app`): one install writes the same files.
pub struct Devin;

static VARIANTS: [Variant; 2] = [
    Variant {
        kind: Kind::Cli,
        name: "Devin CLI",
        bins: &["devin"],
        apps: &[],
    },
    Variant {
        kind: Kind::Desktop,
        name: "Devin",
        bins: &[],
        apps: &[
            "/Applications/Devin.app",
            "$LOCALAPPDATA/Programs/Devin/Devin.exe",
        ],
    },
];

impl Agent for Devin {
    fn id(&self) -> &'static str {
        "devin"
    }

    fn variants(&self) -> &'static [Variant] {
        &VARIANTS
    }

    fn readme(&self) -> &'static str {
        include_str!("README.md")
    }

    fn support(&self, _kind: Kind, module: &str) -> Support {
        match module {
            "hooks" | "mcp" => Support::Yes,
            "plugin" => Support::Offer("--yes"),
            "proxy" => Support::No(
                "Devin's proxy key is an HTTP proxy for CLI traffic, not a model API base URL",
            ),
            _ => Support::No("not a Devin surface"),
        }
    }

    fn plugin_surfaces(&self) -> &'static [rtok_plugin_sdk::Surface] {
        &[
            rtok_plugin_sdk::Surface::Hook,
            rtok_plugin_sdk::Surface::Mcp,
        ]
    }

    fn shared(&self) -> bool {
        true
    }

    fn files(&self, cfg: &Config, _kind: Kind) -> Vec<PathBuf> {
        vec![config_path(cfg), mcp_path(cfg)]
    }

    fn installed(&self, cfg: &Config, _kind: Kind) -> Vec<&'static str> {
        let mut out = Vec::new();
        if super::read(&config_path(cfg)).contains("rtok hook") {
            out.push("hooks");
        }
        if super::read(&mcp_path(cfg)).contains("\"rtok\"") {
            out.push("mcp");
        }
        out
    }

    fn apply(&self, cfg: &Config, _kind: Kind, mode: Mode) -> Result<Vec<String>> {
        let remove = mode == Mode::Remove;
        if remove {
            return Ok(vec![
                offer_plugin(cfg, true)?,
                run(cfg, true)?,
                unregister_mcp(cfg)?,
            ]);
        }
        let mut lines = vec![run(cfg, false)?];
        if cfg.setup.mcp {
            lines.push(register_mcp(cfg)?);
        }
        // A repeat install is all `NO_CHANGES`, including the offer (kimi's rule):
        // every line counts toward `already installed`.
        if lines.iter().any(|l| l != NO_CHANGES) && cfg.setup.yes {
            lines.insert(0, offer_plugin(cfg, false)?);
        } else {
            lines.insert(0, NO_CHANGES.into());
        }
        Ok(lines)
    }
}

/// `config.json`. On Windows a path ending in the shipped default
/// (`~/.config/devin/config.json`) is not where Devin reads; the docs name
/// `%APPDATA%\devin\config.json`, so that one is redirected. Any other path (tests,
/// `rtok config set`) is kept as is.
pub fn config_path(cfg: &Config) -> PathBuf {
    let path = &cfg.setup.devin.config_path;
    if cfg!(windows)
        && is_xdg_default(path)
        && let Some(appdata) = std::env::var_os("APPDATA")
    {
        return PathBuf::from(appdata).join("devin").join("config.json");
    }
    path.clone()
}

fn is_xdg_default(path: &Path) -> bool {
    path.to_string_lossy()
        .replace('\\', "/")
        .ends_with(".config/devin/config.json")
}

/// `mcp_config.json` beside [`config_path`] (v3000.3+; older builds kept
/// `mcpServers` inside `config.json` and migrate on startup).
pub fn mcp_path(cfg: &Config) -> PathBuf {
    config_path(cfg)
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("mcp_config.json")
}

/// The timeout `plugins/devin/hooks.json` ships with, in seconds: the default
/// `[setup] hook_timeout_s`, since the plugin has no rtok config to read.
const PLUGIN_HOOK_TIMEOUT_S: u64 = 5;

fn command(bin: &str, event: &str) -> String {
    super::claude::command_for("devin", bin, event)
}

/// The POSIX resolver the plugin's `hooks.json` carries on every OS (the file is one
/// checked-in artifact; the installer writes a bare command on Windows instead).
fn plugin_command(event: &str) -> String {
    super::hook_resolver(&format!("hook {event} --host devin"), None)
}

/// The POSIX resolver's `exec rtok hook <event> --host devin;` marker, or a bare
/// `<rtok-bin> hook <event> --host devin` (Windows, absolute path).
fn is_ours(cmd: &str, event: &str) -> bool {
    super::claude::is_ours_for("devin", cmd, event)
}

/// The plugin's `hooks.json`: top-level event names, no `"hooks"` wrapper.
/// Built from [`entries`] and [`plugin_command`], so the installer and the plugin cannot drift.
pub fn hooks_doc() -> Value {
    hooks_doc_with(plugin_command, PLUGIN_HOOK_TIMEOUT_S)
}

/// [`hooks_doc`]'s shape with `command(event)` and `timeout` of the caller's choosing.
fn hooks_doc_with(command: impl Fn(&str) -> String, timeout: u64) -> Value {
    let mut hooks = serde_json::Map::new();
    for (event, matcher) in entries() {
        let mut group = json!({
            "hooks": [{
                "type": "command",
                "command": command(event),
                "timeout": timeout
            }]
        });
        if !matcher.is_empty() {
            group["matcher"] = json!(matcher);
        }
        hooks
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .expect("event array")
            .push(group);
    }
    Value::Object(hooks)
}

/// `devin plugins install --local <resolved plugins/devin>`. rtok never writes
/// the store. `installed` is always false: there is no marker to read.
pub fn offer_plugin(cfg: &Config, remove: bool) -> Result<String> {
    Ok(super::print_offer(
        cfg,
        remove,
        false,
        "plugins/devin",
        &format!(
            "devin plugins install --local {}",
            super::plugin_src("plugins/devin").display()
        ),
        "keep the Devin plugin (its store is undocumented; remove with `devin plugins remove rtok --local`)",
    ))
}

/// Devin's spelling of the hook command; the insert/strip walk is Claude's (same layout).
const FORM: super::claude::HookForm = super::claude::HookForm { command, is_ours };

/// Apply, dry-run, or remove rtok's hooks under `hooks` in `config.json`. Foreign hooks on
/// the same events stay; one of ours the user edited goes only as [`super::takes_hook`] says.
pub fn run(cfg: &Config, remove: bool) -> Result<String> {
    let (a, path) = (apply(cfg), config_path(cfg));
    let timeout = cfg.setup.hook_timeout_s;
    edit_json(&a, &path, |root| {
        if remove {
            let hooks = root.get_mut("hooks");
            super::claude::strip_ours_as(&FORM, &a, &path, hooks, &entries(), "timeout", timeout)
        } else {
            let hooks = object_at(root, "hooks");
            let bin = super::rtok_hook_bin();
            super::claude::insert_ours_as(&FORM, hooks, &entries(), &bin, "timeout", timeout)
        }
    })
}

fn mcp_entry(cmd: &str) -> Value {
    json!({"command": cmd, "args": super::mcp_args("devin")})
}

pub fn register_mcp(cfg: &Config) -> Result<String> {
    let cmd = super::rtok_command();
    rtok_agent_sdk::register_server(
        &apply(cfg),
        &mcp_path(cfg),
        "mcpServers",
        NAME,
        mcp_entry(&cmd),
        &super::mcp_summary(&cmd, "devin"),
    )
}

pub fn unregister_mcp(cfg: &Config) -> Result<String> {
    super::unregister_ours(cfg, &mcp_path(cfg), "mcpServers", NAME, &mcp_entry("rtok"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> (Config, PathBuf) {
        let (mut c, path) =
            super::super::test_scratch_cfg("devin", name, "config.json", false, |c, path| {
                c.setup.devin.config_path = path;
            });
        c.setup.yes = true;
        (c, path)
    }

    fn install(c: &Config) -> Vec<String> {
        Devin.apply(c, Kind::Cli, Mode::Install).unwrap()
    }

    #[test]
    fn install_writes_hooks_and_mcp_and_is_idempotent() {
        let (c, path) = scratch("install");
        let first = install(&c);
        assert!(
            first.iter().any(|l| l.contains("plugins/devin")
                && l.contains("devin plugins install --local")),
            "{first:?}"
        );
        let hooks =
            serde_json::from_str::<Value>(&std::fs::read_to_string(&path).unwrap()).unwrap();
        // What this OS's installer writes: the plugin's resolver on POSIX, a bare command on
        // Windows (`plugin_manifest_matches_the_installer` pins the plugin form itself).
        let want = hooks_doc_with(|e| command("rtok", e), c.setup.hook_timeout_s);
        assert_eq!(hooks["hooks"], want);
        let mcp: Value = serde_json::from_str(
            &std::fs::read_to_string(path.parent().unwrap().join("mcp_config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(mcp["mcpServers"]["rtok"]["command"], "rtok");
        assert_eq!(
            mcp["mcpServers"]["rtok"]["args"],
            json!(["mcp", "--host", "devin"])
        );
        let second = install(&c);
        assert!(second.iter().all(|l| l == NO_CHANGES), "{second:?}");
    }

    #[test]
    fn remove_takes_ours_and_foreign_hooks_survive_both() {
        let (c, path) = scratch("foreign");
        std::fs::write(
            &path,
            r#"{"version":1,"hooks":{"PreToolUse":[{"matcher":"exec","hooks":[{"type":"command","command":"echo foreign","timeout":30}]}],"Stop":[{"hooks":[{"type":"command","command":"echo stop","timeout":9}]}]}}"#,
        )
        .unwrap();
        std::fs::write(
            path.parent().unwrap().join("mcp_config.json"),
            r#"{"mcpServers":{"other":{"command":"other","args":[]}}}"#,
        )
        .unwrap();
        install(&c);
        Devin.apply(&c, Kind::Cli, Mode::Remove).unwrap();
        let hooks: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let text = hooks.to_string();
        assert!(text.contains("echo foreign"), "{text}");
        assert!(text.contains("echo stop"), "{text}");
        assert!(!text.contains("rtok"), "{text}");
        assert_eq!(hooks["version"], 1);
        let mcp: Value = serde_json::from_str(
            &std::fs::read_to_string(path.parent().unwrap().join("mcp_config.json")).unwrap(),
        )
        .unwrap();
        assert!(mcp["mcpServers"].get("rtok").is_none(), "{mcp}");
        assert!(mcp["mcpServers"].get("other").is_some(), "{mcp}");
    }

    #[test]
    fn plugin_manifest_matches_the_installer() {
        let file: Value =
            serde_json::from_str(include_str!("../../../plugins/devin/hooks.json")).unwrap();
        assert_eq!(hooks_doc(), file);
    }

    #[test]
    fn installed_never_reports_plugin() {
        let (c, _) = scratch("plugin");
        install(&c);
        let found = Devin.installed(&c, Kind::Cli);
        assert!(
            found.contains(&"hooks") && found.contains(&"mcp"),
            "{found:?}"
        );
        assert!(!found.contains(&"plugin"), "{found:?}");
    }
}
