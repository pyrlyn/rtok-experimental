// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use super::util::print_json;
use crate::config::Config;
use anyhow::{Result, bail};
use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub(super) enum WorktreeCmd {
    /// Create the worktree for a task: one location, one name, one locked owner; prints its path
    Add {
        /// Task id, e.g. `t158`; directory `<repo>-<task>`, branch `<task>[-<slug>]`
        task: String,
        /// Optional branch suffix
        slug: Option<String>,
        /// Who holds it, as `<provider> / <model>`; written into the lock reason. Defaults to
        /// `<host> / <model>` of the agent
        #[arg(long)]
        owner: Option<String>,
        /// The rtok agent id (any unique prefix) to bind it to; defaults to `RTOK_AGENT_ID`
        #[arg(long)]
        agent: Option<String>,
    },
    /// Bind an existing worktree to an agent: rewrites its lock only when unlocked or already yours
    Claim {
        /// The worktree
        path: PathBuf,
        /// The rtok agent id (any unique prefix); defaults to `RTOK_AGENT_ID`
        #[arg(long)]
        agent: Option<String>,
        /// The owner an old lock names, as `<provider> / <model>`. Defaults to `<host> / <model>`
        /// of the agent
        #[arg(long)]
        owner: Option<String>,
    },
    /// Bind the worktree you are in (made by a host's own tool) to your agent. A worktree in a
    /// pool its host evicts (Cursor, Codex, Windsurf, Devin) is claimed in the store only. With
    /// no agent (a post-create script) the one live agent of that host in the repository takes
    /// it; with none or several the claim waits for the next agent that adopts or starts there
    Adopt {
        /// The worktree, or a directory inside it; defaults to the current directory
        path: Option<PathBuf>,
        /// The task id, when the lock and the branch do not name one (a detached HEAD)
        #[arg(long)]
        task: Option<String>,
        /// The rtok agent id (any unique prefix); defaults to `RTOK_AGENT_ID`
        #[arg(long)]
        agent: Option<String>,
        /// The owner the lock names, as `<provider> / <model>`. Defaults to `<host> / <model>`
        /// of the agent
        #[arg(long)]
        owner: Option<String>,
        /// Print the result as JSON
        #[arg(long)]
        json: bool,
    },
    /// Remove your own finished worktree: unlock, `git worktree remove`, delete the branch
    /// when merged, release the claim; refuses dirty or current worktrees, and a foreign lock
    /// unless the task is finished (merged, clean, with commits of its own)
    Remove {
        /// The worktree's path, or its task id (the lock's task or the branch `<task>[-<slug>]`)
        target: String,
        /// The rtok agent id (any unique prefix); defaults to `RTOK_AGENT_ID`
        #[arg(long)]
        agent: Option<String>,
        /// The owner an old lock names, as `<provider> / <model>`. Defaults to `<host> / <model>`
        /// of the agent
        #[arg(long)]
        owner: Option<String>,
        /// Remove an unmerged worktree too and keep its branch (a merged branch is kept as well)
        #[arg(long)]
        keep_branch: bool,
        /// Show the path, branch, size and file count that would go; remove nothing
        #[arg(long)]
        dry_run: bool,
        /// JSON instead of the text line
        #[arg(long)]
        json: bool,
    },
    /// Every worktree and orphan with its owner, state, source and build-cache bytes
    List {
        /// JSON instead of the table
        #[arg(long)]
        json: bool,
    },
    /// Your agent, where new worktrees go, the worktree you are in and the ones you hold
    Whoami {
        /// JSON instead of the text lines
        #[arg(long)]
        json: bool,
    },
    /// Remove merged, clean, idle worktrees with their branches; drop records of deleted ones
    Gc {
        /// Apply; without it this is a dry run that changes nothing
        #[arg(long)]
        yes: bool,
        /// Say the dry run out loud: print what would go and change nothing (the default)
        #[arg(long, conflicts_with = "yes")]
        dry_run: bool,
        /// The same preview as the dry run: a removal's line already is its stat
        #[arg(long)]
        stat: bool,
        /// Open locks whose reason starts with this owner; any other lock holds until `--stale-lock`
        #[arg(long)]
        owner: Option<String>,
        /// Keep worktrees modified within this window (`24h`, `7d`)
        #[arg(long, default_value = "24h")]
        idle: String,
        /// Treat someone else's lock as abandoned once its merged, clean worktree is untouched this long
        #[arg(long, default_value = "7d")]
        stale_lock: String,
        /// JSON instead of the table
        #[arg(long)]
        json: bool,
    },
    /// Delete idle tagged build caches (`CACHEDIR.TAG`) and keep the worktrees; dry run without `--yes`
    Clean {
        /// Only these worktrees; the one this command runs from is cleaned only when named
        paths: Vec<PathBuf>,
        /// Keep caches modified within this window (`24h`, `7d`)
        #[arg(long, default_value = "24h")]
        idle: String,
        /// Apply; without it this is a dry run that changes nothing
        #[arg(long)]
        yes: bool,
        /// Say the dry run out loud: print what would go and change nothing (the default)
        #[arg(long, conflicts_with = "yes")]
        dry_run: bool,
        /// The same preview as the dry run: a removal's line already is its stat
        #[arg(long)]
        stat: bool,
        /// JSON instead of the table
        #[arg(long)]
        json: bool,
    },
}

pub(super) fn run(config_file: &Option<PathBuf>, action: WorktreeCmd) -> Result<()> {
    // One gate for every subcommand, `list` included (T410).
    if !Config::load_with(config_file.as_deref(), None)?
        .worktree
        .enabled
    {
        bail!(crate::worktree::DISABLED);
    }
    match action {
        WorktreeCmd::Add {
            task,
            slug,
            owner,
            agent,
        } => {
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            let cwd = std::env::current_dir()?;
            let plan = crate::worktree::ops::add(
                &cfg,
                &cwd,
                &task,
                slug.as_deref(),
                owner,
                agent.as_deref(),
            )?;
            crate::agents::task_list::write_for(&cwd, &plan.path, &plan.task);
            println!("{}", plan.path.display());

            Ok(())
        }
        WorktreeCmd::Claim { path, agent, owner } => {
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            let done = crate::worktree::ops::claim(&cfg, &path, agent.as_deref(), owner)?;
            println!("{}", done.path.display());

            Ok(())
        }
        WorktreeCmd::Adopt {
            path,
            task,
            agent,
            owner,
            json,
        } => {
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            let done = crate::worktree::ops::adopt(
                &cfg,
                path.as_deref(),
                task.as_deref(),
                agent.as_deref(),
                owner,
            )?;
            if json {
                print_json(&done)?;
            } else {
                println!("{}", done.path.display());
            }
            if done.pending {
                crate::log::stderr_ln(
                    "pending: no single agent matched; the next agent to adopt or start a session in this worktree takes it",
                );
            }

            Ok(())
        }
        WorktreeCmd::Remove {
            target,
            agent,
            owner,
            keep_branch,
            dry_run,
            json,
        } => {
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            let done = crate::worktree::ops::remove(
                &cfg,
                &target,
                agent.as_deref(),
                owner,
                keep_branch,
                dry_run,
            )?;
            if json {
                print_json(&done)?;
            } else if dry_run {
                print!("{}", done.preview_text());
            } else {
                println!("{}: {}", done.path, done.note);
            }

            Ok(())
        }
        WorktreeCmd::Whoami { json } => {
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            let cwd = std::env::current_dir()?;
            let me = crate::worktree::ops::whoami(&cfg, &cwd);
            if json {
                print_json(&me)?;
            } else {
                print!("{}", crate::worktree::whoami::to_text(&me));
            }

            Ok(())
        }
        WorktreeCmd::List { json } => {
            let cwd = std::env::current_dir()?;
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            let rows = crate::worktree::ops::list(&cfg, &cwd)?;
            if json {
                print_json(&rows)?;
            } else {
                let now = std::time::SystemTime::now();
                print!("{}", crate::worktree::list::to_table(&rows, now));
            }

            Ok(())
        }
        WorktreeCmd::Gc {
            yes,
            dry_run: _,
            stat: _,
            owner,
            idle,
            stale_lock,
            json,
        } => {
            use crate::worktree::gc;
            use anyhow::Context as _;
            // T285: a live agent's worktree is never removed unless its task is finished (T453); no store, no live agents.
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            let live = crate::worktree::ops::live_ids(&cfg);
            let policy = gc::Policy {
                owner: owner.as_deref(),
                idle: crate::measure::stats::parse_since(&idle).context("--idle")?,
                stale_lock: crate::measure::stats::parse_since(&stale_lock)
                    .context("--stale-lock")?,
                now: std::time::SystemTime::now(),
                live,
                pending: crate::worktree::ops::pending_paths(&cfg),
            };
            let outcomes = gc::run(&std::env::current_dir()?, &policy, yes)?;
            if json {
                print_json(&outcomes)?;
            } else {
                print!("{}", gc::to_table(&outcomes, yes));
            }
            if outcomes.iter().any(|o| o.failed) {
                bail!("some worktrees could not be removed");
            }

            Ok(())
        }
        WorktreeCmd::Clean {
            paths,
            idle,
            yes,
            dry_run: _,
            stat: _,
            json,
        } => {
            use crate::worktree::clean;
            use anyhow::Context as _;
            let policy = clean::Policy {
                idle: crate::measure::stats::parse_since(&idle).context("--idle")?,
                now: std::time::SystemTime::now(),
            };
            let outcomes = clean::run(&std::env::current_dir()?, &paths, &policy, yes)?;
            if json {
                print_json(&outcomes)?;
            } else {
                print!("{}", clean::to_table(&outcomes, yes, policy.now));
            }
            if outcomes.iter().any(|o| o.failed) {
                bail!("some caches could not be deleted");
            }

            Ok(())
        }
    }
}
