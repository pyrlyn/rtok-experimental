---
title: rtok
tagline: Token-reduction CLI for AI coding agents — hooks, an MCP server and an API proxy in one Rust binary.
repo: https://github.com/pyrlyn/rtok
homepage: https://github.com/pyrlyn/rtok
install: "curl --proto '=https' --tlsv1.2 -LsSf https://github.com/pyrlyn/rtok/releases/latest/download/rtok-installer.sh | sh"
install_alternatives:
  - 'ketch install pyrlyn/rtok'
  - 'brew install pyrlyn/tap/rtok'
  - 'npm i -g rtok-cli'
  - 'uv tool install rtok-cli'
version: "0.16.0"
accent: "#5CE1FF"
accent2: "#FF6B4A"
accentLight: "#0B7FA0"
featured: true
order: 1
---

<!-- Website copy for the pyrlyn project site. The sync-docs workflow copies this file to
pyrlyn/landing (main) as content/projects/rtok.md on every change to main and on every v*
tag; front matter follows CONTENT_CONTRACT.md in that repository.
Sources (checked 2026-10-14): README.md and the clap CLI in src/cli.rs; version from the latest
GitHub release (v0.16.0); accent from site/assets/css/custom.css (--rtok-accent). -->

## Overview

`rtok` reduces the context that AI coding agents must carry. It is one Rust binary with three
surfaces: Claude Code hooks, an MCP server, and an API proxy. Each reduction is measured, and
shortened payloads stay retrievable by id.

Ten methods, one process, one ledger. A saving that is not a row in that ledger does not exist —
including rtok's own.

## Features

- **Hooks for coding agents.** `rtok agents install claude` writes rtok's hooks and MCP entry
  into Claude Code, backing up the settings file first; `--dry-run` prints the diff and touches
  nothing. Cursor, Codex, OpenCode, pi, Kimi, Copilot, Aider, Windsurf, Zed, VS Code and more
  are registered the same way.
- **Run commands without paying for their output.** `rtok run -- <cmd>` keeps the exit code,
  archives the raw output and prints a compact summary with an id to expand it.
- **Lossless by design.** Archived payloads come back with `rtok expand <id>`, whole or by line
  range or regex match. Hooks fail open: on any error the agent gets its input unmodified.
- **Code graph over MCP.** `rtok graph index .` builds a symbol index that agents query as
  `symbol`, `callers`, `impact`, `outline` and `explore` instead of a grep-and-read chain.
- **Local API proxy.** `rtok proxy` captures provider-reported usage and can replace old tool
  results with stable archive pointers while preserving the cached prefix.
- **Measure before you keep a reduction.** `rtok stats` reads transcripts and proxy usage;
  `rtok doctor` prices the hooks, MCP servers and skills you already run.
- **Pluggable methods.** `measure`, `cmd`, `read`, `archive`, `proxy`, `inject`, `guard`,
  `memory`, `graph` and `toon` — each can be switched off in the one config file.
- **Observability.** `rtok otel flush` exports calls, logs and metrics as OTLP/HTTP JSON to
  Jaeger, Grafana, SigNoz or Maple. `rtok web` and `rtok tui` show the same data locally.

## Install

macOS (Apple silicon) and Linux x86-64, installed into `~/.cargo/bin`:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/pyrlyn/rtok/releases/latest/download/rtok-installer.sh | sh
```

With [ketch](https://github.com/pyrlyn/ketch), straight from the GitHub Release archives:

```bash
ketch install pyrlyn/rtok
```

Homebrew, npm or PyPI (the packages carry the same native binary):

```bash
brew install pyrlyn/tap/rtok
npm i -g rtok-cli
uv tool install rtok-cli
```

From source:

```bash
git clone https://github.com/pyrlyn/rtok && cd rtok
mise install
mise exec -- cargo install --path .
rtok --version
```

## Usage examples

Install the Claude Code integration, preview first, then check it:

```bash
rtok agents install claude --dry-run
rtok agents install claude
rtok doctor
```

Run a command through rtok and read the archived output back later:

```bash
rtok run -- cargo test
rtok expand 7f3a91 --lines 120-180
rtok expand 7f3a91 --grep "panicked" --context 3
```

Index a tree for the code graph and list the plugins:

```bash
rtok graph index .
rtok plugins
rtok config set plugins.toon.enabled false
```

Run the proxy and measure:

```bash
rtok proxy --mode passthrough
rtok stats --since 7d
rtok stats --save-baseline before-rtok
rtok stats --compare before-rtok
```

See where every setting came from:

```bash
rtok config show --sources
```

## Links

- Repository: <https://github.com/pyrlyn/rtok>
- Documentation: <https://github.com/pyrlyn/rtok/tree/main/docs>
- Getting started: <https://github.com/pyrlyn/rtok/blob/main/docs/getting-started.md>
- Releases: <https://github.com/pyrlyn/rtok/releases>
- Changelog: <https://github.com/pyrlyn/rtok/blob/main/CHANGELOG.md>
- License: your choice of GNU GPLv3 or a commercial license (see <https://github.com/pyrlyn/rtok#license>)
