// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T182: `rtok agents junk clear` through the binary — the CLI/JSON surface over a fixture
//! home with a database and a stray `rtok.log.<N>` past `[log] files`. The archive-retention
//! half of `scan`/`run` (referenced vs. orphan archive) is a `src/agents/junk.rs` unit test:
//! backdating a call's `ts` needs `Store::set_call_ts`, `#[cfg(test)]`-only and so reachable
//! only from inside the crate, never from this external test binary.

// T902: the `git`/`age_all`/`names` helpers are used by unix-only tests, so Windows clippy sees them as dead.
#![cfg_attr(windows, allow(dead_code))]

use crate::it::common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    crate::it::common::bin_path("rtok")
}

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-t182-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn rtok(args: &[&str], home: &Path) -> String {
    let out = Command::new(bin())
        .args(args)
        // `list` also judges the worktrees of the repository it runs in (T330.5.3): never ours.
        .current_dir(home)
        .env("RTOK_HOME", home)
        .env("HOME", home)
        // Whatever hosts the test faked with `fake_hosts` (T426); none when it faked none.
        .env("PATH", home.join(".fake-hosts"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "rtok {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn dry_run_lists_the_stale_log_and_yes_removes_it_without_touching_the_db() {
    let home = home("clear");
    let cfg = rtok::config::Config::load_from(&home).expect("config");
    let _store = rtok::store::Store::open(&cfg.core.db_path).expect("store");
    assert!(cfg.core.db_path.is_file());

    let stale_log = cfg.log.path.with_file_name(format!(
        "{}.{}",
        cfg.log.path.file_name().unwrap().to_str().unwrap(),
        cfg.log.files + 1
    ));
    fs::create_dir_all(stale_log.parent().unwrap()).unwrap();
    fs::write(&stale_log, b"stale").unwrap();

    let preview = rtok(&["agents", "junk", "clear", "--json"], &home);
    let rows: serde_json::Value = serde_json::from_str(&preview).unwrap();
    let paths: Vec<String> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(paths, vec![stale_log.display().to_string()]);
    assert!(stale_log.is_file(), "dry run must not delete anything");

    rtok(&["agents", "junk", "clear", "--yes", "--json"], &home);
    assert!(!stale_log.exists());
    assert!(
        cfg.core.db_path.is_file(),
        "the database itself is never touched"
    );
}

/// T330.1: `list` is read-only and prints exactly what the `clear` dry run plans.
#[test]
fn list_reports_the_stale_log_with_exact_bytes_and_removes_nothing() {
    let home = home("list");
    let cfg = rtok::config::Config::load_from(&home).expect("config");
    let stale_log = cfg.log.path.with_file_name(format!(
        "{}.{}",
        cfg.log.path.file_name().unwrap().to_str().unwrap(),
        cfg.log.files + 1
    ));
    fs::create_dir_all(stale_log.parent().unwrap()).unwrap();
    fs::write(&stale_log, b"stale").unwrap();

    let json = rtok(&["agents", "junk", "list", "--json"], &home);
    let report: serde_json::Value = serde_json::from_str(&json).unwrap();
    let rtok_row = &report["agents"][0];
    assert_eq!(rtok_row["name"], "rtok");
    assert_eq!(rtok_row["kinds"][0]["kind"], "log");
    assert_eq!(rtok_row["kinds"][0]["size_bytes"], 5);
    assert_eq!(report["freed_default_bytes"], 5);

    let text = rtok(&["agents", "junk", "list", "--bytes"], &home);
    assert!(text.contains("Freed by `clear`: 5\n"), "{text}");
    assert!(stale_log.is_file(), "list must not delete anything");
}

fn write(path: &Path, bytes: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![b'x'; bytes]).unwrap();
}

/// T330.2: `list` over a fixture HOME. Claude Code and Codex live where `CLAUDE_CONFIG_DIR` and
/// `CODEX_HOME` point, Cursor under its usual folders; every size equals `disk_usage`, Cursor's
/// folders are "not documented", and nothing is read outside the fixture.
#[test]
fn list_shows_each_installed_host_with_its_folders_and_exact_sizes() {
    let home = home("hosts");
    // Every host installed but Gemini CLI, so `--all` still has one not installed to show.
    let hosts = common::agents::fake_hosts(&home);
    for stub in ["gemini", "gemini.cmd"] {
        let _ = fs::remove_file(hosts.join(stub));
    }
    let claude = home.join("cc");
    let codex = home.join("cx");
    write(&claude.join("settings.json"), 10);
    write(&claude.join("debug/a.log"), 100);
    write(&codex.join("config.toml"), 10);
    write(&codex.join("log/codex.log"), 10);
    write(&home.join(".cursor/hooks.json"), 10);
    let cursor_data = home.join("Library/Application Support/Cursor");
    write(&cursor_data.join("state"), 10);
    // The hosts' own config paths still default to `~/.claude` and `~/.codex`.
    fs::create_dir_all(home.join(".claude")).unwrap();
    fs::create_dir_all(home.join(".codex")).unwrap();

    let out = Command::new(bin())
        .args(["agents", "junk", "list", "--json"])
        // Only an installed host lists its folders (T426).
        .env("PATH", &hosts)
        .env("RTOK_HOME", &home)
        .env("HOME", &home)
        .env("CLAUDE_CONFIG_DIR", &claude)
        .env("CODEX_HOME", &codex)
        .env("XDG_CACHE_HOME", home.join("xc"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let folder = |agent: &str, path: &Path| -> serde_json::Value {
        let row = report["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == agent)
            .unwrap_or_else(|| panic!("no `{agent}` row: {report}"));
        row["folders"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == path.display().to_string())
            .unwrap_or_else(|| panic!("`{agent}` has no {}: {row}", path.display()))
            .clone()
    };

    let c = folder("claude", &claude);
    assert_eq!(c["size_bytes"], rtok::agents::junk::disk_usage(&claude));
    assert_eq!(
        (&c["role"], &c["documented"]),
        (&"data".into(), &true.into())
    );
    assert_eq!(folder("claude", &claude.join("debug"))["role"], "logs");
    assert_eq!(folder("codex", &codex.join("log"))["role"], "logs");
    let app = folder("cursor", &cursor_data);
    assert_eq!(
        app["size_bytes"],
        rtok::agents::junk::disk_usage(&cursor_data)
    );
    assert_eq!(app["documented"], false);
    assert_eq!(folder("cursor", &home.join(".cursor"))["documented"], false);

    let text = rtok(&["agents", "junk", "list", "--bytes"], &home);
    assert!(text.contains("not documented: not cleared"), "{text}");
    assert!(
        !text.contains('\x1b'),
        "no escape codes into a pipe: {text}"
    );

    let all = rtok(&["agents", "junk", "list", "--all", "--json"], &home);
    let all: serde_json::Value = serde_json::from_str(&all).unwrap();
    assert!(
        all["agents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["host"] == true && a["installed"] == false),
        "--all lists a host that is not installed"
    );
}

/// T330.4: `clear` with filters through the binary, in a fixture home with no host installed
/// (no process is ever probed) and rtok's own cache under it.
struct Clear {
    home: PathBuf,
    cache: PathBuf,
}

impl Clear {
    fn new(name: &str) -> Self {
        let home = home(name);
        let env = |k: &str| match k {
            "XDG_CACHE_HOME" => Some(home.join(".cache").into_os_string()),
            "LOCALAPPDATA" => Some(home.join("AppData/Local").into_os_string()),
            _ => None,
        };
        let roots = rtok::agents::junk_map::Roots::new(home.clone(), env);
        let cache = roots.resolve("{rtok_cache}");
        Self { home, cache }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        self.run_in(&self.home, args)
    }

    fn run_in(&self, cwd: &Path, args: &[&str]) -> std::process::Output {
        let base = ["agents", "junk", "clear"];
        self.rtok_in(cwd, &[&base[..], args].concat())
    }

    fn rtok_in(&self, cwd: &Path, args: &[&str]) -> std::process::Output {
        Command::new(bin())
            .args(args)
            .current_dir(cwd)
            .env("RTOK_HOME", &self.home)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CACHE_HOME", self.home.join(".cache"))
            .env("XDG_DATA_HOME", self.home.join(".local/share"))
            .env("LOCALAPPDATA", self.home.join("AppData/Local"))
            .env("RTOK_HOST_SANDBOX", "1")
            .env("PATH", self.home.join(".fake-hosts"))
            .output()
            .unwrap()
    }

    fn json(&self, args: &[&str]) -> (i32, serde_json::Value) {
        self.json_in(&self.home, args)
    }

    fn json_in(&self, cwd: &Path, args: &[&str]) -> (i32, serde_json::Value) {
        let out = self.run_in(cwd, args);
        let json = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!("{e}: {}", String::from_utf8_lossy(&out.stderr));
        });
        (out.status.code().unwrap(), json)
    }
}

/// Every file under `dir` with its length and mtime, for "nothing changed".
fn tree(dir: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap().flatten() {
        let meta = e.metadata().unwrap();
        if meta.is_dir() {
            out.extend(tree(&e.path()));
        } else {
            out.push((e.path(), meta.len(), meta.modified().unwrap()));
        }
    }
    out.sort();
    out
}

fn age(path: &Path, secs: u64) {
    let when = std::time::SystemTime::now() - std::time::Duration::from_secs(secs);
    let f = fs::File::options().write(true).open(path).unwrap();
    f.set_modified(when).unwrap();
}

#[test]
fn clear_with_a_kind_plans_exactly_and_yes_empties_the_cache_keeping_the_store() {
    let c = Clear::new("clear-kind");
    let cfg = rtok::config::Config::load_from(&c.home).expect("config");
    drop(rtok::store::Store::open(&cfg.core.db_path).expect("store"));
    write(&c.cache.join("a/blob"), 300);
    age(&c.cache.join("a/blob"), 2 * 86_400);
    write(&c.home.join("project/Cargo.lock"), 10);

    let before = tree(&c.home);
    let (code, plan) = c.json(&["--agent", "rtok", "--kind", "cache", "--json"]);
    assert_eq!(code, 0);
    assert_eq!(tree(&c.home), before, "a dry run changes no file");
    let items = plan["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{plan}");
    assert_eq!(items[0]["kind"], "cache");
    assert_eq!(items[0]["path"], c.cache.display().to_string());
    assert_eq!(items[0]["action"], "clear");
    assert_eq!(plan["freed_bytes"], 0);

    let (code, done) = c.json(&["--agent", "rtok", "--kind", "cache", "--yes", "--json"]);
    assert_eq!(code, 0, "{done}");
    assert_eq!(done["freed_bytes"], plan["planned_bytes"]);
    assert!(c.cache.is_dir() && !c.cache.join("a").exists());
    assert!(cfg.core.db_path.is_file() && c.home.join("project/Cargo.lock").is_file());

    let (code, again) = c.json(&["--kind", "cache", "--yes", "--json"]);
    assert_eq!((code, again["planned_bytes"].as_u64()), (0, Some(0)));
}

/// A file written after the scan's idea of "idle" is refused at removal time and the run
/// exits 1; nothing is removed.
#[test]
fn clear_yes_skips_an_item_modified_in_the_last_minute_and_exits_1() {
    let c = Clear::new("clear-fresh");
    write(&c.cache.join("blob"), 10);
    let out = c.run(&["--kind", "cache", "--yes", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("some junk could not be removed"));
    let done: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(done["items"][0]["note"], "modified in the last minute");
    assert!(c.cache.join("blob").is_file());
}

#[test]
fn a_bad_clear_flag_is_a_usage_error() {
    let c = Clear::new("clear-usage");
    for args in [
        ["--kind", "nope"],
        ["--agent", "nope"],
        ["--older-than", "soon"],
        ["--include", "everything"],
    ] {
        let out = c.run(&args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}

/// `--trash` moves to the freedesktop trash of the fixture home. Only on Linux: the macOS and
/// Windows trash cans belong to the real user, never a test's.
#[cfg(target_os = "linux")]
#[test]
fn clear_trash_moves_the_cache_to_the_home_trash() {
    let c = Clear::new("clear-trash");
    write(&c.cache.join("blob"), 10);
    age(&c.cache.join("blob"), 2 * 86_400);
    let (code, done) = c.json(&["--kind", "cache", "--yes", "--trash", "--json"]);
    assert_eq!(code, 0, "{done}");
    assert_eq!(done["items"][0]["note"], "moved to trash");
    assert!(!c.cache.join("blob").exists());
    assert!(c.home.join(".local/share/Trash/files/blob").is_file());
}

/// T330.5.1: a Claude Code debug log past `keep_logs_days` is `review` junk (only with
/// `--include review` or `--kind logs`), a fresher one stays, and the config key moves the line.
#[test]
fn review_logs_need_the_flag_and_follow_keep_logs_days() {
    let c = Clear::new("clear-logs");
    common::agents::fake_hosts(&c.home);
    let debug = c.home.join(".claude/debug");
    write(&debug.join("old.log"), 100);
    write(&debug.join("new.log"), 10);
    age(&debug.join("old.log"), 31 * 86_400);
    age(&debug.join("new.log"), 29 * 86_400);
    let planned = |args: &[&str]| {
        let (code, plan) = c.json(args);
        assert_eq!(code, 0, "{plan}");
        let items = plan["items"].as_array().unwrap().iter();
        let names = items.filter(|i| i["action"] == "clear" && i["kind"] == "logs");
        let mut names: Vec<String> = names
            .map(|i| {
                i["path"]
                    .as_str()
                    .unwrap()
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap()
                    .into()
            })
            .collect();
        names.sort();
        names
    };

    assert!(planned(&["--agent", "claude", "--json"]).is_empty());
    assert_eq!(
        planned(&["--agent", "claude", "--kind", "logs", "--json"]),
        ["old.log"]
    );
    let review = ["--agent", "claude", "--include", "review", "--json"];
    assert_eq!(planned(&review), ["old.log"]);

    fs::write(
        c.home.join("config.toml"),
        "[agents.junk]\nkeep_logs_days = 7\n",
    )
    .unwrap();
    assert_eq!(planned(&review), ["new.log", "old.log"]);
    let (code, done) = c.json(&["--agent", "claude", "--kind", "logs", "--yes", "--json"]);
    assert_eq!(code, 0, "{done}");
    assert!(!debug.join("old.log").exists() && !debug.join("new.log").exists());
    assert!(debug.is_dir(), "the log folder itself stays");
}

/// T330.5.2: a Claude Code session past `stale_session_days` is junk only for `--kind sessions`
/// (never by default nor with `--include review`), goes with its restore points, leaves the
/// project's memory alone, and `--session-days` or the config key moves the line.
#[test]
fn sessions_need_kind_sessions_and_follow_the_threshold() {
    let c = Clear::new("clear-sessions");
    common::agents::fake_hosts(&c.home);
    let id = "0b1c2d3e-4f50-6172-8394-a5b6c7d8e9f0";
    let transcript = c.home.join(format!(".claude/projects/p/{id}.jsonl"));
    let history = c.home.join(format!(".claude/file-history/{id}/f@v1"));
    let memory = c.home.join(".claude/projects/p/memory/MEMORY.md");
    for p in [&transcript, &history, &memory] {
        write(p, 10);
        age(p, 31 * 86_400);
    }
    let planned = |args: &[&str]| -> usize {
        let (code, plan) = c.json(args);
        assert_eq!(code, 0, "{plan}");
        let items = plan["items"].as_array().unwrap().iter();
        let sessions = items.filter(|i| i["action"] == "clear" && i["kind"] == "sessions");
        sessions.count()
    };

    assert_eq!(planned(&["--agent", "claude", "--json"]), 0);
    assert_eq!(
        planned(&["--agent", "claude", "--include", "review", "--json"]),
        0
    );
    let by_kind = ["--agent", "claude", "--kind", "sessions"];
    assert_eq!(planned(&[&by_kind[..], &["--json"]].concat()), 2);
    assert_eq!(
        planned(&[&by_kind[..], &["--session-days", "60", "--json"]].concat()),
        0
    );

    let list = rtok(&["agents", "junk", "list"], &c.home);
    assert!(
        list.contains("old sessions: not touched for more than 30 days"),
        "{list}"
    );
    assert!(list.contains("cleanupPeriodDays = 30 (default)"), "{list}");

    fs::write(
        c.home.join("config.toml"),
        "[agents.junk]\nstale_session_days = 60\n",
    )
    .unwrap();
    assert_eq!(planned(&[&by_kind[..], &["--json"]].concat()), 0);
    fs::write(
        c.home.join("config.toml"),
        "[agents.junk]\nstale_session_days = 30\n",
    )
    .unwrap();

    let (code, done) = c.json(&[&by_kind[..], &["--yes", "--json"]].concat());
    assert_eq!(code, 0, "{done}");
    assert!(!transcript.exists() && !history.exists());
    assert!(memory.is_file(), "the host's memory is never a session");
}

/// T330.5.2: an `extra` crash folder clears only dumps past `crash_dump_min_age_days` by default;
/// a younger one needs `--kind crash-dumps`.
#[test]
fn an_extra_crash_folder_clears_only_old_dumps_unless_named() {
    let c = Clear::new("clear-crash");
    common::agents::fake_hosts(&c.home);
    let dumps = c.home.join("dumps");
    write(&dumps.join("old.dmp"), 10);
    write(&dumps.join("young.dmp"), 10);
    age(&dumps.join("old.dmp"), 10 * 86_400);
    age(&dumps.join("young.dmp"), 2 * 86_400);
    let extra = format!(
        "[[agents.junk.extra]]\nhost = \"cursor\"\nkind = \"crash-dumps\"\npath = '{}'\n",
        dumps.display()
    );
    fs::write(c.home.join("config.toml"), extra).unwrap();
    let planned = |args: &[&str]| -> Vec<String> {
        let (code, plan) = c.json(args);
        assert_eq!(code, 0, "{plan}");
        let items = plan["items"].as_array().unwrap().iter();
        let found = items.filter(|i| i["kind"] == "crash-dumps" && i["action"] == "clear");
        let mut names: Vec<String> = found
            .map(|i| {
                i["path"]
                    .as_str()
                    .unwrap()
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap()
                    .into()
            })
            .collect();
        names.sort();
        names
    };

    assert_eq!(planned(&["--agent", "cursor", "--json"]), ["old.dmp"]);
    let named = ["--agent", "cursor", "--kind", "crash-dumps", "--json"];
    assert_eq!(planned(&named), ["old.dmp", "young.dmp"]);
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Every file under `dir`, the worktree's `.git` pointer included, last modified `secs` ago.
fn age_all(dir: &Path, secs: u64) {
    for e in fs::read_dir(dir).unwrap().flatten() {
        let kind = e.file_type().unwrap();
        if kind.is_dir() && e.file_name() != ".git" {
            age_all(&e.path(), secs);
        } else if kind.is_file() {
            age(&e.path(), secs);
        }
    }
}

fn names(items: &serde_json::Value, keep: impl Fn(&serde_json::Value) -> bool) -> Vec<String> {
    let mut out: Vec<String> = (items.as_array().unwrap().iter())
        .filter(|i| keep(i))
        .map(|i| {
            let path = i["path"].as_str().unwrap();
            Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into()
        })
        .collect();
    out.sort();
    out
}

/// T330.5.3: `stale-worktrees` is `rtok worktree gc`'s verdict. `list` shows the one it would
/// remove and every other worktree with gc's reason, `--include review --yes` removes the first
/// through `remove::detach` with its merged branch, and nothing else is touched: not a dirty,
/// unmerged, locked or fresh worktree, not a record whose directory is gone (no `prune`), not an
/// orphan, and not through `--trash` or a plain `clear --yes`.
#[cfg(unix)]
#[test]
fn stale_worktrees_are_what_gc_removes_and_clear_removes_only_those() {
    let c = Clear::new("clear-stale-wt");
    let hosts = common::agents::fake_hosts(&c.home);
    // The fixture PATH holds only the fake hosts; rtok needs git, and no real agent.
    let path = std::env::var_os("PATH").unwrap();
    let real = std::env::split_paths(&path)
        .map(|d| d.join("git"))
        .find(|g| g.is_file());
    std::os::unix::fs::symlink(real.unwrap(), hosts.join("git")).unwrap();
    let repo = c.home.join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "base"]);
    // gc judges "merged" against `origin/HEAD`, so the repository needs an origin.
    let origin = c.home.join("origin.git");
    git(
        &c.home,
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            origin.to_str().unwrap(),
        ],
    );
    git(
        &repo,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&repo, &["push", "-q", "origin", "main"]);
    git(&repo, &["remote", "set-head", "origin", "main"]);
    let pool = c.home.join("pool");
    let add = |dir: &Path, branch: &str| {
        git(
            &repo,
            &["worktree", "add", "-q", "-b", branch, dir.to_str().unwrap()],
        );
        dir.to_path_buf()
    };
    let days = |n: u64| n * 86_400;
    let old = add(&pool.join("old"), "old");
    age_all(&old, days(20));
    // Claude Code's own pool, so the item sits under that host's row.
    let claude = add(&repo.join(".claude/worktrees/c1"), "c1");
    age_all(&claude, days(20));
    add(&pool.join("fresh"), "fresh");
    let dirty = add(&pool.join("dirty"), "dirty");
    write(&dirty.join("wip.txt"), 5);
    age_all(&dirty, days(20));
    let unmerged = add(&pool.join("unmerged"), "unmerged");
    write(&unmerged.join("work.txt"), 5);
    git(&unmerged, &["add", "."]);
    git(&unmerged, &["commit", "-q", "-m", "work"]);
    age_all(&unmerged, days(20));
    let locked = add(&pool.join("locked"), "locked");
    git(
        &repo,
        &[
            "worktree",
            "lock",
            "--reason",
            "Cursor / grok | t9 | 2026-09-01",
            locked.to_str().unwrap(),
        ],
    );
    age_all(&locked, days(20));
    let gone = add(&pool.join("gone"), "gone");
    fs::remove_dir_all(&gone).unwrap();
    let orphan = pool.join("orphan");
    write(&orphan.join("f"), 5);
    let admin = repo.join(".git/worktrees/orphan");
    fs::write(
        orphan.join(".git"),
        format!("gitdir: {}\n", admin.display()),
    )
    .unwrap();

    // `list`: the removable ones are counted; each other worktree says why gc keeps it.
    let out = c.rtok_in(&repo, &["agents", "junk", "list", "--json"]);
    let list: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let mut rows = Vec::new();
    for a in list["agents"].as_array().unwrap() {
        for i in a["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["kind"] == "stale-worktrees")
        {
            let name = Path::new(i["path"].as_str().unwrap()).file_name().unwrap();
            // A counted review item is listed with the flag it needs; gc's own verdict
            // is the reason for every other one.
            let reason = i["skip_reason"].as_str().unwrap_or("");
            let reason = reason
                .strip_prefix("review kind: add --include review")
                .unwrap_or(reason);
            rows.push((
                a["name"].as_str().unwrap().to_string(),
                name.to_string_lossy().into_owned(),
                reason.to_string(),
            ));
        }
    }
    rows.sort();
    let row = |name: &str| {
        rows.iter()
            .find(|r| r.1 == name)
            .unwrap_or_else(|| panic!("{name}: {rows:?}"))
    };
    assert_eq!((row("old").0.as_str(), row("old").2.as_str()), ("rtok", ""));
    assert_eq!((row("c1").0.as_str(), row("c1").2.as_str()), ("claude", ""));
    assert!(row("fresh").2.contains("idle window"), "{rows:?}");
    assert!(row("dirty").2.contains("uncommitted"), "{rows:?}");
    assert!(row("unmerged").2.contains("not merged"), "{rows:?}");
    assert!(
        row("locked").2.contains("lock by Cursor / grok"),
        "{rows:?}"
    );
    assert!(row("gone").2.contains("directory is gone"), "{rows:?}");
    assert!(row("orphan").2.contains("orphan"), "{rows:?}");
    assert_eq!(rows.len(), 8, "the main checkout is no item: {rows:?}");

    // The plan is gc's dry run (its lock override switched off, which junk never takes).
    let (code, plan) = c.json_in(&repo, &["--include", "review", "--json"]);
    assert_eq!(code, 0, "{plan}");
    let planned = names(&plan["items"], |i| {
        i["kind"] == "stale-worktrees" && i["action"] == "clear"
    });
    let out = c.rtok_in(
        &repo,
        &[
            "worktree",
            "gc",
            "--json",
            "--idle",
            "14d",
            "--stale-lock",
            "3650d",
        ],
    );
    let gc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let gc_removes = names(&gc, |o| o["action"] == "remove");
    assert_eq!(planned, ["c1", "old"]);
    assert_eq!(planned, gc_removes);

    // No flag, or `--trash`, removes no worktree.
    let (code, done) = c.json_in(&repo, &["--yes", "--json"]);
    assert_eq!(code, 0, "{done}");
    let (code, done) = c.json_in(
        &repo,
        &["--include", "review", "--trash", "--yes", "--json"],
    );
    assert_eq!(code, 1, "{done}");
    assert!(old.is_dir() && claude.is_dir());

    let (code, done) = c.json_in(&repo, &["--include", "review", "--yes", "--json"]);
    assert_eq!(code, 0, "{done}");
    assert!(!old.exists() && !claude.exists());
    let branches = git(&repo, &["branch", "--format=%(refname:short)"]);
    for gone_branch in ["old", "c1"] {
        assert!(!branches.lines().any(|b| b == gone_branch), "{branches}");
    }
    for kept_branch in ["fresh", "dirty", "unmerged", "locked", "gone"] {
        assert!(
            branches.lines().any(|b| b == kept_branch),
            "{kept_branch}: {branches}"
        );
    }
    for stays in ["fresh", "dirty", "unmerged", "locked", "orphan"] {
        assert!(pool.join(stays).is_dir(), "{stays}");
    }
    let listed = git(&repo, &["worktree", "list", "--porcelain"]);
    assert!(listed.contains("pool/gone"), "no blanket prune: {listed}");

    let (code, again) = c.json_in(&repo, &["--include", "review", "--yes", "--json"]);
    assert_eq!(code, 0, "{again}");
    assert!(names(&again["items"], |i| i["kind"] == "stale-worktrees").is_empty());
}

/// T330.6: three stray log generations of different sizes, so the breakdown has an order, a
/// cut and a size floor to check without faking any host.
fn stray_logs(name: &str) -> (PathBuf, Vec<PathBuf>) {
    let home = home(name);
    let cfg = rtok::config::Config::load_from(&home).expect("config");
    let log = cfg.log.path.clone();
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    let base = log.file_name().unwrap().to_str().unwrap().to_string();
    let paths: Vec<PathBuf> = [(1, 30), (2, 10), (3, 20)]
        .into_iter()
        .map(|(n, size)| {
            let p = log.with_file_name(format!("{base}.{}", cfg.log.files + n));
            fs::write(&p, vec![b'x'; size]).unwrap();
            p
        })
        .collect();
    (home, paths)
}

#[test]
fn list_prints_each_item_with_its_reason_and_cuts_a_kind_at_items() {
    let (home, paths) = stray_logs("items");
    let all = rtok(
        &["agents", "junk", "list", "--bytes", "--items", "all"],
        &home,
    );
    // The test home is `$HOME`, so the paths print with `~`.
    let name = |p: &Path| p.file_name().unwrap().to_str().unwrap().to_string();
    let line = |p: &Path| {
        all.lines()
            .find(|l| l.contains(&name(p)))
            .unwrap()
            .to_string()
    };
    assert!(line(&paths[0]).contains("  30  last used "), "{all}");
    assert!(line(&paths[0]).contains("past `[log] files`"), "{all}");
    let first = all.find(&name(&paths[0])).unwrap();
    let last = all.find(&name(&paths[1])).unwrap();
    assert!(first < last, "largest first: {all}");

    let cut = rtok(
        &["agents", "junk", "list", "--bytes", "--items", "1"],
        &home,
    );
    assert!(cut.contains("+2 more (30)"), "{cut}");
    let totals = rtok(
        &["agents", "junk", "list", "--bytes", "--items", "0"],
        &home,
    );
    assert!(
        !totals.contains("last used") && !totals.contains("more ("),
        "{totals}"
    );
    let path_order = rtok(
        &["agents", "junk", "list", "--sort", "path", "--items", "all"],
        &home,
    );
    let (a, b) = (
        path_order.find(&name(&paths[0])),
        path_order.find(&name(&paths[1])),
    );
    assert!(a < b, "by path: {path_order}");
}

#[test]
fn json_carries_every_item_and_min_size_limits_both_list_and_clear() {
    let (home, paths) = stray_logs("min-size");
    let json = rtok(&["agents", "junk", "list", "--json", "--items", "1"], &home);
    let report: serde_json::Value = serde_json::from_str(&json).unwrap();
    let own = report["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "rtok")
        .unwrap();
    let items: Vec<&serde_json::Value> = own["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] == "log")
        .collect();
    assert_eq!(items.len(), 3, "--items does not cut the JSON: {json}");
    assert_eq!(items[0]["size_bytes"], 30);
    assert_eq!(
        items[0]["path"],
        paths[0].display().to_string(),
        "JSON keeps the full path"
    );
    assert_eq!(items[0]["will_clear"], true);
    assert!(items[0]["last_used"].is_i64(), "{json}");
    assert!(
        items[0]["reason"]
            .as_str()
            .unwrap()
            .contains("past `[log] files`")
    );

    let big = rtok(
        &["agents", "junk", "list", "--json", "--min-size", "25"],
        &home,
    );
    let big: serde_json::Value = serde_json::from_str(&big).unwrap();
    let own = big["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "rtok")
        .unwrap();
    assert_eq!(
        own["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["kind"] == "log")
            .count(),
        1
    );

    let plan = rtok(
        &[
            "agents",
            "junk",
            "clear",
            "--kind",
            "log",
            "--min-size",
            "15",
            "--json",
        ],
        &home,
    );
    let plan: serde_json::Value = serde_json::from_str(&plan).unwrap();
    assert_eq!(plan["items"].as_array().unwrap().len(), 2, "{plan}");
    assert!(plan["items"][0]["last_used"].is_i64() && plan["items"][0]["reason"].is_string());
    assert!(
        paths.iter().all(|p| p.exists()),
        "a dry run removes nothing"
    );
    // `clear` never removes what changed in the last minute.
    paths.iter().for_each(|p| age(p, 3_600));
    rtok(
        &[
            "agents",
            "junk",
            "clear",
            "--kind",
            "log",
            "--min-size",
            "15",
            "--yes",
        ],
        &home,
    );
    assert!(
        !paths[0].exists() && !paths[2].exists() && paths[1].exists(),
        "only the items at least 15 B"
    );
}

#[test]
fn doctor_reports_reclaimable_space_in_its_text_only() {
    let (home, _) = stray_logs("doctor");
    let text = rtok(&["doctor"], &home);
    assert!(text.contains("junk\n  reclaimable: "), "{text}");
    assert!(
        !text.contains("rtok agents junk list"),
        "no hint under 1 GB: {text}"
    );
    assert!(!rtok(&["doctor", "--json"], &home).contains("reclaimable"));
}
