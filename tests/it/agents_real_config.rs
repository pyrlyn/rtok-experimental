// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T78: the installers against foreign configs that live only under the throwaway home.
//!
//! Each test writes a fixture (another tool's hook, server, or setting — never this
//! machine's files) into `tmp()`, runs `agents install <host> --yes` and then
//! `agents remove <host>`, and checks that every entry rtok does not own is still
//! there, unchanged, after both — and that a second install changes nothing a first
//! one did not.

use crate::it::common;

use common::agents::{raw, tmp, write_cfg};
use serde_json::Value;
use std::fs;
use std::path::Path;

/// A host and the home-relative configs it writes, spelled as they sit under a real `$HOME`.
struct Host {
    id: &'static str,
    files: &'static [&'static str],
}

const HOSTS: &[Host] = &[
    Host {
        id: "cursor",
        files: &[".cursor/hooks.json", ".cursor/mcp.json"],
    },
    Host {
        id: "claude",
        files: &[".claude/settings.json"],
    },
    Host {
        id: "codex",
        files: &[".codex/config.toml"],
    },
    Host {
        id: "opencode",
        files: &[".config/opencode/opencode.json"],
    },
    Host {
        id: "kilo",
        files: &[".config/kilo/kilo.json"],
    },
    Host {
        id: "kimi",
        files: &[".kimi-code/config.toml"],
    },
    Host {
        id: "grok",
        files: &[".grok/config.toml"],
    },
    // rtok owns `hooks/rtok.json` outright (nothing foreign ever lands there); the file
    // shared with a user's own servers is `mcp-config.json`.
    Host {
        id: "copilot",
        files: &[".copilot/mcp-config.json"],
    },
    Host {
        id: "aider",
        files: &[".aider.conf.yml"],
    },
    Host {
        id: "zed",
        files: &[".config/zed/settings.json"],
    },
    Host {
        id: "windsurf",
        files: &[".codeium/windsurf/mcp_config.json"],
    },
    Host {
        id: "zcode",
        files: &[".zcode/cli/config.json"],
    },
    Host {
        id: "vscode",
        files: &["Library/Application Support/Code/User/settings.json"],
    },
    Host {
        id: "gemini",
        files: &[".gemini/settings.json"],
    },
    Host {
        id: "codewhale",
        files: &[".codewhale/config.toml", ".codewhale/mcp.json"],
    },
    Host {
        id: "mimo",
        files: &[".config/mimocode/mimocode.json"],
    },
    Host {
        id: "omp",
        files: &[".omp/agent/mcp.json"],
    },
    Host {
        id: "devin",
        files: &[".config/devin/config.json", ".config/devin/mcp_config.json"],
    },
    Host {
        id: "roo",
        files: &[
            "Library/Application Support/Code/User/globalStorage/rooveterinaryinc.roo-cline/settings/mcp_settings.json",
        ],
    },
    Host {
        id: "junie",
        files: &[".junie/mcp/mcp.json"],
    },
    Host {
        id: "qwen",
        files: &[".qwen/settings.json"],
    },
    Host {
        id: "goose",
        files: &[".config/goose/config.yaml"],
    },
    Host {
        id: "droid",
        files: &[".factory/mcp.json"],
    },
    Host {
        id: "kiro",
        files: &[".kiro/settings/mcp.json"],
    },
    Host {
        id: "amp",
        files: &[".config/amp/settings.json"],
    },
    Host {
        id: "warp",
        files: &[".warp/.mcp.json"],
    },
    Host {
        id: "continue",
        files: &[".continue/config.yaml"],
    },
    Host {
        id: "crush",
        files: &[".config/crush/crush.json"],
    },
    Host {
        id: "openhands",
        files: &[".openhands/mcp.json"],
    },
    Host {
        id: "augment",
        files: &[".augment/settings.json"],
    },
    Host {
        id: "reasonix",
        files: &[".reasonix/config.toml"],
    },
];

/// True for anything rtok owns: install puts it there and remove takes it away, so it is the
/// one thing this test must not hold on to. Matched on the serialized value, which catches a
/// server entry, a hook command and a proxy URL alike.
fn ours(v: &Value) -> bool {
    v.to_string().to_ascii_lowercase().contains("rtok")
}

/// Every foreign leaf `before` carried, still in `after` under the same path and with the same
/// value. Object members are matched by key; array elements by containment, because rtok
/// appends its own hook entries and the indices of foreign ones shift under them.
fn json_survives(
    before: &Value,
    after: &Value,
    at: &str,
    checked: &mut usize,
    lost: &mut Vec<String>,
) {
    match (before, after) {
        (Value::Object(b), Value::Object(a)) => {
            for (k, v) in b {
                if k.to_ascii_lowercase().contains("rtok") || ours(v) {
                    continue;
                }
                match a.get(k) {
                    Some(av) => json_survives(v, av, &format!("{at}/{k}"), checked, lost),
                    None => {
                        *checked += 1;
                        lost.push(format!("{at}/{k}"));
                    }
                }
            }
        }
        (Value::Array(b), Value::Array(a)) => {
            for (i, v) in b.iter().enumerate() {
                if ours(v) {
                    continue;
                }
                *checked += 1;
                if !a.contains(v) {
                    lost.push(format!("{at}[{i}]"));
                }
            }
        }
        _ => {
            *checked += 1;
            if before != after {
                lost.push(at.to_string());
            }
        }
    }
}

/// The same claim for a file that is not JSON (TOML, or JSON with comments): every foreign
/// line still present, indentation ignored. Weaker than the structural check and uniform
/// across formats, which is what a fallback has to be.
fn lines_survive(before: &str, after: &str, checked: &mut usize, lost: &mut Vec<String>) {
    let kept: Vec<&str> = after.lines().map(str::trim).collect();
    for line in before.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.to_ascii_lowercase().contains("rtok") {
            continue;
        }
        *checked += 1;
        if !kept.contains(&line) {
            lost.push(line.to_string());
        }
    }
}

/// `before` must survive into `after`; returns how many entries were actually compared, so a
/// caller can refuse a vacuous pass.
fn survives(before: &str, after: &str, stage: &str, path: &Path) -> usize {
    let (mut checked, mut lost) = (0usize, Vec::new());
    match (
        serde_json::from_str::<Value>(before),
        serde_json::from_str::<Value>(after),
    ) {
        (Ok(b), Ok(a)) => json_survives(&b, &a, "", &mut checked, &mut lost),
        (Ok(_), Err(e)) => panic!(
            "{} is no longer valid JSON after {stage}: {e}",
            path.display()
        ),
        _ => lines_survive(before, after, &mut checked, &mut lost),
    }
    assert!(
        lost.is_empty(),
        "{stage} dropped or rewrote {} foreign entries in {}: {lost:#?}",
        lost.len(),
        path.display()
    );
    checked
}

/// The bytes written under the throwaway home. Never read from the process `HOME`.
fn fixture(rel: &str) -> &'static str {
    match rel {
        ".cursor/hooks.json" => {
            r#"{"version":1,"hooks":{"beforeShellExecution":[{"command":"foreign"}]}}"#
        }
        ".cursor/mcp.json"
        | ".copilot/mcp-config.json"
        | ".codeium/windsurf/mcp_config.json"
        | ".config/devin/mcp_config.json"
        | ".omp/agent/mcp.json"
        | ".factory/mcp.json"
        | ".kiro/settings/mcp.json"
        | ".junie/mcp/mcp.json"
        | ".warp/.mcp.json"
        | ".openhands/mcp.json"
        | "Library/Application Support/Code/User/globalStorage/rooveterinaryinc.roo-cline/settings/mcp_settings.json" => {
            r#"{"mcpServers":{"foreign":{"command":"x"}}}"#
        }
        ".config/amp/settings.json" => r#"{"amp.mcpServers":{"foreign":{"command":"x"}}}"#,
        ".config/crush/crush.json" => {
            r#"{"$schema":"https://charm.land/crush.json","options":{"debug":true},"mcp":{"foreign":{"type":"http","url":"https://x"}}}"#
        }
        ".augment/settings.json" => {
            r#"{"mcpServers":{"foreign":{"command":"x"}},"hooks":{"Stop":[{"hooks":[{"type":"command","command":"/x/stop.sh"}]}]}}"#
        }
        ".claude/settings.json" => {
            r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"other-tool run"}]}]},"env":{"KEEP":"1"}}"#
        }
        ".codex/config.toml" | ".grok/config.toml" => "[mcp_servers.foreign]\ncommand = \"x\"\n",
        ".reasonix/config.toml" => "# keep\n[[plugins]]\nname = \"foreign\"\ncommand = \"x\"\n",
        ".config/opencode/opencode.json"
        | ".config/kilo/kilo.json"
        | ".config/mimocode/mimocode.json"
        | ".gemini/settings.json" => r#"{"env":{"KEEP":"1"}}"#,
        ".qwen/settings.json" => {
            r#"{"hooks":{"Notification":[{"hooks":[{"type":"command","command":"echo other"}]}]},"mcpServers":{"foreign":{"command":"x"}},"env":{"KEEP":"1"}}"#
        }
        ".kimi-code/config.toml" => "[[hooks]]\nevent = \"Stop\"\ncommand = \"echo other\"\n",
        ".aider.conf.yml" => "model: foreign\n",
        ".config/goose/config.yaml" => {
            "# KEEP: foreign\nGOOSE_PROVIDER: openai\nextensions:\n  developer:\n    type: builtin\n    name: developer\n    enabled: true\nGOOSE_MODE: auto\n"
        }
        ".continue/config.yaml" => {
            "# KEEP: foreign\nname: Mine\nversion: 1.0.0\nschema: v1\nmodels:\n  - name: other\n    provider: ollama\n    model: llama\nmcpServers:\n  - name: foreign\n    command: npx\n    args:\n      - x\nrules:\n  - be brief\n"
        }
        ".config/zed/settings.json" => {
            // KEEP is last and has no trailing comma: install inserts `,\n  context_servers`
            // at the root brace (KEEP's line unchanged), and remove takes that comma with
            // the key (excise prefers the preceding separator) so KEEP still matches.
            "{\n  // foreign\n  \"theme\": \"One Dark\",\n  \"KEEP\": \"1\"\n}\n"
        }
        ".zcode/cli/config.json" => {
            r#"{"hooks":{"events":{"Stop":[{"hooks":[{"type":"command","command":"echo other"}]}]}},"mcp":{"servers":{"foreign":{"command":"x"}}}}"#
        }
        "Library/Application Support/Code/User/settings.json" => {
            r#"{"editor.tabSize": 2, "KEEP": "1"}"#
        }
        ".codewhale/config.toml" => "[other]\nname = \"foreign\"\n",
        ".codewhale/mcp.json" => r#"{"mcpServers":{"foreign":{"command":"x"}}}"#,
        ".config/devin/config.json" => r#"{"hooks":{"Stop":[{"command":"foreign"}]}}"#,
        other => panic!("no fixture for {other}"),
    }
}

/// Seed one host from [`fixture`], install, install again, remove — and hold every foreign
/// entry to its seeded value at each stage.
fn round_trip(host: &Host) {
    let home = tmp(&format!("real-{}", host.id));
    let cfg = write_cfg(&home);
    let seeded: Vec<_> = host
        .files
        .iter()
        .map(|rel| {
            let dest = home.join(rel);
            fs::create_dir_all(dest.parent().expect("rel has a parent")).unwrap();
            let body = fixture(rel);
            fs::write(&dest, body).unwrap();
            (*rel, body.to_string(), dest)
        })
        .collect();

    let install = ["agents", "install", host.id, "--yes"];
    let out = raw(&install, &cfg, &home);
    assert!(
        out.status.success(),
        "install {} failed: {}",
        host.id,
        String::from_utf8_lossy(&out.stderr)
    );

    let mut compared = 0;
    let after_install: Vec<String> = seeded
        .iter()
        .map(|(_, before, dest)| {
            let after = fs::read_to_string(dest).unwrap();
            compared += survives(before, &after, "install", dest);
            after
        })
        .collect();
    assert!(
        compared > 0,
        "{}: fixture had nothing foreign for the round trip to protect",
        host.id
    );

    // A second install leaves the exact bytes the first one produced.
    assert!(raw(&install, &cfg, &home).status.success());
    for ((rel, _, dest), once) in seeded.iter().zip(&after_install) {
        assert_eq!(
            &fs::read_to_string(dest).unwrap(),
            once,
            "{}: second install rewrote {rel}",
            host.id
        );
    }

    assert!(
        raw(&["agents", "remove", host.id], &cfg, &home)
            .status
            .success()
    );
    for (_, before, dest) in &seeded {
        survives(before, &fs::read_to_string(dest).unwrap(), "remove", dest);
    }
    let _ = fs::remove_dir_all(&home);
}

fn by_id(id: &str) -> &'static Host {
    HOSTS.iter().find(|h| h.id == id).expect("host in HOSTS")
}

macro_rules! real_config_round_trip {
    ($($name:ident => $id:literal),* $(,)?) => {
        $(#[test] fn $name() { round_trip(by_id($id)); })*
    };
}

real_config_round_trip! {
    cursor_keeps_the_real_hooks_and_mcp_json => "cursor",
    claude_keeps_the_real_settings_json => "claude",
    codex_keeps_the_real_config_toml => "codex",
    opencode_keeps_the_real_opencode_json => "opencode",
    kilo_keeps_the_real_kilo_json => "kilo",
    kimi_keeps_the_real_config_toml => "kimi",
    grok_keeps_the_real_config_toml => "grok",
    copilot_keeps_the_real_mcp_config_json => "copilot",
    aider_keeps_the_real_conf_yml => "aider",
    windsurf_keeps_the_real_mcp_config_json => "windsurf",
    zcode_keeps_the_real_config_json => "zcode",
    vscode_keeps_the_real_settings_json => "vscode",
    gemini_keeps_the_real_settings_json => "gemini",
    codewhale_keeps_the_real_config_toml_and_mcp_json => "codewhale",
    mimo_keeps_the_real_mimocode_json => "mimo",
    omp_keeps_the_real_mcp_json => "omp",
    devin_keeps_the_real_config_and_mcp_json => "devin",
    roo_keeps_the_real_mcp_settings_json => "roo",
    qwen_keeps_the_real_settings_json => "qwen",
    goose_keeps_the_real_config_yaml => "goose",
    droid_keeps_the_real_mcp_json => "droid",
    kiro_keeps_the_real_mcp_json => "kiro",
    junie_keeps_the_real_mcp_json => "junie",
    amp_keeps_the_real_mcp_json => "amp",
    warp_keeps_the_real_mcp_json => "warp",
    continue_keeps_the_real_config_yaml => "continue",
    crush_keeps_the_real_crush_json => "crush",
    openhands_keeps_the_real_mcp_json => "openhands",
    augment_keeps_the_real_settings_json => "augment",
    reasonix_keeps_the_real_config_toml => "reasonix",
}

/// Zed writes JSONC. The fixture carries a `//` comment; the installer edits the text
/// surgically, and that comment plus the foreign keys survive install and remove.
#[test]
fn zed_keeps_the_real_settings_json() {
    round_trip(by_id("zed"));
}

/// Every HOSTS path has a fixture arm, and every fixture is foreign-only: no `rtok`, and at
/// least one of `foreign` / `KEEP` / `other`. Reads only the fixture strings — never HOME.
#[test]
fn every_host_fixture_is_foreign_only() {
    for rel in HOSTS.iter().flat_map(|h| h.files) {
        let body = fixture(rel);
        assert!(
            !body.to_ascii_lowercase().contains("rtok"),
            "{rel}: fixture must not mention rtok"
        );
        let lower = body.to_ascii_lowercase();
        assert!(
            lower.contains("foreign") || body.contains("KEEP") || lower.contains("other"),
            "{rel}: fixture must carry foreign/KEEP/other marker"
        );
    }
}
