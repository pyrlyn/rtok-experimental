// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use super::util::print_json;
use crate::config::Config;
use anyhow::{Context, Result};
use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub(super) enum SkillsCmd {
    /// Likelihood each listed skill is used in a project: static analysis of its files,
    /// manifests, imports, tests and CI against each skill's description; no model
    Audit {
        /// Project directory; defaults to the current directory
        path: Option<PathBuf>,
        /// JSON instead of the table
        #[arg(long)]
        json: bool,
    },
}

pub(super) fn run(config_file: &Option<PathBuf>, action: SkillsCmd) -> Result<()> {
    match action {
        SkillsCmd::Audit { path, json } => {
            let cfg = Config::load_with(config_file.as_deref(), None)?;
            let root = match path {
                Some(p) => p,
                None => std::env::current_dir()?,
            };
            let root = std::fs::canonicalize(&root)
                .with_context(|| format!("no such directory: {}", root.display()))?;
            // User roots hang off $HOME, as in `rtok doctor`'s skills section.
            let home = crate::config::env_user_home().unwrap_or_else(|| cfg.home.clone());
            let report =
                crate::skills_audit::audit(&root, &crate::doctor::found_skills(&home, &root));
            if json {
                print_json(&report)
            } else {
                print!("{}", report.to_text());
                Ok(())
            }
        }
    }
}
