// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use crate::config::Config;
use crate::config::layers;
use crate::hooks::deadline::{self, Gate};
use anyhow::Result;
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(clap::Args)]
pub(super) struct Args {
    #[arg(required_unless_present = "serve")]
    pub(super) event: Option<String>,
    /// Overlay `[hook] host` (`claude` | `cursor` | `copilot` | `devin` | `cline`)
    #[arg(long)]
    pub(super) host: Option<String>,
    // T178, D32
    /// Run the resident hook process `rtok-hook` talks to
    #[arg(long, hide = true, conflicts_with_all = ["event", "host"])]
    pub(super) serve: bool,
}

pub(super) fn run(config_file: &Option<PathBuf>, args: Args) -> Result<()> {
    match args {
        Args { serve: true, .. } => {
            crate::hooks::resident::serve()?;
            Ok(())
        }
        // T159: these two events answer with a path and an exit code, not the JSON the hook
        // dispatcher writes, so they skip it.
        Args {
            event: Some(event),
            host,
            ..
        } if crate::worktree::host::handles(&event) => {
            let cfg = Config::load_lenient(config_file.as_deref(), None);
            let host = host.as_deref();
            if let Some(out) = crate::worktree::host::run(&event, host, io::stdin(), &cfg)? {
                println!("{out}");
            }

            Ok(())
        }
        Args { event, host, .. } => {
            let event = event.unwrap_or_default();
            // T921: armed before the config loads and stdin is read, the two places a hook can
            // hang without ever reaching the dispatcher.
            let gate = Gate::default();
            let limit = rtok_hook::deadline(|k| std::env::var_os(k));
            let _watchdog = deadline::arm_stdout(&event, limit, &gate);
            let mut cfg =
                Config::load_lenient(config_file.as_deref(), layers::hook_host_flag(host));
            cfg.hook_client_pid = Some(std::process::id());
            let mut out = Vec::new();
            crate::hooks::run(&event, io::stdin(), &mut out, &cfg);
            if gate.claim() {
                let _ = io::stdout().write_all(&out);
                let _ = io::stdout().flush();
            } else {
                // The watchdog is printing `{}` and exits the process; do not add a second answer.
                std::thread::park_timeout(std::time::Duration::from_secs(1));
            }

            Ok(())
        }
    }
}
