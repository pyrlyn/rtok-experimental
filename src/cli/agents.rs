// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use super::util::print_json;
use crate::config::Config;
use crate::model;
use crate::render::with_loader;
use anyhow::{Result, bail};
use clap::Subcommand;
use std::io::{self, IsTerminal};
use std::path::PathBuf;

#[derive(Subcommand)]
pub(super) enum AgentCmd {
    /// Install hooks, MCP server and proxy into a host
    #[command(alias = "setup")]
    Install(super::agents_host::SetupArgs),
    /// Take rtok back out of a host: hooks, MCP entry, proxy variable, plugin link
    #[command(visible_alias = "remove")]
    Uninstall(super::agents_host::RemoveArgs),
    /// Bring what rtok installed up to date: in place where it can, reinstalled where not
    Update(super::agents_host::UpdateArgs),
    /// List hosts whose rtok plugin is older than this binary
    Outdated(super::agents_host::OutdatedArgs),
    /// Every known app: kind and name, path and version, config files, rtok modules
    List {
        /// JSON instead of the table
        #[arg(long)]
        json: bool,
    },
    /// One host: the same block `agents list` prints, just for that app
    ///
    /// Whether that host's rtok MCP server answers: `rtok mcp ping <agent>`.
    Info {
        /// Host(s), comma-separated (`claude`, `cursor`, `codex`, `opencode`, `pi`, `zcode`, `kimi`, `copilot`, `aider`, `windsurf`, `zed`, `vscode`)
        host: String,
        /// JSON instead of the table
        #[arg(long)]
        json: bool,
    },
    /// What is running in this project: host, provider, model, tokens, start, run time
    Sessions {
        /// Also show sessions that have ended
        #[arg(long, global = true)]
        all: bool,
        /// JSON instead of the table
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        action: Option<SessionsCmd>,
    },
    /// Tokens and estimated cost per agent and month (or day), from the agents' logs or rtok
    ///
    /// Prices come from `[stats.prices]`; a model without one counts in the tokens and is
    /// left out of the cost (`--unpriced` names those).
    Usage {
        /// Data source: `logs` (the agents' own session files, the default), `rtok` (what passed through rtok) or `both`
        #[arg(long, value_name = "SOURCE")]
        source: Option<String>,
        /// Only these hosts, comma-separated (`claude,codex`)
        #[arg(long, value_name = "IDS")]
        host: Option<String>,
        /// From a date (`2026-09-01`, whole days in `--tz`) or a duration back from now (`30d`)
        #[arg(long, value_name = "DATE|DUR")]
        since: Option<String>,
        /// Through this date, inclusive
        #[arg(long, value_name = "DATE")]
        until: Option<String>,
        /// Bottom table by day
        #[arg(long, conflicts_with = "monthly")]
        daily: bool,
        /// Bottom table by month (the default)
        #[arg(long)]
        monthly: bool,
        /// Middle table grouping: `agent` (the default) or `model`
        #[arg(long, value_name = "AGENT|MODEL")]
        by: Option<String>,
        /// IANA time zone for day and month boundaries (default: the system zone)
        #[arg(long, value_name = "ZONE")]
        tz: Option<String>,
        /// List the models without a price instead of the tables
        #[arg(long)]
        unpriced: bool,
        /// One JSON document
        #[arg(long)]
        json: bool,
    },
    /// Junk and agent folders: `list` shows rtok's and every installed host's folders with sizes, `clear` removes rtok's own junk or, filtered, any agent's
    Junk {
        #[command(subcommand)]
        action: JunkCmd,
    },
    // T283, D34
    /// This session's own rtok agent id: `RTOK_AGENT_ID`, resolved through the store
    Whoami {
        /// JSON instead of the text lines
        #[arg(long)]
        json: bool,
    },
    /// One agent by id prefix: host, model, ids, parent and sub-agents, cwd, activity, status
    Show {
        /// Agent id or any unique prefix of it
        id: String,
        /// JSON instead of the text lines
        #[arg(long)]
        json: bool,
    },
    /// Say what this agent (`RTOK_AGENT_ID`) is busy with: plain text, at most 120 chars
    Status {
        /// The status text; empty clears it
        text: String,
    },
    // T287
    /// Send a message to an agent: `send <id-prefix> <text|->` or `send --all-live <text|->`
    Send {
        /// Recipient's rtok agent id (any unique prefix); with `--all-live`, the text instead
        to: Option<String>,
        /// The message; `-` reads it from stdin (≤ 4 KiB, control characters stripped)
        text: Option<String>,
        /// Every live agent of this project (same repository or cwd), except the sender
        #[arg(long)]
        all_live: bool,
        /// The task this message is about (e.g. `A12`); the recipient can read just that thread
        #[arg(long, value_name = "ID")]
        task: Option<String>,
    },
    // T287
    /// Read messages: your own inbox from `RTOK_AGENT_ID` (marks them read), or with an
    /// id that agent's queue (marks nothing)
    Inbox {
        /// An agent's rtok id (any unique prefix); omitted: `RTOK_AGENT_ID`
        id: Option<String>,
        /// Only messages not read yet
        #[arg(long)]
        unread: bool,
        /// Only messages about this task (e.g. `A12`)
        #[arg(long, value_name = "ID")]
        task: Option<String>,
        /// JSON instead of the framed text
        #[arg(long)]
        json: bool,
    },
}

/// How `junk list` and the `clear` dry run print their items (T330.6).
#[derive(clap::Args)]
pub(super) struct Breakdown {
    /// Items listed per kind: a count, `all`, or `0` for totals only
    #[arg(long = "items", value_name = "N|all", default_value = "10", value_parser = crate::agents::junk_items::shown_arg)]
    shown: usize,
    /// Order of the items in a kind: `size` (largest first), `last-used` (longest unused first) or `path`
    #[arg(long, value_name = "ORDER", default_value = "size", value_parser = clap::builder::PossibleValuesParser::new(crate::agents::junk_items::SORTS))]
    sort: String,
    /// Only items of at least this size (`10MB`); on `clear` this also limits what `--yes` removes
    #[arg(long, value_name = "SIZE", value_parser = crate::bytes::parse_bytes)]
    min_size: Option<u64>,
}

impl Breakdown {
    fn view(&self, cfg: &Config, exact: bool, links: bool) -> crate::agents::junk_items::View {
        crate::agents::junk_items::View {
            shown: self.shown,
            sort: crate::agents::junk_items::Sort::parse(&self.sort),
            min_size: self.min_size.unwrap_or(0),
            exact,
            links,
            ..crate::agents::junk_items::View::new(cfg)
        }
    }
}

#[derive(Subcommand)]
pub(super) enum JunkCmd {
    /// Folders, junk kinds and sizes per agent, and the space `agents junk clear` would free
    List {
        /// JSON instead of the text
        #[arg(long)]
        json: bool,
        /// Exact byte counts instead of KB/MB/GB
        #[arg(long)]
        bytes: bool,
        /// Also the hosts that are not installed
        #[arg(long)]
        all: bool,
        /// Sessions untouched for more than this many days are old, for this run only
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(0..=3650))]
        session_days: Option<u32>,
        #[command(flatten)]
        view: Breakdown,
    },
    /// List what `agents junk clear` would remove; `--yes` applies it
    ///
    /// Without `--agent`, `--kind`, `--include` or `--older-than` (or with `--agent rtok`
    /// alone) only rtok's own logs and archives: other agents' junk needs a filter.
    Clear {
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
        /// Only this agent: `rtok` or a host id (repeatable)
        #[arg(long = "agent", value_name = "AGENT", value_parser = crate::agents::junk_clear::agent_arg)]
        agents: Vec<String>,
        /// Only this junk kind (repeatable)
        #[arg(long = "kind", value_name = "KIND", value_parser = clap::builder::PossibleValuesParser::new(crate::agents::junk_clear::KINDS))]
        kinds: Vec<String>,
        /// Also the review kinds
        #[arg(long, value_name = "CLASS", value_parser = ["review"])]
        include: Option<String>,
        /// Only items not modified for this long (`7d`, `12h`)
        #[arg(long, value_name = "AGE", value_parser = humantime::parse_duration)]
        older_than: Option<std::time::Duration>,
        /// Sessions untouched for more than this many days are old, for this run only
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(0..=3650))]
        session_days: Option<u32>,
        /// Move to the OS trash instead of deleting
        #[arg(long)]
        trash: bool,
        #[command(flatten)]
        view: Breakdown,
    },
}

/// `rtok agents sessions watch` (T25.3): the same table, live. One screen, no keys:
///
/// the TTY repaints in place through T24.3's `watch_loop`, a pipe gets the whole
/// table again whenever it changes.
#[derive(Subcommand)]
pub(super) enum SessionsCmd {
    /// Redraw the sessions table in place as sessions appear, end or spend
    Watch,
}

/// The `[agents.usage]` overlay for the flags the caller gave. `hosts` is the one list key,
/// so its comma-separated flag is split here.
fn usage_flags<const N: usize>(given: [(&str, Option<String>); N]) -> Option<figment::value::Dict> {
    use figment::value::{Dict, Value};
    let mut usage = Dict::new();
    for (key, value) in given {
        let Some(value) = value else { continue };
        let value = if key == "hosts" {
            Value::from(
                value
                    .split(',')
                    .map(|h| h.trim().to_string())
                    .collect::<Vec<_>>(),
            )
        } else {
            Value::from(value)
        };
        usage.insert(key.into(), value);
    }
    if usage.is_empty() {
        return None;
    }
    let mut agents = Dict::new();
    agents.insert("usage".into(), Value::from(usage));
    let mut flags = Dict::new();
    flags.insert("agents".into(), Value::from(agents));
    Some(flags)
}

/// `--session-days N` as the flag layer of `agents.junk.stale_session_days`.
fn session_days_flag(days: Option<u32>) -> Option<figment::value::Dict> {
    use figment::value::{Dict, Value};
    let junk = Dict::from([("stale_session_days".to_string(), Value::from(days?))]);
    let agents = Dict::from([("junk".to_string(), Value::from(junk))]);
    Some(Dict::from([("agents".to_string(), Value::from(agents))]))
}

pub(super) fn run(config_file: &Option<PathBuf>, action: AgentCmd) -> Result<()> {
    match action {
        AgentCmd::Install(args) => super::agents_host::setup_host(config_file.as_deref(), args)?,
        AgentCmd::Uninstall(args) => super::agents_host::setup_host(
            config_file.as_deref(),
            super::agents_host::SetupArgs::removing(args),
        )?,
        AgentCmd::Update(args) => {
            if args.check {
                super::agents_host::outdated_hosts(
                    config_file.as_deref(),
                    super::agents_host::outdated_from_update(&args),
                )?;
            } else {
                super::agents_host::update_hosts(config_file.as_deref(), args)?;
            }
        }
        AgentCmd::Outdated(args) => {
            super::agents_host::outdated_hosts(config_file.as_deref(), args)?
        }
        AgentCmd::List { json } => {
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            if json {
                let rows = with_loader("listing hosts", || model::agents_list(&cfg));
                print_json(&rows)?;
            } else {
                let text = with_loader("listing hosts", || crate::agents::list(&cfg));
                print!("{text}");
            }
        }
        AgentCmd::Info { host, json } => {
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            let hosts = crate::agents::command::parse_hosts(&host)?;
            if json {
                let agents = crate::agents::resolve(&hosts)?;
                let ids: Vec<&str> = agents.iter().map(|a| a.id()).collect();
                let rows = with_loader("reading host", || model::agents_listed(&cfg, &ids));
                print_json(&rows)?;
            } else {
                let text = with_loader("reading host", || crate::agents::info(&cfg, &hosts))?;
                print!("{text}");
            }
        }
        // The command renders the model's Sessions page (T25.2): newest first, live
        // only unless `--all`. `since = 0` because the default view's window is
        // liveness itself — a `started_at` floor could hide a session that began
        // before it and is still running, which is the row this command exists for.
        AgentCmd::Sessions { all, json, action } => {
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            // T25.3: live repaint through T24.3's `watch_loop` — no second loop.
            // The loop only writes characters (no raw mode, no alternate screen),
            // so Ctrl-C under the default handling leaves the terminal as found.
            if matches!(action, Some(SessionsCmd::Watch)) {
                let mut out = io::stdout();
                let tty = out.is_terminal();
                let mut prev = String::new();
                let run =
                    crate::log::watch_loop(&mut out, tty, crate::log::WATCH_POLL, move || {
                        let now = crate::log::now() as i64;
                        match model::agent_sessions(&cfg, all, now) {
                            Ok(rows) => {
                                Some(crate::render::sessions_tick(&mut prev, &rows, all, now))
                            }
                            Err(_) => {
                                // A transient unreadable store is a missed poll,
                                // not a blank screen: keep showing what we had.
                                let screen: Vec<String> =
                                    prev.lines().map(str::to_string).collect();
                                Some(crate::log::WatchTick {
                                    fresh: Vec::new(),
                                    screen,
                                })
                            }
                        }
                    });
                match run {
                    Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => return Ok(()),
                    other => other?,
                }
                return Ok(());
            }
            let now = crate::log::now() as i64;
            let rows = model::agent_sessions(&cfg, all, now)?;
            if json {
                print_json(&rows)?;
            } else {
                print!("{}", crate::render::sessions_table(&rows, all, now));
            }
        }
        AgentCmd::Usage {
            source,
            host,
            since,
            until,
            daily,
            monthly,
            by,
            tz,
            unpriced,
            json,
        } => {
            let period = daily.then_some("daily").or(monthly.then_some("monthly"));
            let flags = usage_flags([
                ("source", source),
                ("hosts", host),
                ("since", since),
                ("until", until),
                ("period", period.map(str::to_string)),
                ("by", by),
                ("tz", tz),
            ]);
            let cfg = Config::load_with(config_file.as_deref(), flags)?;
            let report = crate::agents::usage::open_report(&cfg)?;
            for s in &report.skipped {
                eprintln!("skipped {}: {} in {}", s.host, s.reason, s.path.display());
            }
            if json {
                print_json(&report)?;
            } else if unpriced {
                print!("{}", report.unpriced_text());
            } else {
                print!("{}", report.to_text());
            }
        }
        AgentCmd::Junk {
            action:
                JunkCmd::List {
                    json,
                    bytes,
                    all,
                    session_days,
                    view,
                },
        } => {
            let cfg = Config::load_with(config_file.as_deref(), session_days_flag(session_days))?;
            let report = crate::agents::junk::report_in_repo(&cfg, all);
            let links = !json && io::stdout().is_terminal();
            let view = view.view(&cfg, bytes, links);
            if json {
                print_json(&crate::agents::junk_items::json(&report, &view))?;
            } else {
                print!("{}", crate::agents::junk::to_list(&report, &view));
            }
        }
        AgentCmd::Junk {
            action:
                JunkCmd::Clear {
                    yes,
                    dry_run: _,
                    stat: _,
                    json,
                    agents,
                    kinds,
                    include,
                    older_than,
                    session_days,
                    trash,
                    view,
                },
        } => {
            let cfg = Config::load_with(config_file.as_deref(), session_days_flag(session_days))?;
            let filter = crate::agents::junk_clear::Filter {
                agents,
                kinds,
                include_review: include.is_some(),
                older_than,
                trash,
                min_size: view.min_size.unwrap_or(0),
            };
            if !filter.is_t182() || trash {
                let cleared = crate::agents::junk_clear::run(&cfg, &filter, yes)?;
                if json {
                    print_json(&cleared)?;
                } else {
                    let view = view.view(&cfg, false, io::stdout().is_terminal());
                    print!("{}", crate::agents::junk_clear::to_text(&cleared, &view));
                }
                if cleared.failed() {
                    bail!("some junk could not be removed");
                }
            } else {
                let outcomes = crate::agents::junk::run(&cfg, yes);
                let failed = outcomes.iter().any(|o| o.failed);
                if json {
                    print_json(&outcomes)?;
                } else {
                    print!("{}", crate::agents::junk::to_table(&outcomes, yes));
                }
                if failed {
                    bail!("some junk could not be removed");
                }
            }
        }
        AgentCmd::Whoami { json } => {
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            let detail = crate::agents::command::whoami(&cfg)?;
            if json {
                print_json(&detail)?;
            } else {
                print!("{}", crate::render::agent_whoami_text(&detail));
            }
        }
        AgentCmd::Show { id, json } => {
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            let now = crate::log::now() as i64;
            let agent = model::agent_show(&cfg, &id, now)?;
            if json {
                print_json(&agent)?;
            } else {
                print!("{}", crate::render::agent_show_text(&agent, now));
            }
        }
        AgentCmd::Status { text } => {
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            let me = crate::agents::command::status_caller(&cfg);
            match model::set_status(&cfg, me.as_deref(), &text)? {
                Some(text) => println!("status: {text}"),
                None => println!("status cleared"),
            }
        }
        AgentCmd::Send {
            to,
            text,
            all_live,
            task,
        } => {
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            crate::agents::command::send(&cfg, to, text, all_live, task.as_deref())?;
        }
        AgentCmd::Inbox {
            id,
            unread,
            task,
            json,
        } => {
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            crate::agents::command::inbox(&cfg, id, unread, task.as_deref(), json)?;
        }
    };
    Ok(())
}
