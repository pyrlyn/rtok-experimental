# measure plugin

![measure: transcripts, proxy usage rows and Measurement rows feed rtok stats, baselines, cache health and rtok bench](measure.svg)

Source: [`measure.d2`](measure.d2). Plugin docs: [README](../../../src/plugins/measure/README.md). Index: [all diagrams](../README.md).

`measure` is the ground truth the other plugins are judged against: nothing ships until it can
show a before/after number. The plugin struct itself only declares a manifest (surfaces `Cli`
and `Proxy`, default on) and a dashboard page; the work lives in `src/measure/`, `src/bench.rs`
and the `rtok stats` / `rtok bench` commands that call into them. It never sits on the hook path.

## Walkthrough

1. **Inputs.** Three sources feed it: agent transcripts parsed on the fly (Claude Code JSONL,
   Codex `token_count` events), `usage` rows the proxy wrote per request (`record_usage` in
   `src/proxy/mod.rs`, via `Store::insert_usage`), and `measurements` rows other plugins wrote
   through `Ctx::record`.
2. **Choose the view.** `src/cli/stats.rs` branches on the flag first: `--calibrate` prints
   `calibrate_or_skip`; `--cache` renders `cache::report` (turns whose cache creation jumped
   while cache reads fell, with a cause read from `call_io` bodies); `--plugin <id>` returns
   that plugin's `measurements` rows through `model::plugin_stats`; anything else builds the
   full report in `model::stats_report`.
3. **Parse.** `jsonl::parse_path` reads one JSON object per line. A malformed line is counted
   in `Parsed::malformed`, never fatal; an unreadable transcript counts as one malformed entry.
4. **Collect.** `stats::collect` walks transcripts in path order, skips sub-agent files
   (`subagents::is_subagent`, they are attributed to the parent), and folds sessions into a
   `Report`: per-tool result sizes, Bash command families (`bash_family`), MCP groups and
   context-token-turns.
5. **Attach store data.** `model::stats_report` opens the store (optional for the report) and
   calls `attach_api`, `attach_lanes`, `attach_bash_cmd`, `attach_checkpoint_notes`, and
   `attach_costs` when `--price` is set; `attach_codex` adds Codex requests.
6. **Baselines.** `--save-baseline <name>` writes the report JSON to
   `<home>/measurements/<name>.json` (`baseline::save`, which rejects names containing path
   separators); `--compare <name>` renders deltas through `baseline::compare`. Otherwise the
   report prints as a table, or JSON with `--json`.
7. **Bench.** `rtok bench` calls `bench::run`: the default suite runs six tasks across
   `[bench.configs]`, `--suite graph` compares with and without the rtok MCP server, and
   `--suite p28` runs the p28 suite. Without `RTOK_BENCH_LIVE` it returns zero rows instead of
   spawning an agent. Results land in `results/<config>.json` beside the task file.

## Where it lives

| Box | Code |
| --- | --- |
| measure plugin manifest | `src/plugins/measure/mod.rs` (`MeasurePlugin`) |
| `jsonl::parse_path` | `src/measure/jsonl.rs` (`Parsed`, `parse_jsonl`, `parse_dir`) |
| `stats::collect` | `src/measure/stats.rs` (`collect`, `Report`, `bash_family`) |
| attach steps | `src/measure/stats.rs` (`attach_api`, `attach_lanes`, `attach_bash_cmd`, `attach_codex`, `attach_costs`) |
| `cache::report` | `src/measure/cache.rs` (`analyse`, `report`, `SessionHealth`) |
| `baseline::save / compare` | `src/measure/baseline.rs` |
| `model::stats_report` | `src/model/mod.rs` (`stats_report`, `plugin_stats`, `cache_health`) |
| `rtok stats` | `src/cli/stats.rs` |
| `bench::run` | `src/bench.rs`, `src/bench/p28.rs`, `src/cli/bench.rs` |
| proxy usage capture | `src/proxy/mod.rs` (`record_usage`) |
| Codex reader | `src/measure/codex.rs` (`requests`, `collect`) |
| `Ctx::record` | `src/plugin.rs` (`Runtime::record`) |

## Invariants

- Only `Measurement` rows count as savings; transcript numbers are estimates and proxy `usage`
  rows are the only real provider counts.
- A malformed transcript line is skipped and counted; it never aborts a report.
- Transcript and Codex reads are never written to the store, so a re-read is idempotent.
- `--calibrate` is the only network-shaped call and prints `skipped` without an API key.
- A store that will not open costs `rtok stats` its API table and nothing else, except `--plugin`
  and `--cache`, which require the store.

## Related

[Overview](../overview.md) · [Plugin contract](../plugin-contract.md) · [proxy plugin](proxy.md) · [archive plugin](archive.md)
