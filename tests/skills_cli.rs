// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok skills` end to end: the real binary against a fake HOME, one run per agent.

#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;

fn rtok(home: &Path, project: &Path, args: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .args(["skills"])
        .args(args)
        .args(["--json", "--project"])
        .arg(project)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("RTOK_HOME", home.join(".rtok"))
        .env_remove("COPILOT_HOME")
        .env_remove("RTOK_AGENT")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn enable_disable_for_every_agent() {
    for (agent, user_dir) in [
        ("claude", ".claude/skills"),
        ("cursor", ".cursor/skills"),
        ("copilot", ".copilot/skills"),
    ] {
        let tmp =
            std::env::temp_dir().join(format!("rtok-skills-cli-{}-{agent}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let (home, project) = (tmp.join("home"), tmp.join("proj"));
        let skill = home.join(user_dir).join("demo");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: demo\ndescription: d\n---\n",
        )
        .unwrap();

        let listed = rtok(&home, &project, &["list", "--agent", agent]);
        assert_eq!(listed["skills"][0]["enabled"], true, "{agent}");
        assert_eq!(listed["skills"][0]["size_bytes"], 34, "{agent}");

        let off = rtok(&home, &project, &["disable", "demo", "--agent", agent]);
        assert_eq!(off["changed"], true);
        assert!(!skill.exists());
        assert!(
            home.join(format!("{user_dir}-disabled/demo/SKILL.md"))
                .is_file()
        );

        let on = rtok(&home, &project, &["enable", "demo", "--agent", agent]);
        assert_eq!(on["skill"]["enabled"], true);
        assert!(skill.join("SKILL.md").is_file());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
