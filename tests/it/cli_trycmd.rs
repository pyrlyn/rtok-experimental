// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! trycmd CLI snapshots: full command-output fixtures in `tests/trycmd/*.toml`
//! and literate `--help` cases in `tests/trycmd/*.trycmd`.
//! Cases are hermetic by construction: `--help` / `--version` never load `Config`,
//! `bench --dry-run` reads only the `--config` fixture plus `bench/tasks.toml`, and
//! `config show` reads the same `--config` fixture with a cleared env (`[env]
//! inherit = false`), so ambient `RTOK_*` never leaks into the layering. An agent
//! session's own env and ambient logging ([`SNAPSHOT_ENV`]) are stripped from every case.
//! Insta stays out of here: it covers structured renderings
//! (`tests/compress_snapshot.rs`), trycmd covers the binary's stdout.

use clap::{Command, CommandFactory};
use rtok::cli::Cli;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

/// What an rtok agent session puts into its own shell: `RTOK_AGENT_ID` arrives through
/// `CLAUDE_ENV_FILE` (T283). Inherited by the binary under test, it turns the "not inside an
/// agent session" cases into the caller's own inbox, and a `hook` case could append to the
/// live session's env file (T406). `CLAUDE_CODE_SESSION_ID` names the caller too (T473).
// Ambient logging adds timestamps to snapshots; dedicated logging tests set it explicitly.
const SNAPSHOT_ENV: [&str; 5] = [
    "RTOK_AGENT_ID",
    "RTOK_CONFIG",
    "CLAUDE_ENV_FILE",
    "CLAUDE_CODE_SESSION_ID",
    "RUST_LOG",
];

// Both subprocess paths must run the same fixture entry point.
fn cli_process() -> std::process::Command {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    child.args(["it::cli_trycmd::cli", "--exact", "--nocapture"]);
    child
}

#[test]
fn cli_snapshots_ignore_ambient_rust_log() {
    let status = cli_process()
        .env("RUST_LOG", "warn")
        .status()
        .expect("run snapshots with ambient logging");
    assert!(status.success(), "ambient RUST_LOG leaked into snapshots");
}

#[test]
fn cli() {
    // trycmd 1.2.1 cannot remove a var for every case, its `TestCases::env` default would
    // override a case's own `[env.add]`, and `unsafe_code = "forbid"` rules out `remove_var`:
    // rerun this test in a child of the test binary without ambient session and logging env instead.
    if SNAPSHOT_ENV.iter().any(|k| std::env::var_os(k).is_some()) {
        let mut child = cli_process();
        for key in SNAPSHOT_ENV {
            child.env_remove(key);
        }
        let status = child
            .status()
            .expect("rerun `cli` without ambient session and logging env");
        assert!(
            status.success(),
            "trycmd cases failed without ambient session and logging env: {status}"
        );
        return;
    }
    let cases = trycmd::TestCases::new();
    cases
        .case("tests/trycmd/*.toml")
        .case("tests/trycmd/*.trycmd");
    // POSIX-only by design: `/bin/echo`, `SHELL=/bin/sh` and `cat` do not exist on Windows (T83.7).
    if cfg!(windows) {
        cases
            .skip("tests/trycmd/run.toml")
            .skip("tests/trycmd/expand.trycmd")
            // snapbox redacts `EXE_SUFFIX` (`.exe`) to `[EXE]` on Windows, which rewrites
            // `tool.execute.after` in `filter --stdin`'s help; the scripts are
            // platform-independent and stay covered on Linux and macOS.
            .skip("tests/trycmd/completions-clink.toml")
            .skip("tests/trycmd/completions-fish.toml")
            .skip("tests/trycmd/completions-powershell.toml")
            .skip("tests/trycmd/completions-zsh.toml")
            // `--list` prints per-user paths with the platform separator.
            .skip("tests/trycmd/completions-list.toml");
    }
}

/// trycmd 1.2.1's `Env` only recognizes `inherit`, `add` and `remove` — and has no
/// `deny_unknown_fields`, so a bare key sitting directly under `[env]` (e.g.
/// `RTOK_HOME = "…"` instead of under `[env.add]`) is silently dropped instead of reaching
/// the process env. A case that meant to isolate `HOME`/`RTOK_HOME` then runs with neither,
/// and `Config::home_dir` used to write `./.rtok/` into whatever the cwd happened to be
/// (T184; found while closing T169).
#[test]
fn trycmd_env_blocks_only_use_known_keys() {
    let known: HashSet<&str> = ["inherit", "add", "remove"].into_iter().collect();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/trycmd");
    for ent in fs::read_dir(&root).unwrap() {
        let path = ent.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let doc: toml_edit::DocumentMut = text
            .parse()
            .unwrap_or_else(|e| panic!("{}: invalid TOML: {e}", path.display()));
        let Some(env) = doc.get("env").and_then(|e| e.as_table_like()) else {
            continue;
        };
        for (key, _) in env.iter() {
            assert!(
                known.contains(key),
                "{}: [env] has key `{key}`, which trycmd's Env struct does not know — it is \
                 silently dropped instead of reaching the process. Move it under [env.add] \
                 (or [env.remove]).",
                path.display()
            );
        }
    }
}

/// Every visible clap command (and every `rtok …` cell in README.md's command
/// table) must appear in a trycmd case, so a new command cannot land without a
/// golden (T60.2).
#[test]
fn every_command_has_a_trycmd_case() {
    let covered = covered_commands();
    let mut commands = Vec::new();
    walk(&Cli::command(), &mut Vec::new(), &mut commands);
    assert!(!commands.is_empty(), "the walk found no commands");
    for cmd in &commands {
        assert!(
            covered.contains(cmd.as_str()),
            "command `{cmd}` has no trycmd case under tests/trycmd/"
        );
    }
    let root = Cli::command();
    for cell in readme_commands() {
        let Some(cmd) = match_command(&root, &cell) else {
            panic!("README command `rtok {cell}` is not a clap command");
        };
        assert!(
            covered.contains(cmd.as_str()),
            "README command `rtok {cell}` ({cmd}) has no trycmd case under tests/trycmd/"
        );
    }
}

fn walk(cmd: &Command, path: &mut Vec<String>, out: &mut Vec<String>) {
    for sub in cmd.get_subcommands() {
        if sub.get_name() == "help" || sub.is_hide_set() {
            continue;
        }
        path.push(sub.get_name().to_string());
        out.push(path.join(" "));
        walk(sub, path, out);
        path.pop();
    }
}

fn covered_commands() -> HashSet<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/trycmd");
    let clap = Cli::command();
    let mut out = HashSet::new();
    for ent in fs::read_dir(&root).unwrap() {
        let path = ent.unwrap().path();
        let ext = path.extension().and_then(OsStr::to_str).unwrap_or("");
        let text = match ext {
            "toml" | "trycmd" => fs::read_to_string(&path).unwrap(),
            _ => continue,
        };
        for args in case_args(&text, ext) {
            if let Some(cmd) = match_command(&clap, &args) {
                out.insert(cmd);
            }
        }
    }
    out
}

fn case_args(text: &str, ext: &str) -> Vec<String> {
    let mut cases = Vec::new();
    if ext == "toml" {
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("args = \"") {
                if let Some(args) = rest.strip_suffix('"') {
                    cases.push(args.to_string());
                }
            } else if let Some(rest) = line.strip_prefix("args = [") {
                cases.push(rest.trim_end_matches(']').replace('"', ""));
            }
        }
    } else {
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("$ rtok ") {
                cases.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("$ rtok") {
                cases.push(rest.trim().to_string());
            }
        }
    }
    cases
}

fn match_command(root: &Command, args: &str) -> Option<String> {
    let mut cur = root;
    let mut path = Vec::new();
    let parts: Vec<&str> = args.split_whitespace().collect();
    let mut i = 0;
    while i < parts.len() {
        let part = parts[i];
        if part == "--" {
            break;
        }
        if part.starts_with('-') {
            if !part.contains('=') && i + 1 < parts.len() && !parts[i + 1].starts_with('-') {
                i += 1;
            }
            i += 1;
            continue;
        }
        match cur.find_subcommand(part) {
            Some(next) => {
                path.push(next.get_name().to_string());
                cur = next;
                i += 1;
            }
            None => break,
        }
    }
    if path.is_empty() {
        None
    } else {
        Some(path.join(" "))
    }
}

fn readme_commands() -> Vec<String> {
    let text = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md")).unwrap();
    let mut cmds = Vec::new();
    let mut in_table = false;
    for line in text.lines() {
        if line.starts_with("## Commands") {
            in_table = true;
            continue;
        }
        if in_table && line.starts_with("## ") {
            break;
        }
        if !in_table || !line.starts_with("| `rtok ") {
            continue;
        }
        let Some(rest) = line.strip_prefix("| `rtok ") else {
            continue;
        };
        let cell = rest.split('`').next().unwrap_or("");
        if !cell.is_empty() {
            cmds.push(cell.to_string());
        }
    }
    cmds
}
