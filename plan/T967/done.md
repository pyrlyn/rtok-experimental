Every type under `src/plugins/` that implements `Plugin` now ends in `Plugin`: `ArchivePlugin`, `CmdPlugin`, `CompressPlugin`, `DocsPlugin`, `GraphPlugin`, `GuardPlugin`, `InjectPlugin`, `JsonTreePlugin`, `MeasurePlugin`, `MemoryPlugin`, `ProxyPlugin`, `ReadPlugin`, `ToonPlugin`, and the test fixture `ExtPlugin`; `WasmPlugin` already did. `plugins::all()`, the plugin modules' tests and the integration tests use the new names.

The other uses came from the compiler, not a text replace, since `Read`, `Graph`, `Proxy` and `Memory` are also tool names, config sections and capability traits. Those stay as they are: `rtok_config::Cmd`/`Toon`/…, the SDK's `Archive` capability, the CLI `Cmd` enum. The architecture pages under `docs/architecture/plugins/` (en, ru, uk) name the new types in `X::pre_tool`-style references, `impl Plugin for X` and the `src/plugins/<id>/mod.rs` rows.

Check: `git grep -nE 'impl (rtok_plugin_sdk::)?Plugin for [A-Za-z]+' -- src/plugins` lists only `*Plugin` types; `cargo check --all-targets --all-features` and `cargo fmt --check` are clean. The disk was full while other sessions were building, so with the creator's OK the full `just check` ran as the PR's CI instead of locally.

Status: done 2026-10-23
Model: Claude Code / claude-opus-5-5
