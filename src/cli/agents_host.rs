// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use crate::config::Config;
use crate::ui::style;
use anyhow::Result;
use std::path::PathBuf;

#[derive(clap::Args)]
pub(super) struct RemoveArgs {
    /// Host(s), comma-separated (`claude`, `cursor`, `codex`, `opencode`, `pi`, `zcode`, `kimi`, `copilot`, `aider`, `windsurf`, `zed`, `vscode`)
    pub(super) host: String,
    /// Print what would be removed and exit
    #[arg(long)]
    pub(super) dry_run: bool,
    // T141
    /// Skip closing/reopening a running desktop app around the write
    #[arg(long)]
    pub(super) no_restart: bool,
    // T246
    /// Also remove rtok entries you changed, without asking
    #[arg(long)]
    pub(super) yes: bool,
    // T289.3
    /// Take rtok out of the project's post-create script (`cursor`, `kilo`, `windsurf`, `devin`) instead of the host's config
    #[arg(long)]
    pub(super) project: bool,
    // T416.3
    /// With --dry-run: stat lines and totals instead of the planned edits
    #[arg(long, requires = "dry_run")]
    pub(super) stat: bool,
}

#[derive(clap::Args)]
pub(super) struct UpdateArgs {
    /// Host(s), comma-separated; omitted = every host rtok is installed in
    pub(super) host: Option<String>,
    /// Print the planned edits and exit
    #[arg(long)]
    pub(super) dry_run: bool,
    /// Only the CLI app (default is all)
    #[arg(long)]
    pub(super) cli: bool,
    /// Only the desktop app (default is all)
    #[arg(long, alias = "gui")]
    pub(super) desktop: bool,
    /// All variants (the default when neither `--cli` nor `--desktop` is given)
    #[arg(long)]
    pub(super) all: bool,
    // T141
    /// Skip closing/reopening a running desktop app around the write
    #[arg(long)]
    pub(super) no_restart: bool,
    // T279
    /// Reinstall the plugin even when it is already at the available version
    #[arg(long)]
    pub(super) force: bool,
    // T279
    /// Compare against this install source instead of the one on record
    #[arg(long, value_enum)]
    pub(super) source: Option<SourceArg>,
    /// List outdated plugins only (same output as `agents outdated`)
    #[arg(long)]
    pub(super) check: bool,
    // T416.3
    /// With --dry-run: stat lines and totals instead of the planned edits
    #[arg(long, requires = "dry_run")]
    pub(super) stat: bool,
}

/// `--source` for `rtok agents update` (T279): mirrors `agents::plugin_version::Source`, kept
/// separate so this module does not need that one's `serde`/`FromStr` shape.
#[derive(Copy, Clone, clap::ValueEnum)]
pub(super) enum SourceArg {
    Github,
    Local,
    Marketplace,
}

impl SourceArg {
    fn as_str(self) -> &'static str {
        match self {
            SourceArg::Github => "github",
            SourceArg::Local => "local",
            SourceArg::Marketplace => "marketplace",
        }
    }
}

/// `rtok agents outdated` (T279.1): plugin versions behind the running rtok.
#[derive(clap::Args)]
pub(super) struct OutdatedArgs {
    /// Host(s), comma-separated; omitted = every host in `agents list`
    pub(super) host: Option<String>,
    /// Only the CLI app (default is all)
    #[arg(long)]
    pub(super) cli: bool,
    /// Only the desktop app (default is all)
    #[arg(long, alias = "gui")]
    pub(super) desktop: bool,
    /// All variants (the default when neither `--cli` nor `--desktop` is given)
    #[arg(long)]
    pub(super) all: bool,
    /// JSON instead of the table
    #[arg(long)]
    pub(super) json: bool,
    /// Exit with code 10 when at least one plugin is outdated
    #[arg(long)]
    pub(super) exit_code: bool,
}

/// One definition behind `rtok agents install` and the deprecated `rtok setup`.
#[derive(clap::Args)]
pub(super) struct SetupArgs {
    /// Host(s), comma-separated (`claude`, `cursor`, `codex`, `opencode`, `pi`, `zcode`, `kimi`, `copilot`, `aider`, `windsurf`, `zed`, `vscode`)
    pub(super) host: String,
    /// Print the planned edits and exit
    #[arg(long)]
    pub(super) dry_run: bool,
    /// Remove rtok from the host (hooks, MCP, proxy, plugin link); prefer `rtok agents uninstall <host>`
    #[arg(long)]
    pub(super) remove: bool,
    /// Enable prompt modes (`terse,yagni`)
    #[arg(long, value_delimiter = ',')]
    pub(super) mode: Vec<String>,
    /// Confirm destructive `--replace`
    #[arg(long)]
    pub(super) yes: bool,
    /// Remove legacy token hooks and retarget the proxy
    #[arg(long)]
    pub(super) replace: bool,
    /// Register `rtok mcp` in the host MCP map
    #[arg(long)]
    pub(super) mcp: bool,
    /// Set `env.ANTHROPIC_BASE_URL` to this proxy
    #[arg(long)]
    pub(super) proxy: bool,
    /// Only the CLI app (`cursor`/`opencode` have CLI and desktop; default is all)
    #[arg(long)]
    pub(super) cli: bool,
    /// Only the desktop app (`cursor`/`opencode` have CLI and desktop; default is all)
    #[arg(long, alias = "gui")]
    pub(super) desktop: bool,
    /// All variants (the default when neither `--cli` nor `--desktop` is given)
    #[arg(long)]
    pub(super) all: bool,
    // T141
    /// Skip closing/reopening a running desktop app around the write
    #[arg(long)]
    pub(super) no_restart: bool,
    // T289.3
    /// Add rtok to the project's post-create script, so a worktree the host makes joins rtok
    /// (`cursor`, `kilo`, `windsurf`, `devin`); run it inside the repository
    #[arg(long)]
    pub(super) project: bool,
    // T416.3
    /// With --dry-run: stat lines and totals instead of the planned edits
    #[arg(long, requires = "dry_run")]
    pub(super) stat: bool,
}

impl SetupArgs {
    /// `rtok agents uninstall <host>` is the install run backwards; nothing else about it differs.
    pub(super) fn removing(args: RemoveArgs) -> Self {
        Self {
            host: args.host,
            dry_run: args.dry_run,
            remove: true,
            mode: Vec::new(),
            yes: args.yes,
            replace: false,
            mcp: false,
            proxy: false,
            cli: false,
            desktop: false,
            all: true,
            no_restart: args.no_restart,
            project: args.project,
            stat: args.stat,
        }
    }
}

/// The host installers, one call site for `rtok agents install|uninstall` and the deprecated
/// `rtok setup`. Unknown hosts are refused before any backup is taken.
pub(super) fn setup_host(config_file: Option<&std::path::Path>, args: SetupArgs) -> Result<()> {
    let SetupArgs {
        host,
        dry_run,
        remove,
        mode,
        yes,
        replace,
        mcp,
        proxy,
        cli,
        desktop,
        all,
        no_restart,
        project,
        stat,
    } = args;
    crate::stat_preview::set(stat);
    crate::agents::command::install(
        config_file,
        crate::agents::command::Install {
            host,
            remove,
            replace,
            cli,
            desktop,
            all,
            no_restart,
            project,
        },
        setup_flags(dry_run, yes, mcp, proxy, &mode, false, None),
    )
}

/// `rtok agents update [host,…]` (T242.2): the named hosts, or every host rtok is installed
/// in, through the same backup/restart path as install.
pub(super) fn outdated_from_update(args: &UpdateArgs) -> OutdatedArgs {
    OutdatedArgs {
        host: args.host.clone(),
        cli: args.cli,
        desktop: args.desktop,
        all: args.all,
        json: false,
        exit_code: false,
    }
}

pub(super) fn outdated_hosts(
    config_file: Option<&std::path::Path>,
    args: OutdatedArgs,
) -> Result<()> {
    crate::agents::command::outdated(
        config_file,
        crate::agents::command::Outdated {
            host: args.host,
            cli: args.cli,
            desktop: args.desktop,
            all: args.all,
            json: args.json,
            exit_code: args.exit_code,
        },
    )
}

pub(super) fn update_hosts(config_file: Option<&std::path::Path>, args: UpdateArgs) -> Result<()> {
    crate::stat_preview::set(args.stat);
    let flags = setup_flags(
        args.dry_run,
        false,
        false,
        false,
        &[],
        args.force,
        args.source.map(SourceArg::as_str),
    );
    crate::agents::command::update(
        config_file,
        crate::agents::command::Update {
            host: args.host,
            cli: args.cli,
            desktop: args.desktop,
            all: args.all,
            no_restart: args.no_restart,
        },
        flags,
    )
}

fn setup_flags(
    dry_run: bool,
    yes: bool,
    mcp: bool,
    proxy: bool,
    mode: &[String],
    force: bool,
    source: Option<&str>,
) -> Option<figment::value::Dict> {
    if !dry_run && !yes && !mcp && !proxy && mode.is_empty() && !force && source.is_none() {
        return None;
    }
    use figment::value::{Dict, Value};
    let mut setup = Dict::new();
    if dry_run {
        setup.insert("dry_run".into(), Value::from(true));
    }
    if yes {
        setup.insert("yes".into(), Value::from(true));
    }
    if mcp {
        setup.insert("mcp".into(), Value::from(true));
    }
    if proxy {
        setup.insert("proxy".into(), Value::from(true));
    }
    if force {
        setup.insert("force".into(), Value::from(true));
    }
    if let Some(s) = source {
        setup.insert("source".into(), Value::from(s));
    }
    if !mode.is_empty() {
        setup.insert(
            "modes".into(),
            Value::from(
                mode.iter()
                    .map(|s| Value::from(s.as_str()))
                    .collect::<Vec<_>>(),
            ),
        );
    }
    let mut flags = Dict::new();
    flags.insert("setup".into(), Value::from(setup));
    Some(flags)
}

pub(super) fn setup(config_file: &Option<PathBuf>, args: SetupArgs) -> Result<()> {
    let msg = format!(
        "`rtok setup {0}` is deprecated; use `rtok agents install {0}`",
        args.host
    );
    eprintln!("{}", style::warn(&format!("warning: {msg}")));
    let cfg = Config::load_lenient(config_file.as_deref(), None);
    crate::log::append(&cfg, "warn", "cli", "setup", &msg);
    setup_host(config_file.as_deref(), args)?;

    Ok(())
}
