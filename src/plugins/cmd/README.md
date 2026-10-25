# `cmd`

Every Bash output archived raw, filtered for the model, measured.

| | |
|---|---|
| Surfaces | PreToolUse(Bash) hook → `rtok run -- <cmd>`; `rtok expand <id>` |
| Spec | the **Replaces** column of [`docs/plugins.md`](../../../docs/plugins.md) |
| Default | on |

## How it works

1. The PreToolUse hook rewrites `command` to `rtok run -- <command>` (on POSIX led by a
   `command -v rtok` guard that runs `<command>` as-is when the host shell has no `rtok`, T922) unless the command
   first word is in `never_wrap` (default `rtok`, `sudo`), or it contains a heredoc, `&`
   background, or `-i`/`--interactive`.
2. `rtok run` executes via `$SHELL -c` (no login shell: the host already ran its profile),
   keeps the exit code, and — when the shortening dropped something — writes the raw
   output to `~/.rtok/archive/<id>` and an `archive` row.
3. A per-family formatter (`cargo`, `git`, test runners, `ls`/`find`; measurement kind
   `formatter`) summarises the output. Other families go through the TOML rules in
   `rules/default.toml` (kind `rule`): head/tail/dedupe/drop, always keeping
   `error|warning|panic|FAIL|Traceback`.
4. Output over 40 lines gets a trailer: `[rtok <id8> · N lines]` (`rtok expand <id8>` resolves the prefix; `--no-ref` or `trailer = false` drops it).
   Below that the trailer prints only when the shortening dropped something a reader
   could miss (a line, a cut string, invalid bytes) and the output does not already name
   `expand <id>` (the rules' `… N lines omitted` marker is a pointer of its own). A
   change that only folds whitespace or strips ANSI colour prints no trailer and stores
   no archive row (T160): there is nothing to expand, and the ~110-byte trailer would
   cost more than the padding saved.
5. Non-zero exit → last 80 lines verbatim. Nothing is redacted.

## Config

```toml
[plugins.cmd]
enabled = true
rewrite = true                   # false: never rewrite Bash commands
never_wrap = ["rtok", "sudo"]    # first-word deny list for the rewrite
```

## Tasks

See `roadmap.md` § `cmd`. Checks in `plan/`.

T3.1 `rtok run` · T3.2 rule engine · T3.3 formatters + default rules · T3.4 PreToolUse rewrite ·
T3.1–T3.6 done 2026-09-02 · T10.2 `rtok filter --stdin` (OpenCode).

## Status

T3.1–T3.6 T10.2 done 2026-09-02.
