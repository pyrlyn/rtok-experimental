# rtok benchmarks

What lives in `bench/`, how to run each suite, what it measures, and how to read the output.
Everything that calls a model spends API money and runs only with `RTOK_BENCH_LIVE=1`; without
it a suite prints its schedule or a zero table and calls nothing.

| Suite | Entry point | Measures | Needs |
| --- | --- | --- | --- |
| T9.1 A/B | `rtok bench` + `tasks.toml` | tokens, cost and pass rate of two host configs on six edits | `claude`, `RTOK_BENCH_LIVE=1` |
| Graph (T68.9) | `rtok bench --suite graph` + `graph.toml` | architecture answers with vs without the rtok graph MCP | `claude`, `RTOK_BENCH_LIVE=1` |
| P28 (T385.9.3) | `rtok bench --suite p28` + `p28.toml` | LLM compressor vs lossless archive pointer | two proxies, `RTOK_BENCH_LIVE=1`; see [`p28/README.md`](p28/README.md) |
| Competitor harness | `python3 bench/harness/run.py <tool>` | output tokens and filter latency, rtok vs one competitor | Docker; see [`harness/README.md`](harness/README.md) |
| C14 / B2 / B3 | `bench/c14/*.py` in the `rtok-c14` images | rtok vs toon, ctags, aider repo map, sqz, lean-ctx, ... | Docker, `tiktoken` |

## Layout

| Path | Contents |
| --- | --- |
| `tasks.toml` | T9.1 tasks: prompt + `check` shell command per task |
| `repos/t9/` | Fixture crate the T9.1 tasks edit (seeded off-by-one bug) |
| `check_tasks.py` | Proves each T9.1 check fails on the untouched fixture and passes on a reference edit |
| `graph.toml`, `repos/mini-rs`, `repos/mini-py` | Graph suite questions and its small repos |
| `p28.toml`, `p28/` | P28 arms, proxy configs, logs and tasks |
| `configs/` | Claude settings per arm (`legacy.json`, `rtok.json`, `graph-on.json`, `graph-off.json`) |
| `results/` | Committed T9.1 and graph results (`a.json`, `b.json`, `graph-tags-vs-lsp.json`) |
| `harness/` | Shared Docker harness and its per-tool adapters |
| `competitors/` | One directory per competitor (install/run scripts) plus `_lib` |
| `c14/` | C14, B2 and B3 runners, report generators, `results/*.jsonl` |

## T9.1 A/B (`rtok bench`)

Two host configurations (`[bench.configs]` in config, default `a = configs/legacy.json`,
`b = configs/rtok.json`) run every task `--runs` times through `claude -p`. After each task ×
config the task's `check` runs from the repo root; it exits 0 only when the edit is really in
`repos/t9`, then restores the fixture so the next config starts from the same state.

```sh
python3 bench/check_tasks.py                               # checks are real (no model calls)
rtok bench --tasks bench/tasks.toml --dry-run              # schedule only
RTOK_BENCH_LIVE=1 rtok bench --tasks bench/tasks.toml --runs 3
```

Run it on a clean worktree: the checks use `git checkout` and `git clean` on `bench/repos/t9`.

Reading the table: `input`, `cache`, `output` are mean tokens per run and `cost` mean USD per
run, from the `claude` JSON usage; `pass` is checks passed out of six. Lower tokens at the same
`pass` is the win. Without `RTOK_BENCH_LIVE` every token column is 0 and `pass` is 0/6, because
the fixture is untouched. Results are written to `bench/results/<config>.json`.

## Graph suite

```sh
rtok bench --suite graph --tasks bench/graph.toml --dry-run
RTOK_BENCH_LIVE=1 rtok bench --suite graph --tasks bench/graph.toml --runs 3
```

Each question in `graph.toml` runs on both arms (`mcp`, `native`); an answer passes when it
matches the question's `expect` regex. Compare pass counts first, then tokens.

## P28

See [`p28/README.md`](p28/README.md): start both proxies, then run with `RTOK_BENCH_LIVE=1`.
The fixtures are checked offline by `cargo test bench::` (no spend).

## Competitor harness and C14

Docker only; see [`harness/README.md`](harness/README.md) for fairness rules and adding a
competitor. The image builds need several GB of free disk. Per-competitor write-ups are in
[`docs/comparison/`](../docs/comparison/README.md); reading a row: fewer tokens is better only
when the case's anchors are kept (the `note` field says how many each tool kept).

## Status (2026-10-23)

* `tasks.toml` had `check = "true"` for every task, so every committed table reported 6/6.
  The checks are now real; `check_tasks.py` gives 6/6 fail on the untouched fixture and 6/6
  pass on the reference edits. No live T9.1 numbers exist yet.
* Not run on this pass: every live suite (API spend), the Docker harness and C14 (disk).
