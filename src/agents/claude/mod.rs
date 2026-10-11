// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Install rtok hooks into Claude Code `settings.json` (plan T2.3).
//!
//! What is Claude-specific is the `hooks` shape below; backup, the write gate and the
//! `mcpServers` entry come from `rtok-agent-sdk` (D28).

pub mod migrate;

use std::fmt;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::config::Config;
use anyhow::Result;
use rtok_agent_sdk::{Apply, KETCH_INSTALL, NO_CHANGES, array_at, edit_json, object_at};
use rtok_mcp::ops::Mode as McpMode;
use rtok_mcp::spec::{
    Client as McpClient, DuplicateName, EntryShape, Format as McpFormat, Launch, McpSpec,
};
use semver::Version;
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, Visitor};
use serde_json::{Value, json};

use super::{Agent, Kind, Mode, Support, Variant, apply, mcp, plugin_version};

/// Claude's own `(event, matcher)` list (empty matcher omits the field), read from the plugin's
/// `hooks/hooks.json` in file order — the one place it is written (T262.1), so the GitHub plugin
/// install and the settings-file install cannot drift. `tests/hook_manifests.rs` and
/// `plugin_tree_matches_the_installer` hold it to the `claude` rows of `hook_events`, which the
/// other Claude-shaped hosts (Kimi, ZCode, Devin) have their own rows beside. `SubagentStart`
/// is the spawn brief's event (T130.2); Kimi discards a `SubagentStart` hook's output, so its
/// rows leave it out.
fn claude_entries() -> &'static [(&'static str, &'static str)] {
    static LIST: LazyLock<Vec<(&str, &str)>> = LazyLock::new(|| {
        serde_json::from_str::<PluginHooks>(include_str!(
            "../../../plugins/claude/hooks/hooks.json"
        ))
        .expect("plugins/claude/hooks/hooks.json parses (plugin_tree_matches_the_installer)")
        .hooks
        .0
        .into_iter()
        // T159: the worktree hooks replace the host's own create/remove, so they need the
        // launcher's plain-git fallback — only the plugin carries that, never settings.json.
        .filter(|(event, _)| !event.starts_with("Worktree"))
        .collect()
    });
    &LIST
}

#[derive(Deserialize)]
struct PluginHooks<'a> {
    #[serde(borrow)]
    hooks: Events<'a>,
}

#[derive(Deserialize)]
struct Matcher<'a> {
    #[serde(borrow)]
    matcher: Option<&'a str>,
}

/// `(event, matcher)` pairs of a `hooks` object in file order; a `Value` would sort the events
/// (no `preserve_order` here) and reorder every install report.
struct Events<'a>(Vec<(&'a str, &'a str)>);

impl<'de: 'a, 'a> Deserialize<'de> for Events<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visit<'a>(PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for Visit<'a> {
            type Value = Events<'a>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a map of hook events")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Events<'a>, A::Error> {
                let mut out = Vec::new();
                while let Some((event, list)) = map.next_entry::<&'a str, Vec<Matcher<'a>>>()? {
                    out.extend(list.into_iter().map(|m| (event, m.matcher.unwrap_or(""))));
                }
                Ok(Events(out))
            }
        }
        d.deserialize_map(Visit(PhantomData))
    }
}

/// T174: `rtok_command()` deliberately keeps the bare name on non-Windows even when PATH
/// lookup fails (an absolute path is the Windows spawn edge, not a Unix one) — so a settings
/// file written on a machine whose install shell had `~/.ketch/bin` on PATH still names bare
/// `rtok`, and a *later* hook-running shell without it hit `/bin/sh: rtok: command not found`
/// (exit 127) on every tool call, 380 times/week in the field. Resolve at hook-run time
/// instead: PATH, then `~/.ketch/bin/rtok` (ketch's own layout), then fail open — silent for
/// every event but SessionStart, which gets one note naming the install command, in Claude's
/// own `hookSpecificOutput` shape, so the miss is visible without repeating on every call.
/// An absolute `bin` (Windows, or any bin already resolved) needs none of this — it already
/// names one exact file or nothing does.
fn command(bin: &str, event: &str) -> String {
    if cfg!(windows) || bin != "rtok" {
        return format!("{bin} hook {event}");
    }
    let note = (event == "SessionStart").then_some(
        r#"{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"rtok is not installed; run ketch install pyrlyn/rtok to enable it."}}"#,
    );
    super::hook_resolver(&format!("hook {event}"), note)
}

/// Exactly `<rtok-bin> hook <event>` (an older or Windows install), or the T174
/// PATH-resolving form whose `exec rtok hook <event>;` marker only a real rtok settings
/// entry would carry. Matching tokens anywhere claimed a user's chain (`notify-send hi &&
/// rtok hook Stop`); the suffix/marker checks keep that command foreign.
pub(super) fn is_ours(cmd: &str, event: &str) -> bool {
    let suffix = format!(" hook {event}");
    if let Some(bin) = cmd.strip_suffix(&suffix) {
        return super::is_rtok_bin(super::unquote_bin(bin));
    }
    cmd.contains(&format!("exec rtok hook {event};"))
}

/// Apply, dry-run, or remove rtok hook entries.
pub fn run(cfg: &Config, remove: bool) -> Result<String> {
    let (a, path) = (apply(cfg), &cfg.setup.claude.settings_path);
    edit_json(&a, path, |root| {
        if remove {
            let timeout = cfg.setup.hook_timeout_s;
            strip_ours(
                &a,
                path,
                root.get_mut("hooks"),
                claude_entries(),
                "timeout",
                timeout,
            )
        } else {
            let bin = super::rtok_hook_bin();
            insert_ours(
                object_at(root, "hooks"),
                claude_entries(),
                &bin,
                "timeout",
                cfg.setup.hook_timeout_s,
            )
        }
    })
}

/// How a host spells rtok's hook command: `command(bin, event)` is what gets written and
/// `is_ours(cmd, event)` recognises one already there. The shared insert/strip walk
/// ([`insert_ours_as`], [`strip_ours_as`]) is the same for every host with Claude's
/// `hooks.<event>[] = {matcher?, hooks: [..]}` layout; only this spelling differs (Devin adds
/// `--host devin`, T89).
pub(super) struct HookForm {
    pub command: fn(&str, &str) -> String,
    pub is_ours: fn(&str, &str) -> bool,
}

/// [`command`] for a host that names itself: `hook <event> --host <host>` (Devin T89, CodeBuddy
/// T560.2), resolved at run time like Claude's on POSIX.
pub(super) fn command_for(host: &str, bin: &str, event: &str) -> String {
    if cfg!(windows) || bin != "rtok" {
        return format!("{bin} hook {event} --host {host}");
    }
    super::hook_resolver(&format!("hook {event} --host {host}"), None)
}

/// [`is_ours`] for [`command_for`]'s spelling.
pub(super) fn is_ours_for(host: &str, cmd: &str, event: &str) -> bool {
    let suffix = format!(" hook {event} --host {host}");
    if let Some(bin) = cmd.strip_suffix(&suffix) {
        return super::is_rtok_bin(super::unquote_bin(bin));
    }
    cmd.contains(&format!("exec rtok hook {event} --host {host};"))
}

/// Claude's own spelling, which Codex and ZCode share.
const CLAUDE_FORM: HookForm = HookForm { command, is_ours };

fn has_ours(form: &HookForm, entry: &Value, event: &str, matcher: &str) -> bool {
    let got = entry.get("matcher").and_then(Value::as_str).unwrap_or("");
    got == matcher
        && entry
            .get("hooks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|h| h.get("command").and_then(Value::as_str))
            .any(|c| (form.is_ours)(c, event))
}

/// Add `<bin> hook <event>` under `hooks.<event>[]` for each entry not already ours, and
/// bring the ones that are up to date (T242.1): an rtok hook written by another binary path
/// or with another timeout is rewritten in its slot, and an rtok hook on a pair `entries` no
/// longer lists (a changed matcher) is dropped. Foreign hooks are never touched.
/// `timeout_key` is the host's spelling (`timeout` seconds in Claude, `timeoutMs` in ZCode).
pub(super) fn insert_ours(
    hooks: &mut Value,
    entries: &[(&str, &str)],
    bin: &str,
    timeout_key: &str,
    timeout: u64,
) -> String {
    insert_ours_as(&CLAUDE_FORM, hooks, entries, bin, timeout_key, timeout)
}

/// [`insert_ours`] for a host whose command is spelled as `form` says.
pub(super) fn insert_ours_as(
    form: &HookForm,
    hooks: &mut Value,
    entries: &[(&str, &str)],
    bin: &str,
    timeout_key: &str,
    timeout: u64,
) -> String {
    let mut changed = prune_ours(form, hooks, entries);
    for &(event, matcher) in entries {
        let want = (form.command)(bin, event);
        let mut found = false;
        for entry in array_at(hooks, event).iter_mut() {
            if !has_ours(form, entry, event, matcher) {
                continue;
            }
            for h in entry["hooks"].as_array_mut().into_iter().flatten() {
                if !h["command"]
                    .as_str()
                    .is_some_and(|c| (form.is_ours)(c, event))
                {
                    continue;
                }
                found = true;
                if h["command"] != json!(want) || h[timeout_key] != json!(timeout) {
                    h["command"] = json!(want);
                    h[timeout_key] = json!(timeout);
                    changed.push(format!("~ {event}{} {want}", show(matcher)));
                }
            }
        }
        if found {
            continue;
        }
        let mut obj = serde_json::Map::new();
        if !matcher.is_empty() {
            obj.insert("matcher".into(), json!(matcher));
        }
        obj.insert(
            "hooks".into(),
            json!([{"type":"command","command":want,timeout_key:timeout}]),
        );
        array_at(hooks, event).push(Value::Object(obj));
        changed.push(format!("+ {event}{} {want}", show(matcher)));
    }
    if changed.is_empty() {
        return NO_CHANGES.into();
    }
    let count = |p: char| changed.iter().filter(|l| l.starts_with(p)).count();
    let summary: Vec<String> = [('+', "additions"), ('~', "updates"), ('-', "removals")]
        .into_iter()
        .filter(|&(p, _)| count(p) > 0)
        .map(|(p, word)| format!("{} {word}", count(p)))
        .collect();
    format!("{}\n{}", changed.join("\n"), summary.join(", "))
}

/// ` <matcher>` for a report line; nothing for an empty matcher.
pub(super) fn show(matcher: &str) -> String {
    if matcher.is_empty() {
        String::new()
    } else {
        format!(" {matcher}")
    }
}

/// Drop rtok hooks sitting on an `(event, matcher)` pair `entries` does not list — what an
/// older install wrote before a matcher changed or an event went (T242.1). Emptied entries
/// and event arrays go, as in [`strip_ours`]; one `- …` report line per dropped hook.
fn prune_ours(form: &HookForm, hooks: &mut Value, entries: &[(&str, &str)]) -> Vec<String> {
    let (mut removed, mut emptied) = (Vec::new(), Vec::new());
    let Some(map) = hooks.as_object_mut() else {
        return removed;
    };
    for (event, arr) in map.iter_mut() {
        let Some(arr) = arr.as_array_mut() else {
            continue;
        };
        let before = removed.len();
        for entry in arr.iter_mut() {
            let matcher = entry["matcher"].as_str().unwrap_or("").to_string();
            if entries.contains(&(event.as_str(), matcher.as_str())) {
                continue;
            }
            let Some(inner) = entry.get_mut("hooks").and_then(Value::as_array_mut) else {
                continue;
            };
            inner.retain(|h| match h["command"].as_str() {
                Some(c) if (form.is_ours)(c, event) => {
                    removed.push(format!("- {event}{} {c}", show(&matcher)));
                    false
                }
                _ => true,
            });
        }
        if removed.len() > before {
            arr.retain(|e| {
                e.get("hooks")
                    .and_then(Value::as_array)
                    .is_none_or(|a| !a.is_empty())
            });
            if arr.is_empty() {
                emptied.push(event.clone());
            }
        }
    }
    map.retain(|k, _| !emptied.contains(k));
    removed
}

/// Remove the `<rtok> hook <event>` hooks under `hooks` (T246.3); empty arrays go. One still as
/// [`insert_ours`] writes it — on a pair `entries` lists, exactly `{type, command,
/// <timeout_key>: timeout}` — goes; one the user changed goes only as
/// [`rtok_agent_sdk::keep_edited`] decides, else a `leave …` line keeps it.
pub(super) fn strip_ours(
    apply: &Apply,
    path: &Path,
    hooks: Option<&mut Value>,
    entries: &[(&str, &str)],
    timeout_key: &str,
    timeout: u64,
) -> String {
    strip_ours_as(
        &CLAUDE_FORM,
        apply,
        path,
        hooks,
        entries,
        timeout_key,
        timeout,
    )
}

/// [`strip_ours`] for a host whose command is spelled as `form` says.
pub(super) fn strip_ours_as(
    form: &HookForm,
    apply: &Apply,
    path: &Path,
    hooks: Option<&mut Value>,
    entries: &[(&str, &str)],
    timeout_key: &str,
    timeout: u64,
) -> String {
    let Some(hooks) = hooks.and_then(Value::as_object_mut) else {
        return NO_CHANGES.into();
    };
    let (mut removed, mut kept) = (0usize, Vec::new());
    for (event, arr) in hooks.iter_mut() {
        let Some(arr) = arr.as_array_mut() else {
            continue;
        };
        for entry in arr.iter_mut() {
            let matcher = entry["matcher"].as_str().unwrap_or("").to_string();
            let listed = entries.contains(&(event.as_str(), matcher.as_str()));
            let Some(inner) = entry.get_mut("hooks").and_then(Value::as_array_mut) else {
                continue;
            };
            inner.retain(|h| {
                let Some(cmd) = h["command"].as_str().filter(|c| (form.is_ours)(c, event)) else {
                    return true;
                };
                let want = json!({"type": "command", "command": cmd, timeout_key: timeout});
                let at = || format!("hooks.{event}{} in {}", show(&matcher), path.display());
                let take = super::takes_hook(apply, listed && *h == want, at, &mut kept);
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
    hooks.retain(|_, v| v.as_array().is_none_or(|a| !a.is_empty()));
    super::with_kept(kept, super::removed_report(removed))
}

/// Claude's MCP entry for one client, as `rtok_mcp` reads it (T277.1): the `mcpServers.rtok`
/// object at `file`, started with `command` and named `--host claude`. Claude Code and Claude
/// Desktop are the two clients; they differ only in file and launch command.
pub(crate) fn mcp_spec_at(client: McpClient, file: PathBuf, command: String) -> McpSpec {
    McpSpec {
        host: "claude",
        client,
        config_path: file,
        format: McpFormat::Json,
        key_path: McpSpec::key_path("mcpServers"),
        server: rtok_mcp::registry::RTOK,
        launch: Launch {
            command: Some(command),
            host_arg: true,
        },
        entry_shape: EntryShape::TypedCommand,
        duplicate_name: DuplicateName::ShowsBoth,
        plugin_serves: None,
    }
}

/// [`mcp_spec_at`] for `kind` at the real file: `~/.claude.json`, or the desktop config.
fn mcp_spec(cfg: &Config, kind: Kind) -> McpSpec {
    match kind {
        Kind::Cli => mcp_spec_at(
            McpClient::Cli,
            cfg.doctor.claude_json.clone(),
            super::rtok_command(),
        ),
        Kind::Desktop => mcp_spec_at(McpClient::Desktop, desktop_path(), desktop_command()),
    }
}

/// Add `rtok mcp` to `mcpServers` in `~/.claude.json` (T4.7).
pub fn register_mcp(cfg: &Config) -> Result<String> {
    mcp::run(cfg, &mcp_spec(cfg, Kind::Cli), McpMode::Install)
}

/// Drop `mcpServers.rtok` from `~/.claude.json` (`rtok agents remove claude`).
pub fn unregister_mcp(cfg: &Config) -> Result<String> {
    mcp::run(cfg, &mcp_spec(cfg, Kind::Cli), McpMode::Remove)
}

/// The plugin tree (T114) and its id in the one-plugin marketplace that tree also is.
const PLUGIN_SRC: &str = "plugins/claude";
/// `doctor::plugin_hooks` (T173) also names this id when it walks
/// `installed_plugins.json` for the hooks the installed plugin carries.
pub(crate) const PLUGIN_ID: &str = "rtok@rtok";

/// The one Claude plugin failure `agents update`'s exit code cannot shrug off (T279 step 3/6
/// "Failure"): [`plugin_against`] already uninstalled the old copy when the install step then
/// failed, so the host is left with nothing — every other `claude` failure keeps the old copy
/// in place and stays a non-fatal `offer …`/`~ …` line. `cli::apply_hosts` greps the rendered
/// report for this marker after printing it (the same "print, then bail on a bad outcome"
/// shape `worktree gc`/`clean` already use) instead of propagating a fresh `Err`, which would
/// cut the report short and abort every other host in the same run.
pub(crate) const REINSTALL_FAILED: &str = "removed, reinstall failed:";

/// GitHub `owner/repo` shorthand `claude plugin marketplace add` resolves (T139): the repo
/// root's `.claude-plugin/marketplace.json` names this one marketplace `rtok`, whose only
/// plugin is `./plugins/claude` (relative to the repo root, not the marketplace file). A
/// local path broke across a ketch upgrade (`store/rtok/vX.Y.Z/…`); GitHub does not move.
/// An entry still naming `listepo/rtok`, the repo before it moved to the `pyrlyn` org, reads
/// as stale and is re-pointed here like any other.
const MARKETPLACE_REPO: &str = "pyrlyn/rtok";

/// Claude Code's config dir: the one `settings_path` lives in (`~/.claude`).
/// `pub(crate)`: `doctor::plugin_hooks` (T173) locates `installed_plugins.json` the
/// same way `plugin_installed` does.
pub(crate) fn config_dir(cfg: &Config) -> PathBuf {
    let s = &cfg.setup.claude.settings_path;
    s.parent().map(PathBuf::from).unwrap_or_else(|| s.clone())
}

/// The modules rtok's hook and MCP registrations carry in the two FILES a foreign importer
/// reads (`~/.claude/settings.json`, `~/.claude.json`) — the Claude plugin serves hooks, not
/// visible here, and MCP is independent of it (T275), so Grok's `[compat.claude]` import
/// counts these alone (T100).
pub fn files_serve_rtok(cfg: &Config) -> Vec<&'static str> {
    let s = super::read(&cfg.setup.claude.settings_path);
    let mut out = Vec::new();
    if s.contains("rtok hook") {
        out.push("hooks");
    }
    if mcp::has_entry(&cfg.doctor.claude_json, "mcpServers", "rtok") {
        out.push("mcp");
    }
    out
}

/// True when Claude Code lists `rtok@rtok` as installed. Read from its own record, so a
/// plugin removed through `/plugin` stops counting at once (T75). `pub(crate)`: also the
/// install check `doctor::plugin_hooks` uses (T173), so the two never drift apart.
pub(crate) fn plugin_installed(cfg: &Config) -> bool {
    super::read(&config_dir(cfg).join("plugins/installed_plugins.json"))
        .contains(&format!("\"{PLUGIN_ID}\""))
}

/// What Claude Code's `known_marketplaces.json` says about the `rtok` marketplace.
#[derive(PartialEq, Eq)]
enum MarketplaceState {
    /// No `"rtok"` entry at all.
    Absent,
    /// Points at the wanted target already: the GitHub repo — T139's own shape,
    /// `{"source": {"source": "github", "repo": "pyrlyn/rtok"}}` (verified against the
    /// Claude Code plugin-marketplaces docs) — or, for `--source local` (T279), the local
    /// checkout as `{"source": {"source": "directory", "path": "<target>"}}`.
    Current,
    /// A `"rtok"` entry exists but not with that shape — in practice the pre-T139 local
    /// ketch-store path, `{"source": {"source": "directory", "path": ".../store/rtok/vX.Y.Z/…"}}`
    /// (the literal shape a real `~/.claude/plugins/known_marketplaces.json` on this machine
    /// carried before this fix). `marketplace add` on top of it errors instead of re-pointing,
    /// so it must be removed first.
    Stale,
}

/// Parses the same file `plugin_installed` reads (T139 follow-up: a plain
/// `.contains("\"rtok\":")` could not tell the GitHub source from a stale local one, so a user
/// still holding the pre-T139 local marketplace kept installing from it forever — `marketplace
/// add` was skipped because *some* `"rtok"` entry existed, never checking what it pointed at).
/// Generalized over `target` (T279 PR 3) so [`plugin_against`] can ask the same question for a
/// local checkout path, not only the GitHub repo shorthand: `"rtok"` pointed at anything else
/// (another directory, or a different repo) reads as `Stale` either way.
fn marketplace_state_for(cfg: &Config, target: &str) -> MarketplaceState {
    let text = super::read(&config_dir(cfg).join("plugins/known_marketplaces.json"));
    let Ok(root) = serde_json::from_str::<Value>(&text) else {
        return MarketplaceState::Absent;
    };
    let Some(entry) = root.get("rtok") else {
        return MarketplaceState::Absent;
    };
    let field = |k: &str| entry["source"][k].as_str();
    let at = match field("source") {
        Some("github") => field("repo"),
        Some("directory") => field("path"),
        _ => None,
    };
    if at == Some(target) {
        MarketplaceState::Current
    } else {
        MarketplaceState::Stale
    }
}

/// One `claude plugin …` call; `CLAUDE_CONFIG_DIR` only when `settings_path` is not the
/// default. Windows shim resolution and stderr handling live in `super::run_cli` (T140), so
/// Codex's own `codex_cli` reuses the same spawn logic instead of respelling it.
fn claude_cli(cfg: &Config, args: &[&str]) -> std::result::Result<(), String> {
    let dir = config_dir(cfg);
    let default = super::home_dir().join(".claude");
    let env = (dir != default).then_some(("CLAUDE_CONFIG_DIR", dir.as_path()));
    super::run_cli("claude", args, env)
}

/// [`plugin`]'s body, generalized over `target` (a GitHub `owner/repo` shorthand or a local
/// checkout path) and `force` (T279 PR 3: the version decision already decided a reinstall is
/// needed, so the "already installed, nothing to do" short-circuit below must not veto it).
fn plugin_against(cfg: &Config, remove: bool, target: &str, force: bool) -> Result<String> {
    let a = apply(cfg);
    let installed = plugin_installed(cfg);
    let state = marketplace_state_for(cfg, target);
    if remove {
        if !installed {
            return Ok(NO_CHANGES.into());
        }
    } else if installed && state != MarketplaceState::Stale && !force {
        return Ok(NO_CHANGES.into());
    }
    let steps: Vec<Vec<&str>> = if remove {
        vec![
            vec!["plugin", "uninstall", PLUGIN_ID],
            vec!["plugin", "marketplace", "remove", "rtok"],
        ]
    } else {
        let mut v = Vec::new();
        let repoint = state == MarketplaceState::Stale;
        // `force` (T279 PR 3, `--force`/a version-driven reinstall) uninstalls first even at
        // an already-current marketplace: the point is a clean reinstall, not a repoint, so
        // `marketplace remove`/`add` stay skipped in that case.
        if installed && (repoint || force) {
            v.push(vec!["plugin", "uninstall", PLUGIN_ID]);
        }
        if repoint {
            v.push(vec!["plugin", "marketplace", "remove", "rtok"]);
        }
        if state != MarketplaceState::Current {
            v.push(vec!["plugin", "marketplace", "add", target]);
        }
        v.push(vec!["plugin", "install", PLUGIN_ID]);
        v
    };
    let shown = steps
        .iter()
        .map(|s| format!("claude {}", s.join(" ")))
        .collect::<Vec<_>>()
        .join(" && ");
    if a.dry_run {
        return Ok(if remove {
            format!("- plugin {PLUGIN_ID} ({shown})")
        } else {
            format!("offer {PLUGIN_SRC} → {shown} {KETCH_INSTALL}")
        });
    }
    if !remove && super::find_on_path("claude").is_none() {
        return Ok(format!(
            "offer {PLUGIN_SRC} → {shown} (claude failed: claude not found on PATH) {KETCH_INSTALL}"
        ));
    }
    let mut removed = false;
    for step in &steps {
        if let Err(e) = claude_cli(cfg, step) {
            return Ok(if !remove && removed {
                format!("plugin {PLUGIN_ID} {REINSTALL_FAILED} {e}")
            } else {
                format!("offer {PLUGIN_SRC} → {shown} (claude failed: {e})")
            });
        }
        if !remove && matches!(step.as_slice(), ["plugin", "uninstall", ..]) {
            removed = true;
        }
    }
    Ok(if remove {
        format!("- plugin {PLUGIN_ID}")
    } else {
        format!("+ plugin {PLUGIN_SRC} → {PLUGIN_ID}")
    })
}

/// Offer, install, or uninstall the plugin through the official `claude plugin` commands
/// (T115), from the GitHub marketplace `pyrlyn/rtok` (T139). Installed by default — no
/// `--yes` needed — once `claude` is on PATH and the plugin is not already installed;
/// already installed from the GitHub marketplace (or already removed) is a no-op.
/// `marketplace add` is skipped once Claude already knows the *GitHub* marketplace, so a
/// rerun never errors; a `"rtok"` marketplace known under any other source (the pre-T139
/// local ketch-store path) is re-pointed — `marketplace remove` then `add` — instead of
/// silently installing from the stale path forever. A failing or missing `claude` keeps the
/// offer open instead of failing the install: the settings-file hooks still go in.
///
/// `installed_plugins.json` (verified against the real file this machine wrote under the
/// pre-T139 flow) carries no field naming which marketplace a plugin came from — only a
/// cache path, install time and version — so there is no direct signal to gate a forced
/// reinstall on. The `rtok` marketplace's own recorded source is used instead: `rtok@rtok`
/// can only ever have come from whatever the one marketplace named `rtok` pointed to, so a
/// `Stale` marketplace state is reason enough to uninstall and reinstall even when the
/// plugin already shows as installed. An `Absent` marketplace with the plugin already
/// installed is left alone (ambiguous, not evidence of anything stale).
fn plugin(cfg: &Config, remove: bool) -> Result<String> {
    plugin_against(cfg, remove, MARKETPLACE_REPO, false)
}

/// `rtok@rtok`'s own `version` field in `installed_plugins.json` — Claude's fallback when
/// neither a version file nor the receipt has one (T279 step 2, plan `installed()`). The real
/// shape (verified against a machine that ran `claude plugin update`, same file `plugin_update`
/// reads elsewhere in this module) is `{"version":2,"plugins":{"rtok@rtok":[{"scope":"user",
/// "version":"latest"}]}}` — an array under `plugins.<id>`, one entry per scope, and Claude's
/// own `version` there is often a channel name (`"latest"`), not SemVer; [`plugin_version::installed`]
/// already drops whatever fails to parse, so that degrades to the same `0.0.0` a missing field
/// would.
fn host_record_version(cfg: &Config) -> Option<String> {
    let text = super::read(&config_dir(cfg).join("plugins/installed_plugins.json"));
    let root: Value = serde_json::from_str(&text).ok()?;
    root.get("plugins")?
        .get(PLUGIN_ID)?
        .get(0)?
        .get("version")?
        .as_str()
        .map(str::to_string)
}

/// The source `agents update`'s version decision compares against: `--source` wins; otherwise
/// the row already on record; otherwise GitHub, [`plugin`]'s own default for a fresh install.
fn update_source(
    cfg: &Config,
    installed: Option<&plugin_version::Installed>,
) -> Result<plugin_version::Source> {
    if let Some(s) = &cfg.setup.source {
        return s.parse();
    }
    Ok(installed.map_or(plugin_version::Source::Github, |i| i.source))
}

/// Write (or refresh) the T279 receipt row after a successful install/update. Never called on
/// failure — the receipt then stays exactly as it was, so the next `agents update` still knows
/// what is really on disk.
fn write_receipt(
    cfg: &Config,
    path: &Path,
    source: plugin_version::Source,
    available: &plugin_version::Available,
) -> Result<()> {
    let mut receipt = plugin_version::Receipt::read(path)?;
    receipt.upsert(
        "claude",
        plugin_version::ReceiptEntry {
            source,
            reference: if source == plugin_version::Source::Local {
                marketplace_target(source)
            } else {
                format!("v{}", available.version)
            },
            marketplace: (source != plugin_version::Source::Local).then(|| "rtok".to_string()),
            path: config_dir(cfg).join("plugins/installed_plugins.json"),
            version: available.version.clone(),
            installed_at: plugin_version::now_iso(),
        },
    );
    receipt.write(path)
}

/// What the `rtok` marketplace should point at for `source`: the local checkout path for
/// `Source::Local`, the GitHub repo shorthand otherwise.
fn marketplace_target(source: plugin_version::Source) -> String {
    if source == plugin_version::Source::Local {
        super::plugin_src(PLUGIN_SRC).display().to_string()
    } else {
        MARKETPLACE_REPO.to_string()
    }
}

/// `agents update`'s reinstall path (T279 step 3/6): the decision already said `Install` or
/// `Reinstall`, so [`plugin_against`] runs unconditionally (`force: true`) against `target` —
/// `source`'s GitHub repo shorthand, or the local checkout path for `Source::Local`. The
/// receipt is written only on the real `+ plugin …` success line; a dry run or a `claude`
/// failure that never removed anything leaves it untouched (both keep their own prefix). A
/// [`REINSTALL_FAILED`] line means the old copy is already gone, so the receipt row is
/// deleted too — the next `agents update` then finds no receipt, no version file and no
/// `claude`-reported install, reads that as "not installed", and installs fresh instead of
/// replaying a decision built on a copy that no longer exists.
fn reinstall(
    cfg: &Config,
    source: plugin_version::Source,
    available: &plugin_version::Available,
    receipt_path: &Path,
) -> Result<String> {
    let line = plugin_against(cfg, false, &marketplace_target(source), true)?;
    if line.starts_with('+') {
        write_receipt(cfg, receipt_path, source, available)?;
    } else if line.contains(REINSTALL_FAILED) {
        let mut receipt = plugin_version::Receipt::read(receipt_path)?;
        receipt.delete("claude");
        receipt.write(receipt_path)?;
    }
    Ok(line)
}

/// `agents update` over a plugin already installed from the GitHub marketplace (T242.3):
/// refresh the marketplace and `claude plugin update` in place; if either step fails, fall
/// back to [`reinstall`]. No `--yes`: accepting a changed marketplace-declared command stays
/// the user's call, and a non-TTY refusal just takes the reinstall path.
fn update_in_place(
    cfg: &Config,
    source: plugin_version::Source,
    available: &plugin_version::Available,
    receipt_path: &Path,
) -> Result<String> {
    const STEPS: [&[&str]; 2] = [
        &["plugin", "marketplace", "update", "rtok"],
        &["plugin", "update", PLUGIN_ID],
    ];
    let shown = || {
        STEPS
            .iter()
            .map(|s| format!("claude {}", s.join(" ")))
            .collect::<Vec<_>>()
            .join(" && ")
    };
    if apply(cfg).dry_run {
        return Ok(format!(
            "~ plugin {PLUGIN_ID} → {} ({})",
            available.version,
            shown()
        ));
    }
    if super::find_on_path("claude").is_none() {
        return Ok(NO_CHANGES.into());
    }
    match STEPS.iter().try_for_each(|s| claude_cli(cfg, s)) {
        Ok(()) => {
            write_receipt(cfg, receipt_path, source, available)?;
            Ok(format!(
                "~ plugin {PLUGIN_ID} updated to {}",
                available.version
            ))
        }
        Err(e) => {
            let line = reinstall(cfg, source, available, receipt_path)?;
            Ok(format!("{line} (update failed: {e})"))
        }
    }
}

/// `agents update` over the Claude plugin (T279 PR 3): [`plugin_version::decide`] replaces the
/// old "did `installed_plugins.json` change any bytes" check, so a stale build is caught even
/// when Claude's own marketplace cache would otherwise never move. `--dry-run`/`--force`/
/// `--source` (`cfg.setup.*`) flow straight into the decision; a `claude` failure never bails
/// the whole `agents update` — it reports the error line and leaves the receipt untouched, so
/// the next run tries again (other hosts in the same run are unaffected either way).
fn plugin_update(cfg: &Config) -> Result<String> {
    let receipt_path = plugin_version::receipt_path(cfg);
    let receipt = plugin_version::Receipt::read(&receipt_path)?;
    let currently_installed = plugin_installed(cfg);
    let installed = currently_installed.then(|| {
        plugin_version::installed(
            None,
            receipt.get("claude"),
            host_record_version(cfg).as_deref(),
            plugin_version::Source::Github,
        )
    });
    let source = update_source(cfg, installed.as_ref())?;
    let available = plugin_version::available(
        source,
        &super::plugin_src(PLUGIN_SRC),
        &Version::parse(env!("CARGO_PKG_VERSION")).expect("CARGO_PKG_VERSION is valid semver"),
    )?;
    // A marketplace pointed elsewhere (the pre-T139 ketch-store path, or the other source)
    // must be repointed, which only the reinstall path does; an in-place update would keep
    // pulling from the stale one.
    let stale = currently_installed
        && marketplace_state_for(cfg, &marketplace_target(source)) == MarketplaceState::Stale;
    match plugin_version::decide(installed, &available, cfg.setup.force || stale) {
        // Nothing to do: report NO_CHANGES like every other up-to-date step, so the block
        // header's "already current" note still fires (`agents::mod::block`). `--dry-run`
        // still names the version, since a dry run has nothing else to show for this host.
        plugin_version::Decision::Skip { .. } => {
            if apply(cfg).dry_run {
                return Ok(format!(
                    "plugin {PLUGIN_ID} {} up to date ({source})",
                    available.version
                ));
            }
            Ok(NO_CHANGES.into())
        }
        plugin_version::Decision::SkipOlder { warning } => {
            Ok(format!("plugin {PLUGIN_ID}: {warning}"))
        }
        plugin_version::Decision::Update => update_in_place(cfg, source, &available, &receipt_path),
        plugin_version::Decision::Install | plugin_version::Decision::Reinstall { .. } => {
            reinstall(cfg, source, &available, &receipt_path)
        }
    }
}

/// Where Claude Desktop reads `mcpServers`: its own file, not `~/.claude.json`.
pub fn desktop_path() -> PathBuf {
    let home = super::home_dir();
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Claude/claude_desktop_config.json")
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or(home)
            .join("Claude/claude_desktop_config.json")
    } else {
        home.join(".config/Claude/claude_desktop_config.json")
    }
}

/// A desktop app starts from the Dock or Start menu without a shell PATH, so a bare `rtok`
/// does not spawn there: write the absolute binary (Claude Desktop, ZCode).
pub(super) fn desktop_command() -> String {
    std::env::current_exe()
        .map(|exe| dunce::simplified(&exe).to_string_lossy().into_owned())
        .unwrap_or_else(|_| super::rtok_command())
}

/// Claude Code: hooks in `settings.json`, MCP in `~/.claude.json`, the proxy as
/// `env.ANTHROPIC_BASE_URL`. Claude Desktop: MCP only, in `claude_desktop_config.json`.
pub struct Claude;

static VARIANTS: [Variant; 2] = [
    Variant {
        kind: Kind::Cli,
        name: "Claude Code",
        bins: &["claude"],
        apps: &[],
    },
    Variant {
        kind: Kind::Desktop,
        name: "Claude Desktop",
        bins: &[],
        apps: &[
            "/Applications/Claude.app",
            "$LOCALAPPDATA/AnthropicClaude/claude.exe",
        ],
    },
];

impl Agent for Claude {
    fn id(&self) -> &'static str {
        "claude"
    }

    fn variants(&self) -> &'static [Variant] {
        &VARIANTS
    }

    fn readme(&self) -> &'static str {
        include_str!("README.md")
    }

    fn support(&self, kind: Kind, module: &str) -> Support {
        match (kind, module) {
            (_, "mcp") | (Kind::Cli, "hooks") => Support::Yes,
            (Kind::Cli, "proxy") => Support::Flag("--proxy"),
            // `plugin`: `plugins/claude` through `claude plugin install`, from the GitHub
            // marketplace `pyrlyn/rtok`, installed by default once `claude` is on PATH
            // and not already installed (T139).
            (Kind::Cli, _) => Support::Yes,
            (Kind::Desktop, "hooks") => Support::No("Claude Desktop has no hook events"),
            (Kind::Desktop, "proxy") => Support::No(
                "Claude Desktop has no base-URL setting; its requests do not pass through the proxy",
            ),
            (Kind::Desktop, _) => Support::No(
                "Claude Desktop loads MCP from claude_desktop_config.json; there is no plugin directory to link",
            ),
        }
    }

    fn files(&self, cfg: &Config, kind: Kind) -> Vec<PathBuf> {
        match kind {
            Kind::Desktop => vec![desktop_path()],
            Kind::Cli => vec![
                cfg.setup.claude.settings_path.clone(),
                cfg.doctor.claude_json.clone(),
            ],
        }
    }

    fn installed(&self, cfg: &Config, kind: Kind) -> Vec<&'static str> {
        if kind == Kind::Desktop {
            return if mcp::has_entry(&desktop_path(), "mcpServers", "rtok") {
                vec!["mcp"]
            } else {
                vec![]
            };
        }
        let s = super::read(&cfg.setup.claude.settings_path);
        // The installed plugin serves the hooks (D21); MCP is independent (T275) and reported
        // from `~/.claude.json` alone, whether the plugin is installed or not.
        let plugin = plugin_installed(cfg);
        let files = files_serve_rtok(cfg);
        let mut out = Vec::new();
        if files.contains(&"hooks") || plugin {
            out.push("hooks");
        }
        if files.contains(&"mcp") {
            out.push("mcp");
        }
        // The URL `register_proxy` writes. Matching the default port `8790` anywhere in the
        // file missed a proxy on another `[proxy] port`.
        let base = serde_json::from_str::<Value>(&s)
            .ok()
            .and_then(|v| v["env"]["ANTHROPIC_BASE_URL"].as_str().map(str::to_string));
        if base.as_deref() == Some(super::anthropic_proxy_url(cfg).as_str()) {
            out.push("proxy");
        }
        if plugin {
            out.push("plugin");
        }
        out
    }

    fn apply(&self, cfg: &Config, kind: Kind, mode: Mode) -> Result<Vec<String>> {
        let remove = mode == Mode::Remove;
        if kind == Kind::Desktop {
            let spec = mcp_spec(cfg, kind);
            // `--replace` is about Claude Code's hooks; on the desktop it is a plain install.
            // Install and update always write this entry; only `remove` takes it out (T275) —
            // the plugin no longer serves MCP, so there is no second rtok server to guard
            // against here (D21, T243, T244 amended).
            return Ok(vec![
                if remove {
                    mcp::run(cfg, &spec, McpMode::Remove)?
                } else if cfg.setup.mcp {
                    let update = mode == Mode::Update;
                    mcp::run(
                        cfg,
                        &spec,
                        if update {
                            McpMode::Update
                        } else {
                            McpMode::Install
                        },
                    )?
                } else {
                    NO_CHANGES.into()
                },
                super::skill::sync("claude", cfg, remove)?,
            ]);
        }
        match mode {
            Mode::Replace => Ok(vec![
                migrate::run(cfg)?,
                super::skill::sync("claude", cfg, false)?,
            ]),
            Mode::Remove => Ok(vec![
                plugin(cfg, true)?,
                run(cfg, true)?,
                unregister_mcp(cfg)?,
                crate::agents::proxy_env::unregister_proxy(cfg)?,
                super::skill::sync("claude", cfg, true)?,
            ]),
            Mode::Install | Mode::Update => {
                // Offer first: once the plugin is installed it is the only call path for hooks
                // (D21 singleton), so the settings-file hooks are stripped, not added — judged
                // by Claude's own record, so a dry run or a declined offer still gets the
                // settings-file install. MCP is independent of the plugin (T275): it always
                // writes to `~/.claude.json`, plugin or no plugin.
                let mut lines = vec![if mode == Mode::Update {
                    plugin_update(cfg)?
                } else {
                    plugin(cfg, false)?
                }];
                if plugin_installed(cfg) {
                    lines.push(run(cfg, true)?);
                } else {
                    lines.push(run(cfg, false)?);
                }
                if cfg.setup.mcp {
                    lines.push(register_mcp(cfg)?);
                }
                if cfg.setup.proxy {
                    lines.push(crate::agents::proxy_env::register_proxy(cfg)?);
                }
                lines.push(super::skill::sync("claude", cfg, false)?);
                Ok(lines)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn cfg(path: std::path::PathBuf, dry: bool) -> Config {
        let mut c = Config::default();
        c.setup.claude.settings_path = path;
        c.setup.dry_run = dry;
        c.setup.backup = false;
        c
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rtok-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("settings.json")
    }

    #[test]
    fn dry_run_empty_is_ten_additions() {
        let path = tmp("setup-dry");
        let report = run(&cfg(path.clone(), true), false).unwrap();
        assert!(report.contains("10 additions"), "{report}");
        assert!(
            report.contains(&format!("+ SessionEnd {}", command("rtok", "SessionEnd"))),
            "{report}"
        );
        // T130.2: the spawn brief's event is installed for Claude (and only Claude).
        assert!(
            report.contains(&format!(
                "+ SubagentStart {}",
                command("rtok", "SubagentStart")
            )),
            "{report}"
        );
        // T159: the worktree hooks are the plugin's alone.
        assert!(!report.contains("Worktree"), "{report}");
        assert!(!path.exists());
    }

    /// T174 check: the settings-file command this module writes for a bare `rtok` bin — run
    /// as Claude Code itself would run it, `/bin/sh -c <command>` — resolves PATH, then
    /// `~/.ketch/bin/rtok`, then fails open: exit 0, empty stdout on PreToolUse/PostToolUse,
    /// and exactly one `hookSpecificOutput` note on SessionStart naming the ketch install.
    #[cfg(unix)]
    #[test]
    fn command_resolves_rtok_at_run_time_and_fails_open_silently() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::{Command as Proc, Stdio};

        let home = tmp("t174-settings-command").parent().unwrap().to_path_buf();
        let empty_path = home.join("empty-path");
        fs::create_dir_all(&empty_path).unwrap();
        let run = |event: &str, home: &std::path::Path, path: &std::path::Path| {
            let mut child = Proc::new("/bin/sh")
                .args(["-c", &command("rtok", event)])
                .env("HOME", home)
                .env("PATH", path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            drop(child.stdin.take());
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "{event}: {out:?}");
            String::from_utf8(out.stdout).unwrap()
        };

        // No `rtok` anywhere: silent on a regular event, one note on SessionStart.
        assert_eq!(run("PreToolUse", &home, &empty_path), "");
        assert_eq!(run("PostToolUse", &home, &empty_path), "");
        let note = run("SessionStart", &home, &empty_path);
        let v: Value = serde_json::from_str(&note).unwrap_or_else(|e| panic!("{note}: {e}"));
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "SessionStart");
        assert!(
            v["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap()
                .contains("ketch install pyrlyn/rtok"),
            "{note}"
        );

        // `~/.ketch/bin/rtok` exists: it gets exec'd instead of the fail-open note.
        let ketch = home.join(".ketch/bin/rtok");
        fs::create_dir_all(ketch.parent().unwrap()).unwrap();
        fs::write(&ketch, "#!/bin/sh\nprintf 'ketch %s' \"$2\"\n").unwrap();
        fs::set_permissions(&ketch, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            run("SessionStart", &home, &empty_path),
            "ketch SessionStart"
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn mcp_dry_run_then_apply_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("rtok-mcp-setup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("claude.json");
        let mut c = Config::default();
        c.doctor.claude_json = path.clone();
        c.setup.backup = false;
        c.setup.dry_run = true;
        let dry = register_mcp(&c).unwrap();
        assert!(dry.contains("mcpServers.rtok"), "{dry}");
        assert!(!path.exists());
        c.setup.dry_run = false;
        assert!(register_mcp(&c).unwrap().contains("rtok mcp"));
        assert_eq!(register_mcp(&c).unwrap(), NO_CHANGES);
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"mcp\""), "{raw}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn apply_twice_then_remove_keeps_foreign() {
        let path = tmp("setup-apply");
        fs::write(&path, r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"echo other"}]}]}}"#).unwrap();
        let first = run(&cfg(path.clone(), false), false).unwrap();
        assert!(first.contains("10 additions"), "{first}");
        assert_eq!(run(&cfg(path.clone(), false), false).unwrap(), NO_CHANGES);
        let rm = run(&cfg(path.clone(), false), true).unwrap();
        assert!(rm.contains("removed"), "{rm}");
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("echo other") && !raw.contains("rtok hook"));
    }

    /// A user command that merely contains `rtok hook <event>` is theirs: remove keeps it.
    #[test]
    fn remove_keeps_a_user_command_that_chains_rtok() {
        let path = tmp("setup-chain");
        let chain = "notify-send hi && rtok hook PreToolUse";
        fs::write(
            &path,
            json!({"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":chain}]}]}})
                .to_string(),
        )
        .unwrap();
        run(&cfg(path.clone(), false), false).unwrap();
        run(&cfg(path.clone(), false), true).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains(chain), "{raw}");
        assert!(!raw.contains("\"rtok hook PreToolUse\""), "{raw}");
    }

    /// The desktop file lives under the platform's app-support dir and gets the absolute
    /// binary, since the app has no shell PATH. Remove takes it back out.
    #[test]
    fn desktop_writes_absolute_rtok_into_claude_desktop_config() {
        let p = desktop_path();
        assert!(
            p.ends_with("Claude/claude_desktop_config.json"),
            "{}",
            p.display()
        );
        if cfg!(target_os = "macos") {
            assert!(p.to_string_lossy().contains("Library/Application Support"));
        } else if !cfg!(windows) {
            assert!(p.to_string_lossy().contains("/.config/"));
        }
        assert!(std::path::Path::new(&desktop_command()).is_absolute());
        assert_eq!(Claude.support(Kind::Desktop, "mcp"), Support::Yes);
        assert!(matches!(
            Claude.support(Kind::Desktop, "hooks"),
            Support::No(_)
        ));
        // Install/remove through `rtok_mcp` at a temp path, the way `apply` does (D29: never
        // the real `desktop_path()`, which is not overridable from a Config).
        let path = tmp("desktop-mcp");
        let c = cfg(path.clone(), false);
        let spec = desktop_spec_at(&path);
        mcp::run(&c, &spec, McpMode::Install).unwrap();
        // Compare the parsed value: a Windows path's `\` is `\\` in the raw JSON (T83.5).
        let raw = fs::read_to_string(&path).unwrap();
        let written: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            written["mcpServers"]["rtok"]["command"],
            json!(desktop_command()),
            "{raw}"
        );
        assert!(mcp::has_entry(&path, "mcpServers", "rtok"));
        assert_ne!(mcp::run(&c, &spec, McpMode::Remove).unwrap(), NO_CHANGES);
        assert!(!fs::read_to_string(&path).unwrap().contains("\"rtok\""));
        assert!(!mcp::has_entry(&path, "mcpServers", "rtok"));
    }

    /// T275: install and update take the exact same branch for the desktop entry — `cfg.setup.mcp`
    /// gates a plain register, whatever the plugin's state; there is nothing left to guard
    /// against a "second rtok server" (the plugin no longer serves MCP). Table over both
    /// surfaces and every mode (T277.1): the first install or update writes the entry, the
    /// rest are the idempotent re-run, and remove takes it out, keeping the file's other keys.
    #[test]
    fn install_update_and_remove_write_the_same_entry_on_both_surfaces() {
        for (name, desktop) in [("cli", false), ("desktop", true)] {
            let spec_at = |p: &Path| {
                if desktop {
                    desktop_spec_at(p)
                } else {
                    cli_spec_fn(p)
                }
            };
            let path = tmp(&format!("mcp-both-{name}"));
            fs::write(&path, r#"{"keep":{"me":1}}"#).unwrap();
            let c = cfg(path.clone(), false);
            let spec = spec_at(&path);
            let steps = [
                (McpMode::Update, false),
                (McpMode::Install, true),
                (McpMode::Update, true),
            ];
            for (mode, idempotent) in steps {
                let line = mcp::run(&c, &spec, mode).unwrap();
                assert_eq!(line == NO_CHANGES, idempotent, "{name} {mode:?}: {line}");
                assert!(
                    line == NO_CHANGES || line.starts_with("mcpServers.rtok: "),
                    "{line}"
                );
            }
            let raw: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(raw["mcpServers"]["rtok"]["type"], "stdio", "{name}");
            assert_eq!(
                raw["mcpServers"]["rtok"]["args"],
                json!(["mcp", "--host", "claude"]),
                "{name}"
            );
            assert_eq!(raw["keep"], json!({"me": 1}), "{name}");

            let removed = mcp::run(&c, &spec, McpMode::Remove).unwrap();
            assert_eq!(removed, "- mcpServers.rtok", "{name}");
            let raw: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(raw, json!({"keep": {"me": 1}}), "{name}");
        }
    }

    fn cli_spec_fn(path: &Path) -> McpSpec {
        mcp_spec_at(McpClient::Cli, path.into(), crate::agents::rtok_command())
    }

    fn desktop_spec_at(path: &Path) -> McpSpec {
        mcp_spec_at(McpClient::Desktop, path.into(), desktop_command())
    }

    /// `--dry-run` reports the write and leaves the file alone; a missing `~/.claude.json`
    /// is not created by it.
    #[test]
    fn dry_run_reports_the_entry_without_creating_the_file() {
        let path = tmp("mcp-dry-run");
        let c = cfg(path.clone(), true);
        let line = mcp::run(&c, &cli_spec_fn(&path), McpMode::Install).unwrap();
        assert!(line.starts_with("mcpServers.rtok: "), "{line}");
        assert!(!path.exists());
    }

    /// T246 via the Claude wrapper: an entry the user pointed elsewhere is left alone on
    /// `remove`, with a `leave` line — for both surfaces MCP now always writes to (T275).
    #[test]
    fn remove_leaves_a_user_edited_mcp_entry_on_both_surfaces() {
        // Still runs rtok (same bin), but with an argument the installer never writes — T246's
        // "changed by you" case, not a foreign server that merely happens to be named `rtok`.
        let edited =
            json!({"mcpServers": {"rtok": {"command": "rtok", "args": ["mcp", "--custom"]}}});
        let cli = tmp("cli-user-edited");
        fs::write(&cli, edited.to_string()).unwrap();
        let mut c = Config::default();
        c.doctor.claude_json = cli.clone();
        c.setup.backup = false;

        let report = unregister_mcp(&c).unwrap();
        assert!(
            report.starts_with("leave mcpServers.rtok") && report.contains("changed by you"),
            "{report}"
        );
        assert!(
            mcp::has_entry(&cli, "mcpServers", "rtok"),
            "kept, not removed"
        );

        let desktop = tmp("desktop-user-edited");
        fs::write(&desktop, edited.to_string()).unwrap();
        let report = mcp::run(&c, &desktop_spec_at(&desktop), McpMode::Remove).unwrap();
        assert!(
            report.starts_with("leave mcpServers.rtok") && report.contains("changed by you"),
            "{report}"
        );
        assert!(
            mcp::has_entry(&desktop, "mcpServers", "rtok"),
            "kept, not removed"
        );
    }

    /// T275 (d)/(f): the plugin being installed no longer implies `mcp` — `installed()` reads
    /// it from `~/.claude.json` alone. Regression for the creator's report: a plugin-installed
    /// Claude Code with no entry in the file must not show `mcp`, and the file getting the
    /// entry (independent of the plugin) makes it show.
    #[test]
    fn cli_installed_reports_mcp_only_from_its_own_file_not_the_plugin() {
        let dir = tmp("cli-installed-mcp").parent().unwrap().to_path_buf();
        fs::create_dir_all(dir.join("plugins")).unwrap();
        fs::write(
            dir.join("plugins/installed_plugins.json"),
            r#"{"rtok@rtok": {"version": "1"}}"#,
        )
        .unwrap();
        let mut c = Config::default();
        c.setup.claude.settings_path = dir.join("settings.json");
        c.doctor.claude_json = dir.join("claude.json");
        c.setup.backup = false;

        // Plugin installed, no MCP entry yet: hooks and plugin show, mcp does not.
        let out = Claude.installed(&c, Kind::Cli);
        assert!(out.contains(&"hooks"), "{out:?}");
        assert!(out.contains(&"plugin"), "{out:?}");
        assert!(
            !out.contains(&"mcp"),
            "the plugin no longer implies mcp: {out:?}"
        );

        // Once `~/.claude.json` carries the entry, mcp shows too — the plugin state is
        // unrelated (T275).
        register_mcp(&c).unwrap();
        let out = Claude.installed(&c, Kind::Cli);
        assert!(out.contains(&"mcp"), "{out:?}");

        let _ = fs::remove_dir_all(&dir);
    }

    /// Windows keeps the desktop file under `%APPDATA%\Claude`, not under the profile root.
    #[cfg(windows)]
    #[test]
    fn desktop_path_lives_under_appdata_on_windows() {
        let appdata = std::env::var_os("APPDATA").expect("APPDATA");
        assert!(
            desktop_path().starts_with(&appdata),
            "{}",
            desktop_path().display()
        );
        assert!(desktop_command().ends_with(".exe"), "{}", desktop_command());
    }

    // --- Vfs twins (T56.3): hook insert/strip against in-memory fixtures; keep disk e2e ---

    fn hooks_roundtrip_vfs(vfs: &mut crate::testutil::Vfs, path: &str, remove: bool) -> String {
        let raw = vfs.read_str(path).unwrap_or("{}");
        let mut root: Value =
            serde_json::from_str(if raw.is_empty() { "{}" } else { raw }).unwrap();
        let report = if remove {
            let yes = Apply {
                yes: true,
                ..Apply::default()
            };
            let at = std::path::Path::new(path);
            strip_ours(
                &yes,
                at,
                root.get_mut("hooks"),
                claude_entries(),
                "timeout",
                5,
            )
        } else {
            insert_ours(
                object_at(&mut root, "hooks"),
                claude_entries(),
                "rtok",
                "timeout",
                5,
            )
        };
        vfs.write(path, serde_json::to_string_pretty(&root).unwrap());
        report
    }

    #[test]
    fn apply_twice_then_remove_keeps_foreign_from_vfs() {
        let mut vfs = crate::testutil::Vfs::new();
        let path = "settings.json";
        vfs.write(
            path,
            r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"echo other"}]}]}}"#,
        );
        let first = hooks_roundtrip_vfs(&mut vfs, path, false);
        assert!(first.contains("10 additions"), "{first}");
        assert_eq!(hooks_roundtrip_vfs(&mut vfs, path, false), NO_CHANGES);
        let rm = hooks_roundtrip_vfs(&mut vfs, path, true);
        assert!(rm.contains("removed"), "{rm}");
        let raw = vfs.read_str(path).unwrap();
        assert!(
            raw.contains("echo other") && !raw.contains("rtok hook"),
            "{raw}"
        );
    }

    #[test]
    fn remove_keeps_a_user_command_that_chains_rtok_from_vfs() {
        let mut vfs = crate::testutil::Vfs::new();
        let path = "settings.json";
        let chain = "notify-send hi && rtok hook PreToolUse";
        vfs.write(
            path,
            json!({"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":chain}]}]}})
                .to_string(),
        );
        hooks_roundtrip_vfs(&mut vfs, path, false);
        hooks_roundtrip_vfs(&mut vfs, path, true);
        let raw = vfs.read_str(path).unwrap();
        assert!(raw.contains(chain), "{raw}");
        assert!(!raw.contains("\"rtok hook PreToolUse\""), "{raw}");
    }

    #[test]
    fn wrong_shaped_hooks_key_is_replaced_from_vfs() {
        for body in [
            r#"{"hooks":[]}"#,
            r#"{"hooks":"nope"}"#,
            r#"{"hooks":{"PreToolUse":"nope"}}"#,
        ] {
            let mut vfs = crate::testutil::Vfs::new();
            let path = "settings.json";
            vfs.write(path, body);
            let report = hooks_roundtrip_vfs(&mut vfs, path, false);
            assert!(report.contains("10 additions"), "{body} → {report}");
            let root: Value = serde_json::from_str(vfs.read_str(path).unwrap()).unwrap();
            assert!(root["hooks"]["PreToolUse"].is_array(), "{body}");
        }
    }

    #[test]
    fn dry_run_empty_is_ten_additions_from_vfs() {
        let mut vfs = crate::testutil::Vfs::new();
        // Absent file → empty object; dry-run style: mutate report only, do not require prior write.
        let path = "Users/Ivan Tuhai/.claude/settings.json";
        let report = hooks_roundtrip_vfs(&mut vfs, path, false);
        assert!(report.contains("10 additions"), "{report}");
        assert!(
            report.contains(&format!("+ SessionEnd {}", command("rtok", "SessionEnd"))),
            "{report}"
        );
        // Vfs now holds the written body (unlike disk dry_run); assert shape instead of absence.
        let root: Value = serde_json::from_str(vfs.read_str(path).unwrap()).unwrap();
        assert!(root["hooks"]["SessionEnd"].is_array());
    }

    /// A settings file whose `hooks` is not an object is user data, not a reason to panic
    /// (`hooks: []` used to hit `as_object_mut().unwrap()`).
    #[test]
    fn wrong_shaped_hooks_key_is_replaced_not_panicked_on() {
        for body in [
            r#"{"hooks":[]}"#,
            r#"{"hooks":"nope"}"#,
            r#"{"hooks":{"PreToolUse":"nope"}}"#,
        ] {
            let path = tmp(&format!("setup-shape-{}", body.len()));
            fs::write(&path, body).unwrap();
            let report = run(&cfg(path.clone(), false), false).unwrap();
            assert!(report.contains("10 additions"), "{body} → {report}");
            let root: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            assert!(root["hooks"]["PreToolUse"].is_array(), "{body}");
        }
    }

    /// T242.1: an rtok hook from an older binary path or timeout is rewritten in its own slot,
    /// the foreign hook beside it stays, and a second pass changes nothing.
    #[test]
    fn stale_bin_and_timeout_are_rewritten_in_place() {
        let mut hooks = json!({"PreToolUse":[{"matcher":"Bash","hooks":[
            {"type":"command","command":"echo mine"},
            {"type":"command","command":"/old/store/rtok/v0.1.0/rtok hook PreToolUse","timeout":3}
        ]}]});
        let want = command("rtok", "PreToolUse");
        let report = insert_ours(&mut hooks, &claude_entries()[..1], "rtok", "timeout", 7);
        assert_eq!(
            report,
            format!("~ PreToolUse Bash {want}\n1 updates"),
            "{report}"
        );
        let inner = &hooks["PreToolUse"][0]["hooks"];
        assert_eq!(inner[0]["command"], "echo mine");
        assert_eq!(inner[1]["command"], want);
        assert_eq!(inner[1]["timeout"], 7);
        assert_eq!(hooks["PreToolUse"].as_array().unwrap().len(), 1);
        let before = hooks.clone();
        assert_eq!(
            insert_ours(&mut hooks, &claude_entries()[..1], "rtok", "timeout", 7),
            NO_CHANGES
        );
        assert_eq!(hooks, before);
    }

    /// T242.1: a pair rtok no longer installs (an old matcher, a dropped event) loses its rtok
    /// hook — foreign hooks on it stay — and the current pair is added, so one event never
    /// ends up with two rtok hooks.
    #[test]
    fn a_pair_rtok_no_longer_installs_is_pruned() {
        let mut hooks = json!({
            "PostToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"rtok hook PostToolUse"}]}],
            "Stop":[{"hooks":[
                {"type":"command","command":"rtok hook Stop"},
                {"type":"command","command":"notify-send done"}
            ]}],
            "Notification":[{"hooks":[{"type":"command","command":"rtok hook Notification"}]}]
        });
        let entries = [("PostToolUse", "*")];
        let report = insert_ours(&mut hooks, &entries, "rtok", "timeout", 5);
        assert!(report.ends_with("1 additions, 3 removals"), "{report}");
        assert!(
            report.contains("- PostToolUse Bash rtok hook PostToolUse"),
            "{report}"
        );
        let post = hooks["PostToolUse"].as_array().unwrap();
        assert_eq!(post.len(), 1, "{hooks}");
        assert_eq!(post[0]["matcher"], "*");
        assert_eq!(
            hooks["Stop"][0]["hooks"],
            json!([{"type":"command","command":"notify-send done"}])
        );
        assert!(hooks.get("Notification").is_none(), "{hooks}");
    }

    /// T242.1: re-running install on a current file writes nothing — the bytes stay.
    #[test]
    fn current_file_stays_byte_identical() {
        let path = tmp("setup-current");
        let c = cfg(path.clone(), false);
        run(&c, false).unwrap();
        let first = fs::read(&path).unwrap();
        assert_eq!(run(&c, false).unwrap(), NO_CHANGES);
        assert_eq!(fs::read(&path).unwrap(), first);
    }

    /// T114: `plugins/claude` carries the installer's hooks (no MCP of its own since T275) and
    /// the marketplace the directory is its own marketplace (`claude plugin marketplace add
    /// plugins/claude`).
    #[test]
    fn plugin_tree_matches_the_installer() {
        use crate::agents::hook_events;
        let parse = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        let hooks = parse(include_str!("../../../plugins/claude/hooks/hooks.json"));
        let timeout = Config::default().setup.hook_timeout_s;
        // T262.1: the file is the list; T390.1: the table's `claude` rows say the same, and
        // Kimi's rows are those minus `SubagentStart`.
        assert_eq!(claude_entries(), hook_events::entries("claude"));
        assert!(claude_entries().contains(&("SubagentStart", "")));
        let mut kimi_expected = claude_entries().to_vec();
        kimi_expected.retain(|&(event, _)| event != "SubagentStart");
        assert_eq!(hook_events::entries("kimi"), kimi_expected);
        let mut want = json!({});
        for &(event, matcher) in claude_entries() {
            // T178: prefer the tiny `rtok-hook` client (resident), then `rtok hook`, then
            // `hook.sh` when neither binary is on PATH (desktop app, fail-open hint).
            let cmd = format!(
                "command -v rtok-hook >/dev/null 2>&1 && exec rtok-hook {event}; \
                 command -v rtok >/dev/null 2>&1 && exec rtok hook {event}; \
                 exec \"${{CLAUDE_PLUGIN_ROOT}}/scripts/hook.sh\" {event}"
            );
            let mut e = json!({"hooks": [{"type": "command", "command": cmd, "timeout": timeout}]});
            if !matcher.is_empty() {
                e["matcher"] = json!(matcher);
            }
            array_at(&mut want, event).push(e);
        }
        // T159: once each, plugin only (D21 singleton), through the launcher with its own
        // fallback and a timeout that fits a fetch plus `git worktree add`.
        for event in ["WorktreeCreate", "WorktreeRemove"] {
            let cmd = format!("\"${{CLAUDE_PLUGIN_ROOT}}/scripts/worktree.sh\" {event}");
            let entry = json!({"hooks": [{"type": "command", "command": cmd, "timeout": 120}]});
            array_at(&mut want, event).push(entry);
        }
        assert_eq!(hooks["hooks"], want);
        let manifest = parse(include_str!(
            "../../../plugins/claude/.claude-plugin/plugin.json"
        ));
        let market = parse(include_str!(
            "../../../plugins/claude/.claude-plugin/marketplace.json"
        ));
        assert_eq!(market["plugins"][0]["name"], manifest["name"]);
        assert_eq!(market["plugins"][0]["source"], "./");
    }

    /// T132: the shipped scout stays cheap (`model: haiku`) and scoped to the rtok MCP tool
    /// names Claude Code resolves for the plain `mcpServers.rtok` config entry (`mcp__<server>__
    /// <tool>`, T275 — the plugin ships no `.mcp.json` of its own, so the scoped
    /// `mcp__plugin_<plugin>_<server>__<tool>` form no longer applies).
    #[test]
    fn scout_agent_ships_with_the_cheap_scoped_frontmatter() {
        let text = include_str!("../../../plugins/claude/agents/rtok-scout.md");
        let front = text
            .strip_prefix("---\n")
            .and_then(|s| s.split_once("\n---\n"))
            .expect("frontmatter fenced by `---`")
            .0;
        assert!(front.contains("name: rtok-scout"), "{front}");
        assert!(front.contains("model: haiku"), "{front}");
        for tool in ["read", "search", "outline", "explore", "expand"] {
            let want = format!("mcp__rtok__{tool}");
            assert!(front.contains(&want), "{front}: missing {want}");
        }
        assert!(
            !front.contains("mcp__plugin_rtok_rtok__"),
            "{front}: stale plugin-scoped MCP name"
        );
    }

    // --- T139: `plugin()` decision logic — installed/known-marketplace/fresh, no real `claude` ---

    fn plugin_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rtok-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("plugins")).unwrap();
        dir
    }

    fn plugin_cfg(dir: &std::path::Path, dry: bool) -> Config {
        let mut c = Config::default();
        c.setup.claude.settings_path = dir.join("settings.json");
        c.setup.dry_run = dry;
        c.setup.backup = false;
        // D29: never let a test's receipt read/write touch the real state directory.
        c.plugin_receipt_path = Some(dir.join("plugins.json"));
        c
    }

    /// Already installed → no-op, whether or not `--dry-run` is given.
    #[test]
    fn plugin_install_is_a_no_op_once_claude_already_has_it() {
        let dir = plugin_dir("plugin-installed");
        fs::write(
            dir.join("plugins/installed_plugins.json"),
            r#"{"rtok@rtok":{}}"#,
        )
        .unwrap();
        assert_eq!(plugin(&plugin_cfg(&dir, false), false).unwrap(), NO_CHANGES);
        assert_eq!(plugin(&plugin_cfg(&dir, true), false).unwrap(), NO_CHANGES);
        let _ = fs::remove_dir_all(dir);
    }

    /// Not installed and not removing it either → no-op (nothing to uninstall).
    #[test]
    fn plugin_remove_is_a_no_op_when_not_installed() {
        let dir = plugin_dir("plugin-remove-noop");
        assert_eq!(plugin(&plugin_cfg(&dir, false), true).unwrap(), NO_CHANGES);
        let _ = fs::remove_dir_all(dir);
    }

    /// A marketplace already pointed at the GitHub repo skips `marketplace add`: only the
    /// install step is offered. Shape verified against a real `known_marketplaces.json`
    /// this machine wrote for `claude plugin marketplace add pyrlyn/rtok`.
    #[test]
    fn plugin_dry_run_skips_marketplace_add_when_already_known() {
        let dir = plugin_dir("plugin-known-market");
        fs::write(
            dir.join("plugins/known_marketplaces.json"),
            r#"{"rtok":{"source":{"source":"github","repo":"pyrlyn/rtok"}}}"#,
        )
        .unwrap();
        let report = plugin(&plugin_cfg(&dir, true), false).unwrap();
        assert!(
            report.contains("claude plugin install rtok@rtok"),
            "{report}"
        );
        assert!(!report.contains("marketplace add"), "{report}");
        assert!(!report.contains("marketplace remove"), "{report}");
        let _ = fs::remove_dir_all(dir);
    }

    /// Nothing known yet: both steps are offered, from the GitHub marketplace, not a local path.
    #[test]
    fn plugin_dry_run_offers_both_steps_from_a_clean_install() {
        let dir = plugin_dir("plugin-fresh");
        let report = plugin(&plugin_cfg(&dir, true), false).unwrap();
        assert!(
            report.contains("claude plugin marketplace add pyrlyn/rtok"),
            "{report}"
        );
        assert!(
            report.contains("claude plugin install rtok@rtok"),
            "{report}"
        );
        assert!(report.starts_with("offer plugins/claude → "), "{report}");
        let _ = fs::remove_dir_all(dir);
    }

    /// A `"rtok"` marketplace registered under the pre-T139 local ketch-store path (the
    /// literal shape a real `known_marketplaces.json` on this machine carried) is removed
    /// before it is re-added from GitHub — `marketplace add` on top of an existing entry
    /// errors, it does not re-point it.
    #[test]
    fn plugin_dry_run_repoints_a_stale_local_marketplace() {
        let dir = plugin_dir("plugin-stale-market");
        fs::write(
            dir.join("plugins/known_marketplaces.json"),
            r#"{"rtok":{"source":{"source":"directory","path":"/Users/x/.ketch/store/rtok/v0.6.3/plugins/claude"},"installLocation":"/Users/x/.ketch/store/rtok/v0.6.3/plugins/claude"}}"#,
        )
        .unwrap();
        let report = plugin(&plugin_cfg(&dir, true), false).unwrap();
        assert!(
            report.contains("claude plugin marketplace remove rtok && claude plugin marketplace add pyrlyn/rtok && claude plugin install rtok@rtok"),
            "{report}"
        );
        assert!(!report.contains("uninstall"), "{report}");
        let _ = fs::remove_dir_all(dir);
    }

    /// A marketplace added before the repository moved from `listepo` to `pyrlyn` still
    /// records `listepo/rtok`. GitHub redirects that name today, but nothing guarantees it
    /// keeps doing so, so the entry reads as stale and is re-pointed at the new repo.
    #[test]
    fn plugin_dry_run_repoints_a_marketplace_added_under_the_old_repo_name() {
        let dir = plugin_dir("plugin-old-repo-market");
        fs::write(
            dir.join("plugins/known_marketplaces.json"),
            r#"{"rtok":{"source":{"source":"github","repo":"listepo/rtok"}}}"#,
        )
        .unwrap();
        let report = plugin(&plugin_cfg(&dir, true), false).unwrap();
        assert!(
            report.contains("claude plugin marketplace remove rtok && claude plugin marketplace add pyrlyn/rtok && claude plugin install rtok@rtok"),
            "{report}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// The plugin already shows as installed, but `installed_plugins.json` carries no field
    /// naming its marketplace (verified against a real file this machine wrote) — the stale
    /// local marketplace is the only signal there is, so the plugin is uninstalled and
    /// reinstalled from GitHub instead of the usual already-installed no-op.
    #[test]
    fn plugin_dry_run_reinstalls_when_already_installed_from_a_stale_marketplace() {
        let dir = plugin_dir("plugin-stale-installed");
        fs::write(
            dir.join("plugins/installed_plugins.json"),
            r#"{"rtok@rtok":{}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("plugins/known_marketplaces.json"),
            r#"{"rtok":{"source":{"source":"directory","path":"/Users/x/.ketch/store/rtok/v0.6.3/plugins/claude"}}}"#,
        )
        .unwrap();
        let report = plugin(&plugin_cfg(&dir, true), false).unwrap();
        assert_ne!(report, NO_CHANGES);
        assert!(
            report.contains("claude plugin uninstall rtok@rtok && claude plugin marketplace remove rtok && claude plugin marketplace add pyrlyn/rtok && claude plugin install rtok@rtok"),
            "{report}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    // --- T279 PR 3: `plugin_update`'s version decision, no real `claude` (dry-run only) ---

    /// Claude's own record of `rtok@rtok` at `version`, and the `rtok` marketplace `source`.
    fn record(dir: &std::path::Path, version: &str, source: &str) {
        let plugins = dir.join("plugins");
        fs::write(
            plugins.join("installed_plugins.json"),
            format!(r#"{{"version":2,"plugins":{{"rtok@rtok":[{{"scope":"user","version":"{version}"}}]}}}}"#),
        )
        .unwrap();
        let known = format!(r#"{{"rtok":{{"source":{source}}}}}"#);
        fs::write(plugins.join("known_marketplaces.json"), known).unwrap();
    }

    const GITHUB: &str = r#"{"source":"github","repo":"pyrlyn/rtok"}"#;

    /// Same version on record as the one running: skip, name the version, run no `claude`.
    #[test]
    fn plugin_update_skips_when_the_host_record_matches_the_running_version() {
        let dir = plugin_dir("update-skip");
        record(&dir, env!("CARGO_PKG_VERSION"), GITHUB);
        // Up to date and not a dry run: NO_CHANGES, same as every other current-already step,
        // so `agents::mod::block`'s "already current" note still fires.
        let report = plugin_update(&plugin_cfg(&dir, false)).unwrap();
        assert_eq!(report, NO_CHANGES);

        // `--dry-run` still names the version, since a dry run has nothing else to show.
        let dry_report = plugin_update(&plugin_cfg(&dir, true)).unwrap();
        assert!(dry_report.contains("up to date"), "{dry_report}");
        assert!(
            dry_report.contains(env!("CARGO_PKG_VERSION")),
            "{dry_report}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// A legacy install (host record predates T279, no version file, no receipt) always reads
    /// as `0.0.0` — lower than any real release, so it updates once.
    #[test]
    fn plugin_update_treats_a_legacy_install_as_an_update() {
        let dir = plugin_dir("update-legacy");
        record(&dir, "0.0.1", GITHUB);
        let report = plugin_update(&plugin_cfg(&dir, true)).unwrap();
        assert!(report.starts_with("~ plugin"), "{report}");
        assert!(
            report.contains("claude plugin marketplace update rtok"),
            "{report}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// Nothing installed at all: `Decision::Install` runs the same fresh-install steps
    /// [`plugin`] would for `agents install` (T279 step 4's "first `agents update`").
    #[test]
    fn plugin_update_installs_fresh_when_nothing_is_on_record() {
        let dir = plugin_dir("update-fresh");
        let report = plugin_update(&plugin_cfg(&dir, true)).unwrap();
        assert!(report.starts_with("offer plugins/claude → "), "{report}");
        assert!(
            report.contains("claude plugin marketplace add pyrlyn/rtok"),
            "{report}"
        );
        assert!(
            report.contains("claude plugin install rtok@rtok"),
            "{report}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// `--force` reinstalls even at the running version: uninstall then install, no
    /// marketplace repoint since the recorded source is already GitHub.
    #[test]
    fn plugin_update_force_reinstalls_even_when_up_to_date() {
        let dir = plugin_dir("update-force");
        record(&dir, env!("CARGO_PKG_VERSION"), GITHUB);
        let mut c = plugin_cfg(&dir, true);
        c.setup.force = true;
        let report = plugin_update(&c).unwrap();
        assert!(
            report.contains("claude plugin uninstall rtok@rtok && claude plugin install rtok@rtok"),
            "{report}"
        );
        assert!(!report.contains("marketplace"), "{report}");
        let _ = fs::remove_dir_all(dir);
    }

    /// Up to date but installed from the pre-T139 ketch-store marketplace: reinstall and
    /// repoint to GitHub, never an in-place update from the stale path.
    #[test]
    fn plugin_update_repoints_a_stale_marketplace_even_when_up_to_date() {
        let dir = plugin_dir("update-stale");
        let stale = r#"{"source":"directory","path":"/x/.ketch/store/rtok/v0.6.3/plugins/claude"}"#;
        record(&dir, env!("CARGO_PKG_VERSION"), stale);
        let report = plugin_update(&plugin_cfg(&dir, true)).unwrap();
        for step in [
            "uninstall rtok@rtok",
            "marketplace remove rtok",
            "marketplace add pyrlyn/rtok",
        ] {
            assert!(report.contains(step), "{step}: {report}");
        }
        assert!(!report.contains("marketplace update"), "{report}");
        let _ = fs::remove_dir_all(dir);
    }

    /// `--source local` reinstalls from the local plugin tree instead of the GitHub repo.
    #[test]
    fn plugin_update_source_local_installs_from_the_local_tree() {
        let dir = plugin_dir("update-source-local");
        let mut c = plugin_cfg(&dir, true);
        c.setup.source = Some("local".to_string());
        let report = plugin_update(&c).unwrap();
        // The same path the code passes, so Windows separators match too.
        let local = marketplace_target(plugin_version::Source::Local);
        assert!(local.ends_with("claude"), "{local}");
        assert!(
            report.contains(&format!("claude plugin marketplace add {local}")),
            "{report}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// An invalid `--source` value is reported as an error rather than silently defaulting.
    #[test]
    fn plugin_update_rejects_an_unknown_source() {
        let dir = plugin_dir("update-source-bad");
        let mut c = plugin_cfg(&dir, true);
        c.setup.source = Some("npm".to_string());
        let err = plugin_update(&c).unwrap_err().to_string();
        assert!(err.contains("npm"), "{err}");
        let _ = fs::remove_dir_all(dir);
    }
}
