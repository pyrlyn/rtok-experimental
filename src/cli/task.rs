// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use super::util::print_json;
use crate::config::Config;
use anyhow::Result;
use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub(super) enum TaskCmd {
    /// Create a task under the next free id and print the id
    Create {
        /// One-line title
        title: String,
        /// Description: why the task exists and what done means
        #[arg(short = 'd', long, conflicts_with = "body_file")]
        description: Option<String>,
        /// Read the description from this file (`-` for stdin)
        #[arg(long, value_name = "PATH")]
        body_file: Option<PathBuf>,
        /// Make it a subtask of this task, e.g. `R2` → `R2.1`
        #[arg(long, value_name = "ID")]
        parent: Option<String>,
        /// Print the task as JSON instead of the id
        #[arg(long)]
        json: bool,
    },
    /// The plan: open and in-progress tasks, subtasks under their parent
    List {
        /// Only these statuses (comma-separated: open, in-progress, done, closed)
        #[arg(long, value_delimiter = ',')]
        status: Vec<String>,
        /// Done and closed tasks too
        #[arg(long)]
        all: bool,
        /// Only the subtasks of this task
        #[arg(long, value_name = "ID")]
        parent: Option<String>,
        /// JSON instead of the table
        #[arg(long)]
        json: bool,
    },
    /// One task: title, status, parent, subtasks, link and description
    Show {
        /// Task id, e.g. `R12` or `R2.1`
        id: String,
        /// JSON instead of the text
        #[arg(long)]
        json: bool,
    },
    /// Print a task's status, or set it; done and closed move it out of the plan
    Status {
        /// Task id, e.g. `R12` or `R2.1`
        id: String,
        /// open, in-progress, done or closed
        status: Option<String>,
        /// Finish a parent even though subtasks are still open
        #[arg(long)]
        force: bool,
        /// Print the task as JSON
        #[arg(long)]
        json: bool,
    },
    /// The first ready task: free or stale, not blocked; a leaf when nothing is blocked
    Next {
        /// JSON instead of the text
        #[arg(long)]
        json: bool,
    },
    /// Tasks that can be claimed, highest priority first
    Ready {
        /// JSON instead of the text
        #[arg(long)]
        json: bool,
    },
    /// Claim a task for this agent, or the first ready task when no id is given.
    /// On GitHub and GitLab the assignee write is last-write-wins, not compare-and-set.
    Claim {
        /// Task id; omit to take the first ready task
        id: Option<String>,
        /// The rtok agent id; defaults to `RTOK_AGENT_ID`
        #[arg(long)]
        agent: Option<String>,
        /// Print the task as JSON, with `changed`
        #[arg(long)]
        json: bool,
    },
    /// Clear the assignee and set the task open. Only the holder, unless --force.
    Release {
        /// Task id, e.g. `R12`
        id: String,
        /// The rtok agent id; defaults to `RTOK_AGENT_ID`
        #[arg(long)]
        agent: Option<String>,
        /// Release a task held by someone else
        #[arg(long)]
        force: bool,
        /// Print the task as JSON
        #[arg(long)]
        json: bool,
    },
    /// Record that this task waits on another. A cycle is refused and nothing is written.
    Dep {
        /// The task that waits
        id: String,
        /// The task that must finish first
        blocker: String,
        /// Print the task as JSON
        #[arg(long)]
        json: bool,
    },
    /// Set priority from 0 (highest) to 4. 2 is the default and is not stored.
    Priority {
        /// Task id, e.g. `R12`
        id: String,
        /// 0 to 4
        level: u8,
        /// Print the task as JSON
        #[arg(long)]
        json: bool,
    },
    /// Record that two tasks touch the same files, so they should not run at once. Symmetric;
    /// `ready` flags a task whose partner is in progress.
    Conflict {
        /// One task
        id: String,
        /// The task it conflicts with
        other: String,
        /// Take the conflict off instead
        #[arg(long)]
        remove: bool,
        /// Print the task as JSON
        #[arg(long)]
        json: bool,
    },
    /// Mark a task safe to run beside any other task.
    Parallel {
        /// Task id, e.g. `R12`
        id: String,
        /// Take the mark off instead
        #[arg(long)]
        off: bool,
        /// Print the task as JSON
        #[arg(long)]
        json: bool,
    },
    /// Raise the id counters to the adapter's highest ids and report drift; never writes the adapter
    Sync {
        /// JSON instead of the text
        #[arg(long)]
        json: bool,
    },
    /// Mirror the disk tasks one way into an issue tracker; the files stay the source of truth.
    /// A second run with nothing changed sends no write.
    Export {
        /// Where to mirror the tasks
        #[arg(value_parser = crate::tasks::export::BACKENDS)]
        backend: String,
        /// Print the planned creates, updates and closes and send nothing
        #[arg(long)]
        dry_run: bool,
        /// JSON instead of the text
        #[arg(long)]
        json: bool,
    },
    /// Write `[tasks]` into this checkout's `.rtok.toml` and seed the counter from existing tasks
    Init {
        /// disk, github or gitlab (default: disk, or what the file already says)
        #[arg(long)]
        adapter: Option<String>,
        /// Task id prefix, 1–8 ASCII letters (default: the project name's first letter)
        #[arg(long)]
        prefix: Option<String>,
    },
}

/// `rtok task …` (T441.5): every subcommand but `init` opens the project's adapter.
pub(super) fn run(action: TaskCmd, config_file: Option<&std::path::Path>) -> Result<()> {
    use crate::tasks::run::{Project, details, filter, init, table};
    use crate::tasks::{NewTask, Status, TaskId};
    use anyhow::Context as _;

    let cwd = std::env::current_dir()?;
    let cfg = Config::load_with(config_file, None)?;
    let open = || Project::open(&cfg.tasks, &cwd);
    let id_of = |s: &str| s.parse::<TaskId>();
    match action {
        TaskCmd::Init { adapter, prefix } => {
            let path = init(&cwd, adapter.as_deref(), prefix.as_deref())?;
            // Re-read: the file just changed what `[tasks]` says.
            let cfg = Config::load_with(config_file, None)?;
            let project = Project::open(&cfg.tasks, &cwd)?;
            let last = crate::tasks::ops::seed(&cfg, &project)?;
            println!(
                "{}: adapter {}, next id {}{}",
                path.display(),
                cfg.tasks.adapter,
                project.prefix,
                last + 1
            );
        }
        TaskCmd::Create {
            title,
            description,
            body_file,
            parent,
            json,
        } => {
            let description = match body_file {
                Some(p) if p.as_os_str() == "-" => std::io::read_to_string(std::io::stdin())?,
                Some(p) => std::fs::read_to_string(&p).with_context(|| p.display().to_string())?,
                None => description.unwrap_or_default(),
            };
            let new = NewTask {
                title,
                description,
                parent: parent.as_deref().map(id_of).transpose()?,
            };
            let task = crate::tasks::ops::create(&cfg, &open()?, &new)?;
            if json {
                print_json(&task)?;
            } else {
                println!("{}", task.id);
            }
        }
        TaskCmd::List {
            status,
            all,
            parent,
            json,
        } => {
            let tasks = open()?
                .adapter()
                .list(&filter(&status, all, parent.as_deref())?)?;
            if json {
                print_json(&tasks)?;
            } else {
                print!("{}", table(&tasks));
            }
        }
        TaskCmd::Show { id, json } => {
            let id = id_of(&id)?;
            let shown = open()?.show(&id)?;
            if json {
                print_json(&shown)?;
            } else {
                print!("{}", details(&shown));
            }
        }
        TaskCmd::Status {
            id,
            status,
            force,
            json,
        } => {
            let id = id_of(&id)?;
            let status = status.as_deref().map(str::parse::<Status>).transpose()?;
            let task = crate::tasks::ops::set_status(&cfg, &open()?, &id, status, force)?;
            if json {
                print_json(&task)?;
            } else {
                println!("{}: {}", task.id, task.status);
            }
        }
        TaskCmd::Sync { json } => {
            let report = crate::tasks::ops::sync(&cfg, &open()?)?;
            if json {
                print_json(&report)?;
            } else {
                print!("{}", crate::tasks::sync::text(&report));
            }
        }
        TaskCmd::Export {
            backend,
            dry_run,
            json,
        } => {
            let report = crate::tasks::ops::export(&cfg, &cwd, &backend, dry_run)?;
            if json {
                print_json(&report)?;
            } else {
                print!("{}", crate::tasks::export::render(&report));
            }
        }
        TaskCmd::Next { json } => {
            let next = crate::tasks::ops::next(&cfg, &open()?)?;
            match (next, json) {
                (next, true) => print_json(&next)?,
                (Some(t), false) => println!("{}  {}", t.id, t.title),
                (None, false) => println!("no ready task"),
            }
        }
        TaskCmd::Ready { json } => {
            let ready = crate::tasks::ops::ready(&cfg, &open()?)?;
            if json {
                print_json(&ready)?;
            } else {
                print!("{}", crate::tasks::run::ready_text(&ready));
            }
        }
        TaskCmd::Claim { id, agent, json } => {
            let outcome =
                crate::tasks::ops::claim(&cfg, &open()?, id.as_deref(), agent.as_deref())?;
            if json {
                print_json(&outcome)?;
            } else if outcome.changed {
                println!("{}  {}", outcome.task.id, outcome.task.title);
            } else {
                println!(
                    "{}  {}  (already yours)",
                    outcome.task.id, outcome.task.title
                );
            }
        }
        TaskCmd::Release {
            id,
            agent,
            force,
            json,
        } => {
            let task = crate::tasks::ops::release(&cfg, &open()?, &id, agent.as_deref(), force)?;
            if json {
                print_json(&task)?;
            } else {
                println!("{}: {}", task.id, task.status);
            }
        }
        TaskCmd::Dep { id, blocker, json } => {
            let task = open()?.block(&id_of(&id)?, &id_of(&blocker)?)?;
            if json {
                print_json(&task)?;
            } else {
                println!("{id}: blocked by {blocker}");
            }
        }
        TaskCmd::Conflict {
            id,
            other,
            remove,
            json,
        } => {
            let task = open()?.conflict(&id_of(&id)?, &id_of(&other)?, remove)?;
            if json {
                print_json(&task)?;
            } else if remove {
                println!("{id}: no longer conflicts with {other}");
            } else {
                println!("{id}: conflicts with {other}");
            }
        }
        TaskCmd::Parallel { id, off, json } => {
            let task = open()?.set_parallel(&id_of(&id)?, !off)?;
            if json {
                print_json(&task)?;
            } else if off {
                println!("{}: not parallel", task.id);
            } else {
                println!("{}: parallel", task.id);
            }
        }
        TaskCmd::Priority { id, level, json } => {
            let task = open()?.set_priority(&id_of(&id)?, level)?;
            if json {
                print_json(&task)?;
            } else {
                println!("{}: priority {level}", task.id);
            }
        }
    }
    Ok(())
}
