// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Out-of-tree `.wasm` plugin host (P32, T32.2). Loaded only via [`Registry::from_plugins`]
//! when Cargo feature `wasm-host` is on and `[plugins.wasm] enabled = true`.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use rtok_plugin_sdk::{Class, Ctx, DashboardPage, Manifest, Measurement, Plugin, Surface, ToolDef};
use serde::Deserialize;
use serde_json::Value;
use wasmi::{
    Caller, Engine, Error, Instance, Linker, Module, Store, StoreLimits, StoreLimitsBuilder,
};

/// Append WASM plugins from `config.plugins.wasm.dir` when enabled.
pub fn append(
    plugins: Vec<(Box<dyn Plugin>, bool)>,
    config: &crate::config::Config,
) -> Vec<(Box<dyn Plugin>, bool)> {
    if !config.plugins.wasm.enabled {
        return plugins;
    }
    let dir = &config.plugins.wasm.dir;
    let mut out = plugins;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(OsStr::to_str) != Some("wasm") {
            continue;
        }
        match WasmPlugin::load(&path, &config.estimator, &config.log) {
            Ok(p) => {
                let m = p.manifest();
                let on = config.plugin_enabled(m.id, m.default_on);
                out.push((Box::new(p), on));
            }
            Err(e) => {
                let msg = format!("{}: {e:#}", path.display());
                crate::log::stderr_ln(&format!("wasm plugin {msg}"));
                crate::log::append(config, "error", "wasm", "load", &msg);
            }
        }
    }
    out
}

struct WasmHost {
    limits: StoreLimits,
    plugin_id: String,
    staged: Vec<Measurement>,
    estimator: crate::config::Estimator,
    log_path: std::path::PathBuf,
    log_max_bytes: u64,
    log_files: u32,
    log_level: String,
}

struct WasmInner {
    store: Store<WasmHost>,
    instance: Instance,
    fuel: u64,
}

/// Fuel per guest call (≈ wasm instructions). Without a budget a guest `loop` never returns and
/// wedges the synchronous `rtok mcp` loop for every tool, not just the plugin's own.
const FUEL: u64 = 100_000_000;

// Fuel bounds execution, but a single allocation can exhaust the host before fuel runs out.
const MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;
const MAX_TABLE_ELEMENTS: usize = 65_536;

/// One loaded `.wasm` plugin instance.
pub struct WasmPlugin {
    id: &'static str,
    surfaces: &'static [Surface],
    default_on: bool,
    page: DashboardPage,
    inner: Mutex<WasmInner>,
}

#[derive(Deserialize)]
struct WasmManifest {
    id: String,
    surfaces: Vec<String>,
    default_on: bool,
    title: String,
    summary: String,
    #[serde(default)]
    saves_tokens: bool,
}

#[derive(Deserialize)]
struct WasmToolDef {
    name: String,
    description: String,
    input_schema: Value,
}

impl WasmPlugin {
    pub fn load(
        path: &Path,
        estimator: &crate::config::Estimator,
        log: &crate::config::Log,
    ) -> Result<Self, Error> {
        Self::load_with_fuel(path, estimator, log, FUEL)
    }

    fn load_with_fuel(
        path: &Path,
        estimator: &crate::config::Estimator,
        log: &crate::config::Log,
        fuel: u64,
    ) -> Result<Self, Error> {
        let wasm = std::fs::read(path).map_err(|e| Error::new(e.to_string()))?;
        let mut config = wasmi::Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, &wasm)?;
        let mut store = Store::new(
            &engine,
            WasmHost {
                limits: StoreLimitsBuilder::new()
                    .memory_size(MAX_MEMORY_BYTES)
                    .table_elements(MAX_TABLE_ELEMENTS)
                    .memories(1)
                    .tables(1)
                    .instances(1)
                    .build(),
                plugin_id: String::new(),
                staged: Vec::new(),
                estimator: estimator.clone(),
                log_path: log.path.clone(),
                log_max_bytes: log.max_bytes,
                log_files: log.files,
                log_level: log.level.clone(),
            },
        );
        store.limiter(|host| &mut host.limits);
        let mut linker = Linker::new(&engine);
        linker.func_wrap(
            "env",
            "rtok_estimate",
            |caller: Caller<WasmHost>, ptr: i32, len: i32, class: i32| -> Result<i32, Error> {
                let text = mem_read(&caller, ptr, len)?;
                let text = String::from_utf8_lossy(&text);
                let class = match class {
                    0 => Class::Code,
                    1 => Class::Prose,
                    2 => Class::Json,
                    _ => Class::Cjk,
                };
                Ok(crate::tokens::estimate(&text, class, &caller.data().estimator) as i32)
            },
        )?;
        linker.func_wrap(
            "env",
            "rtok_record_measurement",
            |mut caller: Caller<WasmHost>, ptr: i32, len: i32| -> Result<i32, Error> {
                // mut: staged push
                let json = mem_read(&caller, ptr, len)?;
                let m: Value =
                    serde_json::from_slice(&json).map_err(|e| Error::new(e.to_string()))?;
                let plugin = m["plugin"].as_str().unwrap_or("");
                if plugin != caller.data().plugin_id {
                    return Err(Error::new("measurement plugin mismatch"));
                }
                caller.data_mut().staged.push(json_measurement(&m, plugin));
                Ok(0)
            },
        )?;
        linker.func_wrap(
            "env",
            "rtok_log",
            |caller: Caller<WasmHost>,
             level_ptr: i32,
             level_len: i32,
             msg_ptr: i32,
             msg_len: i32|
             -> Result<(), Error> {
                let level_bytes = mem_read(&caller, level_ptr, level_len)?;
                let msg_bytes = mem_read(&caller, msg_ptr, msg_len)?;
                let level = String::from_utf8_lossy(&level_bytes);
                let msg = String::from_utf8_lossy(&msg_bytes);
                let data = caller.data();
                let path = data.log_path.clone();
                let max_bytes = data.log_max_bytes;
                let files = data.log_files;
                let floor = data.log_level.clone();
                let name = if data.plugin_id.is_empty() {
                    "guest".to_string()
                } else {
                    data.plugin_id.clone()
                };
                let file = rtok_log::FileLog {
                    path: &path,
                    max_bytes,
                    files,
                    level: &floor,
                };
                let errors = rtok_log::error_path(&path);
                rtok_log::append_split(&file, Some(&errors), &level, "wasm", &name, &msg);
                crate::log::stderr_ln(&format!("wasm {name} {level}: {msg}"));
                Ok(())
            },
        )?;
        // A `start` function runs guest code too, so it gets a budget like every call.
        store.set_fuel(fuel)?;
        let instance = linker.instantiate_and_start(&mut store, &module)?;
        store.set_fuel(fuel)?;
        let packed = instance
            .get_typed_func::<(), i64>(&store, "rtok_manifest")?
            .call(&mut store, ())?;
        let (ptr, len) = unpack_i64(packed);
        let raw = mem_read_store(&store, &instance, ptr, len)?;
        let parsed: WasmManifest =
            serde_json::from_slice(&raw).map_err(|e| Error::new(e.to_string()))?;
        store.data_mut().plugin_id = parsed.id.clone();
        if parsed.surfaces.iter().any(|s| s == "hook") {
            return Err(Error::new("wasm plugin declares hook surface"));
        }
        let surfaces: Vec<Surface> = parsed
            .surfaces
            .iter()
            .filter_map(|s| match s.as_str() {
                "mcp" => Some(Surface::Mcp),
                "proxy" => Some(Surface::Proxy),
                "cli" => Some(Surface::Cli),
                _ => None,
            })
            .collect();
        if surfaces.is_empty() {
            return Err(Error::new("wasm plugin has no allowed surfaces"));
        }
        let id = leak_str(parsed.id);
        Ok(Self {
            id,
            surfaces: leak_slice(surfaces),
            default_on: parsed.default_on,
            page: DashboardPage::new(parsed.title, parsed.summary, parsed.saves_tokens),
            inner: Mutex::new(WasmInner {
                store,
                instance,
                fuel,
            }),
        })
    }

    /// Dispatch one MCP tool export from the guest and flush staged measurements to `cx`.
    #[allow(dead_code)] // `rtok mcp` dispatch wiring follows; tests call this directly.
    pub fn invoke_mcp(&self, _name: &str, _args: &Value, cx: &Ctx) -> Result<String, Error> {
        let mut inner = self.inner.lock().unwrap();
        let fuel = inner.fuel;
        inner.store.set_fuel(fuel)?;
        let packed = inner
            .instance
            .get_typed_func::<(i32, i32, i32, i32, i32, i32), i64>(
                &inner.store,
                "rtok_on_mcp_tool",
            )?
            .call(&mut inner.store, (0, 0, 0, 0, 0, 0))?;
        let staged = std::mem::take(&mut inner.store.data_mut().staged);
        for m in staged {
            cx.record(&m).map_err(|e| Error::new(e.to_string()))?;
        }
        if packed < 0 {
            return Err(Error::new("wasm tool failed"));
        }
        let (ptr, len) = unpack_i64(packed);
        let bytes = mem_read_store(&inner.store, &inner.instance, ptr, len)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn guest_bytes(&self, export: &str) -> Vec<u8> {
        let mut inner = self.inner.lock().unwrap();
        let fuel = inner.fuel;
        if inner.store.set_fuel(fuel).is_err() {
            return Vec::new();
        }
        let packed = inner
            .instance
            .get_typed_func::<(), i64>(&inner.store, export)
            .and_then(|f| f.call(&mut inner.store, ()))
            .unwrap_or(0);
        let (ptr, len) = unpack_i64(packed);
        mem_read_store(&inner.store, &inner.instance, ptr, len).unwrap_or_default()
    }
}

impl Plugin for WasmPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: self.id,
            surfaces: self.surfaces,
            default_on: self.default_on,
        }
    }

    fn dashboard_page(&self) -> DashboardPage {
        self.page.clone()
    }

    fn mcp_tools(&self) -> Vec<ToolDef> {
        let raw = self.guest_bytes("rtok_mcp_tools");
        serde_json::from_slice::<Vec<WasmToolDef>>(&raw)
            .unwrap_or_default()
            .into_iter()
            .map(|t| ToolDef {
                name: leak_str(t.name),
                description: leak_str(t.description),
                input_schema: t.input_schema,
            })
            .collect()
    }
}

fn json_measurement(v: &Value, plugin: &str) -> Measurement {
    Measurement {
        plugin: intern(plugin),
        kind: intern(v["kind"].as_str().unwrap_or("")),
        before_bytes: v["before_bytes"].as_u64().unwrap_or(0),
        after_bytes: v["after_bytes"].as_u64().unwrap_or(0),
        est_before: v["est_before"].as_u64().unwrap_or(0) as u32,
        est_after: v["est_after"].as_u64().unwrap_or(0) as u32,
        ref_id: v["ref_id"].as_str().map(|s| s.to_string()),
        call_id: v["call_id"].as_i64().map(|n| n as i32),
    }
}

fn mem_read(caller: &Caller<WasmHost>, ptr: i32, len: i32) -> Result<Vec<u8>, Error> {
    let memory = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| Error::new("guest memory export missing"))?;
    guest_slice(memory.data(caller), ptr, len)
}

fn mem_read_store(
    store: &Store<WasmHost>,
    instance: &Instance,
    ptr: i32,
    len: i32,
) -> Result<Vec<u8>, Error> {
    let memory = instance
        .get_memory(store, "memory")
        .ok_or_else(|| Error::new("guest memory export missing"))?;
    guest_slice(memory.data(store), ptr, len)
}

/// Copy the guest's `(ptr, len)` out of `data`. Both halves come from an untrusted `.wasm`:
/// `ptr as usize` sign-extended a negative `ptr` and `start + len` overflowed, so the old
/// `data[start..end]` panicked the host (and `rtok mcp` with it) instead of failing the call.
fn guest_slice(data: &[u8], ptr: i32, len: i32) -> Result<Vec<u8>, Error> {
    usize::try_from(ptr)
        .ok()
        .zip(usize::try_from(len).ok())
        .and_then(|(p, l)| data.get(p..p.checked_add(l)?))
        .map(<[u8]>::to_vec)
        .ok_or_else(|| Error::new("guest memory read out of bounds"))
}

fn unpack_i64(packed: i64) -> (i32, i32) {
    let u = packed as u64;
    ((u as u32) as i32, (u >> 32) as u32 as i32)
}

fn leak_str(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// One leak per distinct string. `Measurement` wants `&'static str`, and a guest records one
/// per call; leaking each would grow `rtok mcp` for as long as it runs.
fn intern(s: &str) -> &'static str {
    static POOL: Mutex<Option<HashSet<&str>>> = Mutex::new(None);
    let mut g = POOL.lock().unwrap_or_else(PoisonError::into_inner);
    let pool = g.get_or_insert_with(HashSet::new);
    if let Some(k) = pool.get(s) {
        return k;
    }
    let k = leak_str(s.to_string());
    pool.insert(k);
    k
}

fn leak_slice(v: Vec<Surface>) -> &'static [Surface] {
    Box::leak(v.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtok_plugin_sdk::testing::MemoryHost;
    use std::path::PathBuf;
    use std::process::Command;

    fn build_demo_wasm() -> PathBuf {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let target = manifest_dir.join("target/wasm32-demo");
        let status = Command::new("cargo")
            .current_dir(&manifest_dir)
            .args([
                "build",
                "-p",
                "rtok-wasm-demo-guest",
                "--target",
                "wasm32-unknown-unknown",
                "--release",
            ])
            .env("CARGO_TARGET_DIR", &target)
            .status()
            .expect("cargo build guest");
        assert!(status.success(), "guest wasm build failed");
        target.join("wasm32-unknown-unknown/release/rtok_wasm_demo_guest.wasm")
    }

    #[test]
    fn from_plugins_loads_demo_and_records_measurement() {
        let wasm_src = build_demo_wasm();
        let dir = std::env::temp_dir().join(format!("rtok-wasm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wasm_dst = dir.join("wasm-demo.wasm");
        std::fs::copy(&wasm_src, &wasm_dst).unwrap();

        let mut cfg = crate::config::Config::default();
        cfg.plugins.wasm.enabled = true;
        cfg.plugins.wasm.dir = dir.clone();

        let reg = super::super::Registry::from_plugins(vec![], &cfg);
        let enabled: Vec<_> = reg.enabled().collect();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].manifest().id, "wasm-demo");

        let plugin = WasmPlugin::load(&wasm_dst, &cfg.estimator, &cfg.log).unwrap();
        let host = MemoryHost::new();
        let cx = Ctx::new(&host);
        let text = plugin
            .invoke_mcp("demo", &serde_json::json!({}), &cx)
            .unwrap();
        assert!(text.contains("ok"));

        let rows = host.recorded();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].plugin, "wasm-demo");
        assert_eq!(rows[0].kind, "demo");
        assert!(rows[0].before_bytes >= rows[0].after_bytes);
        assert!(rows[0].est_before >= rows[0].est_after);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn guest_slice_rejects_what_used_to_panic() {
        let data = [7u8; 16];
        assert!(guest_slice(&data, -1, 2).is_err(), "negative ptr");
        assert!(guest_slice(&data, 0, -1).is_err(), "negative len");
        assert!(
            guest_slice(&data, i32::MAX, i32::MAX).is_err(),
            "past the end"
        );
        assert!(guest_slice(&data, 8, 9).is_err(), "one byte past the end");
        assert_eq!(guest_slice(&data, 8, 8).unwrap(), vec![7u8; 8]);
        assert!(guest_slice(&data, 16, 0).unwrap().is_empty());
    }

    /// A guest that never returns fails `load` once its fuel is gone instead of hanging.
    #[test]
    fn endless_guest_runs_out_of_fuel() {
        let dir = std::env::temp_dir().join(format!("rtok-wasm-fuel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("spin.wasm");
        std::fs::write(
            &path,
            r#"(module
                 (memory (export "memory") 1)
                 (func (export "rtok_manifest") (result i64)
                   (loop $l (br $l))
                   (i64.const 0)))"#,
        )
        .unwrap();
        let cfg = crate::config::Config::default();
        let err = WasmPlugin::load_with_fuel(&path, &cfg.estimator, &cfg.log, 10_000).err();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(err.is_some(), "an endless guest must not load");
    }

    #[rstest::rstest]
    #[case("(memory (export \"memory\") 1)", "", true)]
    #[case("(memory (export \"memory\") 1025)", "", false)]
    #[case(
        "(memory (export \"memory\") 1)",
        "(if (i32.ne (memory.grow (i32.const 1024)) (i32.const -1)) (then unreachable))",
        true
    )]
    #[case("(memory (export \"memory\") 1) (table 65537 funcref)", "", false)]
    #[case(
        "(memory (export \"memory\") 1) (table 1 funcref)",
        "(if (i32.ne (table.grow (ref.null func) (i32.const 65536)) (i32.const -1)) (then unreachable))",
        true
    )]
    fn guest_allocations_are_bounded_before_start(
        #[case] declarations: &str,
        #[case] start: &str,
        #[case] accepted: bool,
    ) {
        use std::fmt::Write as _;

        let manifest = r#"{"id":"bounded","surfaces":["mcp"],"default_on":true,"title":"Bounded","summary":"fixture"}"#;
        let encoded = manifest.bytes().fold(String::new(), |mut out, byte| {
            write!(out, "\\{byte:02x}").unwrap();
            out
        });
        let dir = crate::testutil::tmp_dir("wasm-allocation-boundary");
        let path = dir.join("fixture.wasm");
        std::fs::write(
            &path,
            format!(
                r#"(module {declarations}
                    (data (i32.const 0) "{encoded}")
                    (func $start {start}) (start $start)
                    (func (export "rtok_manifest") (result i64) (i64.const {})))"#,
                (manifest.len() as u64) << 32,
            ),
        )
        .unwrap();
        let cfg = crate::config::Config::default();
        let result = WasmPlugin::load(&path, &cfg.estimator, &cfg.log);
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(result.is_ok(), accepted, "{declarations}; {start}");
    }
}
