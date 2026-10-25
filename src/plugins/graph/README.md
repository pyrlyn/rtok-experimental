# `graph`

Five capped tools over a symbol index rtok builds itself with tree-sitter-tags, instead of
four servers with 130+ tool descriptions in every request.

| | |
|---|---|
| Surfaces | MCP `symbol(name, path?, kind?)`, `callers(name?, path?, kind?)`, `impact(name?, depth?, path?)`, `outline(path, kind?)`, `explore(query, path?)` (T68.1); CLI `rtok graph affected [--since|--staged] [--json]` (T68.5); CLI `rtok graph review [--since|--staged] [--json]` (T454) |
| Spec | the **Replaces** column of [`docs/plugins.md`](../../../docs/plugins.md) |
| Default | on |

## Mechanism

Walk the repo (respecting `.gitignore`), run the tags queries of the `read` plugin's grammars
for definitions and reference sites, and store them in the `symbols` table, scoped to the
canonical repo root so one store holds many repos (T8.3). A file whose mtime and size are
unchanged is never opened; a file whose stat moved but whose sha256 did not is not re-parsed
(T8.4). Every reference row also stores `scope`, the innermost definition enclosing it in the
same file, which is the call edge `callers` groups by and `impact` walks (T8.5).

`symbol` returns each definition and its source, at most `body_lines` lines each (T8.6).
`callers` returns one line per calling definition. With no `name` it lists the unreferenced definitions (`graph dead`), narrowed by `path` and `kind` (T665). `outline` takes `kind` (`fn`, `struct`, …) to keep one kind of line. `impact` walks those edges breadth-first
to `depth`, then adds `changes with: a.rs (7), b.rs (4)` when git history pairs the defining file with others (T371). With `path` and no `name`, the same walk starts from that file's
definitions and lists reachable test files (`rtok graph affected` does this for
`git diff --name-only`). `outline` is the `read` plugin's `map` mode. `explore` answers a free-text code
question in one call: the query's identifiers resolve to definitions (exact, else prefix
best-5 by reference count), then each definition body, the caller chains between the resolved
symbols (`symbol_paths`, ≤ 3 hops) and one impact depth-1 line per symbol — the tags index
and the optional LSP backend assemble the same answer through one assembler (T68.1). Every
response is capped at `max_tokens` (head + "N more, expand <id>", full text archived).

It is a tags index, not a type-resolved call graph (`plan.md` §0 non-goals). Measured recall
against hand labels: definitions 30/30, references 40/114 — the misses are type positions,
macro bodies and path-qualified calls, listed in `PLAN.md` under "Known misses".

## Config

```toml
[plugins.graph]
enabled    = true
max_tokens = 2000
body_lines = 40
```

## Tasks

See `roadmap.md` § `graph`. Checks in `plan/`.

T8.1 symbol index · T8.2 MCP tools · T8.3 per-root scoping · T8.4 stat-gated freshness ·
T8.8 labelled hit rate · T8.5 call edges · T8.6 definition bodies · T8.7 `impact` ·
T68.1 `explore` · T68.5 `affected`.

## Status

T8.1–T8.8 done (T8.1/T8.2 2026-09-02, T8.3–T8.7 2026-09-04). Gate P8 passed 2026-09-03 on
description tokens and index time. Gate P8b is open: it needs the P9 task-set comparison, and
its recall clause was amended after T8.8 measured the index (`plan.md` §6). T68.1 added
`explore` 2026-09-18 and T329.18 added `graph_diff` 2026-10-10 and T329.16 added `graph_export` 2026-10-10; the surface is seven tools / 165
description tokens (measured, `cargo nextest run -p rtok graph_surface`), still ≤ 170. T68.5 added `rtok graph affected`
and `impact(path)` with no `name` (same walk, no Measurement on print). T454 added
`rtok graph review`: a risk-ranked reading list for a git diff, CLI only, same five MCP tools. `graph_diff` ends a project's changed list with `untested: a, b` for changed functions, methods and classes that no test reaches within two callers (T664).
T329.18 added `rtok graph diff` and the sixth tool `graph_diff` (what a change did to the graph, the old
side read from git's object database); the surface is then 155 description tokens, gate 160.
T329.16 added `rtok graph export` and the seventh tool `graph_export` (the graph as redacted `rtok.graph.v1`
JSON, schema in `docs/schemas/`); the surface is then 165 description tokens, gate 170. T329.31 added `--format svg|png` to the command (`draw.rs`);
the MCP tool stays JSON.
T329.29 gave `graph diff` a `--from` per project (`PROJECT:REF`), `--from-export FILE` (added and removed symbols and
registry links against a saved export; CLI only), and the `changed, not analysed` listing.
T329.35 put the same report on the web graph page (Compare mode): `diff::page` answers the `/ws` `diff` request with
`DiffReport`, the typed shape `--json` prints; the old side is a ref or the text of a saved export the page read itself.
T485 added the same view to `rtok tui` (Graph tab, `c`): a typed ref, `PROJECT:REF` list or saved-export path goes through `diff::page_of`, the core `page` shares, and the panel uses the web panel's marks and sections.
