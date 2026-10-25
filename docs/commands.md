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
| `rtok discover` | shell commands your agents ran that have no rule or formatter, most output first |
| `rtok doctor` | inspect hooks, MCP servers, proxy chain — including what each costs per turn, and how much space `rtok agents junk clear` would free |
| `rtok doctor --project [PATH]` | add setup advice for one checkout: language servers with install commands, graph index, instruction files, plugins left off, with savings only from measurements ([lsp.md](lsp.md)) |
| `rtok agents install claude\|cursor\|codex\|opencode\|kilo\|pi\|zcode\|kimi\|copilot\|aider\|windsurf\|zed` | install into a host, with backups; `--dry-run` |
| `rtok agents uninstall claude\|cursor\|codex\|opencode\|kilo\|pi\|zcode\|kimi\|copilot\|aider\|windsurf\|zed` | take rtok back out of a host, with backups; `--dry-run` |
| `rtok agents list` | every known app: kind and name, path and version, config files, rtok modules, installed plugin version |
| `rtok agents junk list\|clear` | each agent's folders, junk kinds and sizes; `clear` is a dry run until `--yes`, clears only rtok's own logs and archives unless `--agent`, `--kind`, `--include review` or `--older-than` selects more, checks each item again right before removing it, `--include review` also removes the merged, clean, idle worktrees `rtok worktree gc` would remove (`stale-worktrees`; any other worktree is listed with gc's reason and kept), `--items`, `--sort` and `--min-size` shape the per-item lines, `--trash` moves to the OS trash, exits 1 when a planned item stays |
| `rtok graph index [path]` | build the tree-sitter symbol index for a tree |
| `rtok graph impact <name> [--all] [--project <id\|dir>]` | what breaks if a symbol changes (files grouped and cut at `plugins.graph.impact_tokens`; `--all` prints every row), over the project and the projects it links to; `dead` and `affected` run over that scope too (a symbol only a linked project calls is not dead; `git diff` is read in every project, tests are listed per project); `index` and `status` take `--project` too. One answer, one cap; with `watch` on, `rtok mcp` watches every project of the scope |
| `rtok graph review [--since <ref>\|--staged] [--json] [--project <id\|dir>]` | risk-ranked reading list for a git diff: each changed file's score, the definitions no test reaches, and the line ranges to read; fixtures, benches, examples and Linguist-marked files are listed unscored; see [review.md](review.md) |
| `rtok graph diff [--from <ref>\|<project>:<ref>]... [--from-export <file>] [--to <ref>\|working] [--json] [--project <id\|dir>]` | what a change did to the graph, MCP `graph_diff`: symbols changed (signature or body), added, removed, renamed or moved, and call edges added or removed, each changed or removed symbol with its callers from the project and the projects it links to. The old side is read from git's object database (no checkout, the working tree is not touched); `--from` defaults to `HEAD`, `--to` to the working tree. A renamed symbol is shown only when exactly one removed and one added definition share its body; the text is cut at `plugins.graph.max_tokens` with an `expand <id>` for the rest, `--json` is whole. `--from <project>:<ref>` (a project name, id or directory; repeatable) compares that project from its own ref while the others use the plain `--from`. `--from-export <file>` compares the working tree with a saved `rtok graph export --level symbols` file instead of a revision: it lists added and removed symbols and the registry links added and removed (links cannot be compared with a git ref, the registry keeps no history), but not changes or edges, because an export keeps no signatures; MCP does not take a file path. Files that differ but no grammar reads (binary, a language without a grammar, unparseable) are listed as `changed, not analysed`; files the project excludes are not |
| `rtok graph export [--level overview\|symbols] [--focus <symbol> [--depth N]] [--no-redact] [--from <file>] [--format json\|svg\|png] [--scale 1-4] [--transparent] [-o <file>] [--project <id\|dir>]` | the graph as a file, MCP `graph_export` (JSON only): `rtok.graph.v1` JSON (projects, links, symbols, calls; schema in `docs/schemas/`, format in [plugins](plugins.md#graph-export-format)). `overview` is projects and links, `symbols` adds every symbol and call, `--focus` keeps the symbols within `--depth` calls of one. Home, user name and absolute paths are redacted unless `--no-redact`; `--from` shows a saved export and touches neither the registry nor the index. `--format svg` draws a picture of the same export (the 200 best connected nodes, a legend and a footer with the projects, scope, backend, `indexed_at`, version, export time and a partial marker; the footer says how many nodes are hidden); `--format png` rasterises it at `--scale` times its size (1 to 4, 1 by default) and needs `-o`; `--transparent` drops the background. A PNG needs a font on the machine. In `rtok tui`, `e` on the Graph page writes the same file with the same function (format, level, redaction on unless switched off, the file you name), and `v` shows a saved JSON read-only |
| `rtok graph projects` | list the registered projects with their index status (`add`, `select`, `remove`, `link` and `unlink` change the registry; `reindex` re-reads a project's files into its index) |
| `rtok memory import <file>` | import notes as JSONL, deduped by body hash |
| `rtok otel flush\|status` | export the ledgers over OTLP/HTTP, or report the watermarks |
| `rtok task create\|list\|show\|status\|next\|ready\|claim\|release\|dep\|priority\|sync\|init` | the project's plan in its `[tasks]` adapter; `claim` takes a ready task, `dep` records a blocker, `sync` raises the id counters and reports drift |
| `rtok task export github\|linear [--dry-run] [--json]` | mirror the disk tasks one way into GitHub Issues or Linear; a second run with nothing changed sends no write |
| `rtok bench` | A/B two host configurations on fixed tasks |

## Data permissions

On Unix, `rtok doctor` checks permissions under rtok's dedicated data home (`~/.rtok`, or
`RTOK_HOME`). `rtok doctor --fix --only permissions` previews repairs; add `--yes` to apply them,
or add `--dry-run` with `--yes` to print the changes without writing. The repair sets owned
directories to `0700` and regular files to `0600`. It refuses symlinks, multiply-linked files and
paths outside the dedicated home. Custom external data directories are reported for manual
review; Windows ACLs are not checked.

## The graph page

The Graph page of `rtok dashboard` draws the code graph in two levels, as a 3D scene, a flat 2D
scene, or a plain list (the choice is remembered; without WebGL the page shows 2D and says why).

- **All projects.** One node per registered project, a line per link. Right-click a node for
  Select, Open, Re-index, Remove…, Fly to and Copy path; double-click opens the project. Re-index
  runs `rtok graph projects reindex` on a worker: the node reads `indexing` meanwhile and the old
  rows keep answering, then `ok`, or `failed` with the error. Remove… shows the dry run of
  `rtok graph projects remove` (index rows and links that go, files untouched) and removes only
  after you confirm. `rtok tui` has the same two actions on the Graph page.
- **One project.** Files are cubes, types and modules octahedra, functions spheres, each in its
  project's colour, grouped by directory. Click a node to select it; click the selected file to
  expand its definitions, or the selected function to focus it (its callers and callees, one to
  four calls deep, set with the depth buttons). A call into a linked project ends at an outlined
  node in that project's colour, and clicking it opens that project with the symbol focused.
  `+N more` raises the node cap by 500. A `⚠` marks a file changed since the last index run, and a
  banner says the picture is partial while any file is.
- **Side panel and search.** The selected node's panel gives its path and line, its signature, its
  callers and callees (the other ends of the `calls` edges in the picture; click one to select
  it) and an Open in editor link, `vscode://file/<root>/<path>:<line>`. For a node of a linked
  project the link uses that project's root, and Open in `<project>` steps into it. The search box
  sends its text when you press Enter (typing alone sends nothing) and lists the matches in the
  project and its linked projects, each with its project; a match in this project is focused, a
  match in another opens that project with the symbol focused.
- **Where you are is in the address.** The project, the expanded files, the focus and the depth
  are in the page's URL, so every step is a history entry, the breadcrumb (`All projects / rtok /
  src/plugins/graph`) climbs back, and a copied link opens the same view.
- **Index states.** A project that has not been indexed shows the command to run
  (`rtok graph index --project <id>`) and fills in once it is; a project whose directory is gone
  is drawn hollow and cannot be opened. While `watch` re-indexes, the page asks again and
  updates in place without moving the scene.
- **Compare.** The Compare button in the one-project toolbar asks the server for the same report
  as `rtok graph diff` (nothing is recomputed in the page) and colours the picture: added green
  `+`, removed red `−` (drawn as an outlined ghost in an expanded file, since the working tree no
  longer has it), changed amber `~`, moved blue `→`; a file takes the one change of its symbols,
  several kinds make it changed, and an added or removed call is a green or a dashed red line.
  Every change also has its mark in the label and its word in the tooltip and the list, so colour
  is never the only cue. The side panel gives the counts, the changed (signature or body) and
  removed symbols with their callers, renamed, moved and added symbols, call edges, links
  added and removed, and `changed, not analysed` files. The old side is `HEAD`, a ref or
  `PROJECT:REF` typed in the box (sent on Enter), or a saved `rtok graph export --level symbols`
  chosen with the file picker: the page reads the file itself and sends its text, never a path, so
  nothing on `/ws` can make rtok open a file. Against an export only added and removed symbols and
  links are listed. Each list is cut at 500 rows per project (the panel says how many more;
  `rtok graph diff` has them all). The live graph keeps running; turning Compare off puts the
  picture back.
- **Live graph.** Beside the explorer (stacked under 900 px) a second, read-only picture shows what
  the graph tools are doing, from the calls arriving on `/ws`. It draws the level the explorer
  shows with the same layout and colours: the registered projects, or the drilled project. It
  takes no input (no pointer, wheel or keys, and the cursor stays the default arrow). A call lights
  the project node on the overview, or the node named like its target in the one-project view: a
  ring per running call in one of eight accents, a halo for calls over the last 5 minutes and a
  red ring on a failed call; beyond eight running calls a "busy" count says how many more. Until
  the first call the picture is dimmed and reads "Waiting for graph calls". The camera frames the
  nodes of the running calls, and of calls that ended in the last 4 seconds, and eases back to the
  whole picture when none is left (it jumps under `prefers-reduced-motion`). The "2D" and "3D"
  buttons in the corner of the picture switch it; 3D is the same Three.js view as part 1 without
  orbit or picking, with a halo for heat, a shell per running call and a red wireframe for a
  failure, and falls back to 2D with a notice when WebGL is not available. Each running call carries a small
  label on its node, in 2D and in 3D, with its tool and a seconds counter ticking up; a call that
  ended keeps its label with its duration for 4 seconds and fades after 2, a call folded into "+N
  more" labels the node that holds the count, and a call outside the scope has none. A bar between the two
  parts resizes them (drag, arrow keys, double-click for 50/50) and is remembered; "Hide live
  graph" hides the picture and ends the `/ws` subscription, and is remembered too. The page
  subscribes only while the live graph is shown. Below it: running calls, calls and failures, tokens sent, tokens without rtok and saved,
  per-tool bars and backend shares, over the last 1, 5 or 15 minutes or since `rtok web` started: the totals are read from the store, so a page reload shows the same
  figures (the store keeps the newest 5000 call events, so a call older than that is gone from them).
  Under 900 px, where the live part stacks below the explorer, these numbers fold into one line (running,
  calls, saved) that opens the full cards and shuts them again; from 900 px up nothing changes.
  Totals come from each frame's `summary` and the `graph` measurement rows, so a window equals
  `rtok stats` over the same calls (the per-tool bars and backend shares cover the calls a frame
  lists). The page also counts symbols asked (and how many calls crossed more than one project),
  `lsp_fallback` rows and answers cut at `max_tokens`, exactly, from `summary`, and for `symbol`,
  `callers`, `impact`, `outline`, `explore` and `graph_diff` how many asked symbols the answers list
  (`explore` asks one per identifier of its question; the others ask none), the files of the rows
  they list (the caller sites quoted under a changed symbol in `graph_diff` are not rows of the
  diff) and the projects with a hit, as the graph backends counted them while building the answer; latency (median
  and 95th percentile) is measured on the calls frames list, the newest 1000. "Freeze" holds the picture and counts what arrives meanwhile; "Unfreeze" catches up
  with every call. The feed keeps the newest `live_feed_rows` (200) finished calls, can be filtered by caller (the
  agent and host the store names for the session, such as `claude 3f9a1c2e`; an `rtok mcp` process shows
  its host and process id, and a session the store does not know shows its id), tool and project, shows failed calls in red and marks a call that never ended
  after two minutes as interrupted. `[plugins.graph]` `live_heat_window_s` (300), `live_max_events_per_s` (50;
  call events a second the panel lists, the counters still count every call) and `live_feed_rows` (200)
  are read when the live part starts.
  The "calls" card and the "saved" card each carry a sparkline of the server's 30 slots over the
  chosen window (the last 15 minutes for "since start"), from the same buckets the totals add up,
  named "calls over the last 5 min" and "tokens saved over the last 5 min". When the page has a
  scope (the drilled project, else the selected one, with everything its links reach), a feed row
  for a project outside it carries an "outside scope" mark and does not light the canvas. In the
  one-project view, when the 500-node cap folded nodes away, a call on one of them counts under
  "folded calls" on the drawn file whose path holds its target, else on the "+N more" group.
- **Alerts.** When the health check (see [LSP backend](lsp.md#health-check-and-alerts)) raises an alert for a
  project, its node carries a red badge in both levels (a red marker in 3D, and the word `alert` in
  the list and the tooltip), and so does every link into it and every outlined symbol of that
  project in the one-project view. An alerts list at the top of the page gives one line per kind
  ("share-1, share-2, share-3 unreachable since 14:02 (3 projects)"). A toast appears when a
  snapshot raises an alert or clears one, and goes away after a few seconds or on Dismiss; alerts
  that were already up when the page opened are listed but not toasted.
- **Graph backend.** Each project in the list carries `lsp` or `tags`, and the selected project's
  header says why: the language whose server answers, or the reason the project fell back to tags,
  when it was last checked and when the health check retries. A project nobody has asked yet says
  it has no record.

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

`rtok discover [--since 30d] [--limit 20] [--json]` reads the `PostToolUse` shell calls the hook
already stored in the database and lists the command families that have no rule and no
formatter (the default cut still applies), ranked by the output tokens the agent read. It
reruns nothing and claims no saving: only a `Measurement` row is a saving. Calls whose body
exceeded `core.call_io_inline_bytes` were never stored and show up as `unreadable rows`.
Pick the next rule family from the top rows.

`rtok bench` runs two host configurations over the fixed task set in `bench/tasks.toml` and
compares them. How to run every suite and read the results: [bench/README.md](../bench/README.md).

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
