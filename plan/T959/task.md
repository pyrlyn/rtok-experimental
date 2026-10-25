---
status: in progress
priority: P2
complexity: 3
readiness: 70%
agent: Claude Code / claude-opus-5-5
---

# T959. Architecture diagrams in D2, one per part, each with its doc

Creator request. `architecture.md` describes the shape in prose and two ASCII sketches; nothing shows how a hook, an MCP call or a proxied request actually moves through the code, and no plugin has a picture. Each part of rtok gets a [D2](https://d2lang.com) diagram and a doc page that walks through it, so a reader (human or agent) sees the mechanism before opening the code.

Done:

- `docs/architecture/` holds one `<part>.d2` source, its rendered `<part>.svg` and a `<part>.md` page per part; `docs/architecture/README.md` is the index and `architecture.md` links it.
- Parts: the overview (surfaces → host runtime → plugins → store), the layer DAG, the workspace crates, the hook, MCP and proxy request flows, the plugin contract and registry, the store and measurement ledger, and one diagram per plugin (archive, checkpoint, cmd, compress, docs, graph, guard, inject, json_tree, measure, memory, proxy, read, toon, wasm).
- SVGs are rendered in-process by the creator's [`d2-render`](https://crates.io/crates/d2-render) crate (native backend, no `d2` binary). `tests/diagrams.rs` fails when a `.svg` is missing or older than its `.d2` (source-hash stamp); `just diagrams` re-renders the stale ones and fails on any construct the native backend would not draw.
- Each `.md` has its `docs/ru/` and `docs/uk/` translation (CONTRIBUTING). The landing sync mirrors only `*.md`, so `architecture/` is excluded there until it can carry images.
- A gap in `d2-render` that a diagram needs goes to pyrlyn/crates as its own PR, not worked around here.

Plan:

1. `Cargo.toml` dev-dependency `d2-render`; `tests/diagrams.rs` (verify, or refresh under `RTOK_DIAGRAMS=overwrite`); `just diagrams`; add the test to `docs-check`; `toolchain.md` row.
2. Overview, layers, crates, hook/MCP/proxy flows, plugin contract, store: read the code, write `.d2` + `.md`.
3. One diagram + page per plugin, from `src/plugins/<id>/` (README, AGENTS, mod.rs).
4. `docs/ru/architecture/`, `docs/uk/architecture/`; link from `architecture.md`; exclude from landing sync.
5. `just diagrams`, `just docs-check`, `just check`; read every SVG once.

Check: `cargo test -p rtok --test diagrams` passes with every `docs/architecture/*.d2` fresh, and `docs/{ru,uk}/architecture/` mirror every page.
