// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Kimi Code CLI + Desktop (`rtok agents install kimi`, plan T46.2, T86).
//!
//! Moonshot's Kimi Code CLI reads hooks as `[[hooks]]` tables in `~/.kimi-code/config.toml`
//! (`event`, `matcher`, `command`, `timeout` in seconds; Claude-compatible stdin, exit 2
//! blocks) and MCP from the sibling `mcp.json` (`mcpServers.<name> = {command, args}`, no
//! `type`). The TOML goes through `toml_edit` so the user's comments and other hooks survive.
//!
//! T86 (plugin offer + D21 singleton): Kimi owns its plugin store
//! (`<kimi home>/plugins/managed/`, no local directory to link), so install never writes
//! there — it prints the exact `/plugins install <resolved plugins/kimi path>` line
//! (dry-run and apply alike). While the plugin is installed
//! (`<kimi home>/plugins/managed/rtok/kimi.plugin.json` exists), setup strips rtok's own
//! `[[hooks]]` tables and `mcpServers.rtok` instead of adding them, so no event fires
//! twice and one `rtok mcp` serves the store.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rtok_agent_sdk::NO_CHANGES;
use rtok_mcp::spec::{Client, EntryShape, McpSpec};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

use super::claude::{is_ours, show};
use super::hook_events;
use super::{Agent, Kind, Mode, Support, Variant, apply};
use crate::config::Config;

const NAME: &str = "rtok";

/// Kimi Code CLI + Desktop: `[[hooks]]` in `config.toml`, `mcpServers.rtok` in `mcp.json`.
/// The desktop app (`Kimi Code.app`) manages the same files as the CLI.
pub struct Kimi;

/// CLI (`kimi` on PATH) and Desktop (`Kimi Code.app`): one install writes the
/// same two files (T86).
static VARIANTS: [Variant; 2] = [
    Variant {
        kind: Kind::Cli,
        name: "Kimi Code",
        bins: &["kimi"],
        apps: &[],
    },
    Variant {
        kind: Kind::Desktop,
        name: "Kimi Code Desktop",
        bins: &[],
        apps: &[
            "/Applications/Kimi Code.app",
            "$LOCALAPPDATA/Programs/Kimi Code/Kimi Code.exe",
        ],
    },
];

/// `mcp.json` lives beside `config.toml`; one key configures both.
pub fn mcp_path(cfg: &Config) -> PathBuf {
    cfg.setup
        .kimi
        .config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("mcp.json")
}

impl Agent for Kimi {
    super::agent_identity!("kimi");

    fn support(&self, _kind: Kind, module: &str) -> Support {
        match module {
            "hooks" | "mcp" => Support::Yes,
            "plugin" => Support::Offer("--yes"),
            "proxy" => Support::No(
                "Kimi Code providers are [providers.<name>] tables with their own base_url and keys; setup does not edit them",
            ),
            _ => Support::No(
                "Kimi Code plugins live in plugins/managed/, owned by `kimi plugin install`; there is no local plugin directory to link",
            ),
        }
    }

    fn plugin_surfaces(&self) -> &'static [rtok_plugin_sdk::Surface] {
        &[rtok_plugin_sdk::Surface::Hook]
    }

    fn shared(&self) -> bool {
        true
    }

    fn files(&self, cfg: &Config, _kind: Kind) -> Vec<PathBuf> {
        vec![cfg.setup.kimi.config_path.clone(), mcp_path(cfg)]
    }

    fn installed(&self, cfg: &Config, _kind: Kind) -> Vec<&'static str> {
        let mut out = Vec::new();
        if super::read(&cfg.setup.kimi.config_path).contains("rtok hook") {
            out.push("hooks");
        }
        if super::mcp::has_entry(&mcp_path(cfg), "mcpServers", NAME) {
            out.push("mcp");
        }
        if plugin_detected(cfg) {
            out.push("plugin");
        }
        out
    }

    fn apply(&self, cfg: &Config, _kind: Kind, mode: Mode) -> Result<Vec<String>> {
        let remove = mode == Mode::Remove;
        if remove {
            let mut lines = vec![offer_plugin(cfg, true)?];
            lines.push(run(cfg, true)?);
            lines.extend(super::mcp::sync(cfg, &mcp_spec(cfg), mode)?);
            return Ok(lines);
        }
        let plugin = plugin_detected(cfg);
        // D21 singleton: while the plugin is installed it is the only call path for hooks,
        // so rtok's own `[[hooks]]` tables are stripped instead of added. MCP is
        // independent of the plugin (T275/D33): install/update always write
        // `mcpServers.rtok` regardless of plugin state; only remove takes it out.
        let mut lines = vec![run(cfg, plugin)?];
        lines.extend(super::mcp::sync(cfg, &mcp_spec(cfg), mode)?);
        // The offer line goes first unconditionally once the plugin is already detected;
        // otherwise it follows only when something else changed and `--yes` is set — a
        // repeat install is all `NO_CHANGES` and reads back as `already installed` (every
        // line counts, including guidance).
        if plugin || (lines.iter().any(|l| l != NO_CHANGES) && cfg.setup.yes) {
            lines.insert(0, offer_plugin(cfg, false)?);
        } else {
            lines.insert(0, NO_CHANGES.into());
        }
        Ok(lines)
    }
}

/// The plugin marker rtok can honestly read (T86): the managed copy Kimi
/// writes on `/plugins install <dir>` — `<kimi home>/plugins/managed/rtok/`,
/// where `<kimi home>` is the dir holding `config.toml`.
pub fn plugin_marker(cfg: &Config) -> PathBuf {
    cfg.setup
        .kimi
        .config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("plugins")
        .join("managed")
        .join("rtok")
        .join("kimi.plugin.json")
}

/// True while Kimi's own install of the plugin serves hooks and MCP (D21):
/// setup then strips its own tables instead of adding them.
pub fn plugin_detected(cfg: &Config) -> bool {
    plugin_marker(cfg).is_file()
}

/// The `/plugins install <resolved plugins/kimi path>` line (T86, [`super::print_offer`]):
/// rtok never writes `plugins/managed/` or `installed.json` — that format is Kimi's and
/// undocumented.
pub fn offer_plugin(cfg: &Config, remove: bool) -> Result<String> {
    Ok(super::print_offer(
        cfg,
        remove,
        plugin_detected(cfg),
        "plugins/kimi",
        &format!(
            "/plugins install {}",
            super::plugin_src("plugins/kimi").display()
        ),
        "keep plugins/managed/rtok (owned by Kimi; remove with `/plugins remove rtok`)",
    ))
}

/// Apply, dry-run, or remove the `[[hooks]]` tables.
pub fn run(cfg: &Config, remove: bool) -> Result<String> {
    let path = &cfg.setup.kimi.config_path;
    let mut doc = load(path)?;
    let report = if remove {
        strip_ours(&apply(cfg), path, &mut doc, cfg.setup.hook_timeout_s)
    } else {
        insert_ours(&mut doc, cfg.setup.hook_timeout_s)?
    };
    rtok_agent_sdk::write(&apply(cfg), path, &doc.to_string(), &report)?;
    Ok(report)
}

/// `mcpServers.rtok = {command, args}` in `mcp.json` — Kimi's documented shape carries no
/// `type`; `rtok_mcp::ops` writes and removes it (T277.2).
pub(crate) fn mcp_spec(cfg: &Config) -> McpSpec {
    super::mcp::json_spec(
        "kimi",
        Client::Cli,
        mcp_path(cfg),
        "mcpServers",
        super::rtok_mcp_command(),
        EntryShape::Command,
    )
}

/// An absent file is an empty document; an unreadable one is an error (never overwrite a
/// config that was not read).
fn load(path: &Path) -> Result<DocumentMut> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| path.display().to_string()),
    };
    raw.parse::<DocumentMut>()
        .with_context(|| path.display().to_string())
}

/// Kimi's `(event, matcher)` pairs: Claude's list minus `SubagentStart`, whose hook output Kimi
/// discards, and the MCP `read` matcher (Kimi's `updatedInput` is unverified).
fn entries() -> Vec<(&'static str, &'static str)> {
    hook_events::entries("kimi")
}

fn table_is_ours(t: &Table, event: &str, matcher: &str) -> bool {
    t.get("event").and_then(Item::as_str) == Some(event)
        && t.get("matcher").and_then(Item::as_str).unwrap_or("") == matcher
        && t.get("command")
            .and_then(Item::as_str)
            .is_some_and(|c| is_ours(c, event))
}

/// Add an rtok `[[hooks]]` table per [`entries`] pair and bring the existing ones up to date
/// (T242.5, T242.1's rule): an rtok table on another binary path or timeout is rewritten in
/// place, and one on a pair [`entries`] no longer lists (a changed matcher) is dropped. Foreign
/// tables are never touched.
fn insert_ours(doc: &mut DocumentMut, timeout: u64) -> Result<String> {
    let entries = entries();
    let hooks = doc
        .entry("hooks")
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    let Some(hooks) = hooks.as_array_of_tables_mut() else {
        bail!("`hooks` is not an array of tables");
    };
    let show = |m: &str| {
        if m.is_empty() {
            String::new()
        } else {
            format!(" {m}")
        }
    };
    let mut lines = Vec::new();
    hooks.retain(|t| {
        let event = t.get("event").and_then(Item::as_str).unwrap_or("");
        let matcher = t.get("matcher").and_then(Item::as_str).unwrap_or("");
        let ours = t
            .get("command")
            .and_then(Item::as_str)
            .is_some_and(|c| is_ours(c, event));
        let keep = !ours || entries.contains(&(event, matcher));
        if !keep {
            lines.push(format!("- [[hooks]] {event}{}", show(matcher)));
        }
        keep
    });
    let removed = lines.len();
    let bin = super::rtok_hook_bin();
    let (mut added, mut updated) = (0usize, 0usize);
    for &(event, matcher) in &entries {
        let cmd = format!("{bin} hook {event}");
        let mut found = false;
        for t in hooks
            .iter_mut()
            .filter(|t| table_is_ours(t, event, matcher))
        {
            found = true;
            if t.get("command").and_then(Item::as_str) != Some(cmd.as_str())
                || t.get("timeout").and_then(Item::as_integer) != Some(timeout as i64)
            {
                t["command"] = value(cmd.as_str());
                t["timeout"] = value(timeout as i64);
                lines.push(format!("~ [[hooks]] {event}{} {cmd}", show(matcher)));
                updated += 1;
            }
        }
        if found {
            continue;
        }
        let mut t = Table::new();
        t["event"] = value(event);
        if !matcher.is_empty() {
            t["matcher"] = value(matcher);
        }
        t["command"] = value(cmd.as_str());
        t["timeout"] = value(timeout as i64);
        hooks.push(t);
        lines.push(format!("+ [[hooks]] {event}{} {cmd}", show(matcher)));
        added += 1;
    }
    if lines.is_empty() {
        return Ok(NO_CHANGES.into());
    }
    let counts = [
        (added, "additions"),
        (updated, "updates"),
        (removed, "removals"),
    ]
    .iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect::<Vec<_>>()
    .join(", ");
    Ok(format!("{}\n{counts}", lines.join("\n")))
}

/// Remove rtok's `[[hooks]]` tables (T246.6). One still as [`insert_ours`] writes it — on a
/// pair [`entries`] lists, exactly `event`, `matcher` (when set), `command` and `timeout` — goes;
/// one the user changed goes only as [`rtok_agent_sdk::keep_edited`] decides.
fn strip_ours(
    apply: &rtok_agent_sdk::Apply,
    path: &Path,
    doc: &mut DocumentMut,
    timeout: u64,
) -> String {
    let Some(hooks) = doc.get_mut("hooks").and_then(Item::as_array_of_tables_mut) else {
        return NO_CHANGES.into();
    };
    let (mut removed, mut kept) = (0usize, Vec::new());
    hooks.retain(|t| {
        let event = t.get("event").and_then(Item::as_str).unwrap_or("");
        if !t
            .get("command")
            .and_then(Item::as_str)
            .is_some_and(|c| is_ours(c, event))
        {
            return true;
        }
        let matcher = t.get("matcher").and_then(Item::as_str).unwrap_or("");
        let keys = if t.contains_key("matcher") { 4 } else { 3 };
        let unchanged = entries().contains(&(event, matcher))
            && t.len() == keys
            && t.get("timeout").and_then(Item::as_integer) == Some(timeout as i64);
        let at = || format!("[[hooks]] {event}{} in {}", show(matcher), path.display());
        let take = super::takes_hook(apply, unchanged, at, &mut kept);
        removed += usize::from(take);
        !take
    });
    if hooks.is_empty() {
        doc.remove("hooks");
    }
    super::with_kept(kept, super::removed_report(removed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Shared fixture (the T86 tests below reuse it).
    fn cfg(name: &str, dry: bool) -> (Config, PathBuf) {
        let dir = std::env::temp_dir().join(format!("rtok-kimi-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let mut c = Config::default();
        c.setup.kimi.config_path = path.clone();
        c.setup.dry_run = dry;
        c.setup.backup = false;
        (c, path)
    }

    #[test]
    fn dry_run_names_nine_tables_and_touches_nothing() {
        let (c, path) = cfg("dry", true);
        fs::write(&path, "# mine\n").unwrap();
        let out = run(&c, false).unwrap();
        assert!(out.contains("9 additions"), "{out}");
        assert!(out.contains("+ [[hooks]] PreToolUse Bash "), "{out}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "# mine\n");
        assert!(!mcp_path(&c).exists());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn apply_keeps_comments_and_foreign_hooks_and_is_idempotent() {
        let (c, path) = cfg("apply", false);
        fs::write(
            &path,
            "# kimi config\nmodel = \"k2\"\n\n[[hooks]]\nevent = \"Stop\"\ncommand = \"echo other\"\n",
        )
        .unwrap();
        assert!(run(&c, false).unwrap().contains("9 additions"));
        assert_eq!(run(&c, false).unwrap(), NO_CHANGES);
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.starts_with("# kimi config\nmodel = \"k2\"\n"), "{raw}");
        assert!(raw.contains("command = \"echo other\""), "{raw}");
        let doc: DocumentMut = raw.parse().unwrap();
        let hooks = doc["hooks"].as_array_of_tables().unwrap();
        assert_eq!(hooks.len(), 10);
        let bash = hooks
            .iter()
            .find(|t| t.get("matcher").and_then(Item::as_str) == Some("Bash"))
            .unwrap();
        assert_eq!(bash["event"].as_str(), Some("PreToolUse"));
        assert_eq!(bash["timeout"].as_integer(), Some(5));
        assert!(
            bash["command"]
                .as_str()
                .unwrap()
                .ends_with("rtok hook PreToolUse")
        );
        assert_eq!(Kimi.installed(&c, Kind::Cli), ["hooks"]);

        assert_eq!(run(&c, true).unwrap(), "9 removed");
        assert_eq!(run(&c, true).unwrap(), NO_CHANGES);
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("echo other") && !raw.contains("rtok"), "{raw}");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    // --- Vfs twins (T56.3): TOML hook insert/strip in memory; keep disk e2e ---

    fn hooks_roundtrip_vfs(
        vfs: &mut crate::testutil::Vfs,
        path: &str,
        remove: bool,
        timeout: u64,
    ) -> String {
        let raw = vfs.read_str(path).unwrap_or("");
        let mut doc: DocumentMut = raw.parse().unwrap_or_default();
        let report = if remove {
            strip_ours(
                &rtok_agent_sdk::Apply::default(),
                Path::new(path),
                &mut doc,
                timeout,
            )
        } else {
            insert_ours(&mut doc, timeout).unwrap()
        };
        vfs.write(path, doc.to_string());
        report
    }

    #[test]
    fn dry_run_names_nine_tables_from_vfs() {
        let mut vfs = crate::testutil::Vfs::new();
        let path = "config.toml";
        vfs.write(path, "# mine\n");
        let before = vfs.read_str(path).unwrap().to_string();
        let out = {
            // Report-only: parse + insert without writing back (dry twin).
            let mut doc: DocumentMut = before.parse().unwrap();
            insert_ours(&mut doc, 5).unwrap()
        };
        assert!(out.contains("9 additions"), "{out}");
        assert!(out.contains("+ [[hooks]] PreToolUse Bash "), "{out}");
        assert_eq!(vfs.read_str(path).unwrap(), before);
    }

    #[test]
    fn apply_keeps_comments_and_foreign_hooks_from_vfs() {
        let mut vfs = crate::testutil::Vfs::new();
        let path = "config.toml";
        vfs.write(
            path,
            "# kimi config\nmodel = \"k2\"\n\n[[hooks]]\nevent = \"Stop\"\ncommand = \"echo other\"\n",
        );
        assert!(hooks_roundtrip_vfs(&mut vfs, path, false, 5).contains("9 additions"));
        assert_eq!(hooks_roundtrip_vfs(&mut vfs, path, false, 5), NO_CHANGES);
        let raw = vfs.read_str(path).unwrap();
        assert!(raw.starts_with("# kimi config\nmodel = \"k2\"\n"), "{raw}");
        assert!(raw.contains("command = \"echo other\""), "{raw}");
        let doc: DocumentMut = raw.parse().unwrap();
        let hooks = doc["hooks"].as_array_of_tables().unwrap();
        assert_eq!(hooks.len(), 10);
        assert_eq!(hooks_roundtrip_vfs(&mut vfs, path, true, 5), "9 removed");
        assert_eq!(hooks_roundtrip_vfs(&mut vfs, path, true, 5), NO_CHANGES);
        let raw = vfs.read_str(path).unwrap();
        assert!(raw.contains("echo other") && !raw.contains("rtok"), "{raw}");
    }

    #[test]
    fn spaced_profile_config_path_from_vfs() {
        let mut vfs = crate::testutil::Vfs::new();
        let path = "Users/Ivan Tuhai/.kimi-code/config.toml";
        vfs.write(path, "# mine\n");
        let out = hooks_roundtrip_vfs(&mut vfs, path, false, 5);
        assert!(out.contains("9 additions"), "{out}");
        assert!(vfs.read_str(path).unwrap().contains("rtok hook"));
    }

    #[test]
    fn mcp_lands_beside_the_config_without_a_type_field() {
        let (c, path) = cfg("mcp", false);
        let first = crate::agents::mcp::install(&c, &mcp_spec(&c)).unwrap();
        assert!(first.starts_with("mcpServers.rtok: "), "{first}");
        assert_eq!(
            crate::agents::mcp::install(&c, &mcp_spec(&c)).unwrap(),
            NO_CHANGES
        );
        assert_eq!(mcp_path(&c), path.parent().unwrap().join("mcp.json"));
        let raw = fs::read_to_string(mcp_path(&c)).unwrap();
        assert!(!raw.contains("\"type\""), "{raw}");
        assert!(raw.contains("\"mcp\""), "{raw}");
        assert_eq!(Kimi.installed(&c, Kind::Cli), ["mcp"]);
        assert_eq!(
            crate::agents::mcp::remove(&c, &mcp_spec(&c)).unwrap(),
            "- mcpServers.rtok"
        );
        assert!(!fs::read_to_string(mcp_path(&c)).unwrap().contains("rtok"));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// `plugins/kimi/kimi.plugin.json` (T85) is the installer's hooks in Kimi's plugin shape:
    /// same entries in the same order, same default timeout. A new `kimi` row of [`hook_events`] fails here
    /// until the manifest follows. It ships no MCP server of its own (T275/D33):
    /// `mcpServers.rtok` is written by `rtok agents install kimi` into `mcp.json` directly.
    #[test]
    fn plugin_manifest_matches_the_installer() {
        let m: serde_json::Value =
            serde_json::from_str(include_str!("../../../plugins/kimi/kimi.plugin.json")).unwrap();
        assert_eq!(m["name"], NAME);
        let timeout = Config::default().setup.hook_timeout_s;
        let want: Vec<_> = entries()
            .into_iter()
            .map(|(event, matcher)| {
                let mut h = json!({"event": event, "command": format!("rtok hook {event}"), "timeout": timeout});
                if !matcher.is_empty() {
                    h["matcher"] = json!(matcher);
                }
                h
            })
            .collect();
        assert_eq!(m["hooks"], json!(want));
        for h in m["hooks"].as_array().unwrap() {
            let (cmd, event) = (h["command"].as_str().unwrap(), h["event"].as_str().unwrap());
            assert!(is_ours(cmd, event), "{cmd}");
        }
        assert!(m.get("mcpServers").is_none(), "{m}");
    }

    /// T86: the offer names `plugins/kimi` and `/plugins install` (dry-run and
    /// apply alike) and writes nothing; Kimi's store is never touched by rtok.
    #[test]
    fn offer_names_the_plugin_path_and_ketch_hint() {
        let (mut c, dir) = cfg("offer-dry", true);
        // Without --yes the offer stays declined: NO_CHANGES, nothing written.
        assert_eq!(offer_plugin(&c, false).unwrap(), NO_CHANGES);
        assert!(!dir.join("plugins").exists(), "dry-run writes nothing");
        c.setup.yes = true;
        let line = offer_plugin(&c, false).unwrap();
        assert!(line.contains("plugins/kimi"), "{line}");
        assert!(line.contains("/plugins install"), "{line}");
        assert!(line.contains("ketch install pyrlyn/rtok"), "{line}");
        assert!(!dir.join("plugins").exists(), "dry-run writes nothing");
        c.setup.dry_run = false;
        let line = offer_plugin(&c, false).unwrap();
        assert!(line.contains("plugins/kimi"), "{line}");
        assert!(line.contains("/plugins install"), "{line}");
        assert!(!dir.join("plugins").exists(), "apply writes nothing either");
        let _ = fs::remove_dir_all(dir);
    }

    /// T86 D21 singleton: with a seeded `managed/rtok/kimi.plugin.json` a second install
    /// removes the nine `[[hooks]]` tables and reports `plugin`. MCP is independent of the
    /// plugin (T275/D33): `mcpServers.rtok` stays written regardless, and only remove takes
    /// it out.
    #[test]
    fn plugin_detected_strips_own_hooks_mcp_is_independent() {
        let (mut c, dir) = cfg("singleton", false);
        c.setup.yes = true;
        // Plain install first: hooks + MCP land in the user files; the `--yes`
        // offer prints alongside the changing run.
        let first = Kimi.apply(&c, Kind::Cli, Mode::Install).unwrap();
        assert_eq!(Kimi.installed(&c, Kind::Cli), ["hooks", "mcp"]);
        assert!(first[0].contains("/plugins install"), "{first:?}");
        // A repeat install changes nothing: every line is NO_CHANGES, so the
        // matrix reads it back as `already installed`.
        let repeat = Kimi.apply(&c, Kind::Cli, Mode::Install).unwrap();
        assert!(repeat.iter().all(|l| l == NO_CHANGES), "{repeat:?}");
        // Kimi installs the plugin: seed the managed copy it would write.
        let marker = plugin_marker(&c);
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::write(&marker, "{}").unwrap();
        let second = Kimi.apply(&c, Kind::Cli, Mode::Install).unwrap();
        assert_eq!(Kimi.installed(&c, Kind::Cli), ["mcp", "plugin"]);
        assert!(!super::super::read(&c.setup.kimi.config_path).contains("rtok hook"));
        assert!(
            super::super::read(&mcp_path(&c)).contains("\"rtok\""),
            "T275/D33: mcp is independent of the plugin"
        );
        assert!(second[0].contains("/plugins install"), "{second:?}");
        // Remove: own hooks already gone, mcpServers.rtok taken out, managed copy kept
        // with its own remove line.
        let rm = Kimi.apply(&c, Kind::Cli, Mode::Remove).unwrap();
        assert!(rm[0].contains("/plugins remove rtok"), "{rm:?}");
        assert!(marker.is_file(), "remove leaves the managed copy alone");
        assert_eq!(Kimi.installed(&c, Kind::Cli), ["plugin"]);
        let _ = fs::remove_dir_all(dir);
    }

    /// T86: remove on a clean home prints `NO_CHANGES` for the plugin line.
    #[test]
    fn remove_without_plugin_is_no_changes() {
        let (c, dir) = cfg("rm-clean", false);
        let rm = Kimi.apply(&c, Kind::Cli, Mode::Remove).unwrap();
        assert!(rm.iter().all(|l| l == NO_CHANGES), "{rm:?}");
        let _ = fs::remove_dir_all(dir);
    }

    /// T242.5: a stale rtok table (other binary path, other timeout) is rewritten in place, an
    /// rtok table on a pair rtok no longer installs is dropped, a foreign table stays, and a
    /// second pass changes nothing.
    #[test]
    fn stale_rtok_tables_are_rewritten_and_pruned() {
        let mut doc: DocumentMut = "[[hooks]]\nevent = \"PreToolUse\"\nmatcher = \"Bash\"\n\
command = \"/old/store/rtok/v0.1.0/rtok hook PreToolUse\"\ntimeout = 1\n\n\
[[hooks]]\nevent = \"PostToolUse\"\nmatcher = \"Bash\"\ncommand = \"rtok hook PostToolUse\"\n\n\
[[hooks]]\nevent = \"Stop\"\ncommand = \"echo other\"\n"
            .parse()
            .unwrap();
        let out = insert_ours(&mut doc, 5).unwrap();
        assert!(out.contains("- [[hooks]] PostToolUse Bash"), "{out}");
        assert!(out.ends_with("8 additions, 1 updates, 1 removals"), "{out}");
        let hooks = doc["hooks"].as_array_of_tables().unwrap();
        assert_eq!(hooks.len(), 10, "{doc}");
        let bash = hooks.get(0).unwrap();
        let bin = super::super::rtok_hook_bin();
        assert_eq!(
            bash["command"].as_str(),
            Some(format!("{bin} hook PreToolUse").as_str())
        );
        assert_eq!(bash["timeout"].as_integer(), Some(5));
        assert_eq!(
            hooks.get(1).unwrap()["command"].as_str(),
            Some("echo other")
        );
        let current = doc.to_string();
        assert_eq!(insert_ours(&mut doc, 5).unwrap(), NO_CHANGES);
        assert_eq!(doc.to_string(), current);
    }

    /// T275/D33: install/update always write `mcpServers.rtok`, plugin detected or not;
    /// only remove takes it out; a user-edited entry is left with a `leave` line.
    #[test]
    fn t275_mcp_entry_always_written_except_on_remove() {
        let (c, dir) = cfg("t275-mcp", false);
        let marker = plugin_marker(&c);
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::write(&marker, "{}").unwrap();
        assert!(plugin_detected(&c));

        let path = mcp_path(&c);
        crate::agents::mcp::assert_json_entry_lifecycle(
            &Kimi,
            &c,
            Kind::Cli,
            &path,
            "mcpServers",
            || crate::agents::mcp::install(&c, &mcp_spec(&c)),
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_entry_without_host_is_upgraded_and_still_removable() {
        let (c, cfg_path) = cfg("legacy-host", false);
        let path = mcp_path(&c);
        crate::agents::mcp::assert_legacy_entry_upgraded(
            &path,
            r#"{"mcpServers":{"other":{"command":"x"},"rtok":{"command":"rtok","args":["mcp"]}}}"#,
            "kimi",
            &["\"other\""],
            || crate::agents::mcp::install(&c, &mcp_spec(&c)),
            || crate::agents::mcp::remove(&c, &mcp_spec(&c)),
        );
        let _ = fs::remove_dir_all(cfg_path.parent().unwrap());
    }
}
