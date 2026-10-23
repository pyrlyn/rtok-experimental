# Graph review

`rtok graph review` turns a git diff into a short, risk-ranked reading list: which changed
files to read first, which changed definitions no test reaches, and which line ranges to
read. It is deterministic. No model is called, and nothing is posted unless CI does it.

## Why

A reviewer, human or agent, usually reads a whole diff or whole files. On an agent that costs
tokens, and most of those lines are not where the bugs are. `graph review` caps the reading at
800 source lines and puts the riskiest code first, so the review starts where a mistake is
most likely and most expensive:

- **Untested code first.** A changed function no test reaches is the strongest signal. The
  list names each one (`untested: ...`), which is also a ready list of tests to write.
- **Security-named code next.** Names built from words like `auth`, `token`, `sql`,
  `verify` or `sign` get extra weight.
- **Widely called code.** Each direct caller adds a little, up to a cap.
- **Whole definitions, not fragments.** A hunk inside a short function shows the whole
  function, so the reader does not open the file again for context.
- **Nothing hidden silently.** Regions that do not fit the budget are named under
  `omitted:`.

It is a reading order, not a gate: a high score never fails a command or a CI job.

## Usage

```bash
rtok graph review                    # working tree against HEAD
rtok graph review --staged           # staged changes only
rtok graph review --since origin/main
rtok graph review --since origin/main --json
rtok graph review --project <id|dir> # another registered project
```

The diff is read in every project of the scope (the project and the projects it links to);
with more than one, each project with changes gets its own `[name]` block.

## How it works

1. **Diff.** `git diff --name-only` lists the changed files; `git diff -U0` gives the changed
   line ranges in the new file. An empty diff prints `no changes`.
2. **Index.** The project's symbol index is refreshed (the same index as `rtok graph
   impact`), and each changed file's definitions are read from it.
3. **Support code.** Fixtures, benchmarks, examples and files marked in `.gitattributes`
   are listed but not scored (see [Support code](#support-code)).
4. **Score.** The functions, methods and classes a hunk touches are scored: up to 8 per file
   and 400 per diff. Untouched definitions in the same file are not.
   For each, the impact walk (depth 3) finds its callers and whether any path reaches a test
   file. A file's score is the highest score of its definitions.
5. **Regions.** Each hunk is widened to the enclosing definition when that definition is at
   most 40 lines, else padded by 4 lines each side. One region is at most 120 lines; touching
   regions merge.
6. **Budget.** 800 lines in total, at most 0.4 of them (and at most 500 lines) for one file. Files
   are served round-robin, highest score first. Files that score 0 get lines only after every
   scored file got all its regions.

### Score

| Term | Adds |
|---|---|
| No test reaches the definition | 0.30 (0.05 when a test does) |
| Name has a security token | 0.20 |
| Direct callers | callers / 20, at most 0.10 |

A token is a whole word of a `snake_case` or `camelCase` name: `sign_in` matches `sign`,
`assign` and `signature` do not. A definition inside a test file counts as tested.

The terms follow code-review-graph's `compute_risk_score`, without its flow and community
terms (this index has neither), so the score ranges from 0.05 to 0.6.

### Level

The first line names the level of the highest file score:

| Level | Score | Means |
|---|---|---|
| `high` | above 0.45 | untested and security-named |
| `medium` | above 0.25 | untested, or tested but security-named with callers |
| `low` | otherwise | reached by a test |

### Support code

Code that ships to no user would otherwise be listed `untested` next to production code.
A changed file is support code when:

- a `.gitattributes` attribute `linguist-vendored`, `linguist-generated` or
  `linguist-documentation` is set for it, or
- no such attribute is set or unset for it, and a directory on its path is `fixtures`,
  `fixture`, `testdata`, `bench`, `benches` or `examples`.

An explicit `false` or unset attribute keeps a file in review even inside those directories.
These are GitHub Linguist's attribute names, so one line also folds the file in the pull
request diff view:

```gitattributes
tests/data/** linguist-generated
examples/** -linguist-vendored
```

A support file stays in the list with score 0, the word `support` and no `untested`, and reads
only from the lines scored files leave. When `git check-attr` fails, the directories alone
decide.

## Output

This change itself, reviewed together with the commit before it (which added the bench
files); most line ranges are cut here:

```text
review: medium 0.4000
src/plugins/graph/review.rs 0.4000 untested: allocate_budget, support_paths, parse_check_attr, scored_names, file_json
  L8-26
  L40-57
  L357-423
src/plugins/graph/mod.rs 0.1500
  L1290-1306
bench/check_tasks.py 0.0000 support
bench/repos/t9/src/lib.rs 0.0000 support
docs/commands.md 0.0000
omitted: src/plugins/graph/review.rs 2 regions
```

The `untested` names here are a known gap: their tests sit in the same file's
`#[cfg(test)]` module, which is not a test path (see [Limits](#limits)).

`--json` prints `level`, `score` and `files`, each with `path`, `score`, `untested`,
`regions` (`start`, `end`), `regions_omitted` and `support`. A long text answer is cut at
`plugins.graph.max_tokens` with an `expand <id>` for the rest; the command writes no
`Measurement`.

## In CI

Two workflows post the list on every pull request to `main` that is not a draft:

- `.github/workflows/graph-review.yml` runs with read-only rights. It builds rtok, runs
  `rtok graph review --since origin/<base>` for at most 3 minutes (a timeout says so instead of
  failing), heads the text `## rtok review`, cuts it at 50 000 bytes, and uploads it as an artifact.
- `.github/workflows/graph-review-comment.yml` runs from the default branch on
  `workflow_run` and never checks out pull request code. It treats the artifact as untrusted:
  exactly two regular files, a size cap, UTF-8 without control characters, the expected
  heading and footer, `@` escaped, and the pull request head still at the analysed commit.
  Then it updates the one comment that starts with `<!-- rtok-review -->`, or creates it.

## Limits

- Test reachability comes from the symbol index: a test that reaches code through dynamic
  dispatch or macros the index cannot follow is missed, and the code is listed `untested`.
- A test counts only when it lives in a test path (`tests/`, `__tests__/`, `test_*`,
  `*_test.*`, `*.test.*`, `*.spec.*`). Rust unit tests in a `#[cfg(test)]` module of the same
  file do not, so their code is listed `untested`.
- Only functions, methods and classes are scored; a change to a constant, type or config file
  scores 0.
- A deleted file is listed with score 0 and no line ranges: there is nothing left to read.
