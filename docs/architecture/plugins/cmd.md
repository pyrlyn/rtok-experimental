# cmd plugin

![A PreToolUse hook rewrites a Bash command to rtok run, which runs it, filters the output through formatters and rules, archives the raw bytes and prints the shortened text with an expand trailer](cmd.svg)

Source: [`cmd.d2`](cmd.d2). Plugin docs: [README](../../../src/plugins/cmd/README.md). Index: [all diagrams](../README.md).

`cmd` shortens what a Bash tool call prints. A `PreToolUse` hook rewrites the command so it
runs through `rtok run`; that process captures stdout and stderr, filters them with a
per-family formatter or a TOML rule, archives the raw bytes when something was dropped and
prints the result with a one-line trailer that names the archive id. The exit code of the
wrapped command is preserved.

## Walkthrough

1. **The hook sees a Bash call.** `CmdPlugin::pre_tool` calls `hook::pre_tool`, which returns early
   unless the tool is `Bash` and `[plugins.cmd] rewrite` is true. The hook path does no
   filesystem walk and starts no subprocess.
2. **Decide whether to wrap.** `skip_wrap_host` refuses a command whose first word is in
   `never_wrap` (default `rtok`, `sudo`), one with a heredoc (`<<`), a trailing `&`,
   `--interactive`, or `-i` on a stem listed in `interactive_stems`. `changes_shell_state`
   refuses `cd`, `export`, `source` and the other builtins whose effect would be lost in a
   child shell, and a bare assignment. A sub-agent's command that runs `git` is never wrapped
   (`runs_git`). Leading `cd <dir> &&` hops are peeled by `guard::strip_cd_hop` and stay
   outside the wrapper.
3. **Rewrite.** The hook returns `PreToolDecision::Rewrite` with
   `<run_guard>rtok run [--agent <id>] -- '<cmd>'`. The guard (`names::run_guard`) makes a
   POSIX host shell run the original command if `rtok` is not on its `PATH`. `--agent`
   carries a validated sub-agent id so dedup pointers stay inside one context.
4. **Run.** `run::run` picks the shell (`[plugins.cmd] shell`, then the environment, then
   the host default), runs the command through `capture_body`, which caps the captured
   bytes and appends a notice when it cuts, and keeps the exit code. `rtok filter --archive`
   (OpenCode) enters at `emit_filtered` with a ready payload instead.
5. **Print raw, or point at an earlier result.** `emit_filtered` prints the bytes untouched
   when the command is an `rtok expand` or the store cannot be opened. Otherwise
   `identical_result` replaces a byte-identical body from the same session with a one-line
   pointer, and for `cat` of a file or range read before, `read::cache::hit` returns a
   pointer or a diff. `identical_result` records a `dedup` measurement.
6. **Skip tiny bodies.** A body no longer than its trailer line prints verbatim and records
   a `raw` measurement with equal before and after bytes.
7. **Compress.** `formatters::compress` first tidies `mise` chatter and collapses polling
   loops (`mise::tidy`, `loops::collapse`). `compress_text` then passes a command the agent
   already bounded (`bounded::is_bounded`: `sed -n`, `head`, `tail -n`, `grep -A/-B/-C/-m`)
   through unchanged. `resolve` picks the family; a chain of two or more distinct programs
   (`bounded::mixed_chain`) goes to the `[script]` rule.
8. **Pick the filter.** A JSON body goes to `rules::apply_parsed`. Otherwise `format()`
   dispatches by program and subcommand to a formatter (`cargo`, `git`, `ls -l`, `cat`,
   `grep`/`rg`, `gh`, `pytest`, `python`, `docker ps`, `kubectl get`, `mise`, and more
   in `compact.rs`, `diffs.rs`, `catfile.rs`, `grep.rs`, `gh.rs`, `python.rs`). With no
   formatter, `rules::apply` uses the rule `Settings::pick` selects from
   `rules/default.toml` and the user's `rules` and `rules_dir` files: drop and keep
   patterns (error, warning, panic, fail and traceback lines always stay), dedupe,
   grouping, then a head and tail cut with an `… N lines omitted (expand <id>)` marker.
   A non-zero exit keeps the last `fail_tail_lines` lines verbatim.
9. **Never worse than raw.** Valid UTF-8 output that the filter did not shorten (at or under
   `trailer_min_lines` lines), or whose printed form (filtered text plus trailer) is not
   shorter than the raw body, is printed as the raw bytes with a `raw` measurement.
10. **Archive and trailer.** `needs_pointer` decides whether a trailer is needed: more than
    `trailer_min_lines` lines, or a shortening that dropped something a reader could miss.
    Folding whitespace or stripping ANSI colour alone needs none. When a pointer or an inline
    `expand <id>` marker exists, `put_archive_for(body)` stores the raw bytes (not the lossy
    UTF-8 view) and the output ends with `[rtok <id8> · N lines]`.
11. **Measure.** One `Measurement` (`plugin: "cmd"`) records the host-visible bytes before and
    after, with kind `formatter`, `rule`, `skill`, `unmatched` or `raw`. `ref_id` is
    `<family>:<id>` when an archive row exists.
12. **Expand.** The model runs `rtok expand <id8>` (or the MCP `expand` tool) and gets the
    original bytes. That output is never deduped or shortened again.

## Where it lives

| Box | Code |
| --- | --- |
| `CmdPlugin::pre_tool` and wrap gate | `src/plugins/cmd/hook.rs` (`pre_tool`, `skip_wrap_host`, `changes_shell_state`) |
| Rewrite text | `src/plugins/cmd/run.rs` (`wrap_quote`), `src/names.rs` (`run_guard`) |
| `rtok run` | `src/plugins/cmd/run.rs` (`run`, `capture_body`); `src/cli/run_cmd.rs` |
| `rtok filter --archive` | `src/cli/run_cmd.rs` (`filter`) |
| `emit_filtered`, trailer | `src/plugins/cmd/run.rs` (`emit_filtered`, `needs_pointer`, `trailer`) |
| Dedup and re-read | `src/plugin.rs` (`identical_result`); `src/plugins/cmd/catfile.rs` (`reread_key`); `src/plugins/read/cache.rs` |
| Route and filter choice | `src/plugins/cmd/formatters.rs` (`compress`, `compress_text`, `format`, `family_of`) |
| Bounded commands, mixed chains | `src/plugins/cmd/bounded.rs` (`is_bounded`, `mixed_chain`) |
| Formatters | `src/plugins/cmd/compact.rs`, `diffs.rs`, `catfile.rs`, `grep.rs`, `gh.rs`, `python.rs`, `mise.rs`, `loops.rs` |
| Rules | `src/plugins/cmd/rules.rs` (`Settings`, `apply`, `apply_parsed`), `src/plugins/cmd/logfmt.rs`, `rules/default.toml` |
| `expand` | `src/expand.rs` (`fetch_range`) |
| Config | `crates/rtok-config/src/lib.rs` (`Cmd` section) |

## Invariants

- Lossless: anything a reader could miss is archived before it is dropped, and `expand <id>`
  returns the raw bytes, invalid UTF-8 included.
- The wrapped command's exit code is preserved exactly.
- Nothing is redacted.
- Fail open: if the store cannot be opened or the archive write fails, the command's own
  bytes are printed.
- The hook path stays within its 10 ms budget: no filesystem walks, no subprocesses.
- The shell is parsed only as far as the first argv word, except in `bounded.rs`, which
  lexes quotes and the `|`, `&&`, `||`, `;` separators.
- Every run writes one `Measurement`; no row, no saving claim.

## Related

[Overview](../overview.md) · [Plugin contract](../plugin-contract.md) ·
[Hook flow](../hook-flow.md) · [archive](archive.md) · [guard](guard.md) · [read](read.md) ·
[toon](toon.md)
