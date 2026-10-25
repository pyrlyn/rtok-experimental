# `guard`

Stops the model from paying twice for the same result.

| | |
|---|---|
| Surfaces | PreToolUse hook; `rtok guard check` (pi / OpenCode plugin) |
| Spec | the **Replaces** column of [`docs/plugins.md`](../../../docs/plugins.md) |
| Default | on |

## Mechanism

An identical Read or Bash call (same tool, same normalised input) within the last N turns of
the same session is denied with a reason pointing at the earlier result:

```
guard: identical to toolu_… 2 turns ago — use rtok expand <archive_id> or change the input
```

Denials are lossless because the earlier result is archived; the model can always `expand`.

Opt-in (T62.1, Claude Code): with `skills = true` a `Skill` call whose `SKILL.md` exceeds
`skill_max_bytes` is denied before the host injects the body. The reason is the file's
heading map (each heading with its first line) plus `[rtok <id> · N lines · expand: rtok
expand <id>]`, so the model pulls one section with `rtok expand <id> --grep <heading>`
instead of carrying the whole body in every later request. Skills whose frontmatter names
`allowed-tools`, `model`, `context` or `agent` always load whole (a denied skill does not
apply them). Measured 2026-09-17 on this machine: 17 bodies, median 8,863 B, max 248,175 B
(`research.md` §10.2).

Opt-in (T369): with `grep_symbol = true` a native `Grep` for one identifier (`foo`, `fn foo`,
`class Foo`, also `\bfoo\b`, `\bfoo\(`, `foo\(`; no `glob`, `type`, `-i`, multiline, and a `path` only
when it is a directory inside the project, which keeps the definitions under it) is denied
with that symbol's definitions from the index, the same `path:line kind` and source `symbol`
prints, plus the reference count, within `plugins.inject.budget_tokens`. One to five
definitions only; none, more, a missing index or an index that no longer matches the file
lets the Grep run. Replay: `research.md` §29.5.

Opt-in (T50.4): with `deny_grep_glob = true` a native `Grep` or `Glob` is denied with a
reason naming rtok's MCP `search` or `tree` instead; a `Grep` that `grep_symbol` answers gets
that answer first. The deny is recorded as a `native_deny` measurement with zero bytes, so it
claims no saving. While the `read` plugin is disabled there is no `search`/`tree` to point at,
and both run.

## Config

```toml
[plugins.guard]
enabled = true
window_turns = 8
deny_grep_glob = false  # T50.4: deny native Grep/Glob, point at MCP search/tree
grep_symbol = false     # T369: answer a one-identifier Grep from the symbol index
skills = false          # T62.1: digest oversized SKILL.md on PreToolUse(Skill)
skill_max_bytes = 8192
```

## Tasks

See `roadmap.md` § `guard`. Checks in `plan/`.

T2.6 deny duplicate Read/Bash within `window_turns` when an archive id exists.
T62.1 digest a skill body over `skill_max_bytes` (opt-in).
T369 answer a symbol-shaped `Grep` with the indexed definition (opt-in).
T50.4 deny native `Grep`/`Glob` in favour of MCP `search`/`tree` (opt-in).

## Status

Implemented: duplicate deny (T2.6), skill digest (T62.1), `grep_symbol` (T369),
`deny_grep_glob` (T50.4).
