// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! A real `rtok mcp` process driven over stdin/stdout (T285, T287).

use std::path::Path;
use std::process::Command;

/// One `rtok mcp` session: `initialize`, then `hooks_fire` (the session's hooks registering
/// its agent: a cwd link only trusts a row seen since the process started), then `calls` as
/// `tools/call`s; `(isError, text)` of each answer.
pub fn session(
    home: &Path,
    cwd: &Path,
    hooks_fire: impl FnOnce(),
    calls: &[(&str, &str)],
) -> Vec<(bool, String)> {
    session_as("claude", home, cwd, hooks_fire, calls)
}

/// [`session`] as `rtok mcp --host <host>`.
pub fn session_as(
    host: &str,
    home: &Path,
    cwd: &Path,
    hooks_fire: impl FnOnce(),
    calls: &[(&str, &str)],
) -> Vec<(bool, String)> {
    use std::io::{BufRead as _, Write as _};
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .args(["mcp", "--host", host])
        .current_dir(cwd)
        .env("HOME", home)
        .env_remove("RTOK_AGENT_ID")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("rtok spawns");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
    let init = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{}}}"#;
    writeln!(stdin, "{init}").unwrap();
    // The answer proves the process is up, so its start time is behind us.
    stdout.read_line(&mut String::new()).unwrap();
    hooks_fire();
    for (i, (name, args)) in calls.iter().enumerate() {
        let id = i + 1;
        let call = format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{name}","arguments":{args}}}}}"#
        );
        writeln!(stdin, "{call}").unwrap();
    }
    drop(stdin);
    let answers: Vec<serde_json::Value> = stdout
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str(&l).ok())
        .collect();
    child.wait().unwrap();
    (1..=calls.len())
        .map(|id| {
            let v = answers.iter().find(|a| a["id"] == id).expect("an answer");
            (
                v["result"]["isError"] == true,
                v["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect()
}
