# Measuring a reduction

How rtok measures savings, and the current state of the A/B bench.

## Measure before keeping a reduction

`rtok stats` reads transcript estimates and proxy usage. The proxy's provider-reported
usage is ground truth; transcript estimates use a fixed chars-per-token heuristic
(`[estimator]` in config) and are directional only — no calibration against a real
tokenizer has been measured yet, so treat them as a trend line, not a bound.

```bash
rtok stats --since 7d
rtok stats --save-baseline before-rtok
rtok stats --compare before-rtok
```

A fresh install has nothing to report yet, and says so rather than inventing a number:

```text
sessions 0  lines 0  malformed 0
usage input=0 cache_create=0 cache_read=0 output=0  hit=0.0%  median_context=0
```

What keeps that `hit=` high with rtok installed: [docs/prompt-cache.md](prompt-cache.md).

Batch, Flex, and model routing on the proxy (pass-through vs rewrite, what is planned): [docs/batch-flex.md](batch-flex.md).

## Measured results

The committed T9.2 A/B harness schedules six tasks × three runs. Its first run was offline
(`RTOK_BENCH_LIVE` was unset), so both configurations have zero provider usage and equal
task checks. This is a reproducible baseline, **not** evidence of a bill reduction; rerun it
with live traffic before adopting a configuration.

| config | mean input | mean cache | mean output | mean cost USD | passed |
|---|---:|---:|---:|---:|---:|
| A — legacy baseline | 0 | 0 | 0 | 0.0000 | 6/6 |
| B — rtok | 0 | 0 | 0 | 0.0000 | 6/6 |
| delta (B − A) | 0 | 0 | 0 | 0.0000 | 0 |

These rows predate real checks: `bench/tasks.toml` used `check = "true"`, so 6/6 meant
nothing, and the zero tokens mean no live run. The checks are real now; see
[`bench/README.md`](../bench/README.md).

Sources: [`bench/results/a.json`](../bench/results/a.json),
[`bench/results/b.json`](../bench/results/b.json), and [`research.md`](../research.md).
