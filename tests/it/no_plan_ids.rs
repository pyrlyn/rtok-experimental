// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T409: internal plan ids (`T70.2`, `D21`, `I-101`, `P9.3`, `§16`) never reach the terminal.
//!
//! They mean nothing to a user and leak the plan files. Every surface generated from the clap
//! tree or embedded as text is rendered here: `--help` of the root and of every subcommand
//! (hidden ones too), the man pages, every completion script, and the config template that
//! `rtok config init` writes. Ids belong in plain `//` comments, which clap ignores.

use clap::{Command, CommandFactory, ValueEnum};
use regex::Regex;
use rtok::cli::Cli;
use rtok::completions::{Shell, generate};
use rtok::config::DEFAULT_TOML;

/// Task, decision, idea, plan-phase and research-section ids. The word boundaries keep out
/// `UTF-8`, `x86_64`, `ID3` and a `T` or `D` inside a longer word; `P` needs a dotted number
/// (`P9.3`) because a bare `P1` is also a legitimate priority label.
fn plan_id() -> Regex {
    Regex::new(r"\b(?:T\d+(?:\.\d+)*|D\d+|I-\d+|P\d+(?:\.\d+)+)|§\s?\d+").unwrap()
}

/// Version strings and the like that match `plan_id` but are not plan ids. Empty today.
const ALLOW: &[&str] = &[];

fn offenders(surface: &str, text: &str, out: &mut Vec<String>) {
    let re = plan_id();
    for (n, line) in text.lines().enumerate() {
        for m in re.find_iter(line) {
            if !ALLOW.contains(&m.as_str()) {
                out.push(format!("{surface}:{}: `{}` in {line:?}", n + 1, m.as_str()));
            }
        }
    }
}

fn walk(cmd: &Command, path: &str, out: &mut Vec<String>) {
    offenders(
        &format!("--help of `{path}`"),
        &cmd.clone().render_long_help().to_string(),
        out,
    );
    for sub in cmd.get_subcommands() {
        walk(sub, &format!("{path} {}", sub.get_name()), out);
    }
}

fn assert_clean(found: Vec<String>) {
    assert!(
        found.is_empty(),
        "plan ids in user-visible output ({}); move each into a `//` comment:\n{}",
        found.len(),
        found.join("\n")
    );
}

#[test]
fn help_of_every_command_has_no_plan_ids() {
    let mut found = Vec::new();
    walk(&Cli::command(), "rtok", &mut found);
    assert_clean(found);
}

#[test]
fn man_pages_have_no_plan_ids() {
    let dir = rtok::testutil::tmp_dir("no-plan-ids-man");
    let pages = rtok::man::write_all(Cli::command(), &dir).unwrap();
    assert!(!pages.is_empty());
    let mut found = Vec::new();
    for page in &pages {
        let text = std::fs::read_to_string(page).unwrap();
        offenders(&page.display().to_string(), &text, &mut found);
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert_clean(found);
}

#[test]
fn completion_scripts_have_no_plan_ids() {
    let mut found = Vec::new();
    for shell in Shell::value_variants() {
        let mut out = Vec::new();
        generate(*shell, Cli::command(), &mut out);
        let text = String::from_utf8(out).unwrap();
        assert!(!text.is_empty());
        offenders(&format!("completions {shell:?}"), &text, &mut found);
    }
    assert_clean(found);
}

#[test]
fn config_template_has_no_plan_ids() {
    let mut found = Vec::new();
    offenders("config/default.toml", DEFAULT_TOML, &mut found);
    assert_clean(found);
}

#[test]
fn plan_id_pattern_hits_ids_and_skips_lookalikes() {
    let re = plan_id();
    for hit in [
        "T70.2",
        "(T15.1)",
        "D21",
        "I-101",
        "P9.3",
        "§16",
        "plan T12.4",
    ] {
        assert!(re.is_match(hit), "{hit} should match");
    }
    for miss in [
        "UTF-8", "x86_64", "ID3", "SHA256", "ANSI-X3", "TLS1.3", "P1", "UTC",
    ] {
        assert!(!re.is_match(miss), "{miss} should not match");
    }
}
