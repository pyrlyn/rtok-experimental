// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Disk usage walk shared by `agents junk` and the removal previews. A leaf module: it knows
//! nothing of agents or worktrees, so both can preview a removal without importing each other.

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

/// What one walk found. A folder it could not read or finish is a lower bound, never an error.
#[derive(Debug, Default, Clone, Copy)]
pub struct Usage {
    pub bytes: u64,
    /// Everything that is not a directory, a symlink included; the count a removal preview shows.
    pub files: u64,
    pub denied: bool,
    pub timed_out: bool,
}

/// How long one agent's folders may take to size before the rest is reported as a lower bound
/// (a huge cache on a network volume must not hang `list`).
pub const AGENT_SCAN_LIMIT: Duration = Duration::from_secs(10);

/// Disk usage, not apparent size: allocated blocks on Unix so sparse files and APFS clones are
/// not over-counted, a hard link once. Elsewhere it is the apparent size with every link
/// counted, because std has no stable file id to dedupe by there. A symlink costs itself and is
/// never followed, so a link out of an agent folder cannot pull foreign bytes into the total.
pub fn disk_usage(path: &Path) -> u64 {
    disk_usage_until(path, None).bytes
}

/// [`disk_usage`] that stops at `deadline` and reports a directory it was not allowed to read.
pub fn disk_usage_until(path: &Path, deadline: Option<Instant>) -> Usage {
    fn walk(path: &Path, seen: &mut HashSet<(u64, u64)>, deadline: Option<Instant>, u: &mut Usage) {
        if u.timed_out || deadline.is_some_and(|d| Instant::now() >= d) {
            u.timed_out = true;
            return;
        }
        let meta = match std::fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                u.denied |= e.kind() == std::io::ErrorKind::PermissionDenied;
                return;
            }
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.nlink() < 2 || seen.insert((meta.dev(), meta.ino())) {
                u.bytes += meta.blocks() * 512;
            }
        }
        #[cfg(not(unix))]
        {
            let _ = &seen;
            u.bytes += meta.len();
        }
        u.files += u64::from(!meta.is_dir());
        if meta.is_dir() {
            match std::fs::read_dir(path) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        walk(&entry.path(), seen, deadline, u);
                    }
                }
                Err(e) => u.denied |= e.kind() == std::io::ErrorKind::PermissionDenied,
            }
        }
    }
    let mut usage = Usage::default();
    walk(path, &mut HashSet::new(), deadline, &mut usage);
    usage
}
