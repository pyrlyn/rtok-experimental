// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T275.1: `rtok mcp ping` is its own subcommand, and a configured server answers `ping`.

use clap::CommandFactory;
use rtok::cli::Cli;

#[test]
fn mcp_ping_is_a_subcommand_and_wrap_stays_behind_double_dash() {
    let ping = Cli::command()
        .try_get_matches_from([
            "rtok",
            "mcp",
            "ping",
            "claude",
            "--cli",
            "--timeout",
            "5",
            "--json",
        ])
        .expect("ping parses");
    let mcp = ping.subcommand_matches("mcp").expect("mcp");
    assert!(mcp.get_many::<String>("wrap").is_none());
    let sub = mcp.subcommand_matches("ping").expect("ping");
    assert_eq!(
        sub.get_one::<String>("agent").map(String::as_str),
        Some("claude")
    );
    assert!(sub.get_flag("cli"));
    assert!(sub.get_flag("json"));
    assert_eq!(sub.get_one::<u64>("timeout").copied(), Some(5));

    let wrap = Cli::command()
        .try_get_matches_from(["rtok", "mcp", "--", "npx", "some-server"])
        .expect("wrap parses");
    let mcp = wrap.subcommand_matches("mcp").expect("mcp");
    assert!(mcp.subcommand_matches("ping").is_none());
    let argv: Vec<_> = mcp
        .get_many::<String>("wrap")
        .expect("wrap argv")
        .map(String::as_str)
        .collect();
    assert_eq!(argv, ["npx", "some-server"]);
}

#[test]
fn probe_spawns_the_command_from_the_host_config() {
    let home = rtok::testutil::tmp_dir("mcp-ping-probe");
    let bin = env!("CARGO_BIN_EXE_rtok");
    let body = serde_json::json!({
        "mcpServers": {
            "rtok": {
                "command": bin,
                "args": ["mcp"],
                "env": {"RTOK_HOME": home.display().to_string()}
            }
        }
    })
    .to_string();
    let launch = rtok::mcp::ping::launch_from_body(&body, "cursor").expect("rtok entry");
    assert_eq!(launch.command, bin);
    assert_eq!(launch.args, ["mcp"]);
    let text = rtok::mcp::ping::probe(&launch, "Test", std::time::Duration::from_secs(30))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(text, "MCP Test жив");
    let _ = std::fs::remove_dir_all(home);
}

/// A `claude` on PATH that runs `rtok mcp --call ping` and echoes the tool text.
#[cfg(unix)]
#[test]
fn fake_claude_on_path_echoes_the_ping_reply() {
    let home = rtok::testutil::tmp_dir("mcp-ping-claude");
    let bin_dir = home.join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let rtok_bin = env!("CARGO_BIN_EXE_rtok");
    let script = bin_dir.join("claude");
    std::fs::write(
        &script,
        "#!/bin/sh\n\
         if [ \"$1\" = \"--version\" ]; then echo claude-test; exit 0; fi\n\
         agent=$(printf '%s' \"$2\" | sed -n 's/.*agent=\"\\([^\"]*\\)\".*/\\1/p')\n\
         exec \"$RTOK_BIN\" mcp --call ping --json \"{\\\"agent\\\":\\\"$agent\\\"}\"\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let entry = serde_json::json!({
        "mcpServers": {"rtok": {"command": rtok_bin, "args": ["mcp"]}}
    });
    std::fs::write(home.join(".claude.json"), entry.to_string()).unwrap();
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = std::process::Command::new(rtok_bin)
        .args(["mcp", "ping", "claude", "--cli", "--json"])
        .env("HOME", &home)
        .env("RTOK_HOME", &home)
        .env("RTOK_BIN", rtok_bin)
        .env("PATH", path)
        .output()
        .expect("spawn rtok");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stdout {stdout}\nstderr {stderr}");
    assert!(stdout.contains("\"ok\":true"), "{stdout}");
    assert!(stdout.contains("MCP Claude Code жив"), "{stdout}");
    assert!(stdout.contains("\"mode\":\"chat\""), "{stdout}");
    let _ = std::fs::remove_dir_all(home);
}
