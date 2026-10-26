// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Tags vs LSP graph backends: context tokens + latency/throughput.
//!
//! Measures the text each backend returns into the agent context (tool output),
//! estimated with the project's `tokens::estimate` / `Class::Code` rates — the
//! same estimator `cap()` and Measurement rows use. Does not invent a tokenizer.
//!
//! Run (release recommended for latency numbers):
//! ```text
//! mise exec -- cargo test --release --test graph_backend_bench -- --ignored --nocapture --test-threads=1
//! ```
//!
//! LSP arm skips when `rust-analyzer --version` is not on PATH.
//! Live model spend (`RTOK_BENCH_LIVE` / `claude -p`) is separate — see `rtok bench`
//! suite `graph` in `src/bench.rs`; this file is offline tool-output measurement.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rtok::config::Config;
use rtok::plugin::{Ctx, Runtime};
use rtok::plugins::graph::{Filter, callers, explore, impact, index, outline, symbol};
use rtok::tokens::{Class, estimate};

const WARMUP: usize = 2;
const SAMPLES: usize = 8;

struct Row {
    backend: &'static str,
    tool: &'static str,
    query: String,
    bytes: usize,
    chars: usize,
    tokens: u32,
    samples_ms: Vec<f64>,
}

impl Row {
    fn p50_ms(&self) -> f64 {
        pct(&self.samples_ms, 50.0)
    }
    fn p95_ms(&self) -> f64 {
        pct(&self.samples_ms, 95.0)
    }
    fn mean_ms(&self) -> f64 {
        if self.samples_ms.is_empty() {
            return 0.0;
        }
        self.samples_ms.iter().sum::<f64>() / self.samples_ms.len() as f64
    }
    fn throughput_qps(&self) -> f64 {
        let m = self.mean_ms();
        if m <= 0.0 { 0.0 } else { 1000.0 / m }
    }
}

fn pct(sorted_src: &[f64], p: f64) -> f64 {
    if sorted_src.is_empty() {
        return 0.0;
    }
    let mut v = sorted_src.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let rank = ((p / 100.0) * (v.len() as f64 - 1.0)).round() as usize;
    v[rank.min(v.len() - 1)]
}

fn rust_analyzer_on_path() -> bool {
    Command::new("rust-analyzer")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn open(tag: &str, backend: &str) -> (Runtime, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rtok-backend-bench-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let mut cfg = Config::default();
    cfg.core.db_path = dir.join("rtok.db");
    cfg.core.archive_dir = dir.join("archive");
    cfg.plugins.graph.backend = backend.into();
    // Keep cap high enough that small fixtures are not truncated mid-comparison.
    cfg.plugins.graph.max_tokens = 50_000;
    cfg.plugins.graph.body_lines = 40;
    cfg.plugins.read.allow_paths = vec![dir.clone()];
    (Runtime::open(cfg, tag).unwrap(), dir)
}

/// Multi-file Rust crate: enough symbols for symbol/callers/impact/outline/explore,
/// plus a type-position use so LSP recall differs from tags (Gate P30 fixture shape).
fn write_fixture(home: &Path) -> PathBuf {
    let root = home.join("crate");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"backend_bench\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        root.join("src/lib.rs"),
        r#"mod sink;
mod callers;
mod typed;

pub use sink::sink;
pub use callers::{alpha, beta, gamma};
pub use typed::{OnlyTyped, user};
"#,
    )
    .unwrap();
    fs::write(
        root.join("src/sink.rs"),
        r#"/// Shared leaf.
pub fn sink() {}
"#,
    )
    .unwrap();
    fs::write(
        root.join("src/callers.rs"),
        r#"use crate::sink::sink;

pub fn alpha() {
    sink();
}

pub fn beta() {
    sink();
}

pub fn gamma() {
    alpha();
}
"#,
    )
    .unwrap();
    fs::write(
        root.join("src/typed.rs"),
        r#"pub struct OnlyTyped;

pub fn user(_t: Vec<OnlyTyped>) {}
"#,
    )
    .unwrap();
    root
}

fn repo_source_bytes(root: &Path) -> (usize, usize, u32) {
    let mut blob = String::new();
    for rel in [
        "src/lib.rs",
        "src/sink.rs",
        "src/callers.rs",
        "src/typed.rs",
    ] {
        let p = root.join(rel);
        let s = fs::read_to_string(&p).unwrap_or_default();
        blob.push_str(&format!("// ---- {rel} ----\n"));
        blob.push_str(&s);
        if !blob.ends_with('\n') {
            blob.push('\n');
        }
    }
    let rates = Config::default().estimator;
    (
        blob.len(),
        blob.chars().count(),
        estimate(&blob, Class::Code, &rates),
    )
}

fn measure<F>(
    backend: &'static str,
    tool: &'static str,
    query: &str,
    rates: &rtok::config::Estimator,
    mut f: F,
) -> Row
where
    F: FnMut() -> String,
{
    for _ in 0..WARMUP {
        let _ = f();
    }
    let mut samples_ms = Vec::with_capacity(SAMPLES);
    let mut last = String::new();
    for _ in 0..SAMPLES {
        let t0 = Instant::now();
        last = f();
        samples_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    Row {
        backend,
        tool,
        query: query.to_string(),
        bytes: last.len(),
        chars: last.chars().count(),
        tokens: estimate(&last, Class::Code, rates),
        samples_ms,
    }
}

fn run_backend(backend: &'static str, tag: &str) -> (Vec<Row>, PathBuf, Duration) {
    let (cx, home) = open(tag, backend);
    let root = write_fixture(&home);
    let rates = cx.config.estimator.clone();
    let ctx = Ctx::new(&cx);

    let index_ms = if backend == "tags" {
        let t0 = Instant::now();
        index::run(&ctx, &root, false).unwrap();
        t0.elapsed()
    } else {
        Duration::ZERO
    };

    let rows = vec![
        measure(backend, "symbol", "sink", &rates, || {
            symbol(&Ctx::new(&cx), &root, "sink").unwrap()
        }),
        measure(backend, "callers", "sink", &rates, || {
            callers(&Ctx::new(&cx), &root, "sink").unwrap()
        }),
        measure(backend, "impact", "sink depth=2", &rates, || {
            impact(&Ctx::new(&cx), &root, "sink", 2, None).unwrap()
        }),
        measure(backend, "outline", "src/callers.rs", &rates, || {
            outline(
                &Ctx::new(&cx),
                &root.join("src/callers.rs").to_string_lossy(),
            )
            .unwrap()
        }),
        measure(backend, "explore", "who calls sink", &rates, || {
            explore(&Ctx::new(&cx), &root, "who calls sink", &Filter::none()).unwrap()
        }),
        measure(backend, "callers", "OnlyTyped", &rates, || {
            callers(&Ctx::new(&cx), &root, "OnlyTyped").unwrap()
        }),
        measure(backend, "symbol", "alpha", &rates, || {
            symbol(&Ctx::new(&cx), &root, "alpha").unwrap()
        }),
    ];

    (rows, home, index_ms)
}

fn print_rows(rows: &[Row]) {
    for row in rows {
        eprintln!(
            "{:<6} {:<8} {:<16} {:>7} {:>7} {:>7} {:>8.2} {:>8.2} {:>8.2} {:>8.2}",
            row.backend,
            row.tool,
            row.query,
            row.bytes,
            row.chars,
            row.tokens,
            row.mean_ms(),
            row.p50_ms(),
            row.p95_ms(),
            row.throughput_qps()
        );
    }
}

/// Always-on smoke: tags path indexes and returns a capped symbol under the estimator.
#[test]
fn tags_backend_smoke_tokens_and_latency() {
    let (cx, home) = open("smoke-tags", "tags");
    let root = write_fixture(&home);
    let ctx = Ctx::new(&cx);
    index::run(&ctx, &root, false).unwrap();
    let t0 = Instant::now();
    let out = symbol(&ctx, &root, "sink").unwrap();
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    let tok = cx.estimate(&out, Class::Code);
    assert!(out.contains("sink"), "{out}");
    assert!(tok > 0, "expected non-zero estimated tokens");
    eprintln!(
        "smoke tags symbol sink: {tok} est_tokens chars={} {ms:.3} ms",
        out.chars().count()
    );
    let _ = fs::remove_dir_all(&home);
}

/// Full tags vs LSP comparison. Ignored so CI stays offline/fast; run locally with --ignored.
#[ignore]
#[test]
fn tags_vs_lsp_tokens_and_latency() {
    let rates = Config::default().estimator;
    eprintln!("estimator Class::Code chars_per_token = {}", rates.code);
    eprintln!("warmup={WARMUP} samples={SAMPLES} (mean/p50/p95 over samples after warmup)");

    let (tags_rows, tags_home, tags_index) = run_backend("tags", "tags");
    let (base_bytes, base_chars, base_tokens) = repo_source_bytes(&tags_home.join("crate"));
    eprintln!(
        "baseline full-repo source dump: bytes={base_bytes} chars={base_chars} est_tokens={base_tokens}"
    );
    eprintln!("tags cold index: {tags_index:?}");

    let lsp_rows = if rust_analyzer_on_path() {
        let (rows, home, _) = run_backend("lsp", "lsp");
        let _ = fs::remove_dir_all(&home);
        Some(rows)
    } else {
        eprintln!("LSP arm SKIPPED: rust-analyzer not on PATH");
        None
    };

    eprintln!();
    eprintln!(
        "{:<6} {:<8} {:<16} {:>7} {:>7} {:>7} {:>8} {:>8} {:>8} {:>8}",
        "back", "tool", "query", "bytes", "chars", "tokens", "mean_ms", "p50_ms", "p95_ms", "qps"
    );
    print_rows(&tags_rows);
    if let Some(ref rows) = lsp_rows {
        print_rows(rows);
    }

    let sum_tok = |rows: &[Row]| rows.iter().map(|r| r.tokens as u64).sum::<u64>();
    let sum_ms = |rows: &[Row]| rows.iter().map(|r| r.mean_ms()).sum::<f64>();
    let tags_tok = sum_tok(&tags_rows);
    let tags_ms = sum_ms(&tags_rows);
    let naive = base_tokens as u64 * tags_rows.len() as u64;
    let tags_vs_repo = if naive == 0 {
        0.0
    } else {
        (1.0 - tags_tok as f64 / naive as f64) * 100.0
    };

    eprintln!();
    eprintln!("=== SUMMARY ===");
    eprintln!(
        "tags: sum_est_tokens={tags_tok} ({} queries) sum_mean_ms={tags_ms:.2} vs naive_full_repo_xN={naive} est_tokens → {tags_vs_repo:.1}% savings",
        tags_rows.len()
    );

    if let Some(ref lsp) = lsp_rows {
        let lsp_tok = sum_tok(lsp);
        let lsp_ms = sum_ms(lsp);
        let naive_l = base_tokens as u64 * lsp.len() as u64;
        let lsp_vs_repo = if naive_l == 0 {
            0.0
        } else {
            (1.0 - lsp_tok as f64 / naive_l as f64) * 100.0
        };
        let tok_delta = lsp_tok as i64 - tags_tok as i64;
        let tok_pct = if tags_tok == 0 {
            0.0
        } else {
            (tok_delta as f64 / tags_tok as f64) * 100.0
        };
        let speedup = if tags_ms <= 0.0 {
            0.0
        } else {
            lsp_ms / tags_ms
        };
        eprintln!(
            "lsp:  sum_est_tokens={lsp_tok} ({} queries) sum_mean_ms={lsp_ms:.2} vs naive_full_repo_xN={naive_l} est_tokens → {lsp_vs_repo:.1}% savings",
            lsp.len()
        );
        eprintln!(
            "delta lsp-tags tokens: {tok_delta:+} ({tok_pct:+.1}% vs tags); latency ratio lsp/tags mean sums: {speedup:.2}x"
        );

        eprintln!();
        eprintln!(
            "{:<8} {:<16} {:>8} {:>8} {:>8} {:>10} {:>10}",
            "tool", "query", "tok_tags", "tok_lsp", "tok_sav%", "ms_tags", "ms_lsp"
        );
        for (t, l) in tags_rows.iter().zip(lsp.iter()) {
            let sav = if t.tokens == 0 {
                0.0
            } else {
                (1.0 - l.tokens as f64 / t.tokens as f64) * 100.0
            };
            eprintln!(
                "{:<8} {:<16} {:>8} {:>8} {:>8.1} {:>10.2} {:>10.2}",
                t.tool,
                t.query,
                t.tokens,
                l.tokens,
                sav,
                t.mean_ms(),
                l.mean_ms()
            );
        }
    }

    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bench/results");
    let _ = fs::create_dir_all(&out_dir);
    let payload = serde_json::json!({
        "estimator_code_chars_per_token": rates.code,
        "baseline_repo_est_tokens": base_tokens,
        "baseline_repo_bytes": base_bytes,
        "baseline_repo_chars": base_chars,
        "tags_index_ms": tags_index.as_secs_f64() * 1000.0,
        "tags": tags_rows.iter().map(|r| serde_json::json!({
            "tool": r.tool,
            "query": r.query,
            "bytes": r.bytes,
            "chars": r.chars,
            "est_tokens": r.tokens,
            "mean_ms": r.mean_ms(),
            "p50_ms": r.p50_ms(),
            "p95_ms": r.p95_ms(),
            "qps": r.throughput_qps(),
        })).collect::<Vec<_>>(),
        "lsp": lsp_rows.as_ref().map(|rows| rows.iter().map(|r| serde_json::json!({
            "tool": r.tool,
            "query": r.query,
            "bytes": r.bytes,
            "chars": r.chars,
            "est_tokens": r.tokens,
            "mean_ms": r.mean_ms(),
            "p50_ms": r.p50_ms(),
            "p95_ms": r.p95_ms(),
            "qps": r.throughput_qps(),
        })).collect::<Vec<_>>()),
        "live_model": "not run in this file; set RTOK_BENCH_LIVE=1 and `rtok bench` suite=graph for billed Claude usage",
    });
    let path = out_dir.join("graph-tags-vs-lsp.json");
    fs::write(
        &path,
        serde_json::to_string_pretty(&payload).unwrap() + "\n",
    )
    .unwrap();
    eprintln!("wrote {}", path.display());

    let _ = fs::remove_dir_all(&tags_home);
}
