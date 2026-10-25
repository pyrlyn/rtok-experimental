---
status: todo
priority: P2
complexity: 4
readiness: 0%
agent:
---

# T985. Fold `tests/*.rs` into one integration test binary

From `ideas.md` I-139 (T261 item 4). Measured on a 16-core Mac after `touch src/lib.rs`: the 129 test units took 360 CPU-s (median 2.98 s, smallest 1.18 s) and 25 s of the 46 s `cargo test --no-run`; on a 3-core hosted macOS runner that share alone is about 120 s. One binary (`tests/it/main.rs` with one `mod` per file, `autotests = false` plus one `[[test]]`) relinks once instead of 129 times. nextest still runs every test in its own process, so per-process globals stay isolated.

Unmeasured: the folded binary's own compile and link time, which decides the real saving (somewhere between about 150 CPU-s and the full 360). Step 1 measures it on a branch before the rest lands; if the saving is small, close the task with the numbers. `tools/test-changed.sh` picks `--test <name>` targets and needs a filter by module instead.

Done means: one integration binary, the same set of tests passing, and a measured cut in `cargo test --no-run` time.

Check: `cargo nextest list` shows the same test names (under module paths) before and after; `cargo test --no-run` wall and CPU time after `touch src/lib.rs`, before and after, recorded in `research.md`; `just check` passes.
