// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Host install and agent messaging for `rtok agents`. The CLI parses flags and, where the
//! result is a value, prints it; the installers and the store live here.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::link::shell_agent;
use super::{Mode, OutdatedSelection, Request, installed_hosts, print_human, report};
use crate::config::Config;
use crate::render::with_loader;
use crate::store::{AgentDetail, Store};
use crate::ui::style;

/// Comma-separated host ids. Empty input is refused before any backup is taken.
pub fn parse_hosts(host: &str) -> Result<Vec<String>> {
    let hosts: Vec<String> = host
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if hosts.is_empty() {
        bail!("unknown host: {host}");
    }
    Ok(hosts)
}

/// What `rtok agents install` / `uninstall` / the deprecated `setup` ask for, after flags
/// have been turned into the config overlay.
pub struct Install {
    pub host: String,
    pub remove: bool,
    pub replace: bool,
    pub cli: bool,
    pub desktop: bool,
    pub all: bool,
    pub no_restart: bool,
    /// T289.3: write the project's post-create script entry instead of the host's own config.
    pub project: bool,
}

/// The host installers, one call site for `rtok agents install|uninstall` and the deprecated
/// `rtok setup`. Unknown hosts are refused before any backup is taken.
pub fn install(
    config_file: Option<&Path>,
    args: Install,
    flags: Option<figment::value::Dict>,
) -> Result<()> {
    let mut cfg = Config::load_with(config_file, flags)?;
    // Comma-separated hosts: `rtok agents install opencode,cursor` installs both.
    let hosts = parse_hosts(&args.host)?;
    if args.project {
        let root = crate::config::layers::git_root(&std::env::current_dir()?)
            .context("--project: run it inside the project's git repository")?;
        crate::log::stdout(&super::post_create::run(&cfg, &hosts, args.remove, &root)?);
        return Ok(());
    }
    let mode = if args.remove {
        Mode::Remove
    } else if args.replace {
        Mode::Replace
    } else {
        Mode::Install
    };
    let req = Request {
        hosts,
        mode,
        cli: args.cli,
        desktop: args.desktop,
        all: args.all,
    };
    apply_hosts(&mut cfg, &req, args.no_restart)
}

/// `rtok agents outdated`.
pub struct Outdated {
    pub host: Option<String>,
    pub cli: bool,
    pub desktop: bool,
    pub all: bool,
    pub json: bool,
    pub exit_code: bool,
}

pub fn outdated(config_file: Option<&Path>, args: Outdated) -> Result<()> {
    let cfg = Config::load_with(config_file, None)?;
    let hosts = match &args.host {
        Some(h) => parse_hosts(h)?,
        None => Vec::new(),
    };
    let selection = OutdatedSelection {
        hosts,
        cli: args.cli,
        desktop: args.desktop,
        all: args.all,
    };
    let rep = report(&cfg, &selection)?;
    if args.json {
        crate::log::stdout_ln(&serde_json::to_string(&rep)?);
    } else {
        crate::log::stdout(&print_human(&rep));
    }
    if args.exit_code && !rep.outdated.is_empty() {
        std::process::exit(super::EXIT_OUTDATED);
    }
    Ok(())
}

/// `rtok agents update`.
pub struct Update {
    pub host: Option<String>,
    pub cli: bool,
    pub desktop: bool,
    pub all: bool,
    pub no_restart: bool,
}

pub fn update(
    config_file: Option<&Path>,
    args: Update,
    flags: Option<figment::value::Dict>,
) -> Result<()> {
    let mut cfg = Config::load_with(config_file, flags)?;
    let hosts = match &args.host {
        Some(h) => parse_hosts(h)?,
        None => installed_hosts(&cfg),
    };
    if hosts.is_empty() {
        crate::log::stdout_ln(&style::info(
            "nothing to update: rtok is not installed in any host (rtok agents install <host>)",
        ));
        return Ok(());
    }
    let req = Request {
        hosts,
        mode: Mode::Update,
        cli: args.cli,
        desktop: args.desktop,
        all: args.all,
    };
    apply_hosts(&mut cfg, &req, args.no_restart)
}

/// Run `req` with the loader and desktop-restart handling every `agents` writer shares.
fn apply_hosts(cfg: &mut Config, req: &Request, no_restart: bool) -> Result<()> {
    // T81: `agents::run` may ask the plugin question mid-run, and a loader ticking on
    // stderr redraws right over a prompt — the question turns invisible and the wait for
    // its answer reads as a hang. A spinner must never share a terminal with a question,
    // so interactive runs render no loader; pipes and CI (which can never be asked) keep it.
    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin());
    // T141: closes a running desktop app before the write if it would change that host's
    // config, and reopens it after; CLI-only hosts just get a "restart your session" note.
    let out = if interactive {
        super::restart::run(cfg, req, no_restart)?
    } else {
        with_loader("updating host", || {
            super::restart::run(cfg, req, no_restart)
        })?
    };
    crate::log::stdout(&out);
    // T279 step 3/6 "Failure": a Claude reinstall that removed the old plugin and then failed
    // to install the new one must not exit 0 like every other `claude` degrade — the host is
    // left with nothing. Printed first, same as `worktree gc`/`clean`'s own "print the table,
    // then bail if something in it failed" shape, so the line above is not lost.
    if out.contains(super::claude::REINSTALL_FAILED) {
        bail!("a plugin reinstall failed");
    }
    Ok(())
}

fn env_var(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

/// T287: the caller's own rtok agent id — `RTOK_AGENT_ID` resolved, else (T473) the host
/// session's agent, `None` when neither names one (the user at a terminal). A set but
/// unresolvable `RTOK_AGENT_ID` is an error, never a silent "user".
fn caller_agent(store: &Store) -> Result<Option<String>> {
    match shell_agent(Some(store), env_var) {
        None => Ok(None),
        Some(raw) => store
            .resolve_agent(&raw)
            .map(Some)
            .map_err(|e| anyhow::anyhow!("RTOK_AGENT_ID {raw}: {e}")),
    }
}

/// `rtok agents whoami`.
pub fn whoami(cfg: &Config) -> Result<AgentDetail> {
    let store = Store::open(&cfg.core.db_path)?;
    let detail = shell_agent(Some(&store), env_var)
        .and_then(|raw| store.resolve_agent(&raw).ok())
        .and_then(|id| store.agent_detail(&id).ok().flatten());
    detail.ok_or_else(|| anyhow::anyhow!("not inside an agent session"))
}

/// `rtok agents status`: the caller's id, if a store and a session name one.
pub fn status_caller(cfg: &Config) -> Option<String> {
    let store = Store::open(&cfg.core.db_path).ok();
    shell_agent(store.as_ref(), env_var)
}

/// `rtok agents send` (T287): one recipient by id prefix, or every live agent of the caller's
/// project (`project_name` of its cwd, else the cwd itself) minus the sender.
pub fn send(
    cfg: &Config,
    to: Option<String>,
    text: Option<String>,
    all_live: bool,
    task: Option<&str>,
) -> Result<()> {
    let (to, text) = match (all_live, to, text) {
        (false, Some(to), Some(text)) => (Some(to), text),
        (true, Some(text), None) => (None, text),
        _ => bail!("usage: rtok agents send <id-prefix> <text|->, or --all-live <text|->"),
    };
    let text = if text == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        text
    };
    let store = Store::open(&cfg.core.db_path)?;
    let from = caller_agent(&store)?;
    let targets = if all_live {
        let here = match &from {
            Some(id) => store
                .agent_detail(id)?
                .and_then(|d| d.cwd)
                .map(PathBuf::from),
            None => std::env::current_dir().ok(),
        };
        let key =
            |p: &Path| crate::project::project_name(p).unwrap_or_else(|| p.display().to_string());
        let Some(here) = here.as_deref().map(key) else {
            bail!("--all-live: the caller has no cwd to match a project by");
        };
        let ids: Vec<String> = store
            .live_agents(&cfg.agents.idle)?
            .into_iter()
            .filter(|a| Some(&a.id) != from.as_ref())
            .filter(|a| a.cwd.as_deref().is_some_and(|c| key(Path::new(c)) == here))
            .map(|a| a.id)
            .collect();
        if ids.is_empty() {
            bail!("no other live agent in this project");
        }
        ids
    } else {
        let prefix = to.unwrap_or_default();
        let id = store
            .resolve_agent(&prefix)
            .map_err(|e| anyhow::anyhow!("agent {prefix}: {e}"))?;
        vec![id]
    };
    for id in targets {
        let msg = store.send_message_on(from.as_deref(), &id, &text, task)?;
        crate::log::stdout_ln(&format!(
            "sent #{msg} to {}",
            crate::store::short_agent_id(&id)
        ));
    }
    Ok(())
}

/// `rtok agents inbox` (T287): with no id the caller reads — and marks read — its own queue;
/// with one the user peeks at that agent's queue and marks nothing.
pub fn inbox(
    cfg: &Config,
    id: Option<String>,
    unread: bool,
    task: Option<&str>,
    json: bool,
) -> Result<()> {
    let store = Store::open(&cfg.core.db_path)?;
    let (to, mark) = match id {
        Some(prefix) => (
            store
                .resolve_agent(&prefix)
                .map_err(|e| anyhow::anyhow!("agent {prefix}: {e}"))?,
            false,
        ),
        None => match caller_agent(&store)? {
            Some(me) => (me, true),
            None => bail!("not inside an agent session; pass an agent id"),
        },
    };
    let rows = store.inbox_limited(&to, unread, mark, None, task)?;
    if json {
        let framed: Vec<serde_json::Value> = rows
            .iter()
            .map(|m| {
                let mut v = serde_json::to_value(m).unwrap_or_default();
                v["framed"] = crate::render::agent_message_frame(m).into();
                v
            })
            .collect();
        crate::log::stdout_ln(&serde_json::to_string_pretty(&framed)?);
        return Ok(());
    }
    if rows.is_empty() {
        crate::log::stdout_ln("no messages");
    }
    for (i, m) in rows.iter().enumerate() {
        if i > 0 {
            crate::log::stdout_ln("");
        }
        crate::log::stdout(&crate::render::agent_message_frame(m));
    }
    Ok(())
}
