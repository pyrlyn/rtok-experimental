// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T135: the per-transcript numbers `doctor` needs, cached by (path, size, mtime) so an
//! unchanged JSONL is parsed once. The web/TUI snapshot loop hits the in-process map;
//! `rtok doctor` runs reload it from a JSON file beside the store. A cache that cannot be
//! read or written only costs a re-parse — doctor stays fail-open.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

/// One transcript's totals, valid while `size` and `mtime_ns` match the file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAgg {
    size: u64,
    mtime_ns: u64,
    /// `est_tokens` of every tool_result, per tool name — `stats::collect`'s `tools` rows.
    pub tool_tokens: BTreeMap<String, u64>,
    /// `Skill` tool_uses per `input.skill` (T61.1).
    pub skills: BTreeMap<String, u64>,
    /// T392: per skill, `(loads, body tokens)` of the `Skill` calls that repeated an earlier one
    /// with no compaction between.
    #[serde(default)]
    pub skill_repeats: BTreeMap<String, (u64, u64)>,
    /// A cached entry written before a field existed lacks it, and would report no repeats for a
    /// finished transcript forever; a version mismatch re-parses instead.
    #[serde(default)]
    version: u8,
}

const AGG_VERSION: u8 = 2;

pub type Cache = BTreeMap<PathBuf, FileAgg>;

static MEMORY: Mutex<Option<Cache>> = Mutex::new(None);

/// Aggregates for every transcript under `dir` modified after `cutoff`, in path order,
/// through the process cache seeded from `file`. The file is rewritten when anything
/// was parsed.
pub fn scan(dir: &Path, cutoff: SystemTime, file: Option<&Path>) -> Vec<(PathBuf, FileAgg)> {
    let mut guard = MEMORY.lock().unwrap_or_else(PoisonError::into_inner);
    let cache = guard.get_or_insert_with(|| file.map(load).unwrap_or_default());
    let (out, parsed) = scan_with(cache, dir, cutoff);
    if parsed > 0
        && let Some(f) = file
    {
        // Deleted transcripts leave the file with it, so the cache never outgrows the dir.
        cache.retain(|p, _| p.exists());
        let _ = save(cache, f);
    }
    out
}

/// [`scan`] over an explicit cache; also returns the bytes it had to parse.
pub fn scan_with(
    cache: &mut Cache,
    dir: &Path,
    cutoff: SystemTime,
) -> (Vec<(PathBuf, FileAgg)>, u64) {
    let mut paths = super::codex::jsonl_paths(dir, cutoff);
    paths.sort();
    let mut parsed = 0;
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let Some((size, mtime_ns)) = stamp(&p) else {
            continue;
        };
        let hit = cache
            .get(&p)
            .filter(|a| a.size == size && a.mtime_ns == mtime_ns && a.version == AGG_VERSION)
            .cloned();
        let agg = match hit {
            Some(a) => a,
            None => {
                let Ok(t) = super::jsonl::parse_path(&p) else {
                    continue;
                };
                parsed += size;
                let a = aggregate(&t, size, mtime_ns);
                cache.insert(p.clone(), a.clone());
                a
            }
        };
        out.push((p, agg));
    }
    (out, parsed)
}

fn aggregate(t: &super::jsonl::Parsed, size: u64, mtime_ns: u64) -> FileAgg {
    let mut agg = FileAgg {
        size,
        mtime_ns,
        version: AGG_VERSION,
        ..FileAgg::default()
    };
    let bodies: BTreeMap<&str, u64> = t
        .injected
        .iter()
        .map(|i| (i.tool_use_id.as_str(), i.bytes))
        .collect();
    // The compaction count before a call names the context window it loaded into.
    let mut window: BTreeMap<&str, usize> = BTreeMap::new();
    let mut names: BTreeMap<&str, &str> = BTreeMap::new();
    for u in &t.tool_uses {
        names.insert(&u.id, &u.name);
        if u.name == "Skill"
            && let Some(s) = u.input.get("skill").and_then(|v| v.as_str())
        {
            *agg.skills.entry(s.to_string()).or_default() += 1;
            // The parser appends boundaries in turn order, including equal-turn ones.
            let now = t.compactions.partition_point(|&c| c <= u.turn);
            if window.insert(s, now) == Some(now) {
                let e = agg.skill_repeats.entry(s.to_string()).or_default();
                e.0 += 1;
                e.1 += super::stats::est_tokens(bodies.get(u.id.as_str()).copied().unwrap_or(0));
            }
        }
    }
    for r in &t.tool_results {
        let name = names
            .get(r.tool_use_id.as_str())
            .copied()
            .unwrap_or("unknown");
        *agg.tool_tokens.entry(name.to_string()).or_default() +=
            super::stats::est_tokens(r.content.len() as u64);
    }
    agg
}

fn stamp(p: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(p).ok()?;
    let ns = m
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((m.len(), u64::try_from(ns).ok()?))
}

fn load(file: &Path) -> Cache {
    std::fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save(cache: &Cache, file: &Path) -> std::io::Result<()> {
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(cache)?)?;
    std::fs::rename(tmp, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(uid: &str, name: &str, input: &str, body: &str) -> String {
        format!(
            "{{\"type\":\"assistant\",\"message\":{{\"id\":\"a{uid}\",\"content\":[{{\"type\":\"tool_use\",\"id\":\"{uid}\",\"name\":\"{name}\",\"input\":{input}}}]}}}}\n\
             {{\"type\":\"user\",\"message\":{{\"id\":\"r{uid}\",\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"{uid}\",\"content\":\"{body}\"}}]}}}}\n"
        )
    }

    /// T392: a second load in the same window is a repeat, one after a compaction is not.
    #[test]
    fn a_skill_loaded_again_before_a_compaction_is_a_repeat() {
        let load = |n: u32| {
            format!(
                "{{\"type\":\"assistant\",\"message\":{{\"id\":\"a{n}\",\"content\":[{{\"type\":\"tool_use\",\"id\":\"s{n}\",\"name\":\"Skill\",\"input\":{{\"skill\":\"rtok\"}}}}]}}}}\n\
                 {{\"type\":\"user\",\"isMeta\":true,\"sourceToolUseID\":\"s{n}\",\"message\":{{\"id\":\"u{n}\",\"content\":\"{}\"}}}}\n",
                "B".repeat(40)
            )
        };
        let boundary = "{\"type\":\"system\",\"subtype\":\"compact_boundary\"}\n";
        let text = load(1) + &load(2) + boundary + &load(3);
        let agg = aggregate(&super::super::jsonl::parse_jsonl(&text), 0, 0);
        assert_eq!(agg.skills["rtok"], 3);
        assert_eq!(agg.skill_repeats["rtok"], (1, 10), "{agg:?}");
    }

    #[test]
    fn repeated_compaction_boundaries_separate_skill_windows() {
        let load = |uid| pair(uid, "Skill", r#"{"skill":"rtok"}"#, "ok");
        let boundary = "{\"type\":\"system\",\"subtype\":\"compact_boundary\"}\n";
        let text =
            load("a") + &load("b") + boundary + boundary + &load("c") + &load("d") + boundary;
        let parsed = super::super::jsonl::parse_jsonl(&text);
        assert_eq!(parsed.compactions, [4, 4, 8]);
        let agg = aggregate(&parsed, 0, 0);
        assert_eq!(agg.skills["rtok"], 4);
        assert_eq!(agg.skill_repeats["rtok"].0, 2);
    }

    /// The T135 Check: a second pass over unchanged transcripts parses 0 bytes, a changed
    /// one is parsed again, and a fresh process reads the saved file instead of the JSONL.
    #[test]
    fn unchanged_transcripts_are_parsed_once() {
        let dir = crate::testutil::tmp_dir("t135-scan");
        let t = dir.join("s.jsonl");
        std::fs::write(
            &t,
            pair("u1", "Read", "{}", &"R".repeat(40))
                + &pair("u2", "Skill", r#"{"skill":"rtok"}"#, "ok"),
        )
        .unwrap();
        let mut cache = Cache::new();
        let (first, parsed) = scan_with(&mut cache, &dir, UNIX_EPOCH);
        assert!(parsed > 0);
        assert_eq!(first[0].1.tool_tokens["Read"], 10);
        assert_eq!(first[0].1.skills["rtok"], 1);
        let (again, parsed) = scan_with(&mut cache, &dir, UNIX_EPOCH);
        assert_eq!((again, parsed), (first.clone(), 0));

        let file = dir.join("cache.json");
        save(&cache, &file).unwrap();
        let mut reloaded = load(&file);
        assert_eq!(scan_with(&mut reloaded, &dir, UNIX_EPOCH).1, 0);

        std::fs::write(&t, pair("u3", "Grep", "{}", "GGGG")).unwrap();
        let (changed, parsed) = scan_with(&mut reloaded, &dir, UNIX_EPOCH);
        assert!(parsed > 0);
        assert_eq!(changed[0].1.tool_tokens["Grep"], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
