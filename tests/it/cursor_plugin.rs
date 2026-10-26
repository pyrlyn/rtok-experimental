// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T10.5 + D21: Cursor host plugin is hooks only (T275/D33 dropped its MCP server),
//! desktop+CLI, ketch if missing.
//!
//! Check: `rtok agents install cursor --dry-run` names `plugins/cursor` and
//! `~/.cursor/plugins/local`; `--yes` links the plugin and `~/.cursor/mcp.json`'s
//! `mcpServers.rtok` is written independently, plugin linked or not; second apply is `no
//! changes`.

use crate::common;

use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins/cursor")
}

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rtok")
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rtok-t105-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_cfg(home: &Path) -> PathBuf {
    let cursor = home.join(".cursor");
    fs::create_dir_all(&cursor).unwrap();
    let cfg = home.join("config.toml");
    fs::write(
        &cfg,
        format!(
            "[setup.cursor]\nhooks_path = \"{}/hooks.json\"\n",
            // `/`: a `\` in a TOML basic string starts an escape (T83.4).
            cursor.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    cfg
}

/// Every call here is `agents install cursor …`; `--no-restart` (T141) keeps the test from
/// ever probing or touching a real Cursor process on the machine running it.
fn setup(args: &[&str], cfg: &Path, home: &Path) -> (String, String, i32) {
    let out = Command::new(bin())
        .args(["--config", cfg.to_str().unwrap()])
        .args(args)
        .arg("--no-restart")
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("PATH", common::agents::fake_hosts(home))
        .env("RTOK_HOME", home.join(".rtok"))
        .output()
        .expect("rtok setup");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(1),
    )
}

#[test]
fn d21_manifest_carries_hooks_and_no_mcp() {
    let dir = root();
    let cursor = fs::read_to_string(dir.join(".cursor-plugin/plugin.json")).unwrap();
    let agent = fs::read_to_string(dir.join("plugin.json")).unwrap();
    assert!(cursor.contains("\"hooks\""), "{cursor}");
    assert!(agent.contains("\"name\": \"rtok\""), "{agent}");
    assert!(
        !cursor.contains("mcpServers"),
        "T275/D33: the plugin no longer ships an MCP server: {cursor}"
    );
    assert!(
        !dir.join("mcp.json").exists(),
        "T275/D33: the plugin no longer ships an MCP server"
    );
}

#[test]
fn d21_desktop_and_cli_manifests() {
    let dir = root();
    assert!(dir.join(".cursor-plugin/plugin.json").is_file());
    assert!(dir.join("plugin.json").is_file());
}

#[test]
fn d21_no_duplicate_call_paths() {
    let hooks: Value =
        serde_json::from_str(&fs::read_to_string(root().join("hooks/hooks.json")).unwrap())
            .unwrap();
    let cmd = hooks["hooks"]["beforeShellExecution"][0]["command"]
        .as_str()
        .unwrap_or("");
    assert!(cmd.contains("rtok hook PreToolUse --host cursor"), "{cmd}");
    assert!(
        !cmd.contains("rtok read") && !cmd.contains("rtok search"),
        "hooks must not duplicate MCP read/search: {cmd}"
    );
}

#[test]
fn d21_no_launcher_scripts_rtok_must_be_on_path() {
    // T197 (T85/I-37): rtok's own `~/.cursor/mcp.json` spawns `rtok mcp` directly through a
    // single `command`/`args` pair with no per-OS slot, so launcher scripts could
    // never run — the deleted `scripts/mcp.*` were dead code with green tests. The plugin
    // itself ships no MCP server at all now (T275/D33).
    assert!(
        !root().join("scripts").exists(),
        "no scripts/ in plugins/cursor: mcp.json spawns rtok directly"
    );
    let readme = fs::read_to_string(root().join("README.md")).unwrap();
    assert!(
        readme.contains("ketch install pyrlyn/rtok"),
        "README must name the ketch install: {readme}"
    );
}

#[test]
fn setup_cursor_dry_run_offers_plugin() {
    let home = tmp("dry");
    let cfg = write_cfg(&home);
    let (stdout, stderr, code) = setup(&["agents", "install", "cursor", "--dry-run"], &cfg, &home);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("plugins/cursor"), "stdout={stdout}");
    assert!(
        stdout.contains("~/.cursor/plugins/local"),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("ketch install pyrlyn/rtok"),
        "stdout={stdout}"
    );
    assert!(
        !home.join(".cursor/plugins/local/rtok").exists(),
        "dry-run must not link"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn setup_cursor_yes_links_plugin_and_still_registers_mcp() {
    let home = tmp("yes");
    let cfg = write_cfg(&home);
    let (stdout, stderr, code) = setup(&["agents", "install", "cursor", "--yes"], &cfg, &home);
    assert_eq!(code, 0, "stderr={stderr} stdout={stdout}");
    let dest = home.join(".cursor/plugins/local/rtok");
    let meta = fs::symlink_metadata(&dest).unwrap_or_else(|e| panic!("{}: {e}", dest.display()));
    assert!(meta.file_type().is_symlink() || dest.is_dir(), "{dest:?}");
    // T275/D33: mcp.json gets rtok's entry even with the plugin linked.
    let mcp_path = home.join(".cursor/mcp.json");
    let body =
        fs::read_to_string(&mcp_path).unwrap_or_else(|e| panic!("{}: {e}", mcp_path.display()));
    assert!(body.contains("\"rtok\""), "{body}");
    let (again, stderr2, code2) = setup(&["agents", "install", "cursor", "--yes"], &cfg, &home);
    assert_eq!(code2, 0, "stderr={stderr2}");
    assert!(again.contains("already installed"), "second apply: {again}");
    assert!(
        fs::read_to_string(&mcp_path).unwrap().contains("\"rtok\""),
        "a repeat install must not strip the mcp entry"
    );
    let (rm, stderr3, code3) = setup(&["agents", "install", "cursor", "--remove"], &cfg, &home);
    assert_eq!(code3, 0, "stderr={stderr3}");
    assert!(!dest.exists(), "remove must unlink plugin; stdout={rm}");
    assert!(
        !fs::read_to_string(&mcp_path)
            .unwrap_or_default()
            .contains("\"rtok\""),
        "remove takes the mcp entry out too"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn setup_cursor_keeps_mcp_entry_while_plugin_already_linked() {
    let home = tmp("leftover-mcp");
    let cfg = write_cfg(&home);
    let (stdout, stderr, code) = setup(&["agents", "install", "cursor", "--yes"], &cfg, &home);
    assert_eq!(code, 0, "stderr={stderr} stdout={stdout}");
    let dest = home.join(".cursor/plugins/local/rtok");
    assert!(dest.symlink_metadata().is_ok(), "plugin must be linked");
    let mcp_path = home.join(".cursor/mcp.json");
    fs::write(
        &mcp_path,
        r#"{"mcpServers":{"rtok":{"type":"stdio","command":"rtok","args":["mcp"]},"other":{"command":"x"}}}"#,
    )
    .unwrap();
    // T275/D33: MCP is independent of the plugin — a repeat run keeps the entry (and the
    // foreign one beside it), it never clears it just because the plugin is linked.
    let (again, stderr2, code2) = setup(&["agents", "install", "cursor"], &cfg, &home);
    assert_eq!(code2, 0, "stderr={stderr2} stdout={again}");
    let body = fs::read_to_string(&mcp_path).unwrap();
    assert!(body.contains("\"rtok\""), "{body}");
    assert!(
        body.contains("other"),
        "foreign MCP servers must remain: {body}"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn post_tool_use_shortens_long_mcp_results_and_skips_small_and_rtok() {
    let home = tmp("mcp-hook");
    let rtok_home = home.join(".rtok");
    fs::create_dir_all(&rtok_home).unwrap();
    let long: String = (1..=200).map(|i| format!("line {i}\n")).collect();
    let hook = |server: &str, tool: &str, text: &str| -> Value {
        let result = serde_json::json!({"content":[{"type":"text","text": text}]});
        let stdin = serde_json::json!({
            "hook_event_name": "postToolUse",
            "tool_name": tool,
            "tool_input": {},
            "tool_output": result.to_string(),
            "conversation_id": "e2e",
            "mcp_server_name": server
        });
        let mut child = Command::new(bin())
            .args(["hook", "PostToolUse", "--host", "cursor"])
            .env("RTOK_HOME", &rtok_home)
            .env("HOME", &home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&stdin).unwrap())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let big = hook("linear", "MCP:list_issues", &long);
    let printed = big["updated_mcp_tool_output"]["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    assert!(printed.contains("expand: rtok expand "), "{big}");
    assert!(printed.len() < long.len());
    let rows = rtok::store::Store::open(&rtok_home.join("rtok.db"))
        .unwrap()
        .list_measurements("archive")
        .unwrap();
    assert_eq!(rows.iter().filter(|r| r.kind == "mcp").count(), 1);
    assert_eq!(hook("linear", "MCP:list_issues", "ok\n"), json!({}));
    assert_eq!(hook("rtok", "MCP:search", &long), json!({}));
    let _ = fs::remove_dir_all(&home);
}

/// T250.3 check: every plugin hook runs as Cursor runs it on Unix —
/// `sh -c "<command> <<'CURSOR_HOOK_EOF' …"` — with an empty PATH and a temp HOME. With no
/// rtok anywhere each exits 0 silently, sessionStart alone printing Cursor's flat note; a
/// fake `~/.ketch/bin/rtok` then gets the event and the heredoc payload on its stdin.
#[cfg(unix)]
#[test]
fn hooks_resolve_rtok_from_path_then_ketch_else_exit_0() {
    let hooks: Value =
        serde_json::from_str(&fs::read_to_string(root().join("hooks/hooks.json")).unwrap())
            .unwrap();
    let hooks = hooks["hooks"].as_object().unwrap().clone();
    assert_eq!(
        hooks.len(),
        rtok::agents::hook_events::for_host("cursor").count(),
        "{hooks:?}"
    );
    let sh = common::HookShell::new("t250-cursor");
    // The heredoc replaces the harness's own stdin, as it does in Cursor.
    let run = |command: &str| {
        sh.run(&format!(
            "{command} <<'CURSOR_HOOK_EOF'\n{{\"n\":1}}\nCURSOR_HOOK_EOF"
        ))
    };
    let note = r#"{"additional_context":"rtok is not installed; run ketch install pyrlyn/rtok to enable it."}"#;
    for (event, entries) in &hooks {
        let (ok, stdout) = run(entries[0]["command"].as_str().unwrap());
        assert!(ok, "{event}");
        let want = if event == "sessionStart" { note } else { "" };
        assert_eq!(stdout, want, "{event}");
    }

    // Builtins only: PATH is empty inside the hook.
    sh.install_fake_ketch_rtok(
        "#!/bin/sh\nIFS= read -r line\nprintf '%s %s\\n' \"$2\" \"$line\"\n",
    );
    for (event, entries) in &hooks {
        let command = entries[0]["command"].as_str().unwrap();
        let rtok_event = command
            .split_once("exec rtok hook ")
            .and_then(|(_, r)| r.split_once(' '))
            .unwrap()
            .0;
        let (ok, stdout) = run(command);
        assert!(ok, "{event}");
        assert_eq!(stdout, format!("{rtok_event} {{\"n\":1}}\n"), "{event}");
    }
}
