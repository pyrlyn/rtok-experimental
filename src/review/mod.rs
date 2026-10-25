// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok review`: token-efficient code review context (see `src/review/README.md`).
//!
//! Diff in → changed files and symbols ([`diff`]) → files that use them or that they use,
//! from the graph index ([`usage`]) → signature digests with no bodies ([`digest`]) →
//! one budgeted, deterministic prompt ([`prompt`]) → a reviewer model ([`provider`]).

pub mod diff;
pub mod digest;
pub mod imports;
pub mod prompt;
pub mod provider;
pub mod usage;

use std::path::Path;

use anyhow::Result;
use rtok_plugin_sdk::{Class, Ctx};
use serde_json::{Value, json};

use prompt::{Prompt, Section};
use usage::Related;

/// What context goes next to the diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum ContextMode {
    /// Digests of changed and related files (the point of this command).
    #[default]
    Digest,
    /// The diff alone (a baseline for evaluation).
    None,
    /// Changed and related files in full (a baseline for evaluation).
    Full,
}

/// Knobs for [`build`].
#[derive(Debug, Clone)]
pub struct Options {
    pub budget: Option<u32>,
    pub max_related: usize,
    pub mode: ContextMode,
    /// Same-name definitions above which an unproven link is ambiguous (ranked last).
    pub max_defs: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            budget: None,
            max_related: 12,
            mode: ContextMode::Digest,
            max_defs: usage::MAX_DEFS,
        }
    }
}

/// Everything [`build`] found, for the JSON report.
#[derive(Debug, Clone)]
pub struct Built {
    pub files: Vec<diff::FileDiff>,
    pub changed: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    pub related: Vec<Related>,
    pub prompt: Prompt,
}

fn read(root: &Path, path: &str) -> Option<String> {
    std::fs::read_to_string(root.join(path)).ok()
}

/// Index `root` (as `rtok graph` does), map the diff onto it and assemble the prompt.
pub fn build(cx: &Ctx, root: &Path, diff_text: &str, opts: &Options) -> Result<Built> {
    let files = diff::parse(diff_text);
    crate::plugins::graph::index_for(cx, root)?;
    let key = crate::plugins::graph::index::canon(root);
    let changed = usage::changed_symbols(cx, &key, &files);
    let mut related = usage::related(
        cx,
        &key,
        root,
        &changed,
        usage::Search {
            max_defs: opts.max_defs,
        },
    )?;
    related.truncate(opts.max_related);
    let mut sections = Vec::new();
    if opts.mode != ContextMode::None {
        for f in files.iter().filter(|f| !f.deleted) {
            if let Some(src) = read(root, &f.path) {
                let body = match opts.mode {
                    ContextMode::Full => src,
                    _ => digest::digest(
                        &f.path,
                        &src,
                        &cx.symbol_file_defs(&key, &f.path).unwrap_or_default(),
                        None,
                    ),
                };
                sections.push(Section {
                    path: f.path.clone(),
                    role: "changed".into(),
                    body,
                });
            }
        }
        for r in &related {
            if let Some(src) = read(root, &r.path) {
                let body = match opts.mode {
                    ContextMode::Full => src,
                    _ => digest::digest(
                        &r.path,
                        &src,
                        &cx.symbol_file_defs(&key, &r.path).unwrap_or_default(),
                        Some(&r.focus),
                    ),
                };
                let role = format!("{:?}", r.relation).to_lowercase();
                sections.push(Section {
                    path: r.path.clone(),
                    role,
                    body,
                });
            }
        }
    }
    let prompt = prompt::assemble(diff_text, &sections, opts.budget, |s| {
        cx.estimate(s, Class::Code)
    });
    Ok(Built {
        files,
        changed,
        related,
        prompt,
    })
}

/// Right-hand side of a `git diff` range: `A..B` and `A...B` give `B` (`HEAD` when empty);
/// a single revision is compared with the working tree, which gives `None`.
pub fn rhs_of(range: &str) -> Option<String> {
    let rhs = range
        .split_once("...")
        .or_else(|| range.split_once(".."))
        .map(|(_, b)| b)?;
    Some(if rhs.is_empty() {
        "HEAD".to_string()
    } else {
        rhs.to_string()
    })
}

fn git_line(root: &Path, args: &[&str]) -> Result<String> {
    let out = crate::plugins::graph::git_stdout_result(root, args)?;
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

/// Where tree checkouts for `--range` are cached: `RTOK_REVIEW_TREES`, else the temp dir.
pub fn trees_dir() -> std::path::PathBuf {
    std::env::var_os("RTOK_REVIEW_TREES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("rtok-review-trees"))
}

/// The tree to index for `range`: the post-change side, so hunks line up with the index.
/// That is `root` itself when the right-hand side is the working tree, or `HEAD` of a clean
/// checkout; otherwise a detached `git worktree` at that commit under [`trees_dir`], named
/// by SHA and reused by later runs (the graph index keys on the path, so it is cached by
/// commit too). Remove them with `git worktree remove` / `git worktree prune`.
pub fn tree_for(root: &Path, range: &str) -> Result<std::path::PathBuf> {
    tree_for_in(root, range, &trees_dir())
}

/// [`tree_for`] with an explicit cache directory.
pub fn tree_for_in(root: &Path, range: &str, cache: &Path) -> Result<std::path::PathBuf> {
    let Some(rhs) = rhs_of(range) else {
        return Ok(root.to_path_buf());
    };
    let sha = git_line(
        root,
        &["rev-parse", "--verify", &format!("{rhs}^{{commit}}")],
    )?;
    let head = git_line(root, &["rev-parse", "HEAD"]).unwrap_or_default();
    let clean = git_line(root, &["status", "--porcelain", "--untracked-files=no"])
        .map(|s| s.is_empty())
        .unwrap_or(false);
    if sha == head && clean {
        return Ok(root.to_path_buf());
    }
    let dir = cache.join(&sha[..sha.len().min(16)]);
    if !dir.join(".git").exists() {
        std::fs::create_dir_all(cache)?;
        let d = dir.to_string_lossy().into_owned();
        crate::plugins::graph::git_stdout_result(
            root,
            &["worktree", "add", "--detach", "--quiet", &d, &sha],
        )?;
    }
    Ok(dir)
}

/// JSON report: what was found, prompt accounting, and the review when one ran.
pub fn report(b: &Built, review: Option<(&str, &str)>) -> Value {
    json!({
        "files": b.files.iter().map(|f| json!({
            "path": f.path, "hunks": f.hunks.len(), "added": f.added, "deleted": f.deleted,
            "symbols": b.changed.get(&f.path),
        })).collect::<Vec<_>>(),
        "related": b.related,
        "prompt": b.prompt,
        "reviewer": review.map(|r| r.0),
        "review": review.map(|r| r.1),
    })
}

#[cfg(test)]
mod tests;
