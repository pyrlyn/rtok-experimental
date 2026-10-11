// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T44.4: the `rtok agents install` matrix over every host and both app kinds.
//!
//! Check: per host, a second setup takes no new backup and heads its block `already
//! installed`; a second remove says `no changes`; `--dry-run` creates nothing and copies
//! nothing; `agent` and `agents` print the same; `--cli` / `--desktop` pick the block; Claude
//! Desktop installs MCP with the absolute binary under a temp home; an unknown host is refused
//! before any backup; a missing `rtok` on PATH is a warning at the top of the output; `agents
//! list` shows each host's modules installed after install and none after remove.

mod common;

use common::agents::{backups, claude_desktop_config, json, raw, rtok, slash, tmp, write_cfg};
use std::fs;
use std::path::{Path, PathBuf};

/// `(host, extra setup flags, the file whose backups we count)`; pi links a directory and
/// edits no file, so it has nothing to back up. claude, cursor, opencode, kilo, mimo, pi and
/// zcode install their plugin by default (T139, T164); `--yes` is kept here only so
/// `already installed` needs the link regardless of a future default change.
fn hosts(home: &Path) -> Vec<(&'static str, Vec<&'static str>, Option<PathBuf>)> {
    vec![
        // Must run before claude: once claude has written `~/.claude.json`'s `mcpServers.rtok`,
        // grok's own MCP is covered by `[compat.claude]` importing that file, so grok's own
        // config gets no write on its first install (T275) — this test needs a real write.
        ("grok", vec!["--yes"], Some(home.join(".grok/config.toml"))),
        // T115: the plugin installs by default (fake `claude`), which then serves hooks, so
        // rtok itself writes no settings file there (D21). MCP is independent of the plugin
        // (T275): `~/.claude.json` still gets `mcpServers.rtok`, tracked here for backups.
        (
            "claude",
            vec!["--yes", "--cli"],
            Some(home.join(".claude.json")),
        ),
        // The linked plugin carries hooks only (D21, T244), so `hooks.json` stays unwritten;
        // MCP is independent of it (T275/D33) and `mcp.json` is written and backed up here.
        ("cursor", vec!["--yes"], Some(home.join(".cursor/mcp.json"))),
        ("codex", vec![], Some(home.join(".codex/config.toml"))),
        (
            "opencode",
            vec!["--yes"],
            Some(home.join(".config/opencode/opencode.json")),
        ),
        (
            "kilo",
            vec!["--yes"],
            Some(home.join(".config/kilo/kilo.json")),
        ),
        ("pi", vec!["--yes"], None),
        // omp (a pi fork) keeps `default_install: false` (T164 left it asking): `--yes`
        // is required, not just kept for symmetry, or the plugin offer never links.
        ("omp", vec!["--yes"], Some(home.join(".omp/agent/mcp.json"))),
        (
            "zcode",
            vec!["--yes"],
            Some(home.join(".zcode/cli/config.json")),
        ),
        (
            "kimi",
            vec!["--yes"],
            Some(home.join(".kimi-code/config.toml")),
        ),
        (
            "cline",
            vec!["--yes"],
            Some(home.join(".cline/data/settings/cline_mcp_settings.json")),
        ),
        (
            "copilot",
            vec![],
            Some(home.join(".copilot/hooks/rtok.json")),
        ),
        (
            "commandcode",
            vec!["--yes"],
            Some(home.join(".commandcode/settings.json")),
        ),
        ("aider", vec!["--proxy"], Some(home.join(".aider.conf.yml"))),
        (
            "windsurf",
            vec![],
            Some(home.join(".codeium/windsurf/mcp_config.json")),
        ),
        (
            // T117: the plugin needs `--yes` to link (no `default_install`, since it also
            // edits the user's own settings.json); `settings.json` (`chat.pluginLocations`)
            // is the file that stays written and backed up — its own `mcp.servers.rtok` entry
            // is independent of the plugin (T275/D33) and is written there too.
            "vscode",
            vec!["--yes"],
            Some(home.join("Library/Application Support/Code/User/settings.json")),
        ),
        ("zed", vec![], Some(home.join(".config/zed/settings.json"))),
        ("gemini", vec![], Some(home.join(".gemini/settings.json"))),
        (
            "codewhale",
            vec!["--yes"],
            Some(home.join(".codewhale/config.toml")),
        ),
        // T520: the plugin links by default, so `.config/mimocode/plugins/rtok.ts` is the
        // second thing install writes beside `mimocode.json`.
        (
            "mimo",
            vec!["--yes"],
            Some(home.join(".config/mimocode/mimocode.json")),
        ),
        // Desktop links the plugin on `--yes` (no `default_install`); nothing file-backed.
        ("antigravity", vec!["--yes"], None),
        ("devin", vec!["--yes"], Some(home.join("devin/config.json"))),
    ]
}

fn setup_args<'a>(host: &'a str, flags: &[&'a str]) -> Vec<&'a str> {
    let mut a = vec!["agents", "install", host];
    a.extend_from_slice(flags);
    a
}

#[test]
fn setup_twice_takes_one_backup_and_says_already_installed() {
    let home = tmp("twice");
    let cfg = write_cfg(&home);
    for (host, flags, file) in hosts(&home) {
        if let Some(f) = &file {
            fs::write(
                f,
                if f.extension().is_some_and(|e| e == "toml" || e == "yml") {
                    "# mine\n"
                } else {
                    "{}"
                },
            )
            .unwrap();
        }
        let first = rtok(&setup_args(host, &flags), &cfg, &home);
        assert!(!first.contains("already installed"), "{host}: {first}");
        assert!(!first.contains("did not read back"), "{host}: {first}");
        let second = rtok(&setup_args(host, &flags), &cfg, &home);
        assert!(second.contains("already installed"), "{host}: {second}");
        assert!(!second.contains("backup "), "{host}: {second}");
        if let Some(f) = &file {
            assert_eq!(
                backups(f).len(),
                1,
                "{host}: one copy, taken before the first write"
            );
            assert!(first.contains("backup "), "{host}: {first}");
            assert!(
                slash(&second).contains(&slash(f.display().to_string())),
                "{host}: {second}"
            );
        }
        // Both runs print the same module and plugin rows.
        let rows = |s: &str| -> Vec<String> {
            s.lines()
                .filter(|l| {
                    l.starts_with("  ✓")
                        || l.starts_with("  ✗")
                        || l.starts_with("  −")
                        || l.starts_with("    ")
                })
                .map(str::to_string)
                .collect()
        };
        assert_eq!(rows(&first), rows(&second), "{host}");
        assert!(first.contains("  plugins\n"), "{host}: {first}");
    }
}

/// A remove/install block printed a real edit (diff line or "N removed").
/// Sibling Desktop/CLI headers may still say `— no changes` while the other kind edits.
fn remove_made_edits(out: &str) -> bool {
    out.lines().any(|l| {
        let bare = strip_ansi(l.trim_start());
        bare.starts_with('-') || bare.starts_with('+') || bare.ends_with(" removed")
    })
}

fn strip_ansi(s: &str) -> &str {
    // `render::paint` wraps diff lines in a CSI colour sequence ending in `m`.
    if s.as_bytes().starts_with(&[0x1b, b'[']) {
        s.find('m').map(|i| &s[i + 1..]).unwrap_or(s)
    } else {
        s
    }
}

/// Every CLI:/Desktop: header is idle (`— no changes` / not found / same files).
fn all_variants_idle(out: &str) -> bool {
    out.lines()
        .filter(|l| l.starts_with("CLI:") || l.starts_with("Desktop:"))
        .all(|l| {
            l.contains("— no changes")
                || l.contains("— not found")
                || l.contains("— same files as above")
        })
}

#[test]
fn remove_twice_says_no_changes_and_the_second_takes_no_backup() {
    let home = tmp("remove");
    let cfg = write_cfg(&home);
    for (host, flags, file) in hosts(&home) {
        rtok(&setup_args(host, &flags), &cfg, &home);
        let first = rtok(&["agents", "remove", host], &cfg, &home);
        assert!(
            remove_made_edits(&first),
            "{host}: expected removals, got {first}"
        );
        let second = rtok(&["agents", "remove", host], &cfg, &home);
        assert!(
            all_variants_idle(&second) && !remove_made_edits(&second),
            "{host}: {second}"
        );
        assert!(!second.contains("backup "), "{host}: {second}");
        if let Some(f) = &file {
            // setup created the file (no copy), the first remove copied it, the second
            // changed nothing and left no copy behind.
            assert_eq!(backups(f).len(), 1, "{host}: {:?}", backups(f));
        }
    }
}

/// The `agents list` blocks (header line to the next blank line) whose header is `needle` or
/// whose `config` line names it; the module rows are the two-space `✓` / `✗` / `−` lines.
fn installed_modules(list: &str, needle: &str) -> Vec<String> {
    let list = slash(list);
    let blocks: Vec<&str> = list
        .split("\n\n")
        .filter(|b| {
            b.lines()
                .any(|l| l == needle || (l.starts_with("  config") && l.contains(needle)))
        })
        .collect();
    assert!(!blocks.is_empty(), "no block for {needle}: {list}");
    blocks
        .iter()
        .flat_map(|b| b.lines())
        .filter(|l| l.starts_with("  ✓"))
        .map(str::to_string)
        .collect()
}

#[test]
fn list_reports_installed_modules_per_host() {
    let home = tmp("list");
    let cfg = write_cfg(&home);
    let before = rtok(&["agents", "list"], &cfg, &home);
    for (host, flags, file) in hosts(&home) {
        let needle = file.map_or_else(
            || "CLI: pi".to_string(), // pi edits no config file
            |f| slash(f.display().to_string()),
        );
        assert!(installed_modules(&before, &needle).is_empty(), "{host}");
        rtok(&setup_args(host, &flags), &cfg, &home);
        let after = rtok(&["agents", "list"], &cfg, &home);
        assert!(
            !installed_modules(&after, &needle).is_empty(),
            "{host}: nothing shows installed after install:\n{after}"
        );
        rtok(&["agents", "remove", host], &cfg, &home);
        let gone = rtok(&["agents", "list"], &cfg, &home);
        assert_eq!(
            installed_modules(&gone, &needle),
            Vec::<String>::new(),
            "{host}: still installed after remove"
        );
    }
}

/// T416.3: the default dry run ends with the totals line; `--stat` swaps the step lines for the
/// stat row and the same totals.
#[test]
fn dry_run_ends_with_totals_and_stat_replaces_the_steps() {
    let home = tmp("dry-stat");
    let cfg = write_cfg(&home);
    let mut args = setup_args("codex", &[]);
    args.push("--dry-run");
    let plain = rtok(&args, &cfg, &home);
    assert!(plain.contains("+ [mcp_servers.rtok]"), "{plain}");
    assert!(plain.contains(" changed, "), "no totals line:\n{plain}");
    args.push("--stat");
    let stat = rtok(&args, &cfg, &home);
    assert!(!stat.contains("+ [mcp_servers.rtok]"), "{stat}");
    assert!(stat.contains(" | "), "no stat row:\n{stat}");
    assert!(stat.contains(" changed, "), "no totals line:\n{stat}");
    assert!(!home.join(".codex/config.toml").exists());
}

#[test]
fn dry_run_setup_creates_nothing_and_copies_nothing() {
    let home = tmp("dry");
    let cfg = write_cfg(&home);
    let settings = home.join(".claude/settings.json");
    fs::write(&settings, "{}").unwrap();
    for (host, flags, file) in hosts(&home) {
        let mut args = setup_args(host, &flags);
        args.push("--dry-run");
        let out = rtok(&args, &cfg, &home);
        assert!(out.contains("— dry run, nothing written"), "{host}: {out}");
        assert!(!out.contains("backup "), "{host}: {out}");
        if let Some(f) = file.filter(|f| f != &settings) {
            assert!(!f.exists(), "{host}: {} created by a dry run", f.display());
        }
    }
    assert_eq!(fs::read_to_string(&settings).unwrap(), "{}");
    assert!(backups(&settings).is_empty());
    assert!(
        !home
            .join(".pi/agent/extensions/rtok")
            .symlink_metadata()
            .is_ok()
    );
}

#[test]
fn the_agent_alias_prints_what_agents_prints() {
    let home = tmp("alias");
    let cfg = write_cfg(&home);
    assert_eq!(
        rtok(&["agent", "list"], &cfg, &home),
        rtok(&["agents", "list"], &cfg, &home)
    );
    let a = rtok(&["agent", "setup", "codex", "--dry-run"], &cfg, &home);
    let b = rtok(&["agents", "install", "codex", "--dry-run"], &cfg, &home);
    assert_eq!(a, b);
}

#[test]
fn cli_and_desktop_flags_pick_the_block() {
    let home = tmp("kinds");
    let cfg = write_cfg(&home);
    let cli = rtok(
        &["agents", "install", "cursor", "--yes", "--cli"],
        &cfg,
        &home,
    );
    assert!(cli.contains("CLI: Cursor CLI"), "{cli}");
    assert!(!cli.contains("Desktop: Cursor"), "{cli}");
    let desktop = rtok(
        &["agents", "install", "cursor", "--yes", "--desktop"],
        &cfg,
        &home,
    );
    assert!(desktop.contains("Desktop: Cursor"), "{desktop}");
    assert!(!desktop.contains("CLI: Cursor CLI"), "{desktop}");
    // Both kinds: the shared-config sibling points at the block above it.
    let both = rtok(&["agents", "install", "cursor", "--yes"], &cfg, &home);
    assert!(
        both.contains("CLI: Cursor CLI") && both.contains("Desktop: Cursor — same files as above"),
        "{both}"
    );
    // `--desktop` never touches the CLI file of a host whose apps keep separate configs.
    let only = rtok(&["agents", "install", "opencode", "--desktop"], &cfg, &home);
    assert!(only.contains("Desktop: OpenCode Desktop"), "{only}");
    assert!(!only.contains("CLI: OpenCode"), "{only}");
    assert!(!home.join(".config/opencode/opencode.json").exists());
}

#[test]
fn claude_desktop_installs_mcp_with_the_absolute_binary_under_a_temp_home() {
    let home = tmp("desktop");
    let cfg = write_cfg(&home);
    let file = claude_desktop_config(&home);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, r#"{"mcpServers":{"foreign":{"command":"x"}}}"#).unwrap();

    let out = rtok(&["agents", "install", "claude", "--desktop"], &cfg, &home);
    assert!(out.contains("Desktop: Claude Desktop"), "{out}");
    assert!(!out.contains("CLI: Claude Code"), "{out}");
    assert!(out.contains("✓ mcp     desktop  entry "), "{out}");
    assert!(
        out.contains("− hooks   not supported: Claude Desktop has no hook events"),
        "{out}"
    );
    assert!(
        !home.join(".claude/settings.json").exists(),
        "the CLI files stay untouched"
    );
    let servers = json(&file);
    let command = servers["mcpServers"]["rtok"]["command"].as_str().unwrap();
    assert!(Path::new(command).is_absolute(), "{command}");
    assert_eq!(servers["mcpServers"]["rtok"]["args"][0], "mcp");
    assert!(servers["mcpServers"]["foreign"].is_object(), "{servers}");
    assert_eq!(backups(&file).len(), 1);

    let again = rtok(&["agents", "install", "claude", "--desktop"], &cfg, &home);
    assert!(
        again.contains("Desktop: Claude Desktop — already installed"),
        "{again}"
    );
    assert_eq!(backups(&file).len(), 1);

    rtok(&["agents", "remove", "claude"], &cfg, &home);
    let servers = json(&file);
    assert!(servers["mcpServers"]["rtok"].is_null(), "{servers}");
    assert!(servers["mcpServers"]["foreign"].is_object(), "{servers}");
}

/// T81 + T139: a pipe is the CI / agent shape. The plugin no longer asks a question at all —
/// it installs by default once `claude` is on PATH — so a plain install on a pipe still
/// finishes green, with the plugin as the only call path for hooks (D21); MCP is written
/// independently of it (T275).
#[test]
fn a_pipe_still_installs_the_plugin_without_asking() {
    let home = tmp("offer-pipe");
    let cfg = write_cfg(&home);
    let out = raw(&["agents", "install", "claude"], &cfg, &home);
    assert!(out.status.success(), "{:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("[Y/n]"),
        "a pipe must never see a question: {stdout}"
    );
    assert!(
        stdout.contains("+ plugin plugins/claude → rtok@rtok"),
        "{stdout}"
    );
    assert!(stdout.contains("✓ hooks   installed"), "{stdout}");
    assert!(stdout.contains("✓ plugin  installed"), "{stdout}");
}

#[test]
fn an_unknown_host_is_refused_before_any_backup() {
    let home = tmp("unknown");
    let cfg = write_cfg(&home);
    let settings = home.join(".claude/settings.json");
    fs::write(&settings, "{}").unwrap();
    let out = raw(&["agents", "install", "notahost"], &cfg, &home);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("unknown host: notahost"), "{err}");
    let out = raw(&["agents", "remove", "notahost"], &cfg, &home);
    assert!(!out.status.success());
    assert!(
        backups(&settings).is_empty(),
        "nothing copied before the refusal"
    );
    assert_eq!(fs::read_to_string(&settings).unwrap(), "{}");
}

/// T426: a config folder an uninstalled host left behind is not the host. Naming it refuses the
/// whole run with a non-zero exit before any backup or write, an installed host named beside
/// it included; `remove` still cleans it up.
#[test]
fn a_host_whose_app_is_missing_is_refused_before_any_write() {
    let home = tmp("absent");
    let cfg = write_cfg(&home);
    let path = common::agents::fake_hosts(&home);
    for stub in ["gemini", "gemini.cmd"] {
        let _ = fs::remove_file(path.join(stub));
    }
    let settings = home.join(".gemini/settings.json");
    fs::write(&settings, "{}").unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new(common::agents::bin())
            .args(["--config", cfg.to_str().unwrap()])
            .args(args)
            .arg("--no-restart")
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("PATH", &path)
            .output()
            .unwrap()
    };
    for args in [
        &["agents", "install", "gemini"][..],
        &["agents", "install", "codex,gemini"],
        &["agents", "update", "gemini"],
    ] {
        let out = run(args);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{args:?} must fail: {err}");
        assert!(err.contains("gemini is not installed"), "{args:?}: {err}");
    }
    assert_eq!(fs::read_to_string(&settings).unwrap(), "{}");
    assert!(
        backups(&settings).is_empty(),
        "nothing copied before the refusal"
    );
    assert!(
        !home.join(".codex/config.toml").exists(),
        "the installed host named beside it is not written either"
    );
    let out = run(&["agents", "remove", "gemini"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The configs spawn `rtok` by name; without it on PATH the setup still runs and says so
/// first. Windows writes the absolute exe instead, so it has nothing to warn about.
#[test]
fn a_missing_rtok_on_path_is_a_warning_at_the_top() {
    let home = tmp("path");
    let cfg = write_cfg(&home);
    let out = std::process::Command::new(common::agents::bin())
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "agents",
            "setup",
            "codex",
            "--no-restart",
        ])
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        // Codex itself is installed (T426); only `rtok` is missing from PATH.
        .env("PATH", common::agents::fake_hosts(&home))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    if cfg!(windows) {
        assert!(!stdout.contains("not on PATH"), "{stdout}");
    } else {
        assert!(
            stdout.starts_with("warning: rtok is not on PATH"),
            "{stdout}"
        );
    }
    assert!(stdout.contains("CLI: Codex"), "{stdout}");
    let with = raw(&["agents", "remove", "codex"], &cfg, &home);
    assert!(
        !String::from_utf8_lossy(&with.stdout).contains("not on PATH"),
        "remove never warns"
    );
}

/// T249: `[setup] backup_files` caps the copies per file, and only a run that changed
/// something prunes — a no-change run deletes its own copy and must not cost an older one.
#[test]
fn backup_files_caps_copies_and_a_no_change_run_prunes_nothing() {
    let home = tmp("backup-cap");
    let cfg = write_cfg(&home);
    let mut toml = fs::read_to_string(&cfg).unwrap();
    toml.push_str("[setup]\nbackup_files = 1\n");
    fs::write(&cfg, toml).unwrap();
    let file = home.join(".codex/config.toml");
    fs::write(&file, "# mine\n").unwrap();
    rtok(&setup_args("codex", &[]), &cfg, &home);
    let first = backups(&file);
    assert_eq!(first.len(), 1);
    // Edited since, still installed: the up-front copy is taken, then dropped.
    let mut body = fs::read_to_string(&file).unwrap();
    body.push_str("# edited\n");
    fs::write(&file, &body).unwrap();
    let again = rtok(&setup_args("codex", &[]), &cfg, &home);
    assert!(again.contains("already installed"), "{again}");
    assert_eq!(backups(&file), first, "no change, nothing pruned");
    // A run that changes the file keeps its copy and prunes down to the cap.
    rtok(&["agents", "remove", "codex"], &cfg, &home);
    let after = backups(&file);
    assert_eq!(after.len(), 1, "{after:?}");
    assert!(fs::read_to_string(&after[0]).unwrap().contains("# edited"));
    let _ = fs::remove_dir_all(home);
}

/// T520: MiMo writes `mimocode.jsonc` itself, so a home that holds only that file gets the MCP
/// entry in it, with its comments and every other byte kept, and the plugin link beside it;
/// remove takes both back and leaves the user's text as it was.
#[test]
fn mimo_edits_a_lone_mimocode_jsonc_and_links_the_plugin() {
    let home = tmp("mimo-jsonc");
    let cfg = write_cfg(&home);
    let dir = home.join(".config/mimocode");
    let jsonc = dir.join("mimocode.jsonc");
    let mine = "{\n  // mine\n  \"theme\": \"dark\", // inline\n}\n";
    fs::write(&jsonc, mine).unwrap();

    let first = rtok(&setup_args("mimo", &[]), &cfg, &home);
    assert!(first.contains("mcp.rtok"), "{first}");
    assert!(!dir.join("mimocode.json").exists(), "{first}");
    let body = fs::read_to_string(&jsonc).unwrap();
    assert!(body.starts_with(&mine[..mine.len() - 2]), "{body}");
    assert!(body.contains("\"rtok\""), "{body}");
    assert!(dir.join("plugins/rtok.ts").exists(), "{first}");
    let second = rtok(&setup_args("mimo", &[]), &cfg, &home);
    assert!(second.contains("already installed"), "{second}");

    rtok(&["agents", "remove", "mimo"], &cfg, &home);
    let left = fs::read_to_string(&jsonc).unwrap();
    assert!(
        left.contains("// mine") && left.contains("// inline"),
        "{left}"
    );
    assert!(!left.contains("rtok"), "{left}");
    assert!(dir.join("plugins/rtok.ts").symlink_metadata().is_err());
    let _ = fs::remove_dir_all(home);
}
