# Plugins

Every token-reduction method in rtok is a plugin behind one trait. Plugins are in-tree
modules behind Cargo features — no daemon, no subprocesses. Out-of-tree `.wasm` plugins load
only in a build with the `wasm-host` feature (not in the default set) and with
`[plugins.wasm] enabled = true`.

The **replaces** column is a specification target, never a dependency: rtok reimplements the
behaviour from scratch and never runs, links, or reads the tool named.

| Plugin | Replaces (spec only) | Surface |
|--------|----------------------|---------|
| [`measure`](../src/plugins/measure/README.md) | rtk gain, headroom savings, lean-ctx gain | `stats`, `bench`, proxy |
| [`cmd`](../src/plugins/cmd/README.md) | rtk hook, ctx_shell, bash_compress | PreToolUse(Bash) → `rtok run` |
| [`read`](../src/plugins/read/README.md) | lean-ctx read/search/tree, read_cache | MCP `read` |
| [`json_tree`](../src/plugins/json_tree/README.md) | — | proxy, MCP |
| [`archive`](../src/plugins/archive/README.md) | CCR | proxy, MCP `expand`, `rtok archive rewrite` |
| [`proxy`](../src/plugins/proxy/README.md) | caveman-proxy | `ANTHROPIC_BASE_URL`, `OPENAI_BASE_URL` |
| [`inject`](../src/plugins/inject/README.md) | caveman/ponytail, lean-ctx | SessionStart, UserPromptSubmit |
| [`guard`](../src/plugins/guard/README.md) | — | PreToolUse, `rtok guard check` |
| [`memory`](../src/plugins/memory/README.md) | claude-mem | MCP, PreCompact |
| [`graph`](../src/plugins/graph/README.md) | codebase-memory-mcp | MCP |
| [`toon`](../src/plugins/toon/README.md) | — | proxy, MCP `expand` |

## Turning plugins off

At runtime, in `~/.rtok/config.toml`:

```toml
[plugins.cmd]
enabled = false
```

Or at build time, so the code is not compiled in at all:

```bash
cargo build --no-default-features --features cmd,read
```

## Each plugin directory

| File | Holds |
|------|-------|
| `README.md` | what the plugin does and why — linked from the table above |
| `AGENTS.md` | invariants and task rules for whoever works on it |
| `PLAN.md` | the plugin's own build plan |

Writing your own: [plugin authoring](plugin-authoring.md).

## Graph export format

`rtok graph export` and the MCP tool `graph_export` write one JSON document, `"schema": "rtok.graph.v1"`,
described by [`docs/schemas/rtok.graph.v1.schema.json`](schemas/rtok.graph.v1.schema.json) (generated from the
Rust types and checked by a test). Top-level keys:

| Key | Holds |
|-----|-------|
| `projects` | `id`, `name`, `root`, `origin`, `backend` (`tags`, `lsp` or `text`), `health` (`ok`, `stale`, `not indexed`, `missing`), `indexed_at` |
| `links` | `from`, `to`, `kind` (`manual` or `auto`), `reason`, `references` (call references from `from` into `to`) |
| `nodes` | `id`, `project`, `kind`, `name`, `path` (relative to the project), `line`; empty at the `overview` level |
| `edges` | `from`, `to` (node ids), `kind` |
| `meta` | `scope`, `level` (`overview`, `symbols` or `focus`), `focus`, `depth`, `exported_at`, `rtok_version`, `redacted`, `partial`, `notes` |

Absolute paths, the home directory and the user name are redacted unless `--no-redact` is given (`graph_export` always
redacts); source text is never included, file and symbol names are. A call whose name has more than eight definitions
draws no edge. `--from FILE` shows a saved export without reading or writing the
registry or the index.

### Pictures

`rtok graph export --format svg` draws the same export as a picture, and `--format png` rasterises that picture
(`-o FILE` is required, `--scale 1` to `4` is the multiple of the SVG size). The picture is a function of the export
alone, so `--from FILE` draws a saved export exactly as the live scope. Projects are panels, symbols are dots on a
spiral (the best connected in the middle; circle: function, square: type, diamond: module), a solid arrow is a call
within a project, a dashed one a call across projects, and a thick line a project link. At most 200 nodes are drawn;
the footer says how many are hidden, and the JSON always has all of them. The footer also names each project with its
backend and `indexed_at`, the scope, the rtok version, the export time and, when a project could not answer, a
PARTIAL marker. A PNG needs a font installed on the machine; without one the command says so instead of drawing no
text. MCP `graph_export` stays JSON.

### On the Graph page

The Graph page has an Export button and a file picker, "open an export". The button opens a panel that says the
file contains file names and symbol names (never source text) and that the home directory, the user name and
absolute paths are replaced, then offers the overview, the open project's symbol graph and, with a function in focus,
the subgraph around it, as JSON, SVG or PNG. The server writes the file with the function `rtok graph export` calls,
so the JSON is the CLI's, byte for byte but for `exported_at`; the page only saves it. An image of the live frame is
not part of this menu. The picker sends a saved JSON export as text to the server, which checks it like
`export::read` and returns it: the page shows it read-only under "viewing export from FILE" and writes nothing to
the registry or the index. `rtok tui` gets the same actions in T329.41.
