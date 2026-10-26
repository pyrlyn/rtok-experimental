// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T99: the Grok Build plugin tree ships a manifest and rtok's hooks. MCP is independent of the
//! plugin (T275/D33): `rtok agents install grok` writes `[mcp_servers.rtok]` into
//! `~/.grok/config.toml` itself, so the plugin carries no `.mcp.json` of its own.

use crate::it::common;

use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

fn read(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("plugins/grok")
        .join(name);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// T250.4: the shell one-liner every hook runs — PATH first, then ketch's install dir, else a
/// silent no-op (fail open; Grok's PowerShell runner has no per-OS field, so Windows users get
/// `rtok agents install claude` instead — plugin stays macOS/Linux only).
fn hook_command(event: &str) -> String {
    format!(
        "command -v rtok >/dev/null 2>&1 && exec rtok hook {event} --host grok; \
[ -x \"$HOME/.ketch/bin/rtok\" ] && exec \"$HOME/.ketch/bin/rtok\" hook {event} --host grok; exit 0"
    )
}

#[test]
fn manifest_is_rtok() {
    assert_eq!(read(".grok-plugin/plugin.json")["name"], "rtok");
}

/// Claude's installer entries (`agents::claude::ENTRIES`) minus `Read` and `Skill` (plan T98):
/// Grok's matcher is a regex, so the catch-all PostToolUse omits it rather than use Claude's `*`.
#[test]
fn hooks_are_the_claude_set_without_read_and_skill() {
    let want = [
        ("PreToolUse", Some("Bash")),
        ("PostToolUse", None),
        ("UserPromptSubmit", None),
        ("SessionStart", None),
        ("PreCompact", None),
        ("PostCompact", None),
        ("SessionEnd", None),
    ];
    let hooks = read("hooks/hooks.json")["hooks"].clone();
    assert_eq!(hooks.as_object().unwrap().len(), want.len(), "{hooks}");
    for (event, matcher) in want {
        let mut group = json!({"hooks": [{
            "type": "command",
            "command": hook_command(event),
            "timeout": 5
        }]});
        if let Some(m) = matcher {
            group["matcher"] = json!(m);
        }
        assert_eq!(hooks[event], json!([group]), "{event}");
    }
}

/// T250.4 check: each event's hook command resolves rtok from `PATH`, falls back to
/// `~/.ketch/bin/rtok`, and exits 0 silently — no note, not even on `SessionStart` — when
/// neither exists (fail open); with only the ketch copy present, it runs that one.
#[cfg(unix)]
#[test]
fn hooks_resolve_rtok_from_path_then_ketch_else_exit_0_silently() {
    let hooks = read("hooks/hooks.json")["hooks"].clone();
    let events: Vec<String> = hooks.as_object().unwrap().keys().cloned().collect();
    assert!(!events.is_empty());

    let sh = common::HookShell::new("t250.4-grok");
    for event in &events {
        let command = hooks[event][0]["hooks"][0]["command"].as_str().unwrap();
        let (ok, stdout) = sh.run(command);
        assert!(ok, "{event}: expected exit 0 with no rtok anywhere");
        assert_eq!(stdout, "", "{event}: expected silence, got {stdout:?}");
    }

    sh.install_fake_ketch_rtok(common::KETCH_ECHO);
    for event in &events {
        let command = hooks[event][0]["hooks"][0]["command"].as_str().unwrap();
        let (ok, stdout) = sh.run(command);
        assert!(ok, "{event}: expected exit 0 with ketch rtok");
        assert_eq!(stdout, format!("ketch {event}"), "{event}");
    }
}
