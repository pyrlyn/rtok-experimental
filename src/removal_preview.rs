// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The deletion commands' preview (T416.2): one `- path  size  N files` line per path and the
//! totals line. A removal's line already is its stat, so `--stat` and the default dry run print
//! the same thing.

use std::path::Path;

use change_preview::{Mode, Preview};

use crate::disk_usage::disk_usage_until;

/// How many bytes and files a removal of `path` takes, counted before it goes.
pub fn size_of(path: &Path) -> (u64, u64) {
    let usage = disk_usage_until(path, None);
    (usage.bytes, usage.files)
}

/// The preview of removing each `(path, bytes, files)`; empty when nothing is removed.
pub fn render<'a>(rows: impl IntoIterator<Item = (&'a str, u64, u64)>) -> String {
    let mut preview = Preview::default();
    for (path, bytes, files) in rows {
        preview.remove(path, bytes, files);
    }
    change_preview::paint(&preview.render(Mode::Stat))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_removal_prints_its_line_and_the_totals() {
        let out = render([("/w/a", 2048, 3), ("/w/b", 1024, 1)]);
        insta::assert_snapshot!(out, @r"
        - /w/a  2.0 KB  3 files
        - /w/b  1.0 KB  1 file
        2 paths, 4 files, -3.0 KB
        ");
        assert_eq!(render([]), "");
    }

    #[test]
    fn files_counts_files_not_directories() {
        let dir = crate::testutil::tmp_dir("removal-files");
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/x"), "x").unwrap();
        std::fs::write(dir.join("a/b/y"), "y").unwrap();
        assert_eq!(size_of(&dir).1, 2);
    }
}
