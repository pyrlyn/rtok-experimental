// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use super::util::{print_diff, print_json};
use crate::config::Config;
use crate::config::validate;
use crate::model;
use crate::ui::style;
use anyhow::{Result, bail};
use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub(super) enum ConfigCmd {
    /// Write the annotated reference file to `<home>/config.toml`
    Init {
        /// Overwrite an existing file
        #[arg(long)]
        force: bool,
        /// Print the diff it would write and exit
        #[arg(long)]
        dry_run: bool,
        // T416.3
        /// With --dry-run: stat lines and totals instead of the diff
        #[arg(long, requires = "dry_run")]
        stat: bool,
    },
    /// Print the path of the config file
    Path,
    /// Print every effective key
    Show {
        /// Append `(default|user|project|env|flag)` from figment metadata
        #[arg(long)]
        sources: bool,
        /// JSON array of `{key,value,source}`
        #[arg(long)]
        json: bool,
    },
    /// Print one key's effective value
    Get {
        key: String,
        // T228
        /// JSON `{key,value,source}` instead of the bare value
        #[arg(long)]
        json: bool,
    },
    /// Reject unknown keys, wrong types, and out-of-range values
    Validate {
        /// File to check (else the user config file)
        path: Option<PathBuf>,
    },
    /// Edit one key in the user file, preserving comments
    Set {
        key: String,
        value: String,
        /// Print the diff it would write and exit
        #[arg(long)]
        dry_run: bool,
        // T416.3
        /// With --dry-run: stat lines and totals instead of the diff
        #[arg(long, requires = "dry_run")]
        stat: bool,
    },
}

/// The diff of a dry run closes with the totals line (or becomes the stat lines); a real write
/// prints its diff as before.
fn preview(path: &std::path::Path, diff: &str, dry_run: bool) -> String {
    if dry_run {
        // The registered painter has already coloured `diff`; it only needs counting.
        let shown = if crate::stat_preview::on() { "" } else { diff };
        crate::stat_preview::show(&[(path.to_path_buf(), diff.to_string())], shown)
    } else {
        diff.to_string()
    }
}

fn show(rows: &[model::ConfigEntry], sources: bool, json: bool) -> Result<()> {
    if json {
        return print_json(rows);
    }
    for r in rows {
        if sources {
            println!("{} = {} ({})", r.key, r.value, r.source);
        } else {
            println!("{} = {}", r.key, r.value);
        }
    }
    Ok(())
}

pub(super) fn run(config_file: &Option<PathBuf>, action: ConfigCmd) -> Result<()> {
    let home = Config::home_dir();
    let user = Config::user_path(&home, config_file.as_deref());
    match action {
        ConfigCmd::Init {
            force,
            dry_run,
            stat,
        } => {
            crate::stat_preview::set(stat);
            let (path, diff) = Config::init_maybe(&home, config_file.as_deref(), force, dry_run)?;
            println!("{}", path.display());
            print_diff(&preview(&path, &diff, dry_run));
        }
        ConfigCmd::Path => println!("{}", user.display()),
        ConfigCmd::Show { sources, json } => {
            let rows = model::config_entries(&home, config_file.as_deref())?;
            show(&rows, sources, json)?;
        }
        ConfigCmd::Get { key, json } => {
            let rows = model::config_entries(&home, config_file.as_deref())?;
            match rows.into_iter().find(|r| r.key == key) {
                Some(r) if json => print_json(&r)?,
                Some(r) => println!("{}", r.value),
                None => bail!("unknown key: {key}"),
            }
        }
        ConfigCmd::Validate { path } => {
            // T362: only the implicit default file is created, as `load_with` does;
            // a path the user typed must exist.
            if path.is_none() {
                Config::ensure_user_file(&home, config_file.as_deref())?;
            }
            let path = path.unwrap_or(user);
            let mut errs = validate::issues(&path)?;
            let notes = validate::pinned_notes(&path);
            // The filter drop-ins are deployment state, not part of the
            // file: read them through the same file as the user layer
            // (`--config` wins when both are given). `layers::load`
            // does not create the user config. A reported issue is
            // also appended to the log.
            let layer = config_file.as_deref().or(Some(&path));
            let cfg = crate::config::layers::load(&home, layer, None).unwrap_or_default();
            // Values from the project file, `.env` and the environment skip the file check.
            errs.extend(validate::layered_issues(crate::config::layers::sourced(
                &crate::config::layers::figment(&home, layer, None),
            )));
            errs.extend(validate::rules_issues(
                &cfg.plugins.cmd.rules,
                &cfg.plugins.cmd.rules_dir,
            ));
            if errs.is_empty() {
                for note in &notes {
                    println!("{note}");
                }
                println!("{}", style::success(&format!("ok {}", path.display())));
            } else {
                for e in &errs {
                    eprintln!("{}", style::error(&e.to_string()));
                    crate::log::append(&cfg, "error", "config", "validate", &e.to_string());
                }
                std::process::exit(1);
            }
        }
        ConfigCmd::Set {
            key,
            value,
            dry_run,
            stat,
        } => {
            crate::stat_preview::set(stat);
            let (path, diff) =
                validate::set_with(&home, config_file.as_deref(), &key, &value, dry_run)?;
            if dry_run {
                // Nothing was written, so the loader would still report the old value.
                print_diff(&preview(&path, &diff, dry_run));
            } else {
                let rows = model::config_entries(&home, config_file.as_deref())?;
                match rows.into_iter().find(|r| r.key == key) {
                    Some(r) => println!("{}", r.value),
                    None => println!("{value}"),
                }
                print_diff(&diff);
            }
        }
    }

    Ok(())
}
