// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Codex installer (`rtok agents install codex`, plan T10.3).
//!
//! Codex reads MCP servers from `~/.codex/config.toml` as `[mcp_servers.<name>]`
//! tables with `command` and `args`, and lifecycle hooks from `hooks.json` (or
//! inline `[hooks]`). Compaction events `PreCompact`/`PostCompact` (T58.2) plus
//! MCP and proxy wiring (T11.5) are the install. Edits go through `toml_edit` so
//! the user's comments and other servers survive.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rtok_agent_sdk::{Apply, KETCH_INSTALL, NO_CHANGES, edit_json, object_at};
use serde_json::json;
use toml_edit::{Array, DocumentMut, Item, Table, value};

use super::{Agent, Kind, Mode, Support, Variant, apply};
use crate::config::Config;

const NAME: &str = "rtok";

/// Server-wide approval in `[mcp_servers.rtok]` (T528). Without it Codex asks before every
/// rtok call, and where nobody can answer (code mode, `approval_policy = "never"`) the call
/// fails. `"writes"` skips the ask only for tools whose `readOnlyHint` is true (`mcp::to_tool`)
/// and keeps it for the ones that change worktrees, tasks, notes or messages (openai/codex
/// `codex-rs/core/src/mcp_tool_call.rs` `requires_mcp_tool_approval_for_mode`, 78a19f5,
/// 2026-10-08). codex-cli 0.162.1 refuses the whole config on a value outside
/// `auto|prompt|writes|approve`, so the value is a constant, never user input.
const APPROVAL_KEY: &str = "default_tools_approval_mode";
const APPROVAL: &str = "writes";

const COMPACT: &[(&str, &str)] = &[("PreCompact", ""), ("PostCompact", "")];

/// The plugin tree (T121) and its id in the one-plugin marketplace `pyrlyn/rtok` also is.
const PLUGIN_SRC: &str = "plugins/codex";
const PLUGIN_ID: &str = "rtok@rtok";

/// GitHub `owner/repo` shorthand `codex plugin marketplace add` resolves (T140): the repo
/// root's `.agents/plugins/marketplace.json` names this one marketplace `rtok`, whose only
/// plugin is `./plugins/codex` (relative to the repo root, not the marketplace file).
const MARKETPLACE_REPO: &str = "pyrlyn/rtok";

/// What `codex plugin marketplace add {MARKETPLACE_REPO}` records in `~/.codex/config.toml`
/// under `[marketplaces.rtok]` — verified empirically against codex-cli 0.155.1 in a scratch
/// `CODEX_HOME` (2026-09-22): `source_type = "git"`, `source =
/// "https://github.com/pyrlyn/rtok.git"`.
const MARKETPLACE_GIT_SOURCE: &str = "https://github.com/pyrlyn/rtok.git";

/// Sibling of `config.toml`: `~/.codex/hooks.json` (Codex also reads inline `[hooks]`).
pub fn hooks_path(cfg: &Config) -> std::path::PathBuf {
    cfg.setup.codex.config_path.with_file_name("hooks.json")
}

/// Register `PreCompact` → `pre_compact` and `PostCompact` → `session_start` source=compact.
pub fn run_hooks(cfg: &Config, remove: bool) -> Result<String> {
    let (a, path) = (apply(cfg), hooks_path(cfg));
    edit_json(&a, &path, |root| {
        if remove {
            let hooks = root.get_mut("hooks");
            let timeout = cfg.setup.hook_timeout_s;
            super::claude::strip_ours(&a, &path, hooks, COMPACT, "timeout", timeout)
        } else {
            super::claude::insert_ours(
                object_at(root, "hooks"),
                COMPACT,
                &super::rtok_hook_bin(),
                "timeout",
                cfg.setup.hook_timeout_s,
            )
        }
    })
}

/// Codex's config dir: the one `config_path` lives in (`~/.codex`).
fn config_dir(cfg: &Config) -> PathBuf {
    let p = &cfg.setup.codex.config_path;
    p.parent().map(PathBuf::from).unwrap_or_else(|| p.clone())
}

/// One `codex plugin …` call; `CODEX_HOME` only when `config_path` is not the default.
fn codex_cli(cfg: &Config, args: &[&str]) -> std::result::Result<(), String> {
    let dir = config_dir(cfg);
    let default = super::home_dir().join(".codex");
    let env = (dir != default).then_some(("CODEX_HOME", dir.as_path()));
    super::run_cli("codex", args, env)
}

/// True when Codex's own config already enables `rtok@rtok`. Unlike Claude Code's separate
/// `installed_plugins.json`, `codex plugin add` writes this straight into
/// `~/.codex/config.toml` as `[plugins."rtok@rtok"]` `enabled = true` — verified empirically
/// against codex-cli 0.155.1 in a scratch `CODEX_HOME` (2026-09-22), since the docs at
/// developers.openai.com/plugins/build/plugins do not name `codex plugin add`/`remove`/`list`
/// at all (only `marketplace add|list|upgrade|remove`), yet `codex plugin add --help` on the
/// installed CLI shows they exist and work exactly like Claude's `plugin install`/`uninstall`.
pub(super) fn plugin_installed(cfg: &Config) -> bool {
    load(&cfg.setup.codex.config_path).ok().is_some_and(|doc| {
        doc.get("plugins")
            .and_then(Item::as_table)
            .and_then(|t| t.get(PLUGIN_ID))
            .and_then(Item::as_table)
            .and_then(|t| t.get("enabled"))
            .and_then(Item::as_bool)
            == Some(true)
    })
}

/// What `~/.codex/config.toml`'s `[marketplaces.rtok]` says (T140's counterpart of T139's own
/// `MarketplaceState` for Claude). Empirically verified against codex-cli 0.155.1: `codex
/// plugin marketplace add owner/repo` records `source_type = "git"` and `source =
/// "https://github.com/<owner>/<repo>.git"`; re-adding the identical source is a no-op, but a
/// `"rtok"` marketplace already pointing elsewhere errors "already added from a different
/// source" instead of re-pointing it, so a stale entry (a local-path marketplace from before
/// this flow, or a different checkout) must be removed before it is re-added from GitHub.
#[derive(PartialEq, Eq)]
enum MarketplaceState {
    /// No `"rtok"` entry at all.
    Absent,
    /// Points at `MARKETPLACE_GIT_SOURCE` already.
    Github,
    /// A `"rtok"` entry exists but not with that source.
    Stale,
}

fn marketplace_state(cfg: &Config) -> MarketplaceState {
    let Ok(doc) = load(&cfg.setup.codex.config_path) else {
        return MarketplaceState::Absent;
    };
    let Some(entry) = doc
        .get("marketplaces")
        .and_then(Item::as_table)
        .and_then(|t| t.get("rtok"))
        .and_then(Item::as_table)
    else {
        return MarketplaceState::Absent;
    };
    let is_github = entry.get("source_type").and_then(Item::as_str) == Some("git")
        && entry.get("source").and_then(Item::as_str) == Some(MARKETPLACE_GIT_SOURCE);
    if is_github {
        MarketplaceState::Github
    } else {
        MarketplaceState::Stale
    }
}

/// Offer, add, or remove rtok's Codex plugin through the real `codex plugin` commands
/// (T140, the Codex counterpart of T139's Claude flow), from the GitHub marketplace
/// `pyrlyn/rtok`. Added by default — no flag needed — once `codex` is on PATH and the
/// plugin is not already enabled; already enabled from the GitHub marketplace (or already
/// removed) is a no-op. `marketplace add` is skipped once Codex already knows the *GitHub*
/// marketplace; a `"rtok"` marketplace known under any other source is re-pointed —
/// `marketplace remove` then `add` — instead of erroring or installing from the stale source
/// forever. A failing or missing `codex` keeps the offer open instead of failing the
/// install: the config.toml/hooks.json surfaces still go in (`apply`).
fn plugin(cfg: &Config, remove: bool) -> Result<String> {
    let a = apply(cfg);
    let installed = plugin_installed(cfg);
    let state = marketplace_state(cfg);
    if remove {
        if !installed {
            return Ok(NO_CHANGES.into());
        }
    } else if installed && state != MarketplaceState::Stale {
        return Ok(NO_CHANGES.into());
    }
    let steps: Vec<&[&str]> = if remove {
        vec![
            &["plugin", "remove", PLUGIN_ID],
            &["plugin", "marketplace", "remove", "rtok"],
        ]
    } else {
        let mut v: Vec<&[&str]> = Vec::new();
        if state == MarketplaceState::Stale {
            if installed {
                v.push(&["plugin", "remove", PLUGIN_ID]);
            }
            v.push(&["plugin", "marketplace", "remove", "rtok"]);
        }
        if state != MarketplaceState::Github {
            v.push(&["plugin", "marketplace", "add", MARKETPLACE_REPO]);
        }
        v.push(&["plugin", "add", PLUGIN_ID]);
        v
    };
    let shown = steps
        .iter()
        .map(|s| format!("codex {}", s.join(" ")))
        .collect::<Vec<_>>()
        .join(" && ");
    if a.dry_run {
        return Ok(if remove {
            format!("- plugin {PLUGIN_ID} ({shown})")
        } else {
            format!("offer {PLUGIN_SRC} → {shown} {KETCH_INSTALL}")
        });
    }
    if !remove && super::find_on_path("codex").is_none() {
        return Ok(format!(
            "offer {PLUGIN_SRC} → {shown} (codex failed: codex not found on PATH) {KETCH_INSTALL}"
        ));
    }
    for step in steps {
        if let Err(e) = codex_cli(cfg, step) {
            return Ok(format!("offer {PLUGIN_SRC} → {shown} (codex failed: {e})"));
        }
    }
    Ok(if remove {
        format!("- plugin {PLUGIN_ID}")
    } else {
        format!("+ plugin {PLUGIN_SRC} → {PLUGIN_ID}")
    })
}

/// Every file of the installed plugin cache with its bytes, in path order. `marketplace
/// upgrade` keeps the `<version>` dir and writes no record to `config.toml`, so the cache
/// content is the only evidence an upgrade changed anything (codex-cli 0.155.1, 2026-09-24).
fn cache_bytes(cfg: &Config) -> Vec<(PathBuf, Vec<u8>)> {
    let dir = config_dir(cfg).join("plugins/cache/rtok/rtok");
    ignore::WalkBuilder::new(dir)
        .standard_filters(false)
        .sort_by_file_name(|a, b| a.cmp(b))
        .build()
        .flatten()
        .filter_map(|e| Some((e.path().to_path_buf(), fs::read(e.path()).ok()?)))
        .collect()
}

/// `agents update` over a plugin already enabled from the GitHub marketplace (T242.4). Codex
/// has no `plugin update`: `marketplace upgrade rtok` re-fetches the snapshot and the installed
/// cache in place. A failed upgrade means a broken snapshot, so the whole chain is reinstalled
/// (`plugin remove`, `marketplace remove`, `marketplace add`, `plugin add`). An unchanged cache
/// reads as [`NO_CHANGES`], so an upgrade that found nothing new says `already current`.
fn plugin_update(cfg: &Config) -> Result<String> {
    const UPDATE: [&[&str]; 1] = [&["plugin", "marketplace", "upgrade", "rtok"]];
    const REINSTALL: [&[&str]; 4] = [
        &["plugin", "remove", PLUGIN_ID],
        &["plugin", "marketplace", "remove", "rtok"],
        &["plugin", "marketplace", "add", MARKETPLACE_REPO],
        &["plugin", "add", PLUGIN_ID],
    ];
    let shown = |steps: &[&[&str]]| {
        steps
            .iter()
            .map(|s| format!("codex {}", s.join(" ")))
            .collect::<Vec<_>>()
            .join(" && ")
    };
    if apply(cfg).dry_run {
        return Ok(format!("~ plugin {PLUGIN_ID} ({})", shown(&UPDATE)));
    }
    if super::find_on_path("codex").is_none() {
        return Ok(NO_CHANGES.into());
    }
    let before = cache_bytes(cfg);
    let Err(e) = UPDATE.iter().try_for_each(|s| codex_cli(cfg, s)) else {
        return Ok(if cache_bytes(cfg) == before {
            NO_CHANGES.into()
        } else {
            format!("~ plugin {PLUGIN_ID} updated")
        });
    };
    match REINSTALL.iter().try_for_each(|s| codex_cli(cfg, s)) {
        Ok(()) => Ok(format!(
            "~ plugin {PLUGIN_ID} reinstalled (update failed: {e})"
        )),
        Err(e2) => Ok(format!(
            "offer {PLUGIN_SRC} → {} (codex failed: {e2})",
            shown(&REINSTALL)
        )),
    }
}

/// Codex CLI: `[mcp_servers.rtok]` and, with `--proxy`, `[model_providers.rtok]`.
pub struct Codex;

static VARIANTS: [Variant; 1] = [Variant {
    kind: Kind::Cli,
    name: "Codex",
    bins: &["codex"],
    apps: &[],
}];

impl Agent for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn variants(&self) -> &'static [Variant] {
        &VARIANTS
    }

    fn readme(&self) -> &'static str {
        include_str!("README.md")
    }

    fn support(&self, _kind: Kind, module: &str) -> Support {
        match module {
            "mcp" => Support::Yes,
            "proxy" => Support::Flag("--proxy"),
            "hooks" => Support::Yes,
            // `plugin`: `plugins/codex` through `codex plugin marketplace add`, from the
            // GitHub marketplace `pyrlyn/rtok`, enabled by default once `codex` is on PATH
            // and not already enabled (T140).
            "plugin" => Support::Yes,
            _ => Support::No("Codex has no such module"),
        }
    }

    fn files(&self, cfg: &Config, _kind: Kind) -> Vec<std::path::PathBuf> {
        vec![cfg.setup.codex.config_path.clone(), hooks_path(cfg)]
    }

    fn installed(&self, cfg: &Config, _kind: Kind) -> Vec<&'static str> {
        // The enabled plugin serves the hooks itself (D21); MCP is independent of it
        // (T275/D33): only the config's own `[mcp_servers.rtok]` table counts.
        let plugin = plugin_installed(cfg);
        let mut out = Vec::new();
        if super::mcp::has_toml_entry(&cfg.setup.codex.config_path, "mcp_servers", NAME) {
            out.push("mcp");
        }
        if super::read(&cfg.setup.codex.config_path).contains("[model_providers.rtok]") {
            out.push("proxy");
        }
        if super::read(&hooks_path(cfg)).contains(" hook PreCompact") || plugin {
            out.push("hooks");
        }
        if plugin {
            out.push("plugin");
        }
        out
    }

    fn apply(&self, cfg: &Config, _kind: Kind, mode: Mode) -> Result<Vec<String>> {
        let remove = mode == Mode::Remove;
        // Offer first: once the plugin is enabled it is the only call path (D21 singleton),
        // so hooks.json is stripped, not added — judged by Codex's own record, so a dry run
        // or a declined offer still gets the file-based hooks install. MCP is independent of
        // the plugin (T275/D33): `[mcp_servers.rtok]` is written on every install/update
        // regardless of plugin state, only remove takes it out.
        let update = mode == Mode::Update
            && plugin_installed(cfg)
            && marketplace_state(cfg) == MarketplaceState::Github;
        let mut lines = vec![if update {
            plugin_update(cfg)?
        } else {
            plugin(cfg, remove)?
        }];
        lines.push(run(cfg, remove)?);
        lines.push(run_hooks(cfg, remove || plugin_installed(cfg))?);
        // On the way out the provider block goes whether or not `--proxy` asked for it.
        if remove || cfg.setup.proxy {
            lines.push(register_proxy(cfg, remove)?);
        }
        lines.push(super::skill::sync("codex", cfg, remove)?);
        Ok(lines)
    }
}
/// Apply, dry-run, or remove the `[mcp_servers.rtok]` block.
pub fn run(cfg: &Config, remove: bool) -> Result<String> {
    let path = &cfg.setup.codex.config_path;
    let mut doc = load(path)?;
    let report = if remove {
        strip_ours(&apply(cfg), path, &mut doc)
    } else {
        insert_ours(&mut doc, path.display())?
    };
    persist(cfg, path, &doc, &report)?;
    Ok(report)
}

/// Point Codex `model_provider` at this proxy (T11.5).
pub fn register_proxy(cfg: &Config, remove: bool) -> Result<String> {
    let path = &cfg.setup.codex.config_path;
    let mut doc = load(path)?;
    let url = super::openai_proxy_url(cfg);
    let report = if remove {
        strip_proxy(&apply(cfg), path, &mut doc, &url)
    } else {
        insert_proxy(&mut doc, &url)?
    };
    persist(cfg, path, &doc, &report)?;
    Ok(report)
}

/// Read Codex's config, or start from an empty document when the file is simply absent.
///
/// An unreadable file (non-UTF-8 byte, wrong permissions) is an error, not an empty
/// document: swallowing it made the installer overwrite a config it never read, leaving only
/// the `.bak-<ts>` as a way back.
fn load(path: &Path) -> Result<DocumentMut> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| path.display().to_string()),
    };
    raw.parse::<DocumentMut>()
        .with_context(|| path.display().to_string())
}

fn persist(cfg: &Config, path: &Path, doc: &DocumentMut, report: &str) -> Result<()> {
    rtok_agent_sdk::write(&apply(cfg), path, &doc.to_string(), report)
}

fn insert_ours(doc: &mut DocumentMut, path: impl std::fmt::Display) -> Result<String> {
    let servers = doc
        .entry("mcp_servers")
        .or_insert(Item::Table(Table::new()));
    let Some(servers) = servers.as_table_mut() else {
        bail!("mcp_servers is not a table in {path}");
    };
    // No bare `[mcp_servers]` header: Codex's own files only carry the per-server tables.
    servers.set_implicit(true);
    if let Some(ours) = servers
        .get_mut(NAME)
        .and_then(Item::as_table_mut)
        .filter(|t| is_ours(t))
    {
        // An entry written before T528 gains the key; a value the user chose stays.
        if ours.contains_key(APPROVAL_KEY) {
            return Ok(NO_CHANGES.into());
        }
        ours[APPROVAL_KEY] = value(APPROVAL);
        return Ok(format!(
            "+ [mcp_servers.rtok] {APPROVAL_KEY} = \"{APPROVAL}\""
        ));
    }
    let cmd = super::rtok_command();
    let mut entry = Table::new();
    entry["command"] = value(cmd.as_str());
    let args = Array::from_iter(super::mcp_args("codex"));
    entry["args"] = value(args);
    entry[APPROVAL_KEY] = value(APPROVAL);
    servers.insert(NAME, Item::Table(entry));
    Ok(format!(
        "+ [mcp_servers.rtok]\ncommand = \"{cmd}\"\nargs = [\"{}\"]\n{APPROVAL_KEY} = \"{APPROVAL}\"",
        super::mcp_args("codex").join("\", \"")
    ))
}

/// `have` without our own approval value, so remove judges an entry written before T528 and
/// one written after it the same way; any other value is the user's edit.
fn without_our_approval(mut have: serde_json::Value) -> serde_json::Value {
    if let Some(m) = have.as_object_mut()
        && m.get(APPROVAL_KEY).and_then(serde_json::Value::as_str) == Some(APPROVAL)
    {
        m.remove(APPROVAL_KEY);
    }
    have
}

/// The `[mcp_servers.rtok]` table [`insert_ours`] writes, as JSON: the shape
/// [`rtok_agent_sdk::judge_owned`] compares a live table against (T246.5, T275/D33). The
/// literal command never matters — `judge_owned` folds every rtok binary string to one.
fn mcp_entry() -> serde_json::Value {
    json!({"command": "rtok", "args": super::mcp_args("codex")})
}

/// Take the `rtok` slot back only as far as rtok wrote it (T246, T246.5, T275/D33):
/// [`rtok_agent_sdk::judge_owned`] leaves a slot that does not run the rtok binary, or one the
/// user changed from [`mcp_entry`], unless `--yes` (or an accepted prompt) says remove. A
/// missing slot is already the goal.
fn strip_ours(apply: &Apply, path: &Path, doc: &mut DocumentMut) -> String {
    let have = doc
        .get("mcp_servers")
        .and_then(|s| s.get(NAME))
        .map(super::mcp::toml_item_to_json)
        .map(without_our_approval);
    let Some(have) = have else {
        return NO_CHANGES.into();
    };
    let at = format!("mcp_servers.{NAME} in {}", path.display());
    if let Some(leave) =
        rtok_agent_sdk::judge_owned(apply, &at, &have, &mcp_entry(), super::is_rtok_bin)
    {
        return leave;
    }
    let removed = doc
        .get_mut("mcp_servers")
        .and_then(Item::as_table_mut)
        .and_then(|t| t.remove(NAME))
        .is_some();
    if removed {
        "- [mcp_servers.rtok]".into()
    } else {
        NO_CHANGES.into()
    }
}

fn is_ours(t: &Table) -> bool {
    let args: Vec<&str> = t
        .get("args")
        .and_then(Item::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .collect();
    t.get("command")
        .and_then(Item::as_str)
        .is_some_and(super::is_rtok_bin)
        && args == super::mcp_args("codex")
}

fn insert_proxy(doc: &mut DocumentMut, url: &str) -> Result<String> {
    let old = doc
        .get("model_provider")
        .and_then(Item::as_str)
        .map(str::to_string);
    let ours = doc
        .get("model_providers")
        .and_then(Item::as_table)
        .and_then(|t| t.get(NAME))
        .and_then(Item::as_table)
        .is_some_and(|t| {
            t.get("name").and_then(Item::as_str) == Some(NAME)
                && t.get("base_url").and_then(Item::as_str) == Some(url)
        });
    if old.as_deref() == Some(NAME) && ours {
        return Ok(NO_CHANGES.into());
    }
    // A re-install keeps the value the first install replaced.
    let was = match old.as_deref() {
        Some(NAME) => replaced_provider(doc),
        other => other.map(str::to_string),
    };
    let mut key = toml_edit::Value::from(NAME);
    if let Some(s) = &was {
        let s = toml_edit::Value::from(s.as_str());
        key.decor_mut().set_suffix(format!(" {WAS}{s}"));
    }
    doc["model_provider"] = Item::Value(key);
    let tables = doc
        .entry("model_providers")
        .or_insert(Item::Table(Table::new()));
    let Some(tables) = tables.as_table_mut() else {
        bail!("model_providers is not a table");
    };
    tables.set_implicit(true);
    let mut entry = Table::new();
    entry["name"] = value(NAME);
    entry["base_url"] = value(url);
    tables.insert(NAME, Item::Table(entry));
    let revert = match &was {
        Some(s) => format!("revert: set model_provider to {s}"),
        None => "revert: remove [model_providers.rtok]".into(),
    };
    Ok(format!(
        "+ [model_providers.rtok]\nbase_url = \"{url}\"\n{revert}"
    ))
}

/// Trailing comment [`insert_proxy`] leaves on the `model_provider` line it took over: the
/// value it replaced, as a TOML string, so [`strip_proxy`] can put it back (T307). It lives
/// in the file itself, so it survives re-installs and needs no state outside Codex's config.
const WAS: &str = "# rtok: was ";

/// The `model_provider` value rtok replaced, read back from its [`WAS`] comment.
fn replaced_provider(doc: &DocumentMut) -> Option<String> {
    let suffix = doc.get("model_provider")?.as_value()?.decor().suffix()?;
    let was = suffix.as_str()?.trim().strip_prefix(WAS)?;
    let was = was.parse::<toml_edit::Value>().ok()?;
    was.as_str().map(str::to_string)
}

/// The `[model_providers.rtok]` table [`insert_proxy`] writes, as JSON.
fn proxy_entry(url: &str) -> serde_json::Value {
    json!({"name": NAME, "base_url": url})
}

/// Undo [`insert_proxy`] only as far as rtok wrote it (T307): a `[model_providers.rtok]`
/// the user changed from [`proxy_entry`] stays, with `model_provider`, unless `--yes` (or an
/// accepted prompt) says remove — [`rtok_agent_sdk::keep_edited`], the ownership half of
/// `judge_owned` (its "runs the rtok binary" half does not fit a provider table). A
/// `model_provider` rtok replaced is restored; one rtok added is removed.
fn strip_proxy(apply: &Apply, path: &Path, doc: &mut DocumentMut, url: &str) -> String {
    let have = doc
        .get("model_providers")
        .and_then(|t| t.get(NAME))
        .map(super::mcp::toml_item_to_json);
    if have.is_some_and(|have| have != proxy_entry(url)) {
        let at = format!("model_providers.{NAME} in {}", path.display());
        if let Some(leave) = rtok_agent_sdk::keep_edited(apply, &at) {
            return leave;
        }
    }
    let key = doc.get("model_provider").and_then(Item::as_str) == Some(NAME);
    let restored = key.then(|| replaced_provider(doc)).flatten();
    match &restored {
        Some(was) => doc["model_provider"] = value(was.as_str()),
        None if key => drop(doc.remove("model_provider")),
        None => {}
    }
    let table = doc
        .get_mut("model_providers")
        .and_then(Item::as_table_mut)
        .and_then(|t| t.remove(NAME))
        .is_some();
    match restored {
        Some(was) => format!(
            "- [model_providers.rtok]\nrestore model_provider = {}",
            toml_edit::Value::from(was)
        ),
        None if key || table => "- [model_providers.rtok]".into(),
        None => NO_CHANGES.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn cfg(dir: &str, dry: bool) -> (Config, PathBuf) {
        let dir = std::env::temp_dir().join(format!("rtok-codex-{dir}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let mut c = crate::testutil::config_in(&dir);
        c.setup.codex.config_path = path.clone();
        c.setup.dry_run = dry;
        c.setup.backup = false;
        (c, path)
    }

    #[test]
    fn dry_run_shows_one_block_and_touches_nothing() {
        let (c, path) = cfg("dry", true);
        fs::write(&path, "model = \"o3\" # keep me\n").unwrap();
        let out = run(&c, false).unwrap();
        assert!(out.starts_with("+ [mcp_servers.rtok]"), "{out}");
        assert!(
            out.contains("args = [\"mcp\", \"--host\", \"codex\"]"),
            "{out}"
        );
        assert_eq!(out.matches("[mcp_servers.rtok]").count(), 1);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "model = \"o3\" # keep me\n"
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// An unreadable config is not an empty one: the installer must fail rather than
    /// overwrite a file it could not read.
    #[test]
    fn an_unreadable_config_is_refused_not_overwritten() {
        let (c, path) = cfg("unreadable", false);
        let original = b"model = \"o3\"\n# \xff\xfe not utf-8\n";
        fs::write(&path, original).unwrap();
        let err = run(&c, false).unwrap_err();
        assert!(err.to_string().contains("config.toml"), "{err}");
        assert_eq!(fs::read(&path).unwrap(), original, "file untouched");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn apply_keeps_comments_and_other_servers_and_is_idempotent() {
        let (c, path) = cfg("apply", false);
        fs::write(
            &path,
            "# codex config\nmodel = \"o3\"\n\n[mcp_servers.other]\ncommand = \"x\"\n",
        )
        .unwrap();
        assert!(run(&c, false).unwrap().starts_with("+ [mcp_servers.rtok]"));
        assert_eq!(run(&c, false).unwrap(), NO_CHANGES);
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.starts_with("# codex config\nmodel = \"o3\"\n"), "{raw}");
        assert!(
            raw.contains("[mcp_servers.other]\ncommand = \"x\"\n"),
            "{raw}"
        );
        assert!(raw.contains("[mcp_servers.rtok]"), "{raw}");
        assert!(
            raw.contains("args = [\"mcp\", \"--host\", \"codex\"]"),
            "{raw}"
        );
        assert!(!raw.contains("\n[mcp_servers]\n"), "{raw}");
        let parsed: toml_edit::DocumentMut = raw.parse().unwrap();
        assert_eq!(
            parsed["mcp_servers"]["rtok"]["args"][0].as_str(),
            Some("mcp")
        );
        assert_eq!(
            parsed["mcp_servers"]["rtok"][APPROVAL_KEY].as_str(),
            Some("writes")
        );
        assert!(
            super::super::is_rtok_bin(
                parsed["mcp_servers"]["rtok"]["command"]
                    .as_str()
                    .unwrap_or("")
            ),
            "{raw}"
        );
        assert_eq!(run(&c, true).unwrap(), "- [mcp_servers.rtok]");
        assert_eq!(run(&c, true).unwrap(), NO_CHANGES);
        assert!(!fs::read_to_string(&path).unwrap().contains("rtok"));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// T528: an entry written before the approval key gains it on the next install, its
    /// per-tool tables survive, and remove still knows it as rtok's.
    #[test]
    fn an_entry_without_approval_gains_it_and_stays_ours() {
        let (c, path) = cfg("approval-upgrade", false);
        let old =
            "[mcp_servers.rtok]\ncommand = \"rtok\"\nargs = [\"mcp\", \"--host\", \"codex\"]\n";
        fs::write(
            &path,
            format!("{old}\n[mcp_servers.rtok.tools.read]\napproval_mode = \"approve\"\n"),
        )
        .unwrap();
        assert_eq!(
            run(&c, false).unwrap(),
            "+ [mcp_servers.rtok] default_tools_approval_mode = \"writes\""
        );
        assert_eq!(run(&c, false).unwrap(), NO_CHANGES);
        let raw = fs::read_to_string(&path).unwrap();
        let parsed: DocumentMut = raw.parse().unwrap();
        assert_eq!(
            parsed["mcp_servers"]["rtok"][APPROVAL_KEY].as_str(),
            Some("writes"),
            "{raw}"
        );
        assert_eq!(
            parsed["mcp_servers"]["rtok"]["tools"]["read"]["approval_mode"].as_str(),
            Some("approve"),
            "{raw}"
        );
        fs::write(&path, old).unwrap();
        assert_eq!(run(&c, true).unwrap(), "- [mcp_servers.rtok]");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// T528: an approval mode the user picked is theirs; install leaves the file alone.
    #[test]
    fn a_user_approval_mode_is_kept() {
        let (c, path) = cfg("approval-user", false);
        let mine = "[mcp_servers.rtok]\ncommand = \"rtok\"\nargs = [\"mcp\", \"--host\", \"codex\"]\ndefault_tools_approval_mode = \"prompt\" # mine\n";
        fs::write(&path, mine).unwrap();
        assert_eq!(run(&c, false).unwrap(), NO_CHANGES);
        assert_eq!(fs::read_to_string(&path).unwrap(), mine);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn missing_file_is_created_on_apply() {
        let (c, path) = cfg("new", false);
        run(&c, false).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("[mcp_servers.rtok]"), "{raw}");
        assert!(
            raw.contains("args = [\"mcp\", \"--host\", \"codex\"]"),
            "{raw}"
        );
        assert!(
            super::super::is_rtok_bin(
                raw.parse::<toml_edit::DocumentMut>().unwrap()["mcp_servers"]["rtok"]["command"]
                    .as_str()
                    .unwrap_or("")
            ),
            "{raw}"
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn proxy_dry_run_shows_one_change_and_touches_nothing() {
        let (c, path) = cfg("proxy-dry", true);
        fs::write(&path, "# keep me\n").unwrap();
        let out = register_proxy(&c, false).unwrap();
        assert_eq!(out.matches("+ [model_providers.rtok]").count(), 1);
        assert!(out.contains("8790/v1"), "{out}");
        assert!(out.contains("revert:"), "{out}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "# keep me\n");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn proxy_apply_is_idempotent_and_remove_strips() {
        let (c, path) = cfg("proxy-apply", false);
        fs::write(&path, "# keep me\nmodel = \"o3\"\n").unwrap();
        let first = register_proxy(&c, false).unwrap();
        assert!(first.contains("[model_providers.rtok]"), "{first}");
        assert_eq!(register_proxy(&c, false).unwrap(), NO_CHANGES);
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("model_provider = \"rtok\""), "{raw}");
        assert!(raw.contains("base_url"), "{raw}");
        assert!(raw.contains("# keep me"), "{raw}");
        assert_eq!(
            register_proxy(&c, true).unwrap(),
            "- [model_providers.rtok]"
        );
        assert_eq!(register_proxy(&c, true).unwrap(), NO_CHANGES);
        let gone = fs::read_to_string(&path).unwrap();
        assert!(!gone.contains("model_providers.rtok"), "{gone}");
        assert!(gone.contains("# keep me"), "{gone}");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// T307: the `model_provider` rtok replaced comes back on remove, byte for byte, even
    /// after a re-install over rtok's own value.
    #[test]
    fn proxy_remove_restores_the_replaced_model_provider() {
        let (c, path) = cfg("proxy-restore", false);
        let before = "# keep me\nmodel_provider = \"openai\"\nmodel = \"o3\"\n";
        fs::write(&path, before).unwrap();
        let first = register_proxy(&c, false).unwrap();
        assert!(
            first.contains("revert: set model_provider to openai"),
            "{first}"
        );
        let raw = fs::read_to_string(&path).unwrap();
        assert!(
            raw.contains("model_provider = \"rtok\" # rtok: was \"openai\"\n"),
            "{raw}"
        );
        // A re-install that rewrites rtok's table still remembers "openai", not "rtok".
        fs::write(&path, raw.replace("8790", "1")).unwrap();
        let again = register_proxy(&c, false).unwrap();
        assert!(
            again.contains("revert: set model_provider to openai"),
            "{again}"
        );
        assert_eq!(register_proxy(&c, false).unwrap(), NO_CHANGES);
        assert_eq!(
            register_proxy(&c, true).unwrap(),
            "- [model_providers.rtok]\nrestore model_provider = \"openai\""
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(register_proxy(&c, true).unwrap(), NO_CHANGES);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// T307: a `[model_providers.rtok]` the user edited stays on remove (with
    /// `model_provider`), unless `--yes` says remove.
    #[test]
    fn proxy_remove_leaves_a_hand_edited_provider_table() {
        let (mut c, path) = cfg("proxy-edited", false);
        fs::write(&path, "model_provider = \"openai\"\n").unwrap();
        register_proxy(&c, false).unwrap();
        let edited = fs::read_to_string(&path)
            .unwrap()
            .replace("http://127.0.0.1:8790/v1", "http://example.test/v1");
        assert!(edited.contains("example.test"), "{edited}");
        fs::write(&path, &edited).unwrap();
        let out = register_proxy(&c, true).unwrap();
        assert!(out.starts_with("leave model_providers.rtok"), "{out}");
        assert_eq!(fs::read_to_string(&path).unwrap(), edited);
        c.setup.yes = true;
        let out = register_proxy(&c, true).unwrap();
        assert!(out.contains("restore model_provider = \"openai\""), "{out}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "model_provider = \"openai\"\n"
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn compact_hooks_save_and_post_restores() {
        let (c, path) = cfg("compact", false);
        let report = run_hooks(&c, false).unwrap();
        assert!(report.contains("PreCompact"), "{report}");
        assert!(report.contains("PostCompact"), "{report}");
        let raw = fs::read_to_string(hooks_path(&c)).unwrap();
        assert!(raw.contains("hook PreCompact"), "{raw}");
        assert!(raw.contains("hook PostCompact"), "{raw}");
        assert_eq!(Codex.installed(&c, Kind::Cli), ["hooks"]);

        let pre = serde_json::json!({
            "hook_event_name":"PreCompact",
            "session_id":"cdx",
            "transcript_path":"",
            "trigger":"auto"
        });
        let mut out = Vec::new();
        crate::hooks::run("PreCompact", pre.to_string().as_bytes(), &mut out, &c);
        assert_eq!(out, b"{}");
        assert!(
            crate::store::Store::open(&c.core.db_path)
                .unwrap()
                .latest_note("checkpoint:cdx")
                .unwrap()
                .is_some()
        );
        let post = serde_json::json!({
            "hook_event_name":"PostCompact",
            "session_id":"cdx",
            "trigger":"auto"
        });
        out.clear();
        crate::hooks::run("PostCompact", post.to_string().as_bytes(), &mut out, &c);
        let text = serde_json::from_slice::<serde_json::Value>(&out).unwrap()["hookSpecificOutput"]
            ["additionalContext"]
            .as_str()
            .unwrap_or("")
            .to_string();
        assert!(text.contains("checkpoint"), "{text}");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// T275/D33: install/update always write `[mcp_servers.rtok]`, plugin enabled or not;
    /// only remove takes it out; a user-edited entry is left with a `leave` line.
    #[test]
    fn t275_mcp_entry_always_written_except_on_remove() {
        let (c, path) = cfg("t275-mcp", false);
        fs::write(&path, "[plugins.\"rtok@rtok\"]\nenabled = true\n").unwrap();
        assert!(plugin_installed(&c));

        crate::agents::mcp::assert_toml_entry_lifecycle(
            &Codex,
            &c,
            Kind::Cli,
            &path,
            "mcp_servers",
            || run(&c, false),
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    // --- T140: `plugin()` decision logic — enabled/known-marketplace/fresh, no real `codex` ---

    /// Already enabled from the GitHub marketplace → no-op, whether or not `--dry-run` is given.
    #[test]
    fn plugin_install_is_a_no_op_once_codex_already_enables_it() {
        let (c, path) = cfg("plugin-enabled", false);
        fs::write(
            &path,
            format!(
                "[marketplaces.rtok]\nsource_type = \"git\"\nsource = \"{MARKETPLACE_GIT_SOURCE}\"\n\n[plugins.\"rtok@rtok\"]\nenabled = true\n"
            ),
        )
        .unwrap();
        assert_eq!(plugin(&c, false).unwrap(), NO_CHANGES);
        let mut dry = c.clone();
        dry.setup.dry_run = true;
        assert_eq!(plugin(&dry, false).unwrap(), NO_CHANGES);
        assert!(plugin_installed(&c));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// Not enabled and not removing it either → no-op (nothing to disable).
    #[test]
    fn plugin_remove_is_a_no_op_when_not_installed() {
        let (c, path) = cfg("plugin-remove-noop", false);
        assert_eq!(plugin(&c, true).unwrap(), NO_CHANGES);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// A marketplace already pointed at the GitHub repo skips `marketplace add`: only the
    /// `plugin add` step is offered.
    #[test]
    fn plugin_dry_run_skips_marketplace_add_when_already_known() {
        let (c, path) = cfg("plugin-known-market", true);
        fs::write(
            &path,
            format!("[marketplaces.rtok]\nsource_type = \"git\"\nsource = \"{MARKETPLACE_GIT_SOURCE}\"\n"),
        )
        .unwrap();
        let report = plugin(&c, false).unwrap();
        assert!(report.contains("codex plugin add rtok@rtok"), "{report}");
        assert!(!report.contains("marketplace add"), "{report}");
        assert!(!report.contains("marketplace remove"), "{report}");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// Nothing known yet: dry run previews both steps, from the GitHub marketplace, not a
    /// local path — and touches no file.
    #[test]
    fn plugin_dry_run_offers_both_steps_from_a_clean_install() {
        let (c, path) = cfg("plugin-fresh", true);
        let report = plugin(&c, false).unwrap();
        assert!(
            report.contains("codex plugin marketplace add pyrlyn/rtok"),
            "{report}"
        );
        assert!(report.contains("codex plugin add rtok@rtok"), "{report}");
        assert!(report.starts_with("offer plugins/codex → "), "{report}");
        assert!(report.contains(KETCH_INSTALL), "{report}");
        assert!(!path.exists(), "{report}");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// A marketplace added before the repository moved from `listepo` to `pyrlyn` still
    /// records the old clone URL; it is re-pointed at the new one rather than trusted to
    /// GitHub's redirect.
    #[test]
    fn plugin_dry_run_repoints_a_marketplace_added_under_the_old_repo_name() {
        let (c, path) = cfg("plugin-old-repo-market", true);
        fs::write(
            &path,
            "[marketplaces.rtok]\nsource_type = \"git\"\nsource = \"https://github.com/listepo/rtok.git\"\n",
        )
        .unwrap();
        let report = plugin(&c, false).unwrap();
        assert!(
            report.contains(
                "codex plugin marketplace remove rtok && codex plugin marketplace add pyrlyn/rtok && codex plugin add rtok@rtok"
            ),
            "{report}"
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// A `"rtok"` marketplace registered under a different source (e.g. a local-path
    /// checkout) is removed before it is re-added from GitHub — `marketplace add` on top of
    /// an existing entry errors instead of re-pointing it (verified against codex-cli
    /// 0.155.1: "already added from a different source").
    #[test]
    fn plugin_dry_run_repoints_a_stale_marketplace() {
        let (c, path) = cfg("plugin-stale-market", true);
        fs::write(
            &path,
            "[marketplaces.rtok]\nsource_type = \"local\"\nsource = \"/Users/x/rtok\"\n",
        )
        .unwrap();
        let report = plugin(&c, false).unwrap();
        assert!(
            report.contains(
                "codex plugin marketplace remove rtok && codex plugin marketplace add pyrlyn/rtok && codex plugin add rtok@rtok"
            ),
            "{report}"
        );
        assert!(!report.contains("plugin remove"), "{report}");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// The plugin already shows as enabled, but under a stale local marketplace: it is
    /// removed and re-added from GitHub instead of the usual already-installed no-op.
    #[test]
    fn plugin_dry_run_reinstalls_when_already_enabled_from_a_stale_marketplace() {
        let (c, path) = cfg("plugin-stale-installed", true);
        fs::write(
            &path,
            "[marketplaces.rtok]\nsource_type = \"local\"\nsource = \"/Users/x/rtok\"\n\n[plugins.\"rtok@rtok\"]\nenabled = true\n",
        )
        .unwrap();
        let report = plugin(&c, false).unwrap();
        assert_ne!(report, NO_CHANGES);
        assert!(
            report.contains(
                "codex plugin remove rtok@rtok && codex plugin marketplace remove rtok && codex plugin marketplace add pyrlyn/rtok && codex plugin add rtok@rtok"
            ),
            "{report}"
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_entry_without_host_is_upgraded_and_still_removable() {
        let (c, path) = cfg("legacy-host", false);
        let seed = "# keep me\n[mcp_servers.other]\ncommand = \"x\"\n\n[mcp_servers.rtok]\ncommand = \"rtok\"\nargs = [\"mcp\"]\n";
        crate::agents::mcp::assert_legacy_entry_upgraded(
            &path,
            seed,
            "codex",
            &["# keep me", "[mcp_servers.other]\ncommand = \"x\"\n"],
            || run(&c, false),
            || run(&c, true),
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
