// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Clap tree. `tests/config_coverage.rs` walks [`Cli::command`] (plan T12.4).
//!
//! Plan ids sit in plain `//` lines above the `///` they belong to: clap turns doc comments
//! into help, man pages and completion scripts, and users never see plan ids (T409,
//! `tests/no_plan_ids.rs`).

#![allow(clippy::print_stdout, clippy::print_stderr)]

mod agents;
mod agents_host;
#[cfg(feature = "archive")]
mod archive;
mod batch;
mod bench;
mod completions;
mod config;
mod demon;
#[cfg(feature = "docs")]
mod docs;
mod doctor;
mod expand;
#[cfg(feature = "graph")]
mod graph;
#[cfg(feature = "guard")]
mod guard;
mod hook;
mod info;
mod logs;
mod man_page;
mod mcp;
#[cfg(feature = "memory")]
mod memory;
mod otel;
mod plugins;
mod proxy;
mod report;
mod run_cmd;
mod stats;
mod task;
mod util;
mod web;
mod worktree;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

/// Token-reduction CLI for AI coding agents. See plan.md for the task list.
#[derive(Parser)]
// `bin_name`: clap would print argv[0]'s file name, `rtok.exe` on Windows (T83.7).
#[command(name = "rtok", bin_name = "rtok", version = crate::VERSION, about)]
#[command(styles = crate::ui::style::CLAP)]
pub struct Cli {
    /// User config file (else `RTOK_CONFIG` or `<home>/config.toml`)
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Claude Code hook entry point: reads the event JSON on stdin, writes JSON to stdout
    Hook(hook::Args),

    /// Serve MCP tools over stdio; `-- <server argv>` wraps a foreign server instead
    Mcp(mcp::Args),

    /// Local API proxy for ANTHROPIC_BASE_URL
    Proxy(proxy::Args),

    /// Local web UI over the same data as `rtok tui` (WebSocket API + React SPA)
    Web(web::WebArgs),

    // D23
    /// Terminal UI over the same data as `rtok web` (one model, two renderings)
    Tui(web::TuiArgs),

    /// Measurements from session logs and the proxy
    Stats(stats::Args),

    /// A/B benchmark of host configurations
    Bench(bench::Args),

    /// Inspect hooks, MCP servers and the proxy chain
    Doctor(doctor::Args),

    /// Git worktrees of this repository: owner, state and disk cost
    Worktree {
        #[command(subcommand)]
        action: worktree::WorktreeCmd,
    },

    /// This project's tasks: numbered by rtok, stored by the `[tasks]` adapter
    Task {
        #[command(subcommand)]
        action: task::TaskCmd,
    },

    // T385.12.1
    /// Provider Batch jobs (Anthropic, OpenAI) sent through `rtok proxy`
    Batch {
        #[command(subcommand)]
        action: batch::BatchCmd,
    },

    /// Version, effective paths, disk usage, error count and proxy status
    Info(info::Args),

    /// Agent hosts (`rtok agents install|uninstall|list|info …`)
    #[command(visible_alias = "agent")]
    Agents {
        #[command(subcommand)]
        action: agents::AgentCmd,
    },

    /// Deprecated spelling of `rtok agents install <host>`; still runs, still prints where to go
    #[command(hide = true)]
    Setup(agents_host::SetupArgs),

    /// Execute a command, archive its raw output, print the filtered version
    Run(run_cmd::RunArgs),

    /// Filter text from stdin without executing
    Filter(run_cmd::FilterArgs),

    /// Print an archived payload
    Expand(expand::Args),

    // T70.2
    /// The archive live zone (`rtok archive rewrite` — pi `context` carrier)
    #[cfg(feature = "archive")]
    Archive {
        #[command(subcommand)]
        action: archive::ArchiveCmd,
    },

    /// Print, install or pick shell completions (bash, zsh, fish, powershell, elvish; clink for Windows cmd)
    ///
    /// With a shell, prints its script. With `--install` or `--uninstall`, writes or removes the
    /// script in that shell's per-user completions directory (default shell: `$SHELL`). With
    /// nothing, in a terminal, opens a list of the shells: the ones with completions installed
    /// are checked; checking one installs it, unchecking removes it. `--list` shows the same
    /// state without a terminal.
    Completions(completions::Args),

    /// Print the man page (roff), or write every page with `--dir`
    Man(man_page::Args),

    /// List plugins: id, enabled, surfaces
    Plugins(plugins::Args),

    /// The one config file
    Config {
        #[command(subcommand)]
        action: config::ConfigCmd,
    },

    /// Notes (`mem_save` / import / export)
    #[cfg(feature = "memory")]
    Memory {
        #[command(subcommand)]
        action: memory::MemoryCmd,
    },

    /// Cached rustdoc for Cargo.lock dependencies (`rtok docs fetch`)
    #[cfg(feature = "docs")]
    Docs {
        #[command(subcommand)]
        action: docs::DocsCmd,
    },

    /// Symbol index (`rtok graph index`)
    #[cfg(feature = "graph")]
    Graph {
        #[command(subcommand)]
        action: graph::GraphCmd,
    },

    // T70.5
    /// Duplicate-call verdict (`rtok guard check` — pi / OpenCode plugin path)
    #[cfg(feature = "guard")]
    Guard {
        #[command(subcommand)]
        action: guard::GuardCmd,
    },

    /// Deprecated spelling of `rtok web`; still runs, still prints where to go
    #[command(hide = true)]
    Dashboard(web::DashboardArgs),

    /// Keep `rtok proxy` (or `mcp` / `web`) running in the background
    Demon {
        #[command(subcommand)]
        action: demon::DemonCmd,
    },

    /// OpenTelemetry export (`rtok otel flush | status`)
    Otel {
        #[command(subcommand)]
        action: otel::OtelCmd,
    },

    /// rtok's own log (`rtok logs` prints, `rtok logs export` strips numbering and colour)
    Logs(logs::Args),

    // D24
    /// The operator model as one document: Markdown, HTML and PDF
    Report(report::Args),
}

pub fn run() -> Result<()> {
    // Library code never writes the terminal. These hooks are the one stdout/stderr path.
    crate::log::on_stdout(|text| print!("{text}"));
    crate::log::on_stderr(|text| eprint!("{text}"));
    crate::store::install_migrate_spinner();
    // T225: `RUST_LOG` debug log on stderr, before clap so a parse failure is logged too.
    crate::log::init_stderr();
    // `[ui]` takes effect for every command. Config load calls this hook instead of
    // naming `ui` itself. The error path in `main` still loads config after this.
    crate::config::layers::on_early_warning(|msg| eprintln!("rtok: {msg}"));
    crate::config::layers::on_load(|cfg| {
        crate::log::emit_config_notes(cfg);
        crate::ui::style::configure(&cfg.ui);
    });
    crate::config::validate::register_file_diff(crate::render::file_diff);
    #[cfg(feature = "cmd")]
    crate::config::validate::register_rules(crate::plugins::cmd::rules::issues_in);
    log::debug!(target: "rtok::cli", "argv {:?}", std::env::args_os().collect::<Vec<_>>());
    let cli = Cli::parse();
    let config_file = cli.config.clone();
    match cli.cmd {
        Cmd::Hook(args) => hook::run(&config_file, args),
        Cmd::Mcp(args) => mcp::run(&config_file, args),
        Cmd::Proxy(args) => proxy::run(&config_file, args),
        Cmd::Web(args) => web::serve(&config_file, args),
        Cmd::Tui(args) => web::tui(&config_file, args),
        Cmd::Stats(args) => stats::run(&config_file, args),
        Cmd::Bench(args) => bench::run(&config_file, args),
        Cmd::Doctor(args) => doctor::run(&config_file, args),
        Cmd::Worktree { action } => worktree::run(&config_file, action),
        Cmd::Task { action } => task::run(action, config_file.as_deref()),
        Cmd::Batch { action } => batch::run(action, config_file.as_deref()),
        Cmd::Info(args) => info::run(&config_file, args),
        Cmd::Agents { action } => agents::run(&config_file, action),
        Cmd::Setup(args) => agents_host::setup(&config_file, args),
        Cmd::Run(args) => run_cmd::run(&config_file, args),
        Cmd::Filter(args) => run_cmd::filter(&config_file, args),
        Cmd::Expand(args) => expand::run(&config_file, args),
        #[cfg(feature = "archive")]
        Cmd::Archive { action } => archive::run(&config_file, action),
        Cmd::Completions(args) => completions::run(&config_file, args),
        Cmd::Man(args) => man_page::run(&config_file, args),
        Cmd::Plugins(args) => plugins::run(&config_file, args),
        Cmd::Config { action } => config::run(&config_file, action),
        #[cfg(feature = "memory")]
        Cmd::Memory { action } => memory::run(&config_file, action),
        #[cfg(feature = "docs")]
        Cmd::Docs { action } => docs::run(&config_file, action),
        #[cfg(feature = "graph")]
        Cmd::Graph { action } => graph::run(&config_file, action),
        #[cfg(feature = "guard")]
        Cmd::Guard { action } => guard::run(&config_file, action),
        Cmd::Dashboard(args) => web::dashboard(&config_file, args),
        Cmd::Demon { action } => demon::run(&config_file, action),
        Cmd::Otel { action } => otel::run(&config_file, action),
        Cmd::Logs(args) => logs::run(&config_file, args),
        Cmd::Report(args) => report::run(&config_file, args),
    }
}

#[cfg(test)]
mod tests;
