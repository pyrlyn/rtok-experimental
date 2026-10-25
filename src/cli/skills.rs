// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok skills list | enable | disable`.

use std::path::PathBuf;

use anyhow::{Context, Result};

use super::util::print_json;
use crate::skills::{self, Agent, Roots, Scope, ToggleRequest};

#[derive(clap::Subcommand)]
pub(super) enum SkillsCmd {
    /// List skills of the agent, enabled and disabled
    List(ListArgs),
    /// Enable a disabled skill
    Enable(Toggle),
    /// Disable a skill (moves it to `<skills dir>-disabled/`)
    Disable(Toggle),
}

#[derive(clap::Args)]
pub(super) struct Common {
    /// claude, cursor or copilot (else detected from the environment)
    #[arg(long)]
    agent: Option<String>,
    /// user or project (default: both)
    #[arg(long, value_parser = ["user", "project"])]
    scope: Option<String>,
    /// Project directory for project-scope skills (default: current directory)
    #[arg(long, value_name = "DIR")]
    project: Option<PathBuf>,
    /// JSON (the OpenAPI contract shapes) instead of text
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
pub(super) struct ListArgs {
    #[command(flatten)]
    common: Common,
    /// Add a column with the exact size in bytes (the terminal has no hover)
    #[arg(long)]
    bytes: bool,
}

#[derive(clap::Args)]
pub(super) struct Toggle {
    /// Skill name (its directory name)
    name: String,
    #[command(flatten)]
    common: Common,
    /// Show what would move without moving it
    #[arg(long)]
    dry_run: bool,
}

fn scope(s: Option<&str>) -> Option<Scope> {
    match s {
        Some("user") => Some(Scope::User),
        Some("project") => Some(Scope::Project),
        _ => None,
    }
}

fn resolve(c: &Common) -> Result<(Agent, Roots)> {
    let agent = match &c.agent {
        Some(a) => skills::parse_agent(a)?,
        None => skills::detect_agent(|k| std::env::var(k).ok()).context(
            "cannot tell which agent rtok runs in; pass --agent claude|cursor|copilot or set RTOK_AGENT",
        )?,
    };
    let home = rtok_hook::user_home_from(std::env::var_os("HOME"), std::env::var_os("USERPROFILE"))
        .context("no HOME or USERPROFILE")?;
    let project = match &c.project {
        Some(p) => p.clone(),
        None => std::env::current_dir()?,
    };
    let copilot_home = std::env::var_os("COPILOT_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    Ok((
        agent,
        Roots {
            home,
            project,
            copilot_home,
        },
    ))
}

pub(super) fn run(cmd: SkillsCmd) -> Result<()> {
    match cmd {
        SkillsCmd::List(ListArgs { common: c, bytes }) => {
            let (agent, roots) = resolve(&c)?;
            let out = skills::list(&roots, agent, scope(c.scope.as_deref()));
            if c.json {
                return print_json(&out);
            }
            if out.skills.is_empty() {
                println!("no {agent} skills");
            }
            for s in &out.skills {
                let state = if s.enabled { "on " } else { "off" };
                let scope = if s.scope == Scope::User {
                    "user   "
                } else {
                    "project"
                };
                let desc = s.description.as_deref().unwrap_or("");
                let size = skills::format_size(s.size_bytes);
                if bytes {
                    println!(
                        "{state} {scope} {size:>8} {:>9} {}  {desc}",
                        s.size_bytes, s.name
                    );
                } else {
                    println!("{state} {scope} {size:>8} {}  {desc}", s.name);
                }
            }
            Ok(())
        }
        SkillsCmd::Enable(t) => toggle(t, true),
        SkillsCmd::Disable(t) => toggle(t, false),
    }
}

fn toggle(t: Toggle, enabled: bool) -> Result<()> {
    let (agent, roots) = resolve(&t.common)?;
    let mut req = ToggleRequest::new(agent, t.name, enabled);
    req.scope = scope(t.common.scope.as_deref());
    req.dry_run = Some(t.dry_run);
    let res = skills::toggle(&roots, &req)?;
    if t.common.json {
        return print_json(&res);
    }
    let verb = if enabled { "enabled" } else { "disabled" };
    match (res.changed, &res.from, &res.to) {
        (false, ..) => println!("{} already {verb}", res.skill.name),
        (true, Some(from), Some(to)) => {
            let pre = if t.dry_run { "would move" } else { "moved" };
            println!(
                "{agent} skill {} {verb}: {pre} {from} -> {to}",
                res.skill.name
            );
        }
        _ => println!("{} {verb}", res.skill.name),
    }
    Ok(())
}
