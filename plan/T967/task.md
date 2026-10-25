---
status: in progress
priority: P3
complexity: 2
readiness: 10%
agent: Claude Code / claude-opus-5-5
---

# T967. Plugin types under src/plugins end in Plugin

The creator's request. The types that `impl Plugin` under `src/plugins/` carry the bare id
(`Cmd`, `Read`, `Graph`, `Memory`, `Proxy`, …). Those names collide with unrelated types:
the CLI `Cmd` enum, `std::io::Read`, the config sections `rtok_config::Cmd`/`Toon`/….
Only `WasmPlugin` already says what it is.

Done: every type under `src/plugins/` that implements `Plugin` ends in `Plugin` (`Cmd` →
`CmdPlugin`, `JsonTree` → `JsonTreePlugin`, the test fixture `Ext` → `ExtPlugin`). There is no
behaviour change, and docs that name a plugin type use the new name.

Check: `just check` green; `git grep -nE 'impl (rtok_plugin_sdk::)?Plugin for [A-Za-z]+' -- src/plugins`
lists only `*Plugin` types.

Plan:

1. Rename the 13 plugin structs in `src/plugins/<id>/mod.rs` and the `Ext` fixture in
   `src/plugins/mod.rs`, then the `plugins::all()` list.
2. Let the compiler find the remaining uses (tests and helpers inside each plugin module)
   and fix them; no blind replace, because `Read`, `Graph`, `Proxy` and `Memory` are common words.
3. Update the docs and per-plugin `AGENTS.md`/`README.md` that name a type.
4. `just check`, commit `T967: …`, PR.
