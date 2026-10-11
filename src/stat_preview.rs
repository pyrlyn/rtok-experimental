// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `--stat` for the dry runs that show a diff (T416.3): the per-file stat lines and the totals
//! line instead of the diff, and the same totals line closing the default diff.
//!
//! The commands keep producing the text they always did (a unified diff, or an installer's
//! `+`/`-` step lines), because the code that builds it has no before/after pair to hand over.
//! This module counts the marked lines and lets `change_preview` format them, so the stat and
//! totals wording stays the crate's.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use change_preview::{Mode, Preview};

/// A process-wide switch rather than a parameter: the flag is read deep inside the installers'
/// block renderer and the doctor report, and a config key would put a CLI-only toggle in the
/// reference file.
static STAT: AtomicBool = AtomicBool::new(false);

pub fn set(on: bool) {
    STAT.store(on, Ordering::Relaxed);
}

pub fn on() -> bool {
    STAT.load(Ordering::Relaxed)
}

/// Text with the colour escapes removed, so a painted diff can be counted.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for e in chars.by_ref() {
                if e.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Rebuild a before/after pair from `+`/`-`/` ` lines. Headers, hunk markers and unmarked
/// lines (an installer's continuation text) are not part of either side.
fn sides(text: &str) -> (String, String) {
    let (mut before, mut after) = (String::new(), String::new());
    for line in plain(text).lines() {
        if line.starts_with("+++") || line.starts_with("---") || line.starts_with('@') {
            continue;
        }
        let put = |s: &mut String| {
            s.push_str(&line[1..]);
            s.push('\n');
        };
        match line.chars().next() {
            Some('+') => put(&mut after),
            Some('-') => put(&mut before),
            Some(' ') => {
                put(&mut before);
                put(&mut after);
            }
            _ => {}
        }
    }
    (before, after)
}

fn preview(items: &[(PathBuf, String)]) -> Preview {
    let mut p = Preview::default();
    for (path, text) in items {
        let (before, after) = sides(text);
        p.edit(path, &before, &after);
    }
    p
}

/// What to print for `items` (a path and the text that shows its change): with `--stat` the
/// stat lines and totals, otherwise `shown` (the diff the caller renders itself) and the totals
/// line under it. Empty when nothing changes.
pub fn show(items: &[(PathBuf, String)], shown: &str) -> String {
    let p = preview(items);
    if p.is_empty() {
        return shown.to_string();
    }
    if on() {
        return change_preview::paint(p.render(Mode::Stat).trim_end());
    }
    // Without edit diffs `Diff` mode prints the totals line alone.
    let totals = p.render(Mode::Diff);
    let totals = totals.lines().last().unwrap_or_default();
    if shown.is_empty() {
        totals.to_string()
    } else {
        format!("{}\n{totals}", shown.trim_end())
    }
}

/// [`show`] for one file's plain unified diff, painted when it is shown.
pub fn show_file(path: &Path, diff: &str) -> String {
    show(&[(path.to_path_buf(), diff.to_string())], &shown(diff))
}

/// `diff`, painted, unless `--stat` replaces it.
pub fn shown(diff: &str) -> String {
    if on() {
        String::new()
    } else {
        change_preview::paint(diff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "--- a/f\n+++ b/f\n@@ -1,2 +1,3 @@\n keep\n-old\n+new\n+more\n";

    #[test]
    fn totals_close_the_diff() {
        set(false);
        let out = show_file(Path::new("f"), DIFF);
        assert!(out.starts_with("--- a/f"), "{out}");
        assert!(
            out.ends_with("1 file changed, 2 insertions(+), 1 deletion(-)"),
            "{out}"
        );
    }

    #[test]
    fn stat_replaces_the_diff() {
        set(true);
        let out = show_file(Path::new("f"), DIFF);
        set(false);
        assert!(!out.contains("@@"), "{out}");
        assert!(out.contains("f | 3 ++-"), "{out}");
        assert!(out.contains("1 file changed"), "{out}");
    }

    #[test]
    fn a_painted_diff_counts_the_same() {
        let painted = DIFF.replace("+new", "\u{1b}[32m+new\u{1b}[0m");
        assert_eq!(sides(&painted), sides(DIFF));
    }

    #[test]
    fn nothing_changed_prints_nothing() {
        assert_eq!(show_file(Path::new("f"), ""), "");
    }
}
