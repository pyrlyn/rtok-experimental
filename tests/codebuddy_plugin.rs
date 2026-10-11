// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T560.1 + D21: the CodeBuddy Code / WorkBuddy plugin tree carries a `.codebuddy-plugin`
//! manifest and marketplace, Claude-shaped `hooks/hooks.json` that run `rtok hook <Event> --host
//! codebuddy` (events per `hook_events`), and no MCP server: that is the installer's
//! `mcpServers.rtok` (D33, T560.2).

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins/codebuddy")
}

fn read(name: &str) -> Value {
    let path = root().join(name);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn manifest_and_marketplace_name_the_rtok_plugin() {
    let m = read(".codebuddy-plugin/plugin.json");
    assert_eq!(m["name"], "rtok");
    assert_eq!(m["version"], env!("CARGO_PKG_VERSION"));
    let mp = read(".codebuddy-plugin/marketplace.json");
    assert_eq!(mp["name"], "rtok");
    assert_eq!(mp["owner"]["name"], "pyrlyn");
    assert_eq!(
        mp["plugins"],
        json!([{
            "name": "rtok",
            "source": "./",
            "description": "rtok hooks for CodeBuddy Code and WorkBuddy"
        }])
    );
}

/// D33: the installer's `mcpServers.rtok` is the only MCP entry; a plugin `.mcp.json` would
/// start a second rtok server and double the tool set.
#[test]
fn the_plugin_ships_no_mcp_server() {
    assert!(!root().join(".mcp.json").exists());
    assert!(
        read(".codebuddy-plugin/plugin.json")
            .get("mcpServers")
            .is_none()
    );
}

/// The events and matchers themselves are held to `hook_events::HOOK_EVENTS` by
/// `tests/hook_manifests.rs`; this pins the command chain and what the table cannot say.
#[test]
fn hooks_run_the_three_step_chain_with_the_codebuddy_host() {
    let doc = read("hooks/hooks.json");
    let hooks = doc["hooks"].as_object().unwrap();
    let matchers: Vec<_> = hooks["PreToolUse"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["matcher"].as_str().unwrap())
        .collect();
    assert_eq!(matchers, ["Bash", "Read", "Skill"]);
    assert!(
        !hooks.contains_key("Stop"),
        "rtok registers no Stop hook for Claude-shaped hosts"
    );
    for (event, entries) in hooks {
        let hook = &entries[0]["hooks"][0];
        assert_eq!(
            hook["command"],
            format!(
                "command -v rtok-hook >/dev/null 2>&1 && exec rtok-hook {event} --host codebuddy; \
                 command -v rtok >/dev/null 2>&1 && exec rtok hook {event} --host codebuddy; \
                 exec \"${{CODEBUDDY_PLUGIN_ROOT}}/scripts/hook.sh\" {event}"
            ),
            "{event}"
        );
        assert_eq!(hook["type"], "command");
        assert_eq!(hook["timeout"], 5);
    }
}

#[cfg(unix)]
mod launcher {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::Command;

    /// A directory holding an `rtok` shim that prints its arguments, and an `rtok-hook` one
    /// when `with_daemon_client` (it prefixes the output so the two are told apart).
    fn shim_dir(name: &str, with_daemon_client: bool) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rtok-t560-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut shims = vec![("rtok", "#!/bin/sh\necho \"$@\"\n")];
        if with_daemon_client {
            shims.push(("rtok-hook", "#!/bin/sh\necho \"rtok-hook $@\"\n"));
        }
        for (name, body) in shims {
            let shim = dir.join(name);
            fs::write(&shim, body).unwrap();
            fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir
    }

    fn run(cmd: &str, path: &Path, home: &Path) -> (bool, String) {
        let out = Command::new("/bin/sh")
            .args(["-c", cmd])
            .env("PATH", format!("{}:/usr/bin:/bin", path.display()))
            .env("HOME", home)
            .env("CODEBUDDY_PLUGIN_ROOT", root())
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    }

    #[test]
    fn every_hook_command_prefers_rtok_hook_then_rtok() {
        let doc = read("hooks/hooks.json");
        for (with_client, want) in [(true, "rtok-hook "), (false, "")] {
            let dir = shim_dir(&format!("path{with_client}"), with_client);
            for (event, entries) in doc["hooks"].as_object().unwrap() {
                let cmd = entries[0]["hooks"][0]["command"].as_str().unwrap();
                let (ok, stdout) = run(cmd, &dir, &dir);
                assert!(ok, "{event}");
                let rest = if with_client { "" } else { "hook " };
                assert_eq!(
                    stdout.trim(),
                    format!("{want}{rest}{event} --host codebuddy")
                );
            }
            let _ = fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn the_launcher_falls_back_to_the_ketch_store_and_fails_open_without_rtok() {
        let script = root().join("scripts/hook.sh");
        let launch = |home: &Path, event: &str| {
            run(
                &format!("\"{}\" {event}", script.display()),
                Path::new("/nonexistent"),
                home,
            )
        };

        let home = shim_dir("ketch", false);
        fs::create_dir_all(home.join(".ketch/bin")).unwrap();
        fs::rename(home.join("rtok"), home.join(".ketch/bin/rtok")).unwrap();
        assert_eq!(
            launch(&home, "Stop"),
            (true, "hook Stop --host codebuddy\n".into())
        );
        let _ = fs::remove_dir_all(&home);

        // Another install location on the machine would make "missing" untestable.
        if ["/usr/local/bin/rtok", "/opt/homebrew/bin/rtok"]
            .iter()
            .any(|p| Path::new(p).exists())
        {
            return;
        }
        let empty = std::env::temp_dir().join(format!("rtok-t560-empty-{}", std::process::id()));
        fs::create_dir_all(&empty).unwrap();
        assert_eq!(launch(&empty, "PreToolUse"), (true, String::new()));
        let (ok, start) = launch(&empty, "SessionStart");
        assert!(ok);
        assert!(start.contains("ketch install pyrlyn/rtok"), "{start}");
        assert!(start.contains("additionalContext"), "{start}");
        let _ = fs::remove_dir_all(&empty);
    }
}
