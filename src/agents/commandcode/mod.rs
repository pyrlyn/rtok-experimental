// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Command Code installer (`rtok agents install commandcode`).
//!
//! Command Code (`command-code`, alias `cmd`) keeps user state in `~/.commandcode`:
//! hooks merge into the `hooks` key of `settings.json` (same Claude-shaped schema —
//! `PreToolUse`/`PostToolUse`/`SessionStart` with `matcher` + `command`), MCP goes to
//! the user scope `mcp.json` (`mcpServers.<name>`, stdio `command`/`args`). Setup writes
//! both through the normal edit-JSON path, so foreign hooks and servers survive, and
//! `remove` takes back exactly rtok's own entries. Its commands run
//! `rtok hook <Event> --host commandcode`, which maps Command Code's tool names
//! (`shell_command`, `read_file`, `write_file`, `edit_file`) to Claude's.
//!
//! Evidence: https://commandcode.ai/docs/hooks, https://commandcode.ai/docs/mcp
//! (fetched 2026-09-23); `~/.commandcode/settings.json` and `~/.commandcode/mcp.json`
//! shapes verified on this machine.

use std::path::PathBuf;

use anyhow::Result;
use rtok_agent_sdk::{NO_CHANGES, array_at, edit_json, object_at};
use serde_json::{Value, json};

use super::hook_events;
use super::plugin::HostPlugin;
use super::{Agent, Kind, Mode, Support, Variant, apply};
use crate::config::Config;

/// Command Code: hooks + MCP in `~/.commandcode`, plus the linked plugin tree.
/// The CLI and the desktop app read the same user files.
pub struct CommandCode;

// CLI-only variant: the desktop app reads the same `~/.commandcode` files, but its
// bundle path is undocumented, so there is no `apps[]` entry to probe (the restart
// matrix requires every Desktop variant to name one).
static VARIANTS: [Variant; 1] = [Variant {
    kind: Kind::Cli,
    name: "Command Code CLI",
    bins: &["command-code", "cmd"],
    apps: &[],
}];

// The events and matchers are the `commandcode` rows of `hook_events`: Command Code matches
// `tool_display_name` (`SHELL`, `READ`, …), and lifecycle events carry no tool, so their rows
// have no matcher (one there never fires).

impl Agent for CommandCode {
    fn id(&self) -> &'static str {
        "commandcode"
    }

    fn variants(&self) -> &'static [Variant] {
        &VARIANTS
    }

    fn readme(&self) -> &'static str {
        include_str!("README.md")
    }

    fn shared(&self) -> bool {
        true
    }

    fn plugin_surfaces(&self) -> &'static [rtok_plugin_sdk::Surface] {
        // The linked tree registers no server of its own (`scripts/mcp.sh` is only the command
        // for a by-hand `cmd mcp add`): `mcpServers.rtok` comes from the installer alone (D33).
        &[rtok_plugin_sdk::Surface::Hook]
    }

    fn support(&self, _kind: Kind, module: &str) -> Support {
        match module {
            "hooks" | "mcp" | "plugin" => Support::Yes,
            _ => Support::No("Command Code has no documented base-URL override for the proxy"),
        }
    }

    fn files(&self, cfg: &Config, _kind: Kind) -> Vec<PathBuf> {
        vec![settings_path(cfg), mcp_path(cfg)]
    }

    fn markers(&self, cfg: &Config, kind: Kind) -> Vec<PathBuf> {
        let mut paths = self.files(cfg, kind);
        paths.push(plugin_dest(cfg));
        paths
    }

    fn installed(&self, cfg: &Config, _kind: Kind) -> Vec<&'static str> {
        let mut out = Vec::new();
        if super::read(&settings_path(cfg)).contains("rtok hook") {
            out.push("hooks");
        }
        // Only the config's own entry counts (T275/D33): the plugin never stands in for it.
        if super::mcp::has_entry(&mcp_path(cfg), "mcpServers", "rtok") {
            out.push("mcp");
        }
        if PLUGIN.ours(cfg) {
            out.push("plugin");
        }
        out
    }

    fn apply(&self, cfg: &Config, _kind: Kind, mode: Mode) -> Result<Vec<String>> {
        let remove = mode == Mode::Remove;
        let mut lines = vec![run(cfg, remove)?, PLUGIN.offer(cfg, remove)?];
        // MCP is independent of the plugin (T275/D33): written on every install and update,
        // taken back only by `remove`.
        if remove {
            lines.push(unregister_mcp(cfg)?);
        } else if cfg.setup.mcp {
            lines.push(register_mcp(cfg)?);
        }
        lines.push(super::skill::sync("commandcode", cfg, remove)?);
        Ok(lines)
    }
}

fn hook_cmd(event: &str) -> String {
    format!("{} hook {event} --host commandcode", super::rtok_hook_bin())
}

/// `~/.commandcode/settings.json` — merged under the `hooks` key, foreign entries survive.
pub fn settings_path(cfg: &Config) -> PathBuf {
    cfg.setup.commandcode.dir.join("settings.json")
}

/// `~/.commandcode/mcp.json` — the user scope (available across all projects).
pub fn mcp_path(cfg: &Config) -> PathBuf {
    cfg.setup.commandcode.dir.join("mcp.json")
}

/// Merge, dry-run, or strip rtok's hook entries in `settings.json`.
pub fn run(cfg: &Config, remove: bool) -> Result<String> {
    edit_json(&apply(cfg), &settings_path(cfg), |root| {
        if remove {
            strip_ours(root)
        } else {
            insert_ours(root)
        }
    })
}

/// Register `rtok mcp` in the user-scope `mcp.json`.
pub fn register_mcp(cfg: &Config) -> Result<String> {
    let cmd = super::rtok_command();
    let entry = json!({"command": cmd, "args": super::mcp_args("commandcode")});
    rtok_agent_sdk::register_server(
        &apply(cfg),
        &mcp_path(cfg),
        "mcpServers",
        "rtok",
        entry,
        &super::mcp_summary(&cmd, "commandcode"),
    )
}

/// Drop `mcpServers.rtok` from `mcp.json`, unless the user edited it (T246).
pub fn unregister_mcp(cfg: &Config) -> Result<String> {
    super::unregister_mcp_ours(cfg, &mcp_path(cfg), "rtok")
}

/// The linked Command Code plugin tree (D21). Dry-run and the unaccepted offer MUST
/// contain the substrings `plugins/commandcode` and `ketch install pyrlyn/rtok`.
pub static PLUGIN: HostPlugin = HostPlugin {
    src_rel: "plugins/commandcode",
    host: "Command Code",
    label: None,
    dest: plugin_dest,
    default_install: false,
};

/// Command Code has no documented local-plugin directory, so the plugin is an offer:
/// linked next to the user files when accepted.
pub fn plugin_dest(cfg: &Config) -> PathBuf {
    cfg.setup.commandcode.dir.join("plugins").join("rtok")
}

fn insert_ours(root: &mut Value) -> String {
    let hooks = object_at(root, "hooks");
    let mut added = Vec::new();
    for row in hook_events::installer_rows("commandcode") {
        let (event, claude) = (row.host_event, row.rtok_event);
        let matcher = (!row.matcher.is_empty()).then_some(row.matcher);
        let cmd = hook_cmd(claude);
        let arr = array_at(hooks, event);
        // One definition per matcher: Command Code matches `tool_display_name`.
        let def = match matcher {
            Some(m) => arr
                .iter_mut()
                .find(|e| e.get("matcher").and_then(Value::as_str) == Some(m)),
            None => arr.iter_mut().find(|e| e.get("matcher").is_none()),
        };
        let def = match def {
            Some(d) => d,
            None => {
                arr.push(match matcher {
                    Some(m) => json!({"matcher": m, "hooks": []}),
                    None => json!({"hooks": []}),
                });
                arr.last_mut().expect("just pushed")
            }
        };
        let entries = array_at(def, "hooks");
        if !entries.iter().any(|e| is_cmd(e, &cmd)) {
            entries.push(json!({"type": "command", "command": cmd, "timeout": 5}));
            added.push(format!("+ {event} {cmd}"));
        }
    }
    if added.is_empty() {
        NO_CHANGES.into()
    } else {
        added.join("\n")
    }
}

fn strip_ours(root: &mut Value) -> String {
    let mut removed = Vec::new();
    for (event, _) in hook_events::installed("commandcode") {
        let Some(arr) = root
            .pointer_mut(&format!("/hooks/{event}"))
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        for def in arr.iter_mut() {
            let Some(entries) = def.get_mut("hooks").and_then(Value::as_array_mut) else {
                continue;
            };
            let before = entries.len();
            entries.retain(|e| !is_ours(e));
            if entries.len() != before {
                removed.push(format!("- {event}"));
                break;
            }
        }
        // Drop definitions left with no hooks behind (but keep foreign ones).
        arr.retain(|def| {
            def.get("hooks")
                .and_then(Value::as_array)
                .is_none_or(|h| !h.is_empty())
        });
    }
    // Drop the `hooks` key itself when nothing is left (keeps `installed()` honest).
    let empty = root
        .get("hooks")
        .and_then(Value::as_object)
        .is_some_and(|h| h.is_empty());
    if empty {
        root.as_object_mut()
            .expect("object_at made it an object")
            .remove("hooks");
    }
    if removed.is_empty() {
        NO_CHANGES.into()
    } else {
        removed.join("\n")
    }
}

fn is_cmd(entry: &Value, cmd: &str) -> bool {
    entry.get("command").and_then(Value::as_str) == Some(cmd)
}

fn is_ours(entry: &Value) -> bool {
    let Some(cmd) = entry.get("command").and_then(Value::as_str) else {
        return false;
    };
    // The events `insert_ours` writes (`rtok hook <Claude event> --host commandcode`).
    for (_, claude) in hook_events::installed("commandcode") {
        let suffix = format!(" hook {claude} --host commandcode");
        if let Some(bin) = cmd.strip_suffix(&suffix)
            && super::is_rtok_bin(super::unquote_bin(bin))
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{dispatch, types::HookInput};
    use crate::plugin::Runtime;
    use serde_json::json;
    use std::fs;

    fn tmp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rtok-commandcode-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cfg(dir: PathBuf, dry: bool) -> Config {
        let mut c = Config::default();
        c.setup.commandcode.dir = dir;
        c.setup.dry_run = dry;
        c.setup.backup = false;
        c
    }

    #[test]
    fn dry_run_names_the_settings_file_and_writes_nothing() {
        let dir = tmp("dry");
        let c = cfg(dir.clone(), true);
        let out = run(&c, false).unwrap();
        assert!(out.contains("PreToolUse"), "{out}");
        assert!(out.contains("--host commandcode"), "{out}");
        assert!(!settings_path(&c).exists());
        assert!(CommandCode.installed(&c, Kind::Cli).is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn apply_writes_four_events_is_idempotent_and_remove_strips() {
        let dir = tmp("apply");
        let c = cfg(dir.clone(), false);
        assert!(run(&c, false).unwrap().contains("+ PreToolUse"));
        assert_eq!(run(&c, false).unwrap(), NO_CHANGES);
        let doc: Value =
            serde_json::from_str(&fs::read_to_string(settings_path(&c)).unwrap()).unwrap();
        for event in ["PreToolUse", "PostToolUse", "SessionStart", "Stop"] {
            let defs = doc["hooks"][event].as_array().unwrap();
            assert!(
                defs.iter()
                    .any(|d| d["hooks"]
                        .as_array()
                        .is_some_and(|h| h.iter().any(|e| e["command"]
                            .as_str()
                            .is_some_and(|s| s.contains("--host commandcode"))))),
                "{event}: {doc}"
            );
        }
        // Lifecycle events carry no matcher (a matcher there never fires).
        assert!(doc["hooks"]["SessionStart"][0].get("matcher").is_none());
        assert!(doc["hooks"]["Stop"][0].get("matcher").is_none());
        assert_eq!(
            doc["hooks"]["PreToolUse"][0]["matcher"],
            "SHELL|READ|WRITE|EDIT"
        );
        assert!(CommandCode.installed(&c, Kind::Cli).contains(&"hooks"));

        let gone = run(&c, true).unwrap();
        assert!(gone.contains("- PreToolUse"), "{gone}");
        let left = fs::read_to_string(settings_path(&c)).unwrap();
        assert!(!left.contains("rtok hook"), "{left}");
        assert_eq!(run(&c, true).unwrap(), NO_CHANGES);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn foreign_hooks_survive_install_and_remove() {
        let dir = tmp("foreign");
        let c = cfg(dir.clone(), false);
        fs::write(
            settings_path(&c),
            r#"{"hooks":{"PreToolUse":[{"matcher":".*","hooks":[{"type":"command","command":"other-guard"}]}]}}"#,
        )
        .unwrap();
        run(&c, false).unwrap();
        let doc: Value =
            serde_json::from_str(&fs::read_to_string(settings_path(&c)).unwrap()).unwrap();
        let pre: Vec<String> = doc["hooks"]["PreToolUse"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|d| d["hooks"].as_array().cloned().unwrap_or_default())
            .filter_map(|e| e["command"].as_str().map(str::to_string))
            .collect();
        assert!(pre.iter().any(|s| s == "other-guard"), "{pre:?}");
        assert!(
            pre.iter().any(|s| s.contains("--host commandcode")),
            "{pre:?}"
        );
        run(&c, true).unwrap();
        let left: Value =
            serde_json::from_str(&fs::read_to_string(settings_path(&c)).unwrap()).unwrap();
        let pre: Vec<String> = left["hooks"]["PreToolUse"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|d| d["hooks"].as_array().cloned().unwrap_or_default())
            .filter_map(|e| e["command"].as_str().map(str::to_string))
            .collect();
        assert_eq!(pre, vec!["other-guard".to_string()]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn mcp_entry_registers_and_unregisters_keeping_foreign() {
        let dir = tmp("mcp");
        let c = cfg(dir.clone(), false);
        fs::write(mcp_path(&c), r#"{"mcpServers":{"other":{"command":"x"}}}"#).unwrap();
        let first = register_mcp(&c).unwrap();
        assert!(first.starts_with("mcpServers.rtok: "), "{first}");
        assert_eq!(register_mcp(&c).unwrap(), NO_CHANGES);
        let doc: Value = serde_json::from_str(&fs::read_to_string(mcp_path(&c)).unwrap()).unwrap();
        assert_eq!(
            doc["mcpServers"]["rtok"]["args"],
            json!(["mcp", "--host", "commandcode"])
        );
        assert_eq!(doc["mcpServers"]["other"]["command"], "x");
        assert!(CommandCode.installed(&c, Kind::Cli).contains(&"mcp"));
        assert_eq!(unregister_mcp(&c).unwrap(), "- mcpServers.rtok");
        assert!(!fs::read_to_string(mcp_path(&c)).unwrap().contains("rtok"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn dry_run_offer_names_plugin_and_ketch_and_writes_nothing() {
        let dir = tmp("offer-dry");
        let c = cfg(dir.clone(), true);
        let s = PLUGIN.offer(&c, false).unwrap();
        assert!(s.contains("plugins/commandcode"), "{s}");
        assert!(s.contains("ketch install pyrlyn/rtok"), "{s}");
        assert!(!plugin_dest(&c).exists());
        let _ = fs::remove_dir_all(dir);
    }

    fn mcp_doc(c: &Config) -> Value {
        serde_json::from_str(&fs::read_to_string(mcp_path(c)).unwrap()).unwrap()
    }

    /// T275.2/D33: a linked plugin neither suppresses nor strips `mcpServers.rtok` — install and
    /// update both leave the entry, with the host's own args, next to a foreign server.
    #[test]
    fn linked_plugin_install_and_update_keep_the_mcp_entry() {
        let dir = tmp("plugin-mcp");
        let mut c = cfg(dir.clone(), false);
        c.setup.yes = true;
        fs::write(mcp_path(&c), r#"{"mcpServers":{"other":{"command":"x"}}}"#).unwrap();
        CommandCode.apply(&c, Kind::Cli, Mode::Install).unwrap();
        assert!(PLUGIN.linked(&c));
        assert_eq!(
            mcp_doc(&c)["mcpServers"]["rtok"]["args"],
            json!(["mcp", "--host", "commandcode"])
        );
        assert_eq!(mcp_doc(&c)["mcpServers"]["other"]["command"], "x");

        // The update path rewrites a stale entry while the plugin stays linked.
        fs::write(
            mcp_path(&c),
            r#"{"mcpServers":{"rtok":{"command":"rtok","args":["mcp"]}}}"#,
        )
        .unwrap();
        CommandCode.apply(&c, Kind::Cli, Mode::Update).unwrap();
        assert!(PLUGIN.linked(&c));
        assert_eq!(
            mcp_doc(&c)["mcpServers"]["rtok"]["args"],
            json!(["mcp", "--host", "commandcode"])
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn remove_takes_the_entry_out_even_with_the_plugin_linked() {
        let dir = tmp("plugin-mcp-remove");
        let mut c = cfg(dir.clone(), false);
        c.setup.yes = true;
        CommandCode.apply(&c, Kind::Cli, Mode::Install).unwrap();
        assert!(CommandCode.installed(&c, Kind::Cli).contains(&"mcp"));
        CommandCode.apply(&c, Kind::Cli, Mode::Remove).unwrap();
        assert!(!CommandCode.installed(&c, Kind::Cli).contains(&"mcp"));
        assert!(!fs::read_to_string(mcp_path(&c)).unwrap().contains("rtok"));
        let _ = fs::remove_dir_all(dir);
    }

    /// The plugin alone never reads as an installed `mcp` module: only the config's entry does.
    #[test]
    fn plugin_without_a_config_entry_does_not_read_as_mcp() {
        let dir = tmp("plugin-only");
        let mut c = cfg(dir.clone(), false);
        c.setup.yes = true;
        c.setup.mcp = false;
        CommandCode.apply(&c, Kind::Cli, Mode::Install).unwrap();
        let have = CommandCode.installed(&c, Kind::Cli);
        assert!(have.contains(&"plugin"), "{have:?}");
        assert!(!have.contains(&"mcp"), "{have:?}");
        assert!(!mcp_path(&c).exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_user_edited_entry_is_left_by_remove() {
        let dir = tmp("edited");
        let c = cfg(dir.clone(), false);
        fs::write(
            mcp_path(&c),
            r#"{"mcpServers":{"rtok":{"command":"/opt/mine/server","args":["serve"]}}}"#,
        )
        .unwrap();
        let line = unregister_mcp(&c).unwrap();
        assert!(line.starts_with("leave "), "{line}");
        assert_eq!(
            mcp_doc(&c)["mcpServers"]["rtok"]["command"],
            "/opt/mine/server"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn commandcode_shell_payload_reaches_pre_tool_use() {
        let dir = tmp("pre");
        let mut c = Config::default();
        c.hook.host = "commandcode".into();
        c.core.db_path = dir.join("rtok.db");
        c.core.archive_dir = dir.join("archive");
        let cx = Runtime::open(c, "cc").unwrap();
        let raw = json!({
            "session_id": "cc-1",
            "transcript_path": "/tmp/t.jsonl",
            "cwd": "/tmp",
            "hook_event_name": "PreToolUse",
            "permission_mode": "default",
            "tool_name": "shell_command",
            "tool_input": {"command": "ls -la"}
        });
        let bytes = serde_json::to_vec(&raw).unwrap();
        let mut input: HookInput = serde_json::from_slice(&bytes).unwrap();
        input.adapt_commandcode("PreToolUse", None);
        let out = dispatch(&bytes, &input, &cx);
        let v: Value = serde_json::from_slice(&out).unwrap();
        let cmd = v["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap_or("");
        assert!(cmd.contains("rtok run --") && cmd.contains("ls -la"), "{v}");
        let _ = fs::remove_dir_all(dir);
    }
}
