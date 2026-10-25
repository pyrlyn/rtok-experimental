// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok mcp ping` (T275.1): prove a host's rtok MCP server answers.
//!
//! Headless hosts run the agent's own non-interactive command. Every other variant
//! spawns the `command` / `args` / `env` written in that host's config and calls `ping`.

use rtok_sys::proc::ProgressRunner;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::agents::{self, Agent, Kind};
use crate::config::Config;

/// The MCP tool's reply. The display name is what `agents list` prints.
pub(super) fn ping_text(agent: &str) -> String {
    format!("MCP {agent} жив")
}

/// The one prompt a headless agent is given, and the line a desktop user pastes.
pub fn prompt_for(display: &str) -> String {
    format!(
        "Call the rtok MCP tool \"ping\" with agent=\"{display}\" and reply with its result only, nothing else."
    )
}

/// Where a host writes rtok's MCP server object. `keys` walks to that object.
/// `argv` means `command` is the full argv (`["rtok", "mcp"]`), not a string plus `args`.
struct Shape {
    host: &'static str,
    keys: &'static [&'static str],
    toml: bool,
    argv: bool,
}

const SHAPES: &[Shape] = &[
    Shape {
        host: "claude",
        keys: &["mcpServers", "rtok"],
        toml: false,
        argv: false,
    },
    Shape {
        host: "cursor",
        keys: &["mcpServers", "rtok"],
        toml: false,
        argv: false,
    },
    Shape {
        host: "codex",
        keys: &["mcp_servers", "rtok"],
        toml: true,
        argv: false,
    },
    Shape {
        host: "opencode",
        keys: &["mcp", "rtok"],
        toml: false,
        argv: true,
    },
    Shape {
        host: "kilo",
        keys: &["mcp", "rtok"],
        toml: false,
        argv: true,
    },
    Shape {
        host: "omp",
        keys: &["mcpServers", "rtok"],
        toml: false,
        argv: false,
    },
    Shape {
        host: "zcode",
        keys: &["mcp", "servers", "rtok"],
        toml: false,
        argv: false,
    },
    Shape {
        host: "kimi",
        keys: &["mcpServers", "rtok"],
        toml: false,
        argv: false,
    },
    Shape {
        host: "grok",
        keys: &["mcp_servers", "rtok"],
        toml: true,
        argv: false,
    },
    Shape {
        host: "vscode",
        keys: &["servers", "rtok"],
        toml: false,
        argv: false,
    },
    Shape {
        host: "copilot",
        keys: &["mcpServers", "rtok"],
        toml: false,
        argv: false,
    },
    Shape {
        host: "windsurf",
        keys: &["mcpServers", "rtok"],
        toml: false,
        argv: false,
    },
    Shape {
        host: "zed",
        keys: &["context_servers", "rtok"],
        toml: false,
        argv: false,
    },
    Shape {
        host: "cline",
        keys: &["mcpServers", "rtok"],
        toml: false,
        argv: false,
    },
    Shape {
        host: "gemini",
        keys: &["mcpServers", "rtok"],
        toml: false,
        argv: false,
    },
    Shape {
        host: "codewhale",
        keys: &["mcpServers", "rtok"],
        toml: false,
        argv: false,
    },
    Shape {
        host: "mimo",
        keys: &["mcp", "rtok"],
        toml: false,
        argv: true,
    },
    Shape {
        host: "devin",
        keys: &["mcpServers", "rtok"],
        toml: false,
        argv: false,
    },
];

/// `{prompt}` is replaced with [`prompt_for`]. Flags checked against each vendor's docs
/// on 2026-09-27; the host README records the same line.
struct Headless {
    host: &'static str,
    bin: &'static str,
    argv: &'static [&'static str],
}

const HEADLESS: &[Headless] = &[
    // https://code.claude.com/docs/en/cli-reference — `claude -p "query"` then exit
    Headless {
        host: "claude",
        bin: "claude",
        argv: &["-p", "{prompt}"],
    },
    // https://cursor.com/docs/cli/using — `-p` / `--print`; `agent` is the same binary
    Headless {
        host: "cursor",
        bin: "cursor-agent",
        argv: &["-p", "{prompt}"],
    },
    // https://developers.openai.com/codex/cli/reference — `codex exec` non-interactive
    Headless {
        host: "codex",
        bin: "codex",
        argv: &["exec", "{prompt}"],
    },
    // https://docs.github.com/en/copilot/concepts/agents/copilot-cli/about-copilot-cli — `-p` / `--prompt`
    Headless {
        host: "copilot",
        bin: "copilot",
        argv: &["-p", "{prompt}", "--allow-all-tools"],
    },
    // https://geminicli.com/docs/cli/cli-reference — `gemini -p "query"`
    Headless {
        host: "gemini",
        bin: "gemini",
        argv: &["-p", "{prompt}"],
    },
];

/// Command line taken from a host config. Never a path this binary invented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// What one variant's check concluded.
#[derive(Debug)]
struct Report {
    agent: String,
    variant: String,
    display: String,
    mode: &'static str,
    reply: String,
    ok: bool,
    reason: Option<String>,
}

pub struct PingOpts<'a> {
    pub agent: Option<&'a str>,
    pub cli: bool,
    pub desktop: bool,
    pub timeout: Duration,
    pub json: bool,
}

/// Run the check. `Ok(0)` only when every selected variant succeeded.
pub fn run(cfg: &Config, opts: PingOpts<'_>) -> Result<i32> {
    let named = named_hosts(opts.agent)?;
    let targets = select(cfg, named.as_deref(), opts.cli, opts.desktop);
    if targets.is_empty() {
        let msg = if named.is_some() {
            "no variant selected; pass --cli or --desktop for a variant this host has"
        } else {
            "no host has rtok MCP installed"
        };
        eprintln!("{msg}");
        return Ok(1);
    }
    let reports: Vec<Report> = targets
        .iter()
        .map(|t| check(cfg, t, opts.timeout))
        .collect();
    if opts.json {
        for r in &reports {
            println!("{}", serde_json::to_string(&row_json(r))?);
        }
    } else {
        print!("{}", render_text(&reports));
    }
    Ok(if reports.iter().all(|r| r.ok) { 0 } else { 1 })
}

fn named_hosts(agent: Option<&str>) -> Result<Option<Vec<String>>> {
    let Some(agent) = agent else {
        return Ok(None);
    };
    let names: Vec<String> = agent
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        bail!("no host given");
    }
    for name in &names {
        if agents::host(name).is_none() {
            bail!("unknown host: {name}");
        }
    }
    Ok(Some(names))
}

struct Target {
    agent: &'static dyn Agent,
    kind: Kind,
    display: &'static str,
}

fn select(cfg: &Config, named: Option<&[String]>, cli: bool, desktop: bool) -> Vec<Target> {
    let ids: Vec<&str> = match named {
        Some(names) => names.iter().map(String::as_str).collect(),
        None => agents::HOSTS.to_vec(),
    };
    let mut out = Vec::new();
    for id in ids {
        let Some(agent) = agents::host(id) else {
            continue;
        };
        for v in agent.variants() {
            if !agents::wants(v.kind, cli, desktop, false) {
                continue;
            }
            if named.is_none()
                && (!agents::present(cfg, id, v) || !agent.installed(cfg, v.kind).contains(&"mcp"))
            {
                continue;
            }
            out.push(Target {
                agent,
                kind: v.kind,
                display: v.name,
            });
        }
    }
    out
}

fn check(cfg: &Config, target: &Target, timeout: Duration) -> Report {
    let expect = ping_text(target.display);
    // A plugin can serve MCP with no config-file entry (Claude Code). Headless still runs.
    if target.kind == Kind::Cli
        && let Some(spec) = HEADLESS.iter().find(|h| h.host == target.agent.id())
    {
        return match run_headless(spec, target, &expect, timeout) {
            Ok(reply) => pass(target, "chat", reply),
            Err(reason) => fail(target, "chat", reason),
        };
    }
    let files = target.agent.files(cfg, target.kind);
    let Some(launch) = entry_in_files(&files, target.agent.id()) else {
        return fail(
            target,
            "server-only",
            format!(
                "no rtok MCP entry in the host config; run `rtok agents install {}`",
                target.agent.id()
            ),
        );
    };
    match probe(&launch, target.display, timeout) {
        Ok(reply) if reply.trim() == expect => pass(target, "server-only", expect),
        Ok(reply) => fail(
            target,
            "server-only",
            format!("reply was {reply:?}, expected {expect:?}"),
        ),
        Err(reason) => fail(target, "server-only", reason),
    }
}

fn pass(target: &Target, mode: &'static str, reply: String) -> Report {
    Report {
        agent: target.agent.id().to_string(),
        variant: target.kind.as_str().to_string(),
        display: target.display.to_string(),
        mode,
        reply,
        ok: true,
        reason: None,
    }
}

fn fail(target: &Target, mode: &'static str, reason: String) -> Report {
    Report {
        agent: target.agent.id().to_string(),
        variant: target.kind.as_str().to_string(),
        display: target.display.to_string(),
        mode,
        reply: String::new(),
        ok: false,
        reason: Some(reason),
    }
}

fn run_headless(
    spec: &Headless,
    target: &Target,
    expect: &str,
    timeout: Duration,
) -> Result<String, String> {
    let prompt = prompt_for(target.display);
    let args: Vec<String> = spec
        .argv
        .iter()
        .map(|a| {
            if *a == "{prompt}" {
                prompt.clone()
            } else {
                (*a).to_string()
            }
        })
        .collect();
    // `cursor-agent` and `agent` are the same CLI; use whichever is on PATH.
    let bins: &[&str] = if spec.bin == "cursor-agent" {
        &["cursor-agent", "agent"]
    } else {
        std::slice::from_ref(&spec.bin)
    };
    let mut missing = String::new();
    for (i, bin) in bins.iter().enumerate() {
        match capture(bin, &args, timeout) {
            Err(reason) if i + 1 < bins.len() => missing = reason,
            Err(reason) => return Err(reason),
            Ok((_, true)) => return Err(format!("timed out after {}s", timeout.as_secs())),
            Ok((stdout, false)) => {
                let reply = stdout.trim().to_string();
                return if reply == expect {
                    Ok(reply)
                } else {
                    Err(format!("reply was {reply:?}, expected {expect:?}"))
                };
            }
        }
    }
    Err(missing)
}

fn render_text(reports: &[Report]) -> String {
    let mut out = String::new();
    let many = reports.len() > 1;
    for r in reports {
        if r.mode == "server-only" {
            out.push_str(&format!(
                "Paste into {}:\n{}\n\n",
                r.display,
                prompt_for(&r.display)
            ));
            if r.ok {
                out.push_str(&format!(
                    "Checked the configured server, not whether {} loaded it: {}\n",
                    r.display, r.reply
                ));
                continue;
            }
        }
        if r.ok {
            if many || r.mode == "server-only" {
                out.push_str(&format!("{}: {}\n", r.display, r.reply));
            } else {
                out.push_str(&r.reply);
                out.push('\n');
            }
        } else {
            out.push_str(&format!(
                "{}: {}\n",
                r.display,
                r.reason.as_deref().unwrap_or("failed")
            ));
        }
    }
    out
}

fn row_json(r: &Report) -> Value {
    json!({
        "agent": r.agent,
        "variant": r.variant,
        "mode": r.mode,
        "reply": r.reply,
        "ok": r.ok,
        "reason": r.reason.clone(),
    })
}

fn shape_for(host: &str) -> Option<&'static Shape> {
    SHAPES.iter().find(|s| s.host == host)
}

/// First rtok entry among `paths`. The reader is the only source of `command` / `args`.
pub fn launch_from_read(
    read: impl Fn(&str) -> Option<String>,
    paths: &[&str],
    host: &str,
) -> Option<Launch> {
    let shape = shape_for(host)?;
    for path in paths {
        let Some(body) = read(path) else {
            continue;
        };
        if let Some(launch) = launch_from_body_shape(&body, shape) {
            return Some(launch);
        }
    }
    None
}

fn entry_in_files(paths: &[PathBuf], host: &str) -> Option<Launch> {
    let keys: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    launch_from_read(|path| std::fs::read_to_string(path).ok(), &key_refs, host)
}

/// Parse one config body. `None` when this host's rtok object is absent.
pub fn launch_from_body(body: &str, host: &str) -> Option<Launch> {
    launch_from_body_shape(body, shape_for(host)?)
}

fn launch_from_body_shape(body: &str, shape: &Shape) -> Option<Launch> {
    if shape.toml {
        let doc = body.parse::<toml_edit::DocumentMut>().ok()?;
        let table = toml_at(&doc, shape.keys)?;
        let command = table.get("command")?.as_str()?.to_string();
        if command.is_empty() {
            return None;
        }
        return Some(Launch {
            command,
            args: toml_args(table),
            env: toml_env(table),
        });
    }
    let value = crate::agents::jsonc::parse(body).ok()?;
    let obj = json_at(&value, shape.keys)?.as_object()?;
    if shape.argv {
        let mut argv: Vec<String> = obj
            .get("command")?
            .as_array()?
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        if argv.is_empty() {
            return None;
        }
        let command = argv.remove(0);
        return Some(Launch {
            command,
            args: argv,
            env: json_env(obj),
        });
    }
    let command = obj.get("command")?.as_str()?.to_string();
    if command.is_empty() {
        return None;
    }
    let args = obj
        .get("args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    Some(Launch {
        command,
        args,
        env: json_env(obj),
    })
}

fn json_at<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    let mut cur = value;
    for key in keys {
        cur = cur.get(*key)?;
    }
    Some(cur)
}

fn json_env(obj: &serde_json::Map<String, Value>) -> Vec<(String, String)> {
    obj.get("env")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .collect()
}

fn toml_at<'a>(doc: &'a toml_edit::DocumentMut, keys: &[&str]) -> Option<&'a toml_edit::Table> {
    let mut cur = doc.as_table();
    for (i, key) in keys.iter().enumerate() {
        let item = cur.get(key)?;
        if i + 1 == keys.len() {
            return item.as_table();
        }
        cur = item.as_table()?;
    }
    None
}

fn toml_args(table: &toml_edit::Table) -> Vec<String> {
    table
        .get("args")
        .and_then(toml_edit::Item::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

fn toml_env(table: &toml_edit::Table) -> Vec<(String, String)> {
    let Some(item) = table.get("env") else {
        return Vec::new();
    };
    if let Some(inline) = item.as_inline_table() {
        return inline
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.to_string(), s.to_string())))
            .collect();
    }
    item.as_table()
        .map(|t| {
            t.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.to_string(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Spawn the configured server, `initialize`, `tools/list`, `tools/call ping`.
pub fn probe(launch: &Launch, agent: &str, timeout: Duration) -> Result<String, String> {
    let env: BTreeMap<String, String> = launch.env.iter().cloned().collect();
    let call = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {"name": "ping", "arguments": {"agent": agent}},
    });
    let values = crate::doctor::mcp_roundtrip(
        &launch.command,
        &launch.args,
        &env,
        Some(&call.to_string()),
        timeout,
    )?;
    let listed = values.iter().any(|v| {
        v.pointer("/result/tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| {
                tools
                    .iter()
                    .any(|t| t.get("name").and_then(Value::as_str) == Some("ping"))
            })
    });
    if !listed {
        return Err("the ping tool is missing".into());
    }
    let reply_v = values
        .iter()
        .find(|v| {
            v.get("id")
                .is_some_and(|id| id.as_i64() == Some(3) || id.as_str() == Some("3"))
        })
        .ok_or_else(|| format!("timed out after {}s", timeout.as_secs()))?;
    if reply_v.pointer("/result/isError") == Some(&json!(true)) {
        let text = reply_v
            .pointer("/result/content/0/text")
            .and_then(Value::as_str)
            .unwrap_or("tool error");
        return Err(text.to_string());
    }
    Ok(reply_v
        .pointer("/result/content/0/text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string())
}

/// `bin`'s stdout, and whether it was killed at `timeout`. Windows npm shims go through
/// `cmd.exe` (T139).
fn capture(bin: &str, args: &[String], timeout: Duration) -> Result<(String, bool), String> {
    let run = ProgressRunner::new(&format!("asking {bin}…"), bin, args)
        .windows_shim()
        .timeout(timeout)
        .run();
    match run {
        Ok(out) => Ok((String::from_utf8_lossy(&out.stdout).into_owned(), false)),
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok((String::new(), true)),
        Err(e) => Err(format!("failed to start: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Vfs;

    fn fixture(shape: &Shape, cmd: &str) -> String {
        if shape.toml {
            return format!(
                "[mcp_servers.other]\ncommand = \"decoy\"\nargs = [\"no\"]\n\n[mcp_servers.rtok]\ncommand = \"{cmd}\"\nargs = [\"mcp\", \"{host}\"]\nenv = {{ PING_HOST = \"{host}\" }}\n",
                host = shape.host,
            );
        }
        let entry = if shape.argv {
            json!({"command": [cmd, "mcp", shape.host], "env": {"PING_HOST": shape.host}})
        } else {
            json!({"command": cmd, "args": ["mcp", shape.host], "env": {"PING_HOST": shape.host}})
        };
        let decoy = if shape.argv {
            json!({"command": ["decoy", "no"]})
        } else {
            json!({"command": "decoy", "args": ["no"]})
        };
        let mut node = json!({"rtok": entry, "other": decoy});
        for key in shape.keys.iter().rev().skip(1) {
            node = json!({*key: node});
        }
        node.to_string()
    }

    #[test]
    fn server_only_reads_command_and_args_from_each_host_config() {
        let mut vfs = Vfs::new();
        for shape in SHAPES {
            let cmd = format!("/opt/ping/{}", shape.host);
            let path = format!("/cfg/{}", shape.host);
            let miss = format!("/cfg/{}-miss", shape.host);
            vfs.write(&path, fixture(shape, &cmd));
            vfs.write(&miss, "{\"unrelated\":true}");
            let keys = [miss.as_str(), path.as_str()];
            let got = launch_from_read(|p| vfs.read_str(p).map(str::to_string), &keys, shape.host)
                .unwrap_or_else(|| panic!("{}: no launch", shape.host));
            assert_eq!(got.command, cmd, "{}", shape.host);
            assert_eq!(
                got.args,
                vec!["mcp".to_string(), shape.host.to_string()],
                "{}",
                shape.host
            );
            assert_eq!(
                got.env,
                vec![("PING_HOST".to_string(), shape.host.to_string())],
                "{}",
                shape.host
            );
            assert_ne!(got.command, "rtok");
            assert_ne!(got.command, "decoy");
        }
    }
}
