// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Risk-ranked reading list for a git diff (T454).
//!
//! Score endpoints follow code-review-graph `changes.py` `compute_risk_score`
//! (flow and community terms omitted: this index has neither). Region budgets
//! follow `tools/review.py`: whole hunks, round-robin, one shared line budget.
//! Writes no `Measurement`; a shortened answer is capped by `cap` like the other tools.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;
use serde_json::{Value, json};

use rtok_plugin_sdk::Ctx;

use super::{
    WalkMemo, git_diff_args, git_stdout, git_stdout_result, impact_walk_memo, index, index_for,
    is_support_path, is_test_path,
};

/// Shared source lines for one review (`tools/review.py` `_MAX_REVIEW_SOURCE_LINES`).
pub const SOURCE_BUDGET: u32 = 800;
/// One file holds at most this share of [`SOURCE_BUDGET`].
pub const FILE_SHARE: f64 = 0.4;
/// Lines either side of a hunk that is not widened to its definition.
pub const REGION_CONTEXT: u32 = 4;
/// A definition at most this many lines is shown whole when a hunk lands in it.
pub const MAX_WIDEN_SPAN: u32 = 40;
/// One granted region, so a single added block cannot take the file share.
pub const MAX_REGION_LINES: u32 = 120;
/// Hard per-file line cap used when computing the share (`review.py` `_MAX_LINES_PER_FILE`).
pub const MAX_LINES_PER_FILE: u32 = 500;
/// Defs scored in one file before the next file is visited.
pub const MAX_RISK_NODES_PER_FILE: usize = 8;
/// Defs scored across the whole diff.
pub const MAX_RISK_SCORED_NODES: usize = 400;

pub(super) const SCORED_KINDS: &[&str] = &["function", "method", "class"];

/// GitHub Linguist's names, so one `.gitattributes` line folds a path in the PR diff view and
/// marks it support code here.
const SUPPORT_ATTRS: &[&str] = &[
    "linguist-vendored",
    "linguist-generated",
    "linguist-documentation",
];
/// Paths per `git check-attr` call, under every platform's argument length limit.
const ATTR_BATCH: usize = 500;

/// Whole-token keywords. Substring matches (`sign` inside `assign`) are not hits.
const SECURITY: &[&str] = &[
    "auth",
    "login",
    "password",
    "token",
    "session",
    "crypt",
    "secret",
    "credential",
    "permission",
    "sql",
    "query",
    "execute",
    "connect",
    "socket",
    "request",
    "http",
    "sanitize",
    "validate",
    "encrypt",
    "decrypt",
    "hash",
    "sign",
    "verify",
    "admin",
    "privilege",
];

/// One changed region in a file, 1-based inclusive lines in the new file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Debug)]
pub struct FileReview {
    pub path: String,
    /// Highest score among scored defs in this file. 0.0 if none were scored.
    pub score: f64,
    /// Changed defs for which no test path is reached.
    pub untested: Vec<String>,
    pub regions: Vec<Region>,
    /// Regions that lost the shared line budget.
    pub regions_omitted: u32,
    /// Fixture, benchmark, example or Linguist-marked file: listed, never scored.
    pub support: bool,
}

/// `callers` is depth-1 impact rows. `tested` is test-path reachability.
/// `security` is a whole-token keyword hit.
pub fn risk_score(callers: u32, tested: bool, security: bool) -> f64 {
    let mut score = if tested { 0.05 } else { 0.30 };
    if security {
        score += 0.20;
    }
    score += (f64::from(callers) / 20.0).min(0.10);
    (score.clamp(0.0, 1.0) * 10_000.0).round() / 10_000.0
}

/// Thresholds fit [`risk_score`]'s reachable 0.05..=0.6, not code-review-graph's 0..=1: only an
/// untested security-named definition is `high`, any other untested one is `medium`.
pub fn level(score: f64) -> &'static str {
    if score > 0.45 {
        "high"
    } else if score > 0.25 {
        "medium"
    } else {
        "low"
    }
}

/// `true` when a snake_case or camelCase token is a security keyword.
pub fn security_token(name: &str) -> bool {
    identifier_tokens(name)
        .iter()
        .any(|t| SECURITY.contains(&t.as_str()))
}

fn identifier_tokens(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in name.chars() {
        if c == '_' || c == '-' {
            push_token(&mut out, &mut cur);
            continue;
        }
        if c.is_uppercase() && !cur.is_empty() {
            push_token(&mut out, &mut cur);
        }
        cur.push(c);
    }
    push_token(&mut out, &mut cur);
    out
}

fn push_token(out: &mut Vec<String>, cur: &mut String) {
    if !cur.is_empty() {
        out.push(cur.to_ascii_lowercase());
        cur.clear();
    }
}

/// Parse `git diff -U0` hunk headers. Keys are repo-relative paths using `/`.
pub fn parse_hunks(diff: &str) -> BTreeMap<String, Vec<(u32, u32)>> {
    let mut out: BTreeMap<String, Vec<(u32, u32)>> = BTreeMap::new();
    let mut path: Option<String> = None;
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            let raw = rest.split_whitespace().next().unwrap_or(rest);
            if raw == "/dev/null" {
                path = None;
            } else {
                let p = raw.strip_prefix("b/").unwrap_or(raw);
                path = Some(p.replace('\\', "/"));
            }
            continue;
        }
        let Some(rest) = line.strip_prefix("@@ ") else {
            continue;
        };
        let Some(path) = path.as_ref() else {
            continue;
        };
        if let Some(hunk) = parse_hunk_header(rest) {
            out.entry(path.clone()).or_default().push(hunk);
        }
    }
    out
}

fn parse_hunk_header(rest: &str) -> Option<(u32, u32)> {
    let plus = rest.find('+')?;
    let spec = rest[plus + 1..].split_whitespace().next()?;
    let mut parts = spec.split(',');
    let start: u32 = parts.next()?.parse().ok()?;
    let len: u32 = match parts.next() {
        Some(s) => s.parse().ok()?,
        None => 1,
    };
    if start == 0 || len == 0 {
        return None;
    }
    Some((start, start + len - 1))
}

fn region_len(region: &Region) -> u32 {
    region.end.saturating_sub(region.start).saturating_add(1)
}

/// Widen each hunk to an enclosing definition when that definition is at most
/// [`MAX_WIDEN_SPAN`] lines, otherwise pad by [`REGION_CONTEXT`]. Cap one region
/// at [`MAX_REGION_LINES`].
pub fn widen(hunks: &[(u32, u32)], defs: &[(String, String, i32, i32)]) -> Vec<Region> {
    let mut regions = Vec::new();
    for &(hs, he) in hunks {
        let mut start = hs.saturating_sub(REGION_CONTEXT).max(1);
        let mut end = he.saturating_add(REGION_CONTEXT);
        let mut best: Option<(u32, u32)> = None;
        for (_, _, line, end_line) in defs {
            if *line < 1 || *end_line < *line {
                continue;
            }
            let line = *line as u32;
            let end_line = *end_line as u32;
            if line <= hs && end_line >= he {
                let span = end_line.saturating_sub(line).saturating_add(1);
                if span <= MAX_WIDEN_SPAN {
                    let tighter = best
                        .map(|(s, e)| span < e.saturating_sub(s).saturating_add(1))
                        .unwrap_or(true);
                    if tighter {
                        best = Some((line, end_line));
                    }
                }
            }
        }
        if let Some((s, e)) = best {
            start = s;
            end = e;
        }
        let len = end.saturating_sub(start).saturating_add(1);
        if len > MAX_REGION_LINES {
            end = he;
            start = end.saturating_sub(MAX_REGION_LINES - 1).max(1);
            if start > hs {
                start = hs;
                end = start + MAX_REGION_LINES - 1;
            }
        }
        regions.push(Region { start, end });
    }
    merge_regions(&mut regions);
    regions
}

fn merge_regions(regions: &mut Vec<Region>) {
    if regions.is_empty() {
        return;
    }
    regions.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Region> = Vec::new();
    for region in regions.drain(..) {
        if let Some(last) = merged.last_mut()
            && region.start <= last.end.saturating_add(1)
        {
            last.end = last.end.max(region.end);
            continue;
        }
        merged.push(region);
    }
    *regions = merged;
}

fn share_cap(total: u32) -> u32 {
    let share = (f64::from(total) * FILE_SHARE) as u32;
    share.clamp(1, MAX_LINES_PER_FILE)
}

/// Grant whole regions from `total_budget`, round-robin in the order of `files`
/// (highest score first). A region bigger than the per-region cap is truncated
/// to that cap; one that still does not fit is omitted. Files scoring 0 get only
/// what is left once every scored file had all its regions.
pub fn allocate_budget(files: &mut [FileReview], total_budget: u32) {
    let pending: Vec<Vec<Region>> = files.iter().map(|f| f.regions.clone()).collect();
    for file in files.iter_mut() {
        file.regions.clear();
        file.regions_omitted = 0;
    }
    if files.is_empty() || total_budget == 0 {
        for (file, regions) in files.iter_mut().zip(&pending) {
            file.regions_omitted = regions.len() as u32;
        }
        return;
    }
    let share = share_cap(total_budget);
    let region_cap = MAX_REGION_LINES.min(share);
    let mut used = vec![0u32; files.len()];
    let mut remaining = total_budget;
    let mut granted = vec![Vec::new(); files.len()];
    let (scored, unscored): (Vec<usize>, Vec<usize>) =
        (0..files.len()).partition(|&i| files[i].score > 0.0);
    for group in [scored, unscored] {
        let deepest = group.iter().map(|&i| pending[i].len()).max().unwrap_or(0);
        for depth in 0..deepest {
            if remaining == 0 {
                break;
            }
            for &i in &group {
                if depth >= pending[i].len() || remaining == 0 {
                    continue;
                }
                let region = &pending[i][depth];
                let full = region_len(region);
                if full == 0 {
                    continue;
                }
                let cost = full.min(region_cap);
                let room = share.saturating_sub(used[i]);
                if cost > remaining.min(room) {
                    continue;
                }
                granted[i].push(Region {
                    start: region.start,
                    end: region.start + cost - 1,
                });
                used[i] += cost;
                remaining -= cost;
            }
        }
    }
    for (i, file) in files.iter_mut().enumerate() {
        granted[i].sort_by_key(|r| (r.start, r.end));
        file.regions_omitted = (pending[i].len() as u32).saturating_sub(granted[i].len() as u32);
        file.regions = std::mem::take(&mut granted[i]);
    }
}

pub fn allocate(files: &mut [FileReview]) {
    allocate_budget(files, SOURCE_BUDGET);
}

fn changed_files(root: &Path, since: Option<&str>, staged: bool) -> Result<Vec<String>> {
    let args = git_diff_args(since, staged, &["--name-only", "--relative", "-z"]);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let stdout = git_stdout_result(root, &refs)?;
    Ok(stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .filter_map(|s| String::from_utf8(s.to_vec()).ok())
        .map(|p| p.replace('\\', "/"))
        .collect())
}

fn hunks_of(
    root: &Path,
    since: Option<&str>,
    staged: bool,
) -> Result<BTreeMap<String, Vec<(u32, u32)>>> {
    let args = git_diff_args(since, staged, &["-U0", "--relative"]);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let stdout = git_stdout_result(root, &refs)?;
    let text = String::from_utf8(stdout).map_err(|e| anyhow::anyhow!("git diff failed: {e}"))?;
    Ok(parse_hunks(&text))
}

/// Changed paths that are support code: a set Linguist attribute, else [`is_support_path`]. An
/// explicit `false` or unset attribute keeps a built-in directory in review. When
/// `git check-attr` fails the directories alone decide.
fn support_paths(root: &Path, paths: &[String]) -> BTreeSet<String> {
    let mut attrs = BTreeMap::new();
    for chunk in paths.chunks(ATTR_BATCH) {
        let mut args = vec!["check-attr", "-z"];
        args.extend(SUPPORT_ATTRS);
        args.push("--");
        args.extend(chunk.iter().map(String::as_str));
        let Some(out) = git_stdout(root, &args) else {
            break;
        };
        for (path, on) in parse_check_attr(&out) {
            *attrs.entry(path).or_insert(on) |= on;
        }
    }
    paths
        .iter()
        .filter(|p| {
            attrs
                .get(p.as_str())
                .copied()
                .unwrap_or_else(|| is_support_path(p))
        })
        .cloned()
        .collect()
}

/// `git check-attr -z` output is `path NUL attr NUL value NUL` triples. `true` for a set
/// attribute, `false` for one explicitly off; an unspecified one yields nothing.
fn parse_check_attr(out: &[u8]) -> Vec<(String, bool)> {
    let fields: Vec<&[u8]> = out.split(|b| *b == 0).collect();
    fields
        .as_chunks::<3>()
        .0
        .iter()
        .filter_map(|[path, _, value]| {
            let on = match *value {
                b"set" | b"true" => true,
                b"unset" | b"false" => false,
                _ => return None,
            };
            Some((String::from_utf8(path.to_vec()).ok()?, on))
        })
        .collect()
}

/// Scored definitions a hunk touches, in file order. Untouched neighbours are not the change.
fn scored_names(defs: &[(String, String, i32, i32)], hunks: &[(u32, u32)]) -> Vec<String> {
    let touched = |line: i32, end: i32| {
        line >= 1
            && end >= line
            && hunks
                .iter()
                .any(|&(hs, he)| line as u32 <= he && end as u32 >= hs)
    };
    defs.iter()
        .filter(|(_, kind, line, end)| {
            SCORED_KINDS.contains(&kind.as_str()) && touched(*line, *end)
        })
        .take(MAX_RISK_NODES_PER_FILE)
        .map(|(name, ..)| name.clone())
        .collect()
}

fn score_file(
    cx: &Ctx,
    key: &str,
    path: &str,
    names: &[String],
    memo: &mut WalkMemo,
) -> Result<(f64, Vec<String>)> {
    let mut score: f64 = 0.0;
    let mut untested = Vec::new();
    let in_test = is_test_path(path);
    for name in names {
        let rows = impact_walk_memo(cx, &[key], name, 3, true, memo)?;
        let callers = rows.iter().filter(|(_, depth, ..)| *depth == 1).count() as u32;
        let tested = in_test || rows.iter().any(|(_, _, p, _)| is_test_path(p));
        let security = security_token(name);
        score = score.max(risk_score(callers, tested, security));
        if !tested {
            untested.push(name.clone());
        }
    }
    Ok((score, untested))
}

fn render(files: &[FileReview], json: bool) -> String {
    let overall = files.iter().map(|f| f.score).fold(0.0, f64::max);
    if json {
        let body = json!({
            "level": level(overall),
            "score": overall,
            "files": files.iter().map(file_json).collect::<Vec<_>>(),
        });
        return format!("{body}\n");
    }
    let mut out = format!("review: {} {overall:.4}\n", level(overall));
    for file in files {
        out.push_str(&format!("{} {:.4}", file.path, file.score));
        if file.support {
            out.push_str(" support");
        }
        if !file.untested.is_empty() {
            out.push_str(" untested: ");
            out.push_str(&file.untested.join(", "));
        }
        out.push('\n');
        for region in &file.regions {
            out.push_str(&format!("  L{}-{}\n", region.start, region.end));
        }
    }
    for file in files {
        if file.regions_omitted > 0 {
            let word = if file.regions_omitted == 1 {
                "region"
            } else {
                "regions"
            };
            out.push_str(&format!(
                "omitted: {} {} {word}\n",
                file.path, file.regions_omitted
            ));
        }
    }
    out
}

fn file_json(file: &FileReview) -> Value {
    json!({
        "path": file.path,
        "score": file.score,
        "untested": file.untested,
        "regions": file.regions.iter().map(|r| json!({"start": r.start, "end": r.end})).collect::<Vec<_>>(),
        "regions_omitted": file.regions_omitted,
        "support": file.support,
    })
}

/// Index `root`, read the diff, score, allocate, render.
/// Git failure returns `Err` whose Display starts with `git diff failed`.
/// An empty diff returns the exact line `no changes`.
/// Writes no `Measurement` unless `cap` shortens a long answer.
pub fn review(
    cx: &Ctx,
    root: &Path,
    since: Option<&str>,
    staged: bool,
    json: bool,
) -> Result<String> {
    let paths = changed_files(root, since, staged)?;
    if paths.is_empty() {
        return Ok("no changes\n".to_string());
    }
    index_for(cx, root)?;
    let key = index::canon(root);
    let hunks = hunks_of(root, since, staged)?;
    let support = support_paths(root, &paths);
    let mut remaining = MAX_RISK_SCORED_NODES;
    // The index is not written again before `review` returns, so every walk may share answers.
    let mut memo = WalkMemo::default();
    let mut files = Vec::with_capacity(paths.len());
    for path in &paths {
        let stored = cx.symbol_file_defs(&key, path).unwrap_or_default();
        let file_hunks = hunks.get(path).map(Vec::as_slice).unwrap_or(&[]);
        let is_support = support.contains(path);
        let mut names = if is_support {
            Vec::new()
        } else {
            scored_names(&stored, file_hunks)
        };
        if names.len() > remaining {
            names.truncate(remaining);
        }
        remaining = remaining.saturating_sub(names.len());
        let (score, untested) = if names.is_empty() {
            (0.0, Vec::new())
        } else {
            score_file(cx, &key, path, &names, &mut memo)?
        };
        let regions = widen(file_hunks, &stored);
        files.push(FileReview {
            path: path.clone(),
            score,
            untested,
            regions,
            regions_omitted: 0,
            support: is_support,
        });
    }
    files.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    allocate(&mut files);
    super::cap(cx, render(&files, json))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::graph::index::tests::cx;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    #[test]
    fn risk_score_endpoints() {
        assert_eq!(risk_score(0, false, false), 0.3);
        assert_eq!(risk_score(0, true, false), 0.05);
        assert_eq!(risk_score(100, false, true), 0.6);
        // Untested and security-named is high; any other untested is medium; tested is low
        // unless security-named with callers.
        assert_eq!(level(risk_score(0, false, true)), "high");
        assert_eq!(level(risk_score(0, false, false)), "medium");
        assert_eq!(level(risk_score(100, false, false)), "medium");
        assert_eq!(level(risk_score(100, true, true)), "medium");
        assert_eq!(level(risk_score(0, true, true)), "low");
        assert_eq!(level(risk_score(100, true, false)), "low");
    }

    #[test]
    fn support_dirs_are_fixtures_benches_and_examples() {
        use crate::plugins::graph::is_support_path;
        assert!(is_support_path("bench/check.py"));
        assert!(is_support_path("bench/repos/t9/src/lib.rs"));
        assert!(is_support_path("crates/x/benches/b.rs"));
        assert!(is_support_path("tests/fixtures/a.rs"));
        assert!(is_support_path("go/testdata/a.go"));
        assert!(is_support_path("examples/demo.rs"));
        assert!(!is_support_path("src/bench.rs"));
        assert!(!is_support_path("src/benchmark/run.rs"));
        assert!(!is_support_path("src/lib.rs"));
    }

    #[test]
    fn check_attr_reads_set_and_explicitly_off_and_skips_unspecified() {
        let out = b"a.rs\0linguist-vendored\0set\0a.rs\0linguist-generated\0unspecified\0\
b.rs\0linguist-vendored\0false\0c.rs\0linguist-documentation\0true\0d.rs\0linguist-vendored\0unset\0";
        assert_eq!(
            parse_check_attr(out),
            vec![
                ("a.rs".to_string(), true),
                ("b.rs".to_string(), false),
                ("c.rs".to_string(), true),
                ("d.rs".to_string(), false),
            ]
        );
    }

    #[test]
    fn unscored_files_read_only_what_scored_files_leave() {
        let file = |path: &str, score: f64, starts: &[u32]| FileReview {
            path: path.into(),
            score,
            untested: vec![],
            regions: starts
                .iter()
                .map(|&s| Region {
                    start: s,
                    end: s + 3,
                })
                .collect(),
            regions_omitted: 0,
            support: false,
        };
        // Budget 24, per-file share 9, regions of 4 lines. Plain round-robin would grant
        // README's region at depth 0 and leave c.rs without its second; now the three scored
        // files take all 24 lines and README is the one omitted.
        let mut files = vec![
            file("a.rs", 0.5, &[1, 20]),
            file("b.rs", 0.4, &[1, 20]),
            file("c.rs", 0.3, &[1, 20]),
            file("README.md", 0.0, &[1]),
        ];
        allocate_budget(&mut files, 24);
        for scored in &files[..3] {
            assert_eq!(scored.regions.len(), 2, "{files:?}");
        }
        assert!(files[3].regions.is_empty(), "{files:?}");
        assert_eq!(files[3].regions_omitted, 1);
    }

    #[test]
    fn security_token_is_a_whole_token() {
        assert!(security_token("sign"));
        assert!(security_token("sign_in"));
        assert!(security_token("httpClient"));
        assert!(!security_token("assign"));
        assert!(!security_token("signature"));
    }

    #[test]
    fn parse_hunks_reads_new_file_ranges_and_skips_deletes() {
        let diff = "\
diff --git a/src/a.rs b/src/a.rs
--- a/src/a.rs
+++ b/src/a.rs
@@ -10 +10,2 @@ fn
 context
diff --git a/gone.rs b/gone.rs
--- a/gone.rs
+++ /dev/null
@@ -1 +0,0 @@
-fn gone() {}
";
        let hunks = parse_hunks(diff);
        assert_eq!(
            hunks.get("src/a.rs").map(Vec::as_slice),
            Some([(10, 11)].as_slice())
        );
        assert!(!hunks.contains_key("gone.rs"));
    }

    #[test]
    fn widen_covers_a_short_definition_and_not_a_long_one() {
        let short = [("small".into(), "function".into(), 1, 10)];
        let regions = widen(&[(5, 5)], &short);
        assert_eq!(regions, vec![Region { start: 1, end: 10 }]);

        let long = [("big".into(), "function".into(), 1, 400)];
        let regions = widen(&[(200, 200)], &long);
        let len = regions[0].end - regions[0].start + 1;
        assert!(len < 40, "{regions:?}");
        assert!(regions[0].start <= 200 && regions[0].end >= 200);
    }

    #[test]
    fn allocate_serves_the_higher_score_first() {
        let region = |start| Region {
            start,
            end: start + 3,
        };
        let file = |path: &str, score: f64| FileReview {
            path: path.into(),
            score,
            untested: vec![],
            regions: vec![region(1)],
            regions_omitted: 0,
            support: false,
        };
        // Budget 10, per-file share 4, each region 4 lines. The first two fit (8);
        // the lowest-ranked file is the one left out.
        let mut files = vec![file("a.rs", 0.5), file("b.rs", 0.3), file("c.rs", 0.1)];
        allocate_budget(&mut files, 10);
        assert_eq!(files[0].regions.len(), 1, "{files:?}");
        assert_eq!(files[1].regions.len(), 1, "{files:?}");
        assert!(files[2].regions.is_empty(), "{files:?}");
        assert_eq!(files[2].regions_omitted, 1);
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .current_dir(dir)
            .envs([
                ("GIT_AUTHOR_NAME", "t"),
                ("GIT_AUTHOR_EMAIL", "t@t"),
                ("GIT_COMMITTER_NAME", "t"),
                ("GIT_COMMITTER_EMAIL", "t@t"),
            ])
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "{args:?}");
    }

    fn repo(tag: &str, lib: &str, test: Option<&str>) -> (crate::plugin::Runtime, PathBuf) {
        let (cx, dir) = cx(tag);
        fs::write(dir.join("lib.rs"), lib).unwrap();
        if let Some(test) = test {
            fs::create_dir_all(dir.join("tests")).unwrap();
            fs::write(dir.join("tests/parse.rs"), test).unwrap();
        }
        git(&dir, &["init", "-q", "-b", "main"]);
        // The runtime's database lives in this directory. Adding it makes a later
        // wal write look like a change (Windows keeps `rtok.db-wal` dirty).
        git(&dir, &["add", "--", "lib.rs"]);
        if test.is_some() {
            git(&dir, &["add", "--", "tests/parse.rs"]);
        }
        git(&dir, &["commit", "-q", "-m", "init"]);
        (cx, dir)
    }

    #[test]
    fn review_marks_an_unreached_function_and_clears_it_when_a_test_calls_it() {
        let lib = "fn parse() {\n    let _ = 1;\n}\n";
        let (cx, dir) = repo("review-gap", lib, None);
        fs::write(dir.join("lib.rs"), "fn parse() {\n    let _ = 2;\n}\n").unwrap();
        let ctx = Ctx::new(&cx);
        let out = review(&ctx, &dir, Some("HEAD"), false, false).unwrap();
        assert!(out.contains("untested: parse"), "{out}");
        assert!(out.starts_with("review: "), "{out}");
        assert!(out.contains("lib.rs"), "{out}");
        assert_eq!(cx.store.measurement_count("graph").unwrap(), 0);
        let _ = fs::remove_dir_all(&dir);

        let (cx, dir) = repo(
            "review-tested",
            lib,
            Some("fn test_parse() {\n    parse();\n}\n"),
        );
        fs::write(dir.join("lib.rs"), "fn parse() {\n    let _ = 2;\n}\n").unwrap();
        let ctx = Ctx::new(&cx);
        let out = review(&ctx, &dir, Some("HEAD"), false, false).unwrap();
        assert!(!out.contains("untested: parse"), "{out}");
        assert_eq!(cx.store.measurement_count("graph").unwrap(), 0);
        let js = review(&ctx, &dir, Some("HEAD"), false, true).unwrap();
        let v: Value = serde_json::from_str(&js).unwrap();
        assert!(v.get("level").is_some(), "{js}");
        assert!(v.get("score").is_some(), "{js}");
        assert!(v["files"][0]["path"].as_str().unwrap().contains("lib.rs"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn only_definitions_a_hunk_touches_are_scored() {
        let lib = "fn first() {\n    let _ = 1;\n}\n\nfn second() {\n    let _ = 1;\n}\n";
        let (cx, dir) = repo("review-touched", lib, None);
        fs::write(
            dir.join("lib.rs"),
            "fn first() {\n    let _ = 1;\n}\n\nfn second() {\n    let _ = 2;\n}\n",
        )
        .unwrap();
        let out = review(&Ctx::new(&cx), &dir, Some("HEAD"), false, false).unwrap();
        assert!(out.contains("untested: second\n"), "{out}");
        assert!(!out.contains("first"), "{out}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn support_code_is_listed_unscored_and_an_attribute_overrides_the_directory() {
        let (cx, dir) = repo("review-support", "fn parse() {\n    let _ = 1;\n}\n", None);
        let files = ["bench/run.rs", "gen/out.rs", "examples/demo.rs"];
        for f in files {
            fs::create_dir_all(dir.join(f).parent().unwrap()).unwrap();
            fs::write(dir.join(f), "fn run() {\n    let _ = 1;\n}\n").unwrap();
        }
        fs::write(
            dir.join(".gitattributes"),
            "gen/** linguist-generated\nexamples/** -linguist-vendored\n",
        )
        .unwrap();
        git(&dir, &["add", "--", ".gitattributes"]);
        git(&dir, &[&["add", "--"][..], &files[..]].concat());
        git(&dir, &["commit", "-q", "-m", "support"]);
        for f in files {
            fs::write(dir.join(f), "fn run() {\n    let _ = 2;\n}\n").unwrap();
        }
        let js = review(&Ctx::new(&cx), &dir, Some("HEAD"), false, true).unwrap();
        let v: Value = serde_json::from_str(&js).unwrap();
        let by_path = |p: &str| {
            v["files"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["path"] == p)
                .unwrap_or_else(|| panic!("{p} missing: {js}"))
                .clone()
        };
        for p in ["bench/run.rs", "gen/out.rs"] {
            let f = by_path(p);
            assert_eq!(f["support"], true, "{js}");
            assert_eq!(f["score"], 0.0, "{js}");
            assert!(f["untested"].as_array().unwrap().is_empty(), "{js}");
        }
        let demo = by_path("examples/demo.rs");
        assert_eq!(demo["support"], false, "{js}");
        assert_eq!(demo["untested"][0], "run", "{js}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn review_without_a_git_repo_names_the_failure() {
        let (cx, dir) = cx("review-nogit");
        fs::write(dir.join("lib.rs"), "fn parse() {}\n").unwrap();
        let err = review(&Ctx::new(&cx), &dir, None, false, false)
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("git diff failed"), "{err}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn review_of_a_clean_tree_says_no_changes() {
        let (cx, dir) = repo("review-clean", "fn parse() {}\n", None);
        let out = review(&Ctx::new(&cx), &dir, Some("HEAD"), false, false).unwrap();
        assert_eq!(out, "no changes\n");
        let _ = fs::remove_dir_all(dir);
    }
}
