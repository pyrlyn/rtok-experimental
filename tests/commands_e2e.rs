// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T38.1/T38.5: e2e for the commands without direct coverage — `hook`, `run`,
//! `expand`, `plugins`, `config`, `bench --dry-run`, `doctor`, `stats` — driven
//! through `assert_cmd` (plan Working agreement).
//!
//! Check: one case per command through the binary with an isolated HOME.

use assert_cmd::Command as AssertCmd;
use std::fs;
use std::path::{Path, PathBuf};

fn cmd(args: &[&str], home: &Path) -> AssertCmd {
    let mut c = AssertCmd::cargo_bin("rtok").unwrap();
    // The checkout as cwd would make `doctor`'s junk section walk the repository's real
    // worktrees (T666).
    c.args(args)
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .current_dir(home);
    c
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rtok-t381-{name}-{}-{}",
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

/// Run to success, return stdout.
fn ok(args: &[&str], home: &Path) -> String {
    let out = cmd(args, home).assert().success().get_output().clone();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn hook(event: &str, fixture: &[u8], home: &Path) -> String {
    let out = cmd(&["hook", event], home)
        .write_stdin(fixture)
        .assert()
        .success()
        .get_output()
        .clone();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn hook_pre_tool_bash_exits_0_with_json() {
    let home = tmp("pre");
    let stdout = hook(
        "PreToolUse",
        include_bytes!("fixtures/hooks/pre_tool_bash.json"),
        &home,
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(&stdout).is_ok_and(|v| v.is_object()),
        "hook output is a JSON object: {stdout}"
    );
    let _ = fs::remove_dir_all(&home);
}

/// T50.4: native Grep/Glob pass through by default; with
/// `RTOK_PLUGINS_GUARD_DENY_GREP_GLOB=true` the hook exits 0 with a deny
/// naming the MCP replacement (`search` for Grep, `tree` for Glob).
#[test]
fn hook_grep_glob_deny_is_opt_in() {
    for (fixture, tool, pointer) in [
        (
            include_bytes!("fixtures/hooks/pre_tool_grep.json").as_slice(),
            "Grep",
            "search",
        ),
        (
            include_bytes!("fixtures/hooks/pre_tool_glob.json").as_slice(),
            "Glob",
            "tree",
        ),
    ] {
        // Default: no deny.
        let home = tmp("native-off");
        let stdout = hook("PreToolUse", fixture, &home);
        let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_ne!(
            v.pointer("/hookSpecificOutput/permissionDecision")
                .and_then(|x| x.as_str()),
            Some("deny"),
            "{tool} must pass by default: {stdout}"
        );
        let _ = fs::remove_dir_all(&home);
        // Opt-in: deny with a pointer, still exit 0 (fail open at the process).
        let home = tmp("native-on");
        let out = cmd(&["hook", "PreToolUse"], &home)
            .env("RTOK_PLUGINS_GUARD_DENY_GREP_GLOB", "true")
            .write_stdin(fixture)
            .assert()
            .success()
            .get_output()
            .clone();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(
            v.pointer("/hookSpecificOutput/permissionDecision")
                .and_then(|x| x.as_str()),
            Some("deny"),
            "{tool} must deny when opted in: {stdout}"
        );
        let reason = v
            .pointer("/hookSpecificOutput/permissionDecisionReason")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        assert!(reason.contains(pointer), "{tool}: {reason}");
        let _ = fs::remove_dir_all(&home);
    }
}

/// Emulates the host: PreToolUse rewrites `sleep; cat file | tail`, then the
/// rewritten command runs under `sh` exactly as Claude Code would run it.
#[cfg(unix)]
#[test]
fn hook_rewrite_of_sleep_cat_tail_runs_through_rtok() {
    let home = tmp("sleep-tail");
    let file = home.join("task.output");
    let lines: Vec<String> = (1..=50).map(|i| format!("line {i}")).collect();
    fs::write(&file, lines.join("\n") + "\n").unwrap();
    let original = format!("sleep 1; cat {} | tail -30", file.display());
    let event = serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": original, "description": "t"},
        "session_id": "t",
        "cwd": home,
    });
    let stdout = hook("PreToolUse", event.to_string().as_bytes(), &home);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let rewritten = v
        .pointer("/hookSpecificOutput/updatedInput/command")
        .and_then(|x| x.as_str())
        .unwrap_or_else(|| panic!("not rewritten: {stdout}"));
    assert!(rewritten.contains("rtok run -- '"), "{rewritten}");

    let bin = assert_cmd::cargo::cargo_bin("rtok");
    let path = format!(
        "{}:{}",
        bin.parent().unwrap().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = std::process::Command::new("sh")
        .args(["-c", rewritten])
        .env("PATH", path)
        .env("RTOK_HOME", &home)
        .env("HOME", &home)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("line 50"), "{text}");
    assert!(!text.contains("line 20\n"), "tail -30 ran inside: {text}");
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn hook_session_start_exits_0_with_json() {
    let home = tmp("start");
    let stdout = hook(
        "SessionStart",
        include_bytes!("fixtures/hooks/session_start.json"),
        &home,
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(&stdout).is_ok_and(|v| v.is_object()),
        "hook output is a JSON object: {stdout}"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn run_echo_prints_its_output() {
    let home = tmp("run");
    let out = ok(&["run", "echo", "hello-e2e"], &home);
    assert!(out.contains("hello-e2e"), "{out}");
    let _ = fs::remove_dir_all(&home);
}

/// T366: a command killed by a signal exits `128 + signal` like a shell, not `1`; a plain
/// non-zero exit keeps its code. The `kill` targets the test's own `sh` (`$$`).
#[cfg(unix)]
#[test]
fn run_reports_128_plus_the_signal_for_a_killed_command() {
    let home = tmp("signal");
    for (script, code) in [
        ("kill -TERM $$", 143),
        ("kill -KILL $$", 137),
        ("exit 7", 7),
    ] {
        cmd(&["run", "sh", "-c", script], &home).assert().code(code);
    }
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn run_long_output_then_expand_round_trips() {
    let home = tmp("expand");
    let out = ok(
        &["run", "awk", "BEGIN{for(i=1;i<=50;i++)print \"line \"i}"],
        &home,
    );
    let trailer = out
        .lines()
        .find(|l| l.starts_with("[rtok "))
        .expect("{out}");
    let id = trailer.split_whitespace().nth(1).expect("{trailer}");
    let full = ok(&["expand", id], &home);
    assert!(
        full.contains("line 1") && full.contains("line 50"),
        "{full}"
    );
    let head = ok(&["expand", id, "--lines", "1-2"], &home);
    assert_eq!(head.lines().count(), 2, "{head}");
    let out = cmd(&["expand", "no-such-id"], &home)
        .assert()
        .failure()
        .get_output()
        .clone();
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(err.contains("unknown archive id"), "{err}");
    let _ = fs::remove_dir_all(&home);
}

/// T683: a logfmt log is cut with its shared pairs stated once, and `expand` still returns
/// every raw line, shared pairs included.
#[test]
fn run_logfmt_output_states_shared_pairs_once_and_expands_raw() {
    let home = tmp("logfmt");
    let script = "BEGIN{for(i=1;i<=120;i++)print \"level=info svc=api msg=ok n=\" i}";
    let out = ok(&["run", "awk", script], &home);
    assert!(
        out.contains("[every record: level=info svc=api msg=ok]"),
        "{out}"
    );
    assert!(
        out.contains("varying: n=") && out.contains("n=120"),
        "{out}"
    );
    assert!(!out.contains("level=info svc=api msg=ok n=120"), "{out}");
    let id = out
        .lines()
        .find(|l| l.starts_with("[rtok "))
        .and_then(|t| t.split_whitespace().nth(1))
        .expect("{out}");
    let full = ok(&["expand", id], &home);
    assert_eq!(full.lines().count(), 120, "{full}");
    assert!(full.contains("level=info svc=api msg=ok n=57"), "{full}");
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn plugins_lists_the_catalogue() {
    let home = tmp("plugins");
    let out = ok(&["plugins"], &home);
    for id in [
        "measure",
        "cmd",
        "read",
        "json_tree",
        "archive",
        "proxy",
        "inject",
        "guard",
        "memory",
        "graph",
        "toon",
        "compress",
        "docs",
    ] {
        assert!(out.contains(id), "{id} missing:\n{out}");
    }
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn config_init_get_set_validate_round_trips() {
    let home = tmp("config");
    ok(&["config", "init"], &home);
    assert!(home.join("config.toml").exists());
    let path = ok(&["config", "path"], &home);
    assert!(path.contains("config.toml"), "{path}");
    assert!(ok(&["config", "show"], &home).contains("proxy.port"));
    assert!(
        !ok(&["config", "get", "proxy.port"], &home)
            .trim()
            .is_empty()
    );
    ok(&["config", "set", "proxy.port", "8791"], &home);
    assert_eq!(ok(&["config", "get", "proxy.port"], &home).trim(), "8791");
    // A pin that is not today's default is a note, then the same ok line.
    let validated = ok(&["config", "validate"], &home);
    assert!(
        validated.lines().any(|line| line.starts_with("ok "))
            && validated.contains("proxy.port = 8791")
            && validated.contains("(default 8790)"),
        "{validated}"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn bench_dry_run_lists_the_schedule() {
    let home = tmp("bench");
    let tasks = concat!(env!("CARGO_MANIFEST_DIR"), "/bench/tasks.toml");
    let out = ok(&["bench", "--tasks", tasks, "--dry-run"], &home);
    assert_eq!(out.lines().count(), 6 * 2 * 3, "{out}");
    assert!(out.contains("add-fn a 1"), "{out}");
    let _ = fs::remove_dir_all(&home);
}

/// T666: every command here runs in its temp home, not the checkout. From the checkout,
/// `doctor`'s junk section walked every real worktree of the repository and blew nextest's
/// 180 s limit on a machine with many large worktrees.
#[cfg(unix)] // `pwd`
#[test]
fn commands_run_in_the_temp_home_not_the_checkout() {
    let home = tmp("cwd");
    let out = ok(&["run", "pwd", "-P"], &home);
    assert_eq!(
        PathBuf::from(out.trim()),
        fs::canonicalize(&home).unwrap(),
        "{out}"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn doctor_reports_the_chain() {
    let home = tmp("doctor");
    let out = ok(&["doctor"], &home);
    assert!(out.contains("hooks ") && out.contains("proxy "), "{out}");
    let _ = fs::remove_dir_all(&home);
}

/// T977: `--project` analyses the checkout named, and whether its language server is installed
/// follows `PATH`; a stub file stands in for the real server.
#[test]
fn doctor_project_reports_the_language_server_from_path() {
    let home = tmp("doctor-project");
    let repo = home.join("repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join(".git"), "").unwrap();
    fs::write(repo.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
    let with = home.join("with");
    let without = home.join("without");
    fs::create_dir_all(&with).unwrap();
    fs::create_dir_all(&without).unwrap();
    fs::write(with.join("rust-analyzer"), "stub").unwrap();

    let rust = |path: &Path| -> serde_json::Value {
        let out = cmd(&["doctor", "--json", "--project"], &home)
            .arg(&repo)
            .env("PATH", path)
            .assert()
            .success()
            .get_output()
            .clone();
        let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        report["project"]["languages"]
            .as_array()
            .and_then(|l| l.iter().find(|l| l["language"] == "rust"))
            .cloned()
            .unwrap_or_else(|| panic!("no rust row in {report}"))
    };
    let found = rust(&with);
    assert_eq!((found["files"].as_u64(), found["installed"].as_bool()), (Some(1), Some(true)));
    let missing = rust(&without);
    assert_eq!(missing["installed"], false);
    assert_eq!(missing["install"], "rustup component add rust-analyzer");
    let _ = fs::remove_dir_all(&home);
}

/// T977: a path that does not exist is refused before any probe runs.
#[test]
fn doctor_project_refuses_a_missing_path() {
    let home = tmp("doctor-project-missing");
    let out = cmd(&["doctor", "--project", "no-such-dir"], &home)
        .assert()
        .failure()
        .get_output()
        .clone();
    assert!(String::from_utf8_lossy(&out.stderr).contains("no such path"));
    let _ = fs::remove_dir_all(&home);
}

/// T276.3: the MCP probe draws a bar on a terminal only; a piped stderr gets no bar bytes.
#[cfg(unix)] // POSIX command path
#[test]
fn doctor_probing_mcp_servers_writes_no_bar_to_a_pipe() {
    let home = tmp("doctor-bar");
    let servers = r#"{"mcpServers":{"fake":{"command":"/bin/sh","args":["-c","exit 0"]}}}"#;
    fs::write(home.join(".claude.json"), servers).unwrap();
    // Outside any repository, so doctor does not size every worktree of the checkout.
    let out = cmd(&["doctor"], &home)
        .current_dir(&home)
        .assert()
        .success()
        .get_output()
        .clone();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(String::from_utf8_lossy(&out.stdout).contains("fake"));
    assert!(
        !err.contains("probing") && !err.contains('\u{1b}') && !err.contains('\r'),
        "{err:?}"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn stats_json_parses() {
    let home = tmp("stats");
    let out = ok(&["stats", "--json"], &home);
    assert!(
        serde_json::from_str::<serde_json::Value>(&out).is_ok_and(|v| v.get("sessions").is_some()),
        "{out}"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn info_prints_paths_sizes_and_proxy() {
    let home = tmp("info");
    let out = ok(&["info"], &home);
    for want in [
        "rtok ",
        "binary ",
        &format!("home {}", home.display()),
        "config.toml",
        "rtok.db (-)",
        "archive",
        "errors",
        "proxy ",
        "8790",
        "otel off",
        "store calls",
        "disk ",
    ] {
        assert!(out.contains(want), "{want} missing:\n{out}");
    }
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn info_counts_error_lines_and_json_parses() {
    let home = tmp("info-err");
    ok(&["info"], &home);
    let log = home.join("logs").join("rtok.log");
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(
        &log,
        "2026-09-09 15:04:05 error t/n: boom\n2026-09-09 15:04:06 info t/n: ok\n",
    )
    .unwrap();
    let out = ok(&["info"], &home);
    assert!(out.contains("2 lines, 1 errors"), "{out}");
    let json = ok(&["info", "--json"], &home);
    let v: serde_json::Value = serde_json::from_str(&json).expect("info --json is JSON");
    assert_eq!(v["log"]["errors"], 1);
    assert_eq!(v["proxy"]["port"], 8790);
    assert_eq!(v["otel"]["enabled"], false);
    assert!(v["db"]["bytes"].is_number(), "{v}");
    let _ = fs::remove_dir_all(&home);
}

/// T367: every path-taking graph subcommand fails on a missing path, naming it, before walking.
#[test]
fn graph_subcommands_reject_a_missing_path() {
    let home = tmp("graph-missing-home");
    let missing = home.join("nonexistent");
    let missing = missing.to_str().unwrap();
    for args in [
        vec!["graph", "index", missing],
        vec!["graph", "dead", missing],
        vec!["graph", "status", missing],
        vec!["graph", "impact", "main", missing],
    ] {
        let out = cmd(&args, &home).assert().failure().get_output().clone();
        let err = String::from_utf8_lossy(&out.stderr);
        // Only the path: the OS error text differs (Windows says "cannot find the file").
        assert!(err.contains(missing), "{args:?}: {err}");
        assert!(out.stdout.is_empty(), "{args:?}: {:?}", out.stdout);
    }
}

#[test]
fn graph_index_rejects_a_file_path_and_still_indexes_a_project() {
    let home = tmp("graph-file-home");
    let project = tmp("graph-project");
    let file = project.join("a.rs");
    fs::write(&file, "fn alpha() {}\n").unwrap();
    let err = cmd(&["graph", "index", file.to_str().unwrap()], &home)
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    assert!(String::from_utf8_lossy(&err).contains("not a directory"));
    let out = ok(&["graph", "index", project.to_str().unwrap()], &home);
    assert!(out.contains("indexed 1 files"), "{out}");
}

/// T362: the first `config validate` on an empty HOME creates the default file like every other
/// subcommand, while a path the user typed must exist.
#[test]
fn config_validate_creates_the_default_file_but_not_an_explicit_one() {
    let home = tmp("config-validate-fresh");
    let out = String::from_utf8_lossy(
        &cmd(&["config", "validate"], &home)
            .env_remove("RTOK_CONFIG")
            .assert()
            .success()
            .get_output()
            .stdout,
    )
    .into_owned();
    assert!(
        out.starts_with("ok ") && out.contains("config.toml"),
        "{out}"
    );
    assert!(home.join("config.toml").exists());

    let missing = home.join("nope.toml");
    let missing = missing.to_str().unwrap();
    let out = cmd(&["config", "validate", missing], &home)
        .env_remove("RTOK_CONFIG")
        .assert()
        .failure()
        .get_output()
        .clone();
    assert!(String::from_utf8_lossy(&out.stderr).contains(missing));
}

/// T331.5: `doctor --fix` is a dry run until `--yes`; then it backs the file up, drops only the
/// broken hook and keeps every other byte.
#[cfg(unix)] // POSIX hook paths
#[test]
fn doctor_fix_removes_only_the_broken_hook_after_a_backup() {
    let home = tmp("doctor-fix");
    let claude = home.join(".claude");
    fs::create_dir_all(&claude).unwrap();
    let settings = claude.join("settings.json");
    let raw = format!(
        "{{\n  // mine\n  \"hooks\": {{\n    \"Stop\": [\n      {{ \"hooks\": [\n        {{ \"type\": \"command\", \"command\": \"{}/gone.sh\" }},\n        {{ \"type\": \"command\", \"command\": \"echo done\" }}\n      ] }}\n    ]\n  }}\n}}\n",
        home.display()
    );
    fs::write(&settings, &raw).unwrap();
    let run = |args: &[&str]| cmd(args, &home).assert().get_output().clone();

    let dry = run(&["doctor", "--fix"]);
    assert_eq!(dry.status.code(), Some(0));
    let text = String::from_utf8_lossy(&dry.stdout);
    assert!(text.contains("dry run: nothing is written"), "{text}");
    assert!(text.contains("would remove Stop"), "{text}");
    assert_eq!(fs::read_to_string(&settings).unwrap(), raw);

    let done = run(&["doctor", "--fix", "--yes"]);
    assert_eq!(done.status.code(), Some(0), "{done:?}");
    let text = String::from_utf8_lossy(&done.stdout);
    assert!(text.contains("1 entry removed, 0 left"), "{text}");
    let after = fs::read_to_string(&settings).unwrap();
    assert!(
        after.contains("// mine") && after.contains("echo done"),
        "{after}"
    );
    assert!(!after.contains("gone.sh"), "{after}");
    let backups: Vec<_> = fs::read_dir(claude.join(".backup_configs"))
        .unwrap()
        .collect();
    assert_eq!(backups.len(), 1);
    let bak = backups[0].as_ref().unwrap().path();
    assert_eq!(fs::read_to_string(bak).unwrap(), raw);

    let again = run(&["doctor", "--fix", "--yes"]);
    assert!(String::from_utf8_lossy(&again.stdout).contains("nothing to remove"));
}

/// T331.6: `--only duplicate-mcp` removes the extra copy of a server from the host's JSON and
/// keeps the rest of the file byte for byte, after a backup.
#[cfg(unix)] // POSIX command paths
#[test]
fn doctor_fix_removes_an_extra_mcp_copy_and_nothing_else() {
    let home = tmp("doctor-fix-mcp");
    let json = home.join(".claude.json");
    let before = "{\n  \"theme\": \"dark\",\n  \"mcpServers\": {\n    \"a\": {\"command\": \"/bin/tool\"},\n    \"b\": {\"command\": \"/bin/tool\"}\n  }\n}\n";
    fs::write(&json, before).unwrap();
    let out = cmd(
        &["doctor", "--fix", "--yes", "--only", "duplicate-mcp"],
        &home,
    )
    .assert()
    .get_output()
    .clone();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(text.contains("1 entry removed, 0 left"), "{text}");
    assert_eq!(
        fs::read_to_string(&json).unwrap(),
        "{\n  \"theme\": \"dark\",\n  \"mcpServers\": {\n    \"a\": {\"command\": \"/bin/tool\"}\n  }\n}\n"
    );
    let _ = fs::remove_dir_all(&home);
}

/// T365: a value `config validate` rejects in the file is rejected the same way when it arrives
/// through the environment, and the message names that layer.
#[test]
fn config_validate_checks_env_overrides_and_names_the_layer() {
    let home = tmp("config-validate-env");
    let validate = |level: Option<&str>| {
        let mut c = cmd(&["config", "validate"], &home);
        c.env_remove("RTOK_CONFIG").env_remove("RTOK_LOG_LEVEL");
        if let Some(level) = level {
            c.env("RTOK_LOG_LEVEL", level);
        }
        c.assert().get_output().clone()
    };

    let clean = validate(None);
    assert!(clean.status.success());
    assert!(String::from_utf8_lossy(&clean.stdout).starts_with("ok "));

    let good = validate(Some("debug"));
    assert!(good.status.success(), "{good:?}");

    let bad = validate(Some("verbose"));
    assert!(!bad.status.success());
    let stderr = String::from_utf8_lossy(&bad.stderr);
    assert!(
        stderr.contains("env: log.level must be error, warn, info, or debug"),
        "{stderr}"
    );
    let _ = fs::remove_dir_all(&home);
}

/// T379: a bad window is blamed on where it came from: the config key, or the flag.
#[test]
fn report_since_errors_name_their_source() {
    let home = tmp("report-since");
    let cfg = home.join("c.toml");
    fs::write(&cfg, "[report]\nsince = \"7x\"\n").unwrap();
    let cfg = cfg.to_str().unwrap();
    let stderr = |args: &[&str]| {
        let out = cmd(args, &home).assert().failure().get_output().clone();
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let from_config = stderr(&["--config", cfg, "report"]);
    assert!(
        from_config.contains("report.since") && !from_config.contains("--since"),
        "{from_config}"
    );
    let from_flag = stderr(&["report", "--since", "7x"]);
    assert!(
        from_flag.contains("--since") && !from_flag.contains("report.since"),
        "{from_flag}"
    );
    let _ = fs::remove_dir_all(&home);
}
