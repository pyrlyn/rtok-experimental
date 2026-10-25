# `rtok review`: token-efficient code review context

`rtok review` takes a unified diff and builds one prompt for a reviewer model: the diff, plus
short digests of the changed files and of the files that use them or that they use. A digest
holds imports, doc comments, signatures and data types. It never holds a function body. The
command can send the prompt to a model, or print it with its token estimate.

It is not `rtok graph review` ([docs/review.md](../../docs/review.md)). That command calls no
model and answers "what should I read first". This one answers "what does a reviewer model
need to see next to the diff".

## How it works

1. **Diff in** ([`diff.rs`](diff.rs)). With `--range A..B` the post-change tree `B` is
   indexed, not whatever is checked out. That tree is a detached `git worktree` under
   `RTOK_REVIEW_TREES` (default: the temp dir), one per commit SHA and reused, so its index
   is cached by commit too. A clean checkout already at `B`, and a single-revision range
   (compared with the working tree), use the checkout as is. The diff comes from `--diff FILE`, from `--diff -` or
   piped stdin, or from `git diff <--range>`. With no flag and no pipe it is `git diff HEAD`.
   The parser reads `git diff` and plain `diff -u` output into files, hunks and
   added/removed lines. It also handles new, deleted and renamed files.
2. **Changed symbols** ([`usage.rs`](usage.rs)). The project is indexed through the existing
   graph index (`index_for`, the same call `rtok graph` makes), so no second indexer is
   involved. A changed symbol is either:
   - an index definition whose span a hunk touches, or
   - a name declared on an added or removed line (`fn`, `struct`, `def`, `class`, ...).
     These catch deletions and renames, which the new-side index no longer has.

   Only source files count, by extension.
3. **Usage search** ([`usage.rs`](usage.rs), [`imports.rs`](imports.rs)).
   - **Dependents** reference a changed symbol (`symbol_ref_groups`), or an alias of it
     (`use … as X`, found through the files that import the name). Their digest shows the
     calling definitions.
   - **Dependencies** define something the changed code calls (`symbol_callees` →
     `symbol_defs`). Their digest shows the callee.

   The index links references by bare name. Each link is checked before it counts:
   1. **Imports.** The referencing file's `use` / `from … import` / `import … from`
      statements are parsed, and a definition file's module path is derived from its path
      (`crates/rtok-store/src/symbols.rs` → `rtok_store::symbols`). A file that imports the
      name from one definition's module, or from a parent that re-exports it, belongs to
      that definition only. A name imported from a module no indexed file is in (another
      crate, std) links to none of them.
   2. **Qualified paths.** An inline `store::value::Value` counts as an import, but only if
      the whole path points at the file, so `crate::web::value::Value` does not count for
      `store/value.rs`.
   3. **Kinds.** Reference kinds come from the index (`symbol_ref_kinds`). A `call` links
      only to callables, or to a struct or class (constructors). A `type`, `class` or
      `implementation` reference links only to types. A module is never a dependency.
   4. **Ambiguity.** If imports cannot decide, exactly one same-directory definition is
      taken as evidence. Otherwise a name with more than `--max-defs` (default 3)
      definitions makes the link `ambiguous`. Ambiguous files are kept but ranked last, so
      `--max-related` and `--budget` drop them first.

   Common std and trait method names (`new`, `push`, `unwrap`, `cfg`, ...) are not followed
   as callees. Related files are ranked unambiguous first, then dependents, then by hits,
   then by path. `--max-related` caps them (default 12).
4. **Digests** ([`digest.rs`](digest.rs)). The text is cut from the file by the index's line
   spans:
   - imports;
   - doc comments and attributes above a definition;
   - the signature up to its opening `{` (shown as `{ … }`), `;` or `:`;
   - Python docstrings;
   - whole bodies of data types only: struct, enum, type alias and interface, and a Rust or
     Go `class`, each up to 40 lines.

   A related file is cut down to its focus definitions plus data types.
5. **Prompt** ([`prompt.rs`](prompt.rs)). The order is fixed: instructions, diff,
   changed-file digests, then related digests in rank order. A section repeated by path or
   by body is dropped as a duplicate. With `--budget N`, sections that would push the
   estimate past N are dropped, and the prompt says how many. The diff is always kept.
   The same input gives the same prompt byte for byte.
6. **Review** ([`provider.rs`](provider.rs)). A `Reviewer` trait sits behind the call, so
   tests use a fake. The HTTP reviewer uses the providers and key variables of `rtok batch`:
   `ANTHROPIC_API_KEY`, then `OPENAI_API_KEY`, or the one named by `--provider`.
   - The model comes from `--model`, then `RTOK_REVIEW_MODEL`, then a default per provider.
   - The base URL comes from `--base-url`, then `RTOK_REVIEW_BASE_URL`.
   - The key is never printed.

## Why this beats the usual approaches

- **Changed files only.** A reviewer that sees only the diff, or only the changed files,
  has to guess the contracts of the code around it. Those guesses produce false positives:
  "this may panic if `x` is empty" when the callee already handles it, or "the caller does
  not handle the new error" when it does.
- **Full context.** Sending every changed and related file whole removes the guessing, but
  costs about 7× the tokens of a digest prompt on the sample below. Most of those tokens are
  function bodies that have nothing to do with the change.
- **Digests.** `rtok review` sends the part of the neighbouring code that sets the contract,
  meaning signatures, types and docs. It costs a small multiple of the diff.

## Why it works

Most cross-module review mistakes are about the API of a neighbouring module: what a
function takes and returns, what a type holds, what a doc comment promises. They are rarely
about logic inside the changed file, and that logic is already in the diff. Signatures,
types and docs are what a reviewer needs to check a contract, and they are a small share of
a file.

So far this has been measured for tokens only (below). The quality comparison (false
positives per mode) has not run yet, so no claim about fewer false positives is backed by
numbers here.

## Evaluation

[`bench/review-eval/eval.py`](../../bench/review-eval/eval.py) runs three real merged
commits through three context modes:

- `--context none`: the diff only;
- `--context digest`: the default;
- `--context full`: changed and related files in full.

Tokens are counted with tiktoken `o200k_base`. Results are in
[`bench/review-eval/results/`](../../bench/review-eval/results/).

| commit | files | diff only | diff + digests | diff + full files | digest saving vs full |
|---|---|---|---|---|---|
| `badf2ec3c` (#1280) | 4 | 2,636 | 8,796 | 65,482 | 86.6% |
| `8a292af85` (#1258) | 8 | 3,564 | 21,161 | 127,246 | 83.4% |
| `659e107f2` (#1274) | 4 | 5,643 | 14,535 | 119,977 | 87.9% |
| **total** | | 11,843 | 44,492 | 312,705 | **85.8%** |

**Quality: Bonsai 27B, local.** Reviewer: `prism-ml/bonsai-27b` (Ternary-Bonsai-27B, MLX
2-bit), run on LM Studio's OpenAI-compatible server at a 32k context. The run was
`--provider openai --base-url http://localhost:1234`, with `RTOK_REVIEW_TIMEOUT_SECS`
raised for the slow local model. `full` does not fit in 32k on any of the three commits, so
it was skipped.

Each finding was checked against the code. Results and the per-finding notes are in
[`results/quality-bonsai-before.md`](../../bench/review-eval/results/quality-bonsai-before.md).

| mode | findings | real | FP from missing or misleading context | other FP |
|---|---|---|---|---|
| diff only | 3 | 0 | 1 | 2 |
| diff + digests | 2 | 0 | 2 | 0 |

The sample is small: three already reviewed commits, and no real issue in any of them. All
findings were false positives in both modes, so this run shows no quality gain. One
digest-mode error came from a same-name `scan` in another file. That is the noise the usage
filters below address.

## Enable, disable, use

`rtok review` is built with the `graph` feature, which is in the default build. There is no
config switch. It does nothing unless you run it. To leave it out of a build, build without
`graph`.

```sh
rtok review --range main...HEAD --dry-run         # sizes only
rtok review --range HEAD~1 --print-prompt > p.md  # the prompt itself, estimate on stderr
git diff | rtok review --budget 20000 --json      # review with a budget, JSON out
rtok review --diff change.patch --provider openai --model gpt-4.1
rtok review --range HEAD~1 --context none         # baselines: none | digest | full
```

The JSON output holds:

- `files` (path, hunks, changed symbols);
- `related` (path, relation, symbols, focus, hits);
- `prompt` (tokens, diff_tokens, context_tokens, included, dropped, duplicates,
  over_budget);
- `reviewer` and `review`.

## Test

```sh
cargo test --lib review::
```

| what | where |
|---|---|
| diff parsing: files, hunks, new, deleted, renamed, plain `diff -u`, declared names | `diff.rs` tests |
| digests keep imports, docs, attributes, signatures, fields and docstrings, and drop bodies (Rust, Python, focus) | `digest.rs` tests |
| prompt order, dedup, determinism, token budget, over-budget diff | `prompt.rs` tests |
| provider request and reply shapes | `provider.rs` tests |
| fixture repo: usage search finds the dependent and the dependency and skips the unrelated file; digests land in the prompt without bodies; full mode costs more and none less; budget; pluggable reviewer | `tests.rs` |
| import parsing (Rust trees, aliases, multiline, Python, JS/TS), module paths, `points_at`, inline qualified paths | `imports.rs` tests |
| same-name `Value` in several modules: the import filter keeps only the importer of the changed module and drops an external-crate import; qualified paths and `use … as` aliases count; a call does not link to a same-name trait or a module; more than `max_defs` unproven definitions are ambiguous and ranked last; `--range` indexes the right-hand commit and caches it by SHA | `tests.rs` |

## Limitations

- **Trees for `--range` stay.** Each reviewed commit leaves a worktree (and its index) under
  `RTOK_REVIEW_TREES`. Remove them with `git worktree remove <dir>` or delete the directory
  and run `git worktree prune`. A rhs that is not a commit (`--diff`, stdin) uses the
  checkout.
- **Text-level import parsing.** Rust, Python and JS/TS imports are parsed line by line.
  Glob imports (`use x::*`), `import x as m` followed by `m.f`, macros, re-exports through
  several hops and `#[path]` modules give no evidence. Those links fall back to directory
  or name and may be marked ambiguous. There is no type resolution and no SCIP: a method
  call `x.len()` is still matched by name.
- **Common names skipped.** A call through a common name (`new`, `run`, ...) is not
  followed.
- **Line-based digests.** The digest is cut from text by spans. A signature that runs past
  8 lines is truncated, and an odd brace layout can leave part of a line.
- **Rough budget.** The budget uses rtok's chars-per-token estimate, not the provider's
  tokenizer. Leave a margin.
- **Free-text output.** The review is the model's text, not structured findings.
- **One request, no retry.** Each run sends one request with a 300 s timeout.
- **Busy store.** A busy rtok store can answer `database is locked`. Run again.
