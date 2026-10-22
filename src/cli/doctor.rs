// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use super::util::print_json;
use crate::config::Config;
use crate::model;
use anyhow::Result;
use clap::ValueEnum;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

/// The checklist terminal: the screen on stdout, the answer from stdin.
struct Terminal;

impl crate::doctor::checklist::Prompt for Terminal {
    fn ask(&mut self, screen: &str) -> Option<String> {
        print!("{screen}");
        io::stdout().flush().ok()?;
        let mut line = String::new();
        (io::stdin().read_line(&mut line).ok()? > 0).then_some(line)
    }
}

/// `--only` for `rtok doctor --fix` (D14: a `ValueEnum`); each is a `Problem::kind` of the check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(super) enum FixClass {
    BrokenHooks,
    DuplicateHooks,
    DuplicateMcp,
    Permissions,
}

impl FixClass {
    fn kind(self) -> &'static str {
        match self {
            Self::Permissions => "permissions",
            _ => crate::doctor::fix::KINDS[self as usize],
        }
    }
}

/// `rtok doctor --agent <HOST>`: the host id, or an error naming the valid ones.
fn doctor_host(agent: Option<&str>) -> Result<Option<&'static str>> {
    agent
        .map(crate::doctor::hooks::host_id)
        .transpose()
        .map_err(anyhow::Error::msg)
}

fn doctor_flags(instructions: bool) -> Option<figment::value::Dict> {
    if !instructions {
        return None;
    }
    use figment::value::{Dict, Value};
    let mut doctor = Dict::new();
    doctor.insert("instructions".into(), Value::from(true));
    let mut flags = Dict::new();
    flags.insert("doctor".into(), Value::from(doctor));
    Some(flags)
}

#[derive(clap::Args)]
pub(super) struct Args {
    // T7.2
    /// Also run the instruction-file audit
    #[arg(long)]
    pub(super) instructions: bool,
    /// JSON instead of the table
    #[arg(long, conflicts_with = "fix")]
    pub(super) json: bool,
    /// Repair hooks, MCP entries and private data permissions; --yes applies, a pipe previews
    #[arg(long)]
    pub(super) fix: bool,
    /// With --fix: apply changes (config edits are backed up; permissions change in place)
    #[arg(long, requires = "fix")]
    pub(super) yes: bool,
    /// With --fix --yes: print the diffs and write nothing
    #[arg(long, requires = "yes")]
    pub(super) dry_run: bool,
    // T416.3
    /// With --fix: stat lines and totals instead of the diffs (a dry run)
    #[arg(long, requires = "fix")]
    pub(super) stat: bool,
    /// With --fix: limit it to these problems (repeatable; default: all of them)
    #[arg(long, requires = "fix", value_enum)]
    pub(super) only: Vec<FixClass>,
    /// Check (and with --fix, repair) the hooks of one host only (an id of `rtok agents list`)
    #[arg(long, value_name = "HOST")]
    pub(super) agent: Option<String>,
}

pub(super) fn run(config_file: &Option<PathBuf>, args: Args) -> Result<()> {
    match args {
        Args {
            instructions,
            fix: true,
            yes,
            dry_run,
            stat,
            only,
            agent,
            ..
        } => {
            crate::stat_preview::set(stat);
            let agent = doctor_host(agent.as_deref())?;
            anyhow::ensure!(
                agent.is_none() || !only.contains(&FixClass::Permissions),
                "--only permissions checks rtok data, not a host; omit --agent"
            );
            anyhow::ensure!(
                cfg!(unix) || !only.contains(&FixClass::Permissions),
                "permission-bit repair is Unix-only; Windows ACLs require manual review"
            );
            let cfg = Config::load_with(config_file.as_deref(), doctor_flags(instructions))?;
            let kinds: Vec<&str> = if only.is_empty() {
                crate::doctor::fix::KINDS.to_vec()
            } else {
                only.iter().map(|c| c.kind()).collect()
            };
            // A terminal and no `--yes`: the user picks what goes. Pipes and CI keep the dry run.
            let mut terminal = Terminal;
            let ask = (!yes && !stat && io::stdin().is_terminal() && io::stdout().is_terminal())
                .then_some(&mut terminal as &mut dyn crate::doctor::checklist::Prompt);
            let (mut text, mut code) = if kinds.iter().any(|k| *k != "permissions") {
                crate::doctor::fix::run(&cfg, yes && !dry_run && !stat, agent, &kinds, ask)
            } else {
                (String::new(), 0)
            };
            if agent.is_none() && (only.is_empty() || only.contains(&FixClass::Permissions)) {
                // Permission repair is explicit; a hook checklist cannot consent to chmod.
                let (permission_text, permission_code) =
                    crate::doctor::permissions::run(&cfg, yes && !dry_run && !stat);
                text.push_str(&permission_text);
                code = code.max(permission_code);
            }
            print!("{text}");
            if code != 0 {
                std::process::exit(code);
            }

            Ok(())
        }
        Args {
            instructions,
            json,
            agent,
            ..
        } => {
            let agent = doctor_host(agent.as_deref())?;
            let cfg = Config::load_with(config_file.as_deref(), doctor_flags(instructions))?;
            let mut report = model::doctor(&cfg)?;
            report
                .problems
                .retain(|p| agent.is_none_or(|a| p.agent == a));
            if json {
                print_json(&report)?;
            } else {
                report.junk_bytes = Some(crate::agents::junk::report(&cfg).freed_default_bytes);
                print!("{}", report.to_console());
            }

            Ok(())
        }
    }
}
