// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Gemini CLI installer (`rtok agents install gemini`, plan T118.2).
//!
//! Gemini CLI reads one file, `[setup.gemini] dir`/`settings.json` (default
//! `~/.gemini/settings.json`, https://geminicli.com/docs/cli/settings/, fetched 2026-09-24):
//! hooks live under `hooks.<Event>[]` — Gemini's own event names, the reverse of
//! `hooks::types::gemini_event` (T118.1) — as `{hooks: [{type: "command", command, timeout}]}`,
//! and `mcpServers.<name>` sits beside it (https://geminicli.com/docs/tools/mcp-server/). No
//! `matcher` is written: Gemini's matcher filters on its *own* tool-name spelling
//! (`run_shell_command`, `read_file`, …), so a Claude-shaped matcher list (`Bash`, `Read`)
//! would silently never fire — every event runs on every call instead, the way `PostToolUse`
//! already does on every other host. The extension tree (`plugins/gemini/`, T118.3) is a D21
//! singleton with this module for hooks only: while `gemini extensions link` has it installed,
//! setup takes back the hooks this module would otherwise write directly into `settings.json`
//! (mirrors `src/agents/copilot/mod.rs`). MCP is the T275/D33 exception: `mcpServers.rtok` is
//! written into `settings.json` on every install/update regardless of the extension, because
//! Gemini's `settings.json` wins over the extension's same-name server (both stay `rtok`, so
//! they merge into one live process rather than racing two).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use rtok_agent_sdk::{Apply, NO_CHANGES, array_at, edit_json, object_at};
use rtok_mcp::spec::{Client, EntryShape, McpSpec};
use serde_json::{Value, json};

use super::hook_events;
use super::{Agent, Kind, Mode, Support, Variant, apply};
use crate::config::Config;

const NAME: &str = "rtok";
const PLUGIN_SRC: &str = "plugins/gemini";

/// Gemini CLI: one `settings.json`, no separate desktop app.
pub struct Gemini;

static VARIANTS: [Variant; 1] = [Variant {
    kind: Kind::Cli,
    name: "Gemini CLI",
    bins: &["gemini"],
    apps: &[],
}];

/// `<dir>/settings.json`.
pub fn settings_path(cfg: &Config) -> PathBuf {
    cfg.setup.gemini.dir.join("settings.json")
}

fn command(bin: &str, claude_event: &str) -> String {
    format!("{bin} hook {claude_event} --host gemini")
}

/// Exactly `<rtok-bin> hook <event> --host gemini`, matching only the tail so an absolute
/// path (Windows) still counts.
fn is_ours(cmd: &str, claude_event: &str) -> bool {
    let suffix = format!(" hook {claude_event} --host gemini");
    cmd.strip_suffix(&suffix)
        .is_some_and(|bin| super::is_rtok_bin(super::unquote_bin(bin)))
}

/// Apply, dry-run, or remove rtok's `hooks.<Event>[]` entries; foreign entries survive.
pub fn run(cfg: &Config, remove: bool) -> Result<String> {
    let (a, path) = (apply(cfg), settings_path(cfg));
    edit_json(&a, &path, |root| {
        let hooks = object_at(root, "hooks");
        if remove {
            strip_ours(&a, &path, hooks, cfg.setup.hook_timeout_s)
        } else {
            insert_ours(hooks, &super::rtok_hook_bin(), cfg.setup.hook_timeout_s)
        }
    })
}

fn insert_ours(hooks: &mut Value, bin: &str, timeout_s: u64) -> String {
    let mut added = Vec::new();
    let mut updated = 0usize;
    for (gevent, cevent) in hook_events::installed("gemini") {
        let cmd = command(bin, cevent);
        let ms = timeout_s * 1000;
        // T242.5: an rtok hook on another binary path or timeout is rewritten in its slot.
        let mut found = false;
        for entry in array_at(hooks, gevent).iter_mut() {
            for h in entry["hooks"].as_array_mut().into_iter().flatten() {
                if !h["command"].as_str().is_some_and(|c| is_ours(c, cevent)) {
                    continue;
                }
                found = true;
                if h["command"] != json!(cmd) || h["timeout"] != json!(ms) {
                    h["command"] = json!(cmd);
                    h["timeout"] = json!(ms);
                    added.push(format!("~ {gevent} {cmd}"));
                    updated += 1;
                }
            }
        }
        if found {
            continue;
        }
        let entry = json!({"hooks": [{"type": "command", "command": cmd, "timeout": ms}]});
        array_at(hooks, gevent).push(entry);
        added.push(format!("+ {gevent} {cmd}"));
    }
    if added.is_empty() {
        NO_CHANGES.into()
    } else {
        let counts = [(added.len() - updated, "additions"), (updated, "updates")]
            .iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, what)| format!("{n} {what}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{}\n{counts}", added.join("\n"))
    }
}

/// Remove rtok's hooks (T246.6); emptied entries and arrays go. One still as [`insert_ours`]
/// writes it — an entry of only `hooks`, the hook exactly `{type, command, timeout}` — goes;
/// one the user changed goes only as [`rtok_agent_sdk::keep_edited`] decides.
fn strip_ours(apply: &Apply, path: &Path, hooks: &mut Value, timeout_s: u64) -> String {
    let Some(obj) = hooks.as_object_mut() else {
        return NO_CHANGES.into();
    };
    let (mut removed, mut kept) = (0usize, Vec::new());
    for (gevent, cevent) in hook_events::installed("gemini") {
        let Some(arr) = obj.get_mut(gevent).and_then(Value::as_array_mut) else {
            continue;
        };
        for entry in arr.iter_mut() {
            let bare = entry.as_object().is_some_and(|o| o.len() == 1);
            let Some(inner) = entry.get_mut("hooks").and_then(Value::as_array_mut) else {
                continue;
            };
            inner.retain(|h| {
                let Some(cmd) = h["command"].as_str().filter(|c| is_ours(c, cevent)) else {
                    return true;
                };
                let want = json!({"type": "command", "command": cmd, "timeout": timeout_s * 1000});
                let at = || format!("hooks.{gevent} in {}", path.display());
                let take = super::takes_hook(apply, bare && *h == want, at, &mut kept);
                removed += usize::from(take);
                !take
            });
        }
        arr.retain(|e| {
            e.get("hooks")
                .and_then(Value::as_array)
                .is_none_or(|a| !a.is_empty())
        });
    }
    obj.retain(|_, v| v.as_array().is_none_or(|a| !a.is_empty()));
    super::with_kept(kept, super::removed_report(removed))
}

/// `mcpServers.rtok = {command, args}` in `settings.json` — the minimal stdio shape the Gemini
/// MCP docs show; `rtok_mcp::ops` writes and removes it (T277.2).
pub(crate) fn mcp_spec(cfg: &Config) -> McpSpec {
    super::mcp::json_spec(
        "gemini",
        Client::Cli,
        settings_path(cfg),
        "mcpServers",
        super::rtok_mcp_command(),
        EntryShape::Command,
    )
}

/// `{hooks: {<Event>: [{hooks: [{type: "command", command, timeout}]}]}}` — the extension
/// tree's own `hooks/hooks.json` (T118.3), built from the same [`hook_events`] table `run` merges
/// into `settings.json`, so the two surfaces never drift apart. Pinned by
/// `tests/gemini_plugin.rs`; no `matcher`, same reason as `insert_ours`.
pub fn hooks_doc(bin: &str, timeout_s: u64) -> Value {
    let mut hooks = serde_json::Map::new();
    for (gevent, cevent) in hook_events::installed("gemini") {
        let cmd = command(bin, cevent);
        hooks.insert(
            gevent.into(),
            json!([{"hooks": [{"type": "command", "command": cmd, "timeout": timeout_s * 1000}]}]),
        );
    }
    json!({"hooks": hooks})
}

/// `<dir>/extensions` — where `gemini extensions link`/`install` put each extension
/// (https://geminicli.com/docs/extensions/reference/, fetched 2026-09-24).
fn extensions_dir(cfg: &Config) -> PathBuf {
    cfg.setup.gemini.dir.join("extensions")
}

/// The manifest of rtok's extension, whose `mcpServers` carries a second `rtok` entry next to
/// the one in `settings.json` (`rtok doctor` reads it, T331.10). Only the folder named after
/// the extension, where `gemini extensions link` puts it; a link under another name is not read.
pub(crate) fn plugin_manifest(cfg: &Config) -> PathBuf {
    extensions_dir(cfg).join(NAME).join("gemini-extension.json")
}

/// True while `gemini extensions` has rtok's extension linked or installed: some
/// `extensions/*/gemini-extension.json` names it — robust to whatever folder name the CLI
/// actually gives the link, since that is not pinned by the docs.
fn plugin_installed(cfg: &Config) -> bool {
    let Ok(entries) = fs::read_dir(extensions_dir(cfg)) else {
        return false;
    };
    entries
        .flatten()
        .any(|e| super::manifest_names(&e.path(), "gemini-extension.json", NAME))
}

/// One `gemini …` call; `GEMINI_CLI_HOME` only when `dir` is not the default.
fn gemini_cli(cfg: &Config, args: &[&str]) -> std::result::Result<(), String> {
    let dir = cfg.setup.gemini.dir.as_path();
    let default = super::home_dir().join(".gemini");
    let env = (dir != default).then_some(("GEMINI_CLI_HOME", dir));
    super::run_cli(cfg, "gemini", args, env)
}

/// Offer, install, or uninstall `plugins/gemini` through `gemini extensions` (T118.3): the
/// documented dev-style local link (`gemini extensions link <dir>`), uninstall by the
/// manifest's `name`. Behind `--yes` (`Support::Flag`) — rtok never writes
/// `~/.gemini/extensions/`, that store is Gemini's, so the flag never turns the printed line
/// into state (`installed()` reads the marker alone). A failing or missing `gemini` keeps the
/// offer open instead of failing the install: the settings-file hooks/MCP still go in.
/// Shares its skeleton with `copilot::plugin` through `super::offer_plugin` (D21).
fn plugin(cfg: &Config, remove: bool) -> Result<String> {
    let installed = plugin_installed(cfg);
    super::offer_plugin(
        cfg,
        remove,
        installed,
        super::PluginOffer {
            bin: "gemini",
            name: NAME,
            src_rel: PLUGIN_SRC,
            install_verb: &["extensions", "link"],
            uninstall_verb: &["extensions", "uninstall"],
        },
        |args| gemini_cli(cfg, args),
    )
}

impl Agent for Gemini {
    super::agent_identity!("gemini");

    fn support(&self, _kind: Kind, module: &str) -> Support {
        match module {
            "proxy" => Support::No(
                "Gemini CLI has no documented base-URL setting; its own HTTP_PROXY/HTTPS_PROXY covers MCP server transport only, not the model API",
            ),
            "hooks" | "mcp" => Support::Yes,
            "plugin" => Support::Flag("--yes"),
            _ => Support::No("unknown module"),
        }
    }

    fn plugin_surfaces(&self) -> &'static [rtok_plugin_sdk::Surface] {
        &[
            rtok_plugin_sdk::Surface::Mcp,
            rtok_plugin_sdk::Surface::Hook,
        ]
    }

    fn files(&self, cfg: &Config, _kind: Kind) -> Vec<PathBuf> {
        vec![settings_path(cfg)]
    }

    fn installed(&self, cfg: &Config, _kind: Kind) -> Vec<&'static str> {
        let text = super::read(&settings_path(cfg));
        let mut out = Vec::new();
        if text.contains(" hook PreToolUse --host gemini") {
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
        // D21: the extension is the hooks unit — its own hooks serve already, so rtok's
        // settings.json hook entries go instead of coming, on the same run that installs it
        // too (mirrors `src/agents/copilot/mod.rs`). MCP is the T275/D33 exception: settings.json
        // always gets `mcpServers.rtok` too, alongside the extension's own.
        super::d21_plugin_apply(
            cfg,
            mode,
            super::D21Plugin {
                offer: plugin,
                plugin_installed,
                run,
                register_mcp: |cfg| super::mcp::install(cfg, &mcp_spec(cfg)),
                unregister_mcp: |cfg| super::mcp::remove(cfg, &mcp_spec(cfg)),
            },
            super::no_extra,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn cfg(name: &str, dry: bool) -> (Config, PathBuf) {
        let dir = std::env::temp_dir().join(format!("rtok-gemini-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut c = Config::default();
        c.setup.gemini.dir = dir.clone();
        c.setup.dry_run = dry;
        c.setup.backup = false;
        (c, dir)
    }

    #[test]
    fn dry_run_names_six_events_and_creates_nothing() {
        let (c, dir) = cfg("dry", true);
        let out = run(&c, false).unwrap();
        assert!(
            out.starts_with("+ BeforeTool ") && out.contains("6 additions"),
            "{out}"
        );
        assert!(!settings_path(&c).exists());
        assert!(Gemini.installed(&c, Kind::Cli).is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn apply_is_idempotent_and_remove_keeps_foreign() {
        let (c, dir) = cfg("apply", false);
        fs::write(
            settings_path(&c),
            json!({
                "hooks": {"Notification": [{"hooks": [{"type": "command", "command": "echo hi"}]}]},
                "mcpServers": {"other": {"command": "npx", "args": ["x"]}}
            })
            .to_string(),
        )
        .unwrap();
        assert!(
            crate::agents::mcp::install(&c, &mcp_spec(&c))
                .unwrap()
                .starts_with("mcpServers.rtok: ")
        );
        assert!(run(&c, false).unwrap().contains("6 additions"));
        assert_eq!(
            crate::agents::mcp::install(&c, &mcp_spec(&c)).unwrap(),
            NO_CHANGES
        );
        assert_eq!(run(&c, false).unwrap(), NO_CHANGES);

        let raw_after_install = fs::read_to_string(settings_path(&c)).unwrap();
        let root = serde_json::from_str::<Value>(&raw_after_install).unwrap();
        let before = &root["hooks"]["BeforeTool"][0];
        assert!(before.get("matcher").is_none(), "{before}");
        let cmd = before["hooks"][0]["command"].as_str().unwrap();
        assert!(cmd.ends_with("rtok hook PreToolUse --host gemini"), "{cmd}");
        assert_eq!(before["hooks"][0]["timeout"], 5000);
        assert!(
            root["hooks"]["Notification"].is_array(),
            "foreign event stays"
        );
        assert_eq!(root["mcpServers"]["rtok"]["args"][0], "mcp");
        assert_eq!(root["mcpServers"]["other"]["command"], "npx");
        assert_eq!(Gemini.installed(&c, Kind::Cli), ["hooks", "mcp"]);

        assert!(run(&c, true).unwrap().contains("removed"));
        assert_eq!(
            crate::agents::mcp::remove(&c, &mcp_spec(&c)).unwrap(),
            "- mcpServers.rtok"
        );
        let raw = fs::read_to_string(settings_path(&c)).unwrap();
        assert!(!raw.contains("rtok hook"), "{raw}");
        let root: Value = serde_json::from_str(&raw).unwrap();
        assert!(
            root["hooks"]["Notification"].is_array(),
            "foreign event survives remove"
        );
        assert_eq!(root["mcpServers"]["other"]["command"], "npx");
        assert!(Gemini.installed(&c, Kind::Cli).is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn support_matches_hooks_mcp_yes_proxy_no_plugin_flag() {
        assert!(matches!(Gemini.support(Kind::Cli, "hooks"), Support::Yes));
        assert!(matches!(Gemini.support(Kind::Cli, "mcp"), Support::Yes));
        assert!(matches!(Gemini.support(Kind::Cli, "proxy"), Support::No(_)));
        assert!(matches!(
            Gemini.support(Kind::Cli, "plugin"),
            Support::Flag("--yes")
        ));
    }

    /// T275/D33 check (a)-(e): install and update always write `mcpServers.rtok` into
    /// `settings.json` — the extension linked or not — with the right command/args; only
    /// remove takes it out; a user-edited entry is left with a `leave` line instead of touched.
    /// Gemini is the exception host: unlike Copilot, hooks stay a D21 unit with the extension
    /// too, so hooks also go missing while it is linked — only MCP is unconditional here.
    #[test]
    fn t275_mcp_entry_always_written_except_on_remove() {
        let (c, dir) = cfg("t275-mcp", false);
        let manifest = extensions_dir(&c).join("rtok/gemini-extension.json");
        fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        fs::write(&manifest, r#"{"name":"rtok"}"#).unwrap();
        assert!(plugin_installed(&c));

        let path = settings_path(&c);
        crate::agents::mcp::assert_json_entry_lifecycle(
            &Gemini,
            &c,
            Kind::Cli,
            &path,
            "mcpServers",
            || crate::agents::mcp::install(&c, &mcp_spec(&c)),
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// T242.5: an rtok hook on another binary path and timeout is rewritten in its slot, a
    /// foreign hook in the same entry stays, and a second pass changes nothing.
    #[test]
    fn stale_rtok_hook_is_rewritten_in_place() {
        let (gevent, cevent) = hook_events::installed("gemini").next().unwrap();
        let stale = command("/old/store/rtok/v0.1.0/rtok", cevent);
        let mut hooks = json!({gevent: [{"matcher": "run_shell_command", "hooks": [
            {"type": "command", "command": "audit.sh"},
            {"type": "command", "command": stale, "timeout": 1000}
        ]}]});
        let out = insert_ours(&mut hooks, "rtok", 5);
        let want = command("rtok", cevent);
        assert!(out.contains(&format!("~ {gevent} {want}")), "{out}");
        assert!(out.ends_with(" updates"), "{out}");
        let entries = hooks[gevent].as_array().unwrap();
        assert_eq!(entries.len(), 1, "no second entry: {hooks}");
        assert_eq!(entries[0]["matcher"], "run_shell_command");
        assert_eq!(entries[0]["hooks"][0]["command"], "audit.sh");
        assert_eq!(entries[0]["hooks"][1]["command"], want.as_str());
        assert_eq!(entries[0]["hooks"][1]["timeout"], 5000);
        let current = hooks.clone();
        assert_eq!(insert_ours(&mut hooks, "rtok", 5), NO_CHANGES);
        assert_eq!(hooks, current);
    }

    /// T118.3: the extension tree's `hooks/hooks.json` is exactly what `settings.json` merges
    /// in per event, minus the foreign-event preservation a shared file needs — one map
    /// ([`hook_events`]), two surfaces (D21).
    #[test]
    fn hooks_doc_uses_the_shared_event_table() {
        let doc = hooks_doc("rtok", 5);
        assert_eq!(
            doc["hooks"].as_object().unwrap().len(),
            hook_events::installed("gemini").count()
        );
        let before = &doc["hooks"]["BeforeTool"][0];
        assert!(before.get("matcher").is_none(), "{before}");
        assert_eq!(
            before["hooks"][0]["command"],
            "rtok hook PreToolUse --host gemini"
        );
        assert_eq!(before["hooks"][0]["timeout"], 5000);
    }
}
