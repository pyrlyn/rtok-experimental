# Architecture diagrams

Every part of rtok has a [D2](https://d2lang.com) diagram here and a page that walks through
it, naming the code each box stands for. Start with the overview, then follow a request
through the surface you care about, then open the plugin.

[`architecture.md`](../../architecture.md) keeps the principles and decisions in prose; these
pages show the mechanism.

## The system

| Diagram | Shows |
| --- | --- |
| [Overview](overview.md) | Agents, the three surfaces, the registry, the plugins, the store and the operator surfaces |
| [Layers](layers.md) | The module DAG that `tests/module_graph.rs` enforces |
| [Crates](crates.md) | The Cargo workspace: binaries, library and `crates/` |
| [Plugin contract](plugin-contract.md) | `Plugin`, `Ctx`, the capability traits, selection by feature and config |
| [Store](store.md) | The SQLite tables by domain, the archive on disk and who reads what |

## Request flows

| Diagram | Shows |
| --- | --- |
| [Hook flow](hook-flow.md) | `rtok hook <event>`: guard, adapt, dispatch by event, merge, fail open |
| [MCP flow](mcp-flow.md) | `rtok mcp`: stdio, HTTP and the wrapper; listing, gating, invoking and recording a tool call |
| [Proxy flow](proxy-flow.md) | `rtok proxy`: lanes, loop and capacity checks, shaping, forward and tee |

## Plugins

| Plugin | Diagram |
| --- | --- |
| archive | [archive](plugins/archive.md) |
| checkpoint | [checkpoint](plugins/checkpoint.md) |
| cmd | [cmd](plugins/cmd.md) |
| compress | [compress](plugins/compress.md) |
| docs | [docs](plugins/docs.md) |
| graph | [graph](plugins/graph.md) |
| guard | [guard](plugins/guard.md) |
| inject | [inject](plugins/inject.md) |
| json_tree | [json_tree](plugins/json_tree.md) |
| measure | [measure](plugins/measure.md) |
| memory | [memory](plugins/memory.md) |
| proxy | [proxy](plugins/proxy.md) |
| read | [read](plugins/read.md) |
| toon | [toon](plugins/toon.md) |
| wasm | [wasm](plugins/wasm.md) |

## Editing a diagram

The `.d2` file is the source; the `.svg` beside it is generated and committed. SVGs are
rendered in-process by [`d2-render`](https://crates.io/crates/d2-render)'s pure-Rust backend,
so no `d2` binary is needed.

1. Edit `<name>.d2` and its page `<name>.md`.
2. Run `just diagrams`. It re-renders every stale SVG and fails if the source uses a construct
   the native backend does not draw (`sql_table`, `sequence_diagram`, Markdown labels, `near`
   on nested shapes, imports, layers, …), so a committed picture never drops part of its
   source.
3. `cargo test -p rtok --test diagrams` (part of `just docs-check`) fails when an SVG is missing
   or older than its source, and when a diagram has no page.

Keep the shared palette at the top of each file: blue for surfaces, violet for the runtime,
green for plugin parts, amber for data, pink for operator output, grey for things outside
rtok. A new page also gets its `docs/ru/` and `docs/uk/` translation.
