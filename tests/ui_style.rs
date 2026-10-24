// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `[ui]` emoji and colour against the real binary. Every run here is on pipes, which is how
//! hooks, agents and CI see rtok: plain bytes unless colour is forced, and never an emoji.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const ESC: char = '\u{1b}';
use operation_icons::OPERATION_ICONS;
use rtok::ui::style::{Kind, OWN_ICONS};

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-ui-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // `config validate` checks a file; it does not create one.
    fs::write(dir.join("config.toml"), "").unwrap();
    dir
}

/// `rtok <args>` with the colour environment cleared, then `env` on top.
fn rtok(home: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rtok"));
    cmd.args(args).env("RTOK_HOME", home).env("HOME", home);
    for var in [
        "NO_COLOR",
        "FORCE_COLOR",
        "CLICOLOR",
        "CLICOLOR_FORCE",
        "TERM",
        "RTOK_UI_EMOJI",
        "RTOK_UI_COLOR",
        "RTOK_CONFIG",
        "RUST_LOG",
    ] {
        cmd.env_remove(var);
    }
    cmd.envs(env.iter().copied()).output().unwrap()
}

fn stdout(out: &Output) -> String {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn no_emoji(text: &str) {
    let tones = [Kind::Success, Kind::Info, Kind::Warn, Kind::Error].map(Kind::emoji);
    let ops = OPERATION_ICONS
        .iter()
        .chain(OWN_ICONS)
        .map(|(_, icon)| *icon);
    for e in tones.into_iter().chain(ops) {
        assert!(!text.contains(e), "no emoji off a terminal: {text:?}");
    }
}

#[test]
fn a_pipe_gets_the_bare_line() {
    let home = home("pipe");
    let out = stdout(&rtok(&home, &["config", "validate"], &[]));
    assert_eq!(out, format!("ok {}\n", home.join("config.toml").display()));
    // NO_COLOR on a pipe changes nothing: it was already plain.
    assert_eq!(
        stdout(&rtok(&home, &["config", "validate"], &[("NO_COLOR", "1")])),
        out
    );
}

#[test]
fn an_error_on_a_pipe_keeps_std_s_bytes() {
    let home = home("error");
    let out = rtok(&home, &["config", "get", "no.such.key"], &[]);
    assert!(!out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "Error: unknown key: no.such.key\n"
    );
}

#[test]
fn clicolor_force_paints_a_pipe_but_adds_no_emoji() {
    let home = home("force");
    let out = stdout(&rtok(
        &home,
        &["config", "validate"],
        &[("CLICOLOR_FORCE", "1")],
    ));
    assert!(out.starts_with(&format!("{ESC}[32mok ")), "{out:?}");
    no_emoji(&out);
}

#[test]
fn the_color_key_wins_over_clicolor_force() {
    let home = home("key");
    let force = [("CLICOLOR_FORCE", "1"), ("RTOK_UI_COLOR", "false")];
    let out = stdout(&rtok(&home, &["config", "validate"], &force));
    assert!(!out.contains(ESC), "RTOK_UI_COLOR=false: {out:?}");
    // The same through the file: `[ui] color = false`.
    let cfg = home.join("config.toml");
    fs::write(&cfg, "[ui]\ncolor = false\n").unwrap();
    let out = stdout(&rtok(&home, &["config", "validate"], &force[..1]));
    // `color = false` is a pin, so validate names it, then the same plain ok line.
    assert!(!out.contains(ESC), "file color=false: {out:?}");
    assert!(out.contains(&format!("ok {}", cfg.display())), "{out}");
    assert!(out.contains("ui.color = false (default true)"), "{out}");
    no_emoji(&out);
}

/// T436.2: every wait that gained a spinner stays byte-clean on pipes. indicatif draws nothing
/// when stderr is not a terminal; this pins that for each command, so a later swap of the
/// helper for a hand-rolled one cannot start printing control bytes into CI logs.
#[test]
fn a_wait_draws_nothing_on_a_pipe() {
    let home = home("waits");
    let claude_md = home.join("CLAUDE.md");
    let claude_md = claude_md.to_str().unwrap();
    // No `PATH` for the host run: a dry run must not reach a real agent's CLI.
    type Run<'a> = (&'a [&'a str], &'a [(&'a str, &'a str)]);
    let runs: [Run; 5] = [
        (&["info"], &[]),
        (&["report"], &[]),
        (&["otel", "flush"], &[]),
        (&["memory", "sync", "--dry-run", "--file", claude_md], &[]),
        (
            &["agents", "install", "claude", "--dry-run"],
            &[("PATH", "/nonexistent")],
        ),
    ];
    for (args, env) in runs {
        let out = rtok(&home, args, env);
        let (so, se) = (
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        for text in [&so, &se] {
            assert!(!text.contains(ESC), "{args:?}: control bytes: {text:?}");
            no_emoji(text);
            for msg in [
                "migrating the store",
                "syncing memory",
                "building the report",
                "flushing telemetry",
                "reading status",
            ] {
                assert!(!text.contains(msg), "{args:?}: spinner text: {text:?}");
            }
        }
    }
}

/// T276.4: `rtok run` shows `running <command>…` while the command works, but only on a
/// terminal; on a pipe (a host's shell tool) its bytes are the command's output and nothing else.
#[test]
fn rtok_run_draws_no_spinner_on_a_pipe() {
    let home = home("run");
    let out = rtok(
        &home,
        &["run", "--", "sh", "-c", "printf hi; sleep 0.3"],
        &[],
    );
    assert_eq!(stdout(&out), "hi\n");
    assert!(out.stderr.is_empty(), "{:?}", out.stderr);
}
