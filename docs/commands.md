# Commands

Every command below is a surface onto the same plugin registry and the same SQLite store,
and every one of them works today. A subcommand whose plugin was compiled out (for example
`rtok run` without the `cmd` feature) prints `not implemented` and exits 0, so a stripped
build never blocks the host.

| Command | Does |
|---------|------|
| `rtok plugins` | list plugins: id, enabled, surfaces |
| `rtok config show\|init\|validate\|get\|set\|path` | one config file; `show --sources` says where each value came from |
| `rtok hook <event>` | Claude Code hook entry point (stdin JSON → stdout JSON) |
| `rtok mcp` | MCP server over stdio: `read`, `search`, `tree`, `expand`, `mem_*`, graph tools |
| `rtok proxy` | `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` hop: usage capture, optional compress mode |
| `rtok batch` | provider Batch jobs (`submit`, `status`, `fetch`) sent through `rtok proxy`; see [batch-flex.md](batch-flex.md) |
| `rtok dashboard` | local React UI over a WebSocket API, reading the same store |
| `rtok run -- <cmd>` | run a command, archive the raw output, print the filtered version |
| `rtok filter --stdin` | filter a payload without executing it (OpenCode `tool.execute.after`) |
| `rtok expand <id>` | print an archived payload, whole or by `--lines` / `--grep` |
| `rtok stats` | measurements from session logs and the proxy |
| `rtok doctor` | inspect hooks, MCP servers, proxy chain — including what each costs per turn |
| `rtok agents install claude\|cursor\|codex\|opencode\|kilo\|pi\|zcode\|kimi\|copilot\|aider\|windsurf\|zed` | install into a host, with backups; `--dry-run` |
| `rtok agents uninstall claude\|cursor\|codex\|opencode\|kilo\|pi\|zcode\|kimi\|copilot\|aider\|windsurf\|zed` | take rtok back out of a host, with backups; `--dry-run` |
| `rtok agents list` | every known app: kind and name, path and version, config files, rtok modules, installed plugin version |
| `rtok agents junk list\|clear` | each agent's folders, junk kinds and sizes; `clear` is a dry run until `--yes`, clears only rtok's own logs and archives unless `--agent`, `--kind`, `--include review` or `--older-than` selects more, checks each item again right before removing it, `--include review` also removes the merged, clean, idle worktrees `rtok worktree gc` would remove (`stale-worktrees`; any other worktree is listed with gc's reason and kept), `--trash` moves to the OS trash, exits 1 when a planned item stays |
| `rtok graph index [path]` | build the tree-sitter symbol index for a tree |
| `rtok graph impact <name> [--all] [--project <id\|dir>]` | what breaks if a symbol changes (files grouped and cut at `plugins.graph.impact_tokens`; `--all` prints every row), over the project and the projects it links to; `dead` and `affected` run over that scope too (a symbol only a linked project calls is not dead; `git diff` is read in every project, tests are listed per project); `index` and `status` take `--project` too. One answer, one cap; with `watch` on, `rtok mcp` watches every project of the scope |
| `rtok graph review [--since <ref>\|--staged] [--json] [--project <id\|dir>]` | risk-ranked reading list for a git diff: each changed file's score, the definitions no test reaches, and the line ranges to read |
| `rtok graph projects` | list the registered projects with their index status (`add`, `select`, `remove`, `link` and `unlink` change the registry) |
| `rtok memory import <file>` | import notes as JSONL, deduped by body hash |
| `rtok otel flush\|status` | export the ledgers over OTLP/HTTP, or report the watermarks |
| `rtok task create\|list\|show\|status\|next\|ready\|claim\|release\|dep\|priority\|sync\|init` | the project's plan in its `[tasks]` adapter; `claim` takes a ready task, `dep` records a blocker, `sync` raises the id counters and reports drift |
| `rtok bench` | A/B two host configurations on fixed tasks |

## The graph page

The Graph page of `rtok dashboard` draws the code graph in two levels, as a 3D scene, a flat 2D
scene, or a plain list (the choice is remembered; without WebGL the page shows 2D and says why).

- **All projects.** One node per registered project, a line per link. Right-click a node for
  Select, Open, Fly to and Copy path; double-click opens the project.
- **One project.** Files are cubes, types and modules octahedra, functions spheres, each in its
  project's colour, grouped by directory. Click a node to select it; click the selected file to
  expand its definitions, or the selected function to focus it (its callers and callees, one to
  four calls deep, set with the depth buttons). A call into a linked project ends at an outlined
  node in that project's colour, and clicking it opens that project with the symbol focused.
  `+N more` raises the node cap by 500. A `⚠` marks a file changed since the last index run, and a
  banner says the picture is partial while any file is.
- **Where you are is in the address.** The project, the expanded files, the focus and the depth
  are in the page's URL, so every step is a history entry, the breadcrumb (`All projects / rtok /
  src/plugins/graph`) climbs back, and a copied link opens the same view.
- **Index states.** A project that has not been indexed shows the command to run
  (`rtok graph index --project <id>`) and fills in once it is; a project whose directory is gone
  is drawn hollow and cannot be opened. While `watch` re-indexes, the page asks again and
  updates in place without moving the scene.
- **Live graph.** Beside the explorer (stacked below it under 900 px) the same level is drawn
  read-only and lit by the graph calls arriving on `/ws`: a ring in one of eight accents per
  running call (more than eight merge into a "busy" count), a heat halo on the nodes queried in
  the last 5 minutes, a red ring for a failed call. The overview lights the call's project; the
  drill-down lights the node named like its target. The canvas takes no pointer or keyboard input.
  Drag the bar between the parts (arrow keys also work, double-click resets to 50/50) or press
  "Hide live graph"; the choice is remembered, and a hidden live graph does not subscribe.
  Until the first call it shows "Waiting for graph calls".
- **Live numbers.** Running calls, calls and failures, tokens sent, tokens without rtok and saved,
  per-tool bars and backend shares, over the last 1, 5 or 15 minutes or since the page opened.
  Totals come from each frame's `summary` and the `graph` measurement rows, so a window equals
  `rtok stats` over the same calls (the per-tool bars and backend shares cover the calls a frame
  lists). "Freeze" holds the picture and counts what arrives meanwhile; "Unfreeze" catches up
  with every call. The feed keeps the newest 200 finished calls, can be filtered by caller (the
  session id), tool and project, shows failed calls in red and marks a call that never ended
  after two minutes as interrupted.

## Every flag is a config key

There is no flag that cannot be made permanent. `rtok proxy --port 8791` and
`[proxy] port = 8791` are the same setting reached two ways, and `rtok config show --sources`
reports which layer won. See [Configuration](config.md) for the precedence
chain.

## Shortening and getting it back

`rtok run -- <cmd>` executes the command, archives the raw output, and prints a filtered
version with an id attached. `rtok expand <id>` prints the original payload back. That pair
is the whole lossless contract — nothing rtok shortens is unrecoverable.

## Measurement

`rtok stats` reads the `measurements` table. Rows come from plugins calling `Ctx::record`
with a `Measurement`, and from the proxy capturing real `usage` from API responses.
`rtok stats --calibrate` refits the characters-per-token estimator against those real
counts.

`rtok bench` runs two host configurations over the fixed task set in `bench/tasks.toml` and
compares them.

## Graph call events

`rtok dashboard` streams what the graph tools are doing. Every `symbol`, `callers`, `impact`,
`outline` and `explore` call, in any rtok process (an MCP server, `rtok mcp --call`), appends a
start, a progress and an end event to the store. The dashboard reads new events every 250 ms and
pushes them on `/ws` as `{"type":"calls"}` frames, so a call made by another process shows up
within a second (`tests/web_live.rs` pins both). A page asks for the frames with
`{"calls":{"subscribe":true}}` and stops them with `false`; nothing is read while no socket is
subscribed.

An end event names the tool, symbol, project and backend (`lsp`, `tags` or `text`, read from the
answer's header and empty for a fixed mode), the elapsed time, the answer's estimated tokens and
the `graph` measurement rows the call wrote (`samples`: kind, bytes and estimated tokens before
and after). Those are the rows `rtok stats --plugin graph` lists. A failed call ends with
`ok: false` and its error. Events carry names, paths and numbers, never source text.

A frame holds at most 100 events. A burst folds a finished call's start and progress into its end,
and the frame's `summary` counts every call of the poll, so totals do not drop events. Nothing is
replayed: a socket sees the events written after it subscribed (the first frame is empty and
carries the newest event id), and a window total is read from the store. The store keeps the
newest 5000 events.
