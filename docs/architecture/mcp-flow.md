# MCP flow

![rtok mcp: stdio, HTTP and wrapper transports feed one Server that lists built-in, coordination and plugin tools, gates each tools/call and records it](mcp-flow.svg)

Source: [`mcp-flow.d2`](mcp-flow.d2). Index: [all diagrams](README.md).

`rtok mcp` is the surface that can *replace* a host tool instead of decorating it: an agent
that calls rtok's `read` or `search` gets the shortened answer directly, which a
PostToolUse hook can never give (D2). The same `Server` answers over stdio and over HTTP, and
a third mode wraps someone else's MCP server and shortens its long results losslessly.

## Walkthrough

1. **Start.** `mcp::run` builds a `Server` (one `Runtime` for the process), starts the store
   retention task and the OTLP ticker, and spawns background threads: the `graph` file
   watcher (unless `plugins.graph.watch = "off"`), the graph health check, the worktree
   reaper (when `worktree.enabled`) and a refresh of followed graph projects.
2. **List tools once.** `Server::new` collects the built-ins (`expand`, `ping`, `whoami`),
   the coordination tools (`worktree_*`, `agent_send`/`agent_inbox`, `agents_list`,
   `agent_show`, `agent_status_set`, `task_*`) and every enabled in-tree plugin's
   `Plugin::mcp_tools()`. Out-of-tree WASM plugins are not listed, because `invoke` has no
   arm for them.
3. **Read a line.** The stdio loop reads one JSON-RPC line (a batch array is answered with an
   array) and `handle_line` routes by method: `initialize` negotiates the protocol version and
   notes whether the client offers `roots`; notifications get no answer; `tools/list` returns
   the listed tools.
4. **Gate a call.** `call_tool` first normalises argument aliases (`prepare_args`), then
   refuses a tool that is not listed and a call missing a field the tool's own `input_schema`
   requires. Both answer `CallToolResult::error`, so the agent sees why.
5. **Invoke.** `invoke` routes the name to its owner: `read`/`search`/`tree` to the `read`
   plugin, `mem_*` and `handoff` to `memory`, `symbol`/`callers`/`impact`/`outline`/
   `explore`/`graph_diff`/`graph_export` to `graph` (scoped by `graph::scope::resolve`),
   `docs_*` to `docs`, `expand` to the archive. A plugin that shortens its answer keeps the
   original behind an `expand` id; `expand` itself caps a large original with
   `cap_result`, which leaves an `expand(<id>)` marker to page on.
6. **Record.** Every call, failed or not, goes through `record`: one `calls` row
   (`kind = mcp_call`, plugin and tool name), its `call_io`, and three `tokens` rows (the
   arguments as `before`, the result as `after` and as the plugin's `mcp` row).
7. **HTTP.** `rtok mcp --http` serves the same `Server` over MCP Streamable HTTP; rmcp owns the
   wire, including the Host and Origin checks, and hands only `tools/list` and `tools/call` to
   the same gate, dispatch and `record`.
8. **Wrap.** `rtok mcp -- <server argv>` starts a foreign stdio MCP server and forwards every
   frame untouched, except a `tools/call` response whose text blocks exceed the `[mcp]` cmd
   rule: each such block is archived raw, cut by `rules::apply` and closed with the same
   `expand` trailer `rtok run` prints. If the store cannot open, the wrapper is a plain pipe.

## Where it lives

| Box | Code |
| --- | --- |
| stdio server | `src/mcp.rs` (`run`, `Server`, `handle_line`, `call_tool`, `invoke`, `record`) |
| HTTP | `src/mcp/http.rs` |
| Wrapper | `src/mcp/wrap.rs` |
| Coordination tools | `src/mcp/worktrees.rs`, `src/mcp/messages.rs`, `src/mcp/agents.rs`, `src/mcp/tasks.rs` |
| Plugin tools | `src/plugins/read/`, `src/plugins/memory/`, `src/plugins/graph/`, `src/plugins/docs/` |
| Ledger | `crates/rtok-store/` (`insert_call`, `insert_call_io`, `insert_tokens`) |

## Invariants

- A tool that is not listed cannot be called, over stdio or HTTP.
- A result rtok cuts always carries an `expand` id; the original is in the archive.
- The wrapper never changes a frame it does not shorten, and never fails the foreign server.

## Related

[Overview](overview.md) · [Hook flow](hook-flow.md) · [Proxy flow](proxy-flow.md) ·
[read](plugins/read.md) · [memory](plugins/memory.md) · [graph](plugins/graph.md) ·
[archive](plugins/archive.md)
