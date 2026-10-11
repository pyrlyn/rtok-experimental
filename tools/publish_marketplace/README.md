# publish_marketplace

T183: publish rtok host plugins (`plugins/<host>/`) to their marketplaces via CI.

```
python -m publish_marketplace all|<host> [--dry-run]
```

Run from `tools/`. `--dry-run` prints the plan and fires nothing. A real run shells out to
`gh workflow run marketplace.yml -f host=<host>` for each supported host and prints the
triggered run's URL — it never publishes directly from this machine.

## Hosts

A host is **supported** only when its marketplace is a git-hosted catalog file already
committed in this repo — the git push that merges a PR *is* the publish, so
`marketplace.yml` only re-verifies the checked-out catalog (no network call). Checked
against each host's current docs 2026-09-23; re-verify before flipping a host to supported.

| Host | Marketplace? | Mechanism | Doc |
| --- | --- | --- | --- |
| claude | yes | git-hosted catalog (`.claude-plugin/marketplace.json`) | [docs.claude.com](https://docs.claude.com/en/docs/claude-code/plugin-marketplaces) |
| codex | yes | git-hosted catalog (`.agents/plugins/marketplace.json`) | [developers.openai.com](https://developers.openai.com/codex/plugins) |
| antigravity | no | bundled/local install only, no marketplace | [antigravity.google](https://antigravity.google/docs/plugins/) |
| cline | no | no marketplace: `cline plugin install <source>` installs straight from a git repo or local path (SDK/CLI/Kanban only, not the VS Code extension), no catalog file to publish | [docs.cline.bot](https://docs.cline.bot/customization/plugins.md) |
| codebuddy | no | git-hosted catalog possible, not wired in this repo yet | [codebuddy.ai](https://www.codebuddy.ai/docs/cli/plugin-marketplaces) |
| copilot | no | git-hosted catalog possible, not wired in this repo yet | [docs.github.com](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-plugin-reference) |
| cursor | no | manual review via cursor.com/marketplace/publish | [cursor.com](https://cursor.com/docs/plugins) |
| devin | no | no public catalog to publish to: `devin plugins install <owner/repo>` installs straight from a git repo, and the Devin marketplace is an org/enterprise manifest in the web app | [docs.devin.ai](https://docs.devin.ai/cli/extensibility/plugins/overview) |
| gemini | no | no marketplace: `gemini extensions install <source>` links a git repo or local path directly, no catalog file to publish | [geminicli.com](https://geminicli.com/docs/extensions/reference/) |
| grok | no | catalog is the externally-owned `xai-org/plugin-marketplace` repo | [docs.x.ai](https://docs.x.ai/build/features/skills-plugins-marketplaces) |
| kimi | no | git-hosted catalog possible, not wired in this repo yet | [kimi.com](https://www.kimi.com/code/docs/en/kimi-code-cli/customization/plugins.html) |
| opencode | no | no marketplace: npm + community ecosystem page | [opencode.ai](https://opencode.ai/docs/plugins/) |
| pi | no | no marketplace: npm registry + auto-populated gallery | [pi.dev](https://pi.dev/docs/latest/packages) |
| qwen | no | no marketplace: `qwen extensions install <source>` installs straight from a git URL or local path, no catalog file to publish | [qwenlm.github.io](https://qwenlm.github.io/qwen-code-docs/en/developers/extensions/extension/) |
| zcode | no | git-hosted catalog possible, not wired in this repo yet | [zcode.z.ai](https://zcode.z.ai/en/docs/plugin) |

## Adding a host

1. Confirm the host's current docs describe a git-hosted catalog this repo can serve — cite
   the URL and the date checked. Manual/API-only submission does not qualify; leave it in
   `registry.UNSUPPORTED` with the reason.
2. Add a `GitCatalogHost(...)` entry to `registry.SUPPORTED` pointing at the catalog file and
   `plugins/<host>`.
3. Add a row to the table above and remove the host's `UNSUPPORTED` entry.
4. `mise exec -- pytest tools/tests` covers the new host automatically via
   `test_registry_covers_every_plugins_host`; add a targeted `verify_local` test only if the
   new catalog's schema differs from claude/codex's.

## Safety

This tool only ever triggers `marketplace.yml`, a read-only verification job — it never pushes,
never calls a marketplace API, and never submits a form. The actual "publish" for a supported
host is the ordinary PR merge that updates its catalog file.
