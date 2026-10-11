// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use anyhow::Result;
use clap::CommandFactory;
use std::io;
use std::path::PathBuf;

#[derive(clap::Args)]
pub(super) struct Args {
    /// Shell to complete for (with `--install`/`--uninstall`: default `$SHELL`)
    pub(super) shell: Option<crate::completions::Shell>,
    /// Write the script to the shell's per-user completions directory
    #[arg(long, conflicts_with_all = ["uninstall", "list"], group = "install_or_uninstall")]
    pub(super) install: bool,
    /// Remove what `--install` wrote
    #[arg(long, conflicts_with = "list", group = "install_or_uninstall")]
    pub(super) uninstall: bool,
    /// Print each shell, whether its completions are installed (yes/no) and the file
    #[arg(long, conflicts_with = "shell")]
    pub(super) list: bool,
    /// With `--install`/`--uninstall`: show the file diff and write nothing
    #[arg(long, requires = "install_or_uninstall")]
    pub(super) dry_run: bool,
}

pub(super) fn run(_config_file: &Option<PathBuf>, args: Args) -> Result<()> {
    let Args {
        shell,
        install,
        uninstall,
        list,
        dry_run,
    } = args;
    use crate::completions::install::{self as inst, Places};
    use crate::completions::picker;
    use std::io::IsTerminal;
    let lines = if list {
        picker::list(&Places::from_env()?)
    } else if install || uninstall {
        let places = Places::from_env()?;
        let shell = places.pick(shell)?;
        if dry_run {
            let preview = if install {
                inst::preview_install(shell, super::Cli::command(), &places)?
            } else {
                inst::preview_uninstall(shell, &places)?
            };
            let text = change_preview::paint(&preview.render(change_preview::Mode::Diff));
            vec![if text.is_empty() {
                "nothing to change".to_string()
            } else {
                text.trim_end().to_string()
            }]
        } else if install {
            inst::install(shell, super::Cli::command(), &places)?
        } else {
            inst::uninstall(shell, &places)?
        }
    } else if let Some(shell) = shell {
        crate::completions::generate(shell, super::Cli::command(), &mut io::stdout());
        Vec::new()
    } else if io::stdin().is_terminal() && io::stdout().is_terminal() {
        picker::run(
            &Places::from_env()?,
            super::Cli::command(),
            picker::ask_terminal,
        )?
    } else {
        // Never wait for input that cannot come (pipes, CI, agents).
        anyhow::bail!(
            "no shell named and no terminal for the picker; run `rtok completions <shell>` \
             to print a script, `rtok completions --install` to write one, or `--list` for the state"
        )
    };
    for line in lines {
        println!("{line}");
    }

    Ok(())
}
