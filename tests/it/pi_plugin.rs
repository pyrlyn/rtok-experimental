// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T10.6 + D21: pi host plugin is one bash call path, no MCP, ketch if missing.
//!
//! Check: `rtok agents install pi --dry-run` names `plugins/pi` and
//! `ketch install pyrlyn/rtok` and touches nothing; `--yes` links the
//! extension, second apply is `no changes`, `--remove` unlinks; the TS
//! extension owns the single bash call path with no `read`/`search`
//! duplication; pi's own loader loads the linked directory once (T48.1).

use crate::common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins/pi")
}

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rtok")
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rtok-t106-{name}-{}-{}",
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
    let cfg = home.join("config.toml");
    fs::write(
        &cfg,
        format!(
            "[setup.pi]\nextensions_path = \"{}/extensions\"\n",
            // `/`: a `\` in a TOML basic string starts an escape (T83.4).
            home.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    cfg
}

/// Every call here is `agents install pi …`; `--no-restart` (T141) keeps the test from ever
/// probing or touching a real pi process on the machine running it.
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
fn pi_package_is_extension_and_skill_without_tools() {
    let dir = root();
    let pkg: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("package.json")).unwrap()).unwrap();
    // `pi.extensions` is also the key oh my pi's loader falls back to — `rtok agents install
    // omp` links this directory as is (T92), so renaming it breaks two hosts.
    let exts = pkg["pi"]["extensions"].as_array().expect("pi.extensions");
    assert_eq!(exts.len(), 1, "one extension entry");
    assert!(exts[0].as_str().unwrap().ends_with("rtok.ts"), "{exts:?}");
    assert!(dir.join("extensions/rtok.ts").is_file());
}

#[test]
fn pi_extension_owns_the_single_bash_call_path() {
    let ts = fs::read_to_string(root().join("extensions/rtok.ts")).unwrap();
    assert!(ts.contains("\"tool_call\""), "rewrites bash calls");
    assert!(ts.contains("\"bash\""), "scoped to bash");
    assert!(ts.contains("rtok run --"), "rewrite target");
    assert!(ts.contains("\"tool_result\""), "compresses results");
    assert!(ts.contains("rtok filter"), "filter path");
    assert!(ts.contains("expand"), "expand trailer");
    assert!(ts.contains("ketch install pyrlyn/rtok"), "ketch hint");
    assert!(ts.contains("\"read\""), "filters pi read results (T70.1)");
    assert!(ts.contains("\"grep\""), "filters pi grep results");
    assert!(ts.contains("\"find\""), "filters pi find results");
    assert!(ts.contains("\"ls\""), "filters pi ls results");
    assert!(ts.contains("--cmd"), "file/search tools pass --cmd");
    assert!(ts.contains("guard"), "T70.5 guard check");
    assert!(ts.contains("block: true"), "deny returns a reasoned block");
    assert!(ts.contains("registerTool"), "T70.3 pi.registerTool");
    assert!(ts.contains("mcp --call"), "thin CLI shim, one call path");
    for dup in ["rtok read", "rtok search"] {
        assert!(!ts.contains(dup), "no duplicate call path: {dup}");
    }
}

/// T47.3: the extension's own unit test (`plugins/pi/tests/rtok.test.ts`) — rewrite, quoting,
/// fail-open with the ketch hint, and the filter result — against a fake `rtok` on PATH.
#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "host plugin tests run on Linux only"
)]
fn pi_extension_unit_test_with_fake_rtok() {
    common::vitest("plugins/pi/tests/rtok.test.ts", &[]);
}

/// T380: pi has no bundled skill. Install copies `rtok-worktrees` and drops a marked legacy
/// `worktrees` the same way every other host does.
#[test]
fn pi_install_ships_the_prefixed_skill_and_drops_a_marked_legacy_copy() {
    let home = tmp("skills");
    let cfg = write_cfg(&home);
    let skills = home.join("skills");
    let legacy = skills.join("worktrees");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("SKILL.md"), "old skill\n").unwrap();
    fs::write(legacy.join(".rtok-owned"), "").unwrap();
    let (stdout, stderr, code) = setup(&["agents", "install", "pi", "--yes"], &cfg, &home);
    assert_eq!(code, 0, "stderr={stderr} stdout={stdout}");
    assert!(skills.join("rtok-worktrees/SKILL.md").is_file(), "{stdout}");
    assert!(skills.join("rtok/SKILL.md").is_file(), "{stdout}");
    assert!(!legacy.exists(), "{stdout}");
    assert!(
        stdout.lines().any(|l| {
            let l = l.replace('\\', "/");
            l.starts_with("- skill ") && l.ends_with("/skills/worktrees")
        }),
        "{stdout}"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn setup_pi_dry_run_offers_plugin() {
    let home = tmp("dry");
    let cfg = write_cfg(&home);
    let (stdout, stderr, code) = setup(&["agents", "install", "pi", "--dry-run"], &cfg, &home);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("plugins/pi"), "stdout={stdout}");
    assert!(
        stdout.contains("ketch install pyrlyn/rtok"),
        "stdout={stdout}"
    );
    assert!(
        !home.join("extensions/rtok").exists(),
        "dry-run must not link"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn setup_pi_yes_links_remove_unlinks() {
    let home = tmp("yes");
    let cfg = write_cfg(&home);
    let (stdout, stderr, code) = setup(&["agents", "install", "pi", "--yes"], &cfg, &home);
    assert_eq!(code, 0, "stderr={stderr} stdout={stdout}");
    let dest = home.join("extensions/rtok");
    let meta = fs::symlink_metadata(&dest).unwrap_or_else(|e| panic!("{}: {e}", dest.display()));
    assert!(meta.file_type().is_symlink() || dest.is_dir(), "{dest:?}");
    // T48.1: pi's own loader (skipped without pi) runs the extension from the linked dir.
    common::vitest(
        "plugins/pi/tests/load.test.ts",
        &[("RTOK_PI_AGENT_DIR", &home)],
    );
    let (again, stderr2, code2) = setup(&["agents", "install", "pi", "--yes"], &cfg, &home);
    assert_eq!(code2, 0, "stderr={stderr2}");
    assert!(again.contains("already installed"), "second apply: {again}");
    let (rm, stderr3, code3) = setup(&["agents", "install", "pi", "--remove"], &cfg, &home);
    assert_eq!(code3, 0, "stderr={stderr3}");
    assert!(!dest.exists(), "remove must unlink; stdout={rm}");
    let _ = fs::remove_dir_all(&home);
}
