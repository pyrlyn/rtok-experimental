# Agent notes — `cmd`

**Owns** `src/plugins/cmd/**` (`run.rs`, `rules.rs`, `formatters.rs`, `hook.rs`), `rules/default.toml`,
`src/expand.rs`, `tests/cmd_golden/`.

**Contract**: the `Plugin` trait, `Ctx` and the host capabilities come from the published
`rtok-plugin-sdk` crate (`crates/rtok-plugin-sdk`), not from `crate::plugin` — import them as
`rtok_plugin_sdk::…`.

**Invariants**
- Lossless: anything a reader could miss is archived before it is dropped; `rtok expand <id>` returns the raw bytes. A whitespace/ANSI-only change drops nothing and stores no archive row (T160).
- JSON: a top-level array of uniform scalar objects prints as one TOON table when that
  form is shorter (keys once, same first `json_items` rows and `… +K more` tail); mixed
  and nested arrays keep the per-element lines. Builds without `toon` keep the old path.
- Exit code of the wrapped command is preserved exactly.
- Logfmt cut (T683, `logfmt.rs`): when a head/tail cut happens, the pairs every record shares print once in an `[every record: …]` first line and leave the printed records; error/warn lines stay verbatim, and each `omitted` marker lists the run's varying keys as `first..last`. Never emit a pair that is not in the raw lines.
- Never wrap: first word in `never_wrap` (default `rtok`, `sudo`), heredocs (`<<`), trailing `&`,
  `-i`/`--interactive`, or when `rewrite = false`.
- Never redact. A fixture with a fake AWS key must pass through unchanged (T3.3 Check).
- Every run writes one `Measurement { kind: formatter | rule | raw | unmatched | skill }`.
  `raw` is a tiny body below the trailer gate or a T176 bounded passthrough (both by
  design); `unmatched` is a picked rule/formatter that shrank nothing (T177's actionable
  share — `rtok stats` splits the two).
- The PreToolUse hook path must stay under 10 ms: no filesystem walks, no subprocesses.
- No third-party tool is executed, linked or imported (D6). Formatters are written here from
  the family list in `research.md`.

**Do not** parse shell syntax beyond the first argv word; do not add a shell parser dependency.
One exception (T176): `bounded.rs` lexes quotes, `\` escapes (T444; `\`-newline is a continuation), and
`|`/`&&`/`||`/`;`/newline to spot a command the agent already bounded (`sed -n a,bp`,
`head`/`tail -n`, `grep -A/-B/-C/-m` — a recursive `grep` (`-r`/`-R`) also needs
`-m`/`--max-count`, or the hit list is unbounded; `rg` is recursive by default and has no such
recursive flag (`-r` is `--replace`), so this gate is `grep`-only and `rg` stays on the plain
context/max-count check — `cat -n` of exactly one named file; T931: a literal `echo`/`printf`
label between windows (`sed -n 1,80p a; echo ---; sed -n 1,40p b`) is skipped like `cd`, an
expansion in it is not); those pass through unchanged up
to `bounded::MAX_BYTES`. `formatters::compress` reuses the same lexer
(`bounded::mixed_chain`, T177) to route a single Bash string only when it chains 2+ DISTINCT
programs to `[script]` in `rules/default.toml` — `cargo build && cargo test` or
`cd x && cargo test` keep using `cargo`'s own formatter/rule.
T386 peels `mise exec`/`mise x` when the command is after `--` (or `-c`), and
`just -c`/`--command`. T454 peels the same way, at most four times: a leading
`FOO=1`, `uv run`/`uvx`/`npx`/`pnpm exec|dlx`/`yarn dlx`/`poetry|pipenv|hatch run`,
`python|python3|py -m`, and `cmd /c`. `mise run` / `just <recipe>` use an echoed line
(`[task] $ …`, a recipe line) with that same peel. No visible inner command stays on argv[0].

**Checks**: `plan/` T3.1–T3.6. Golden tests live in `tests/cmd_golden/*.{in,out}`. Each `.in`
carries an `argv:` / `exit:` / `min_saving: <percent>` header before `---` (T238); the golden
test in `formatters.rs` fails a file below its floor, one whose output outgrows its input, or
one missing `min_saving:`.
