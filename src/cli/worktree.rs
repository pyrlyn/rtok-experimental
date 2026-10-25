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
        /// Task id, e.g. `t158`, or `t158 read-hook-same-path` as one argument; directory
        /// `<repo>-<task>`, branch `<task>[-<slug>]`
        task: String,
        /// Optional branch suffix (spaces become dashes, as in the one-argument form)
        slug: Option<String>,
        /// Who holds it, as `<provider> / <model>`; written into the lock reason. Defaults to
        /// `<host> / <model>` of the agent, else `RTOK_OWNER`, else the login user
        #[arg(long)]
        owner: Option<String>,
        /// The rtok agent id (any unique prefix) to bind it to; defaults to `RTOK_AGENT_ID`
        #[arg(long)]
        agent: Option<String>,
        /// Do not seed `[worktree] seed_dirs` from a sibling checkout this time
        #[arg(long)]
        no_seed: bool,
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
    /// Hand your task worktree to review: commit what is left, push, open or reuse the PR, ask
    /// for squash auto-merge, then unlock. Refuses the main checkout, another agent's lock and
    /// credential files; never closes the task
    Finish {
        /// The worktree, or a directory inside it; defaults to the current directory
        path: Option<PathBuf>,
        /// Commit message for uncommitted work; defaults to `<task>: <branch slug>`
        #[arg(short, long)]
        message: Option<String>,
        /// `owner/name` for gh, when the `origin` URL does not name the repository
        #[arg(long)]
        repo: Option<String>,
        /// Open the PR without auto-merge
        #[arg(long)]
        no_auto_merge: bool,
        /// The rtok agent id (any unique prefix); defaults to `RTOK_AGENT_ID`
        #[arg(long)]
        agent: Option<String>,
        /// The owner an old lock names, as `<provider> / <model>`. Defaults to `<host> / <model>`
        /// of the agent
        #[arg(long)]
        owner: Option<String>,
        /// JSON instead of the text lines
        #[arg(long)]
        json: bool,
    },
    /// Every worktree and orphan with its owner, state and source bytes
    List {
        /// JSON instead of the table
        #[arg(long)]
        json: bool,
        /// Also walk every build cache for its bytes (slow with large target/ folders)
        #[arg(long)]
        sizes: bool,
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
        /// Keep worktrees modified within this window (`30m`, `24h`, `7d`)
        #[arg(long, default_value = "24h")]
        idle: String,
        /// Treat someone else's lock as abandoned once its merged, clean worktree is untouched this long (`30m`, `7d`)
        #[arg(long, default_value = crate::worktree::gc::STALE_LOCK_DEFAULT)]
        stale_lock: String,
        /// JSON instead of the table
        #[arg(long)]
        json: bool,
    },
    /// Delete idle tagged build caches (`CACHEDIR.TAG`) and keep the worktrees; dry run without `--yes`
    Clean {
        /// Only these worktrees; the one this command runs from is cleaned only when named
        paths: Vec<PathBuf>,
        /// Keep caches modified within this window (`30m`, `24h`, `7d`)
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
    /// Move this repository's worktrees into `[worktree] root`; dry run without `--yes`
    Migrate {
        /// Apply; without it this is a dry run that changes nothing
        #[arg(long)]
        yes: bool,
        /// Also move old locks without an agent whose owner is this
        #[arg(long)]
        owner: Option<String>,
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
            no_seed,
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
                no_seed,
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
        WorktreeCmd::Finish {
            path,
            message,
            repo,
            no_auto_merge,
            agent,
            owner,
            json,
        } => {
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            let opts = crate::worktree::finish::Options {
                message,
                repo,
                no_auto_merge,
            };
            let path = match path {
                Some(path) => path,
                None => std::env::current_dir()?,
            };
            let done = crate::worktree::ops::finish(&cfg, &path, agent.as_deref(), owner, &opts)?;
            if json {
                print_json(&done)?;
            } else {
                print!("{}", done.to_text());
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
        WorktreeCmd::List { json, sizes } => {
            let cwd = std::env::current_dir()?;
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            let rows = crate::worktree::ops::list(&cfg, &cwd, sizes)?;
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
            // T285: a live agent's worktree is never removed unless its task is finished (T453); no store, no live agents.
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            let live = crate::worktree::ops::live_ids(&cfg);
            let policy = gc::Policy {
                owner: owner.as_deref(),
                idle: crate::store::parse_idle_from(&idle, "--idle")?,
                stale_lock: crate::store::parse_idle_from(&stale_lock, "--stale-lock")?,
                now: std::time::SystemTime::now(),
                live,
                ended: crate::worktree::ops::ended_ids(&cfg),
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
        WorktreeCmd::Migrate { yes, owner, json } => {
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            let cwd = std::env::current_dir()?;
            let outcomes = crate::worktree::ops::migrate(&cfg, &cwd, owner.as_deref(), yes)?;
            if json {
                print_json(&outcomes)?;
            } else {
                print!("{}", crate::worktree::migrate::to_table(&outcomes, yes));
            }
            if outcomes.iter().any(|o| o.failed) {
                bail!("some worktrees could not be moved");
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
            let policy = clean::Policy {
                idle: crate::store::parse_idle_from(&idle, "--idle")?,
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
