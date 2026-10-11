// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Where each host keeps an installed rtok plugin on disk (T279 step 2, T279.1). Feeds
//! [`super::plugin_version::read_installed`] — no network, no host CLI.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::plugin_version::{Receipt, Source};
use super::{Kind, read};
use crate::config::Config;

/// Inputs to [`super::plugin_version::read_installed`] besides the receipt row.
pub(crate) struct InstallProbe {
    pub version_file: PathBuf,
    pub host_record_version: Option<String>,
    pub fallback_source: Source,
}

/// Resolve the installed copy for `host_id` when the plugin module is present. `None` when
/// the host has no rtok plugin tree to inspect (should not happen if `plugin` is installed).
pub(crate) fn install_probe(
    host_id: &str,
    cfg: &Config,
    kind: Kind,
    receipt: &Receipt,
) -> Option<InstallProbe> {
    if let Some(entry) = receipt.get(host_id) {
        return Some(InstallProbe {
            version_file: entry.path.join(".rtok-plugin-version"),
            host_record_version: None,
            fallback_source: entry.source,
        });
    }
    match host_id {
        "claude" if kind == Kind::Cli => claude_probe(cfg),
        "codex" => Some(cache_probe(codex_home(cfg), Source::Github)),
        "codebuddy" => Some(cache_probe(
            super::codebuddy::dir(cfg, kind).to_path_buf(),
            Source::Local,
        )),
        "cursor" => linked_probe(super::cursor::PLUGIN.path(cfg), Source::Local),
        "opencode" => linked_probe(
            super::opencode::plugin_dest(&super::opencode::for_kind(cfg, kind)),
            Source::Local,
        ),
        "kilo" => linked_probe(super::kilo::PLUGIN.path(cfg), Source::Local),
        "mimo" => linked_probe(super::mimo::PLUGIN.path(cfg), Source::Local),
        "pi" => linked_probe(super::pi::PLUGIN.path(cfg), Source::Local),
        "omp" => linked_probe(super::omp::PLUGIN.path(cfg), Source::Local),
        "zcode" => linked_probe(super::zcode::PLUGIN.path(cfg), Source::Local),
        "gemini" => tree_probe(
            cfg.setup.gemini.dir.join("extensions/rtok"),
            "gemini-extension.json",
            "rtok",
            Source::Marketplace,
        ),
        "copilot" => scan_probe(
            cfg.setup.copilot.dir.join("installed-plugins"),
            "plugin.json",
            "rtok",
            Source::Marketplace,
        ),
        "grok" => linked_probe(super::grok::plugin_marker(cfg), Source::Marketplace),
        "vscode" => vscode_probe(cfg),
        "kimi" => kimi_probe(cfg),
        "antigravity" => antigravity_probe(cfg, kind),
        _ => None,
    }
}

fn kimi_probe(cfg: &Config) -> Option<InstallProbe> {
    let marker = super::kimi::plugin_marker(cfg);
    let dir = marker.parent()?.to_path_buf();
    Some(InstallProbe {
        version_file: dir.join(".rtok-plugin-version"),
        host_record_version: manifest_version(&dir),
        fallback_source: Source::Marketplace,
    })
}

fn linked_probe(root: PathBuf, fallback: Source) -> Option<InstallProbe> {
    Some(InstallProbe {
        version_file: root.join(".rtok-plugin-version"),
        host_record_version: manifest_version(&root),
        fallback_source: fallback,
    })
}

fn tree_probe(root: PathBuf, manifest: &str, name: &str, fallback: Source) -> Option<InstallProbe> {
    let dir = if super::manifest_names(&root, manifest, name) {
        root
    } else {
        find_manifest_parent(&root, manifest, name)?
    };
    Some(InstallProbe {
        version_file: dir.join(".rtok-plugin-version"),
        host_record_version: manifest_version(&dir),
        fallback_source: fallback,
    })
}

fn scan_probe(root: PathBuf, manifest: &str, name: &str, fallback: Source) -> Option<InstallProbe> {
    let dir = find_manifest_parent(&root, manifest, name)?;
    Some(InstallProbe {
        version_file: dir.join(".rtok-plugin-version"),
        host_record_version: manifest_version(&dir),
        fallback_source: fallback,
    })
}

fn find_manifest_parent(root: &Path, manifest: &str, name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    for outer in entries.flatten() {
        let p = outer.path();
        if p.is_dir() {
            if super::manifest_names(&p, manifest, name) {
                return Some(p);
            }
            if let Some(inner) = std::fs::read_dir(&p).ok().and_then(|it| {
                it.flatten()
                    .find(|e| super::manifest_names(&e.path(), manifest, name))
                    .map(|e| e.path())
            }) {
                return Some(inner);
            }
        }
    }
    None
}

fn manifest_version(dir: &Path) -> Option<String> {
    for manifest in [
        "plugin.json",
        ".cursor-plugin/plugin.json",
        ".codex-plugin/plugin.json",
        ".codebuddy-plugin/plugin.json",
        "gemini-extension.json",
        "kimi.plugin.json",
        "devin.plugin.json",
        "package.json",
    ] {
        let path = dir.join(manifest);
        let text = read(&path);
        if text.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(&text)
            && let Some(ver) = v.get("version").and_then(Value::as_str)
        {
            return Some(ver.to_string());
        }
    }
    None
}

/// Where Claude Code and Codex unpack the rtok plugin, one directory per version.
pub(super) const PLUGIN_CACHE: &str = "plugins/cache/rtok/rtok";

fn claude_probe(cfg: &Config) -> Option<InstallProbe> {
    let dir = super::claude::config_dir(cfg);
    let text = read(&dir.join("plugins/installed_plugins.json"));
    let root: Value = serde_json::from_str(&text).ok()?;
    let entry = root
        .get("plugins")
        .and_then(|p| p.get(super::claude::PLUGIN_ID))
        .and_then(Value::as_array)
        .and_then(|a| a.first())?;
    let version = entry
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string);
    let install = entry
        .get("installPath")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| version.as_deref().map(|v| dir.join(PLUGIN_CACHE).join(v)))?;
    Some(InstallProbe {
        version_file: install.join(".rtok-plugin-version"),
        host_record_version: version,
        fallback_source: Source::Github,
    })
}

/// The `plugins/cache/rtok/rtok/<version>` copy a host unpacks into its config dir (Codex,
/// CodeBuddy, whose marketplace is a local directory); the newest version directory wins.
fn cache_probe(home: PathBuf, fallback_source: Source) -> InstallProbe {
    let base = home.join(PLUGIN_CACHE);
    let root = newest_child_dir(&base).unwrap_or(base);
    InstallProbe {
        version_file: root.join(".rtok-plugin-version"),
        host_record_version: manifest_version(&root),
        fallback_source,
    }
}

fn vscode_probe(cfg: &Config) -> Option<InstallProbe> {
    for (plugin, _) in [
        (&super::vscode::PLUGIN_STABLE, false),
        (&super::vscode::PLUGIN_INSIDERS, true),
    ] {
        if plugin.ours(cfg) {
            return linked_probe(plugin.path(cfg), Source::Marketplace);
        }
    }
    None
}

fn antigravity_probe(cfg: &Config, kind: Kind) -> Option<InstallProbe> {
    let root = match kind {
        Kind::Cli => super::antigravity::cli_plugin_dest(cfg),
        Kind::Desktop => super::antigravity::PLUGIN.path(cfg),
    };
    linked_probe(root, Source::Marketplace)
}

fn codex_home(cfg: &Config) -> PathBuf {
    let p = &cfg.setup.codex.config_path;
    p.parent().map(PathBuf::from).unwrap_or_else(|| p.clone())
}

fn newest_child_dir(base: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(base)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs.pop()
}
