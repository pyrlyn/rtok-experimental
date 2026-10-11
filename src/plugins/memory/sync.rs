// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok memory sync` — a managed block in `CLAUDE.md` / `AGENTS.md` (T69.6).
//!
//! Not a hook: writing a host file on SessionStart would break fail-open.

use anyhow::{Result, bail};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::tokens::{self, Class};

pub const START: &str = "<!-- rtok:memory -->";
pub const END: &str = "<!-- /rtok:memory -->";

/// T59.7 overlap line: the same titles in the synced block and hook recall.
pub fn overlap_line() -> &'static str {
    "duplicate: a memory sync block and hook recall both carry titles — rtok side: [plugins.memory] enabled = false"
}

pub fn has_block(text: &str) -> bool {
    matches!(block_range(text), Ok(Some(_)))
}

fn kv_key(path: &Path) -> String {
    format!(
        "memory.sync.sha:{}",
        path.file_name().unwrap_or_default().to_string_lossy()
    )
}

/// Pinned-first, then remaining live notes by id desc, trimmed to `budget` tokens.
pub fn render(notes: &[(i32, String)], budget: u32, estimate: impl Fn(&str) -> u32) -> String {
    let mut entries = notes.to_vec();
    loop {
        let mut lines = Vec::with_capacity(entries.len() + 2);
        lines.push(START.to_string());
        for (id, title) in &entries {
            lines.push(format!("{id} {title}"));
        }
        lines.push(END.to_string());
        let text = lines.join("\n");
        if estimate(&text) <= budget || entries.is_empty() {
            return text;
        }
        entries.pop();
    }
}

fn block_range(text: &str) -> Result<Option<(usize, usize)>> {
    let Some(start) = text.find(START) else {
        return Ok(None);
    };
    let rest = &text[start + START.len()..];
    if rest.contains(START) {
        bail!("multiple rtok memory blocks");
    }
    let Some(rel) = rest.find(END) else {
        bail!("unclosed rtok memory block");
    };
    Ok(Some((start, start + START.len() + rel + END.len())))
}

/// Replace or remove the managed block. Bytes outside the markers are unchanged.
pub fn splice(file: &str, block: Option<&str>) -> Result<String> {
    match (block_range(file)?, block) {
        (None, None) => Ok(file.to_string()),
        (None, Some(b)) => {
            let mut out = file.to_string();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(b);
            if !out.ends_with('\n') {
                out.push('\n');
            }
            Ok(out)
        }
        (Some((a, b)), None) => Ok(format!("{}{}", &file[..a], &file[b..])),
        (Some((a, b)), Some(block)) => Ok(format!("{}{}{}", &file[..a], block, &file[b..])),
    }
}

pub fn guarded_splice(
    file: &str,
    block: Option<&str>,
    stored_sha: Option<&str>,
    force: bool,
) -> Result<String> {
    if let Some((a, b)) = block_range(file)? {
        let sha = crate::store::hex_sha256(&file.as_bytes()[a..b]);
        if !force && stored_sha != Some(sha.as_str()) {
            bail!("memory block was hand-edited; pass --force to overwrite");
        }
    }
    splice(file, block)
}

pub fn run(
    cfg: &Config,
    file: PathBuf,
    budget: Option<u32>,
    dry_run: bool,
    remove: bool,
    force: bool,
) -> Result<()> {
    let name = file.file_name().and_then(OsStr::to_str).unwrap_or("");
    if name != "CLAUDE.md" && name != "AGENTS.md" {
        bail!("--file must be CLAUDE.md or AGENTS.md");
    }
    let path = if file.is_absolute() {
        file
    } else {
        std::env::current_dir()?.join(&file)
    };
    let before = std::fs::read_to_string(&path).unwrap_or_default();
    let cx = crate::plugin::Runtime::open(cfg.clone(), "memory")?;
    let key = kv_key(&path);
    let stored = cx.store.kv_get(&key)?;
    let budget = budget.unwrap_or(cfg.plugins.memory.sync_tokens);
    let estimate = |s: &str| tokens::estimate(s, Class::Prose, &cfg.estimator);
    let new_block = if remove {
        None
    } else {
        let project = std::env::current_dir()
            .ok()
            .and_then(|d| super::project_name(&d));
        let notes = cx.store.list_note_titles(project.as_deref(), u32::MAX)?;
        Some(render(&notes, budget, estimate))
    };
    let after = guarded_splice(&before, new_block.as_deref(), stored.as_deref(), force)?;
    if dry_run {
        let diff = change_preview::unified_diff(&path, &before, &after);
        crate::log::stdout(&crate::stat_preview::show_file(&path, &diff));
    } else {
        if after != before {
            rtok_agent_sdk::backup(&path, cfg.setup.backup_files as usize)?;
            rtok_agent_sdk::write(
                &rtok_agent_sdk::Apply::default(),
                &path,
                &after,
                "memory sync",
            )?;
        }
        match &new_block {
            Some(b) => cx
                .store
                .kv_set(&key, &crate::store::hex_sha256(b.as_bytes()))?,
            None => cx.store.kv_delete(&key)?,
        }
    }
    let recall_on = cfg.plugin_enabled("memory", true) && cfg.plugins.memory.recall_tokens > 0;
    if !remove && recall_on {
        crate::log::stdout_ln(overlap_line());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Vfs;

    fn est(s: &str) -> u32 {
        s.len() as u32
    }

    fn notes() -> Vec<(i32, String)> {
        vec![
            (3, "pinned-high".into()),
            (2, "newer".into()),
            (1, "older".into()),
        ]
    }

    fn vfs_file(body: &str) -> Vfs {
        let mut v = Vfs::new();
        v.write("CLAUDE.md", body.as_bytes());
        v
    }

    #[test]
    fn vfs_create_appends_block_at_end() {
        let mut v = vfs_file("# intro\nkeep me\n");
        let block = render(&notes(), 10_000, est);
        let after =
            guarded_splice(v.read_str("CLAUDE.md").unwrap(), Some(&block), None, false).unwrap();
        v.write("CLAUDE.md", after.as_bytes());
        let got = v.read_str("CLAUDE.md").unwrap();
        assert!(got.starts_with("# intro\nkeep me\n"), "{got}");
        assert!(got.contains("3 pinned-high\n2 newer\n1 older\n"), "{got}");
        assert!(got.ends_with(&format!("{END}\n")), "{got}");
    }

    #[test]
    fn vfs_update_leaves_outside_bytes_identical() {
        let block1 = render(&[(1, "a".into())], 10_000, est);
        let file = splice("pre\n", Some(&block1)).unwrap() + "post\n";
        let block2 = render(&[(2, "b".into())], 10_000, est);
        let sha = crate::store::hex_sha256(block1.as_bytes());
        let after = guarded_splice(&file, Some(&block2), Some(&sha), false).unwrap();
        assert!(after.starts_with("pre\n"), "{after}");
        assert!(after.ends_with("post\n"), "{after}");
        assert!(after.contains("2 b"), "{after}");
        assert!(!after.contains("1 a"), "{after}");
    }

    #[test]
    fn vfs_hand_edit_refused_unless_force() {
        let block = render(&[(1, "title".into())], 10_000, est);
        let file = splice("pre\n", Some(&block)).unwrap();
        let sha = crate::store::hex_sha256(block.as_bytes());
        let edited = file.replace("1 title", "1 edited");
        let mut v = vfs_file(&edited);
        let err = guarded_splice(
            v.read_str("CLAUDE.md").unwrap(),
            Some(&block),
            Some(&sha),
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("--force"), "{err}");
        let after = guarded_splice(
            v.read_str("CLAUDE.md").unwrap(),
            Some(&block),
            Some(&sha),
            true,
        )
        .unwrap();
        v.write("CLAUDE.md", after.as_bytes());
        assert!(v.read_str("CLAUDE.md").unwrap().contains("1 title"));
    }

    #[test]
    fn vfs_remove_deletes_only_the_block() {
        let block = render(&[(1, "x".into())], 10_000, est);
        let file = format!("alpha\n{block}omega\n");
        let sha = crate::store::hex_sha256(block.as_bytes());
        let after = guarded_splice(&file, None, Some(&sha), false).unwrap();
        let v = vfs_file(&after);
        let got = v.read_str("CLAUDE.md").unwrap();
        assert_eq!(got, "alpha\nomega\n");
        assert!(!has_block(got));
    }

    #[test]
    fn vfs_budget_trims_from_the_end() {
        let block = render(&notes(), 65, est);
        assert!(block.contains("3 pinned-high"), "{block}");
        assert!(block.contains("2 newer"), "{block}");
        assert!(!block.contains("1 older"), "{block}");
        assert!(est(&block) <= 65, "{} {block}", est(&block));
    }

    #[test]
    fn unchanged_store_is_byte_stable() {
        let a = render(&notes(), 300, est);
        let b = render(&notes(), 300, est);
        assert_eq!(a, b);
        assert!(!a.contains("20"), "{a}");
    }
}
