// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T115 + T139 + D21: `rtok agents install claude` installs `plugins/claude` through the official
//! `claude plugin` commands, by default once `claude` is on PATH — no `--yes` needed — from the
//! GitHub marketplace `pyrlyn/rtok` (a fake `claude` first on PATH records the calls), and while
//! the plugin is installed it is the only call path for hooks — the settings-file hooks go.
//! MCP is independent of the plugin (T275): `mcpServers.rtok` is always written to
//! `~/.claude.json` and `claude_desktop_config.json`, plugin or no plugin.
#![cfg(unix)]

use crate::it::common;

use common::agents::{claude_desktop_config, claude_log, json, rtok, tmp, write_cfg};
use std::fs;

/// T275: the desktop app's Code tab loads `claude_desktop_config.json` and the Claude Code
/// plugin both, but the plugin no longer serves MCP, so there is no second rtok server to
/// guard against there any more — installing the plugin leaves the desktop entry alone, and
/// keeps any foreign ones.
#[test]
fn plugin_install_keeps_the_desktop_mcp_entry() {
    let home = tmp("claude-plugin-desktop");
    let cfg = write_cfg(&home);
    let file = claude_desktop_config(&home);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, r#"{"mcpServers":{"foreign":{"command":"x"}}}"#).unwrap();

    // Desktop alone, no plugin yet: the entry is written.
    rtok(&["agents", "install", "claude", "--desktop"], &cfg, &home);
    assert!(json(&file)["mcpServers"]["rtok"].is_object());

    // Both variants: the CLI installs the plugin, and the desktop entry stays put.
    let out = rtok(&["agents", "install", "claude"], &cfg, &home);
    assert!(out.contains("+ plugin plugins/claude → rtok@rtok"), "{out}");
    let servers = json(&file);
    assert!(servers["mcpServers"]["rtok"].is_object(), "{servers}");
    assert!(servers["mcpServers"]["foreign"].is_object(), "{servers}");
    let claude_json = home.join(".claude.json");
    assert!(
        json(&claude_json)["mcpServers"]["rtok"].is_object(),
        "the CLI's own file gets it too"
    );

    // A rerun, or the desktop alone, is idle either way.
    let again = rtok(&["agents", "install", "claude"], &cfg, &home);
    assert!(again.contains("already installed"), "{again}");
    rtok(&["agents", "install", "claude", "--desktop"], &cfg, &home);
    assert!(json(&file)["mcpServers"]["rtok"].is_object());
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn dry_run_offers_the_claude_commands_and_runs_nothing() {
    let home = tmp("claude-plugin-dry");
    let cfg = write_cfg(&home);
    // No `--yes`: a dry run previews the default install (T139).
    let out = rtok(&["agents", "install", "claude", "--dry-run"], &cfg, &home);
    assert!(out.contains("offer plugins/claude"), "{out}");
    assert!(
        out.contains("claude plugin marketplace add pyrlyn/rtok"),
        "{out}"
    );
    assert!(out.contains("claude plugin install rtok@rtok"), "{out}");
    assert!(out.contains("ketch install pyrlyn/rtok"), "{out}");
    assert_eq!(claude_log(&home), "", "a dry run calls no claude");
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn installs_the_plugin_by_default_as_the_only_call_path_and_remove_uninstalls() {
    let home = tmp("claude-plugin-default");
    let cfg = write_cfg(&home);
    let settings = home.join(".claude/settings.json");
    let claude_json = home.join(".claude.json");

    // A plain install (no `--yes`) installs the plugin outright once `claude` is on PATH (T139).
    let plain = rtok(&["agents", "install", "claude"], &cfg, &home);
    assert!(
        plain.contains("+ plugin plugins/claude → rtok@rtok"),
        "{plain}"
    );
    // T132: the plugin's `agents/rtok-scout.md` rides along with the rest of the plugin tree.
    let scout = home.join(".claude/plugins/cache/rtok/agents/rtok-scout.md");
    assert!(scout.is_file(), "{}", scout.display());
    let log = claude_log(&home);
    let calls: Vec<&str> = log.lines().collect();
    assert_eq!(calls.len(), 2, "{log}");
    assert_eq!(calls[0], "plugin marketplace add pyrlyn/rtok");
    assert_eq!(calls[1], "plugin install rtok@rtok");
    // D21 singleton: the plugin serves hooks instead — the settings file is never written (or,
    // if written, carries none). MCP is independent of the plugin (T275): `~/.claude.json`
    // still gets `mcpServers.rtok`.
    assert!(
        !fs::read_to_string(&settings)
            .unwrap_or_default()
            .contains(" hook "),
        "settings hooks stripped"
    );
    assert!(
        fs::read_to_string(&claude_json)
            .unwrap_or_default()
            .contains("\"rtok\""),
        "mcpServers.rtok must still be registered (T275)"
    );

    let again = rtok(&["agents", "install", "claude"], &cfg, &home);
    assert!(again.contains("already installed"), "{again}");
    assert_eq!(claude_log(&home).lines().count(), 2, "no second install");

    let removed = rtok(&["agents", "remove", "claude"], &cfg, &home);
    assert!(removed.contains("- plugin rtok@rtok"), "{removed}");
    assert!(!scout.exists(), "removal must take the agent file with it");
    assert!(
        !fs::read_to_string(&claude_json)
            .unwrap_or_default()
            .contains("\"rtok\""),
        "remove takes the mcp entry out too"
    );
    let log = claude_log(&home);
    let tail: Vec<&str> = log.lines().skip(2).collect();
    assert_eq!(
        tail,
        [
            "plugin uninstall rtok@rtok",
            "plugin marketplace remove rtok"
        ]
    );
    let _ = fs::remove_dir_all(&home);
}

/// T178: every plugin hook command execs `rtok` from PATH in Claude Code's own shell, stdin
/// included, and falls back to `scripts/hook.sh` (the fail-open launcher) only when PATH has none.
#[test]
fn hook_commands_exec_rtok_from_path_and_fall_back_to_hook_sh() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};

    let home = tmp("claude-plugin-launch");
    let script = |path: std::path::PathBuf, body: &str| {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    };
    script(
        home.join("bin/rtok"),
        r#"printf 'rtok %s %s ' "$1" "$2"; cat"#,
    );
    script(
        home.join("root/scripts/hook.sh"),
        // Drains stdin first: exiting before the test's write lands made it fail with EPIPE.
        r#"cat >/dev/null; printf 'fallback %s' "$1""#,
    );
    let run = |cmd: &str, path: String| {
        let mut child = Command::new("/bin/sh")
            .args(["-c", cmd])
            .env("PATH", path)
            .env("CLAUDE_PLUGIN_ROOT", home.join("root"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        // A fail-open hook may exit before reading stdin: a broken pipe is fine (T251).
        drop(child.stdin.take().unwrap().write_all(b"{}"));
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{cmd}");
        String::from_utf8(out.stdout).unwrap()
    };
    let hooks: serde_json::Value =
        serde_json::from_str(include_str!("../../plugins/claude/hooks/hooks.json")).unwrap();
    for (event, entries) in hooks["hooks"].as_object().unwrap() {
        // T159: the worktree launcher has its own contract (`tests/worktree_hooks.rs`).
        if event.starts_with("Worktree") {
            continue;
        }
        for cmd in entries
            .as_array()
            .unwrap()
            .iter()
            .map(|e| &e["hooks"][0]["command"])
        {
            let cmd = cmd.as_str().unwrap();
            let with = format!("{}:/usr/bin:/bin", home.join("bin").display());
            assert_eq!(run(cmd, with), format!("rtok hook {event} {{}}"));
            assert_eq!(
                run(cmd, "/usr/bin:/bin".into()),
                format!("fallback {event}")
            );
        }
    }
    let _ = fs::remove_dir_all(&home);
}

/// T174 check: the real `scripts/hook.sh`, with an empty PATH and a `HOME` carrying no
/// `.ketch/bin/rtok`, fails open silently (exit 0, empty stdout) on every event but
/// `SessionStart`, which gets exactly one `hookSpecificOutput` note naming the ketch
/// install — and still prefers a `~/.ketch/bin/rtok` that does exist over that note.
#[test]
fn hook_sh_fails_open_silently_except_one_session_start_note() {
    let sh = common::HookShell::new("claude-hook-sh-fail-open");
    let script = plugins_dir().join("claude/scripts/hook.sh");
    let run = |event: &str| {
        let (ok, stdout) = sh.sh(&[script.to_str().unwrap(), event]);
        assert!(ok, "{event}: {stdout:?}");
        stdout
    };

    assert_eq!(run("PreToolUse"), "");
    assert_eq!(run("PostToolUse"), "");
    let note = run("SessionStart");
    let v: serde_json::Value =
        serde_json::from_str(&note).unwrap_or_else(|e| panic!("{note}: {e}"));
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "SessionStart");
    assert!(
        v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("ketch install pyrlyn/rtok"),
        "{note}"
    );

    sh.install_fake_ketch_rtok(common::KETCH_ECHO);
    assert_eq!(run("SessionStart"), "ketch SessionStart");
}

fn plugins_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins")
}
