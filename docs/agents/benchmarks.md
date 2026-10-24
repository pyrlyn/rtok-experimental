# Benchmarks — instructions for agents

After **any** benchmark run (fixture test `tests/bench_fixtures.rs`, the Docker/Colima harness `bench/harness/run.py`, a competitor comparison), update every place the old numbers appear, in the same change. No measurement, no claim: a missing number is written `no data`, never estimated.

## 1. Run it

Always in Docker, never bare metal.

1. Colima (macOS): use the `rtok-bench` profile (4 CPU, 8 GiB). Set `DOCKER_HOST` to its socket (`~/.colima/rtok-bench/docker.sock`); if the credential helper fails, point `DOCKER_CONFIG` at a dir with an empty `credsStore`.
2. Harness: `python3 bench/harness/run.py rtk --rtok-ref origin/main --rtok-pr origin/<branch>`. Refs must resolve in your clone (`origin/...`, not a bare branch name). It takes 10+ minutes: run it as a background job and wait; a killed shell kills the run.
3. Results land in `bench/harness/results/<tool>/<stamp>/` (`results.json`, `summary.md`, `negatives.txt`, `versions.json`, `samples/`). They are tracked in the repo.
4. Fixture test: `cargo test --test bench_fixtures`. In a container, copy the source in with `tar --no-xattrs … | docker cp - <container>:/src` (macOS extended attributes make a plain `docker cp` fail; a bind mount of `/tmp` came up empty).
5. Refresh fixtures only from a real capture: `run.py rtk --refresh-fixtures <dir>`, then copy into `tests/fixtures/bench`. Never hand-edit them. After a refresh, re-check each `min_savings_pct` in `case.toml`.
6. Competitor versions: pinned in the competitor Dockerfiles under `bench/harness/tools/`.
7. Disk: a Colima VM and Rust `target` dirs fill the disk fast. Build inside the container, and remove containers and temporary dirs you created when done.

## 2. Where the numbers live

Find them, do not guess: `rg -n "<old number>|bench/harness/results" README.md docs/`.

- `docs/comparison/rtok-vs-<tool>.md` — one page per competitor.
- `docs/comparison/README.md` — index with a status per competitor.
- `docs/comparison.md` — summary tables.
- `docs/research/competitors-techniques.md` — gaps and targets (targets are not measurements).
- `plan.md` / `todo.md` — task rows that quote targets.
- `README.md` — headline figures, if it quotes any (it quoted none at the time of writing).
- `bench/harness/README.md` — reproduce command and fairness rules, if flags changed.
- `docs/ru/`, `docs/uk/` — mirrors; keep in sync (see `CONTRIBUTING.md`).

## 3. Page structure (rendered as website blocks)

Each comparison page keeps these `##` sections, in this order:

1. **What changed** — run date, rtok SHAs, deltas vs the previous run.
2. **Performance figures** — totals per command: raw, rtok main, rtok PR, rtok PR `--no-ref`, competitor.
3. **Command table** — one `###` table per command, one row per case.
4. **Where rtok wins / loses** — bullets with numbers; every `WARN NEGATIVE` line listed; flag `nondet!` cases.
5. **Methodology** — fixture SHA, tool versions, tokenizer, runs, results directory.
6. **Competitor landscape** and **How to reproduce**.

## 4. Checklist

- [ ] Run done in Docker; results dir path recorded.
- [ ] `cargo test --test bench_fixtures` passes.
- [ ] `negatives.txt` read; each negative appears in "Where rtok wins / loses".
- [ ] Old numbers grepped and replaced in every file of section 2.
- [ ] SHAs and tool versions match `versions.json`.
- [ ] Latency not presented as filter speed (the replay shim dominates it).
- [ ] English only; ru/uk mirrors updated or listed as a follow-up.
- [ ] Temporary dirs and containers removed.

## 5. T991 developer test-runtime run (2026-10-24)

For the full-workspace nextest comparison, keep the 4,236 executed tests and 8 ignored tests fixed. Use Rust 1.98.1, 16 test threads, `CARGO_INCREMENTAL=0`, and `CARGO_PROFILE_DEV_DEBUG=0` for both runs. Record compilation/setup wall time separately from nextest execution time. See the [T991 validation evidence](../../plan/T991/validation.md).
