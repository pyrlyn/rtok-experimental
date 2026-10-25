// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Gate P29 — optional embed search beside FTS5 (T29.2).

use rtok::plugin::{Measurement, Runtime};
use rtok::plugins::memory::{mem_save, mem_search};

struct Fixture {
    notes: Vec<(String, String, String, Option<String>)>,
    fts: String,
    embed: String,
    hybrid: String,
    embed_off: String,
    planted: String,
}

fn fixture() -> Fixture {
    let raw = include_str!("fixtures/p29_memory.toml");
    let doc: toml_edit::DocumentMut = raw.parse().expect("fixture parses");
    let planted = doc["expect"]["planted_title"].as_str().unwrap().to_string();
    let notes = doc["note"]
        .as_array_of_tables()
        .expect("[[note]]")
        .iter()
        .map(|t| {
            (
                t["kind"].as_str().unwrap().to_string(),
                t["title"].as_str().unwrap().to_string(),
                t["body"].as_str().unwrap().to_string(),
                t.get("project")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            )
        })
        .collect();
    let q = doc["query"].as_table().expect("[query]");
    Fixture {
        notes,
        fts: q["fts"].as_str().unwrap().into(),
        embed: q["embed"].as_str().unwrap().into(),
        hybrid: q["hybrid"].as_str().unwrap().into(),
        embed_off: q["embed_off"].as_str().unwrap().into(),
        planted,
    }
}

fn open(tag: &str, embed_enabled: bool, hybrid: bool) -> (Runtime, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("rtok-p29-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = rtok::testutil::config_in(&dir);
    cfg.plugins.memory.embed.enabled = embed_enabled;
    cfg.plugins.memory.embed.hybrid = hybrid;
    (Runtime::open(cfg, tag).unwrap(), dir)
}

fn seed(rt: &Runtime, fix: &Fixture) {
    for (kind, title, body, project) in &fix.notes {
        mem_save(rt, kind, title, body, project.as_deref()).unwrap();
    }
}

fn top_title(rt: &Runtime, query: &str) -> Option<String> {
    mem_search(rt, query, 5)
        .ok()?
        .into_iter()
        .next()
        .map(|h| h.title)
}

#[test]
fn gate_p29_fts5_default_and_embed_paths() {
    let fix = fixture();

    let (rt, dir) = open("p29-off", false, true);
    seed(&rt, &fix);
    assert_eq!(
        top_title(&rt, &fix.fts).as_deref(),
        Some(fix.planted.as_str()),
        "FTS5 with embed off"
    );
    assert!(
        top_title(&rt, &fix.embed_off).is_none(),
        "embed off: FTS must not rank the planted note on the embed query"
    );
    drop(rt);
    let _ = std::fs::remove_dir_all(&dir);

    let (rt, dir) = open("p29-embed", true, false);
    seed(&rt, &fix);
    assert_eq!(
        top_title(&rt, &fix.embed).as_deref(),
        Some(fix.planted.as_str()),
        "vector leg alone"
    );
    drop(rt);
    let _ = std::fs::remove_dir_all(&dir);

    let (rt, dir) = open("p29-hybrid", true, true);
    seed(&rt, &fix);
    assert_eq!(
        top_title(&rt, &fix.hybrid).as_deref(),
        Some(fix.planted.as_str()),
        "hybrid RRF"
    );
    let hit = top_title(&rt, &fix.hybrid).is_some();
    rt.record(&Measurement {
        plugin: "memory",
        kind: "p29_hybrid_recall",
        before_bytes: fix.hybrid.len() as u64,
        after_bytes: u64::from(hit as u8),
        est_before: 0,
        est_after: 0,
        ref_id: None,
        call_id: None,
    })
    .unwrap();
    assert_eq!(rt.store.measurement_count("memory").unwrap(), 1);
    drop(rt);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn flag_off_search_matches_fts5_only() {
    let fix = fixture();
    let (mut rt, dir) = open("p29-bytes", false, true);
    seed(&rt, &fix);
    let off = mem_search(&rt, &fix.fts, 3).unwrap();
    rt.config.plugins.memory.embed.enabled = true;
    rt.config.plugins.memory.embed.hybrid = false;
    let on_fts = rt.store.search_notes(&fix.fts, 3).unwrap();
    assert_eq!(off.len(), on_fts.len());
    for (a, b) in off.iter().zip(&on_fts) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.title, b.title);
        assert_eq!(a.snippet, b.snippet);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// T374: a note linked to a file the session has read comes back in the recall, and first,
/// although five unlinked notes match the prompt's words equally well and were saved earlier.
#[test]
fn file_context_cases_recall_the_linked_note() {
    use rtok_plugin_sdk::{Ctx, Plugin, PromptSubmit};

    let raw = include_str!("fixtures/p29_memory.toml");
    let doc: toml_edit::DocumentMut = raw.parse().expect("fixture parses");
    let cases = doc["file_case"]
        .as_array_of_tables()
        .expect("[[file_case]]");
    let min = doc["file_case_expect"]["min_recall_at_5"]
        .as_float()
        .unwrap();
    let (mut recalled, mut first) = (0, 0);
    for (i, case) in cases.iter().enumerate() {
        let file = case["file"].as_str().unwrap();
        let words = case["words"].as_str().unwrap();
        let project =
            std::env::temp_dir().join(format!("rtok-t374-p29-{i}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&project);
        let path = project.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "x").unwrap();
        let (mut rt, dir) = open(&format!("p29-file-{i}"), false, true);
        rt.cwd = Some(project.to_string_lossy().into_owned());
        // Same tokens, no such file: the last stem character becomes `x`, so lengths match.
        let (stem, ext) = file.rsplit_once('.').unwrap();
        let ghost = format!("{}x.{ext}", &stem[..stem.len() - 1]);
        for n in 0..5 {
            let body = format!("{words} see {ghost}");
            mem_save(&rt, "note", &format!("decoy-{n}"), &body, None).unwrap();
        }
        mem_save(&rt, "note", "target", &format!("{words} see {file}"), None).unwrap();
        rt.store
            .put_read_cache(
                &format!("p29-file-{i}"),
                &format!("{file}\tfull\t"),
                "h",
                None,
            )
            .unwrap();
        let prompt = PromptSubmit { prompt: words };
        let inj = rtok::plugins::memory::MemoryPlugin
            .prompt_submit(&prompt, &Ctx::new(&rt))
            .expect("recall injects");
        let lines: Vec<&str> = inj.text.lines().skip(1).collect();
        recalled += usize::from(lines.iter().any(|l| l.contains(" target (")));
        first += usize::from(lines.first().is_some_and(|l| l.contains(" target (")));
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&dir);
    }
    let total = cases.len() as f64;
    assert!(
        recalled as f64 / total >= min,
        "recall@5 {recalled}/{total}"
    );
    assert_eq!(first, cases.len(), "the linked note comes back first");
}
