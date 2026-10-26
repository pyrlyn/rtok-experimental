// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T8.8: recall and precision of `symbol` / `callers` against hand-labelled ground truth.
//!
//! The labels in `fixtures/graph_truth.toml` come from a plain-text scan of this repo, not from
//! the tags index they score: a graph measured against its own edges is an upper bound, not an
//! accuracy number. `defs` is complete per symbol, so precision is real; `refs` is a
//! must-appear subset, so recall is a lower bound and the file survives ordinary edits.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rtok::plugin::{Ctx, Runtime};
use rtok::plugins::graph::index;

struct Truth {
    name: String,
    defs: Vec<String>,
    refs: Vec<String>,
    /// T368: this name is defined in more than one crate and has an import-resolved
    /// definition. `refs` is that definition's caller files, not every same-named site.
    rank: bool,
}

fn truth() -> Vec<Truth> {
    let raw = include_str!("../fixtures/graph_truth.toml");
    let doc: toml_edit::DocumentMut = raw.parse().expect("fixture parses");
    let list = |t: &toml_edit::Table, k: &str| -> Vec<String> {
        t.get(k)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    doc["symbol"]
        .as_array_of_tables()
        .expect("[[symbol]] entries")
        .iter()
        .map(|t| Truth {
            name: t["name"].as_str().unwrap_or("").to_string(),
            defs: list(t, "defs"),
            refs: list(t, "refs"),
            rank: t.get("rank").and_then(|v| v.as_bool()).unwrap_or(false),
        })
        .collect()
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A fresh store in its own temp dir, so the tests here can run in parallel.
fn open(tag: &str) -> (Runtime, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rtok-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = rtok::testutil::config_in(&dir);
    (Runtime::open(cfg, tag).unwrap(), dir)
}

/// `name` as a whole identifier outside `//` lines, the rule the labels were scanned with:
/// `int_field` inside `int_field_accepts` or a comment does not count.
fn mentions(src: &str, name: &str) -> bool {
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .any(|l| {
            l.match_indices(name).any(|(i, _)| {
                !l[..i].chars().next_back().is_some_and(ident)
                    && !l[i + name.len()..].chars().next().is_some_and(ident)
            })
        })
}

/// Every labelled file still exists and still names its symbol. A label the tree dropped
/// scores as an index miss below: T34.6 moved every `int_field` call into `wire.rs` and ref
/// recall fell 0.318 → 0.290 with the index unchanged. This names the label instead. It reads
/// the ~60 labelled files and parses nothing, so it costs milliseconds; the scoring test pays
/// for a cold index of the whole repo.
#[test]
fn every_label_names_a_file_that_mentions_the_symbol() {
    let mut stale = Vec::new();
    for t in truth() {
        for f in t.defs.iter().chain(&t.refs) {
            let src = std::fs::read_to_string(repo().join(f)).unwrap_or_default();
            if !mentions(&src, &t.name) {
                stale.push(format!("{} {f}", t.name));
            }
        }
    }
    assert!(
        stale.is_empty(),
        "labels the tree no longer backs — repair tests/fixtures/graph_truth.toml and say why \
         in its header: {stale:?}"
    );
}

/// The constructs `src/plugins/graph/PLAN.md` lists under "Known misses", pinned on a small
/// repo: a change in what the tags query captures fails here by name instead of drifting a
/// repo-wide ratio. One parse per file, milliseconds. `scoped_callee` is caught by rtok's own
/// `RUST_SCOPED_CALL` pattern (`outline.rs`); the grammar's query alone would miss it.
/// T52.5: `OnlyTyped` is caught by rtok's own `RUST_EXTRA_REF` (bare
/// `type_identifier`); path segments (`outer`, `middle`) by its
/// `scoped_identifier` arms. The TypeScript half (`TS_CALL_TYPE_REF`: plain,
/// member and nested-member calls, generic type arguments) is pinned on
/// `main.ts` in the same repo. Macro bodies stay missed — their arguments
/// parse as an opaque `token_tree` no query can reach into.
#[test]
fn reference_capture_matches_the_known_misses() {
    let (cx, dir) = open("truth-constructs");
    let root = dir.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let src = "\
pub struct OnlyTyped;
pub struct Recv;
impl Recv {
    pub fn method_callee(&self) {}
    pub fn macro_method(&self) -> u8 { 1 }
}
pub fn plain_callee() {}
pub fn scoped_callee() {}
pub fn macro_callee() -> bool { true }
pub mod outer {
    pub mod middle {
        pub fn leaf() {}
    }
}
pub fn seg_user() {
    outer::middle::leaf();
}
pub fn user(r: Recv, t: Vec<OnlyTyped>) {
    plain_callee();
    self::scoped_callee();
    r.method_callee();
    assert!(macro_callee());
    assert_eq!(r.macro_method(), 1);
}
";
    std::fs::write(root.join("lib.rs"), src).unwrap();
    let ts = "\
export interface TsTyped { v: number; }
export function ts_plain_callee(): void {}
export const holder = { ts_member_callee(): void {} };
export function ts_user(t: Array<TsTyped>) {
    ts_plain_callee();
    holder.ts_member_callee();
}
";
    std::fs::write(root.join("main.ts"), ts).unwrap();
    index::run(&Ctx::new(&cx), &root, false).unwrap();
    let key = index::canon(&root);
    let seen = |n: &str| !cx.store.symbol_refs(&key, n).unwrap().is_empty();
    let changed: Vec<(&str, bool)> = [
        ("plain_callee", true),
        ("scoped_callee", true),
        ("method_callee", true),
        ("OnlyTyped", true),        // T52.5 type position via RUST_EXTRA_REF
        ("Recv", true),             // T52.5 parameter type via RUST_EXTRA_REF
        ("outer", true),            // T52.5 path root via RUST_EXTRA_REF
        ("middle", true),           // T52.5 intermediate segment via RUST_EXTRA_REF
        ("leaf", true),             // qualified call name (RUST_SCOPED_CALL)
        ("ts_plain_callee", true),  // T52.5 plain call via TS_CALL_TYPE_REF
        ("ts_member_callee", true), // T52.5 member call via TS_CALL_TYPE_REF
        ("TsTyped", true),          // T52.5 generic argument via TS_CALL_TYPE_REF
        ("macro_callee", true),     // T663 call in macro arguments via RUST_EXTRA_REF
        ("macro_method", true),     // T663 method call in macro arguments
    ]
    .into_iter()
    .filter(|(n, want)| seen(n) != *want)
    .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        changed.is_empty(),
        "reference capture changed for {changed:?} (name, expected); update PLAN.md \
         \"Known misses\" and this table"
    );
}

/// T52.5: the new constructs flow into `callers`. Rust groups them under the
/// enclosing definition; TypeScript groups them at file level, because the
/// upstream TS tags query captures no `function_declaration` definitions —
/// a separate definitions gap, not covered here (see `PLAN.md` Known misses).
#[test]
fn new_constructs_group_under_the_enclosing_definition() {
    let (cx, dir) = open("truth-scopes");
    let root = dir.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("lib.rs"),
        "pub struct OnlyTyped;\npub fn helper() {}\npub fn user(t: Vec<OnlyTyped>) {\n    crate::helper();\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("main.ts"),
        "export interface TsTyped { v: number; }\nexport const holder = { ts_member_callee(): void {} };\nexport function ts_user(t: Array<TsTyped>) {\n    holder.ts_member_callee();\n}\n",
    )
    .unwrap();
    index::run(&Ctx::new(&cx), &root, false).unwrap();
    let key = index::canon(&root);
    let scopes = |n: &str| {
        cx.store
            .symbol_ref_groups(&key, n)
            .unwrap()
            .into_iter()
            .map(|(p, s, c, _)| format!("{p}/{s}x{c}"))
            .collect::<Vec<_>>()
    };
    let got = scopes("OnlyTyped");
    assert_eq!(
        got,
        ["lib.rs/userx1"],
        "type use groups under user, got {got:?}"
    );
    let got = scopes("helper");
    assert_eq!(
        got,
        ["lib.rs/userx1"],
        "scoped call groups under user, got {got:?}"
    );
    // No `function_declaration` defs upstream, so both TS refs sit at file level —
    // the rows exist (the pre-T52.5 answer was no rows at all).
    let got = scopes("TsTyped");
    assert_eq!(
        got,
        ["main.ts/x1"],
        "generic argument is found, got {got:?}"
    );
    let got = scopes("ts_member_callee");
    assert_eq!(got, ["main.ts/x1"], "member call is found, got {got:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Definitions must clear the P8b bar of 0.9 and do, at 1.0. References clear it
/// too since T52.5: rtok's own `RUST_EXTRA_REF` / `TS_CALL_TYPE_REF` cover type
/// positions, scoped calls and path segments, and every remaining miss is inside
/// a macro body (opaque `token_tree`), which only T663's `token_tree` pattern reaches. The floor
/// below is a regression guard on the measured 1.000 after T663 (0.944 before it; 0.924 at T387, 0.314 while the T52.5 queries were reverted, 0.351
/// at T8.8, 0.339 at T8.19, 0.318 before T34.6; repo drift, not index drift),
/// with slack for label drift — not a target. `src/plugins/graph/PLAN.md`
/// names the constructs under "Known misses".
///
/// Why it is the slowest test here: it indexes the whole repo cold, in a debug build, one file
/// after another — walk, stat query, read, sha256, a tags parse that first compiles the Rust
/// tags query afresh (`outline::config` builds a `TagsConfiguration` per file), then one
/// transaction per file with one INSERT per row. The printed line splits the cost.
#[test]
fn labelled_symbols_are_found() {
    let (cx, dir) = open("truth");
    let root = repo();
    let t0 = Instant::now();
    let cold = index::run(&Ctx::new(&cx), &root, false).unwrap();
    let cold_t = t0.elapsed();
    let t1 = Instant::now();
    let warm = index::run(&Ctx::new(&cx), &root, false).unwrap();
    println!(
        "index: cold {} files, {} rows in {cold_t:.2?}; warm re-walk (each watcher settle) \
         read {} files in {:.2?}",
        cold.indexed,
        cold.inserted,
        warm.read,
        t1.elapsed()
    );
    let key = index::canon(&root);

    let (mut dwant, mut dgot) = (0usize, 0usize);
    let (mut rwant, mut rgot) = (0usize, 0usize);
    let (mut def_returned, mut def_right) = (0usize, 0usize);
    let mut misses: Vec<String> = Vec::new();
    let mut false_positives: Vec<(&Truth, Vec<String>)> = Vec::new();
    let truth = truth();
    for t in &truth {
        let defs: HashSet<String> = cx
            .store
            .symbol_defs(&key, &t.name)
            .unwrap()
            .into_iter()
            .map(|(p, ..)| p)
            .collect();
        let refs: HashSet<String> = cx
            .store
            .symbol_refs(&key, &t.name)
            .unwrap()
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        def_returned += defs.len();
        def_right += defs.iter().filter(|p| t.defs.contains(p)).count();
        let fp: Vec<String> = defs
            .iter()
            .filter(|p| !t.defs.contains(p))
            .cloned()
            .collect();
        if !fp.is_empty() {
            false_positives.push((t, fp));
        }
        for d in &t.defs {
            dwant += 1;
            if defs.contains(d) {
                dgot += 1;
            } else {
                misses.push(format!("def {} {d}", t.name));
            }
        }
        for r in &t.refs {
            rwant += 1;
            if refs.contains(r) {
                rgot += 1;
            } else {
                misses.push(format!("ref {} {r}", t.name));
            }
        }
    }
    let def_recall = dgot as f64 / dwant.max(1) as f64;
    let ref_recall = rgot as f64 / rwant.max(1) as f64;
    let (got, want) = (dgot + rgot, dwant + rwant);
    let recall = got as f64 / want as f64;
    let precision = def_right as f64 / def_returned.max(1) as f64;
    println!(
        "graph_truth: precision {precision:.3} recall {recall:.3} over {} labels\n  sites {got}/{want}\n  definitions {dgot}/{dwant} recall {def_recall:.3} precision {precision:.3} ({def_right}/{def_returned})\n  references {rgot}/{rwant} recall {ref_recall:.3}",
        truth.len()
    );
    for m in &misses {
        println!("  miss: {m}");
    }
    for (t, defs) in false_positives {
        for p in defs {
            println!("  def {} in {p} is not a labelled definition", t.name);
        }
    }
    let overall = if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    };
    println!("graph_truth overall {overall:.3}");
    assert_rank_gates(&cx, &root, &key);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        def_recall >= 0.9,
        "definition recall {def_recall:.3} below the P8b bar of 0.9; misses: {misses:?}"
    );
    assert!(
        precision >= 0.99,
        "definition precision {precision:.3}: a name resolved to a file it is not defined in"
    );
    assert!(
        ref_recall >= 0.98,
        "reference recall {ref_recall:.3} fell below the 0.98 floor (measured 1.000 after \
         T663, which reads calls inside macro arguments; 0.944 before); run \
         every_label_names_a_file_that_mentions_the_symbol first — a stale label reads as a miss"
    );
    assert!(
        Path::new(&root).join("src").is_dir(),
        "ground truth was scored against this repo"
    );
    assert!(
        overall >= 0.93,
        "T8.8 overall score {overall:.3} below 0.93"
    );
}

/// T368: precision of `callers` on the `rank` labels against the unfiltered
/// ref-file list, and `impact` bytes at depth 2 against that same walk after
/// the same token cap `impact` applies.
fn assert_rank_gates(cx: &Runtime, root: &Path, key: &str) {
    let labels: Vec<Truth> = truth().into_iter().filter(|t| t.rank).collect();
    assert!(
        labels.len() >= 10,
        "need at least 10 ambiguous names, got {}",
        labels.len()
    );
    let ctx = Ctx::new(cx);
    let (mut base_hit, mut base_n, mut new_hit, mut new_n) = (0usize, 0usize, 0usize, 0usize);
    let (mut base_rec, mut new_rec, mut label_n) = (0usize, 0usize, 0usize);
    let (mut impact_new, mut impact_base) = (0usize, 0usize);
    for t in &labels {
        // `src/` is the rtok package. `crates/<name>/` is another package. A name
        // the ranker should disambiguate is defined in at least two of those.
        let crates: HashSet<&str> = t
            .defs
            .iter()
            .map(|p| match p.strip_prefix("crates/") {
                Some(rest) => rest.split('/').next().unwrap_or(rest),
                None => "rtok",
            })
            .collect();
        assert!(
            crates.len() >= 2,
            "{} is not defined in two crates ({crates:?})",
            t.name
        );
        let all: HashSet<String> = cx
            .store
            .symbol_ref_groups(key, &t.name)
            .unwrap()
            .into_iter()
            .map(|(p, ..)| p)
            .collect();
        let callers_out = rtok::plugins::graph::callers(&ctx, root, &t.name).unwrap();
        assert!(
            callers_out.contains(&format!("other definitions of {}", t.name)),
            "{callers_out}"
        );
        let listed: HashSet<String> = caller_files(&callers_out).into_iter().collect();
        let want: HashSet<String> = t.refs.iter().cloned().collect();
        base_hit += all.intersection(&want).count();
        base_n += all.len();
        new_hit += listed.intersection(&want).count();
        new_n += listed.len();
        base_rec += all.intersection(&want).count();
        new_rec += listed.intersection(&want).count();
        label_n += want.len();
        let impact = rtok::plugins::graph::impact(&ctx, root, &t.name, 2, None).unwrap();
        assert!(
            impact.contains(&format!("other definitions of {}", t.name)),
            "{impact}"
        );
        let ambiguous = cx.store.symbol_defs(key, &t.name).unwrap().len() > 1;
        let rows = cx.store.symbol_impact(key, &t.name, 2).unwrap();
        let base = capped_len(&ctx, &main_impact_text(&rows, ambiguous));
        println!(
            "rank {name}: callers {listed}/{all} files, impact {impact_b} vs main {base}",
            name = t.name,
            listed = listed.len(),
            all = all.len(),
            impact_b = impact.len(),
        );
        let _ = std::io::Write::flush(&mut std::io::stdout());
        impact_new += impact.len();
        impact_base += base;
    }
    let base_p = base_hit as f64 / base_n.max(1) as f64;
    let new_p = new_hit as f64 / new_n.max(1) as f64;
    let base_r = base_rec as f64 / label_n.max(1) as f64;
    let new_r = new_rec as f64 / label_n.max(1) as f64;
    let ratio = impact_new as f64 / impact_base.max(1) as f64;
    println!(
        "rank gates: callers precision {new_p:.3} vs main {base_p:.3} (+{:.3}), recall {new_r:.3} vs {base_r:.3} (drop {:.3}), impact bytes {impact_new}/{impact_base} ({ratio:.3})",
        new_p - base_p,
        base_r - new_r,
    );
    assert!(
        new_p >= base_p + 0.20,
        "callers precision {new_p:.3} is not +20 pp over main {base_p:.3}"
    );
    assert!(
        base_r - new_r <= 0.02,
        "callers recall dropped {:.3} pp, above 2",
        (base_r - new_r) * 100.0
    );
    assert!(
        ratio <= 0.70,
        "impact bytes {impact_new} vs main {impact_base} ({ratio:.3}) did not fall 30%"
    );
}

/// What `impact` printed before ranking: the unfiltered walk, ambiguous mark included.
fn main_impact_text(rows: &[(u32, String, String)], ambiguous: bool) -> String {
    let mut body = String::new();
    for (d, path, scope) in rows {
        if scope.is_empty() {
            body.push_str(&format!("{d}  {path}  (file)\n"));
        } else {
            body.push_str(&format!("{d}  {path}  {scope}\n"));
        }
    }
    if !ambiguous {
        return body;
    }
    let marked = if body.is_empty() {
        String::new()
    } else {
        body.lines()
            .map(|line| format!("{line} ?"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    };
    format!("1 names ambiguous (?): narrow with path or kind, or backend = \"lsp\"\n{marked}")
}

/// Byte length of `cap`: same char budget and 8-char archive prefix, without writing one.
fn capped_len(cx: &Ctx, text: &str) -> usize {
    let max = cx.plugin_config::<rtok::config::Graph>("graph").max_tokens;
    let est = cx.estimate(text, rtok::plugin::Class::Code);
    if est <= max {
        return text.len();
    }
    let text_chars = text.chars().count();
    let budget_chars = (text_chars * max as usize / est as usize).saturating_sub(120);
    let total = text.lines().count();
    let mut head = String::new();
    let mut shown = 0usize;
    for line in text.lines() {
        if shown > 0 && head.chars().count() + line.chars().count() + 1 > budget_chars {
            break;
        }
        head.push_str(line);
        head.push('\n');
        shown += 1;
    }
    // Same length `names::short_id` prints (`MIN_PREFIX`). An integration test cannot see that const.
    let id = "0".repeat(8);
    format!("{head}{} more, expand {id}", total - shown).len()
}

/// Files `callers` printed, ignoring the banner, the cap trailer, and `+N other definitions`.
fn caller_files(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|line| {
            let line = line.trim_end().trim_end_matches(" ?").trim();
            if line.is_empty()
                || line.starts_with('+')
                || line.contains("ambiguous")
                || line.contains("more, expand")
            {
                return None;
            }
            let path = line.split(" ×").next()?.split("  ").next()?.trim();
            if path.is_empty() {
                None
            } else {
                Some(path.to_string())
            }
        })
        .collect()
}
