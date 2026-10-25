// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use crate::config::Config;
use anyhow::Result;
use clap::Subcommand;
use std::io::{self, Read, Write};
use std::path::PathBuf;

/// `rtok archive rewrite` — the pi `context` carrier (T70.2): the same live-zone
/// rewrite the proxy runs, driven over a pi message array on stdin.
#[cfg(feature = "archive")]
#[derive(Subcommand)]
pub(super) enum ArchiveCmd {
    /// Rewrite old tool results to archive pointers; message array JSON on stdin,
    /// rewritten array on stdout (input bytes echoed when nothing is eligible)
    Rewrite {
        /// Read the message array from stdin (pi `context` event)
        #[arg(long)]
        stdin: bool,
    },
    /// Delete old and surplus archive bodies and old calls, compact the store, print what
    /// was freed (the limits are `core.retain_archive_days`, `max_archive_mb`, `max_store_mb`)
    Prune {
        /// Print what would be deleted; delete nothing
        #[arg(long)]
        dry_run: bool,
    },
}

pub(super) fn run(config_file: &Option<PathBuf>, action: ArchiveCmd) -> Result<()> {
    let cfg = Config::load_with(config_file.as_deref(), None)?;
    match action {
        ArchiveCmd::Rewrite { stdin: _ } => {
            let cx = crate::plugin::Runtime::open(cfg, "archive-rewrite")?;
            let mut buf = Vec::new();
            let _ = io::stdin().read_to_end(&mut buf);
            let out =
                crate::plugins::archive::pi::rewrite_stdin(&buf, &crate::plugin::Ctx::new(&cx))?;
            io::stdout().write_all(&out)?;
        }
        ArchiveCmd::Prune { dry_run } => {
            let store = crate::store::Store::open(&cfg.core.db_path)?;
            let policy = crate::store::prune_policy(&cfg);
            let r = store.prune(&cfg.core.db_path, &policy, dry_run)?;
            println!("{}", prune_text(&r, dry_run));
        }
    }

    Ok(())
}

fn prune_text(r: &crate::store::PruneReport, dry_run: bool) -> String {
    use change_preview::human_bytes;
    let (verb, calls) = match dry_run {
        true => ("would delete", "not previewed".to_string()),
        false => ("deleted", format!("deleted {}", r.calls)),
    };
    let freed = r.store_before.saturating_sub(r.store_after);
    format!(
        "archive {verb} {} bodies ({})\ncalls {calls}\nstore {} -> {} (freed {})",
        r.archive_entries,
        human_bytes(r.archive_bytes),
        human_bytes(r.store_before),
        human_bytes(r.store_after),
        human_bytes(freed),
    )
}
