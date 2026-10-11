// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use crate::config::Config;
use crate::config::layers;
use anyhow::{Result, bail};
use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub(super) enum McpCmd {
    /// Prove the agent's rtok MCP server is alive and answering
    Ping {
        /// Host (`claude`, `cursor`, …); omitted checks every host with MCP installed
        agent: Option<String>,
        /// Only the CLI app
        #[arg(long)]
        cli: bool,
        /// Only the desktop app
        #[arg(long, alias = "gui")]
        desktop: bool,
        /// Seconds to wait for the agent or the server
        #[arg(long, value_name = "SECS", default_value_t = 60)]
        timeout: u64,
        /// One JSON object per host
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::Args)]
pub(super) struct Args {
    #[command(subcommand)]
    pub(super) action: Option<McpCmd>,
    // T70.3
    /// Call one listed tool and print the text result (pi `registerTool` shim)
    #[arg(long, value_name = "TOOL")]
    pub(super) call: Option<String>,
    /// JSON arguments for `--call`
    #[arg(long, value_name = "ARGS")]
    pub(super) json: Option<String>,
    // T283.1
    /// The host this MCP entry belongs to (`claude`, `cursor`, `grok`, …): overlays `[hook] host` so
    /// the process can find its rtok agent
    #[arg(long, value_name = "HOST")]
    pub(super) host: Option<String>,
    // T401
    /// Serve over Streamable HTTP at `IP:PORT` (default `[mcp] http`) instead of stdio; the
    /// bearer token comes from `[mcp] token` or `RTOK_MCP_TOKEN`
    #[arg(long, value_name = "ADDR", num_args = 0..=1, conflicts_with_all = ["call", "json"])]
    pub(super) http: Option<Option<String>>,
    /// Foreign stdio MCP server to wrap losslessly (`rtok mcp -- npx some-server`)
    #[arg(last = true)]
    pub(super) wrap: Vec<String>,
}

pub(super) fn run(config_file: &Option<PathBuf>, args: Args) -> Result<()> {
    let Args {
        action,
        call,
        json,
        host,
        http,
        wrap,
    } = args;
    let cfg = Config::load_with(config_file.as_deref(), layers::hook_host_flag(host))?;
    if let Some(addr) = http {
        if action.is_some() || !wrap.is_empty() {
            bail!("rtok mcp --http serves rtok's own tools; it takes no subcommand or `--`");
        }
        let addr = addr.unwrap_or_else(|| cfg.mcp.http.clone());
        rtok_sys::proc::disable_progress();
        return crate::mcp::http::serve_blocking(&cfg, &addr);
    }
    if let Some(McpCmd::Ping {
        agent,
        cli,
        desktop,
        timeout,
        json: as_json,
    }) = action
    {
        let code = crate::mcp::ping::run(
            &cfg,
            crate::mcp::ping::PingOpts {
                agent: agent.as_deref(),
                cli,
                desktop,
                timeout: std::time::Duration::from_secs(timeout),
                json: as_json,
            },
        )?;
        let _ = std::io::Write::flush(&mut std::io::stdout());
        std::process::exit(code);
    }
    // stderr belongs to the host from here on, so a tool that runs a program draws no spinner.
    rtok_sys::proc::disable_progress();
    if let Some(name) = call {
        if !wrap.is_empty() {
            bail!("rtok mcp --call does not wrap a foreign server");
        }
        let raw = json.as_deref().unwrap_or("{}");
        let args: serde_json::Value = serde_json::from_str(raw)?;
        match crate::mcp::call(&cfg, &name, &args) {
            Ok(text) => print!("{text}"),
            Err(e) => {
                print!("{e}");
                std::process::exit(1);
            }
        }
    } else if wrap.is_empty() {
        crate::mcp::run(&cfg)?;
    } else {
        #[cfg(feature = "cmd")]
        std::process::exit(crate::mcp::wrap::run(&cfg, &wrap)?);
        #[cfg(not(feature = "cmd"))]
        bail!("rtok mcp -- <server>: the wrapper needs the `cmd` feature");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::super::{Cli, Cmd};
    use super::{Args, McpCmd};

    #[test]
    fn mcp_ping_is_not_parsed_as_a_wrap_argv() {
        let ping = Cli::try_parse_from([
            "rtok",
            "mcp",
            "ping",
            "claude",
            "--cli",
            "--timeout",
            "5",
            "--json",
        ])
        .unwrap();
        assert!(matches!(
            ping.cmd,
            Cmd::Mcp(Args {
                action: Some(McpCmd::Ping {
                    ref agent,
                    cli: true,
                    desktop: false,
                    timeout: 5,
                    json: true,
                }),
                call: None,
                ref wrap,
                ..
            }) if agent.as_deref() == Some("claude") && wrap.is_empty()
        ));
        let wrap = Cli::try_parse_from(["rtok", "mcp", "--", "npx", "some-server"]).unwrap();
        assert!(matches!(
            wrap.cmd,
            Cmd::Mcp(Args {
                action: None,
                call: None,
                ref wrap,
                ..
            }) if wrap.as_slice() == ["npx".to_string(), "some-server".to_string()]
        ));
    }
}
