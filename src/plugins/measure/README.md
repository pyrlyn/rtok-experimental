# `measure`

Ground truth for everything else: nothing in rtok ships until this plugin can show a
before/after number (decision D3).

| | |
|---|---|
| Surfaces | `rtok stats`, `rtok bench`, proxy `usage` capture |
| Spec | the **Replaces** column of [`docs/plugins.md`](../../../docs/plugins.md) |
| Default | on |

## What it does

- Parses Claude Code transcripts (`~/.claude/projects/**/*.jsonl`): tool calls, tool
  results, `usage` blocks, turn index.
- Reads `usage` rows written by the proxy.
- Reports per-tool result sizes, Bash by command family, MCP server groups, cache hit
  rate, and **context-token-turns** (tokens × turns they stay in context).
- Saves and compares baselines (`rtok stats --save-baseline`, `--compare`).
- Optional calibration of the estimator against `count_tokens` (`--calibrate`).

## Config

None beyond `[estimator]` (chars per token per class), which `--calibrate` rewrites.

## Tasks

See `roadmap.md` § `measure`. Checks in `plan/`.

T1.1 JSONL parser · T1.2 `rtok stats` · T1.3 baseline · T1.5 calibration · T5.5 cache health · T9.1 `rtok bench` · T11.6 per-API usage.

## Status

Manifest only. First task: T1.1.
