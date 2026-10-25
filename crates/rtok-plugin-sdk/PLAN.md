# `rtok-plugin-sdk` — where the crate line goes (T23.0, D15, D25)

Design note for the published plugin contract. No code lands from this document; T23.1 does.

## Problem

D6 says third parties extend rtok from outside, through `rtok::plugin` and
`Registry::from_plugins`. Today that costs them the whole binary crate. Measured on this tree 2026-09-09
(`awk '/^\[dependencies\]/{f=1;next} /^\[/{f=0} f && /^[a-z]/' Cargo.toml | wc -l` → 35 entries, 24
of them not optional): **24 direct dependencies always, 33 in a default build** — the default
feature set pulls the optional crates that are on (`lbug` removed, P39). diesel with a bundled SQLite C library, axum,
tokio, reqwest, rmcp, clap, figment, seven tree-sitter grammars. A crate that wants to implement
one trait with two methods compiles all of it, and takes rtok's MSRV, its feature flags and its
release cadence with it.

The second half of the problem is that the contract has no edge. `Plugin` takes `&Ctx`, `Ctx`
exposes `pub store: Store`, and `Store` is 1 192 lines of whatever the last task needed. Measured
the same day (`grep -rhoE "cx\.store\.[a-z_0-9]+" src/plugins/ | sort -u`): the ten plugins call
**26 distinct `Store` methods across 5 areas** — symbols 11 (graph), ledger 6 (archive, guard,
inject, read, graph), archive 3, read cache 3, notes 4 (memory). Nothing says which of the other
~40 `Store` methods a plugin may call; nothing breaks if one starts. That is not an API, it is a
habit, and it cannot be versioned.

Sizes that price the options below, `wc -l` on 2026-09-09: `src/store/` 2 219 (+ 387 for the
`src/config/` 1 631, `src/tokens.rs` 112, `src/proxy/wire.rs` 150.

## Alternatives

How three projects outside this stack drew the same line between "the host" and "a plugin".
Versions and dates from the crates.io API, looked up 2026-09-09.

| Tool | Version | Date | Gets right | Gets wrong |
|------|---------|------|------------|------------|
| `bevy_app::Plugin` | 0.19.1 | 2026-08-13 | One required method (`build(&self, app: &mut App)`), everything else defaulted; the contract crate is small enough to depend on alone rather than pulling the whole engine. | The contract crate *is* the runtime type: `App` lives there, so its 17 direct dependencies are the plugin author's, and every change to the runtime is a change to the published contract. |
| `tower-layer` / `tower-service` | 0.3.3 | 2024-08-13 | **0 direct dependencies each.** The trait is its own crate, so a middleware author compiles nothing but the trait, and the traits have been stable for years precisely because they carry nothing. | The traits are so abstract they grant no capability — every useful thing lives in `tower` (utilities) or the host, so an author reads two crates to write one thing. A pure-contract SDK here would land in the same place: `Plugin` alone cannot record a `Measurement`. |
| `nu-plugin` + `nu-protocol` | 0.115.1 | 2026-08-23 | A process boundary means the host cannot leak types at all; the contract is a wire protocol and is therefore explicit by construction. | `nu-protocol` carries 38 direct dependencies and `nu-plugin` 8; a call costs a process hop. D1 gives the whole hook 10 ms, so a hop per dispatch is out before the ergonomics are discussed. |

The lesson the three agree on: the contract crate must carry *capabilities*, not the runtime that
implements them (bevy's mistake) and not nothing at all (tower's), and it must do it in-process
(nu's cost).

## Mechanism

**Chosen: the middle line (option C).** `rtok-plugin-sdk` carries the trait, the events, the
value types and *traits describing what the host can do for a plugin*. It carries no storage
engine, no config schema and no surface. Its dependency list is **`serde`, `serde_json`,
`anyhow`** — three, none of them C.

What crosses the line:

- **What a plugin is**: `Plugin`, `Manifest`, `Surface`.
- **Events, borrowed**: `PreToolUse`, `PostToolUse`, `SessionStart`, `PromptSubmit`, `PreCompact`,
  and the proxy request view.
- **What a plugin returns**: `PreToolDecision`, `Injection`, `ToolDef`, `DashboardPage`,
  `Measurement`.
- **What a plugin may ask of the host**: `Host` (estimate, record a `Measurement`, record a call,
  record tokens, log, read its own config section) plus one trait per store area a plugin actually
  uses — `Archive`, `Notes`, `ReadCache`, `Symbols`, `Ledger`. Each is a named list, so "what a
  plugin may touch" is a document, not a habit.

What does not cross: `Store` and its schema, `Config` and its 100-odd keys, `rtok hook` / `mcp` /
`proxy` / `web`. `rtok` implements the traits on `Ctx` and stays the only dispatcher.

The 26 methods map onto the five capability traits one for one; no method becomes public that no
plugin calls today, and adding one later is a semver-minor addition with a default body where that
makes sense.

**Config.** A plugin reads its own section, `[plugins.<id>]`, through
`Host::plugin_config::<T>() -> Result<T>` with `T: serde::de::DeserializeOwned` — the plugin owns
the struct, which is what lets an out-of-tree plugin have settings at all. Core values a plugin
legitimately needs (the archive directory) are named accessors. The estimator stays behind
`Host::estimate(text, class)` because its rates are config the host owns (D12).

**Required methods.** `manifest()` — a plugin that will not say its id, surfaces and default state
cannot be registered. `dashboard_page()` — D23 says every plugin has a page on both operator
surfaces, and today the copy for all ten lives in one `DashboardPage::from_id` match in the host,
which is exactly backwards: an out-of-tree plugin falls through it to "External plugin". Making it
required moves each plugin's own description to the plugin. Everything else keeps a no-op default,
so a plugin still implements only the surfaces its `Manifest` declares.

**Versioning.** `cargo-semver-checks` in CI on the SDK only; the `rtok` binary crate stays off
crates.io (dist ships it), so the published surface is exactly this crate.

Target: `rtok stats --json` byte-identical before and after

## Rejected

- **(A) The runtime moves into the SDK** — `Config`, `Store`, `tokens` and the wire views go with
  the trait, `rtok` becomes surfaces on top. Every internal plugin would compile unchanged, which
  is its only real merit. It publishes 4 112 lines of host internals as public API: every `Store`
  method and every config key becomes a semver promise, and a plugin author compiles diesel plus a
  bundled SQLite to implement a trait. This is bevy's line, and bevy pays for it with 17
  dependencies on the contract crate.
- **(B) Contract only, no capabilities** — the tower line: publish `Plugin` and the event types and
  nothing else. Zero dependencies and a surface that never breaks, but a plugin cannot record a
  `Measurement`, which makes it useless under D3: a saving that is not a `Measurement` row does not
  exist. Every plugin would reach back into `rtok` for the one thing it must do, and the crate line
  would be decorative.
- **(C′) Out-of-process plugins** — nu's line, and genuinely the only way to make the boundary
  airtight. D1 gives a hook 10 ms end to end; a process hop per dispatch spends most of that
  budget. Kept where it belongs: the v0.2+ WASM plugin host, which is the same idea without the
  process.

Falsified by: if the first two plugin tasks after the migration each need a new method on a
capability trait, the traits are tracking `Store` rather than describing a contract — and the
honest fix is then option A, publishing the store as the API and paying its cost openly.

---

## v0.2 survey — WASM plugin host (T32.0, P32, I-26) · 2026-09-11

Design note for loading out-of-tree `.wasm` plugins through `Registry::from_plugins`. No code
lands from this section; T32.1–T32.2 do.

### Problem

D6 lets third parties extend rtok from outside via `rtok::plugin` and `Registry::from_plugins`, but
today that means **linking a native `dyn Plugin`** — the author compiles against `rtok` or ships a
dylib the host loads. D1 scoped v0.1 to in-tree native plugins on every surface; v0.2 adds a **WASM
host** so a third party can ship a `.wasm` blob without linking Rust ABIs, while **this repo still
writes every catalogue plugin from scratch** and never vendors someone else's plugin tree (D6).

The constraint that shapes every decision below: **`rtok hook` must stay ≤ 10 ms and fail-open**
(D1, D22). WASM instantiation and interpreter dispatch are milliseconds, not microseconds — fine on
`rtok mcp` and `rtok proxy` (long-running, amortized), **unacceptable on the hook path**. The host
therefore has two registration modes, not one.

### Alternatives — embeddable WASM runtimes

Three runtimes surveyed for embedding in a **static Rust binary** (`cargo-dist`, no runtime DLL
beyond the OS). Versions from crates.io / upstream release pages, looked up **2026-09-11**.

| Runtime | Version | Date | Gets right | Gets wrong |
|---------|---------|------|------------|------------|
| **Wasmtime** (`wasmtime`) | 26.0.0 (crates.io latest stable) | 2025-05 | Production track record; Pulley interpreter + optional Cranelift JIT; Component Model and WASI; `runtime` + `pulley` features embed in static binaries; Bytecode Alliance maintenance. | Default build is **large** (Cranelift, WASI, CLI-sized dependency graph); minimal Pulley-only builds need feature surgery and still outweigh a pure interpreter; overkill when plugins run occasionally on MCP/proxy, not per hook. |
| **Wasmi** (`wasmi`) | 1.0.0 (stable); 2.0.0-beta.2 upstream | 2025-10 / 2026-03 | **Pure Rust**, static-friendly, no C toolchain; API deliberately mirrors Wasmtime; built-in **fuel metering**; `no_std` path documented; 100 % Wasm testsuite compliance claimed; sized for plugin / embedded hosts. | Interpreter-only — slower than Wasmtime JIT on CPU-heavy guests (irrelevant for token-shaping plugins); 2.x still beta if we need bleeding-edge Wasm proposals. |
| **Wasm3** (`wasm3` / `wasm3x`) | `wasm3` 0.3.1 on crates.io; `wasm3x` 0.1.x | 2021-09 / 2025 | **Smallest** interpreter footprint; `wasm3x` wraps C sources into `libwasm3.a` via `cc` for static link; Wasmi/Wasmtime-shaped safe API in `wasm3x`. | **C + bindgen + libclang** in the build (fragile for `cargo-dist` cross-compile); **best-effort validation** — not a fully validating runtime; **single-threaded** (`Engine`/`Store` not `Send`); `wasm3-rs` maintenance stale (2021); thinner host callback model makes `Host` bridging harder. |

Reference (not counted as a fourth surveyed runtime): **WAMR** — also C, also static-friendly,
used in embedded benchmarks; same C-toolchain and validation trade-offs as Wasm3 without a maintained
Wasmi-shaped Rust wrapper.

### Mechanism

**Chosen host: Wasmi** behind Cargo feature `wasm-host` (default **off**, T32.1). It is the only
surveyed runtime that is **Rust-native, validating, and sized for a plugin sandbox** without pulling
Cranelift or a C interpreter into the static binary. Wasmtime remains the documented escape hatch if
Gate P32 fails on capability bridging; Wasm3 is too thin on safety and build ergonomics for
arbitrary third-party `.wasm`.

**D6 — no vendored third-party plugins.** This repository contains:

- native catalogue plugins under `src/plugins/<id>/` only;
- a **first-party** example guest crate (e.g. `crates/rtok-wasm-demo-guest/`) built in CI to
  `*.wasm` — a build artefact and docs sample, **not** a checked-in third-party plugin and not
  listed in `all()`;
- zero `plugins/vendor/`, zero submodule of someone else's plugin repo.

Third parties ship `.wasm` (+ optional sidecar manifest) into **`~/.rtok/plugins/`** on their
machines; rtok loads them at runtime when the feature and config flag are on.

**What stays in-process (in-tree).** Unchanged from v0.1:

| Piece | Stays |
|-------|--------|
| Ten catalogue plugins | Native `impl Plugin` in `src/plugins/`, selected by Cargo features, registered through `all()`. |
| `Registry::new(config)` | `from_plugins(all(), config)` — **native only**; used by `rtok hook` and every path that must stay ≤ 10 ms. |
| Host capabilities | `rtok` implements `Host`, `Archive`, `Notes`, … on `Ctx`; native plugins call them directly. |
| Measurement rows | Host writes `measurements` from `Host::record`; one code path per saving (D3, D6). |
| Operator surfaces | `Plugin::dashboard_page` from native plugins; WASM plugins supply manifest text via sidecar or export. |

**What moves to WASM (out-of-tree only).** Third-party logic that would today require `Box::new(MyNativePlugin)`:

- compiled to `wasm32-unknown-unknown` (or `wasm32-wasip1` later if needed);
- loaded from `~/.rtok/plugins/<id>.wasm`;
- bridged by `WasmPlugin: Plugin` that marshals events across the Wasmi `Linker` imports.

WASM plugins may declare **`Surface::Mcp`**, **`Surface::Proxy`**, **`Surface::Cli`** only — never
`Surface::Hook`. The loader rejects `.wasm` whose manifest lists `hook`.

#### `from_plugins` load path

Two entry points, one trait:

```
rtok hook / fail-open paths          rtok mcp / rtok proxy / tests
        │                                      │
        ▼                                      ▼
 Registry::new(config)              Registry::from_plugins(vec, config)
 = from_plugins(all(), config)       = caller natives + optional WASM dir
 native catalogue only               when [plugins.wasm] enabled + feature on
        │                                      │
        └──────────────┬───────────────────────┘
                       ▼
              Vec<(Box<dyn Plugin>, enabled)>
              config.plugin_enabled(m.id, m.default_on)
```

**Config** (T32.1; all off by default):

```toml
[plugins.wasm]
enabled = false
dir = "~/.rtok/plugins"   # scan at most one level: *.wasm
```

**Load sequence** (long-running surfaces only):

1. `Registry::from_plugins(native, config)` starts from the caller's `Vec<Box<dyn Plugin>>` (for
   `rtok mcp` / `rtok proxy`: `all()` plus nothing else in-tree).
2. If `cfg(feature = "wasm-host")` **and** `config.plugins.wasm.enabled`: read
   `config.plugins.wasm.dir`, for each `*.wasm`:
   - parse sidecar `<stem>.toml` **or** call guest export `rtok_manifest() -> (ptr, len)` (JSON:
     `id`, `surfaces`, `default_on`, `dashboard_title`, `dashboard_blurb`);
   - reject if `surfaces` contains `hook`;
   - `wasmi::Engine::default()` once per process; `Module::new`, `Linker` with **allow-listed**
     host imports only (see below);
   - push `Box::new(WasmPlugin { … })` onto the vec.
3. Existing enablement: `config.plugin_enabled(m.id, m.default_on)` per plugin.
4. Dispatch unchanged: `rtok` is still the only surface owner; `WasmPlugin` methods deserialize
   event → guest call → deserialize result.

**Guest ↔ host ABI (T32.2 scope).** Start with a **core Wasm module** and explicit imports — not
the full Component Model on day one:

| Host import | Maps to |
|-------------|---------|
| `rtok_estimate(ptr, len, class) -> u32` | `Host::estimate` |
| `rtok_record_measurement(ptr, len) -> i32` | `Host::record` — JSON `Measurement` |
| `rtok_log(level_ptr, level_len, msg_ptr, msg_len)` | `Host::log` |

Guest exports (minimal Gate P32 surface): `rtok_manifest`, `rtok_mcp_tools` (JSON `ToolDef[]`),
`rtok_on_mcp_tool(name_ptr, name_len, args_ptr, args_len, out_ptr, out_cap) -> i32`. Proxy and CLI
surfaces stay no-op defaults until a later task; Gate P32 needs only **one** dispatch that records a
`Measurement`.

WIT definitions live in `rtok-plugin-sdk` (guest-facing, versioned); the host linker whitelists
exactly those import names — no raw `Store` escape.

**Sandbox.** Wasmi `Config::consume_fuel(true)` with a fixed budget of 100,000,000 fuel per
guest call and for the `start` function (`FUEL` in `src/plugins/wasm.rs`). It is not
configurable: `[plugins.wasm]` holds only `enabled` and `dir`, and an unknown key such as
`fuel_per_call` fails config parsing. No Wasmi `Store` memory limit is set yet.
Failed guest trap → log + no-op (fail open on MCP/proxy, same spirit as hooks).

#### Measurement example contract

Gate P32 example guest (`wasm-demo`, MCP surface only) must produce **one row** the host persists
unchanged. Shape matches `rtok_plugin_sdk::Measurement` and `examples/hello_plugin.rs`:

```rust
// Guest logic (conceptual — runs inside Wasmi)
let before = host_estimate("012345678901234567", CLASS_CODE); // illustrative payload
host_record_measurement(&json!({
    "plugin": "wasm-demo",
    "kind": "demo",
    "before_bytes": 17,
    "after_bytes": 4,
    "est_before": before,
    "est_after": host_estimate("done", CLASS_CODE),
    "ref_id": null,
    "call_id": null
}));
```

**Host obligation:** deserialize JSON, validate `plugin` matches manifest `id`, call
`Host::record(&Measurement { … })` — same as native. **Test obligation (Gate P32):**

1. Build `wasm-demo.wasm` from the first-party guest crate (CI artefact).
2. `Registry::from_plugins(vec![], config)` with `plugins.wasm.enabled = true` and dir pointing at
   the artefact.
3. Dispatch one MCP tool call on `wasm-demo`.
4. Assert `store` / `rtok stats --json` contains one `measurements` row:
   `plugin = "wasm-demo"`, `kind = "demo"`, `before_bytes >= after_bytes`, `est_before >= est_after`.
5. `Registry::new` + full in-tree `cargo test` green — no catalogue plugin registers differently.

### Rejected

- **Wasmtime as default host** — capability and spec completeness are excellent, but binary size and
  compile-time cost fight P17 and the "occasional MCP tool" workload; Pulley-only mode still heavier
  than Wasmi for this embedding. Revisit only if Wasmi cannot implement the host-import bridge under
  Gate P32.
- **Wasm3 / wasm3x** — smallest code, but C build + non-validating execution + stale `wasm3-rs` +
  non-`Send` store are the wrong trade for **untrusted** third-party plugins in a static Rust ship.
- **WASM on `rtok hook`** — violates D1 latency budget regardless of runtime; hook keeps
  `Registry::new` (native `all()` only).
- **Subprocess / IPC plugins** — same rejection as T23 `(C′)`: process hop per dispatch; nu-line
  without WASM's sandbox benefit.
- **Vendoring third-party `.wasm` in this repo** — D6; example is first-party build output only.
- **Full Wasmtime Component Model on day one** — correct long-term shape, but T32.2 only needs one
  `Measurement`; module + JSON over shared memory is enough to close Gate P32.

### Gate P32 — example shape

**Definition:** the WASM host is shippable when an out-of-tree `.wasm` plugin loaded through
`Registry::from_plugins` records a real `Measurement` row and **every in-tree plugin test passes
unchanged**.

| Step | Action | Pass |
|------|--------|------|
| 1 | Default build: `wasm-host` feature **off**, `[plugins.wasm] enabled = false` | `just check` green; `rtok hook` p95 unchanged; no Wasmi linked. |
| 2 | Build first-party `wasm-demo.wasm` (guest crate in repo; artefact not committed). | `cargo build -p rtok-wasm-demo-guest --target wasm32-unknown-unknown` succeeds in CI. |
| 3 | Enable feature + config; empty native vec + wasm dir | `Registry::from_plugins(vec![], &cfg)` registers `wasm-demo`. |
| 4 | One MCP dispatch | Exactly one `measurements` row: `plugin=wasm-demo`, `kind=demo`, byte and token estimates consistent. |
| 5 | In-tree regression | `Registry::new` path unchanged; `from_plugins_takes_external_plugins` still passes; catalogue `cargo test` count unchanged. |

**T32.2 Check:** steps 3–4 automated in integration test; step 5 is the existing suite.

Falsified by (T32 WASM host): if bridging `Host::record` requires exposing `Store` methods to the
guest linker, the ABI has leaked the pre-SDK habit — stop and narrow imports, or admit option A from
T23 was right.
