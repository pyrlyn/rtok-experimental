// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T150: the worktree inventory against a real repository — a squash-merged branch, a
//! dirty worktree locked by another owner, and a record whose directory was deleted.

use std::path::Path;
use std::process::Command;

mod common;

use common::git::{commit, run};

use rtok::worktree::{Entry, State, git, inventory};

fn find<'a>(entries: &'a [Entry], name: &str) -> &'a Entry {
    let found = entries.iter().find(|e| e.record.path.ends_with(name));
    found.unwrap_or_else(|| panic!("{name} missing in {entries:?}"))
}

/// Claim rows store `fs::canon`. On Windows that drops a safe `\\?\` prefix that
/// `std::fs::canonicalize` keeps, so a string compare against the raw canonical path fails.
fn stored_claim_path(path: &Path) -> String {
    fs_kit::path::canonicalize(path)
        .unwrap()
        .display()
        .to_string()
}

#[test]
fn inventory_sees_a_squash_merge_a_foreign_lock_and_a_deleted_directory() {
    let tmp = rtok::testutil::tmp_dir("worktree");
    run(&tmp, &["init", "-q", "--bare", "origin.git"]);
    run(&tmp, &["clone", "-q", "origin.git", "work"]);
    let work = tmp.join("work");
    commit(&work, "a.txt");
    run(&work, &["push", "-q", "-u", "origin", "main"]);
    run(&work, &["remote", "set-head", "origin", "main"]);
    assert_eq!(git::default_base(&work), "origin/main");

    run(
        &work,
        &["worktree", "add", "-q", "-b", "t1-merged", "../wt-merged"],
    );
    commit(&tmp.join("wt-merged"), "b.txt");
    let reason = "Cursor / grok | t2 | 2026-09-22";
    let locked = ["worktree", "add", "-q", "--lock", "--reason", reason];
    run(
        &work,
        &[&locked[..], &["-b", "t2-open", "../wt-open"]].concat(),
    );
    commit(&tmp.join("wt-open"), "c.txt");
    std::fs::write(tmp.join("wt-open/untracked.txt"), "x").unwrap();
    run(
        &work,
        &[&locked[..], &["-b", "t3-gone", "../wt-gone"]].concat(),
    );
    std::fs::remove_dir_all(tmp.join("wt-gone")).unwrap();

    run(&work, &["merge", "-q", "--squash", "t1-merged"]);
    run(&work, &["commit", "-q", "-m", "squash t1"]);
    run(&work, &["push", "-q", "origin", "main"]);
    let merged = run(&work, &["branch", "--merged", "origin/main"]);
    assert!(
        !merged.contains("t1-merged"),
        "git cannot see a squash merge: {merged}"
    );

    let entries = inventory(&work).unwrap();
    let state = |name: &str| find(&entries, name);
    assert_eq!(state("work").state, State::Main);
    assert_eq!(state("wt-merged").state, State::Merged);
    assert!(!state("wt-merged").record.held_against(None));

    let open = state("wt-open");
    assert_eq!(open.state, State::Dirty);
    assert_eq!(open.record.branch.as_deref(), Some("t2-open"));
    assert_eq!(open.record.owner().unwrap().task, "t2");
    assert!(open.record.held_against(Some("Claude Code / sonnet")));
    assert!(!open.record.held_against(Some("Cursor / grok")));

    // Locked, so git never marks it prunable — the missing directory is the only signal.
    let gone = state("wt-gone");
    assert_eq!((gone.state, &gone.record.prunable), (State::Stale, &None));

    // A committed, unmerged branch in a clean worktree.
    std::fs::remove_file(tmp.join("wt-open/untracked.txt")).unwrap();
    assert_eq!(
        find(&inventory(&work).unwrap(), "wt-open").state,
        State::Unmerged
    );
}

fn rtok(cwd: &Path, args: &[&str]) -> std::process::Output {
    rtok_in(cwd, cwd, args, b"")
}

/// `rtok` with its store under `home`, fed `stdin`.
fn rtok_in(home: &Path, cwd: &Path, args: &[&str], stdin: &[u8]) -> std::process::Output {
    rtok_as(home, cwd, None, args, stdin)
}

/// [`rtok_in`] as the rtok agent `agent` (`RTOK_AGENT_ID`); `None` clears the caller's own.
fn rtok_as(
    home: &Path,
    cwd: &Path,
    agent: Option<&str>,
    args: &[&str],
    stdin: &[u8],
) -> std::process::Output {
    use std::io::Write as _;
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rtok"));
    match agent {
        Some(id) => cmd.env("RTOK_AGENT_ID", id),
        None => cmd.env_remove("RTOK_AGENT_ID"),
    };
    let mut child = cmd
        .current_dir(cwd)
        .env("HOME", home)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("rtok spawns");
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().expect("rtok runs")
}

/// A successful `rtok … --json` run, parsed.
fn json(cwd: &Path, args: &[&str]) -> serde_json::Value {
    json_in(cwd, cwd, args)
}

fn json_in(home: &Path, cwd: &Path, args: &[&str]) -> serde_json::Value {
    let out = rtok_in(home, cwd, args, b"");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "rtok {args:?}: {err}");
    serde_json::from_slice(&out.stdout).unwrap()
}

fn by_name<'a>(rows: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    let found = rows.as_array().unwrap().iter().find(|r| {
        let path = r["path"].as_str().or(r["worktree"].as_str()).unwrap();
        Path::new(path).ends_with(name)
    });
    found.unwrap_or_else(|| panic!("{name} missing in {rows}"))
}

/// T151: `rtok worktree list` splits tagged build cache from source and finds the
/// directory git no longer lists.
#[test]
fn list_splits_cache_from_source_and_reports_an_orphan() {
    let tmp = rtok::testutil::tmp_dir("worktree-list");
    run(&tmp, &["init", "-q", "work"]);
    let work = tmp.join("work");
    commit(&work, "a.txt");
    // Shared by every worktree of the repository, like a committed `.gitignore`.
    std::fs::write(work.join(".git/info/exclude"), "target/\nout/\n").unwrap();
    for name in ["cache", "dirty", "orphan"] {
        let (branch, path) = (format!("t-{name}"), format!("../wt-{name}"));
        run(&work, &["worktree", "add", "-q", "-b", &branch, &path]);
    }
    let cache = tmp.join("wt-cache");
    std::fs::create_dir_all(cache.join("target/debug")).unwrap();
    std::fs::create_dir_all(cache.join("out")).unwrap();
    let tag = "Signature: 8a477f597d28d172789f06886806bc55\n";
    std::fs::write(cache.join("target/CACHEDIR.TAG"), tag).unwrap();
    std::fs::write(cache.join("target/debug/bin"), [0; 4096]).unwrap();
    // Ignored like `target/`, but untagged: nothing says it is safe to delete.
    std::fs::write(cache.join("out/report"), [0; 100]).unwrap();
    std::fs::write(tmp.join("wt-dirty/new.txt"), "x").unwrap();
    // The record is gone (pruned, or the repository moved); the directory stays.
    std::fs::remove_dir_all(work.join(".git/worktrees/wt-orphan")).unwrap();

    // From a linked worktree: git resolves the repository, whichever checkout we stand in.
    // `$HOME` stays outside it: the `[worktree] enabled` gate loads the config first, and a
    // fresh `~/.rtok` inside `wt-cache` would make it dirty.
    let home = tmp.join("home");
    let rows = json_in(&home, &cache, &["worktree", "list", "--json", "--sizes"]);
    let row = |name: &str| by_name(&rows, name);
    assert_eq!(rows.as_array().unwrap().len(), 4, "{rows}");
    assert_eq!(row("work")["state"], "main");
    assert_eq!(row("work")["cache_bytes"], 0);
    assert_eq!(row("wt-dirty")["state"], "dirty");

    let cached = row("wt-cache");
    assert_eq!(cached["state"], "unmerged");
    assert_eq!(cached["branch"], "t-cache");
    assert_eq!(cached["cache_bytes"], 4096 + tag.len() as u64);
    let source = cached["source_bytes"].as_u64().unwrap();
    assert!((100..4096).contains(&source), "{source}");
    assert!(cached["modified_unix"].as_u64().unwrap() > 0);

    let orphan = row("wt-orphan");
    assert_eq!(orphan["state"], "orphan");
    assert!(orphan["branch"].is_null() && orphan["owner"].is_null());

    let table = rtok(&work, &["worktree", "list"]);
    let table = String::from_utf8_lossy(&table.stdout);
    assert!(table.starts_with("path "), "{table}");
    assert!(table.contains(" orphan "), "{table}");
    assert!(table.contains("4 worktrees: "), "{table}");
    assert!(table.contains("build cache not sized (--sizes)"), "{table}");

    // T936: without `--sizes` the build cache is not walked, so it has no bytes to report.
    let plain = json_in(&home, &cache, &["worktree", "list", "--json"]);
    let plain = by_name(&plain, "wt-cache").clone();
    assert!(plain["cache_bytes"].is_null(), "{plain}");
    assert!(plain["source_bytes"].as_u64().unwrap() < 4096, "{plain}");

    let outside = rtok(&tmp, &["worktree", "list"]);
    assert!(!outside.status.success());
    let err = String::from_utf8_lossy(&outside.stderr);
    assert!(err.contains("not a git repository"), "{err}");
}

/// T154: with no lock reason, `list` names the session the hooks saw working in a worktree —
/// one `SessionStart` is enough, a second one from the same session adds no second owner,
/// and the main checkout belongs to nobody. The lock reason still wins where there is one.
#[test]
fn list_names_the_session_the_hooks_saw_in_an_unlocked_worktree() {
    let tmp = rtok::testutil::tmp_dir("worktree-seen");
    run(&tmp, &["init", "-q", "work"]);
    let work = tmp.join("work");
    commit(&work, "a.txt");
    add(&work, "locked", Some(&format!("{ME} | t1 | 2026-09-22")));
    add(&work, "free", None);
    add(&work, "idle", None);
    let free = tmp.join("wt-free");
    let hook = |cwd: &Path, session: &str| {
        let stdin = serde_json::json!({
            "hook_event_name": "SessionStart", "session_id": session,
            "cwd": cwd, "source": "startup",
        });
        let out = rtok_in(
            &tmp,
            cwd,
            &["hook", "SessionStart"],
            stdin.to_string().as_bytes(),
        );
        assert!(out.status.success(), "hooks exit 0");
    };
    hook(&free, "sess-free-1");
    hook(&free, "sess-free-1");
    hook(&work, "sess-main");
    hook(&tmp.join("wt-locked"), "sess-locked");

    let rows = json_in(&tmp, &work, &["worktree", "list", "--json"]);
    let row = |name: &str| by_name(&rows, name);
    assert!(row("work")["session"].is_null(), "{rows}");
    assert!(row("wt-idle")["session"].is_null(), "{rows}");
    let seen = &row("wt-free")["session"];
    assert_eq!(seen["session"], "sess-free-1", "{rows}");
    assert_eq!(seen["host"], "claude");
    assert!(seen["seen_unix"].as_i64().unwrap() > 0 && seen["live"] == true);
    assert_eq!(row("wt-locked")["owner"], ME);
    assert_eq!(row("wt-locked")["session"]["session"], "sess-locked");

    let table = rtok_in(&tmp, &work, &["worktree", "list"], b"");
    let table = String::from_utf8_lossy(&table.stdout);
    assert!(table.contains("seen claude sess-fre"), "{table}");
    assert!(table.contains(ME), "{table}");
}

/// T232: the Worktrees page renders the same table `rtok worktree list` prints — a
/// locked worktree's owner shows up on the page. The Worktrees page has no
/// config-driven root (like `worktree list` itself, T151): it reads the current
/// directory, so this pins it the way `tests/graph_model.rs` pins the Graph page —
/// via cwd. nextest runs each test in its own process, so this does not leak into
/// another test's relative paths; a plain multi-threaded `cargo test` run of this
/// file would race here.
///
/// The walk behind the page is a background read like `hosts_page_text` (T231),
/// never the tick itself (T206) — this real repository's own `target/` takes tens
/// of seconds to walk, which is exactly why a tick must never block on it. So the
/// first read of a fresh process answers "reading worktrees…" and this polls, the
/// same shape `tests/hosts_model.rs` uses for "probing hosts…".
#[test]
fn worktrees_page_shows_a_locked_worktree_s_owner() {
    let tmp = rtok::testutil::tmp_dir("worktree-page");
    run(&tmp, &["init", "-q", "work"]);
    let work = tmp.join("work");
    commit(&work, "a.txt");
    add(&work, "locked", Some(&format!("{ME} | t1 | 2026-09-22")));

    let cfg = rtok::testutil::config_file_in(&tmp);
    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(&work).unwrap();
    // The read runs `git worktree list` on a background thread; on a loaded machine (parallel
    // cargo builds, a full nextest run) it has taken over 10 s, so the bound is generous.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let page = loop {
        let page = rtok::web::model::snapshot(&cfg)
            .worktrees
            .expect("the current directory reads fine");
        if page != "reading worktrees…\n" {
            break page;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the worktrees read never landed"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    std::env::set_current_dir(prev).unwrap();

    assert!(page.contains(ME), "{page}");
    assert!(page.contains("t-locked"), "{page}");
    let _ = std::fs::remove_dir_all(&tmp);
}

const ME: &str = "Claude Code / sonnet";

fn add(work: &Path, name: &str, lock: Option<&str>) {
    let (branch, path) = (format!("t-{name}"), format!("../wt-{name}"));
    let mut args = vec!["worktree", "add", "-q"];
    match lock {
        Some("") => args.push("--lock"),
        Some(reason) => args.extend(["--lock", "--reason", reason]),
        None => {}
    }
    args.extend(["-b", &branch, &path]);
    run(work, &args);
}

fn squash(work: &Path, branch: &str) {
    run(work, &["merge", "-q", "--squash", branch]);
    run(work, &["commit", "-q", "-m", branch]);
}

/// T153: `rtok worktree gc` removes what is finished, drops the records of deleted
/// directories, and stops at every lock that is not the caller's — except on a vanished
/// directory whose lock is past `--stale-lock` (T950).
#[test]
fn gc_removes_only_finished_worktrees_and_never_opens_a_foreign_lock() {
    let tmp = rtok::testutil::tmp_dir("worktree-gc");
    run(&tmp, &["init", "-q", "--bare", "origin.git"]);
    run(&tmp, &["clone", "-q", "origin.git", "work"]);
    let work = tmp.join("work");
    std::fs::write(work.join("list.txt"), "1\n2\n3\n").unwrap();
    run(&work, &["add", "list.txt"]);
    commit(&work, "a.txt");
    run(&work, &["push", "-q", "-u", "origin", "main"]);
    run(&work, &["remote", "set-head", "origin", "main"]);

    let mine = format!("{ME} | t1 | 2026-09-22");
    let theirs = "Cursor / grok | t2 | 2026-09-22";
    let plain = ["done", "adj", "dirty", "open", "gone"];
    plain.iter().for_each(|name| add(&work, name, None));
    add(&work, "mine", Some(&mine));
    add(&work, "gone-mine", Some(&mine));
    add(&work, "theirs", Some(theirs));
    add(&work, "gone-theirs", Some(theirs));
    // T950: a lock taken today on a vanished worktree still waits out `--stale-lock`.
    let today = humantime::format_rfc3339(std::time::SystemTime::now()).to_string();
    let fresh = format!("Cursor / grok | t3 | {}", &today[..10]);
    add(&work, "gone-fresh", Some(&fresh));
    add(&work, "bare-lock", Some(""));

    commit(&tmp.join("wt-done"), "done.txt");
    run(&tmp.join("wt-done"), &["push", "-q", "origin", "t-done"]);
    commit(&tmp.join("wt-mine"), "mine.txt");
    commit(&tmp.join("wt-open"), "open.txt");
    std::fs::write(tmp.join("wt-dirty/new.txt"), "x").unwrap();
    std::fs::write(tmp.join("wt-adj/list.txt"), "1\n2\n3\nx\n").unwrap();
    run(&tmp.join("wt-adj"), &["commit", "-q", "-am", "x"]);
    for branch in ["t-done", "t-mine", "t-adj"] {
        squash(&work, branch);
    }
    // The base moves on next to the merged lines: the trial merge now conflicts, and
    // only the patch-equivalence signal still sees the squash.
    std::fs::write(work.join("list.txt"), "1\n2\n3\nx\ny\n").unwrap();
    run(&work, &["commit", "-q", "-am", "y"]);
    run(&work, &["push", "-q", "origin", "main"]);
    for name in ["gone", "gone-mine", "gone-theirs", "gone-fresh"] {
        std::fs::remove_dir_all(tmp.join(format!("wt-{name}"))).unwrap();
    }
    let listed = || run(&work, &["worktree", "list", "--porcelain"]);
    let before = listed();

    // Dry run from inside a finished worktree: a full plan, and nothing changes. The store
    // lives under `tmp` — one under a worktree would make it dirty (T285: gc reads agents).
    let idle0 = ["worktree", "gc", "--json", "--owner", ME, "--idle", "0h"];
    let plan = json_in(&tmp, &tmp.join("wt-done"), &idle0);
    let planned = |name: &str| {
        let row = by_name(&plan, name);
        let (action, note) = (
            row["action"].as_str().unwrap(),
            row["note"].as_str().unwrap(),
        );
        format!("{action}: {note}")
    };
    assert_eq!(planned("work"), "keep: main checkout");
    assert_eq!(
        planned("wt-done"),
        "keep: the worktree this command runs from"
    );
    assert_eq!(planned("wt-mine"), "remove: merged, clean and idle");
    assert_eq!(planned("wt-adj"), "remove: merged, clean and idle");
    assert_eq!(planned("wt-dirty"), "keep: uncommitted changes");
    let open = "keep: not merged into the base; check `gh pr view t-open`";
    assert_eq!(planned("wt-open"), open);
    assert_eq!(planned("wt-theirs"), "keep: locked by Cursor / grok");
    assert_eq!(planned("wt-bare-lock"), "keep: locked, owner unknown");
    assert_eq!(planned("wt-gone"), "drop-record: directory is gone");
    assert_eq!(planned("wt-gone-mine"), "drop-record: directory is gone");
    // T950: a vanished worktree holds no work; only a lock younger than `--stale-lock` keeps it.
    assert_eq!(planned("wt-gone-theirs"), "drop-record: directory is gone");
    assert_eq!(planned("wt-gone-fresh"), "keep: locked by Cursor / grok");
    // T418: past `--stale-lock` a foreign lock on a merged, clean worktree is abandoned;
    // on a dirty or unmerged one it still holds.
    let stale = json_in(&tmp, &work, &[&idle0[..], &["--stale-lock", "0h"]].concat());
    let abandoned = |name: &str| by_name(&stale, name)["note"].as_str().unwrap().to_owned();
    let reclaimed = "merged, clean, abandoned lock by Cursor / grok";
    assert_eq!(abandoned("wt-theirs"), reclaimed);
    let reclaimed = "merged, clean, abandoned lock, owner unknown";
    assert_eq!(abandoned("wt-bare-lock"), reclaimed);
    assert_eq!(abandoned("wt-gone-fresh"), "directory is gone");
    assert_eq!(listed(), before);

    // No `--owner`, default idle window: only the unlocked stale record and the vanished ones
    // under a lock past `--stale-lock` may go.
    let cautious = json_in(&tmp, &work, &["worktree", "gc", "--json", "--yes"]);
    let note = |rows, name: &str| by_name(rows, name)["note"].as_str().unwrap().to_owned();
    assert_eq!(
        note(&cautious, "wt-done"),
        "modified within the idle window"
    );
    assert_eq!(note(&cautious, "wt-mine"), format!("locked by {ME}"));
    assert_eq!(note(&cautious, "wt-gone"), "removed with its branch");
    assert_eq!(note(&cautious, "wt-gone-mine"), "removed with its branch");
    assert_eq!(note(&cautious, "wt-gone-theirs"), "removed with its branch");
    assert_eq!(note(&cautious, "wt-gone-fresh"), "locked by Cursor / grok");
    assert!(!work.join(".git/worktrees/wt-gone-theirs").exists());

    let applied = json_in(&tmp, &work, &[&idle0[..], &["--yes"]].concat());
    let remote_left = "removed with its branch; remote left: git push origin --delete t-done";
    assert_eq!(note(&applied, "wt-done"), remote_left);
    let kept = [
        "work",
        "wt-dirty",
        "wt-open",
        "wt-theirs",
        "wt-bare-lock",
        "wt-gone-fresh",
    ];
    let after = listed();
    let paths: Vec<&str> = after
        .lines()
        .filter(|l| l.starts_with("worktree "))
        .collect();
    assert_eq!(paths.len(), kept.len(), "{after}");
    assert!(
        kept.iter().all(|k| paths.iter().any(|p| p.ends_with(k))),
        "{after}"
    );
    let branches = run(&work, &["branch", "--format=%(refname:short)"]);
    let mut branches: Vec<&str> = branches.lines().collect();
    branches.sort_unstable();
    let survivors = [
        "main",
        "t-bare-lock",
        "t-dirty",
        "t-gone-fresh",
        "t-open",
        "t-theirs",
    ];
    assert_eq!(branches, survivors);
    // Every admin entry left belongs to a directory, or to a lock gc may not open.
    let admin = std::fs::read_dir(work.join(".git/worktrees")).unwrap();
    let mut admin: Vec<String> = admin
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    admin.sort_unstable();
    assert_eq!(
        admin,
        [
            "wt-bare-lock",
            "wt-dirty",
            "wt-gone-fresh",
            "wt-open",
            "wt-theirs"
        ]
    );
    assert!(after.contains(&format!("locked {theirs}")), "{after}");
}

/// T158: `rtok worktree add` — the path is the only stdout line, the lock reason
/// round-trips through T150's parser, the branch has no upstream, and a second `add`
/// for the same task touches nothing.
#[test]
fn add_creates_one_locked_worktree_per_task_from_a_fresh_base() {
    let tmp = rtok::testutil::tmp_dir("worktree-add");
    run(&tmp, &["init", "-q", "--bare", "origin.git"]);
    run(&tmp, &["clone", "-q", "origin.git", "apps/rtok"]);
    let work = tmp.join("apps/rtok");
    commit(&work, "a.txt");
    run(&work, &["push", "-q", "-u", "origin", "main"]);
    run(&work, &["remote", "set-head", "origin", "main"]);
    // Only on the remote: a stale local `origin/main` would branch from the wrong commit.
    run(&tmp, &["clone", "-q", "origin.git", "other"]);
    commit(&tmp.join("other"), "b.txt");
    // T441.13.3: the host config is only written where git ignores it.
    std::fs::write(tmp.join("other/.gitignore"), ".claude/\n").unwrap();
    run(&tmp.join("other"), &["add", ".gitignore"]);
    run(&tmp.join("other"), &["commit", "-q", "-m", "ignore"]);
    run(&tmp.join("other"), &["push", "-q", "origin", "main"]);
    let tip = run(&tmp.join("other"), &["rev-parse", "HEAD"]);
    // T660.4: the default root is `<main>/.rtok/worktrees`; neither a `_worktrees/` beside the
    // repository nor `~/.rtok/worktrees` (T410) pulls the worktree there.
    std::fs::create_dir_all(tmp.join("_worktrees")).unwrap();
    let home = tmp.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let root = work.join(".rtok/worktrees");

    let owner = "Claude Code / sonnet";
    let add = ["worktree", "add", "T158", "Worktree-Add", "--owner", owner];
    let out = rtok_in(&home, &work, &add, b"");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    let printed = String::from_utf8_lossy(&out.stdout);
    let path = Path::new(printed.trim_end());
    assert_eq!(printed.lines().count(), 1, "{printed}");
    // git reports the main checkout canonicalized (`/private/var` on macOS), and the
    // root follows it.
    let expected = root.join("rtok-t158").canonicalize().unwrap();
    assert_eq!(path.canonicalize().unwrap(), expected);
    assert_eq!(run(path, &["rev-parse", "HEAD"]), tip);
    // T441.13.3: the session started in the worktree shares one Claude Code task list.
    let local: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(path.join(".claude/settings.local.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        local["env"]["CLAUDE_CODE_TASK_LIST_ID"], "origin-t158",
        "{local}"
    );
    assert_eq!(
        run(path, &["branch", "--show-current"]).trim(),
        "t158-worktree-add"
    );
    let upstream = Command::new("git")
        .args([
            "-C",
            printed.trim_end(),
            "rev-parse",
            "--abbrev-ref",
            "@{upstream}",
        ])
        .output()
        .unwrap();
    assert!(
        !upstream.status.success(),
        "the new branch must have no upstream"
    );

    let entries = inventory(&work).unwrap();
    let added = find(&entries, "rtok-t158");
    let parsed = added.record.owner().expect("the lock reason parses");
    assert_eq!(
        (parsed.owner.as_str(), parsed.task.as_str()),
        (owner, "t158")
    );
    assert!(!added.record.held_against(Some(owner)));

    let again = rtok_in(
        &home,
        &work,
        &["worktree", "add", "t158", "--owner", owner],
        b"",
    );
    // T523: a retry by the same owner prints the same path, in any spelling of the name; another
    // owner is still refused.
    let err = String::from_utf8_lossy(&again.stderr);
    assert!(again.status.success(), "{err}");
    assert_eq!(String::from_utf8_lossy(&again.stdout), printed);
    let spelled = ["worktree", "add", "T158 Worktree-Add", "--owner", owner];
    let again = rtok_in(&home, &work, &spelled, b"");
    assert_eq!(String::from_utf8_lossy(&again.stdout), printed);
    let theirs = ["worktree", "add", "t158", "--owner", "Cursor / grok"];
    let refused = rtok_in(&home, &work, &theirs, b"");
    let err = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success(), "{err}");
    assert!(
        err.contains("one worktree per task") && err.contains("locked by"),
        "{err}"
    );
    assert_eq!(inventory(&work).unwrap().len(), 2);

    // `[worktree] root` wins over the default; `~` expands.
    std::fs::write(home.join("config.toml"), "[worktree]\nroot = \"~/wt\"\n").unwrap();
    let cfg = home.join("config.toml").display().to_string();
    let out = rtok(
        &work,
        &["--config", &cfg, "worktree", "add", "t2", "--owner", owner],
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    let printed = String::from_utf8_lossy(&out.stdout);
    // Components, not a string suffix: Windows prints `\wt\rtok-t2`.
    let printed = Path::new(printed.trim_end());
    assert!(printed.ends_with("wt/rtok-t2"), "{}", printed.display());
}

/// T152: `rtok worktree clean` deletes idle tagged caches — and nothing else.
#[test]
fn clean_deletes_idle_tagged_caches_and_nothing_else() {
    let tmp = rtok::testutil::tmp_dir("worktree-clean");
    run(&tmp, &["init", "-q", "work"]);
    let work = tmp.join("work");
    commit(&work, "a.txt");
    std::fs::write(work.join(".git/info/exclude"), "target/\nout/\n").unwrap();
    for name in ["idle", "fresh", "orphan"] {
        add(&work, name, None);
    }
    std::fs::remove_dir_all(work.join(".git/worktrees/wt-orphan")).unwrap();
    let tag = "Signature: 8a477f597d28d172789f06886806bc55\n";
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 86_400);
    let cache = |dir: &Path, stale: bool| {
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        let files = ["target/CACHEDIR.TAG", "target/debug/bin"];
        std::fs::write(dir.join(files[0]), tag).unwrap();
        std::fs::write(dir.join(files[1]), [0; 4096]).unwrap();
        for file in files.iter().filter(|_| stale) {
            let file = std::fs::File::options().write(true).open(dir.join(file));
            file.unwrap().set_modified(old).unwrap();
        }
    };
    let idle = tmp.join("wt-idle");
    cache(&idle, true);
    cache(&tmp.join("wt-fresh"), false);
    cache(&tmp.join("wt-orphan"), true);
    cache(&work, true);
    // Same name, no tag: never touched.
    std::fs::create_dir_all(idle.join("out")).unwrap();
    std::fs::write(idle.join("out/report"), [0; 100]).unwrap();
    let bytes = 4096 + tag.len() as u64;

    // Dry run from the main checkout: its own cache is skipped, nothing is deleted.
    let rows = json(&work, &["worktree", "clean", "--json"]);
    let row = |name: &str| by_name(&rows, name);
    assert_eq!(rows.as_array().unwrap().len(), 4, "{rows}");
    assert_eq!(row("work")["action"], "keep");
    assert!(row("work")["note"].as_str().unwrap().contains("runs from"));
    assert_eq!(row("wt-idle")["action"], "clean");
    assert_eq!(
        (
            row("wt-idle")["cache"].as_str(),
            row("wt-idle")["bytes"].as_u64()
        ),
        (Some("target"), Some(bytes))
    );
    assert_eq!(row("wt-fresh")["action"], "keep");
    assert_eq!(row("wt-orphan")["action"], "clean");
    for dir in ["work", "wt-idle", "wt-fresh", "wt-orphan"] {
        assert!(tmp.join(dir).join("target/debug/bin").exists(), "{dir}");
    }
    let table = rtok(&work, &["worktree", "clean"]);
    let table = String::from_utf8_lossy(&table.stdout);
    assert!(table.contains("dry run: 2 caches, "), "{table}");
    // T416.2: each cache is a removal line with its file count, then the totals; `--stat` and
    // `--dry-run` print the same preview, `--json` carries the count.
    assert_eq!(row("wt-idle")["files"], 2);
    assert_eq!(row("wt-fresh")["files"], 0);
    assert!(table.contains("2 paths, 4 files"), "{table}");
    assert!(table.contains("  2 files"), "{table}");
    // The idle column counts seconds, so two runs may differ in it: compare the words with
    // every age (`9s`, `2h`) masked.
    let words = |out: &std::process::Output| -> Vec<String> {
        let age = |w: &str| {
            let (n, unit) = w.split_at(w.len().saturating_sub(1));
            !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && "smhd".contains(unit)
        };
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let mask = |w: &str| if age(w) { "AGE".into() } else { w.to_string() };
        text.split_whitespace().map(mask).collect()
    };
    for flag in ["--stat", "--dry-run"] {
        let flagged = rtok(&work, &["worktree", "clean", flag]);
        assert_eq!(words(&flagged), words(&rtok(&work, &["worktree", "clean"])));
    }

    let out = rtok(&work, &["worktree", "clean", "--yes"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // T276.2: two caches go, so a bar is due on a terminal; off one, stderr stays untouched.
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!idle.join("target").exists());
    assert!(!tmp.join("wt-orphan/target").exists());
    assert!(tmp.join("wt-fresh/target/debug/bin").exists());
    assert!(work.join("target/debug/bin").exists());
    assert!(idle.join("out/report").exists() && idle.join("a.txt").exists());

    // Named: the current worktree goes too; a stranger to the repository is refused.
    let out = rtok(&work, &["worktree", "clean", "--yes", "."]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!work.join("target").exists() && work.join("a.txt").exists());
    let out = rtok(&work, &["worktree", "clean", "--yes", ".."]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("not a worktree of this repository"), "{err}");
    assert!(tmp.join("wt-fresh/target/debug/bin").exists());
}

/// T285: the store `rtok` opens under `home`, with one fake agent row per host session.
fn agents(home: &Path, sessions: &[&str]) -> (rtok::store::Store, Vec<String>) {
    std::fs::create_dir_all(home.join(".rtok")).unwrap();
    let store = rtok::store::Store::open(&home.join(".rtok/rtok.db")).unwrap();
    let claude = store.host_id("claude").unwrap().unwrap();
    let ids = sessions
        .iter()
        .map(|s| store.register_agent(claude, s, None, None, None));
    let ids = ids.collect::<Result<Vec<_>, _>>().unwrap();
    (store, ids)
}

fn lock_of(work: &Path, name: &str) -> rtok::worktree::Owner {
    let entries = inventory(work).unwrap();
    find(&entries, name)
        .record
        .owner()
        .expect("the lock parses")
}

/// T285: `add` binds the worktree to the calling agent — `RTOK_AGENT_ID` or `--agent` — in
/// a v2 lock and a claim row; the owner defaults to the agent's host; no agent, no owner is
/// an error.
#[test]
fn add_binds_the_worktree_to_the_calling_agent() {
    let tmp = rtok::testutil::tmp_dir("worktree-add-agent");
    run(&tmp, &["init", "-q", "--bare", "origin.git"]);
    run(&tmp, &["clone", "-q", "origin.git", "rtok"]);
    let work = tmp.join("rtok");
    commit(&work, "a.txt");
    run(&work, &["push", "-q", "-u", "origin", "main"]);
    run(&work, &["remote", "set-head", "origin", "main"]);
    std::fs::create_dir_all(tmp.join("_worktrees")).unwrap();
    let (store, ids) = agents(&tmp, &["sess-add"]);
    let me = ids[0].as_str();

    let out = rtok_as(&tmp, &work, Some(me), &["worktree", "add", "t9"], b"");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = Path::new(std::str::from_utf8(&out.stdout).unwrap().trim_end());
    let lock = lock_of(&work, "rtok-t9");
    assert_eq!(
        (lock.owner.as_str(), lock.agent.as_deref()),
        ("claude", Some(me))
    );
    let claims = store.open_worktree_claims().unwrap();
    let real = stored_claim_path(path);
    assert_eq!(claims, [(real, me.to_string())]);

    let args = ["worktree", "add", "t10", "--agent", &me[..8], "--owner", ME];
    let out = rtok_as(&tmp, &work, None, &args, b"");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lock = lock_of(&work, "rtok-t10");
    assert_eq!((lock.owner.as_str(), lock.agent.as_deref()), (ME, Some(me)));

    let bare = rtok_as(&tmp, &work, None, &["worktree", "add", "t11"], b"");
    assert!(String::from_utf8_lossy(&bare.stderr).contains("--owner is required"));
    let unknown = [
        "worktree", "add", "t11", "--agent", "ffffffff", "--owner", ME,
    ];
    let unknown = rtok_as(&tmp, &work, None, &unknown, b"");
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("--agent ffffffff: unknown"));
    assert_eq!(
        inventory(&work).unwrap().len(),
        3,
        "a refused add creates nothing"
    );
}

/// T970: `worktree add` on stand-ins for a full volume and for a `git worktree add` that dies
/// half-way; what it left, as (worktree directories, local branches named `t970*`).
mod all_or_nothing {
    use super::*;
    use rtok::worktree::add::Seams;
    use rtok::worktree::claim::add_with;

    const ME: &str = "Claude Code / sonnet";

    fn plenty(_: &Path) -> std::io::Result<u64> {
        Ok(u64::MAX)
    }
    fn nearly_full(_: &Path) -> std::io::Result<u64> {
        Ok(100 << 20)
    }
    fn unreadable(_: &Path) -> std::io::Result<u64> {
        Err(std::io::Error::other("no statvfs here"))
    }
    /// git got as far as the branch and a half-filled directory, then the disk was full.
    fn dies_half_made(
        repo: &Path,
        path: &Path,
        branch: &str,
        _: &str,
        base: &str,
    ) -> anyhow::Result<()> {
        run(repo, &["branch", "--no-track", branch, base]);
        std::fs::create_dir_all(path.join("src"))?;
        std::fs::write(path.join("src/half.rs"), "x")?;
        anyhow::bail!("git worktree add: No space left on device")
    }
    /// The worktree is complete and locked; the failure comes after it.
    fn dies_after_creating(
        repo: &Path,
        path: &Path,
        branch: &str,
        reason: &str,
        base: &str,
    ) -> anyhow::Result<()> {
        git::add_locked(repo, path, branch, reason, base)?;
        anyhow::bail!("No space left on device")
    }

    fn seams(
        free: fn(&Path) -> std::io::Result<u64>,
        create: fn(&Path, &Path, &str, &str, &str) -> anyhow::Result<()>,
    ) -> Seams {
        Seams {
            free_bytes: free,
            create,
        }
    }

    fn add(work: &Path, seams: Seams, floor: u32) -> anyhow::Result<rtok::worktree::add::Plan> {
        let mut wt = rtok::config::Config::default().worktree;
        wt.min_free_mib = floor;
        add_with(
            seams,
            None,
            work,
            &wt,
            ("t970", Some("x")),
            (None, Some(ME.into())),
            false,
        )
    }

    fn left(work: &Path) -> (usize, String) {
        let dirs = std::fs::read_dir(work.join(".rtok/worktrees")).map_or(0, Iterator::count);
        (dirs, run(work, &["branch", "--list", "t970*"]))
    }

    #[test]
    fn a_nearly_full_volume_is_refused_before_anything_is_made() {
        let tmp = rtok::testutil::tmp_dir("worktree-add-full");
        let work = common::git::repo(&tmp);
        let err = add(&work, seams(nearly_full, git::add_locked), 2048).unwrap_err();
        let text = err.to_string();
        assert!(!text.contains('\n'), "{text}");
        for want in ["100 MiB free", "min_free_mib", "2048"] {
            assert!(text.contains(want), "{text}");
        }
        assert_eq!(
            (inventory(&work).unwrap().len(), left(&work)),
            (1, (0, String::new()))
        );
        assert!(!work.join(".rtok").exists(), "not even the root directory");

        // 0 turns the check off; a reading that fails never blocks.
        for (free, floor) in [
            (nearly_full as fn(&Path) -> _, 0),
            (unreadable, 2048),
            (plenty, 2048),
        ] {
            let plan = add(&work, seams(free, git::add_locked), floor).unwrap();
            assert!(plan.path.is_dir());
            run(&work, &["worktree", "unlock", plan.path.to_str().unwrap()]);
            run(&work, &["worktree", "remove", plan.path.to_str().unwrap()]);
            run(&work, &["branch", "-D", &plan.branch]);
        }
    }

    #[test]
    fn a_git_step_that_dies_leaves_no_directory_branch_or_lock_and_the_task_can_be_retried() {
        let tmp = rtok::testutil::tmp_dir("worktree-add-dies");
        let work = common::git::repo(&tmp);
        for create in [dies_half_made, dies_after_creating] {
            let err = add(&work, seams(plenty, create), 2048).unwrap_err();
            assert!(err.to_string().contains("No space left"), "{err:#}");
            assert_eq!(left(&work), (0, String::new()));
            assert_eq!(inventory(&work).unwrap().len(), 1);
            assert_eq!(
                run(&work, &["worktree", "list", "--porcelain"])
                    .matches("worktree ")
                    .count(),
                1
            );
        }
        let retry = add(&work, Seams::HOST, 2048).unwrap();
        assert!(retry.path.is_dir());
    }

    #[test]
    fn a_branch_that_was_there_before_stays() {
        let tmp = rtok::testutil::tmp_dir("worktree-add-branch");
        let work = common::git::repo(&tmp);
        run(&work, &["branch", "t970-x"]);
        let err = add(&work, Seams::HOST, 2048).unwrap_err();
        assert!(format!("{err:#}").contains("t970-x"), "{err:#}");
        assert_eq!(left(&work).1.trim(), "t970-x");
        assert_eq!(left(&work).0, 0);
    }

    #[test]
    fn an_undone_claim_is_released_and_a_kept_one_is_not() {
        let tmp = rtok::testutil::tmp_dir("worktree-add-claim");
        let (store, ids) = agents(&tmp, &["sess-undo"]);
        let path = tmp.join("wt");
        std::fs::create_dir_all(&path).unwrap();
        for keep in [false, true] {
            store
                .claim_worktree(&stored_claim_path(&path), &ids[0], "t970")
                .unwrap();
            let mut undo = rtok::worktree::add::Undo::default();
            undo.claim(&store, &path);
            if keep {
                undo.keep();
            } else {
                drop(undo);
            }
            assert_eq!(
                store.open_worktree_claims().unwrap().len(),
                usize::from(keep)
            );
        }
    }
}

/// T285: `claim` takes an unlocked worktree or its own old lock, never another owner's or
/// another agent's.
#[test]
fn claim_takes_a_free_or_own_worktree_and_refuses_a_foreign_owner() {
    let tmp = rtok::testutil::tmp_dir("worktree-claim");
    run(&tmp, &["init", "-q", "work"]);
    let work = tmp.join("work");
    commit(&work, "a.txt");
    let (_store, ids) = agents(&tmp, &["sess-me", "sess-other"]);
    let (me, other) = (ids[0].as_str(), ids[1].as_str());
    add(&work, "free", None);
    add(&work, "old", Some("claude | t2 | 2026-09-22"));
    add(&work, "foreign", Some("Cursor / grok | t3 | 2026-09-22"));
    let theirs = format!("claude | t4 | 2026-09-22 | agent {other}");
    add(&work, "other", Some(&theirs));
    let claim = |name: &str| {
        let path = tmp.join(name).display().to_string();
        rtok_as(&tmp, &work, Some(me), &["worktree", "claim", &path], b"")
    };

    for (name, task) in [("wt-free", "t"), ("wt-old", "t2")] {
        let out = claim(name);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let lock = lock_of(&work, name);
        assert_eq!(
            (lock.task.as_str(), lock.agent.as_deref()),
            (task, Some(me))
        );
    }
    for (name, held) in [("wt-foreign", "Cursor / grok"), ("wt-other", other)] {
        let out = claim(name);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success() && err.contains("not taken"), "{err}");
        assert!(err.contains(held), "{err}");
    }
    assert_eq!(lock_of(&work, "wt-other").agent.as_deref(), Some(other));
    let main = claim("work");
    assert!(String::from_utf8_lossy(&main.stderr).contains("not a linked worktree"));
}

/// T289: `adopt` binds the worktree a host's own tool made — from any directory inside it. A
/// pool the host evicts gets a store claim and no lock; any other pool gets the v2 lock. A
/// detached HEAD needs `--task`, a foreign lock is refused, and `list` shows the origin.
#[test]
fn adopt_binds_a_host_made_worktree_and_lists_its_origin() {
    let tmp = rtok::testutil::tmp_dir("worktree-adopt");
    run(&tmp, &["init", "-q", "work"]);
    let work = tmp.join("work");
    commit(&work, "a.txt");
    let (store, ids) = agents(&tmp, &["sess-me"]);
    let me = ids[0].as_str();
    let cursor = tmp.join(".cursor/worktrees/work/abc");
    let kilo = tmp.join(".kilo/worktrees/t7-x");
    let held = tmp.join(".cursor/worktrees/work/held");
    for (args, dir) in [
        (vec!["--detach"], &cursor),
        (vec!["-b", "t7-x"], &kilo),
        (
            vec![
                "--detach",
                "--lock",
                "--reason",
                "Cursor / grok | t3 | 2026-09-22",
            ],
            &held,
        ),
    ] {
        let mut cmd = vec!["worktree", "add", "-q"];
        cmd.extend(args);
        cmd.push(dir.to_str().unwrap());
        run(&work, &cmd);
    }
    let adopt = |dir: &Path, extra: &[&str]| {
        let mut args = vec!["worktree", "adopt", "--json"];
        args.extend(extra);
        rtok_as(&tmp, dir, Some(me), &args, b"")
    };

    let out = adopt(&cursor, &[]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("name it with --task"));
    let out = adopt(&held, &["--task", "t9"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("not taken"));

    let out = adopt(&cursor, &["--task", "t9"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        (got["origin"].as_str(), got["locked"].as_bool()),
        (Some("cursor"), Some(false))
    );
    let entries = inventory(&work).unwrap();
    assert!(
        find(&entries, "abc").record.locked.is_none(),
        "a Cursor-pool worktree gets no lock"
    );
    let claims = store.open_worktree_claims().unwrap();
    let want = stored_claim_path(&cursor);
    assert!(
        claims.iter().any(|(p, a)| *p == want && a == me),
        "{claims:?}"
    );

    std::fs::create_dir(kilo.join("sub")).unwrap();
    let out = adopt(&kilo.join("sub"), &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lock = lock_of(&work, "t7-x");
    assert_eq!(
        (lock.task.as_str(), lock.agent.as_deref()),
        ("t7", Some(me))
    );

    let rows = json_in(&tmp, &work, &["worktree", "list", "--json"]);
    for (name, origin) in [("work", "main"), ("abc", "cursor"), ("t7-x", "kilo")] {
        assert_eq!(by_name(&rows, name)["origin"], origin, "{name}");
    }
}

/// T285: `list --json` names the bound agent with its state — live, ended, none, or an old
/// lock without one — and `gc` keeps a live agent's merged worktree.
#[test]
fn list_shows_each_agent_s_state_and_gc_keeps_a_live_agent_s_worktree() {
    let tmp = rtok::testutil::tmp_dir("worktree-agents");
    run(&tmp, &["init", "-q", "--bare", "origin.git"]);
    run(&tmp, &["clone", "-q", "origin.git", "work"]);
    let work = tmp.join("work");
    commit(&work, "a.txt");
    run(&work, &["push", "-q", "-u", "origin", "main"]);
    run(&work, &["remote", "set-head", "origin", "main"]);
    let (store, ids) = agents(&tmp, &["sess-live", "sess-ended"]);
    store.end_agent(&ids[1], 1).unwrap();
    let v2 = |id: &str| format!("{ME} | t1 | 2026-09-22 | agent {id}");
    add(&work, "live", Some(&v2(&ids[0])));
    add(&work, "ended", Some(&v2(&ids[1])));
    add(&work, "unclaimed", None);
    add(&work, "old", Some(&format!("{ME} | t1 | 2026-09-22")));

    let rows = json_in(&tmp, &work, &["worktree", "list", "--json"]);
    let agent = |name: &str| by_name(&rows, name)["agent"].clone();
    let bound =
        |id: &str, state: &str| serde_json::json!({"id": id, "host": "claude", "state": state});
    assert_eq!(agent("wt-live"), bound(&ids[0], "live"));
    assert_eq!(agent("wt-ended"), bound(&ids[1], "ended"));
    assert!(
        agent("wt-unclaimed").is_null() && agent("wt-old").is_null(),
        "{rows}"
    );
    assert_eq!(by_name(&rows, "wt-old")["owner"], ME);
    let table = rtok_in(&tmp, &work, &["worktree", "list"], b"");
    let table = String::from_utf8_lossy(&table.stdout);
    assert!(table.contains("agent state"), "{table}");
    assert!(
        table.contains(&format!("{} claude live", &ids[0][..8])),
        "{table}"
    );

    let gc = ["worktree", "gc", "--json", "--owner", ME, "--idle", "0h"];
    let plan = json_in(&tmp, &work, &gc);
    let note = |name: &str| by_name(&plan, name)["note"].as_str().unwrap().to_owned();
    assert_eq!(note("wt-live"), format!("agent {} is live", &ids[0][..8]));
    assert_eq!(note("wt-ended"), "merged, clean and idle");
}

/// T286: `remove` takes the caller's own clean worktree — with its branch once merged, or
/// keeping an unmerged one on `--keep-branch` — releases the claim, and refuses a dirty
/// worktree, another agent's, and the one it runs from.
#[test]
fn remove_takes_only_the_caller_s_own_clean_worktree() {
    let tmp = rtok::testutil::tmp_dir("worktree-remove");
    run(&tmp, &["init", "-q", "--bare", "origin.git"]);
    run(&tmp, &["clone", "-q", "origin.git", "work"]);
    let work = tmp.join("work");
    commit(&work, "a.txt");
    run(&work, &["push", "-q", "-u", "origin", "main"]);
    run(&work, &["remote", "set-head", "origin", "main"]);
    let (store, ids) = agents(&tmp, &["sess-me", "sess-other"]);
    let (me, other) = (ids[0].as_str(), ids[1].as_str());
    let lock = |task: &str, agent: &str| format!("claude | {task} | 2026-09-27 | agent {agent}");
    add(&work, "done", Some(&lock("t1", me)));
    add(&work, "open", Some(&lock("t2", me)));
    add(&work, "dirty", Some(&lock("t3", me)));
    add(&work, "theirs", Some(&lock("t4", other)));
    add(&work, "here", None);
    commit(&tmp.join("wt-done"), "done.txt");
    commit(&tmp.join("wt-open"), "open.txt");
    std::fs::write(tmp.join("wt-dirty/new.txt"), "x").unwrap();
    squash(&work, "t-done");
    run(&work, &["push", "-q", "origin", "main"]);
    let done = tmp.join("wt-done").canonicalize().unwrap();
    store
        .claim_worktree(&done.display().to_string(), me, "t1")
        .unwrap();
    let remove = |cwd: &Path, args: &[&str]| {
        let args = [&["worktree", "remove"][..], args].concat();
        rtok_as(&tmp, cwd, Some(me), &args, b"")
    };
    let refused = |cwd: &Path, args: &[&str], why: &str| {
        let out = remove(cwd, args);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.code() == Some(1) && err.contains(why), "{err}");
    };
    let branch = |name: &str| !run(&work, &["branch", "--list", name]).is_empty();

    let out = remove(&work, &["t1"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("removed with its branch"), "{text}");
    assert!(!done.exists() && !branch("t-done"));
    assert!(store.open_worktree_claims().unwrap().is_empty());

    refused(&work, &["../wt-open"], "--keep-branch");
    assert!(tmp.join("wt-open").exists());
    let out = remove(&work, &["../wt-open", "--keep-branch", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let removed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(removed["note"], "removed; branch kept");
    assert!(!tmp.join("wt-open").exists() && branch("t-open"));

    refused(&work, &["t3"], "uncommitted or untracked");
    refused(&work, &["../wt-theirs"], other);
    refused(&tmp.join("wt-here"), &["."], "current directory");
    for name in ["wt-dirty", "wt-theirs", "wt-here"] {
        assert!(tmp.join(name).exists(), "{name}");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

/// T416.4: `remove --dry-run` names the path, branch, size and file count, keeps every refusal,
/// and leaves the worktree, its branch and its claim alone.
#[test]
fn remove_dry_run_previews_and_changes_nothing() {
    let tmp = rtok::testutil::tmp_dir("worktree-remove-dry");
    run(&tmp, &["init", "-q", "--bare", "origin.git"]);
    run(&tmp, &["clone", "-q", "origin.git", "work"]);
    let work = tmp.join("work");
    commit(&work, "a.txt");
    run(&work, &["push", "-q", "-u", "origin", "main"]);
    run(&work, &["remote", "set-head", "origin", "main"]);
    let (store, ids) = agents(&tmp, &["sess-me"]);
    let me = ids[0].as_str();
    add(
        &work,
        "done",
        Some(&format!("claude | t1 | 2026-09-27 | agent {me}")),
    );
    add(
        &work,
        "open",
        Some(&format!("claude | t2 | 2026-09-27 | agent {me}")),
    );
    commit(&tmp.join("wt-done"), "done.txt");
    commit(&tmp.join("wt-open"), "open.txt");
    squash(&work, "t-done");
    run(&work, &["push", "-q", "origin", "main"]);
    let done = tmp.join("wt-done").canonicalize().unwrap();
    store
        .claim_worktree(&done.display().to_string(), me, "t1")
        .unwrap();
    let remove = |args: &[&str]| {
        let args = [&["worktree", "remove", "--dry-run"][..], args].concat();
        rtok_as(&tmp, &work, Some(me), &args, b"")
    };

    let out = remove(&["t1"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains(&stored_claim_path(&done)), "{text}");
    assert!(text.contains("files"), "{text}");
    assert!(text.contains("branch t-done: would be deleted"), "{text}");
    assert!(done.exists() && !run(&work, &["branch", "--list", "t-done"]).is_empty());
    assert_eq!(store.open_worktree_claims().unwrap().len(), 1);

    let out = remove(&["t1", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["files"].as_u64().unwrap() > 0 && v["bytes"].as_u64().is_some(),
        "{v}"
    );

    let out = remove(&["../wt-open"]);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--keep-branch"),
        "an unmerged branch is refused in a dry run too"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// T412: what `remove` guards beyond `git worktree remove` + `git branch -d`. `--keep-branch`
/// never lets a detached HEAD's commits or uncommitted files go; ignored build output does not
/// make a worktree dirty; a merged branch `git branch -D` cannot delete (checked out in a second
/// worktree) leaves a removal that is reported as done, branch kept, claim released; an
/// unreachable `origin` is a warning, and the local base still decides.
#[test]
fn remove_guards_commits_and_reports_a_branch_it_could_not_delete() {
    let tmp = rtok::testutil::tmp_dir("worktree-remove-edges");
    let work = common::git::repo(&tmp);
    std::fs::write(work.join(".git/info/exclude"), "target/\n").unwrap();
    let (store, ids) = agents(&tmp, &["sess-me"]);
    let me = ids[0].as_str();
    add(
        &work,
        "twice",
        Some(&format!("claude | t1 | 2026-10-04 | agent {me}")),
    );
    run(
        &work,
        &["worktree", "add", "-q", "-f", "../wt-twice-2", "t-twice"],
    );
    for name in ["built", "kept", "dirty"] {
        add(&work, name, None);
    }
    std::fs::create_dir_all(tmp.join("wt-built/target")).unwrap();
    std::fs::write(tmp.join("wt-built/target/bin"), "x").unwrap();
    std::fs::write(tmp.join("wt-dirty/a.txt"), "edited").unwrap();
    run(
        &work,
        &["worktree", "add", "-q", "--detach", "../wt-detached"],
    );
    commit(&tmp.join("wt-detached"), "d.txt");
    let twice = tmp.join("wt-twice").canonicalize().unwrap();
    store
        .claim_worktree(&twice.display().to_string(), me, "t1")
        .unwrap();
    let remove = |args: &[&str]| {
        let args = [&["worktree", "remove", "--json"][..], args].concat();
        rtok_as(&tmp, &work, Some(me), &args, b"")
    };
    let removed = |args: &[&str]| {
        let out = remove(args);
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(out.status.success(), "{args:?}: {err}");
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        (v["note"].as_str().unwrap().to_owned(), err)
    };
    let branch = |name: &str| !run(&work, &["branch", "--list", name]).is_empty();

    for (path, why) in [
        ("../wt-detached", "its commits would be lost"),
        ("../wt-dirty", "uncommitted or untracked"),
    ] {
        let out = remove(&[path, "--keep-branch"]);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.code() == Some(1) && err.contains(why), "{err}");
        assert!(work.join(path).exists(), "{path}");
    }

    let (note, _) = removed(&["../wt-built"]);
    assert_eq!(note, "removed with its branch");
    assert!(!tmp.join("wt-built").exists() && !branch("t-built"));

    let (note, _) = removed(&["t1"]);
    assert!(note.starts_with("removed; branch kept: "), "{note}");
    assert!(note.contains("wt-twice-2"), "{note}");
    assert!(!twice.exists() && branch("t-twice"));
    assert!(store.open_worktree_claims().unwrap().is_empty());

    let missing = tmp.join("missing.git");
    run(
        &work,
        &["remote", "set-url", "origin", missing.to_str().unwrap()],
    );
    let (note, err) = removed(&["../wt-kept", "--keep-branch"]);
    assert_eq!(note, "removed; branch kept");
    assert!(
        err.contains("judged against the local origin/main"),
        "{err}"
    );
    assert!(!tmp.join("wt-kept").exists() && branch("t-kept"));
    let _ = std::fs::remove_dir_all(&tmp);
}

/// T453: a finished task — squash-merged, clean, with commits of its own — goes whoever holds
/// it: `remove` from a shell that names no agent opens a foreign lock, and `gc` takes it from
/// a live agent or another owner. A fresh branch under the same lock is not finished and stays.
#[test]
fn a_finished_task_goes_past_a_foreign_lock_and_a_live_agent() {
    let tmp = rtok::testutil::tmp_dir("worktree-finished");
    run(&tmp, &["init", "-q", "--bare", "origin.git"]);
    run(&tmp, &["clone", "-q", "origin.git", "work"]);
    let work = tmp.join("work");
    commit(&work, "a.txt");
    run(&work, &["push", "-q", "-u", "origin", "main"]);
    run(&work, &["remote", "set-head", "origin", "main"]);
    let (_store, ids) = agents(&tmp, &["sess-live", "sess-other"]);
    let v2 = |id: &str| format!("{ME} | t1 | 2026-10-08 | agent {id}");
    let theirs = "Cursor / grok | t9 | 2026-10-08";
    add(&work, "other-done", Some(&v2(&ids[1])));
    add(&work, "other-fresh", Some(&v2(&ids[1])));
    add(&work, "live-done", Some(&v2(&ids[0])));
    add(&work, "theirs-done", Some(theirs));
    for name in ["other-done", "live-done", "theirs-done"] {
        commit(&tmp.join(format!("wt-{name}")), &format!("{name}.txt"));
        squash(&work, &format!("t-{name}"));
    }
    run(&work, &["push", "-q", "origin", "main"]);
    let remove = |path: &str| rtok_as(&tmp, &work, None, &["worktree", "remove", path], b"");

    let out = remove("../wt-other-fresh");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.code() == Some(1) && err.contains("not removed"),
        "{err}"
    );
    let out = remove("../wt-other-done");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let opened = format!(
        "removed with its branch; finished, so the lock by {} was opened",
        v2(&ids[1])
    );
    assert!(text.contains(&opened), "{text}");
    assert!(!tmp.join("wt-other-done").exists());

    let gc = ["worktree", "gc", "--json", "--idle", "0h"];
    let plan = json_in(&tmp, &work, &gc);
    let planned = |name: &str| {
        let row = by_name(&plan, name);
        format!(
            "{}: {}",
            row["action"].as_str().unwrap(),
            row["note"].as_str().unwrap()
        )
    };
    let live = format!(
        "remove: finished task, idle, though agent {} is live",
        &ids[0][..8]
    );
    assert_eq!(planned("wt-live-done"), live);
    let locked = "remove: finished task, idle, though locked by Cursor / grok";
    assert_eq!(planned("wt-theirs-done"), locked);
    let busy = format!("keep: agent {} is live", &ids[1][..8]);
    assert_eq!(planned("wt-other-fresh"), busy);

    json_in(&tmp, &work, &[&gc[..], &["--yes"]].concat());
    for name in ["wt-live-done", "wt-theirs-done"] {
        assert!(!tmp.join(name).exists(), "{name}");
    }
    assert!(tmp.join("wt-other-fresh").exists());
    let branches = run(&work, &["branch", "--format=%(refname:short)"]);
    let mut branches: Vec<&str> = branches.lines().collect();
    branches.sort_unstable();
    assert_eq!(branches, ["main", "t-other-fresh"]);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// T285 PR 2: MCP `worktree_add` creates the worktree for the session's linked agent (lock
/// and claim row name it, as on the CLI) and `worktree_list` shows it bound; a session that
/// is linked to no agent gets an error and creates nothing.
#[test]
fn mcp_worktree_tools_act_for_the_linked_agent() {
    let tmp = rtok::testutil::tmp_dir("worktree-mcp");
    run(&tmp, &["init", "-q", "--bare", "origin.git"]);
    run(&tmp, &["clone", "-q", "origin.git", "rtok"]);
    let work = tmp.join("rtok");
    commit(&work, "a.txt");
    run(&work, &["push", "-q", "-u", "origin", "main"]);
    run(&work, &["remote", "set-head", "origin", "main"]);
    std::fs::create_dir_all(tmp.join("_worktrees")).unwrap();
    let (store, _) = agents(&tmp, &[]);
    let claude = store.host_id("claude").unwrap().unwrap();
    let cwd = work.canonicalize().unwrap();
    let calls = [
        ("worktree_add", r#"{"task":"t9","slug":"mcp"}"#),
        ("worktree_list", "{}"),
        // T523: a retry, spelled as id plus slug in one string, is the same worktree.
        ("worktree_add", r#"{"task":"T9 mcp"}"#),
    ];
    let mut me = String::new();
    let hooks = || {
        me = store
            .register_agent(claude, "sess-mcp", None, cwd.to_str(), None)
            .unwrap();
    };
    let got = common::mcp::session(&tmp, &work, hooks, &calls);
    let (is_err, text) = &got[0];
    assert!(!is_err, "{text}");
    let added: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(added["branch"], "t9-mcp");
    assert_eq!(added["agent"], me);
    assert!(Path::new(added["path"].as_str().unwrap()).is_dir());
    let lock = lock_of(&work, "rtok-t9");
    assert_eq!(lock.agent.as_deref(), Some(me.as_str()));
    let real = stored_claim_path(Path::new(added["path"].as_str().unwrap()));
    assert_eq!(store.open_worktree_claims().unwrap(), [(real, me.clone())]);

    let (is_err, text) = &got[1];
    assert!(!is_err, "{text}");
    let rows: serde_json::Value = serde_json::from_str(text).unwrap();
    let row = by_name(&rows, "rtok-t9");
    assert_eq!(
        (&row["agent"]["id"], &row["agent"]["state"]),
        (&serde_json::json!(me), &serde_json::json!("live"))
    );

    let (is_err, text) = &got[2];
    assert!(!is_err, "{text}");
    let again: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(again["path"], added["path"]);
    assert_eq!(again["branch"], "t9-mcp");
    assert_eq!(store.open_worktree_claims().unwrap().len(), 1);

    // No agent row for this cwd: the worktree has no one to belong to.
    let elsewhere = tmp.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let got = common::mcp::session(
        &tmp,
        &elsewhere,
        || (),
        &[("worktree_add", r#"{"task":"t10"}"#)],
    );
    assert!(
        got[0].0 && got[0].1.starts_with("not linked to an agent session"),
        "{got:?}"
    );
    assert_eq!(inventory(&work).unwrap().len(), 2);
}

/// T410: `[worktree] enabled = false` turns every `rtok worktree` command, `list` included,
/// into one error, and MCP no longer offers the `worktree_*` tools.
#[test]
fn disabled_worktrees_refuse_every_command_and_drop_the_mcp_tools() {
    let tmp = rtok::testutil::tmp_dir("worktree-disabled");
    let work = origin_clone(&tmp, "rtok");
    std::fs::create_dir_all(tmp.join(".rtok")).unwrap();
    std::fs::write(
        tmp.join(".rtok/config.toml"),
        "[worktree]\nenabled = false\n",
    )
    .unwrap();
    let commands: [&[&str]; 4] = [
        &["worktree", "list"],
        &["worktree", "add", "t1", "--owner", "me"],
        &["worktree", "gc"],
        &["worktree", "clean"],
    ];
    for args in commands {
        let out = rtok_in(&tmp, &work, args, b"");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success() && err.contains("worktrees are not enabled"),
            "{args:?}: {err}"
        );
    }
    assert!(!work.join(".rtok").exists());

    let (store, _) = agents(&tmp, &[]);
    let claude = store.host_id("claude").unwrap().unwrap();
    let cwd = work.canonicalize().unwrap();
    let hooks = || {
        store
            .register_agent(claude, "sess-off", None, cwd.to_str(), None)
            .unwrap();
    };
    let calls = [
        ("worktree_list", "{}"),
        ("worktree_add", r#"{"task":"t1"}"#),
    ];
    for (is_err, text) in common::mcp::session(&tmp, &work, hooks, &calls) {
        assert!(
            is_err && text.starts_with("unknown tool: worktree_"),
            "{text}"
        );
    }
    assert_eq!(inventory(&work).unwrap().len(), 1);
}

/// T411: `worktree whoami` names the caller, the root, the worktree the cwd is in and the
/// worktrees bound to the caller, by lock or by claim, but not another agent's.
#[test]
fn whoami_shows_the_root_and_only_the_caller_s_worktrees() {
    let tmp = rtok::testutil::tmp_dir("worktree-whoami");
    let work = origin_clone(&tmp, "rtok");
    let (store, ids) = agents(&tmp, &["sess-me", "sess-other"]);
    let (me, other) = (ids[0].as_str(), ids[1].as_str());
    let as_me = |cwd: &Path, args: &[&str]| rtok_as(&tmp, cwd, Some(me), args, b"");
    let add = as_me(&work, &["worktree", "add", "t1", "--owner", "me"]);
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let locked = std::path::PathBuf::from(String::from_utf8_lossy(&add.stdout).trim_end());
    // Unlocked worktrees held through claim rows: one mine, one another agent's.
    let (claimed, foreign) = (tmp.join("claimed"), tmp.join("foreign"));
    for (dir, branch, agent) in [(&claimed, "c", me), (&foreign, "f", other)] {
        let path = dir.to_str().unwrap();
        run(&work, &["worktree", "add", "-q", "-b", branch, path]);
        store.claim_worktree(path, agent, branch).unwrap();
    }

    let out = as_me(&claimed, &["worktree", "whoami", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let real = |p: &serde_json::Value| Path::new(p.as_str().unwrap()).canonicalize().unwrap();
    assert_eq!(v["agent"], me);
    // T660.4: the default root is under the main checkout, which git reports canonical.
    assert_eq!(
        real(&v["root"]),
        work.join(".rtok/worktrees").canonicalize().unwrap()
    );
    assert_eq!(real(&v["here"]), claimed.canonicalize().unwrap());
    let mut held: Vec<std::path::PathBuf> = v["worktrees"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| real(&w["path"]))
        .collect();
    held.sort();
    let mut want = vec![
        locked.canonicalize().unwrap(),
        claimed.canonicalize().unwrap(),
    ];
    want.sort();
    assert_eq!(held, want);

    let out = as_me(&work, &["worktree", "whoami"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains(&format!("agent: {} (claude)\n", &me[..8])),
        "{text}"
    );
    assert!(text.contains("here: -\n"), "{text}");
    assert_eq!(text.matches("worktree: ").count(), 2, "{text}");
}

/// T286 PR 2: MCP `worktree_remove` for the linked agent: a merged clean worktree goes with
/// its branch; an unmerged one is refused until `keep_branch`; another agent's lock and a
/// call that names nothing are refused.
#[test]
fn mcp_worktree_remove_takes_only_the_linked_agent_s_clean_worktree() {
    let tmp = rtok::testutil::tmp_dir("worktree-mcp-remove");
    run(&tmp, &["init", "-q", "--bare", "origin.git"]);
    run(&tmp, &["clone", "-q", "origin.git", "work"]);
    let work = tmp.join("work");
    commit(&work, "a.txt");
    run(&work, &["push", "-q", "-u", "origin", "main"]);
    run(&work, &["remote", "set-head", "origin", "main"]);
    let (store, ids) = agents(&tmp, &["sess-other"]);
    let claude = store.host_id("claude").unwrap().unwrap();
    let cwd = work.canonicalize().unwrap();
    let register = || {
        store
            .register_agent(claude, "sess-me", None, cwd.to_str(), None)
            .unwrap()
    };
    let me = register();
    let lock = |task: &str, agent: &str| format!("claude | {task} | 2026-10-03 | agent {agent}");
    add(&work, "done", Some(&lock("t1", &me)));
    add(&work, "open", Some(&lock("t2", &me)));
    add(&work, "theirs", Some(&lock("t3", &ids[0])));
    commit(&tmp.join("wt-done"), "done.txt");
    commit(&tmp.join("wt-open"), "open.txt");
    squash(&work, "t-done");
    run(&work, &["push", "-q", "origin", "main"]);

    let calls = [
        ("worktree_remove", r#"{"task":"t1"}"#),
        ("worktree_remove", r#"{"task":"t2"}"#),
        ("worktree_remove", r#"{"task":"t2","keep_branch":true}"#),
        ("worktree_remove", r#"{"task":"t3"}"#),
        ("worktree_remove", "{}"),
    ];
    let got = common::mcp::session(&tmp, &work, || drop(register()), &calls);
    let branch = |name: &str| !run(&work, &["branch", "--list", name]).is_empty();
    assert!(!got[0].0, "{}", got[0].1);
    assert!(got[0].1.contains("removed with its branch"), "{}", got[0].1);
    assert!(!tmp.join("wt-done").exists() && !branch("t-done"));
    assert!(got[1].0 && got[1].1.contains("not merged"), "{}", got[1].1);
    assert!(!got[2].0, "{}", got[2].1);
    assert!(!tmp.join("wt-open").exists() && branch("t-open"));
    assert!(got[3].0 && got[3].1.contains("locked by"), "{}", got[3].1);
    assert!(tmp.join("wt-theirs").exists());
    assert!(got[4].0 && got[4].1.contains("required"), "{}", got[4].1);
}

/// The payload Claude Code sends a command hook (`research.md` §18.3, docs checked 2026-10-02).
fn host_payload(event: &str, session: &str, cwd: &Path, extra: serde_json::Value) -> Vec<u8> {
    let mut v = serde_json::json!({
        "session_id": session,
        "transcript_path": "/t.jsonl",
        "cwd": cwd,
        "hook_event_name": event,
    });
    v.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    v.to_string().into_bytes()
}

fn hook(
    home: &Path,
    cwd: &Path,
    event: &str,
    session: &str,
    extra: serde_json::Value,
) -> std::process::Output {
    let stdin = host_payload(event, session, cwd, extra);
    rtok_in(home, cwd, &["hook", event], &stdin)
}

fn origin_clone(tmp: &Path, name: &str) -> std::path::PathBuf {
    run(tmp, &["init", "-q", "--bare", "origin.git"]);
    run(tmp, &["clone", "-q", "origin.git", name]);
    let work = tmp.join(name);
    commit(&work, "a.txt");
    run(&work, &["push", "-q", "-u", "origin", "main"]);
    run(&work, &["remote", "set-head", "origin", "main"]);
    work
}

/// T159: `WorktreeCreate` goes through `worktree add` — one location, one name, the session's
/// agent on the lock and the claim — and answers with the path alone on stdout.
#[test]
fn worktree_create_hook_returns_the_add_path_bound_to_the_session_agent() {
    let tmp = rtok::testutil::tmp_dir("worktree-hook-create");
    let work = origin_clone(&tmp, "rtok");
    let (store, ids) = agents(&tmp, &["sess-host"]);
    let create = |name: &str| {
        let extra = serde_json::json!({"name": name});
        hook(&tmp, &work, "WorktreeCreate", "sess-host", extra)
    };

    let out = create("bold-oak-a3f2");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    let path = std::str::from_utf8(&out.stdout)
        .unwrap()
        .trim_end()
        .to_string();
    let want = work.join(".rtok/worktrees/rtok-bold-oak-a3f2");
    assert_eq!(
        Path::new(&path).canonicalize().unwrap(),
        want.canonicalize().unwrap()
    );
    let lock = lock_of(&work, "rtok-bold-oak-a3f2");
    assert_eq!(
        (lock.owner.as_str(), lock.agent.as_deref()),
        ("claude", Some(ids[0].as_str()))
    );
    let claims = store.open_worktree_claims().unwrap();
    assert_eq!(claims, [(stored_claim_path(&want), ids[0].clone())]);

    // A resumed `--worktree bold-oak-a3f2` asks again and gets the same worktree.
    let again = create("bold-oak-a3f2");
    assert_eq!(
        again.stdout,
        out.stdout,
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    assert_eq!(inventory(&work).unwrap().len(), 2);

    // A name `add` refuses fails the hook (non-zero, nothing printed); the launcher falls back.
    let bad = create("../evil");
    assert!(!bad.status.success() && bad.stdout.is_empty());
    assert_eq!(inventory(&work).unwrap().len(), 2);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// T159 (D31): whatever goes wrong inside rtok, `scripts/worktree.sh` still hands the host a
/// usable worktree at the host's own default path, and exits 0.
#[cfg(unix)]
#[test]
fn worktree_launcher_creates_a_plain_worktree_when_rtok_cannot() {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;
    use std::process::Stdio;

    let tmp = rtok::testutil::tmp_dir("worktree-hook-launcher");
    let work = origin_clone(&tmp, "rtok");
    let bin = |name: &str, body: &str| {
        let dir = tmp.join(format!("bin-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("rtok");
        std::fs::write(&file, format!("#!/bin/sh\ncat >/dev/null\n{body}\n")).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        dir
    };
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins/claude/scripts/worktree.sh");
    let launch = |dir: &Path, event: &str, extra: serde_json::Value| {
        let mut child = Command::new("/bin/sh")
            .arg(&script)
            .arg(event)
            .env("HOME", &tmp)
            .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = host_payload(event, "sess-launch", &work, extra);
        child.stdin.take().unwrap().write_all(&stdin).unwrap();
        child.wait_with_output().unwrap()
    };
    let same =
        |a: &str, b: &Path| Path::new(a).canonicalize().unwrap() == b.canonicalize().unwrap();

    // rtok fails: the plain worktree on `worktree-<name>`, a name made safe for a path.
    let failing = bin("failing", "echo boom >&2; exit 1");
    let out = launch(
        &failing,
        "WorktreeCreate",
        serde_json::json!({"name": "feature/auth"}),
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    let plain = work.join(".claude/worktrees/feature-auth");
    assert!(
        same(std::str::from_utf8(&out.stdout).unwrap().trim_end(), &plain),
        "{out:?}"
    );
    assert_eq!(
        run(&plain, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "worktree-feature-auth"
    );

    // An rtok too old to know the event answers `{}`: not a path, so the same fallback.
    let old = bin("old", "echo '{}'");
    let out = launch(
        &old,
        "WorktreeCreate",
        serde_json::json!({"name": "second"}),
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(work.join(".claude/worktrees/second/.git").exists());

    // The real rtok on PATH: the T158 location, no `.claude/worktrees` entry.
    let real = tmp.join("bin-real");
    std::fs::create_dir_all(&real).unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_rtok"), real.join("rtok")).unwrap();
    let out = launch(
        &real,
        "WorktreeCreate",
        serde_json::json!({"name": "third"}),
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    let want = work.join(".rtok/worktrees/rtok-third");
    assert!(
        same(std::str::from_utf8(&out.stdout).unwrap().trim_end(), &want),
        "{out:?}"
    );
    assert!(!work.join(".claude/worktrees/third").exists());

    // T410: `[worktree] enabled = false` — the real rtok prints no path, so the host's own
    // worktree with no "failed" warning; its remove is a plain `git worktree remove`.
    std::fs::write(
        tmp.join(".rtok/config.toml"),
        "[worktree]\nenabled = false\n",
    )
    .unwrap();
    let out = launch(&real, "WorktreeCreate", serde_json::json!({"name": "off"}));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && !err.contains("failed"), "{err}");
    let own = work.join(".claude/worktrees/off");
    assert!(
        same(std::str::from_utf8(&out.stdout).unwrap().trim_end(), &own),
        "{out:?}"
    );
    let path = serde_json::json!({"worktree_path": own});
    let gone = launch(&real, "WorktreeRemove", path);
    assert!(gone.status.success() && !own.exists(), "{gone:?}");
    std::fs::remove_file(tmp.join(".rtok/config.toml")).unwrap();

    // Remove: no rtok means plain `git worktree remove`, never forced.
    let none = tmp.join("bin-none");
    std::fs::create_dir_all(&none).unwrap();
    std::fs::write(plain.join("new.txt"), "x").unwrap();
    let path = serde_json::json!({"worktree_path": plain});
    let kept = launch(&none, "WorktreeRemove", path.clone());
    assert!(!kept.status.success() && plain.exists());
    std::fs::remove_file(plain.join("new.txt")).unwrap();
    let gone = launch(&none, "WorktreeRemove", path);
    assert!(gone.status.success() && !plain.exists());
    let _ = std::fs::remove_dir_all(&tmp);
}

/// T159: `WorktreeRemove` removes a clean worktree its session owns, keeps a dirty one and a
/// foreign-locked one with the reason on stderr, and sheds the tagged cache in all three.
#[test]
fn worktree_remove_hook_removes_only_what_the_session_owns_and_cleans_caches() {
    let tmp = rtok::testutil::tmp_dir("worktree-hook-remove");
    let work = origin_clone(&tmp, "work");
    std::fs::write(work.join(".git/info/exclude"), "target/\n").unwrap();
    let (store, ids) = agents(&tmp, &["sess-me", "sess-other"]);
    let (me, other) = (ids[0].as_str(), ids[1].as_str());
    let claude = store.host_id("claude").unwrap().unwrap();
    // A sub-agent's worktree is removed under its parent's `session_id`.
    let sub = store
        .register_agent(claude, "sess-me", Some("sub-1"), None, None)
        .unwrap();
    let lock = |task: &str, agent: &str| format!("claude | {task} | 2026-10-02 | agent {agent}");
    add(&work, "done", Some(&lock("t1", me)));
    add(&work, "open", Some(&lock("t2", me)));
    add(&work, "dirty", Some(&lock("t3", me)));
    add(&work, "theirs", Some(&lock("t4", other)));
    add(&work, "sub", Some(&lock("t5", &sub)));
    commit(&tmp.join("wt-done"), "done.txt");
    commit(&tmp.join("wt-open"), "open.txt");
    std::fs::write(tmp.join("wt-dirty/new.txt"), "x").unwrap();
    squash(&work, "t-done");
    run(&work, &["push", "-q", "origin", "main"]);
    let tag = "Signature: 8a477f597d28d172789f06886806bc55\n";
    for name in ["done", "open", "dirty", "theirs", "sub"] {
        let target = tmp.join(format!("wt-{name}/target"));
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("CACHEDIR.TAG"), tag).unwrap();
        std::fs::write(target.join("bin"), [0; 64]).unwrap();
    }
    let done = tmp.join("wt-done").canonicalize().unwrap();
    store
        .claim_worktree(&done.display().to_string(), me, "t1")
        .unwrap();
    let remove = |name: &str| {
        let path = tmp.join(format!("wt-{name}"));
        // The session's cwd is the worktree itself: the realistic case, and one `worktree
        // remove` refuses, so the hook must run from the main checkout.
        hook(
            &tmp,
            &path,
            "WorktreeRemove",
            "sess-me",
            serde_json::json!({"worktree_path": path}),
        )
    };
    let branch = |name: &str| !run(&work, &["branch", "--list", name]).is_empty();
    let kept = |name: &str, why: &str| {
        let out = remove(name);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.code() == Some(1) && err.contains(why),
            "{name}: {err}"
        );
        assert!(tmp.join(format!("wt-{name}")).exists());
        assert!(
            !tmp.join(format!("wt-{name}/target")).exists(),
            "{name}: cache stays"
        );
    };

    let out = remove("done");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty(), "the host reads only the exit code");
    assert!(!done.exists() && !branch("t-done"));
    assert!(store.open_worktree_claims().unwrap().is_empty());

    let out = remove("open");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !tmp.join("wt-open").exists() && branch("t-open"),
        "unmerged: worktree goes, branch stays"
    );

    let out = remove("sub");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!tmp.join("wt-sub").exists());

    kept("dirty", "uncommitted or untracked");
    assert!(
        tmp.join("wt-dirty/new.txt").exists(),
        "the work itself is untouched"
    );
    kept("theirs", other);

    let out = hook(
        &tmp,
        &work,
        "WorktreeRemove",
        "sess-me",
        serde_json::json!({"worktree_path": done}),
    );
    assert!(
        out.status.success(),
        "a worktree that is already gone counts as removed"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// T289.2: MCP `worktree_adopt` for the linked agent, same code as the CLI: a Cursor-pool
/// worktree is claimed without a lock, any other pool gets the v2 lock, a detached HEAD needs
/// `task`, and a foreign lock is refused.
#[test]
fn mcp_worktree_adopt_binds_a_host_made_worktree_for_the_linked_agent() {
    let tmp = rtok::testutil::tmp_dir("worktree-mcp-adopt");
    run(&tmp, &["init", "-q", "work"]);
    let work = tmp.join("work");
    commit(&work, "a.txt");
    let (store, _) = agents(&tmp, &[]);
    let claude = store.host_id("claude").unwrap().unwrap();
    let cwd = work.canonicalize().unwrap();
    let register = || {
        store
            .register_agent(claude, "sess-me", None, cwd.to_str(), None)
            .unwrap()
    };
    let me = register();
    let cursor = tmp.join(".cursor/worktrees/work/abc");
    let kilo = tmp.join(".kilo/worktrees/t7-x");
    let held = tmp.join(".cursor/worktrees/work/held");
    run(
        &work,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            cursor.to_str().unwrap(),
        ],
    );
    run(
        &work,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "t7-x",
            kilo.to_str().unwrap(),
        ],
    );
    let reason = "Cursor / grok | t3 | 2026-09-22";
    let held_args = [
        "worktree", "add", "-q", "--detach", "--lock", "--reason", reason,
    ];
    run(&work, &[&held_args[..], &[held.to_str().unwrap()]].concat());

    let call = |args: serde_json::Value| ("worktree_adopt", args.to_string());
    let calls = [
        call(serde_json::json!({"path": cursor})),
        call(serde_json::json!({"path": cursor, "task": "t9"})),
        call(serde_json::json!({"path": kilo})),
        call(serde_json::json!({"path": held, "task": "t9"})),
    ];
    let calls: Vec<(&str, &str)> = calls.iter().map(|(n, a)| (*n, a.as_str())).collect();
    let got = common::mcp::session(&tmp, &work, || drop(register()), &calls);
    assert!(
        got[0].0 && got[0].1.contains("name it with"),
        "{}",
        got[0].1
    );
    assert!(!got[1].0, "{}", got[1].1);
    let v: serde_json::Value = serde_json::from_str(&got[1].1).unwrap();
    assert_eq!(
        (v["origin"].as_str(), v["locked"].as_bool()),
        (Some("cursor"), Some(false))
    );
    assert!(!got[2].0, "{}", got[2].1);
    let lock = lock_of(&work, "t7-x");
    assert_eq!(
        (lock.task.as_str(), lock.agent.as_deref()),
        ("t7", Some(me.as_str()))
    );
    assert!(got[3].0 && got[3].1.contains("not taken"), "{}", got[3].1);
    let claims = store.open_worktree_claims().unwrap();
    let want = stored_claim_path(&cursor);
    assert!(
        claims.iter().any(|(p, a)| *p == want && *a == me),
        "{claims:?}"
    );
}

/// T289.3: `adopt` from a post-create script has no agent. The one live agent of the pool's host
/// whose cwd is the repository takes the worktree; none, two, or a lone agent of another host
/// park a claim for the next agent (T289.5), and a pool without a post-create script still needs
/// an agent.
#[test]
fn adopt_without_an_agent_binds_the_one_live_agent_of_the_pools_host() {
    let tmp = rtok::testutil::tmp_dir("worktree-adopt-unattended");
    run(&tmp, &["init", "-q", "work"]);
    let work = tmp.join("work");
    commit(&work, "a.txt");
    let (store, _) = agents(&tmp, &[]);
    let cwd = work.to_str().unwrap();
    let host = |name: &str| store.host_id(name).unwrap().unwrap();
    let wt = |name: &str| tmp.join(".cursor/worktrees/work").join(name);
    for (branch, dir) in [
        ("t7-none", wt("none")),
        ("t7-one", wt("one")),
        ("t7-two", wt("two")),
        ("t9-k", tmp.join(".kilo/worktrees/t9-k")),
        ("t8-x", tmp.join("elsewhere")),
    ] {
        run(
            &work,
            &["worktree", "add", "-q", "-b", branch, dir.to_str().unwrap()],
        );
    }
    let adopt = |dir: &Path| rtok_as(&tmp, dir, None, &["worktree", "adopt"], b"");
    let refused = |dir: &Path, why: &str| {
        let out = adopt(dir);
        assert!(!out.status.success());
        let err = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(err.contains(why), "{err}");
    };

    let parks = |dir: &Path| {
        let out = adopt(dir);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(String::from_utf8_lossy(&out.stderr).contains("pending"));
        assert!(
            store
                .pending_worktrees()
                .unwrap()
                .iter()
                .any(|(p, _)| *p == stored_claim_path(dir))
        );
    };
    parks(&wt("none"));
    let mine = store
        .register_agent(host("cursor"), "cur-1", None, Some(cwd), None)
        .unwrap();
    // Another cursor agent elsewhere, and a Claude one in the repository, do not count.
    store
        .register_agent(host("cursor"), "cur-away", None, Some("/elsewhere"), None)
        .unwrap();
    store
        .register_agent(host("claude"), "cl-1", None, Some(cwd), None)
        .unwrap();

    let out = adopt(&wt("one"));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        store.open_worktree_claims().unwrap(),
        [(stored_claim_path(&wt("one")), mine)]
    );
    parks(&tmp.join(".kilo/worktrees/t9-k"));
    refused(&tmp.join("elsewhere"), "no agent to bind");

    store
        .register_agent(host("cursor"), "cur-2", None, Some(cwd), None)
        .unwrap();
    parks(&wt("two"));
    assert_eq!(store.open_worktree_claims().unwrap().len(), 1);
    assert_eq!(store.pending_worktrees().unwrap().len(), 3);
}

/// T660.3: a worktree nested in another counts as the current one alone; the checkout around
/// it does not, so `clean` and `gc` run from inside it still act on the outer one.
#[test]
fn clean_and_gc_take_the_deepest_worktree_as_current() {
    let tmp = rtok::testutil::tmp_dir("worktree-nested-current");
    // With an origin, so a branch at the base commit reads as merged.
    let work = common::git::repo(&tmp);
    // Nested checkouts are untracked content of their parent: keep them from reading as dirty.
    std::fs::write(work.join(".git/info/exclude"), "target/\ninner/\n").unwrap();
    add(&work, "a", None);
    let (outer, nested) = (tmp.join("wt-a"), tmp.join("wt-a/inner"));
    run(&outer, &["worktree", "add", "-q", "-b", "t-inner", "inner"]);

    let tag = "Signature: 8a477f597d28d172789f06886806bc55\n";
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 86_400);
    for dir in [&outer, &nested] {
        std::fs::create_dir_all(dir.join("target")).unwrap();
        let file = dir.join("target/CACHEDIR.TAG");
        std::fs::write(&file, tag).unwrap();
        let file = std::fs::File::options().write(true).open(file).unwrap();
        file.set_modified(old).unwrap();
    }

    let rows = json_in(&tmp, &nested, &["worktree", "clean", "--json"]);
    assert_eq!(by_name(&rows, "wt-a")["action"], "clean", "{rows}");
    assert_eq!(by_name(&rows, "inner")["action"], "keep", "{rows}");

    let gc = ["worktree", "gc", "--json", "--owner", ME, "--idle", "0h"];
    let plan = json_in(&tmp, &nested, &gc);
    assert_eq!(by_name(&plan, "wt-a")["action"], "remove", "{plan}");
    assert_eq!(by_name(&plan, "inner")["action"], "keep", "{plan}");
}

/// T660.3: the symbol index a new worktree starts from leaves out main's `.rtok/` rows, where
/// the other worktrees live.
#[test]
fn a_new_worktree_inherits_no_symbol_rows_under_rtok() {
    let tmp = rtok::testutil::tmp_dir("worktree-copy-index");
    let work = common::git::repo(&tmp);
    let store = rtok::store::Store::open_in_memory().unwrap();
    let row = rtok_plugin_sdk::SymbolRow::new("main", "function", 1, true, 1, "");
    let main = rtok::store::canon_root(&work);
    for path in ["src/a.rs", ".rtok/worktrees/x/src/a.rs", ".rtokish/b.rs"] {
        let rows = std::slice::from_ref(&row);
        store
            .replace_symbols(&main, path, "sha", (0, 0), rows)
            .unwrap();
    }

    let plan = rtok::worktree::claim::add(
        Some(&store),
        &work,
        &rtok::config::Worktree {
            root: tmp.join("_worktrees"),
            min_free_mib: 0,
            ..Default::default()
        },
        ("t1", None),
        (None, Some(ME.into())),
        false,
    )
    .unwrap();

    let copied = store
        .symbol_stats(&rtok::store::canon_root(&plan.path))
        .unwrap();
    let mut paths: Vec<_> = copied.keys().map(String::as_str).collect();
    paths.sort_unstable();
    assert_eq!(paths, [".rtokish/b.rs", "src/a.rs"]);
}

/// T660.4: `.rtok/` is made ignored once, in `.gitignore` when the project has one (the rest
/// byte-for-byte, a missing final newline added first), else in `info/exclude`; a rule already
/// in effect is left alone.
#[test]
fn rtok_dir_is_ignored_through_gitignore_or_info_exclude() {
    use rtok::worktree::add::ensure_ignored;
    let tmp = rtok::testutil::tmp_dir("worktree-ignore-rtok");
    let work = common::git::repo(&tmp);
    let exclude = work.join(".git/info/exclude");
    let before = std::fs::read(&exclude).unwrap_or_default();

    std::fs::write(work.join(".gitignore"), "target\n# keep\r\n*.log").unwrap();
    ensure_ignored(&work).unwrap();
    let want = "target\n# keep\r\n*.log\n.rtok/\n";
    assert_eq!(
        std::fs::read_to_string(work.join(".gitignore")).unwrap(),
        want
    );
    ensure_ignored(&work).unwrap();
    assert_eq!(
        std::fs::read_to_string(work.join(".gitignore")).unwrap(),
        want
    );
    assert_eq!(std::fs::read(&exclude).unwrap_or_default(), before);

    std::fs::write(work.join(".gitignore"), "/.rtok\n").unwrap();
    ensure_ignored(&work).unwrap();
    assert_eq!(
        std::fs::read_to_string(work.join(".gitignore")).unwrap(),
        "/.rtok\n"
    );

    std::fs::remove_file(work.join(".gitignore")).unwrap();
    ensure_ignored(&work).unwrap();
    ensure_ignored(&work).unwrap();
    let text = std::fs::read_to_string(&exclude).unwrap();
    assert_eq!(text.matches(".rtok/").count(), 1, "{text}");
    assert!(text.ends_with(".rtok/\n"), "{text}");
    assert!(!work.join(".gitignore").exists());
}

/// T660.4: by default `rtok worktree add` puts the worktree under `<main>/.rtok/worktrees`,
/// from main and from inside a linked worktree alike, and main stays clean.
#[test]
fn the_default_root_is_under_the_main_checkout_from_main_and_from_a_worktree() {
    let tmp = rtok::testutil::tmp_dir("worktree-default-root");
    let work = common::git::repo(&tmp);
    let home = tmp.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let add = |cwd: &Path, task: &str| {
        let out = rtok_in(&home, cwd, &["worktree", "add", task, "--owner", ME], b"");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let printed = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
        Path::new(&printed).canonicalize().unwrap()
    };
    let first = add(&work, "t1");
    assert_eq!(first, work.join(".rtok/worktrees/rtok-t1"));
    std::fs::create_dir_all(first.join("sub")).unwrap();
    let second = add(&first.join("sub"), "t2");
    assert_eq!(second, work.join(".rtok/worktrees/rtok-t2"));
    assert_eq!(run(&work, &["status", "--porcelain"]), "");
    assert!(!home.join(".rtok/worktrees").exists());
}

/// T660.5: `migrate` moves the worktrees outside the root into it, named by their task, with
/// lock and claim following; it skips main, the current worktree, another agent's lock or
/// claim, a live session's worktree, one with submodules and a taken target. A dry run moves
/// nothing.
#[test]
fn migrate_moves_free_worktrees_into_the_root_and_skips_held_ones() {
    use rtok::worktree::migrate::{Held, Me, run as migrate};
    let tmp = rtok::testutil::tmp_dir("worktree-migrate");
    let work = common::git::repo(&tmp);
    let store = rtok::store::Store::open_in_memory().unwrap();
    let claude = store.host_id("claude").unwrap().unwrap();
    let agent = store.register_agent(claude, "s", None, None, None).unwrap();
    let other = store.register_agent(claude, "o", None, None, None).unwrap();
    let me = agent.as_str();
    let mine = format!("{ME} | t1 | 2026-10-14 | agent {me}");
    add(&work, "mine", Some(&mine));
    add(&work, "free", None);
    add(
        &work,
        "theirs",
        Some("Other / model | t3 | 2026-10-14 | agent agent-other"),
    );
    add(&work, "claimed", None);
    add(&work, "live", None);
    add(&work, "subs", None);
    add(&work, "here", None);
    add(&work, "taken", None);
    std::fs::write(tmp.join("wt-subs/.gitmodules"), "").unwrap();
    let root = work.join(".rtok/worktrees");
    std::fs::create_dir_all(root.join("wt-taken")).unwrap();
    let wt = |n: &str| tmp.join(format!("wt-{n}")).canonicalize().unwrap();

    let path_of = |p: &Path| p.display().to_string();
    store
        .claim_worktree(&path_of(&wt("mine")), &agent, "t1")
        .unwrap();
    store
        .claim_worktree(&path_of(&wt("claimed")), &other, "t4")
        .unwrap();
    let held = Held {
        claims: store.open_worktree_claims().unwrap(),
        pending: Vec::new(),
        live: vec![("agent-live".into(), wt("live").join("src"))],
    };
    let who = Me {
        agent: Some(me),
        owner: None,
    };
    let rel = Path::new(".rtok/worktrees");

    let dry = migrate(&wt("here"), rel, &who, &held, Some(&store), false).unwrap();
    let action = |rows: &[rtok::worktree::migrate::Outcome], n: &str| {
        let row = rows.iter().find(|o| o.path.ends_with(n)).unwrap();
        (row.action, row.note.clone())
    };
    assert_eq!(action(&dry, "wt-mine").0, "move");
    assert!(wt("mine").exists() && !root.join("rtok-t1").exists());

    let done = migrate(&wt("here"), rel, &who, &held, Some(&store), true).unwrap();
    assert_eq!(action(&done, "rtok").1, "main checkout");
    assert_eq!(action(&done, "wt-mine"), ("move", "moved".into()));
    assert_eq!(action(&done, "wt-free"), ("move", "moved".into()));
    assert_eq!(action(&done, "wt-theirs").1, "locked by Other / model");
    assert!(
        action(&done, "wt-claimed")
            .1
            .starts_with("claimed by agent")
    );
    assert!(action(&done, "wt-live").1.starts_with("live session"));
    assert!(action(&done, "wt-subs").1.contains("submodules"));
    assert_eq!(
        action(&done, "wt-here").1,
        "the worktree this command runs from"
    );
    assert!(action(&done, "wt-taken").1.ends_with("already exists"));
    assert!(done.iter().all(|o| !o.failed));

    // Named by its lock's task; without a lock it keeps its directory name.
    let moved = root.join("rtok-t1").canonicalize().unwrap();
    assert!(root.join("wt-free").is_dir() && !tmp.join("wt-free").exists());
    let records = git::list(&work).unwrap();
    let record = records
        .iter()
        .find(|r| r.path.canonicalize().unwrap() == moved);
    assert_eq!(record.unwrap().locked.as_deref(), Some(mine.as_str()));
    let claims = store.open_worktree_claims().unwrap();
    assert!(
        claims
            .iter()
            .any(|(p, a)| *p == stored_claim_path(&moved) && *a == agent),
        "{claims:?}"
    );
    assert_eq!(run(&work, &["status", "--porcelain"]), "");

    let again = migrate(&work, rel, &who, &held, Some(&store), true).unwrap();
    assert_eq!(action(&again, "rtok-t1").1, "already in the root");
}

/// T660.5: two worktrees that would land on one path: the plan moves one and keeps the other,
/// rather than letting `git worktree move` fail halfway through.
#[test]
fn migrate_gives_one_target_to_one_worktree() {
    use rtok::worktree::migrate::{Held, Me, run as migrate};
    let tmp = rtok::testutil::tmp_dir("worktree-migrate-twin");
    let work = common::git::repo(&tmp);
    for (branch, dir) in [("a", "../a/wt"), ("b", "../b/wt")] {
        run(&work, &["worktree", "add", "-q", "-b", branch, dir]);
    }
    let rel = Path::new(".rtok/worktrees");
    let plan = migrate(&work, rel, &Me::default(), &Held::default(), None, false).unwrap();
    let mut notes: Vec<_> = plan.iter().skip(1).map(|o| o.note.as_str()).collect();
    notes.sort_unstable();
    assert_eq!(notes.len(), 2, "{plan:?}");
    assert!(
        notes[0].ends_with("is another worktree's target"),
        "{plan:?}"
    );
    assert_eq!(notes[1], "outside the root");
}

/// A `gh` stand-in that logs every call to `gh.log`: `pr view` finds a PR once `pr create`
/// made one, and `pr merge` fails while `refuse-auto` exists.
#[cfg(unix)]
fn fake_gh(tmp: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = format!(
        r#"#!/bin/sh
dir='{dir}'
echo "$*" >> "$dir/gh.log"
case "$1 $2" in
"pr view") [ -f "$dir/pr" ] || exit 1; echo '{{"url":"https://example.test/pr/1","state":"OPEN"}}' ;;
"pr create") touch "$dir/pr"; echo "Creating pull request"; echo https://example.test/pr/1 ;;
"pr merge") [ -f "$dir/refuse-auto" ] && {{ echo "auto-merge is not allowed" >&2; exit 1; }}; true ;;
esac
"#,
        dir = tmp.display()
    );
    std::fs::write(bin.join("gh"), script).unwrap();
    std::fs::set_permissions(bin.join("gh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// `rtok worktree add <task> <slug>` in `work` as `agent`; the new worktree's path.
#[cfg(unix)]
fn add_as(tmp: &Path, work: &Path, agent: &str, task: &str, slug: &str) -> std::path::PathBuf {
    let out = rtok_as(
        tmp,
        work,
        Some(agent),
        &["worktree", "add", task, slug],
        b"",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim_end())
}

/// `rtok worktree finish --json` in `cwd` as `agent`, with the fake `gh` first on `PATH` and a
/// git identity of its own (`HOME` is the scratch directory).
#[cfg(unix)]
fn finish_as(tmp: &Path, cwd: &Path, agent: &str, extra: &[&str]) -> std::process::Output {
    let path = format!(
        "{}:{}",
        tmp.join("bin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new(env!("CARGO_BIN_EXE_rtok"))
        .current_dir(cwd)
        .env("HOME", tmp)
        .env("PATH", path)
        .env("RTOK_AGENT_ID", agent)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .args(["worktree", "finish", "--json"])
        .args(extra)
        .output()
        .expect("rtok runs")
}

/// T952.1: `finish` refuses credential files, another agent's lock, the main checkout and a
/// branch with nothing to hand over; otherwise it commits what is left, pushes, opens the PR
/// once (a second run reuses it), asks for auto-merge (a refusal is a warning) and unlocks
/// last.
#[cfg(unix)]
#[test]
fn finish_commits_pushes_opens_the_pr_and_unlocks_only_the_caller_s_worktree() {
    let tmp = rtok::testutil::tmp_dir("worktree-finish");
    let work = origin_clone(&tmp, "rtok");
    fake_gh(&tmp);
    let (_store, ids) = agents(&tmp, &["sess-me", "sess-them"]);
    let (me, them) = (ids[0].as_str(), ids[1].as_str());
    let add = |agent: &str, task: &str, slug: &str| add_as(&tmp, &work, agent, task, slug);
    let err = |out: &std::process::Output| String::from_utf8_lossy(&out.stderr).into_owned();
    let locked = |name: &str| {
        find(&inventory(&work).unwrap(), name)
            .record
            .locked
            .is_some()
    };

    let mine = add(me, "t1", "fin");
    std::fs::write(mine.join("b.txt"), "b").unwrap();
    std::fs::write(mine.join(".env"), "TOKEN=x").unwrap();
    let out = finish_as(&tmp, &mine, me, &[]);
    assert!(
        err(&out).contains("credential files: .env"),
        "{}",
        err(&out)
    );
    assert!(locked("rtok-t1") && git::pending_paths(&mine).unwrap().len() == 2);

    std::fs::remove_file(mine.join(".env")).unwrap();
    let out = finish_as(&tmp, &mine, me, &[]);
    assert!(out.status.success(), "{}", err(&out));
    let done: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(done["committed"], "t1: fin");
    assert_eq!(done["pr"], "https://example.test/pr/1");
    assert_eq!(
        (done["auto_merge"].as_bool(), done["unlocked"].as_bool()),
        (Some(true), Some(true))
    );
    assert!(!locked("rtok-t1"));
    let origin = tmp.join("origin.git");
    assert_eq!(
        run(&origin, &["log", "-1", "--format=%s", "t1-fin"]).trim(),
        "t1: fin"
    );

    std::fs::write(tmp.join("refuse-auto"), "").unwrap();
    let out = finish_as(&tmp, &mine, me, &[]);
    assert!(out.status.success(), "{}", err(&out));
    let again: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(again.get("committed").is_none() && again["auto_merge"] == false);
    assert!(
        again["warnings"][0]
            .as_str()
            .unwrap()
            .contains("auto-merge is not allowed")
    );
    let log = std::fs::read_to_string(tmp.join("gh.log")).unwrap();
    assert_eq!(log.matches("pr create").count(), 1, "{log}");
    assert!(
        log.contains("pr create --head t1-fin --base main --fill-first"),
        "{log}"
    );
    assert!(
        log.contains("pr merge https://example.test/pr/1 --auto --squash"),
        "{log}"
    );

    let theirs = add(them, "t2", "x");
    commit(&theirs, "c.txt");
    let out = finish_as(&tmp, &theirs, me, &[]);
    assert!(err(&out).contains("not finished"), "{}", err(&out));
    assert!(locked("rtok-t2"));

    let fresh = add(me, "t3", "empty");
    assert!(err(&finish_as(&tmp, &fresh, me, &[])).contains("nothing to finish"));
    assert!(err(&finish_as(&tmp, &work, me, &[])).contains("is the main checkout"));
    assert_eq!(
        log,
        std::fs::read_to_string(tmp.join("gh.log")).unwrap(),
        "refusals call no gh"
    );
}

/// T952.2: after `finish`, a round keeps an unmerged and a dirty worktree and a merged one
/// whose task cannot be closed; the next round closes the task, then removes the worktree,
/// its branch and its claim.
#[cfg(unix)]
#[test]
fn a_finished_worktree_goes_after_its_merge_and_its_task_closes_first() {
    use rtok::tasks::run::Project;
    use rtok::tasks::{NewTask, Status};
    use rtok::worktree::reap::{Outcome, sweep};
    let tmp = rtok::testutil::tmp_dir("worktree-reap");
    let work = origin_clone(&tmp, "rtok");
    fake_gh(&tmp);
    let (store, ids) = agents(&tmp, &["sess-me"]);
    let me = ids[0].as_str();
    let tasks = rtok::config::Tasks::default();
    let project = Project::open(&tasks, &work).unwrap();
    let new = NewTask {
        title: "reap".into(),
        description: String::new(),
        parent: None,
    };
    let task = project.create(&store, &new).unwrap().id;

    let merged = add_as(&tmp, &work, me, &task.to_string(), "merged");
    let open = add_as(&tmp, &work, me, "t2", "open");
    let dirty = add_as(&tmp, &work, me, "t3", "dirty");
    for wt in [&merged, &open, &dirty] {
        let name = wt.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(wt.join(format!("{name}.txt")), &name).unwrap();
        let out = finish_as(&tmp, wt, me, &[]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    // Keys before the removal: the stored path is canonical, and a gone path has none.
    let [merged_key, open_key, dirty_key] =
        [&merged, &open, &dirty].map(|wt| stored_claim_path(wt));
    let branch = |wt: &Path| run(wt, &["branch", "--show-current"]).trim().to_owned();
    let merged_branch = branch(&merged);
    squash(&work, &merged_branch);
    squash(&work, &branch(&dirty));
    run(&work, &["push", "-q", "origin", "main"]);
    std::fs::write(dirty.join("late.txt"), "late").unwrap();

    type Round = std::collections::HashMap<String, Result<Outcome, String>>;
    let round = |tasks: &rtok::config::Tasks| -> Round {
        let close =
            |main: &Path, task: &str| rtok::tasks::run::close_if_active(&store, tasks, main, task);
        sweep(&store, &close)
            .into_iter()
            .map(|(path, out)| (path, out.map_err(|e| format!("{e:#}"))))
            .collect()
    };
    let broken = rtok::config::Tasks {
        adapter: "nope".into(),
        ..rtok::config::Tasks::default()
    };
    let first = round(&broken);
    assert!(
        first[&merged_key]
            .clone()
            .unwrap_err()
            .contains("unknown [tasks] adapter"),
        "{first:?}"
    );
    assert_eq!(first[&open_key], Ok(Outcome::Waiting(State::Unmerged)));
    assert_eq!(first[&dirty_key], Ok(Outcome::Waiting(State::Dirty)));
    assert!(merged.exists());
    assert_eq!(project.get(&task).unwrap().status, Status::Open);

    let second = round(&tasks);
    match &second[&merged_key] {
        Ok(Outcome::Removed { closed, .. }) => assert_eq!(closed, &Some(task.to_string())),
        other => panic!("{other:?}"),
    }
    assert!(!merged.exists() && open.exists() && dirty.exists());
    assert_eq!(project.get(&task).unwrap().status, Status::Done);
    assert!(run(&work, &["branch", "--list", &merged_branch]).is_empty());
    let left: Vec<String> = store
        .finished_worktree_claims()
        .unwrap()
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    assert!(!left.contains(&merged_key), "{left:?}");
    assert_eq!(left.len(), 2, "{left:?}");
}
