// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T289.3: `rtok agents install|uninstall <host> --project` adds rtok's post-create entry to the
//! project's Cursor, Kilo and Devin/Windsurf files and takes it out again, changing nothing else.

use crate::common;

use common::agents::{bin, tmp, write_cfg};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A scratch project (a directory with `.git`) and the config that points the binary at it.
struct Project {
    root: PathBuf,
    home: PathBuf,
    cfg: PathBuf,
}

fn project(name: &str) -> Project {
    let home = tmp(name);
    let cfg = write_cfg(&home);
    let root = home.join("proj");
    fs::create_dir_all(root.join(".git")).unwrap();
    Project { root, home, cfg }
}

impl Project {
    fn rtok(&self, verb: &str, host: &str, extra: &[&str]) -> (bool, String) {
        let out = Command::new(bin())
            .current_dir(&self.root)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .arg("--config")
            .arg(&self.cfg)
            .args(["agents", verb, host, "--project"])
            .args(extra)
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.success(), text)
    }

    fn install(&self, host: &str) -> String {
        let (ok, text) = self.rtok("install", host, &[]);
        assert!(ok, "{text}");
        text
    }

    fn uninstall(&self, host: &str) -> String {
        let (ok, text) = self.rtok("uninstall", host, &[]);
        assert!(ok, "{text}");
        text
    }

    fn write(&self, rel: &str, body: &str) {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.root.join(rel)).unwrap()
    }

    fn exists(&self, rel: &str) -> bool {
        Path::new(&self.root.join(rel)).exists()
    }
}

fn json(raw: &str) -> serde_json::Value {
    rtok::agents::jsonc::parse(raw).unwrap()
}

const CURSOR_FOREIGN: &str = "{\n  // keep this comment\n  \"setup-worktree-unix\": [\"npm ci\", \"echo hi\"],\n  \"other\": {\"a\": 1,},\n  \"setup-worktree-windows\": \"setup.ps1\"\n}\n";

#[test]
fn cursor_adds_to_the_command_lists_only_and_removal_restores_the_file() {
    let p = project("pc-cursor");
    p.write(".cursor/worktrees.json", CURSOR_FOREIGN);

    let text = p.install("cursor");
    assert!(text.contains(".cursor/worktrees.json: added"), "{text}");
    assert!(
        text.contains("setup-worktree-windows runs a script"),
        "{text}"
    );
    let got = p.read(".cursor/worktrees.json");
    assert!(
        got.starts_with(
            "{\n  // keep this comment\n  \"setup-worktree-unix\": [\"npm ci\", \"echo hi\", \""
        ),
        "{got}"
    );
    let doc = json(&got);
    let unix = doc["setup-worktree-unix"].as_array().unwrap();
    assert_eq!(unix.len(), 3);
    assert!(unix[2].as_str().unwrap().contains("worktree adopt"));
    assert_eq!(doc["setup-worktree-windows"], "setup.ps1");
    assert!(doc.get("setup-worktree").is_none());

    assert!(p.install("cursor").contains("no changes"));
    assert_eq!(p.read(".cursor/worktrees.json"), got);

    assert!(p.uninstall("cursor").contains("removed"));
    assert_eq!(p.read(".cursor/worktrees.json"), CURSOR_FOREIGN);
    assert!(p.uninstall("cursor").contains("no changes"));
}

#[test]
fn cursor_creates_the_generic_key_and_removal_deletes_the_file_it_made() {
    let p = project("pc-cursor-new");
    p.install("cursor");
    let doc = json(&p.read(".cursor/worktrees.json"));
    assert_eq!(doc["setup-worktree"].as_array().unwrap().len(), 1);
    assert!(
        !p.exists(".cursor/.backup_configs"),
        "project files get no backup folder"
    );

    p.uninstall("cursor");
    assert!(!p.exists(".cursor/worktrees.json"));
}

#[test]
fn cursor_with_only_a_script_changes_nothing_and_says_how() {
    let p = project("pc-cursor-script");
    let body = "{\"setup-worktree\": \"setup.sh\"}\n";
    p.write(".cursor/worktrees.json", body);
    let text = p.install("cursor");
    assert!(
        text.contains("no changes") && text.contains("add `"),
        "{text}"
    );
    assert_eq!(p.read(".cursor/worktrees.json"), body);
}

#[test]
fn windsurf_and_devin_share_one_entry_in_the_file_the_host_reads() {
    let p = project("pc-devin");
    let legacy =
        "{\n  \"hooks\": {\n    \"pre_run_command\": [{\"command\": \"echo pre\"}]\n  }\n}\n";
    p.write(".windsurf/hooks.json", legacy);

    let text = p.install("windsurf,devin");
    assert!(text.contains(".windsurf/hooks.json: added"), "{text}");
    assert!(
        !p.exists(".devin/hooks.json"),
        "a new .devin file would switch the legacy hooks off"
    );
    let doc = json(&p.read(".windsurf/hooks.json"));
    assert_eq!(doc["hooks"]["pre_run_command"][0]["command"], "echo pre");
    let post = doc["hooks"]["post_setup_worktree"].as_array().unwrap();
    assert_eq!(post.len(), 1);
    assert!(
        post[0]["command"]
            .as_str()
            .unwrap()
            .contains("worktree adopt")
    );

    p.uninstall("windsurf,devin");
    assert_eq!(p.read(".windsurf/hooks.json"), legacy);
}

#[test]
fn devin_joins_a_foreign_post_setup_list_and_a_fresh_file_goes_away_on_removal() {
    let p = project("pc-devin-list");
    let body = "{\n  \"hooks\": {\n    \"post_setup_worktree\": [\n      {\"command\": \"bash setup.sh\", \"show_output\": true}\n    ]\n  }\n}\n";
    p.write(".devin/hooks.json", body);
    p.install("devin");
    let doc = json(&p.read(".devin/hooks.json"));
    assert_eq!(
        doc["hooks"]["post_setup_worktree"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        doc["hooks"]["post_setup_worktree"][0]["command"],
        "bash setup.sh"
    );
    p.uninstall("devin");
    assert_eq!(p.read(".devin/hooks.json"), body);

    let fresh = project("pc-devin-fresh");
    fresh.install("devin");
    assert!(json(&fresh.read(".devin/hooks.json"))["hooks"]["post_setup_worktree"].is_array());
    fresh.uninstall("devin");
    assert!(!fresh.exists(".devin/hooks.json"));
}

/// T627: a comment between rtok's entry and its comma must not leave the file unparseable;
/// the user's comment survives and only our entry (and one comma) goes.
const OURS: &str = "( command -v rtok worktree adopt ) || true";

#[test]
fn cursor_removal_keeps_a_valid_file_when_a_comment_follows_our_entry() {
    let p = project("pc-cursor-comment");
    p.write(
        ".cursor/worktrees.json",
        &format!("{{\"setup-worktree\": [\"{OURS}\" /*keep*/, \"npm ci\"]}}\n"),
    );
    assert!(p.uninstall("cursor").contains("removed"));
    let got = p.read(".cursor/worktrees.json");
    assert_eq!(got, "{\"setup-worktree\": [ /*keep*/ \"npm ci\"]}\n");
    assert_eq!(json(&got)["setup-worktree"], serde_json::json!(["npm ci"]));
}

#[test]
fn devin_removal_keeps_a_valid_file_when_a_comment_follows_our_entry() {
    let p = project("pc-devin-comment");
    p.write(
        ".devin/hooks.json",
        &format!(
            "{{\"hooks\": {{\"post_setup_worktree\": [{{\"command\": \"{OURS}\"}} // keep\n, {{\"command\": \"x\"}}]}}}}\n"
        ),
    );
    assert!(p.uninstall("devin").contains("removed"));
    let got = p.read(".devin/hooks.json");
    assert!(got.contains("// keep"), "{got}");
    assert_eq!(
        json(&got)["hooks"]["post_setup_worktree"],
        serde_json::json!([{"command": "x"}])
    );
}

#[test]
fn kilo_puts_a_marked_block_after_the_shebang_and_removal_restores_the_script() {
    let p = project("pc-kilo");
    let script = "#!/bin/bash\nset -e\nnpm ci\n";
    p.write(".kilo/setup-script", script);

    let text = p.install("kilo");
    assert!(text.contains(".kilo/setup-script: added"), "{text}");
    let got = p.read(".kilo/setup-script");
    assert!(
        got.starts_with("#!/bin/bash\n# >>> rtok worktree adopt"),
        "{got}"
    );
    assert!(
        got.ends_with("# <<< rtok worktree adopt <<<\nset -e\nnpm ci\n"),
        "{got}"
    );
    assert!(p.install("kilo").contains("no changes"));
    assert_eq!(p.read(".kilo/setup-script"), got);

    p.uninstall("kilo");
    assert_eq!(p.read(".kilo/setup-script"), script);
}

#[test]
fn kilo_edits_the_sh_script_it_would_otherwise_shadow_and_deletes_a_script_it_created() {
    let p = project("pc-kilo-sh");
    p.write(".kilo/setup-script.sh", "npm ci\n");
    p.install("kilo");
    assert!(
        !p.exists(".kilo/setup-script"),
        "setup-script would shadow setup-script.sh"
    );
    assert!(
        p.read(".kilo/setup-script.sh")
            .ends_with("# <<< rtok worktree adopt <<<\nnpm ci\n")
    );
    p.uninstall("kilo");
    assert_eq!(p.read(".kilo/setup-script.sh"), "npm ci\n");

    let fresh = project("pc-kilo-new");
    fresh.install("kilo");
    assert!(
        fresh
            .read(".kilo/setup-script")
            .starts_with("#!/bin/sh\n# >>>")
    );
    fresh.uninstall("kilo");
    assert!(!fresh.exists(".kilo/setup-script"));
}

#[test]
fn a_dry_run_writes_nothing_and_another_host_is_refused() {
    let p = project("pc-dry");
    let (ok, text) = p.rtok("install", "cursor,kilo,devin", &["--dry-run"]);
    assert!(ok, "{text}");
    assert!(text.contains("would add"), "{text}");
    assert!(!p.exists(".cursor") && !p.exists(".kilo") && !p.exists(".devin"));

    let (ok, text) = p.rtok("install", "claude", &[]);
    assert!(!ok && text.contains("no post-create script"), "{text}");
}

/// Fail open: a refused `adopt` (no single live agent) must not fail the host's worktree setup.
#[cfg(unix)]
#[test]
fn the_written_command_exits_zero_when_adopt_fails() {
    use std::os::unix::fs::PermissionsExt;
    let p = project("pc-fail-open");
    p.install("devin");
    let hooks = json(&p.read(".devin/hooks.json"));
    let cmd = hooks["hooks"]["post_setup_worktree"][0]["command"]
        .as_str()
        .unwrap()
        .to_string();
    let fake = p.home.join("fakebin");
    fs::create_dir_all(&fake).unwrap();
    let marker = p.home.join("adopt-ran");
    let rtok = fake.join("rtok");
    fs::write(
        &rtok,
        format!("#!/bin/sh\necho \"$@\" > '{}'\nexit 3\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&rtok, fs::Permissions::from_mode(0o755)).unwrap();
    let out = Command::new("sh")
        .args(["-c", &cmd])
        .env("PATH", format!("{}:/usr/bin:/bin", fake.display()))
        .env("HOME", &p.home)
        .output()
        .unwrap();
    assert!(out.status.success(), "{cmd}: {out:?}");
    assert_eq!(
        fs::read_to_string(&marker).unwrap().trim(),
        "worktree adopt"
    );
}
