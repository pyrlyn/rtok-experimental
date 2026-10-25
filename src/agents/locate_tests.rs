// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use super::*;
use crate::agents::{claude::Claude, cursor::Cursor, home_dir};

/// A scratch dir under the test home: the host sandbox (T280) only sees paths beneath it.
fn scratch(name: &str) -> PathBuf {
    let dir = home_dir().join(format!("rtok-locate-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn program(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, "#!/bin/sh\necho 1.2.3\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

fn variant(agent: &dyn Agent, kind: Kind) -> &'static Variant {
    agent.variants().iter().find(|v| v.kind == kind).unwrap()
}

fn with_path(host: &str, path: &Path) -> Config {
    let mut cfg = Config::default();
    cfg.setup.paths.insert(host.into(), path.to_path_buf());
    cfg
}

#[test]
fn a_path_is_placed_on_the_variant_it_names() {
    let at = |agent: &dyn Agent, p: &str| variant_for(agent, Path::new(p)).unwrap().kind;
    assert_eq!(at(&Claude, "/opt/bin/claude"), Kind::Cli);
    assert_eq!(at(&Claude, "/Applications/Claude.app"), Kind::Desktop);
    assert_eq!(
        at(&Claude, "C:/x/AnthropicClaude/claude.exe"),
        Kind::Desktop
    );
    assert_eq!(at(&Claude, "/opt/bin/claude-wrapper"), Kind::Cli);
    assert_eq!(at(&Cursor, "/opt/bin/cursor-agent"), Kind::Cli);
    assert_eq!(at(&Cursor, "/usr/local/bin/cursor"), Kind::Desktop);
}

#[test]
fn the_user_path_comes_first_and_only_for_its_variant() {
    let dir = scratch("user");
    let bin = program(&dir, "claude");
    let cfg = with_path("claude", &bin);
    let cli_v = variant(&Claude, Kind::Cli);
    assert_eq!(locate(&cfg, "claude", cli_v), Some(bin.clone()));
    assert!(present(&cfg, "claude", cli_v));
    assert_eq!(cli(&cfg, "claude"), Some(bin.clone()));
    assert_eq!(
        cli_program(&cfg, "claude"),
        Some(bin.clone().into_os_string())
    );
    assert_eq!(
        programs(&cfg, "claude", cli_v)[0],
        bin.clone().into_os_string()
    );
    assert!(user_path(&cfg, "claude", variant(&Claude, Kind::Desktop)).is_none());
    assert!(absent(&cfg, &[&Claude], |_| true).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_user_desktop_app_is_found_and_offered_for_restart() {
    let dir = scratch("app");
    let app = dir.join("Claude.app");
    std::fs::create_dir_all(&app).unwrap();
    let cfg = with_path("claude", &app);
    let desk = variant(&Claude, Kind::Desktop);
    assert_eq!(locate(&cfg, "claude", desk), Some(app.clone()));
    assert_eq!(app_paths(&cfg, "claude", desk)[0], app);
    assert!(locate(&cfg, "claude", variant(&Claude, Kind::Cli)).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_user_path_that_is_gone_finds_nothing_and_the_error_names_the_key() {
    let cfg = with_path("claude", &home_dir().join("rtok-locate-gone/claude"));
    assert!(locate(&cfg, "claude", variant(&Claude, Kind::Cli)).is_none());
    let missing = absent(&cfg, &[&Claude, &Cursor], |_| true);
    assert_eq!(missing, ["claude", "cursor"]);
    let msg = not_found(&missing).to_string();
    assert!(msg.starts_with("claude, cursor is not installed"), "{msg}");
    assert!(
        msg.contains("`rtok config set setup.paths.claude <path>`"),
        "{msg}"
    );
    assert!(
        msg.contains("`rtok config set setup.paths.cursor <path>`"),
        "{msg}"
    );
}

#[test]
fn a_typed_path_is_checked_before_it_is_shown() {
    let dir = scratch("typed");
    let cfg = Config::default();
    let err = |raw: &str| resolve_input(&cfg, &Claude, raw).unwrap_err().to_string();
    assert!(err(&dir.join("nope").display().to_string()).ends_with("no such file"));
    std::fs::create_dir_all(dir.join("folder")).unwrap();
    assert!(err(&dir.join("folder").display().to_string()).contains("a folder"));
    #[cfg(unix)]
    {
        std::fs::write(dir.join("plain"), "x").unwrap();
        assert!(err(&dir.join("plain").display().to_string()).contains("not executable"));
    }
    let bin = program(&dir, "claude");
    let quoted = format!("  '{}'\n", bin.display());
    let got = resolve_input(&cfg, &Claude, &quoted).unwrap();
    assert_eq!(got.path, bin);
    assert_eq!(got.variant.kind, Kind::Cli);
    assert_eq!(got.config, Claude.files(&cfg, Kind::Cli));
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_terminal_dragged_path_loses_its_escapes() {
    assert_eq!(unquote("/a\\ b/claude\n"), "/a b/claude");
    assert_eq!(unquote("\"/a b/claude\""), "/a b/claude");
}

/// Scripted stdin for the questions; the transcript is what a terminal would have shown.
fn script(cfg: &Config, lines: &[String]) -> (Option<Resolved>, String) {
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let mut out = Vec::new();
    let got = ask_one(cfg, &Claude, &mut text.as_bytes(), &mut out).unwrap();
    (got, String::from_utf8(out).unwrap())
}

#[test]
fn an_empty_answer_or_eof_stops_without_a_path() {
    let cfg = Config::default();
    let (got, shown) = script(&cfg, &[String::new()]);
    assert!(got.is_none());
    assert!(shown.starts_with("claude was not found"), "{shown}");
    assert!(
        shown.contains("? Path to Claude Code or Claude Desktop"),
        "{shown}"
    );
    let mut out = Vec::new();
    assert!(
        ask_one(&cfg, &Claude, &mut "".as_bytes(), &mut out)
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_bad_path_and_a_no_ask_again_until_a_yes() {
    let dir = scratch("ask");
    let bin = program(&dir, "claude");
    let cfg = Config::default();
    let good = bin.display().to_string();
    let (got, shown) = script(
        &cfg,
        &[
            dir.join("nope").display().to_string(),
            good.clone(),
            "n".into(),
            good,
            "y".into(),
        ],
    );
    assert_eq!(got.unwrap().path, bin);
    assert!(shown.contains("no such file"), "{shown}");
    assert!(shown.contains("  agent   Claude Code (CLI)"), "{shown}");
    assert!(
        shown.contains(&format!("  path    {}", bin.display())),
        "{shown}"
    );
    assert!(shown.contains("  config  "), "{shown}");
    assert_eq!(
        shown.matches("? Is this correct? [y/N]").count(),
        2,
        "{shown}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// What `ask_hosts` saved: (key, path) pairs.
fn ask_hosts_with(cfg: &mut Config, answers: &str) -> (Result<()>, Vec<(String, PathBuf)>) {
    let mut saved = Vec::new();
    let mut out = Vec::new();
    let mut save = |key: &str, path: &Path| {
        saved.push((key.to_string(), path.to_path_buf()));
        Ok(PathBuf::from("config.toml"))
    };
    let res = ask_hosts(
        cfg,
        &["claude"],
        &mut answers.as_bytes(),
        &mut out,
        &mut save,
    );
    (res, saved)
}

#[test]
fn a_confirmed_path_reaches_the_run_and_is_saved_only_on_yes() {
    let dir = scratch("save");
    let bin = program(&dir, "claude");
    let shown = bin.display();

    let mut cfg = Config::default();
    let (res, saved) = ask_hosts_with(&mut cfg, &format!("{shown}\ny\ny\n"));
    res.unwrap();
    assert_eq!(saved, [("setup.paths.claude".to_string(), bin.clone())]);
    assert_eq!(cfg.setup.paths.get("claude"), Some(&bin));
    assert!(absent(&cfg, &[&Claude], |_| true).is_empty());

    let mut cfg = Config::default();
    let (res, saved) = ask_hosts_with(&mut cfg, &format!("{shown}\ny\nn\n"));
    res.unwrap();
    assert!(saved.is_empty());
    assert_eq!(cfg.setup.paths.get("claude"), Some(&bin));

    let mut cfg = Config::default();
    cfg.setup.dry_run = true;
    let (res, saved) = ask_hosts_with(&mut cfg, &format!("{shown}\ny\ny\n"));
    res.unwrap();
    assert!(saved.is_empty(), "a dry run writes no config");

    let mut cfg = Config::default();
    let (res, saved) = ask_hosts_with(&mut cfg, "\n");
    assert_eq!(res.unwrap_err().to_string(), "stopped: no path for claude");
    assert!(saved.is_empty() && cfg.setup.paths.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The saved value is the TOML string `ask_missing` writes; it loads back as the same path,
/// spaces and all.
#[test]
fn a_saved_path_loads_back_from_the_config_file() {
    let dir = scratch("cfg");
    let file = dir.join("config.toml");
    let path = dir.join("a b").join("claude");
    let value = toml_edit::Value::from(path.to_string_lossy().into_owned()).to_string();
    crate::config::validate::set_with(&dir, Some(&file), &key("claude"), &value, false).unwrap();
    let cfg = Config::load_with(Some(&file), None).unwrap();
    assert_eq!(cfg.setup.paths.get("claude"), Some(&path));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Without a terminal nothing is asked: the test harness has none.
#[test]
fn no_terminal_no_question() {
    assert!(!interactive());
    let mut cfg = Config::default();
    ask_missing(&mut cfg, None, &[&Claude], |_| true).unwrap();
    assert!(cfg.setup.paths.is_empty());
}
