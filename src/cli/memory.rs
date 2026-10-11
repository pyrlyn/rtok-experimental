// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use crate::config::Config;
use crate::render::with_loader;
use crate::ui::style;
use anyhow::Result;
use clap::Subcommand;
use std::io;
use std::path::PathBuf;

#[cfg(feature = "memory")]
#[derive(Subcommand)]
pub(super) enum MemoryCmd {
    /// Import `{kind,title,body}` JSONL; dedupe by body sha256
    Import {
        file: std::path::PathBuf,
        /// Count what would be imported and write nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Print every note but session checkpoints as the JSONL `import` reads
    Export {
        /// Only notes of this project
        #[arg(long)]
        project: Option<String>,
    },
    // T69.1
    /// Retire a note: a tombstone — never recalled or searched, body kept
    Retire {
        id: i32,
        /// The replacement note id this one is superseded by
        #[arg(long)]
        superseded_by: Option<i32>,
    },
    // T69.1
    /// Pin a note so it leads SessionStart recall
    Pin { id: i32 },
    // T69.1
    /// Drop a note back to newest-first recall order
    Unpin { id: i32 },
    // T472
    /// Print the earlier title and body kept when an upsert changed this note
    History { id: i32 },
    // T69.1
    /// Save a replacement (title, body) for a note and retire the old row
    Revise {
        id: i32,
        /// The replacement title
        #[arg(long)]
        title: String,
        /// The replacement body
        #[arg(long)]
        body: String,
    },
    // T69.6
    /// Write pinned-then-newest titles into a managed CLAUDE.md / AGENTS.md block
    Sync {
        /// CLAUDE.md or AGENTS.md
        #[arg(long, default_value = "CLAUDE.md")]
        file: std::path::PathBuf,
        /// Token budget; default `[plugins.memory] sync_tokens`
        #[arg(long)]
        budget: Option<u32>,
        /// Print the unified diff and write nothing
        #[arg(long)]
        dry_run: bool,
        // T416.3
        /// With --dry-run: stat lines and totals instead of the diff
        #[arg(long, requires = "dry_run")]
        stat: bool,
        /// Delete the managed block and nothing else
        #[arg(long)]
        remove: bool,
        /// Overwrite a hand-edited block
        #[arg(long)]
        force: bool,
    },
    // T69.4
    /// Notes live/pinned/retired, recall and MCP call counts
    Status {
        /// Only notes of this project
        #[arg(long)]
        project: Option<String>,
        /// Window for recalls and MCP calls (`30d`, `24h`)
        #[arg(long)]
        since: Option<String>,
        /// JSON instead of the table
        #[arg(long)]
        json: bool,
    },
}

pub(super) fn run(config_file: &Option<PathBuf>, action: MemoryCmd) -> Result<()> {
    let cfg = Config::load_with(config_file.as_deref(), None)?;
    match action {
        MemoryCmd::Import { file, dry_run } => {
            println!(
                "{}",
                crate::plugins::memory::import::run(&cfg, &file, dry_run)?
            );
        }
        MemoryCmd::Export { project } => {
            let mut out = io::stdout().lock();
            crate::plugins::memory::export::run(&cfg, project.as_deref(), &mut out)?;
        }
        MemoryCmd::Retire { id, superseded_by } => {
            let cx = crate::plugin::Runtime::open(cfg, "memory")?;
            println!(
                "{}",
                crate::plugins::memory::mem_update(&cx, id, true, superseded_by, None)?
            );
        }
        MemoryCmd::Pin { id } => {
            let cx = crate::plugin::Runtime::open(cfg, "memory")?;
            println!(
                "{}",
                crate::plugins::memory::mem_update(&cx, id, false, None, Some(true))?
            );
        }
        MemoryCmd::Unpin { id } => {
            let cx = crate::plugin::Runtime::open(cfg, "memory")?;
            println!(
                "{}",
                crate::plugins::memory::mem_update(&cx, id, false, None, Some(false))?
            );
        }
        MemoryCmd::History { id } => {
            let cx = crate::plugin::Runtime::open(cfg, "memory")?;
            println!("{}", crate::plugins::memory::mem_history(&cx, id)?);
        }
        MemoryCmd::Revise { id, title, body } => {
            let cx = crate::plugin::Runtime::open(cfg, "memory")?;
            let (new, retired) = crate::plugins::memory::mem_revise(&cx, id, &title, &body)?;
            match retired {
                Some(old) => {
                    println!("{}", style::success(&format!("revised note {old} → {new}")))
                }
                None => println!(
                    "{}",
                    style::success(&format!("updated note {new} in place"))
                ),
            }
        }
        MemoryCmd::Sync {
            file,
            budget,
            dry_run,
            stat,
            remove,
            force,
        } => with_loader("syncing memory", || {
            crate::stat_preview::set(stat);
            crate::plugins::memory::sync::run(&cfg, file, budget, dry_run, remove, force)
        })?,
        MemoryCmd::Status {
            project,
            since,
            json,
        } => crate::memory_status::run(&cfg, project.as_deref(), since.as_deref(), json)?,
    }

    Ok(())
}
