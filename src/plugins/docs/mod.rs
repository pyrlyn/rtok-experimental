// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Local crate docs from `Cargo.lock` + a cached docs.rs rustdoc JSON index (T455).

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use rtok_plugin_sdk::{Class, DashboardPage, Manifest, Measurement, Plugin, Surface, ToolDef};

pub struct DocsPlugin;

impl Plugin for DocsPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "docs",
            surfaces: &[Surface::Mcp, Surface::Cli],
            default_on: false,
        }
    }

    fn dashboard_page(&self) -> DashboardPage {
        DashboardPage::new(
            "Docs",
            "Ranked rustdoc snippets from Cargo.lock, fetched once and searched offline.",
            true,
        )
    }

    fn mcp_tools(&self) -> Vec<ToolDef> {
        vec![
            ToolDef {
                name: "docs_resolve",
                description: "Exact crate name and version from Cargo.lock for a dependency name.",
                input_schema: json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}),
            },
            ToolDef {
                name: "docs_query",
                description: "FTS snippets for one cached crate version; ids only, docs_get loads a body.",
                input_schema: json!({"type":"object","properties":{"name":{"type":"string"},"version":{"type":"string"},"query":{"type":"string"},"question":{"type":"string","description":"alias of query"},"userQuery":{"type":"string","description":"alias of query"}},"required":["name"],"anyOf":[{"required":["query"]},{"required":["question"]},{"required":["userQuery"]}]}),
            },
            ToolDef {
                name: "docs_get",
                description: "One cached doc item by id.",
                input_schema: json!({"type":"object","properties":{"id":{"type":"integer"}},"required":["id"]}),
            },
        ]
    }
}

#[derive(Debug, Deserialize)]
struct CargoLock {
    #[serde(default)]
    package: Vec<LockPackage>,
}

#[derive(Debug, Deserialize)]
struct LockPackage {
    name: String,
    version: String,
}

/// Exact name + version from a Cargo.lock body. First matching package wins.
pub fn resolve_lock(lock_text: &str, name: &str) -> Option<(String, String)> {
    let lock: CargoLock = toml::from_str(lock_text).ok()?;
    lock.package
        .into_iter()
        .find(|p| p.name == name)
        .filter(|p| safe_part(&p.name) && safe_part(&p.version))
        .map(|p| (p.name, p.version))
}

/// A crate name or version as crates.io allows them. `Cargo.lock` comes from the checkout, and
/// both end up in the docs.rs URL and in `<home>/docs/<name>/<version>`: a version of `../..`
/// or `/abs` would write outside the docs cache.
fn safe_part(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
        && !s.contains("..")
}

fn lock_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("Cargo.lock")
}

fn miss_lock(name: &str) -> String {
    format!("not in Cargo.lock: {name}")
}

fn not_cached(name: &str) -> String {
    format!("not cached: rtok docs fetch {name}")
}

/// Version a workspace's `llms.txt` is cached under: the canonical cwd after `local:`, so
/// workspaces sharing one store never see each other's text. Callers still say `local`.
fn local_version() -> String {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);
    format!("local:{}", cwd.display())
}

pub fn docs_resolve(rt: &crate::plugin::Runtime, name: &str) -> Result<String> {
    let _ = ingest_llms(rt);
    if name == "llms" {
        let cached = rt
            .store
            .doc_crate_cached("llms", &local_version())
            .unwrap_or(false);
        return Ok(json!({"name":"llms","version":"local","cached":cached}).to_string());
    }
    let text = match std::fs::read_to_string(lock_path()) {
        Ok(t) => t,
        Err(_) => return Ok(miss_lock(name)),
    };
    let Some((n, ver)) = resolve_lock(&text, name) else {
        return Ok(miss_lock(name));
    };
    let cached = rt.store.doc_crate_cached(&n, &ver).unwrap_or(false);
    Ok(json!({"name":n,"version":ver,"cached":cached}).to_string())
}

pub fn docs_query(
    rt: &crate::plugin::Runtime,
    name: &str,
    version: Option<&str>,
    query: &str,
) -> Result<String> {
    let _ = ingest_llms(rt);
    let ver = match version {
        Some(v) if !v.is_empty() && !(name == "llms" && v == "local") => v.to_string(),
        _ if name == "llms" => local_version(),
        _ => {
            let text = match std::fs::read_to_string(lock_path()) {
                Ok(t) => t,
                Err(_) => return Ok(not_cached(name)),
            };
            match resolve_lock(&text, name) {
                Some((_, v)) => v,
                None => return Ok(not_cached(name)),
            }
        }
    };
    if !rt.store.doc_crate_cached(name, &ver).unwrap_or(false) {
        return Ok(not_cached(name));
    }
    let limit = rt.config.plugins.docs.query_limit.max(1);
    let snippet = rt.config.plugins.docs.snippet_chars.max(1);
    let max_tokens = rt.config.plugins.docs.max_tokens.max(1);
    let hits = rt
        .store
        .search_doc_items(name, &ver, query, limit, snippet)?;
    let considered: Vec<i32> = hits.iter().map(|h| h.id).collect();
    // The saving's before side is the full text of everything considered, before snippet and
    // budget trimming, for bytes and tokens alike.
    let bodies = rt.store.doc_items_docs(&considered).unwrap_or_default();
    let before_bytes = bodies.iter().map(|d| d.len() as u64).sum();
    let mut kept = Vec::new();
    for h in &hits {
        let next = json!({"id": h.id, "path": h.path, "kind": h.kind, "snippet": h.snippet});
        let mut trial = kept.clone();
        trial.push(next.clone());
        let text = serde_json::to_string(&trial).unwrap_or_default();
        if rt.estimate(&text, Class::Prose) > max_tokens {
            break;
        }
        kept.push(next);
    }
    let text = serde_json::to_string(&kept).unwrap_or_else(|_| "[]".into());
    if !kept.is_empty() {
        let after_bytes = text.len() as u64;
        let est_before = rt.estimate(&bodies.join("\n"), Class::Prose);
        let est_after = rt.estimate(&text, Class::Prose);
        let _ = rt.record(&Measurement {
            plugin: "docs",
            kind: "query",
            before_bytes,
            after_bytes,
            est_before,
            est_after,
            ref_id: Some(format!("{name}@{ver}")),
            call_id: rt.call_id,
        });
    }
    Ok(text)
}

pub fn docs_get(rt: &crate::plugin::Runtime, id: i32) -> Result<String> {
    match rt.store.get_doc_item(id)? {
        Some(item) => Ok(json!({
            "id": item.id,
            "path": item.path,
            "kind": item.kind,
            "docs": item.docs,
        })
        .to_string()),
        None => anyhow::bail!("unknown doc id: {id}"),
    }
}

/// Split `llms.txt` / `llms-full.txt` on heading lines into crate `llms`, filed under this
/// workspace's [`local_version`]. Unchanged bytes touch nothing (ids stay valid); a missing
/// or empty source drops the workspace's cache so it reports uncached, not stale text.
fn ingest_llms(rt: &crate::plugin::Runtime) -> Result<()> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let version = local_version();
    let body = ["llms-full.txt", "llms.txt"]
        .iter()
        .map(|n| cwd.join(n))
        .find(|p| p.is_file())
        .map(std::fs::read_to_string)
        .transpose()?;
    let items = body.as_deref().map(split_llms).unwrap_or_default();
    let (Some(body), false) = (body, items.is_empty()) else {
        if rt.store.doc_crate_cached("llms", &version)? {
            rt.store.delete_doc_crate("llms", &version)?;
        }
        return Ok(());
    };
    let sha = crate::store::hex_sha256(body.as_bytes());
    let now = i64::try_from(crate::log::now()).unwrap_or(0);
    rt.store
        .sync_doc_crate("llms", &version, &sha, 0, now, &items)?;
    Ok(())
}

fn split_llms(body: &str) -> Vec<(String, String, String)> {
    let mut items = Vec::new();
    let mut heading = String::from("llms");
    let mut buf = String::new();
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix('#') {
            if !buf.trim().is_empty() {
                items.push((heading.clone(), "llms".into(), buf.trim().to_string()));
            }
            heading = rest.trim().to_string();
            if heading.is_empty() {
                heading = "llms".into();
            }
            buf.clear();
        } else {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    if !buf.trim().is_empty() {
        items.push((heading, "llms".into(), buf.trim().to_string()));
    }
    items
}

/// Walk rustdoc JSON `paths` + `index`; keep path, kind, docs string.
pub fn index_rustdoc(json: &Value) -> (i32, Vec<(String, String, String)>) {
    let format_version = json
        .get("format_version")
        .and_then(Value::as_u64)
        .unwrap_or(0) as i32;
    let paths = json.get("paths").and_then(Value::as_object);
    let index = json.get("index").and_then(Value::as_object);
    let mut items = Vec::new();
    if let Some(paths) = paths {
        for (id, summary) in paths {
            let path = summary
                .get("path")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("::")
                })
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| id.clone());
            let kind = kind_str(summary.get("kind").unwrap_or(&Value::Null));
            let docs = index
                .and_then(|ix| ix.get(id))
                .and_then(|item| item.get("docs"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if docs.is_empty() {
                continue;
            }
            items.push((path, kind, docs));
        }
    }
    (format_version, items)
}

fn kind_str(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    if let Some(obj) = v.as_object()
        && let Some(k) = obj.keys().next()
    {
        return k.clone();
    }
    "item".into()
}

pub fn fetch_crate(
    rt: &crate::plugin::Runtime,
    name: &str,
    get: impl Fn(&str) -> Result<(u16, Vec<u8>)>,
) -> Result<String> {
    let _ = ingest_llms(rt);
    let text = std::fs::read_to_string(lock_path()).unwrap_or_default();
    let Some((n, ver)) = resolve_lock(&text, name) else {
        return Ok(miss_lock(name));
    };
    let url = format!("https://docs.rs/crate/{n}/{ver}/json.zst");
    let (status, bytes) = get(&url)?;
    if status != 200 {
        return Ok(format!("docs.rs returned {status}"));
    }
    let sha = crate::store::hex_sha256(&bytes);
    let raw = zstd::decode_all(bytes.as_slice()).context("zstd decode of rustdoc JSON")?;
    let json: Value = serde_json::from_slice(&raw).context("rustdoc JSON")?;
    let (format_version, items) = index_rustdoc(&json);
    let dir = rt.config.home.join("docs").join(&n).join(&ver);
    std::fs::create_dir_all(&dir)?;
    let zst_path = dir.join("rustdoc.json.zst");
    let sha_path = dir.join("rustdoc.json.zst.sha256");
    std::fs::write(&zst_path, &bytes)?;
    std::fs::write(&sha_path, format!("{sha}\n"))?;
    let now = i64::try_from(crate::log::now()).unwrap_or(0);
    let n_items = rt
        .store
        .replace_doc_crate(&n, &ver, &sha, format_version, now, &items)?;
    Ok(format!("cached {n} {ver} ({n_items} items)"))
}

pub fn http_get(url: &str) -> Result<(u16, Vec<u8>)> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(format!("rtok/{}", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .context("docs fetch client")?;
    let resp = client.get(url).send().context("docs.rs GET")?;
    let status = resp.status().as_u16();
    let bytes = resp.bytes().map(|b| b.to_vec()).unwrap_or_default();
    Ok((status, bytes))
}

pub fn run_fetch(cfg: &crate::config::Config, name: &str) -> Result<String> {
    let rt = crate::plugin::Runtime::open(cfg.clone(), "docs")?;
    fetch_crate(&rt, name, http_get)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::Runtime;
    use std::sync::Mutex;

    const LOCK: &str = r#"
version = 3

[[package]]
name = "serde"
version = "1.0.210"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
"#;

    static CWD: Mutex<()> = Mutex::new(());

    fn runtime(tag: &str) -> Runtime {
        let mut rt = Runtime::in_memory(tag).unwrap();
        rt.config.plugins.docs.enabled = true;
        rt
    }

    /// A fresh directory holding `files`.
    fn temp_workspace(files: &[(&str, &str)]) -> PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rtok-docs-ws-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, body) in files {
            std::fs::write(dir.join(name), body).unwrap();
        }
        dir
    }

    /// `f` with `dir` as the process cwd, which is what scopes the plugin to a workspace.
    fn in_dir<T>(dir: &std::path::Path, f: impl FnOnce() -> T) -> T {
        let _guard = CWD.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir).unwrap();
        struct Restore(PathBuf);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = std::env::set_current_dir(&self.0);
            }
        }
        let _restore = Restore(prev);
        f()
    }

    fn with_lock_cwd<T>(lock_text: &str, f: impl FnOnce() -> T) -> T {
        let dir = temp_workspace(&[("Cargo.lock", lock_text)]);
        let out = in_dir(&dir, f);
        let _ = std::fs::remove_dir_all(&dir);
        out
    }

    #[test]
    fn resolve_lock_prints_serde_version() {
        let (name, ver) = resolve_lock(LOCK, "serde").expect("serde in lock");
        assert_eq!((name.as_str(), ver.as_str()), ("serde", "1.0.210"));
        assert!(resolve_lock(LOCK, "nope").is_none());
    }

    /// Audit 2026-10-19: a lock entry's version became a path under the docs cache unchecked.
    #[test]
    fn resolve_lock_refuses_path_like_names_and_versions() {
        for (n, v) in [
            ("evil", "../../../tmp/x"),
            ("evil", "/tmp/x"),
            ("evil", "1.0/../x"),
        ] {
            let lock = format!("version = 4\n[[package]]\nname = \"{n}\"\nversion = \"{v}\"\n");
            assert!(resolve_lock(&lock, n).is_none(), "{v}");
        }
        let ok = "version = 4\n[[package]]\nname = \"a_b-c\"\nversion = \"1.0.0-rc.1+g1\"\n";
        assert!(resolve_lock(ok, "a_b-c").is_some());
    }

    #[test]
    fn docs_resolve_prints_serde_version_from_fixture_lock() {
        let rt = runtime("docs-lock");
        let out = with_lock_cwd(LOCK, || docs_resolve(&rt, "serde").unwrap());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["name"], "serde");
        assert_eq!(v["version"], "1.0.210");
        assert_eq!(v["cached"], false);
    }

    #[test]
    fn docs_resolve_unknown_name_is_a_miss_line() {
        let rt = runtime("docs-miss");
        let out = with_lock_cwd(LOCK, || {
            docs_resolve(&rt, "definitely-missing-crate").unwrap()
        });
        assert!(out.starts_with("not in Cargo.lock:"), "{out}");
    }

    #[test]
    fn query_deserialize_ranks_matching_path_first_under_budget() {
        let rt = runtime("docs-q");
        rt.store
            .replace_doc_crate(
                "serde",
                "1.0.210",
                "sha",
                1,
                0,
                &[
                    (
                        "serde::Serialize".into(),
                        "trait".into(),
                        "Serialize a data structure from Rust to a data format.".into(),
                    ),
                    (
                        "serde::Deserialize".into(),
                        "trait".into(),
                        "Deserialize a data structure from a data format into Rust.".into(),
                    ),
                ],
            )
            .unwrap();
        let mut rt = rt;
        rt.config.plugins.docs.max_tokens = 800;
        let out = docs_query(&rt, "serde", Some("1.0.210"), "Deserialize").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let arr = v.as_array().expect(&out);
        assert!(!arr.is_empty(), "{out}");
        let path = arr[0]["path"].as_str().unwrap_or("");
        assert!(
            path.contains("Deserialize"),
            "first hit should be Deserialize: {out}"
        );
        let est = rt.estimate(&out, Class::Prose);
        assert!(est <= 800, "reply {est} tokens: {out}");
        let rows = rt.store.list_measurements("docs").unwrap();
        assert!(
            rows.iter().any(|r| r.kind == "query"),
            "query records a Measurement"
        );
    }

    fn ids_of(out: &str) -> Vec<i64> {
        serde_json::from_str::<Vec<Value>>(out)
            .unwrap_or_else(|_| panic!("{out}"))
            .iter()
            .map(|h| h["id"].as_i64().unwrap())
            .collect()
    }

    /// T549: two workspaces sharing one store each see only their own `llms.txt`; a workspace
    /// without one sees nothing, and an old unscoped `("llms", "local")` row is nobody's.
    #[test]
    fn local_llms_docs_are_scoped_to_their_workspace() {
        let rt = runtime("docs-ws");
        let a = temp_workspace(&[("llms.txt", "# Alpha\nMARKER_ALPHA body\n")]);
        let b = temp_workspace(&[]);
        rt.store
            .replace_doc_crate(
                "llms",
                "local",
                "old",
                0,
                0,
                &[("Legacy".into(), "llms".into(), "MARKER_LEGACY".into())],
            )
            .unwrap();
        let query =
            |dir: &PathBuf, q: &str| in_dir(dir, || docs_query(&rt, "llms", None, q).unwrap());
        let cached = |dir: &PathBuf| {
            let out = in_dir(dir, || docs_resolve(&rt, "llms").unwrap());
            serde_json::from_str::<Value>(&out).unwrap()["cached"]
                .as_bool()
                .unwrap()
        };

        assert_eq!(ids_of(&query(&a, "MARKER_ALPHA")).len(), 1);
        assert!(cached(&a));
        assert!(!cached(&b), "B has no llms file");
        assert_eq!(
            query(&b, "MARKER_ALPHA"),
            "not cached: rtok docs fetch llms"
        );
        assert_eq!(query(&a, "MARKER_LEGACY"), "[]");

        std::fs::write(b.join("llms.txt"), "# Beta\nMARKER_BETA body\n").unwrap();
        assert_eq!(query(&b, "MARKER_ALPHA"), "[]");
        assert_eq!(ids_of(&query(&b, "MARKER_BETA")).len(), 1);
        assert_eq!(
            ids_of(&query(&a, "MARKER_ALPHA")).len(),
            1,
            "B did not overwrite A"
        );
        assert_eq!(query(&a, "MARKER_BETA"), "[]");

        std::fs::remove_file(a.join("llms.txt")).unwrap();
        assert!(!cached(&a), "a deleted source is no longer served");
        std::fs::write(a.join("llms.txt"), "# Alpha\nMARKER_AGAIN body\n").unwrap();
        assert_eq!(ids_of(&query(&a, "MARKER_AGAIN")).len(), 1);
        for d in [a, b] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// T550: re-ingesting an unchanged `llms.txt` (every resolve/query/fetch does) keeps
    /// the ids already handed out, even after another crate was cached in between.
    #[test]
    fn unchanged_local_docs_keep_their_ids_and_edits_refresh() {
        let rt = runtime("docs-ids");
        let ws = temp_workspace(&[("llms.txt", "# Guide\nMARKER_ONE body\n")]);
        let first = ids_of(&in_dir(&ws, || {
            docs_query(&rt, "llms", None, "MARKER_ONE").unwrap()
        }));
        assert_eq!(first.len(), 1);
        rt.store
            .replace_doc_crate(
                "serde",
                "1.0.210",
                "sha",
                1,
                0,
                &[("serde::X".into(), "struct".into(), "other".into())],
            )
            .unwrap();
        in_dir(&ws, || docs_resolve(&rt, "llms").unwrap());
        let again = ids_of(&in_dir(&ws, || {
            docs_query(&rt, "llms", None, "MARKER_ONE").unwrap()
        }));
        assert_eq!(again, first, "ids survive an unrelated read");
        let body: Value =
            serde_json::from_str(&docs_get(&rt, i32::try_from(first[0]).unwrap()).unwrap())
                .unwrap();
        assert!(body["docs"].as_str().unwrap().contains("MARKER_ONE"));

        std::fs::write(ws.join("llms.txt"), "# Guide\nMARKER_TWO body\n").unwrap();
        let q = |m: &str| in_dir(&ws, || docs_query(&rt, "llms", None, m).unwrap());
        assert_eq!(q("MARKER_ONE"), "[]");
        assert_eq!(ids_of(&q("MARKER_TWO")).len(), 1, "an edit is searchable");
        let _ = std::fs::remove_dir_all(ws);
    }

    /// T551: the saving's before side is the full text of everything the query considered,
    /// in bytes and in tokens, whatever the snippet length or the output budget keeps.
    #[test]
    fn query_measurement_uses_one_full_body_baseline() {
        let bodies: Vec<String> = (0..3)
            .map(|i| {
                format!(
                    "Deserialize variant {i}. {}",
                    "long explanation text ".repeat(120)
                )
            })
            .collect();
        let seed = |rt: &Runtime| {
            let items: Vec<_> = bodies
                .iter()
                .enumerate()
                .map(|(i, d)| (format!("serde::D{i}"), "trait".to_string(), d.clone()))
                .collect();
            rt.store
                .replace_doc_crate("serde", "1.0.210", "sha", 1, 0, &items)
                .unwrap();
        };
        let all = bodies.join("\n");
        for max_tokens in [800, 60] {
            let mut rt = runtime("docs-meas");
            seed(&rt);
            rt.config.plugins.docs.snippet_chars = 30;
            rt.config.plugins.docs.max_tokens = max_tokens;
            let out = docs_query(&rt, "serde", Some("1.0.210"), "Deserialize").unwrap();
            assert!(!ids_of(&out).is_empty(), "{out}");
            let rows = rt.store.list_measurements("docs").unwrap();
            assert_eq!(rows.len(), 1, "one row per returned query");
            let r = &rows[0];
            assert_eq!(u64::try_from(r.before_bytes).unwrap(), all.len() as u64 - 2);
            assert_eq!(
                u64::try_from(r.est_before).unwrap(),
                u64::from(rt.estimate(&all, Class::Prose))
            );
            assert_eq!(u64::try_from(r.after_bytes).unwrap(), out.len() as u64);
            assert_eq!(
                u64::try_from(r.est_after).unwrap(),
                u64::from(rt.estimate(&out, Class::Prose))
            );
            assert!(r.est_before > r.est_after);
        }
        let rt = runtime("docs-meas-none");
        seed(&rt);
        let out = docs_query(&rt, "serde", Some("1.0.210"), "absentword").unwrap();
        assert_eq!(out, "[]");
        assert!(rt.store.list_measurements("docs").unwrap().is_empty());
    }

    #[test]
    fn query_uncached_prints_not_cached_without_http() {
        let rt = runtime("docs-empty");
        let out = docs_query(&rt, "serde", Some("1.0.210"), "Deserialize").unwrap();
        assert_eq!(out, "not cached: rtok docs fetch serde");
    }

    #[test]
    fn mcp_surface_stays_within_sixty_description_tokens() {
        let cx = Runtime::in_memory("docs-surface").unwrap();
        let total: i64 = DocsPlugin
            .mcp_tools()
            .iter()
            .map(|t| i64::from(cx.estimate(t.description, Class::Prose)))
            .sum();
        assert!(total <= 60, "docs tool surface is {total} tokens");
    }

    #[test]
    fn fetch_non_200_leaves_cache_untouched() {
        let rt = runtime("docs-404");
        rt.store
            .replace_doc_crate(
                "serde",
                "1.0.210",
                "old",
                1,
                0,
                &[("serde::Keep".into(), "struct".into(), "stay".into())],
            )
            .unwrap();
        let out = with_lock_cwd(LOCK, || {
            fetch_crate(&rt, "serde", |_| Ok((404, b"nope".to_vec()))).unwrap()
        });
        assert!(out.contains("404"), "{out}");
        let hits = rt
            .store
            .search_doc_items("serde", "1.0.210", "stay", 5, 40)
            .unwrap();
        assert_eq!(hits.len(), 1, "old cache kept");
    }

    #[test]
    fn llms_txt_splits_on_hash_headings() {
        let sections = split_llms("# One\nbody a\n# Two\nbody b\n");
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].0, "One");
        assert_eq!(sections[0].2, "body a");
        assert_eq!(sections[1].0, "Two");
    }
}
