// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T47.3 + D21: the OpenCode host plugin is one bash filter beside one `mcp.rtok`, offered by
//! `rtok agents install opencode` and, since OpenCode's own docs have no GitHub/subdir
//! install, linked by default once OpenCode itself is detected (T164).
//!
//! Check: the plugin registers no tool (MCP owns read/search/memory/graph) and names ketch;
//! `--dry-run` offers `plugins/opencode/rtok.ts` at `<config dir>/plugins/rtok.ts` and writes
//! nothing; a plain install links the one file and writes `mcp.rtok`, a second run is
//! `already installed`, and remove unlinks it and drops `mcp.rtok`. The plugin's own unit
//! test (`plugins/opencode/rtok.test.ts`) runs from `tests/filter.rs`.

use crate::common;

use common::agents::{json, rtok, slash, tmp, write_cfg};
use std::fs;
use std::path::PathBuf;

fn src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins/opencode/rtok.ts")
}

#[test]
fn d21_plugin_is_one_bash_filter_without_tools() {
    let ts = fs::read_to_string(src()).unwrap();
    assert!(ts.contains("\"tool.execute.after\""), "filters tool output");
    assert!(ts.contains("\"bash\""), "scoped to bash");
    assert!(ts.contains("\"filter\""), "through `rtok filter`");
    assert!(ts.contains("ketch install pyrlyn/rtok"), "ketch hint");
    assert!(ts.contains("tool.execute.before"), "T70.5 guard check");
    assert!(ts.contains("guard"), "through rtok guard check");
    for dup in [
        "tool: {", // OpenCode custom tools
        "\"mcp\"",
        "rtok read",
        "rtok search",
    ] {
        assert!(!ts.contains(dup), "no second call path: {dup}");
    }
}

#[test]
fn dry_run_offers_the_plugin_and_writes_nothing() {
    let home = tmp("opencode-dry");
    let cfg = write_cfg(&home);
    let dest = home.join(".config/opencode/plugins/rtok.ts");
    let out = rtok(
        &[
            "agents",
            "install",
            "opencode",
            "--cli",
            "--yes",
            "--dry-run",
        ],
        &cfg,
        &home,
    );
    assert!(out.contains("offer plugins/opencode/rtok.ts →"), "{out}");
    assert!(
        slash(&out).contains(&slash(dest.display().to_string())),
        "{out}"
    );
    assert!(out.contains("ketch install pyrlyn/rtok"), "{out}");
    assert!(dest.symlink_metadata().is_err(), "dry-run must not link");
    assert!(!home.join(".config/opencode/opencode.json").exists());
}

#[test]
fn install_without_yes_links_the_plugin_by_default() {
    let home = tmp("opencode-offer");
    let cfg = write_cfg(&home);
    let dest = home.join(".config/opencode/plugins/rtok.ts");
    let out = rtok(&["agents", "install", "opencode", "--cli"], &cfg, &home);
    assert!(out.contains("+ plugin plugins/opencode/rtok.ts →"), "{out}");
    assert!(out.contains("✓ plugin  installed"), "{out}");
    let conf = json(&home.join(".config/opencode/opencode.json"));
    assert_eq!(conf["mcp"]["rtok"]["command"][1], "mcp", "{conf}");
    assert!(dest.symlink_metadata().is_ok(), "linked without --yes");
}

#[test]
fn plain_install_links_one_file_then_remove_unlinks_it() {
    let home = tmp("opencode-yes");
    let cfg = write_cfg(&home);
    let conf = home.join(".config/opencode/opencode.json");
    fs::write(&conf, r#"{"mcp":{"foreign":{"type":"remote","url":"x"}}}"#).unwrap();
    let dest = home.join(".config/opencode/plugins/rtok.ts");
    let args = ["agents", "install", "opencode", "--cli"];

    let out = rtok(&args, &cfg, &home);
    assert!(out.contains("✓ plugin  installed"), "{out}");
    assert!(
        dest.symlink_metadata().is_ok(),
        "linked: {}",
        dest.display()
    );
    assert_eq!(fs::read(&dest).unwrap(), fs::read(src()).unwrap());
    let plugins: Vec<_> = fs::read_dir(dest.parent().unwrap()).unwrap().collect();
    assert_eq!(plugins.len(), 1, "one file in the plugins dir");
    let servers = json(&conf)["mcp"].clone();
    assert!(
        servers["rtok"].is_object() && servers["foreign"].is_object(),
        "{servers}"
    );

    let again = rtok(&args, &cfg, &home);
    assert!(again.contains("already installed"), "{again}");

    let rm = rtok(&["agents", "remove", "opencode"], &cfg, &home);
    assert!(rm.contains("- plugin"), "{rm}");
    assert!(dest.symlink_metadata().is_err(), "unlinked");
    let servers = json(&conf)["mcp"].clone();
    assert!(
        servers["rtok"].is_null() && servers["foreign"].is_object(),
        "{servers}"
    );
    assert!(src().is_file(), "remove never touches the source");
}
