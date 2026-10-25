# wasm plugin

![A feature-gated host reads .wasm files from the plugin directory, runs each manifest export under a fuel budget, rejects hook surfaces, and registers the rest as plugins whose MCP calls stage measurements for the host to record](wasm.svg)

Source: [`wasm.d2`](wasm.d2). Plugin docs: [`src/plugins/wasm.rs`](../../../src/plugins/wasm.rs). Index: [all diagrams](../README.md).

The WASM host lets a third party ship a plugin as a `.wasm` file instead of a Rust crate in
this repository. It is compiled only with the Cargo feature `wasm-host` (backed by `wasmi`)
and loads only when `[plugins.wasm] enabled = true`. It is not a catalogue plugin: it is not
listed in `plugins::all()` and has no manifest of its own.

## Walkthrough

1. **The registry asks for out-of-tree plugins.** `Registry::from_plugins` first builds the
   native list with `config.plugin_enabled`, then (feature `wasm-host`) calls
   `wasm::append`. It returns the list unchanged when `[plugins.wasm] enabled` is false or the
   directory cannot be read.
2. **Each `*.wasm` file is loaded.** `append` scans `[plugins.wasm] dir` (default
   `~/.rtok/plugins`, one level) and calls `WasmPlugin::load`. The engine is created with
   `consume_fuel(true)`; `Module::new` validates the module.
3. **The guest gets three imports.** The `Linker` exposes only `env.rtok_estimate`,
   `env.rtok_record_measurement` and `env.rtok_log`. A measurement whose `plugin` is not the
   guest's own id traps the call; a valid one is only staged, and the host writes it later.
4. **The manifest is read under a budget.** `store.set_fuel(FUEL)` runs before
   `instantiate_and_start` and again before the guest export `rtok_manifest`, which returns a
   packed pointer and length of a JSON `WasmManifest` (`id`, `surfaces`, `default_on`,
   `title`, `summary`, `saves_tokens`). Guest memory reads go through `guest_slice`, which
   rejects a negative or out-of-range pointer instead of panicking.
5. **Validation.** A manifest that lists the `hook` surface is rejected, and so is one with
   no surface among `mcp`, `proxy` and `cli`. Any load error is written to stderr and to the
   log (`wasm` / `load`), and only that plugin is skipped.
6. **Registration.** A valid plugin becomes a `WasmPlugin` (an `impl Plugin` holding a
   `Mutex<WasmInner>`) and is pushed with `config.plugin_enabled(id, default_on)`. Its
   `dashboard_page` comes from the guest manifest text.
7. **MCP tools.** `mcp_tools` calls the guest export `rtok_mcp_tools` and parses a JSON
   `ToolDef` list. `invoke_mcp` calls `rtok_on_mcp_tool` with fuel reset, drains the staged
   measurements into `Ctx::record`, and returns the guest text. Today `invoke_mcp` is called
   only by the tests: `src/mcp.rs` skips tools of ids outside `plugins::all()` when it builds
   `tools/list`, and `invoke` has no arm for them.

## Where it lives

| Box | Code |
| --- | --- |
| Registry::from_plugins | `src/plugins/mod.rs` (`Registry::from_plugins`) |
| wasm::append | `src/plugins/wasm.rs` (`append`) |
| WasmPlugin::load | `src/plugins/wasm.rs` (`WasmPlugin::load`, `load_with_fuel`, `FUEL`) |
| Linker imports | `src/plugins/wasm.rs` (`rtok_estimate`, `rtok_record_measurement`, `rtok_log`) |
| manifest valid? | `src/plugins/wasm.rs` (`WasmManifest`, surface checks in `load_with_fuel`) |
| WasmPlugin | `src/plugins/wasm.rs` (`WasmPlugin`, `WasmInner`, `WasmHost`) |
| mcp_tools, invoke_mcp | `src/plugins/wasm.rs` (`mcp_tools`, `invoke_mcp`, `guest_bytes`) |
| mcp tools/list filter | `src/mcp.rs` (the `builtin` check before `p.mcp_tools()`) |
| Config | `crates/rtok-config/` (`[plugins.wasm]` `enabled`, `dir`) |
| Feature flag | `Cargo.toml` (`wasm-host = ["dep:wasmi"]`) |
| Demo guest | `crates/rtok-wasm-demo-guest/` |

## Invariants

- Fail open: a broken `.wasm` is logged and skipped, never fatal to the registry.
- Untrusted guest: every call runs under a fuel budget (`FUEL`), so an endless loop traps
  instead of wedging `rtok mcp` (test `endless_guest_runs_out_of_fuel`); guest pointers are
  bounds-checked (test `guest_slice_rejects_what_used_to_panic`).
- No hook surface: a WASM plugin cannot declare `hook`, so nothing out-of-tree sits on the 10 ms
  hook path.
- A guest cannot record rows for another plugin id, and the host writes the rows through
  `Ctx::record`, so a saving still needs a `Measurement` row.
- Default off: with the feature off no `wasmi` is linked, and with `enabled = false` nothing is read.

## Related

[Overview](../overview.md) · [Plugin contract](../plugin-contract.md) · [Crates](../crates.md)
