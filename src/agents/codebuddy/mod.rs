// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! CodeBuddy Code CLI + WorkBuddy Desktop installer (`rtok agents install codebuddy`, T560.2).
//!
//! One host, two variants over the same file layout under a config directory:
//! `[setup.codebuddy] dir` (CLI, default `~/.codebuddy`, what `CODEBUDDY_CONFIG_DIR` moves) and
//! `workbuddy_dir` (Desktop, default `~/.workbuddy`). Hooks are Claude's shape under the `hooks`
//! key of `settings.json` (the `codebuddy` rows of [`hook_events`]); MCP is `mcpServers.rtok`
//! in the first existing of `.mcp.json`, `mcp.json`, and the legacy `~/.codebuddy.json`
//! (created as `.mcp.json`), edited as JSONC so the user's comments survive.
//!
//! `plugins/codebuddy` is the D21 hooks unit: `--yes` registers it as a local marketplace and
//! installs it through the `codebuddy` CLI, which also serves WorkBuddy when pointed at its
//! directory through `CODEBUDDY_CONFIG_DIR` (unverified: a third-party report, not WorkBuddy's
//! docs). While the plugin is installed the `settings.json` hooks go instead of coming; MCP
//! stays written regardless (T275/D33).

use std::path::{Path, PathBuf};

use anyhow::Result;
use rtok_agent_sdk::{NO_CHANGES, edit_json, object_at};
use serde_json::Value;

use super::claude::HookForm;
use super::{Agent, Kind, Mode, Support, Variant, apply, hook_events, jsonc};
use crate::config::Config;

const NAME: &str = "rtok";
const HOST: &str = "codebuddy";
const PLUGIN_SRC: &str = "plugins/codebuddy";
/// `<plugin>@<marketplace>`; the marketplace is added under the plugin's own name.
const PLUGIN_ID: &str = "rtok@rtok";

/// CodeBuddy Code (`codebuddy` on PATH) and WorkBuddy (its desktop app, which embeds the CLI).
pub struct Codebuddy;

static VARIANTS: [Variant; 2] = [
    Variant {
        kind: Kind::Cli,
        name: "CodeBuddy Code",
        bins: &["codebuddy"],
        apps: &[],
    },
    Variant {
        kind: Kind::Desktop,
        name: "WorkBuddy",
        bins: &[],
        apps: &[
            "/Applications/WorkBuddy.app",
            "~/Applications/WorkBuddy.app",
            "$LOCALAPPDATA/Programs/WorkBuddy",
        ],
    },
];

/// The config directory of a variant.
pub fn dir(cfg: &Config, kind: Kind) -> &Path {
    match kind {
        Kind::Cli => &cfg.setup.codebuddy.dir,
        Kind::Desktop => &cfg.setup.codebuddy.workbuddy_dir,
    }
}

/// `<dir>/settings.json`.
pub fn settings_path(cfg: &Config, kind: Kind) -> PathBuf {
    dir(cfg, kind).join("settings.json")
}

/// CodeBuddy reads the first existing of these (https://www.codebuddy.ai/docs/cli/mcp); with
/// none, `.mcp.json` is created. `~/.codebuddy.json` is the legacy file of the default CLI dir.
pub fn mcp_path(cfg: &Config, kind: Kind) -> PathBuf {
    let d = dir(cfg, kind);
    let home = super::home_dir();
    let mut candidates = vec![d.join(".mcp.json"), d.join("mcp.json")];
    if kind == Kind::Cli && d == home.join(".codebuddy") {
        candidates.push(home.join(".codebuddy.json"));
    }
    candidates
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| d.join(".mcp.json"))
}

fn command(bin: &str, event: &str) -> String {
    super::claude::command_for(HOST, bin, event)
}

fn is_ours(cmd: &str, event: &str) -> bool {
    super::claude::is_ours_for(HOST, cmd, event)
}

/// CodeBuddy's spelling of the hook command; the insert/strip walk is Claude's (same layout).
const FORM: HookForm = HookForm { command, is_ours };

/// Apply, dry-run, or remove rtok's `hooks.<Event>[]` entries of `settings.json`. Foreign
/// hooks stay; one of ours the user edited goes only as [`super::takes_hook`] says.
pub fn run(cfg: &Config, kind: Kind, remove: bool) -> Result<String> {
    let (a, path) = (apply(cfg), settings_path(cfg, kind));
    let (entries, timeout) = (hook_events::entries(HOST), cfg.setup.hook_timeout_s);
    edit_json(&a, &path, |root| {
        if remove {
            let hooks = root.get_mut("hooks");
            super::claude::strip_ours_as(&FORM, &a, &path, hooks, &entries, "timeout", timeout)
        } else {
            let (hooks, bin) = (object_at(root, "hooks"), super::rtok_hook_bin());
            super::claude::insert_ours_as(&FORM, hooks, &entries, &bin, "timeout", timeout)
        }
    })
}

fn mcp_entry(cmd: &str) -> Value {
    rtok_agent_sdk::mcp_entry(cmd, &super::mcp_args(HOST))
}

/// `mcpServers.rtok = {type: "stdio", command, args}` in the MCP file [`mcp_path`] picks.
pub fn register_mcp(cfg: &Config, kind: Kind) -> Result<String> {
    let cmd = super::rtok_command();
    jsonc::upsert_entry(
        cfg,
        &mcp_path(cfg, kind),
        "mcpServers",
        NAME,
        &mcp_entry(&cmd),
        |_| format!("mcpServers.rtok: {}", super::mcp_summary(&cmd, HOST)),
    )
}

/// Drop `mcpServers.rtok` unless the user edited it (T246).
pub fn unregister_mcp(cfg: &Config, kind: Kind) -> Result<String> {
    jsonc::remove_entry(
        cfg,
        &mcp_path(cfg, kind),
        "mcpServers",
        NAME,
        &mcp_entry("rtok"),
    )
}

/// True while `codebuddy plugin` records rtok as installed. `installed_plugins.json` has no
/// documented schema (the CLI is Claude Code's fork, whose file keys `plugins` by
/// `<plugin>@<marketplace>`), so this looks for that quoted id anywhere in the file instead of
/// walking a shape that may differ; reading it spawns nothing (T75).
pub fn plugin_installed(cfg: &Config, kind: Kind) -> bool {
    super::read(&dir(cfg, kind).join("plugins/installed_plugins.json"))
        .contains(&format!("\"{PLUGIN_ID}\""))
}

/// Offer, install, or uninstall `plugins/codebuddy` through the `codebuddy` CLI, behind
/// `--yes` (`Support::Flag`): rtok never writes `plugins/`, that store is CodeBuddy's. Install
/// is two calls (`marketplace add`, then `install`) that [`super::offer_plugin`]'s single-verb
/// skeleton cannot express. `CODEBUDDY_CONFIG_DIR` is set for WorkBuddy and for a relocated CLI
/// dir. A missing CLI or a failing call keeps the offer open: the own-file hooks still go in.
fn plugin(cfg: &Config, kind: Kind, remove: bool) -> Result<String> {
    let a = apply(cfg);
    let installed = plugin_installed(cfg, kind);
    if installed != remove || (!remove && !a.yes) {
        return Ok(NO_CHANGES.into());
    }
    let src = super::plugin_src(PLUGIN_SRC);
    let src = src.to_string_lossy();
    let steps: Vec<Vec<&str>> = if remove {
        vec![vec!["plugin", "uninstall", PLUGIN_ID, "-s", "user", "-y"]]
    } else {
        vec![
            vec!["plugin", "marketplace", "add", &src, "--name", NAME],
            vec!["plugin", "install", PLUGIN_ID, "-s", "user"],
        ]
    };
    let d = dir(cfg, kind);
    let env = (kind == Kind::Desktop || d != super::home_dir().join(".codebuddy"))
        .then_some(("CODEBUDDY_CONFIG_DIR", d));
    let prefix = env.map_or(String::new(), |(k, v)| format!("{k}={} ", v.display()));
    let shown = steps
        .iter()
        .map(|s| format!("{prefix}codebuddy {}", s.join(" ")))
        .collect::<Vec<_>>()
        .join(" && ");
    if a.dry_run {
        return Ok(if remove {
            format!("- plugin {NAME} ({shown})")
        } else {
            format!(
                "offer {PLUGIN_SRC} → {shown} {}",
                rtok_agent_sdk::KETCH_INSTALL
            )
        });
    }
    let failed = |e: String| format!("offer {PLUGIN_SRC} → {shown} (codebuddy failed: {e})");
    if super::find_on_path("codebuddy").is_none() {
        return Ok(failed(format!(
            "not on PATH; hooks and MCP go into {} instead",
            d.display()
        )));
    }
    let results: Vec<_> = steps
        .iter()
        .map(|s| super::run_cli("codebuddy", s, env))
        .collect();
    // An earlier step may only say "already added"; the last one decides, but the first error
    // is the one worth showing.
    if let Some(Err(_)) = results.last() {
        let first = results
            .into_iter()
            .find_map(Result::err)
            .unwrap_or_default();
        return Ok(failed(first));
    }
    Ok(if remove {
        format!("- plugin {NAME}")
    } else {
        format!("+ plugin {PLUGIN_SRC} → {NAME}")
    })
}

/// The [`super::d21_plugin_apply`] fn items for one variant: `D` picks the variant, since that
/// helper's fn pointers carry no `Kind`.
fn ops<const D: bool>() -> super::D21Plugin {
    const fn kind(desktop: bool) -> Kind {
        if desktop { Kind::Desktop } else { Kind::Cli }
    }
    super::D21Plugin {
        offer: |cfg, remove| plugin(cfg, kind(D), remove),
        plugin_installed: |cfg| plugin_installed(cfg, kind(D)),
        run: |cfg, remove| run(cfg, kind(D), remove),
        register_mcp: |cfg| register_mcp(cfg, kind(D)),
        unregister_mcp: |cfg| unregister_mcp(cfg, kind(D)),
    }
}

impl Agent for Codebuddy {
    fn id(&self) -> &'static str {
        HOST
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
            "plugin" => Support::Flag("--yes"),
            "proxy" => Support::No(
                "CodeBuddy Code takes its endpoint from the CODEBUDDY_BASE_URL environment variable; there is no config file to point at the proxy",
            ),
            _ => Support::No("unknown module"),
        }
    }

    fn plugin_surfaces(&self) -> &'static [rtok_plugin_sdk::Surface] {
        &[
            rtok_plugin_sdk::Surface::Hook,
            rtok_plugin_sdk::Surface::Mcp,
        ]
    }

    fn files(&self, cfg: &Config, kind: Kind) -> Vec<PathBuf> {
        vec![settings_path(cfg, kind), mcp_path(cfg, kind)]
    }

    fn installed(&self, cfg: &Config, kind: Kind) -> Vec<&'static str> {
        let mut out = Vec::new();
        if super::read(&settings_path(cfg, kind)).contains(" hook PreToolUse") {
            out.push("hooks");
        }
        if super::mcp::has_entry(&mcp_path(cfg, kind), "mcpServers", NAME) {
            out.push("mcp");
        }
        if plugin_installed(cfg, kind) {
            out.push("plugin");
        }
        out
    }

    fn apply(&self, cfg: &Config, kind: Kind, mode: Mode) -> Result<Vec<String>> {
        // D21: while the plugin is installed — on the very run that installs it too — its own
        // hooks serve, so the settings.json ones go instead of coming. MCP is the T275/D33
        // exception: `mcpServers.rtok` is written on every install and update.
        let ops = match kind {
            Kind::Cli => ops::<false>(),
            Kind::Desktop => ops::<true>(),
        };
        super::d21_plugin_apply(cfg, mode, ops, super::no_extra)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn cfg(name: &str, dry: bool) -> (Config, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("rtok-codebuddy-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut c = crate::testutil::config_in(&root);
        c.setup.codebuddy.dir = root.join("cli");
        c.setup.codebuddy.workbuddy_dir = root.join("wb");
        c.setup.dry_run = dry;
        c.setup.backup = false;
        (c, root)
    }

    /// CodeBuddy's record of an installed rtok plugin (schema undocumented: the id is enough).
    fn fake_plugin(c: &Config, kind: Kind) {
        let f = dir(c, kind).join("plugins/installed_plugins.json");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(f, r#"{"plugins":{"rtok@rtok":[{"version":"0.0.1"}]}}"#).unwrap();
    }

    #[test]
    fn variants_name_the_cli_and_the_workbuddy_app() {
        assert_eq!(VARIANTS[0].bins, ["codebuddy"]);
        assert!(VARIANTS[1].bins.is_empty());
        assert!(VARIANTS[1].apps.iter().all(|a| a.contains("WorkBuddy")));
        assert_eq!(VARIANTS[1].apps[0], "/Applications/WorkBuddy.app");
        let (c, _) = cfg("dirs", false);
        assert!(dir(&c, Kind::Cli).ends_with("cli") && dir(&c, Kind::Desktop).ends_with("wb"));
    }

    #[test]
    fn the_mcp_file_is_the_first_existing_one_else_dot_mcp_json() {
        let (c, root) = cfg("mcpfile", false);
        let d = dir(&c, Kind::Cli);
        fs::create_dir_all(d).unwrap();
        assert_eq!(mcp_path(&c, Kind::Cli), d.join(".mcp.json"), "none: create");
        fs::write(d.join("mcp.json"), "{}").unwrap();
        assert_eq!(mcp_path(&c, Kind::Cli), d.join("mcp.json"), "existing wins");
        fs::write(d.join(".mcp.json"), "{}").unwrap();
        assert_eq!(
            mcp_path(&c, Kind::Cli),
            d.join(".mcp.json"),
            "first in order"
        );
        assert_eq!(
            mcp_path(&c, Kind::Desktop),
            dir(&c, Kind::Desktop).join(".mcp.json")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn foreign_content_stays_byte_identical_in_the_mcp_file() {
        let (c, root) = cfg("mcpbytes", false);
        let path = dir(&c, Kind::Cli).join("mcp.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let before =
            "{\n  // mine\n  \"mcpServers\": {\n    \"other\": {\"command\": \"x\"},\n  },\n}\n";
        fs::write(&path, before).unwrap();
        assert!(
            register_mcp(&c, Kind::Cli)
                .unwrap()
                .starts_with("mcpServers.rtok: ")
        );
        assert_eq!(register_mcp(&c, Kind::Cli).unwrap(), NO_CHANGES);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("// mine") && text.contains("\"other\": {\"command\": \"x\"}"));
        let doc = jsonc::parse(&text).unwrap();
        assert_eq!(doc["mcpServers"]["rtok"]["type"], "stdio");
        assert_eq!(
            doc["mcpServers"]["rtok"]["args"],
            json!(["mcp", "--host", "codebuddy"])
        );
        assert_eq!(Codebuddy.installed(&c, Kind::Cli), ["mcp"]);
        assert_eq!(unregister_mcp(&c, Kind::Cli).unwrap(), "- mcpServers.rtok");
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hooks_use_the_table_rows_and_leave_foreign_hooks_alone() {
        let (c, root) = cfg("hooks", false);
        let path = settings_path(&c, Kind::Cli);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"model":"x","hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo stop"}]}]}}"#,
        )
        .unwrap();
        assert!(run(&c, Kind::Cli, false).unwrap().contains("9 additions"));
        assert_eq!(run(&c, Kind::Cli, false).unwrap(), NO_CHANGES);
        let doc: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let rows = hook_events::entries(HOST);
        let pre = doc["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(
            pre.len(),
            rows.iter().filter(|r| r.0 == "PreToolUse").count()
        );
        assert!(doc["hooks"].get("SubagentStart").is_none());
        let hook = &pre[0]["hooks"][0];
        assert_eq!(hook["timeout"], 5);
        let cmd = hook["command"].as_str().unwrap();
        assert!(cmd.contains("hook PreToolUse --host codebuddy"), "{cmd}");
        assert_eq!(doc["model"], "x");
        assert_eq!(Codebuddy.installed(&c, Kind::Cli), ["hooks"]);

        assert!(run(&c, Kind::Cli, true).unwrap().contains("removed"));
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("echo stop") && !text.contains("rtok"),
            "{text}"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// D21: the recorded plugin is the hooks unit, so an earlier plain install's hooks are taken
    /// back and never added again; MCP stays (T275/D33). Each variant reads its own directory.
    #[test]
    fn the_plugin_is_the_hooks_singleton_but_mcp_stays() {
        for kind in [Kind::Cli, Kind::Desktop] {
            let (c, root) = cfg(&format!("single-{}", kind.as_str()), false);
            let first = Codebuddy.apply(&c, kind, Mode::Install).unwrap();
            assert!(first.iter().any(|l| l.contains("additions")), "{first:?}");
            assert_eq!(Codebuddy.installed(&c, kind), ["hooks", "mcp"]);
            let other = if kind == Kind::Cli {
                Kind::Desktop
            } else {
                Kind::Cli
            };
            assert!(Codebuddy.installed(&c, other).is_empty(), "separate dirs");

            fake_plugin(&c, kind);
            let lines = Codebuddy.apply(&c, kind, Mode::Install).unwrap();
            assert!(lines.iter().any(|l| l.contains("9 removed")), "{lines:?}");
            assert_eq!(Codebuddy.installed(&c, kind), ["mcp", "plugin"]);
            assert!(!super::super::read(&settings_path(&c, kind)).contains("rtok"));
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn mcp_entry_lifecycle_and_legacy_upgrade() {
        let (c, root) = cfg("lifecycle", false);
        fake_plugin(&c, Kind::Cli);
        let path = mcp_path(&c, Kind::Cli);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        super::super::mcp::assert_json_entry_lifecycle(
            &Codebuddy,
            &c,
            Kind::Cli,
            &path,
            "mcpServers",
            || register_mcp(&c, Kind::Cli),
        );
        super::super::mcp::assert_legacy_entry_upgraded(
            &path,
            r#"{"mcpServers":{"rtok":{"type":"stdio","command":"rtok","args":["mcp"]},"keep":{"command":"k"}}}"#,
            HOST,
            &["keep"],
            || register_mcp(&c, Kind::Cli),
            || unregister_mcp(&c, Kind::Cli),
        );
        let _ = fs::remove_dir_all(root);
    }

    /// `--yes` names both calls with the resolved tree; WorkBuddy adds `CODEBUDDY_CONFIG_DIR`;
    /// dry-run runs nothing, and a recorded plugin stills the offer.
    #[test]
    fn the_offer_names_both_cli_calls_and_the_config_dir_for_workbuddy() {
        let (mut c, root) = cfg("offer", true);
        assert_eq!(
            plugin(&c, Kind::Cli, false).unwrap(),
            NO_CHANGES,
            "no --yes, no line"
        );
        c.setup.yes = true;
        let cli = plugin(&c, Kind::Cli, false).unwrap();
        assert!(cli.contains("plugins/codebuddy"), "{cli}");
        assert!(cli.contains("codebuddy plugin marketplace add"), "{cli}");
        assert!(cli.contains("--name rtok && "), "{cli}");
        assert!(
            cli.contains("codebuddy plugin install rtok@rtok -s user"),
            "{cli}"
        );
        assert!(
            cli.contains("CODEBUDDY_CONFIG_DIR="),
            "relocated dir is passed: {cli}"
        );
        let wb = plugin(&c, Kind::Desktop, false).unwrap();
        assert_eq!(wb.matches("CODEBUDDY_CONFIG_DIR=").count(), 2, "{wb}");
        assert!(
            wb.contains(&dir(&c, Kind::Desktop).display().to_string()),
            "{wb}"
        );
        fake_plugin(&c, Kind::Cli);
        assert_eq!(plugin(&c, Kind::Cli, false).unwrap(), NO_CHANGES);
        let gone = plugin(&c, Kind::Cli, true).unwrap();
        assert!(
            gone.starts_with("- plugin rtok (") && gone.contains("uninstall rtok@rtok -s user -y"),
            "{gone}"
        );
        assert_eq!(
            plugin(&c, Kind::Desktop, true).unwrap(),
            NO_CHANGES,
            "nothing recorded"
        );
        let _ = fs::remove_dir_all(root);
    }
}
