---
status: in progress
priority: P2
complexity: 3
readiness: 100%
agent: Claude Code / claude-opus-5-5
---

# T976. `graph review`: support code, reachable levels, docs

`rtok graph review` (and the "rtok review" PR comment) is noisy. Fixture crates and bench
scripts are listed `untested` and ranked with production code. Every level is `low` because the
formula tops out at 0.6 while `high` starts above 0.7, and an untested function with callers
(0.4) is `low`. Files with no functions (README, TOML) take line budget next to code. There is
also no page that says why the command exists and how it scores.

Done when (creator's choices, 2026-10-24):

- Support code is not scored: built-in directories (`fixtures/`, `fixture/`, `testdata/`,
  `bench/`, `benches/`, `examples/`) plus `.gitattributes` `linguist-vendored`,
  `linguist-generated` and `linguist-documentation` read with `git check-attr`. An explicit
  `false`/unset attribute overrides a built-in directory. Support files stay in the list,
  marked, with no `untested`.
- Levels fit the reachable range: `high` above 0.45 (untested and security-named), `medium`
  above 0.25, else `low`. The score formula is unchanged.
- Files scoring 0 (no scored definitions, or support code) get lines only after every scored
  file got all its regions.
- Only definitions a hunk touches are scored and listed `untested` (found while checking:
  the first 8 definitions of a changed file were scored, touched or not).
- `docs/review.md` explains why and how, including CI; linked from `docs/commands.md`. The
  `docs/ru` and `docs/uk` translations are left out for now (creator, 2026-10-24).

Plan:

1. `src/plugins/graph/mod.rs`: `is_support_path` next to `is_test_path`.
2. `src/plugins/graph/review.rs`: `git check-attr` for changed paths, `support` flag on
   `FileReview`, skip scoring, marker in text and JSON, new `level` thresholds, two-pass
   `allocate_budget`; unit tests for each.
3. Docs: `docs/review.md` + translations, link from `docs/commands.md` (+ translations).
4. `just check`; `rtok graph review --since HEAD~1` on the bench commit shows bench files as
   support.

Check: `cargo nextest run -E 'test(/review/)'`; `just check`.
