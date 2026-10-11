# Copyright (c) 2026 Ivan Tugay
# SPDX-License-Identifier: GPL-3.0-or-later
# Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

"""Every host under `plugins/`: which ones can be published via CI, and why not for the rest.
The full host table and how to add one live in `README.md`; this module only carries the data.
Checked against each host's current docs 2026-09-23 — re-verify before flipping a host to
supported; do not guess.

A supported host's marketplace is a git-hosted catalog file already committed in this repo —
the git push that merges a PR *is* the publish. `verify_local` therefore just confirms the
catalog on the checked-out tree still points at the right plugin dir; no network call needed.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

from . import PublishError


@dataclass(frozen=True)
class GitCatalogHost:
    host: str
    marketplace_path: str  # repo-relative path to the catalog file
    plugin_path: str  # repo-relative path to plugins/<host>
    plugin_name: str = "rtok"

    def verify_local(self, repo_root: Path) -> dict:
        catalog = repo_root / self.marketplace_path
        if not catalog.is_file():
            raise PublishError(f"{self.host}: catalog file not found at {self.marketplace_path}")
        try:
            data = json.loads(catalog.read_text())
        except json.JSONDecodeError as exc:
            raise PublishError(f"{self.host}: {self.marketplace_path} is not valid JSON: {exc}") from exc
        entry = next((p for p in data.get("plugins", []) if p.get("name") == self.plugin_name), None)
        if entry is None:
            raise PublishError(f"{self.host}: {self.marketplace_path} has no plugin named {self.plugin_name!r}")
        # Schemas disagree on `source`'s shape: Claude Code uses a plain string, Codex nests it
        # under `source.path`.
        source = entry.get("source")
        path = source if isinstance(source, str) else (source or {}).get("path")
        expected = self.plugin_path if self.plugin_path.startswith("./") else f"./{self.plugin_path}"
        if path not in (self.plugin_path, expected):
            raise PublishError(f"{self.host}: catalog entry points at {path!r}, expected {expected!r}")
        if not (repo_root / self.plugin_path).is_dir():
            raise PublishError(f"{self.host}: plugin directory {self.plugin_path} does not exist")
        return entry


SUPPORTED: dict[str, GitCatalogHost] = {
    "claude": GitCatalogHost("claude", ".claude-plugin/marketplace.json", "plugins/claude"),
    "codex": GitCatalogHost("codex", ".agents/plugins/marketplace.json", "plugins/codex"),
}

# host -> (reason, doc_url, date checked). The full table is in README.md.
UNSUPPORTED: dict[str, tuple[str, str, str]] = {
    "antigravity": ("no marketplace: bundled/local install only", "https://antigravity.google/docs/plugins/", "2026-09-23"),
    "cline": ("no marketplace: `cline plugin install <source>` installs straight from a git repo or local path (SDK/CLI/Kanban only, not the VS Code extension), no catalog file to publish", "https://docs.cline.bot/customization/plugins.md", "2026-09-24"),
    "commandcode": ("no marketplace: Command Code documents no plugin-bundle format; hooks and MCP are written into settings.json and mcp.json (mods are the TypeScript extension surface, and rtok does not ship one)", "https://commandcode.ai/docs/mods", "2026-09-30"),
    "codebuddy": ("git-hosted catalog possible, not wired in this repo yet: `codebuddy plugin marketplace add` reads a `.codebuddy-plugin/marketplace.json` from a git repo or local dir", "https://www.codebuddy.ai/docs/cli/plugin-marketplaces", "2026-10-11"),
    "copilot": ("git-hosted catalog possible, not wired in this repo yet", "https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-plugin-reference", "2026-09-23"),
    "cursor": ("manual review via cursor.com/marketplace/publish", "https://cursor.com/docs/plugins", "2026-09-23"),
    "devin": ("no public catalog to publish to: `devin plugins install <owner/repo>` installs straight from a git repo, and the Devin marketplace is an org/enterprise manifest in the web app", "https://docs.devin.ai/cli/extensibility/plugins/overview", "2026-09-26"),
    "gemini": ("no marketplace: `gemini extensions install <source>` links a git repo or local path directly, no catalog file to publish", "https://geminicli.com/docs/extensions/reference/", "2026-09-24"),
    "grok": ("catalog is the externally-owned xai-org/plugin-marketplace repo", "https://docs.x.ai/build/features/skills-plugins-marketplaces", "2026-09-23"),
    "kimi": ("git-hosted catalog possible, not wired in this repo yet", "https://www.kimi.com/code/docs/en/kimi-code-cli/customization/plugins.html", "2026-09-23"),
    "opencode": ("no marketplace: npm + community ecosystem page", "https://opencode.ai/docs/plugins/", "2026-09-23"),
    "pi": ("no marketplace: npm registry + auto-populated gallery", "https://pi.dev/docs/latest/packages", "2026-09-23"),
    "qwen": ("no marketplace: `qwen extensions install <source>` installs straight from a git URL or local path, no catalog file to publish", "https://qwenlm.github.io/qwen-code-docs/en/developers/extensions/extension/", "2026-10-05"),
    "zcode": ("git-hosted catalog possible, not wired in this repo yet", "https://zcode.z.ai/en/docs/plugin", "2026-09-23"),
}


def all_hosts() -> list[str]:
    return sorted({*SUPPORTED, *UNSUPPORTED})


def supported_hosts() -> list[str]:
    return sorted(SUPPORTED)


def require_supported(host: str) -> GitCatalogHost:
    if host in SUPPORTED:
        return SUPPORTED[host]
    if host in UNSUPPORTED:
        reason, doc_url, checked = UNSUPPORTED[host]
        raise PublishError(
            f"host {host!r} has no marketplace publish support: {reason} (see {doc_url}, checked {checked})"
        )
    raise PublishError(f"unknown host {host!r}; known hosts: {', '.join(all_hosts())}")
