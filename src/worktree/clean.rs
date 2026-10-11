// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok worktree clean` (T152): delete idle tagged build caches and keep the worktrees.
//! The one deletion in rtok that `expand` cannot undo, allowed because a directory
//! carrying a valid `CACHEDIR.TAG` holds no source by definition and the next build
//! recreates it; nothing untagged is ever touched. [`kept_because`] is pure; [`run`]
//! deletes one cache root at a time.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use rtok_sys::proc::{ProgressRunner, Unit};
use serde::Serialize;

use super::list::{Cache, orphans, usage};
use super::{inventory, noted_table};
use crate::render::{Col, duration};
use change_preview::human_bytes;

pub struct Policy {
    pub idle: Duration,
    pub now: SystemTime,
}

/// Why a cache stays, or `None` when it goes. `current` says the cache sits in the
/// worktree this command runs from; `explicit` that its worktree was named on the
/// command line.
pub fn kept_because(cache: &Cache, current: bool, explicit: bool, p: &Policy) -> Option<String> {
    if current && !explicit {
        return Some("the worktree this command runs from; name its path to clean it".into());
    }
    let age = |m: SystemTime| p.now.duration_since(m).unwrap_or_default();
    let active = cache.modified.is_some_and(|m| age(m) < p.idle);
    active.then(|| "modified within the idle window".into())
}

#[derive(Debug, Serialize)]
pub struct Outcome {
    pub worktree: String,
    /// The cache root, relative to its worktree (`target`).
    pub cache: String,
    pub bytes: u64,
    /// Files in the cache, counted before it is deleted; 0 for a cache that stays.
    pub files: u64,
    pub modified_unix: Option<u64>,
    /// `clean` or `keep`.
    pub action: &'static str,
    /// Why it was kept, what was done, or the OS error when the deletion failed.
    pub note: String,
    pub failed: bool,
}

fn real(p: &Path) -> PathBuf {
    crate::fs::canon(p)
}

/// Every worktree git lists plus the orphans (T151), or exactly the paths named — each
/// of which must be one of those, so nothing outside the repository is ever walked.
fn targets(cwd: &Path, paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let entries = inventory(cwd)?;
    let mut dirs: Vec<PathBuf> = entries.iter().map(|e| e.record.path.clone()).collect();
    dirs.extend(orphans(&entries));
    if paths.is_empty() {
        return Ok(dirs);
    }
    let known: Vec<PathBuf> = dirs.iter().map(|d| real(d)).collect();
    let named = paths.iter().map(|p| {
        ensure!(
            known.iter().any(|k| crate::fs::same_path(k, &real(p))),
            "{} is not a worktree of this repository",
            p.display()
        );
        Ok(p.clone())
    });
    named.collect()
}

pub fn run(cwd: &Path, paths: &[PathBuf], policy: &Policy, yes: bool) -> Result<Vec<Outcome>> {
    let here = real(cwd);
    let explicit = !paths.is_empty();
    let unix = |t: SystemTime| t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs());
    let mut judged = Vec::new();
    for dir in targets(cwd, paths)? {
        let current = here.starts_with(real(&dir));
        for cache in usage(&dir).caches {
            let kept = kept_because(&cache, current, explicit, policy);
            judged.push((dir.clone(), cache, kept));
        }
    }
    let count = judged
        .iter()
        .filter(|(.., kept)| yes && kept.is_none())
        .count();
    // A deletion is silent and can take long; a bar over the caches shows it is moving (T276.2).
    let mut bar =
        (count > 1).then(|| ProgressRunner::batch("cleaning caches…", count as u64, Unit::Items));
    let mut outcomes = Vec::with_capacity(judged.len());
    for (dir, cache, kept) in judged {
        let relative = cache.path.strip_prefix(&dir).unwrap_or(&cache.path);
        // Counted before the delete, so the preview and the report agree on what went.
        let files = if kept.is_none() {
            crate::removal_preview::size_of(&cache.path).1
        } else {
            0
        };
        let done = (yes && kept.is_none()).then(|| {
            if let Some(bar) = bar.as_mut() {
                bar.step(&format!("{} {}", dir.display(), relative.display()));
            }
            std::fs::remove_dir_all(&cache.path)
                .with_context(|| format!("cannot delete {}", cache.path.display()))
        });
        outcomes.push(Outcome {
            worktree: dir.display().to_string(),
            cache: relative.display().to_string(),
            bytes: cache.bytes,
            files,
            modified_unix: cache.modified.and_then(unix),
            action: if kept.is_some() { "keep" } else { "clean" },
            failed: matches!(done, Some(Err(_))),
            note: match (kept, done) {
                (Some(why), _) => why,
                (None, Some(Ok(()))) => "deleted".into(),
                (None, Some(Err(e))) => format!("failed, kept: {e:#}"),
                (None, None) => "tagged cache, idle".into(),
            },
        });
    }
    if let Some(bar) = bar {
        bar.finish();
    }
    Ok(outcomes)
}

/// The removal lines of the caches a `--yes` would delete.
fn preview(outcomes: &[Outcome]) -> String {
    let paths: Vec<(String, u64, u64)> = outcomes
        .iter()
        .filter(|o| o.action == "clean" && !o.failed)
        .map(|o| (format!("{}/{}", o.worktree, o.cache), o.bytes, o.files))
        .collect();
    crate::removal_preview::render(paths.iter().map(|(p, b, f)| (p.as_str(), *b, *f)))
}

pub fn to_table(outcomes: &[Outcome], yes: bool, now: SystemTime) -> String {
    let now = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let head = ["action", "worktree", "cache", "modified", "bytes"];
    let mut lines = vec![head.map(String::from).to_vec()];
    lines.extend(outcomes.iter().map(|o| {
        let age = o
            .modified_unix
            .map(|t| duration(now.saturating_sub(t) as i64));
        vec![
            o.action.into(),
            o.worktree.clone(),
            o.cache.clone(),
            age.unwrap_or_else(|| "-".into()),
            human_bytes(o.bytes),
        ]
    }));
    let cols = [
        Col::left(0),
        Col::left(0),
        Col::left(0),
        Col::left(0),
        Col::right(0),
    ];
    let mut out = noted_table(&cols, &lines, outcomes.iter().map(|o| o.note.as_str()));
    let cleaned = outcomes.iter().filter(|o| o.action == "clean" && !o.failed);
    let (n, bytes) = cleaned.fold((0, 0), |(n, b), o| (n + 1, b + o.bytes));
    let planned = preview(outcomes);
    if !yes && !planned.is_empty() {
        out.push('\n');
        out.push_str(&planned);
    }
    out.push_str(&match (yes, n) {
        (false, 0) => "\nnothing to clean\n".to_string(),
        (false, n) => format!(
            "\ndry run: {n} caches, {} to free, nothing changed; rerun with --yes\n",
            human_bytes(bytes)
        ),
        (true, n) => format!(
            "\nfreed {} in {n} caches (logical bytes)\n",
            human_bytes(bytes)
        ),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[rstest]
    #[case::idle(0, false, false, None)]
    #[case::idle_and_named(0, true, true, None)]
    #[case::never_written(u64::MAX, false, false, None)]
    #[case::active(950, false, false, Some("modified within the idle window"))]
    #[case::active_and_named(950, true, true, Some("modified within the idle window"))]
    #[case::current(
        0,
        true,
        false,
        Some("the worktree this command runs from; name its path to clean it")
    )]
    fn only_an_idle_cache_outside_the_current_worktree_goes(
        #[case] modified: u64,
        #[case] current: bool,
        #[case] explicit: bool,
        #[case] want: Option<&str>,
    ) {
        let cache = Cache {
            path: "/w/x/target".into(),
            bytes: 1,
            modified: (modified != u64::MAX).then(|| at(modified)),
        };
        let policy = Policy {
            idle: Duration::from_secs(100),
            now: at(1_000),
        };
        let got = kept_because(&cache, current, explicit, &policy);
        assert_eq!(got.as_deref(), want);
    }

    #[test]
    fn the_table_sums_only_what_was_or_would_be_deleted() {
        let row = |action, cache: &str, bytes, note: &str, failed| Outcome {
            worktree: "/w/x".into(),
            cache: cache.into(),
            bytes,
            files: 7,
            modified_unix: Some(1_000),
            action,
            note: note.into(),
            failed,
        };
        let rows = [
            row("clean", "target", 2048, "tagged cache, idle", false),
            row(
                "keep",
                "node_modules/.cache",
                512,
                "modified within the idle window",
                false,
            ),
            row(
                "clean",
                "build",
                1024,
                "failed, kept: cannot delete /w/x/build",
                true,
            ),
        ];
        let now = at(1_000 + 3 * 86_400 + 4 * 3_600);
        insta::assert_snapshot!(to_table(&rows, false, now), @r"
        action worktree cache               modified  bytes  note
        clean  /w/x     target              3d04h    2.0 KB  tagged cache, idle
        keep   /w/x     node_modules/.cache 3d04h     512 B  modified within the idle window
        clean  /w/x     build               3d04h    1.0 KB  failed, kept: cannot delete /w/x/build

        - /w/x/target  2.0 KB  7 files
        1 path, 7 files, -2.0 KB
        dry run: 1 caches, 2.0 KB to free, nothing changed; rerun with --yes
        ");
        assert!(to_table(&rows, true, now).ends_with("freed 2.0 KB in 1 caches (logical bytes)\n"));
        assert!(to_table(&rows[1..2], false, now).ends_with("\nnothing to clean\n"));
    }
}
