// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T121 + D21: the Codex plugin tree is a manifest and rtok's hooks (T275/D33 dropped its MCP
//! server).

use crate::it::common;

use rtok::agents::hook_events;
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

fn read(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("plugins/codex")
        .join(name);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn manifest_is_rtok_and_points_at_its_files() {
    let m = read(".codex-plugin/plugin.json");
    assert_eq!(m["name"], "rtok");
    assert_eq!(m["hooks"], "./hooks/hooks.json");
    assert!(m.get("mcpServers").is_none(), "{m}");
    assert!(
        !PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("plugins/codex/.mcp.json")
            .exists(),
        "T275/D33: the plugin no longer ships an MCP server"
    );
}

#[test]
fn marketplace_lists_this_folder() {
    let m = read(".agents/plugins/marketplace.json");
    assert_eq!(m["plugins"][0]["name"], "rtok");
    assert_eq!(
        m["plugins"][0]["source"],
        json!({"source": "local", "path": "./"})
    );
}

/// The repo-root marketplace `codex plugin marketplace add pyrlyn/rtok` resolves (T140):
/// same shape as the nested dev marketplace above, but its one plugin points at the
/// `plugins/codex` subdirectory instead of `./`, since the marketplace file itself lives at
/// the repo root, not inside the plugin's own folder.
#[test]
fn root_marketplace_points_at_the_plugins_codex_subdirectory() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".agents/plugins/marketplace.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let m: Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(m["name"], "rtok");
    assert_eq!(m["plugins"][0]["name"], "rtok");
    assert_eq!(
        m["plugins"][0]["source"],
        json!({"source": "local", "path": "./plugins/codex"})
    );
}

/// The installer's event set is the `codex` rows of `hook_events` (matchers included): the
/// plugin fires what the installer does, each as `rtok hook <event> --host codex` (T645).
#[test]
fn hooks_are_the_installer_rows() {
    let hooks = read("hooks/hooks.json")["hooks"].clone();
    let rows: Vec<_> = hook_events::for_host("codex").collect();
    assert_eq!(hooks.as_object().unwrap().len(), rows.len(), "{hooks}");
    for row in rows {
        let event = row.host_event;
        assert_eq!(row.rtok_event, event, "Codex events keep their names");
        let mut entry = json!({"hooks": [{
            "type": "command",
            "command": format!(
                "command -v rtok >/dev/null 2>&1 && exec rtok hook {event} --host codex; \
                 [ -x \"$HOME/.ketch/bin/rtok\" ] && exec \"$HOME/.ketch/bin/rtok\" hook {event} --host codex; \
                 exit 0"
            ),
            "commandWindows": format!("rtok hook {event} --host codex"),
            "timeout": 5
        }]});
        if !row.matcher.is_empty() {
            entry["matcher"] = json!(row.matcher);
        }
        assert_eq!(hooks[event], json!([entry]), "{event}");
    }
}

/// T250.1 check: on Unix, each event's `command` resolves `rtok` from PATH first, then falls
/// back to `~/.ketch/bin/rtok`, and exits 0 silently when neither exists — matching the fail-open
/// launcher pattern used by `plugins/claude/scripts/hook.sh`.
#[test]
#[cfg(unix)]
fn hook_commands_resolve_path_then_ketch_then_exit_open() {
    let sh = common::HookShell::new("t250.1-codex-hooks");
    let hooks = read("hooks/hooks.json")["hooks"].clone();
    for event in hook_events::for_host("codex").map(|r| r.host_event) {
        let command = hooks[event][0]["hooks"][0]["command"].as_str().unwrap();
        let run = || {
            let (ok, stdout) = sh.run(command);
            assert!(ok, "{event}: {command}");
            stdout
        };

        // Neither PATH nor ~/.ketch/bin has rtok: fail open, no output.
        assert_eq!(run(), "", "{event}");

        // Only ~/.ketch/bin/rtok exists: it is the one that gets exec'd.
        sh.install_fake_ketch_rtok(common::KETCH_ECHO);
        assert_eq!(run(), format!("ketch {event}"), "{event}");
        fs::remove_dir_all(sh.home().join(".ketch")).unwrap();
    }
}
