// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! GitHub Copilot installer (`rtok agents install copilot`, plan T46.4).
//!
//! Copilot CLI (`copilot`) and the GitHub Copilot app share `~/.copilot` (`[setup.copilot] dir`):
//! MCP in `mcp-config.json` (`mcpServers.<name> = {type: "local", command, args, tools}`), hooks
//! as any `hooks/*.json` file. rtok owns `hooks/rtok.json` outright — nothing to merge, `remove`
//! deletes it — and its commands run `rtok hook <Event> --host copilot`, which maps Copilot's
//! camelCase payloads (T46.3). The app does not document hooks, so that variant reports `no`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rtok_agent_sdk::{Apply, NO_CHANGES, edit_json};
use rtok_mcp::spec::{Client, EntryShape, McpSpec};
use serde_json::{Value, json};

use super::hook_events;
use super::{Agent, Kind, Mode, Support, Variant, apply};
use crate::config::Config;

const NAME: &str = "rtok";

/// The CLI and the desktop app read the same `~/.copilot` files.
pub struct Copilot;

static VARIANTS: [Variant; 2] = [
    Variant {
        kind: Kind::Cli,
        name: "Copilot CLI",
        bins: &["copilot"],
        apps: &[],
    },
    Variant {
        kind: Kind::Desktop,
        name: "GitHub Copilot",
        bins: &[],
        apps: &[
            "/Applications/GitHub Copilot.app",
            "$LOCALAPPDATA/Programs/GitHub Copilot/GitHub Copilot.exe",
        ],
    },
];

/// `<dir>/mcp-config.json`.
pub fn mcp_path(cfg: &Config) -> PathBuf {
    cfg.setup.copilot.dir.join("mcp-config.json")
}

/// `<dir>/hooks/rtok.json` — rtok's own file; Copilot loads every `hooks/*.json`.
pub fn hooks_path(cfg: &Config) -> PathBuf {
    cfg.setup.copilot.dir.join("hooks").join("rtok.json")
}

impl Agent for Copilot {
    super::agent_identity!("copilot");

    fn shared(&self) -> bool {
        true
    }

    fn support(&self, kind: Kind, module: &str) -> Support {
        match (module, kind) {
            ("hooks", Kind::Cli) | ("mcp", _) => Support::Yes,
            ("hooks", _) => Support::No(
                "the GitHub Copilot app does not document hooks; the shared hooks/rtok.json is written for the CLI",
            ),
            ("proxy", _) => Support::No(
                "Copilot BYOK is env-only (COPILOT_PROVIDER_BASE_URL); there is no config file to point at the proxy",
            ),
            ("plugin", Kind::Cli) => Support::Flag("--yes"),
            ("plugin", _) => Support::No(
                "the GitHub Copilot app does not document plugin installs; `copilot plugin` serves the CLI",
            ),
            _ => Support::No(
                "Copilot plugins live in installed-plugins/, owned by `copilot plugin`; there is no local plugin directory to link",
            ),
        }
    }

    fn plugin_surfaces(&self) -> &'static [rtok_plugin_sdk::Surface] {
        &[rtok_plugin_sdk::Surface::Hook]
    }

    fn files(&self, cfg: &Config, _kind: Kind) -> Vec<PathBuf> {
        vec![hooks_path(cfg), mcp_path(cfg)]
    }

    fn installed(&self, cfg: &Config, _kind: Kind) -> Vec<&'static str> {
        let mut out = Vec::new();
        if super::read(&hooks_path(cfg)).contains(" hook PreToolUse") {
            out.push("hooks");
        }
        if super::mcp::has_entry(&mcp_path(cfg), "mcpServers", NAME) {
            out.push("mcp");
        }
        if plugin_installed(cfg) {
            out.push("plugin");
        }
        out
    }

    fn apply(&self, cfg: &Config, _kind: Kind, mode: Mode) -> Result<Vec<String>> {
        // D21: the plugin is the hooks unit — its own hooks serve already, so rtok's
        // hooks/rtok.json goes instead of coming, on the same run that installs it too.
        // MCP is no longer part of that unit (T275/D33): `mcp-config.json`'s `mcpServers.rtok`
        // is written on every install/update behind `[setup] mcp`, plugin state or not.
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
            skill_sync,
        )
    }
}

/// The [`super::d21_plugin_apply`] `extra` for Copilot: `skill::sync` runs on every apply,
/// install or remove alike (T234).
fn skill_sync(cfg: &Config, remove: bool) -> Result<Option<String>> {
    Ok(Some(super::skill::sync("copilot", cfg, remove)?))
}

/// Write, dry-run, or delete `hooks/rtok.json`. The file is rtok's, so apply means "make it
/// this document" and an identical file is no change.
pub fn run(cfg: &Config, remove: bool) -> Result<String> {
    let path = hooks_path(cfg);
    if remove {
        return remove_file(&apply(cfg), &path);
    }
    let want = hooks_doc(&super::rtok_hook_bin(), cfg.setup.hook_timeout_s);
    edit_json(&apply(cfg), &path, |root| {
        if *root == want {
            return NO_CHANGES.into();
        }
        *root = want;
        format!(
            "+ {} ({} events)",
            path.display(),
            hook_events::installed("copilot").count()
        )
    })
}

/// Copilot's flat `additionalContext` fallback note, shown once on `sessionStart` when `rtok` resolves nowhere.
const MISSING_RTOK_NOTE: &str =
    r#"{"additionalContext":"rtok is not installed; run ketch install pyrlyn/rtok to enable it."}"#;

/// `{version: 1, hooks: {<event>: [{type: "command", bash, powershell, timeoutSec}]}}`.
/// The plugin tree's `hooks/hooks.json` is this document with `bin = "rtok"`, pinned by
/// `tests/copilot_plugin.rs` — one shape, two surfaces (D21).
/// `bin = "rtok"` resolves `bash`/`powershell` at run time ([`super::hook_resolver`] /
/// [`hook_resolver_ps`], `sessionStart` adds [`MISSING_RTOK_NOTE`]); other `bin` stays plain.
pub fn hooks_doc(bin: &str, timeout: u64) -> Value {
    let mut hooks = serde_json::Map::new();
    for (copilot, claude) in hook_events::installed("copilot") {
        let args = format!("hook {claude} --host copilot");
        let note = (copilot == "sessionStart").then_some(MISSING_RTOK_NOTE);
        let (bash, powershell) = if bin == "rtok" {
            (
                super::hook_resolver(&args, note),
                hook_resolver_ps(&args, note),
            )
        } else {
            let cmd = format!("{bin} {args}");
            (cmd.clone(), cmd)
        };
        hooks.insert(
            copilot.into(),
            json!([{"type": "command", "bash": bash, "powershell": powershell, "timeoutSec": timeout}]),
        );
    }
    json!({"version": 1, "hooks": hooks})
}

/// PowerShell twin of [`super::hook_resolver`] (Copilot's only `powershell` field); `& $r` with
/// no pipeline keeps stdin; `note`, if given, is single-quoted; `json` must not contain `'`.
fn hook_resolver_ps(args: &str, note: Option<&str>) -> String {
    let tail = match note {
        Some(json) => {
            debug_assert!(!json.contains('\''), "{json}");
            format!("'{json}'; exit 0")
        }
        None => "exit 0".to_string(),
    };
    format!(
        "$r = (Get-Command rtok -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1).Source; if (-not $r) {{ $k = Join-Path $env:USERPROFILE '.ketch\\bin\\rtok.exe'; if (Test-Path -LiteralPath $k) {{ $r = $k }} }}; if ($r) {{ & $r {args}; exit $LASTEXITCODE }}; {tail}"
    )
}

/// Delete a file rtok owns: backed up like any edit, reported as `- <path>`; absent is no change.
fn remove_file(apply: &Apply, path: &Path) -> Result<String> {
    if !path.exists() {
        return Ok(NO_CHANGES.into());
    }
    let report = format!("- {}", path.display());
    if apply.writes(&report) {
        if apply.backup {
            rtok_agent_sdk::backup(path, apply.backup_files)?;
        }
        fs::remove_file(path).with_context(|| path.display().to_string())?;
    }
    Ok(report)
}

/// `mcpServers.rtok = {type: "local", command, args, tools: ["*"]}` in `mcp-config.json`;
/// `rtok_mcp::ops` writes and removes it (T277.2).
pub(crate) fn mcp_spec(cfg: &Config) -> McpSpec {
    super::mcp::json_spec(
        "copilot",
        Client::Cli,
        mcp_path(cfg),
        "mcpServers",
        super::rtok_mcp_command(),
        EntryShape::LocalAllTools,
    )
}

const PLUGIN_SRC: &str = "plugins/copilot";

/// True while `copilot plugin` has rtok installed (T116): the cached copy under
/// `installed-plugins/` — `MARKETPLACE/PLUGIN-NAME` from a marketplace, `_direct/<id>` from
/// a local path — is the host's own record. `copilot plugin list --json` reports the same
/// state; the manifest answers without spawning the CLI (T75).
pub fn plugin_installed(cfg: &Config) -> bool {
    let root = cfg.setup.copilot.dir.join("installed-plugins");
    let Ok(outer) = fs::read_dir(&root) else {
        return false;
    };
    outer.flatten().any(|o| {
        fs::read_dir(o.path())
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| super::manifest_names(&e.path(), "plugin.json", NAME))
    })
}

/// One `copilot plugin …` call; `COPILOT_HOME` only when `dir` is not the default. Windows
/// shim resolution and stderr handling live in `super::run_cli` (T140).
fn copilot_cli(cfg: &Config, args: &[&str]) -> std::result::Result<(), String> {
    let dir = cfg.setup.copilot.dir.as_path();
    let default = super::home_dir().join(".copilot");
    let env = (dir != default).then_some(("COPILOT_HOME", dir));
    super::run_cli(cfg, "copilot", args, env)
}

/// Offer, install, or uninstall `plugins/copilot` through `copilot plugin` (T116): the
/// documented local-path install (`copilot plugin install <dir>`), uninstall by the
/// manifest's `name`. Behind `--yes` (`Support::Flag`) — rtok never writes
/// `installed-plugins/`, that store is Copilot's, so the flag never turns the printed line
/// into state (`installed()` reads the marker alone). A failing or missing `copilot` keeps
/// the offer open instead of failing the install: the settings-file hooks still go in.
/// Shares its skeleton with `gemini::plugin` through `super::offer_plugin` (D21).
fn plugin(cfg: &Config, remove: bool) -> Result<String> {
    let installed = plugin_installed(cfg);
    super::offer_plugin(
        cfg,
        remove,
        installed,
        super::PluginOffer {
            bin: "copilot",
            name: NAME,
            src_rel: PLUGIN_SRC,
            install_verb: &["plugin", "install"],
            uninstall_verb: &["plugin", "uninstall"],
        },
        |args| copilot_cli(cfg, args),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(name: &str, dry: bool) -> (Config, PathBuf) {
        let dir = std::env::temp_dir().join(format!("rtok-copilot-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut c = crate::testutil::config_in(&dir);
        c.setup.copilot.dir = dir.clone();
        c.setup.dry_run = dry;
        c.setup.backup = false;
        (c, dir)
    }

    /// Copilot's record of an installed rtok plugin: the manifest under `installed-plugins`.
    fn fake_plugin(c: &Config) -> PathBuf {
        let marker = c
            .setup
            .copilot
            .dir
            .join("installed-plugins/_direct/x/plugin.json");
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::write(&marker, r#"{"name":"rtok"}"#).unwrap();
        marker
    }

    #[test]
    fn dry_run_names_the_hook_file_and_creates_nothing() {
        let (c, dir) = cfg("dry", true);
        let out = run(&c, false).unwrap();
        assert!(
            out.starts_with("+ ") && out.ends_with("(7 events)"),
            "{out}"
        );
        assert!(!hooks_path(&c).exists());
        assert!(Copilot.installed(&c, Kind::Cli).is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn apply_writes_seven_events_is_idempotent_and_remove_deletes() {
        let (c, dir) = cfg("apply", false);
        assert!(run(&c, false).unwrap().starts_with("+ "));
        assert_eq!(run(&c, false).unwrap(), NO_CHANGES);
        let doc: Value =
            serde_json::from_str(&fs::read_to_string(hooks_path(&c)).unwrap()).unwrap();
        assert_eq!(doc["version"], 1);
        assert_eq!(doc["hooks"].as_object().unwrap().len(), 7);
        assert!(doc["hooks"].get("preCompact").is_some());
        let pre = &doc["hooks"]["preToolUse"][0];
        assert_eq!(pre["type"], "command");
        assert_eq!(pre["timeoutSec"], 5);
        let bash = pre["bash"].as_str().unwrap();
        assert!(
            bash.contains("exec rtok hook PreToolUse --host copilot"),
            "{bash}"
        );
        assert!(bash.ends_with("; exit 0"), "{bash}");
        let ps = pre["powershell"].as_str().unwrap();
        assert!(ps.contains("hook PreToolUse --host copilot"), "{ps}");
        assert!(ps.ends_with("; exit 0"), "{ps}");
        assert!(
            doc["hooks"]["userPromptSubmitted"][0]["bash"]
                .as_str()
                .unwrap()
                .contains(" hook UserPromptSubmit ")
        );
        assert_eq!(Copilot.installed(&c, Kind::Cli), ["hooks"]);

        let gone = run(&c, true).unwrap();
        assert_eq!(gone, format!("- {}", hooks_path(&c).display()));
        assert!(!hooks_path(&c).exists());
        assert_eq!(run(&c, true).unwrap(), NO_CHANGES);
        let _ = fs::remove_dir_all(dir);
    }

    /// T116: the offer line names the documented local install and runs nothing without
    /// `--yes`; the marker alone answers `installed()` (T75) and stills the offer.
    #[test]
    fn plugin_offers_the_local_install_behind_yes() {
        let (mut c, dir) = cfg("offer", true);
        assert_eq!(plugin(&c, false).unwrap(), NO_CHANGES, "no --yes, no line");
        c.setup.yes = true;
        let s = plugin(&c, false).unwrap();
        assert!(s.contains("plugins/copilot"), "{s}");
        assert!(s.contains("copilot plugin install"), "{s}");
        assert!(s.contains("ketch install pyrlyn/rtok"), "{s}");
        assert!(!plugin_installed(&c));
        let marker = fake_plugin(&c);
        assert!(plugin_installed(&c));
        assert_eq!(plugin(&c, false).unwrap(), NO_CHANGES);
        assert_eq!(
            plugin(&c, true).unwrap(),
            "- plugin rtok (copilot plugin uninstall rtok)"
        );
        fs::write(&marker, r#"{"name":"other"}"#).unwrap();
        assert!(!plugin_installed(&c), "a foreign manifest is not ours");
        let _ = fs::remove_dir_all(dir);
    }

    /// D21: while the plugin serves, its hooks are the unit — an earlier plain install's
    /// `hooks/rtok.json` is taken back, never added again. MCP is not part of that unit any
    /// more (T275/D33): `mcpServers.rtok` stays written independently of the plugin.
    #[test]
    fn the_plugin_is_the_hooks_singleton_but_mcp_stays_independent() {
        let (mut c, dir) = cfg("single", false);
        c.setup.yes = true;
        assert!(run(&c, false).unwrap().starts_with("+ "));
        assert!(
            crate::agents::mcp::install(&c, &mcp_spec(&c))
                .unwrap()
                .starts_with("mcpServers.rtok: ")
        );
        fake_plugin(&c);
        let lines = Copilot.apply(&c, Kind::Cli, Mode::Install).unwrap();
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("- ") && l.contains("hooks")),
            "{lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains("mcpServers.rtok")),
            "no mcp line while nothing changed: {lines:?}"
        );
        assert!(!hooks_path(&c).exists());
        assert!(
            fs::read_to_string(mcp_path(&c))
                .unwrap_or_default()
                .contains("rtok"),
            "D33: mcp stays even while the plugin serves hooks"
        );
        assert_eq!(Copilot.installed(&c, Kind::Cli), ["mcp", "plugin"]);
        let _ = fs::remove_dir_all(dir);
    }

    /// T275/D33 check (a)-(e): install and update always write `mcpServers.rtok` — plugin
    /// installed or not — with the right command/args; only remove takes it out; a
    /// user-edited entry is left with a `leave` line instead of touched.
    #[test]
    fn t275_mcp_entry_always_written_except_on_remove() {
        let (c, dir) = cfg("t275-mcp", false);
        fake_plugin(&c);
        assert!(plugin_installed(&c));

        let path = mcp_path(&c);
        crate::agents::mcp::assert_json_entry_lifecycle(
            &Copilot,
            &c,
            Kind::Cli,
            &path,
            "mcpServers",
            || crate::agents::mcp::install(&c, &mcp_spec(&c)),
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn mcp_entry_is_local_with_all_tools_and_the_app_reports_no_hooks() {
        let (c, dir) = cfg("mcp", false);
        let first = crate::agents::mcp::install(&c, &mcp_spec(&c)).unwrap();
        assert!(first.starts_with("mcpServers.rtok: "), "{first}");
        assert_eq!(
            crate::agents::mcp::install(&c, &mcp_spec(&c)).unwrap(),
            NO_CHANGES
        );
        let doc: Value = serde_json::from_str(&fs::read_to_string(mcp_path(&c)).unwrap()).unwrap();
        let rtok = &doc["mcpServers"]["rtok"];
        assert_eq!(rtok["type"], "local");
        assert_eq!(rtok["args"], json!(["mcp", "--host", "copilot"]));
        assert_eq!(rtok["tools"], json!(["*"]));
        assert_eq!(Copilot.installed(&c, Kind::Desktop), ["mcp"]);
        assert_eq!(
            crate::agents::mcp::remove(&c, &mcp_spec(&c)).unwrap(),
            "- mcpServers.rtok"
        );
        assert!(!fs::read_to_string(mcp_path(&c)).unwrap().contains("rtok"));

        assert!(matches!(Copilot.support(Kind::Cli, "hooks"), Support::Yes));
        assert!(matches!(
            Copilot.support(Kind::Desktop, "hooks"),
            Support::No(_)
        ));
        assert!(Copilot.shared());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn pre_compact_writes_a_checkpoint_note() {
        let (c, dir) = cfg("precompact", false);
        let mut c = c;
        c.hook.host = "copilot".into();
        let pre = serde_json::json!({
            "sessionId": "cop-compact",
            "trigger": "auto"
        });
        let mut out = Vec::new();
        crate::hooks::run("PreCompact", pre.to_string().as_bytes(), &mut out, &c);
        assert_eq!(out, b"{}");
        let note = crate::store::Store::open(&c.core.db_path)
            .unwrap()
            .latest_note("checkpoint:cop-compact")
            .unwrap();
        assert!(note.is_some(), "preCompact must save a checkpoint");
        let _ = fs::remove_dir_all(dir);
    }
}
