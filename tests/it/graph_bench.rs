// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T8.14: Gate P8c numbers (historical). Ignored; run in release:
//! `mise exec -- cargo test --release --test graph_bench -- --ignored --nocapture --test-threads=1`
//! Ladybug (`graph-lbug`) and Grafeo (`graph-grafeo`) backends were removed (P39); SQLite only.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use rtok::plugin::{Ctx, Runtime};
use rtok::plugins::graph::{callers, impact, index, symbol};
use rtok::store::Store;

use crate::it::common;

#[ignore]
#[test]
fn p8c_numbers() {
    if cfg!(debug_assertions) {
        eprintln!("skip: T8.14 is `cargo test --release --test graph_bench -- --ignored`");
        return;
    }
    let backend = "sqlite";
    eprintln!("P8c backend={backend}");

    let (cx, repo) = home("p8c-3k");
    for i in 0..3000u32 {
        let a = (i + 1) % 3000;
        let b = (i + 2) % 3000;
        std::fs::write(
            repo.join(format!("f{i}.rs")),
            format!("fn f{i}() {{\n    f{a}();\n    f{b}();\n}}\n"),
        )
        .unwrap();
    }
    let t = Instant::now();
    let cold = index::run(&Ctx::new(&cx), &repo, false).unwrap();
    let cold_ms = t.elapsed();
    eprintln!(
        "cold_index files={} rows={} read={} {cold_ms:?}",
        cold.indexed, cold.inserted, cold.read
    );

    let warm = |label: &str, f: fn(&Ctx, &Path, &str) -> anyhow::Result<String>| {
        let t = Instant::now();
        f(&Ctx::new(&cx), &repo, "f0").unwrap();
        let d = t.elapsed();
        eprintln!("warm_{label} {d:?}");
        d
    };
    let ws = warm("symbol", symbol);
    let wc = warm("callers", callers);
    let t = Instant::now();
    impact(&Ctx::new(&cx), &repo, "f0", 2, None).unwrap();
    let wi = t.elapsed();
    eprintln!("warm_impact2 {wi:?}");

    let db = cx.config.core.db_path.clone();
    let store_dir = db.parent().unwrap().to_path_buf();
    drop(cx);
    eprintln!("rtok_db_bytes {}", bytes(&db));
    eprintln!("graph_lbdb_bytes {}", bytes(&store_dir.join("graph.lbdb")));
    if let Ok(rd) = std::fs::read_dir(&store_dir) {
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().is_some_and(|x| x == "rs") {
                continue;
            }
            eprintln!(
                "store_file {} {}",
                path.file_name().unwrap().to_string_lossy(),
                bytes(&path)
            );
        }
    }

    let (cx2, fan) = home("p8c-fan");
    write_fanout(&fan);
    index::run(&Ctx::new(&cx2), &fan, false).unwrap();
    let key = index::canon(&fan);
    let t = Instant::now();
    let rows = cx2.store.symbol_impact(&key, "sink", 4).unwrap();
    let q = t.elapsed();
    eprintln!("impact4_query rows={} {q:?}", rows.len());
    let t = Instant::now();
    let bfs = impact_bfs(&cx2.store, &key, "sink", 4).unwrap();
    let b = t.elapsed();
    eprintln!("impact4_bfs rows={} {b:?}", bfs.len());
    assert_eq!(rows.len(), bfs.len(), "query vs BFS size");

    hook_p95();
    eprintln!(
        "P8c summary backend={backend} cold={cold_ms:?} symbol={ws:?} callers={wc:?} impact2={wi:?} impact4_query={q:?} impact4_bfs={b:?}"
    );
    let _ = std::fs::remove_dir_all(&repo);
    let _ = std::fs::remove_dir_all(&fan);
}

/// T35.4 Check: one edited file in a 3 000-file tree reads one file in < 10 ms (release).
#[ignore]
#[test]
fn p8c_one_edit_reads_one_file_under_10ms() {
    if cfg!(debug_assertions) {
        eprintln!("skip: run `cargo test --release --test graph_bench -- --ignored`");
        return;
    }
    use std::time::Duration;
    let (cx, repo) = home("p8c-changed");
    for i in 0..3000u32 {
        std::fs::write(
            repo.join(format!("f{i}.rs")),
            format!(
                "fn f{i}() {{}}
"
            ),
        )
        .unwrap();
    }
    index::run(&Ctx::new(&cx), &repo, false).unwrap();
    std::fs::write(
        repo.join("f0.rs"),
        "fn f0() {}
fn touched() {}
",
    )
    .unwrap();
    let changed = HashSet::from([repo.join("f0.rs")]);
    let t = Instant::now();
    let r = index::run_changed(&Ctx::new(&cx), &repo, &changed).unwrap();
    let elapsed = t.elapsed();
    eprintln!("run_changed read={} elapsed={elapsed:?}", r.read);
    assert_eq!(r.read, 1, "one edited file");
    assert!(
        elapsed < Duration::from_millis(10),
        "run_changed took {elapsed:?}, want < 10 ms"
    );
    let _ = std::fs::remove_dir_all(&repo);
}

fn write_fanout(dir: &Path) {
    let mut src = String::from("fn sink() {}\n");
    for i in 0..10 {
        src.push_str(&format!("fn a{i}() {{ sink(); }}\n"));
    }
    for i in 0..100 {
        src.push_str(&format!("fn b{i}() {{ a{}(); }}\n", i / 10));
    }
    for i in 0..1000 {
        src.push_str(&format!("fn c{i}() {{ b{}(); }}\n", i / 10));
    }
    for i in 0..10_000 {
        src.push_str(&format!("fn d{i}() {{ c{}(); }}\n", i / 10));
    }
    std::fs::write(dir.join("fan.rs"), src).unwrap();
}

fn impact_bfs(
    store: &Store,
    root: &str,
    name: &str,
    depth: u32,
) -> anyhow::Result<Vec<(u32, String, String)>> {
    let mut seen: HashSet<String> = HashSet::from([name.to_string()]);
    let mut frontier = vec![name.to_string()];
    let mut out = Vec::new();
    for d in 1..=depth.clamp(1, 4) {
        let mut next = Vec::new();
        for from in &frontier {
            for (path, scope, ..) in store.symbol_ref_groups(root, from)? {
                if scope.is_empty() {
                    out.push((d, path, String::new()));
                } else if seen.insert(scope.clone()) {
                    out.push((d, path, scope.clone()));
                    next.push(scope);
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    Ok(out)
}

fn home(name: &str) -> (Runtime, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("rtok-bench-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let c = rtok::testutil::config_in(&dir);
    (Runtime::open(c, name).unwrap(), dir)
}

fn hook_p95() {
    const N: usize = 100;
    let bin = crate::it::common::bin_path("rtok");
    let tmp = std::env::temp_dir().join(format!("rtok-bench-hook-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let fixture: &[u8] = include_bytes!("../fixtures/hooks/post_tool.json");
    let spawn = || {
        let mut child = Command::new(bin)
            .args(["hook", "PostToolUse"])
            .env("RTOK_HOME", &tmp)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(fixture).unwrap();
        child.wait_with_output().unwrap()
    };
    assert!(spawn().status.success());
    let mut samples = Vec::with_capacity(N);
    for _ in 0..N {
        let t = Instant::now();
        let out = spawn();
        samples.push(t.elapsed());
        assert!(out.status.success());
    }
    samples.sort();
    let p95 = common::p95(&samples);
    eprintln!(
        "hook_PostToolUse n={N} p50 {:?} p95 {p95:?} max {:?}",
        samples[N / 2],
        samples[N - 1]
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

fn bytes(p: &Path) -> u64 {
    if !p.exists() {
        return 0;
    }
    if p.is_file() {
        return p.metadata().map(|m| m.len()).unwrap_or(0);
    }
    let mut n = 0u64;
    let mut stack = vec![p.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(m) = e.metadata() {
                n += m.len();
            }
        }
    }
    n
}
