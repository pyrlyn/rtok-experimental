// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T69.3: planted / drifted / superseded recall bench. No network, no LLM.
//!
//! Metrics: `mem_search` hit rate in top-`search_limit` for a query built from the fact's
//! own words; superseded (retired) ids returned; SessionStart recall bytes vs injecting every
//! live body. FTS5 default and P29 hybrid (`embed.enabled`). `half_life_days` is N/A — T69.2
//! shipped no scorer. Never cite graymatter's 83 %. Re-run:
//! `cargo test --test memory_bench -- --nocapture`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::PathBuf;

use rtok::plugin::{Ctx, Plugin, Runtime, SessionStart};
use rtok::plugins::memory::{MemoryPlugin, mem_revise, mem_save, mem_search};

const NS: [u32; 4] = [1, 10, 30, 100];
const K: u32 = 6;
const FACTS: usize = 20;
const REVISE: usize = 5;
const SEED: u64 = 6_903;
const FILLER: &str = "cache router widget latency replica shard mutex queue batch flush \
     metric histogram gauge ticker watchdog sidecar ingress egress namespace volume \
     snapshot restore compaction vacuum journal penguin otter";

/// Floors (hits / 20) locked from this generator; a drop on any row fails the test.
const FTS_FLOOR: [u32; 4] = [20, 20, 20, 20];
const HYBRID_FLOOR: [u32; 4] = [20, 20, 20, 20];

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
        (self.0 >> 32) as u32
    }
}

fn filler(rng: &mut Rng, session: u32, k: u32) -> String {
    let words: Vec<&str> = FILLER.split_whitespace().collect();
    let mut out = Vec::with_capacity(80);
    for _ in 0..80 {
        out.push(words[rng.next() as usize % words.len()]);
    }
    format!("session {session} note {k} {}", out.join(" "))
}

fn nonce(i: usize) -> String {
    format!("zorchtoken{i:02}")
}

fn fact(i: usize, revised: bool) -> (String, String) {
    let token = nonce(i);
    if revised {
        (
            format!("fact-{i}-revised"),
            format!(
                "Correction: {token} now uses rotated credentials. The previous original signing story is retired. Downstream must read this revised note."
            ),
        )
    } else {
        (
            format!("fact-{i}"),
            format!(
                "Planted operator fact {token} documents that cluster {token} must pin the original signing key before rotating sessions."
            ),
        )
    }
}

fn query_of(body: &str) -> String {
    body.split_whitespace()
        .filter(|w| w.len() > 4 && w.chars().all(|c| c.is_ascii_alphanumeric()))
        .take(8)
        .collect::<Vec<_>>()
        .join(" ")
}

fn hits_id(cx: &Runtime, query: &str, limit: u32, id: i32) -> bool {
    mem_search(cx, query, limit)
        .unwrap()
        .iter()
        .any(|h| h.id == id)
}

fn full_bytes(cx: &Runtime, project: &str) -> u64 {
    cx.store
        .list_note_titles(Some(project), u32::MAX)
        .unwrap()
        .into_iter()
        .map(|(id, _)| {
            cx.store
                .get_note_body(id)
                .ok()
                .flatten()
                .map(|b| b.len() as u64)
                .unwrap_or(0)
        })
        .sum()
}

fn seed(n: u32) -> (Runtime, Vec<(i32, String)>, Vec<i32>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rtok-mbench-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let proj = dir.join("benchproj");
    std::fs::create_dir_all(proj.join(".git")).unwrap();
    let mut cx = Runtime::in_memory(format!("mbench-{n}")).unwrap();
    cx.config.plugins.memory.embed.enabled = true;
    cx.config.plugins.memory.embed.hybrid = true;
    cx.cwd = Some(proj.to_string_lossy().into_owned());
    let project = "benchproj";
    let mut rng = Rng(SEED ^ u64::from(n));
    let mut live = Vec::new();
    for s in 0..n {
        for k in 0..K {
            let body = filler(&mut rng, s, k);
            mem_save(&cx, "note", &format!("s{s}-n{k}"), &body, Some(project)).unwrap();
        }
        for i in 0..FACTS {
            let at = if n == 1 { 0 } else { (i as u32 * (n - 1)) / 19 };
            if at != s {
                continue;
            }
            let (title, body) = fact(i, false);
            let (id, _) = mem_save(&cx, "note", &title, &body, Some(project)).unwrap();
            live.push((id, body));
        }
    }
    assert_eq!(live.len(), FACTS, "generator must plant {FACTS} facts");
    let mut retired = Vec::new();
    for (i, slot) in live.iter_mut().enumerate().take(REVISE) {
        let old = slot.0;
        let (title, body) = fact(i, true);
        let (new, old_id) = mem_revise(&cx, old, &title, &body).unwrap();
        *slot = (new, body);
        retired.push(old_id.expect("revise must retire the old row"));
    }
    (cx, live, retired, dir)
}

#[test]
fn memory_recall_bench() {
    eprintln!(
        "memory_bench T69.3 (FTS5 + P29 hybrid; half_life_days N/A — T69.2 shipped no scorer)"
    );
    eprintln!("N\tfts\thybrid\tsuperseded\trecall_b\tfull_b");
    for (idx, n) in NS.iter().copied().enumerate() {
        let (mut cx, live, retired, dir) = seed(n);
        let limit = cx.config.plugins.memory.search_limit.max(1);
        cx.config.plugins.memory.embed.enabled = false;
        let mut fts = 0u32;
        for (id, body) in &live {
            if hits_id(&cx, &query_of(body), limit, *id) {
                fts += 1;
            }
        }
        let mut superseded = 0u32;
        for (i, old) in retired.iter().copied().enumerate() {
            let (_, body) = fact(i, false);
            if hits_id(&cx, &query_of(&body), limit, old) {
                superseded += 1;
            }
        }
        cx.config.plugins.memory.embed.enabled = true;
        let mut hybrid = 0u32;
        for (id, body) in &live {
            if hits_id(&cx, &query_of(body), limit, *id) {
                hybrid += 1;
            }
        }
        let recall_b = MemoryPlugin
            .session_start(&SessionStart { source: "startup" }, &Ctx::new(&cx))
            .map(|i| i.text.len() as u64)
            .unwrap_or(0);
        let full_b = full_bytes(&cx, "benchproj");
        eprintln!("{n}\t{fts}/20\t{hybrid}/20\t{superseded}\t{recall_b}\t{full_b}");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(superseded, 0, "N={n}: T69.1 retired notes must not search");
        assert!(
            fts >= FTS_FLOOR[idx],
            "N={n}: FTS5 hit rate {fts} < floor {}",
            FTS_FLOOR[idx]
        );
        assert!(
            hybrid >= HYBRID_FLOOR[idx],
            "N={n}: hybrid hit rate {hybrid} < floor {}",
            HYBRID_FLOOR[idx]
        );
        assert!(
            full_b > recall_b,
            "N={n}: full injection must exceed recall"
        );
    }
}
