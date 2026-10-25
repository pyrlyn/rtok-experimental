# Plugin contract

![The plugin contract: compile-time features and config select plugins into a Registry; each plugin implements the Plugin trait from rtok-plugin-sdk and reaches the store only through Ctx and the capability traits the host Runtime implements](plugin-contract.svg)

Source: [`plugin-contract.d2`](plugin-contract.d2). Index: [all diagrams](README.md).

Every token-reduction method in rtok is a plugin behind one trait. The trait and every type a
plugin names live in `crates/rtok-plugin-sdk` (D25), so a plugin, in-tree or not, depends on
the contract and never on the rtok binary. The host side, `Runtime` in `src/plugin.rs`,
implements the capability traits the contract asks for.

## Walkthrough

1. **Selection at compile time.** Each plugin id is a Cargo feature (`default = all`), and
   `plugins::all()` pushes each one under `#[cfg(feature = "<id>")]` in dispatch order.
2. **Selection at run time.** `Registry::new(&config)` pairs every compiled plugin with
   `config.plugin_enabled(id, default_on)`: `[plugins.<id>] enabled` if set, else the
   manifest's `default_on`. `config::CATALOGUE` lists the same `(id, default_on)` pairs and a
   test fails when they disagree, so a plugin cannot be half-registered.
3. **The trait.** `manifest()` and `dashboard_page()` are required. Every event method has a
   no-op default, so a plugin writes only what its `Manifest::surfaces` declares:
   `pre_tool` may `Deny` or `Rewrite` a call; `post_tool` returns extra context and nothing
   else; `session_start`, `prompt_submit` and `subagent_start` return an `Injection` (text +
   priority) for `inject` to budget; `pre_compact` runs before compaction; `mcp_tools`
   declares MCP tools; `proxy_filter` edits a `WireRequest` and returns `Measurement`s.
4. **Events are borrowed views** over the parsed hook input (`PreToolUse<'a>`, …), so the hook
   path copies nothing.
5. **`Ctx` is the plugin's whole world.** It carries the agent id and typed plugin config
   (`plugin_config::<T>(id)`), and derefs to `&dyn Capabilities`: `Host` (session, cwd,
   `estimate`, `record`, `log`, config), `Archive` (store originals for `expand`), `Notes`
   (memory), `ReadCache`, `Ledger` and `Symbols` (the graph index).
6. **A saving is a `Measurement`.** `Host::record` is the only way to claim one: plugin, kind,
   bytes and estimated tokens before and after, the `ref_id` that `expand` resolves, and the
   owning `call_id`.
7. **Two implementations of the host.** `Runtime` implements every capability over
   `rtok-store`; `testing::MemoryHost` implements them in memory for plugin unit tests.
8. **Out of tree.** A crate that embeds rtok passes its own plugins to
   `Registry::from_plugins`; with the `wasm-host` feature, `.wasm` guests are appended to the
   registry at run time.

## Where it lives

| Box | Code |
| --- | --- |
| Trait, events, `Manifest`, `Measurement`, `Injection` | `crates/rtok-plugin-sdk/src/lib.rs` |
| `Ctx` and capability traits | `crates/rtok-plugin-sdk/src/host.rs` |
| `WireRequest` for `proxy_filter` | `crates/rtok-plugin-sdk/src/wire.rs` |
| In-memory host | `crates/rtok-plugin-sdk/src/testing.rs` (`MemoryHost`) |
| Host runtime | `src/plugin.rs` (`Runtime`, `impl Host`, `impl Archive`, …) |
| Registry | `src/plugins/mod.rs` (`all`, `Registry`) |
| Catalogue | `crates/rtok-config/` (`CATALOGUE`) |
| Authoring guide | `docs/plugin-authoring.md`, `examples/hello_plugin.rs` |

## Invariants

- A plugin's panic is caught by the surface and drops only that plugin's output.
- PostToolUse returns context only; the type has no way to replace a tool result.
- No plugin depends on another: `cargo build --no-default-features --features measure` builds.
- No `Measurement` row means no saving claim, in code, docs or reports.

## Related

[Overview](overview.md) · [Hook flow](hook-flow.md) · [Store](store.md) ·
[`docs/plugin-authoring.md`](../plugin-authoring.md) · [wasm](plugins/wasm.md)
