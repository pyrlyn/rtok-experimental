---
status: todo
priority: P3
complexity: 2
readiness: 0%
agent:
---

# T986. `rtok stats` parses session files in parallel

From `ideas.md` I-93. `rtok stats` parses every session `.jsonl` one after another (`src/measure/stats.rs` `collect` → `src/measure/codex.rs` `jsonl_paths`) and folds them into one report. The files are independent and no store writer sits in the loop, so map-then-reduce with `rayon` (`par_iter().map(parse_file).reduce(merge)`) fits. `rayon` is listed in `rust.md`; wiring it in needs a `toolchain.md` row and a reason in the commit. Not for the hook path, which cannot afford thread spawn.

Done means: `rtok stats` is faster on a large session directory and its output is byte-identical.

Check: wall time of `rtok stats` before and after on a real session directory of at least 1 000 files, recorded as a `Measurement` row and in `research.md`; `rtok stats --json` is byte-identical before and after.
