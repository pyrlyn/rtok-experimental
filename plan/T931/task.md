---
status: in progress
priority: P2
complexity: 3
readiness: 20%
agent: Claude Code / sonnet-5.5
---

# T931. Bounded `sed -n a,bp` windows inside command chains are still elided

5,306 Bash results carried `N lines omitted (expand <id>)`. 1,260 of them after 2026-09-28 were
`sed -n a,bp` windows of 120 lines or fewer inside chains (`sed ..; grep ..; sed ..`,
`git show | sed`), peaking 2026-10-10..12. Agents then ran another sed (437), native Read (190)
or `rtok expand` (160), and said so ("rtok's compression hides lines in the shell output",
2026-10-19). T176 fixed the single-command case only (`src/plugins/cmd/bounded.rs`).

Done when: a chain whose output is bounded by explicit line windows (each segment bounded, or
the total under the window limit) passes through unshortened.

Check: cmd tests for `sed -n 1,80p a; sed -n 1,40p b` and `git show x | sed -n 1,100p`
passing through; unbounded chains still shortened; `just check`.

## Execution plan

1. Tests first: `bounded.rs` rstest cases (chains of windows, `echo ---` labels) and a
   `formatters.rs` `compress` test on a 240-line body in the single-argv hook shape.
2. `bounded::is_bounded`: a single-stage literal `echo`/`printf` (no `$`/backtick) is silent
   like `cd`/`export`, so a label between windows no longer unbounds the chain. Unbounded
   and mixed chains (`sed ..; cargo test`) are untouched.
3. Verify: `cargo nextest run --lib` for the cmd tests, then `just check`.
