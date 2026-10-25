# LSP backend for `graph`

By default the `graph` plugin answers `symbol`, `callers`, `impact`, `outline` and
`explore` from its tree-sitter-tags index in SQLite (`backend = "tags"`). Setting
`backend = "lsp"` routes those same five MCP tools — same names, same callers —
through a language server spoken to over stdio instead. The tags index is then
not consulted for that call. Each LSP answer records one `Measurement` row with
`plugin = "graph"` and `kind = "lsp.symbol" | "lsp.callers" | "lsp.impact" |
"lsp.outline" | "lsp.explore"`, so `rtok stats` attributes it like any other saving.

Each of the five tools takes an optional `project` (id or directory). Without it a call answers for
the working directory's project and the projects it links to; the language server answers one root,
so with `backend = "lsp"` only the first project of that scope answers and the reply says so. (`auto` asks every project, see below.)

The server is picked from the workspace root — rtok never links or shells out to
anything else (D6); it spawns one of these from `PATH`:

| Root marker | Server command | Install |
|-------------|----------------|---------|
| `Cargo.toml` | `rust-analyzer` | `rustup component add rust-analyzer` |
| `compile_commands.json` | `clangd` | `brew install llvm` (macOS) or `apt install clangd` (Debian/Ubuntu) |
| `tsconfig.json` | `typescript-language-server --stdio` | `npm install -g typescript-language-server typescript` |
| `pubspec.yaml` | `dart language-server` | the Dart SDK: https://dart.dev/get-dart |

Other languages (JavaScript, Go, Python, Java, Kotlin, Ruby, PHP, ...) stay on the tags index.

To see which of these a checkout needs, run `rtok doctor --project [PATH]` (default: the working
directory). It counts source files per language the way the index walks them (`.gitignore` and
`[plugins.graph] exclude` honoured), says which server is installed, prints the install command
for a missing one, and notes a missing root marker (for C, how to generate
`compile_commands.json`). The same section reports the graph index and backend, the
`AGENTS.md` / `CLAUDE.md` files of that checkout and the plugins left off there. Saving figures
come only from your own `Measurement` rows of the last 30 days; without rows the line reads
`no measurements yet`. `--json` puts the section under `project`, and the dashboard's doctor page
shows it as the `project` panel for the checkout rtok runs in.

The walkthrough below covers Rust and Dart.

## Rust: rust-analyzer

Install once via rustup, then check the binary answers:

```bash
rustup component add rust-analyzer
rust-analyzer --version
```

`rustup` puts `rust-analyzer` on `PATH`; rtok additionally resolves it through
`rustup which rust-analyzer` when rustup is present.

## Dart: Dart SDK

Install the SDK (https://dart.dev/get-dart), then check it answers:

```bash
dart --version
```

The server is the SDK's own `language-server` subcommand — rtok spawns
`dart language-server`, so no extra install step exists beyond the SDK itself.

## Point a project at it

The marker file decides the workspace root: `Cargo.toml` for a Rust crate,
`pubspec.yaml` for a Dart package. Either set it per project in
`<git root>/.rtok.toml`:

```toml
[plugins.graph]
backend = "lsp"
```

or per invocation from the environment:

```bash
RTOK_PLUGINS_GRAPH_BACKEND=lsp rtok config get plugins.graph.backend
```

which prints `lsp` (default is `tags`). With `--sources` the row reads
`plugins.graph.backend = lsp (env)`:

```bash
rtok config show --sources | grep plugins.graph.backend
```

## Confirm it works

Call MCP `outline` on a sample file, then `callers` on one of its symbols.
`tests/graph_lsp_gate.rs` is the runnable version of this check: it gets
`outline` of a two-symbol `lib/main.dart` through `dart language-server`, asserts
the `OnlyTyped` reference hits through rust-analyzer (it hits through tags too
since T52.5; the tags-miss pin is now a macro-body `macro_callee` reference),
and asserts one `lsp*` measurement row:

```bash
cargo test --test graph_lsp_gate
```

All six tests pass when both servers are on `PATH`; the Dart and LSP tests
skip otherwise.

## Backend `auto`

`backend = "auto"` (T329.9) is opt-in; the default stays `tags`. It chooses per project and language
instead of once for the process. A project whose language has a server (the table above) is asked
first and the answer is headed `(lsp)`. A language with no server gives the tags answer headed
`(tags)`; a server that is missing, not ready, slower than `lsp_timeout_ms` (default 40000) or dead
mid-session gives the tags answer headed `(tags; lsp: <reason>)`, so a crash is a notice, not an
error. Every tags answer records a `tags.symbol | tags.callers | tags.impact | tags.outline |
tags.explore` measurement row beside the `lsp.*` ones, so `rtok stats` shows how often each mode
answered.

In a scope of several projects where a server is installed for some, each project answers for
itself and its part is labelled `[name]` and headed with its mode, so a Rust project can come from
rust-analyzer and a Go project from tags in one reply. A linked project's callers are that project's
own answer; the linked tags traversal runs only when no project of the scope has a server installed,
and then each project's mode is one `[name] (tags)` line at the top.

`[plugins.graph.backend_by_language]` pins one language, named by the project's marker file (`rust`,
`c`, `typescript`, `dart`, `go`, `python`, `javascript`, `java`, `ruby`, `php`, `elixir`, `swift`):
`go = "tags"` keeps Go on the index while the rest use `auto`.

## Text search

`backend = "text"` (T329.10), or `auto` for a project with no language server and no file a grammar
parses (for example a Java or Ruby project), answers by text search. It runs in rtok's own process
with the walk and ignore rules of the `search` tool (`.gitignore`, the graph's `include` and
`exclude`, `plugins.read.search_max_bytes`): no `rg`, `grep` or `ssh` is started, and `ssh://` roots
are not supported. Every answer is headed `(text)`, and one that lists hits ends with a line saying
it may include comments, strings and same-named symbols.

- `symbol` prints each definition line, `path:line kind`, found by `fn`, `def`, `function`, `func`,
  `class`, `struct`, `enum`, `trait`, `interface`, `type` and a few more keywords before the whole
  word. No body is printed, because a text search cannot tell where a definition ends.
- `callers` prints the files that mention the name outside a definition line, `path xN (Lline)`.
- `impact` is one level of the same files; `to` chains need call edges and answer "not available in
  text mode".
- `outline` lists the definition keywords of one file; `explore` assembles definitions and the
  one-level impact, with no call paths.
- `dead` answers "not available in text mode" instead of guessing.

A text answer records a `text.symbol | text.callers | text.impact | text.outline | text.explore`
measurement row (the time spent, not a saving). A project whose root cannot be read is dropped with
`no graph backend available: <reason>`.

## Checked once

Under `lsp` and `auto` (T329.11) rtok checks each project once: the marker file, the server binary on
`PATH`, and the first request that starts the server. The result is the project's record, and later
requests use it without looking again. A missing server is not searched for on every call, and a server
that broke (it died, closed its pipe or timed out) is not restarted on every call; it is dropped from
that project's answers once, with the reason, and the project answers from tags. A server's error reply
to one request is not a break. Concurrent first requests share one check.

The record is kept in memory and mirrored into the store, so `rtok graph projects --json` (and the web
page) show it under `backend`: `backend` (`lsp` or `tags`), `language`, `server` (the language has one),
`reason`, `config` (the `backend` value it was made under), `checked_at` and `next_probe_at` (unix
seconds; the earliest a health check retries a failed record). Changing `backend` or
`backend_by_language` re-checks only the projects whose value changed. A new process uses a record
another process wrote until `next_probe_at`, then checks for itself, so restarting rtok after installing
the server picks it up. Requests never re-check; the health check below does.

## Health check and alerts

Every process that hosts graph (`rtok mcp`, `rtok web`) runs a light check every
`[plugins.graph] health_check_interval_s` (60 s; `0` turns it off) over the projects of its scope. It is
the only code that looks again. Each round it checks that the project root exists, that the manifest
references of the project point at directories that exist, and the capability record. A server installed
since the record was made is picked up in that round. A server that broke is tried again after 60 s, then
120 s, doubling up to 15 minutes; the retry is the same check as the first one (the binary is on `PATH`),
so the next request is the trial, and a server that fails it is dropped again and the wait keeps growing.
Only the record the process holds is retried, so an `rtok web` page never overrides an `rtok mcp` session.

A problem raises an alert once two checks in a row have seen it, and an alert clears after two good
checks, so a brief unmount does not flap (`[plugins.graph] alerts = false` turns alerts off):

| Alert | Raised when |
| --- | --- |
| `missing` | the root was deleted or moved (its parent still exists) |
| `unreachable` | the root or its parent is gone (an unmounted disk), errors, or does not answer a `stat` in 2 s |
| `backend down` | the language server worked, then broke, and the project answers from tags |
| `link broken` | a manifest reference (Cargo path, npm `file:`, go `replace`, Python path) names a directory that does not exist |

Alerts of the same kind raised together read as one line ("b, c missing since 14:02 (2 projects)"), and a
project linked only through others names the chain ("a to b to c: c missing"). A server that is simply not
installed is not an alert. A project removed from the registry never alerts. When a missing or
unreachable project returns and the alert clears, it is indexed again.

Alerts are mirrored into the store, so they show in `rtok doctor` (a `graph alerts` section and
`graph_alerts` in `--json`), in `rtok graph projects --json` (`alerts` per project) and as a `notice:`
line at the head of a graph tool answer whose scope contains the project ("notice: b missing since 14:02
(directory does not exist); results exclude b").

## Health score

Every project has a score from 0 to 100, built from what the health check and the capability record
already know (nothing probes for it). Three components, each 0 to 1, weigh 40, 30 and 30:

| Component | Weight | 1 when | Lower when |
| --- | --- | --- | --- |
| freshness | 40 | no file is pending | pending files: 0 at a fifth of the files, or when the index is more than 24 h old with changes since |
| backend | 30 | the configured backend works (`tags` and `text` always do) | under `auto` and `lsp` the server does not answer: 0.6 on the tree-sitter fallback, 0.3 on the text one, 0 when nothing answers |
| links | 30 | every link leads to a present, reachable, indexed project (no links also scores 1) | a link target is missing, unreachable or not indexed, or a manifest reference is broken |

A language with no language server is not penalised. Each reason that lowered a component comes with
its fix ("rust-analyzer not on PATH, using tree-sitter": "install the server; it is picked up within one
health-check interval, or restart"). A missing project scores 0 ("missing"); a project on its first index
has no score yet (`level: "indexing"`).

`rtok graph projects` has `health` (score) and `scope` columns, and `--json` carries `health` (`score`,
`level` `good` 80 and up, `warn` 50 to 79, `bad`, `indexing` or `missing`, `components`, `reasons` with
their `fix`) and `scope_health`, the lowest score in the project's scope: the weakest link decides, not an
average. A scope under 80 puts one `notice: graph health ...` line at the head of a graph tool answer
(a project an alert line already names is left out; `alerts = false` silences both), and `rtok doctor`
lists every project under 80 in a `graph health` section (`graph_health` in `--json`). Scores are
recomputed at most once a second per project.

## Without the server

`backend = "lsp"` does not fail when the server cannot answer. When the binary
is missing, no marker file is found above the queried file, the server is not
ready or has died, or it answers "nothing" for a name the tags index knows, the
tool gives the tags answer headed `(tags; lsp: <reason>)` — for example
`(tags; lsp: rust-analyzer not on PATH)`, or
`(tags; lsp: no Cargo.toml / compile_commands.json / tsconfig.json / pubspec.yaml in <root>)` —
so you can see which backend spoke and why. Each fallback records an
`lsp_fallback` measurement row (its `before_bytes` is the time lost). To go back
to the built-in index, set `backend = "tags"` again (the default); the tags
answers are byte-identical with the flag off, per the `graph_lsp_gate` contract
test. The default stays `tags`: a default change needs a new gate with recorded
`lsp.*` latency rows.
