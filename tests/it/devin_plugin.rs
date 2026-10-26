// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T88 + D21: the Devin plugin tree is one unit — a manifest, rtok's hooks and one MCP server,
//! loaded by both the Devin CLI and Devin Desktop.

use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

fn read(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("plugins/devin")
        .join(name);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The shared POSIX resolver (`agents::hook_resolver`, no note): PATH, then ketch's install
/// dir, else a silent exit 0. `exec rtok hook <event> --host devin;` is the marker an
/// installer's `is_ours` keys on (the Cursor pattern), so T89 can recognise these entries.
fn hook_command(event: &str) -> String {
    format!(
        "command -v rtok >/dev/null 2>&1 && exec rtok hook {event} --host devin; \
[ -x \"$HOME/.ketch/bin/rtok\" ] && exec \"$HOME/.ketch/bin/rtok\" hook {event} --host devin; \
true; exit 0"
    )
}

#[test]
fn manifest_is_rtok() {
    assert_eq!(read(".devin-plugin/plugin.json")["name"], "rtok");
}

#[test]
fn mcp_is_exactly_rtok() {
    assert_eq!(
        read(".mcp.json"),
        json!({"mcpServers": {"rtok": {"command": "rtok", "args": ["mcp"]}}})
    );
}

/// Claude's installer entries (`agents::claude::ENTRIES`) in Devin's names: `exec`/`read` for
/// `Bash`/`Read` (regex matchers, anchored), no `Skill`, `PostCompaction` for the compaction
/// pair (Devin has no `PreCompact`). Event names sit at the top level of a plugin's
/// `hooks.json`, with no `"hooks"` wrapper.
#[test]
fn hooks_are_the_claude_set_in_devin_names() {
    let want: &[(&str, &[Option<&str>])] = &[
        ("PreToolUse", &[Some("^exec$"), Some("^read$")]),
        ("PostToolUse", &[None]),
        ("UserPromptSubmit", &[None]),
        ("SessionStart", &[None]),
        ("PostCompaction", &[None]),
        ("SessionEnd", &[None]),
    ];
    let hooks = read("hooks.json");
    assert!(hooks.get("hooks").is_none(), "no wrapper key: {hooks}");
    assert_eq!(hooks.as_object().unwrap().len(), want.len(), "{hooks}");
    for (event, matchers) in want {
        let groups: Vec<Value> = matchers
            .iter()
            .map(|m| {
                let mut group = json!({"hooks": [{
                    "type": "command",
                    "command": hook_command(event),
                    "timeout": 5
                }]});
                if let Some(m) = m {
                    group["matcher"] = json!(m);
                }
                group
            })
            .collect();
        assert_eq!(hooks[event], json!(groups), "{event}");
    }
}

/// Each hook command resolves rtok from `PATH`, falls back to `~/.ketch/bin/rtok`, and exits 0
/// silently when neither exists (fail open: Devin blocks only on exit 2, and a missing binary
/// must not log exit 127 on every call); with only the ketch copy present, it runs that one
/// with the Devin event name and `--host devin`.
#[cfg(unix)]
#[test]
fn hooks_resolve_rtok_from_path_then_ketch_else_exit_0_silently() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};

    let hooks = read("hooks.json");
    let commands: Vec<(String, String)> = hooks
        .as_object()
        .unwrap()
        .iter()
        .flat_map(|(event, groups)| {
            groups.as_array().unwrap().iter().map(move |g| {
                let c = g["hooks"][0]["command"].as_str().unwrap().to_string();
                (event.clone(), c)
            })
        })
        .collect();
    assert_eq!(commands.len(), 7);

    let unique = format!(
        "rtok-t88-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let home = std::env::temp_dir().join(format!("{unique}-home"));
    let empty_path = std::env::temp_dir().join(format!("{unique}-path"));
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&empty_path).unwrap();

    let run = |command: &str| -> (bool, String) {
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .env("HOME", &home)
            .env("PATH", &empty_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        // A fail-open hook with no rtok exits without reading stdin: the write may hit BrokenPipe.
        let _ = child.stdin.take().unwrap().write_all(b"{}");
        let out = child.wait_with_output().unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };

    for (event, command) in &commands {
        let (ok, stdout) = run(command);
        assert!(ok, "{event}: expected exit 0 with no rtok anywhere");
        assert_eq!(stdout, "", "{event}: expected silence, got {stdout:?}");
    }

    let ketch_dir = home.join(".ketch").join("bin");
    fs::create_dir_all(&ketch_dir).unwrap();
    let ketch_rtok = ketch_dir.join("rtok");
    fs::write(
        &ketch_rtok,
        "#!/bin/sh\nprintf 'ketch %s %s %s' \"$2\" \"$3\" \"$4\"\n",
    )
    .unwrap();
    fs::set_permissions(&ketch_rtok, fs::Permissions::from_mode(0o755)).unwrap();

    for (event, command) in &commands {
        let (ok, stdout) = run(command);
        assert!(ok, "{event}: expected exit 0 with ketch rtok");
        assert_eq!(stdout, format!("ketch {event} --host devin"), "{event}");
    }

    let _ = fs::remove_dir_all(&home);
    let _ = fs::remove_dir_all(&empty_path);
}
