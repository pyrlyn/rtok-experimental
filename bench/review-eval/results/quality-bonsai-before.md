# Quality eval, baseline (before the usage-search noise filters)

- Reviewer: `prism-ml/bonsai-27b` (Ternary-Bonsai-27B, MLX 2-bit, ~8.5 GB), local LM Studio
  OpenAI-compatible server, context loaded at 32,768 tokens, temperature 0.
- `rtok review` at commit `a64769ac` (feat/review, before noise filters), max-related 12.
- Mode `full` was skipped for every commit: 65k-127k tokens do not fit the 32k context.
- Every finding was checked against the code at the reviewed commit. All three commits are
  merged and green on CI, so "does not compile" claims are checked against that too.

Classes: **real** (a true problem), **FP-context** (false, and the missing or misleading
context explains it), **FP-other** (false for another reason: language knowledge, taste).

| commit | mode | prompt tokens (o200k) | secs | findings | real | FP-context | FP-other |
|---|---|---|---|---|---|---|---|
| badf2ec3c | none | 2,636 | 284 | 0 | 0 | 0 | 0 |
| badf2ec3c | digest | 8,796 | 548 | 0 | 0 | 0 | 0 |
| 8a292af85 | none | 3,564 | 391 | 1 | 0 | 1 | 0 |
| 8a292af85 | digest | 21,161 | 669 | 1 | 0 | 1 | 0 |
| 659e107f2 | none | 5,643 | 389 | 2 | 0 | 0 | 2 |
| 659e107f2 | digest | 14,535 | 605 | 1 | 0 | 1 | 0 |
| **total** | none | 11,843 | | 3 | 0 | 1 | 2 |
| **total** | digest | 44,492 | | 2 | 0 | 2 | 0 |

## Findings

- `8a292af85` none, high: "`.values((col.eq(v), …))` is invalid Diesel, will not compile".
  False: that is Diesel's insert syntax, and the commit builds. FP-context (external API
  not in the prompt).
- `8a292af85` digest, low: "`tests_store.rs:238` compares an unsorted Vec, may flake".
  False: `measurement_totals` orders by `(plugin, kind)` (`measure_rows.rs`). FP-context:
  the order is in the body, and `measure_rows.rs` was not among the related files.
- `659e107f2` none, high: "`.unwrap_or_else(PoisonError::into_inner)` is invalid syntax".
  False: a function path is a valid closure argument. FP-other.
- `659e107f2` none, low: "`Mutex<Option<HashSet>>` can be `Mutex<HashSet>`". False for a
  `static`: `HashSet::new` is not `const`. FP-other.
- `659e107f2` digest, high: "`plan_dir.rs::find` destructures `Scan { found, skipped }`
  but `scan` returns a tuple". False: the tuple `scan` is `disk.rs`'s, a different
  same-name method. The digest showed both, and the model mixed them up. FP-context
  (misleading same-name context: the noise the usage filters target).

Note: real issues were 0 in both modes. With this model and three small, already reviewed
commits, the sample is too small to tell the modes apart. The one digest-only error came
from a same-name definition in another file.
