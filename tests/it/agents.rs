// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T25.2: `rtok agents sessions` renders the operator model's Sessions page (T25.1) as a
//! table. Against a fixture store — two live sessions across two hosts and one ended —
//! this pins the Check: two rows by default, three with `--all`, the token columns sum to
//! exactly what `rtok stats` prints over the same window, the `agents` alias spells the
//! same command, and an empty store prints the header plus a line saying nothing is
//! running.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rtok")
}

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-t252-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn rtok(args: &[&str], home: &Path) -> String {
    let out = Command::new(bin())
        .args(args)
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "rtok {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Two live sessions across the two hosts the migrations seed (`claude`, `pi`) and one
/// ended — the shape of the T25.1 store test, driven through the public API so the
/// command reads what the model reads. `live-a` spends (in, cc, cr, out) = (30, 1, 7, 7),
/// `gone-c` (7, 2, 0, 1), `live-b` nothing yet. The transcript files mirror the store's
/// `usage` rows turn for turn, so `rtok stats` (which counts transcripts) and
/// `agent sessions` (which sums `usage` rows) agree on this fixture — the same window,
/// both definitions of the numbers.
fn seed(home: &Path) {
    let projects = home.join(".claude/projects/acme");
    fs::create_dir_all(&projects).unwrap();
    fs::write(
        projects.join("live-a.jsonl"),
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"one\"}],\"usage\":{\"input_tokens\":10,\"cache_creation_input_tokens\":1,\"cache_read_input_tokens\":2,\"output_tokens\":3}}}\n\
         {\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"two\"}],\"usage\":{\"input_tokens\":20,\"cache_read_input_tokens\":5,\"output_tokens\":4}}}\n",
    )
    .unwrap();
    fs::write(
        projects.join("gone-c.jsonl"),
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"bye\"}],\"usage\":{\"input_tokens\":7,\"cache_creation_input_tokens\":2,\"output_tokens\":1}}}\n",
    )
    .unwrap();
    let cfg = rtok::config::Config::load_from(home).expect("config");
    let store = rtok::store::Store::open(&cfg.core.db_path).expect("store");
    let claude = store.host_id("claude").unwrap().expect("0002 seeds claude");
    let pi = store.host_id("pi").unwrap().expect("0010 seeds pi");
    store
        .upsert_session("live-a", Some(claude), Some("rtok"), None, Some("proxy"))
        .unwrap();
    store
        .upsert_session("live-b", Some(pi), Some("rtok"), None, None)
        .unwrap();
    store
        .upsert_session("gone-c", Some(pi), None, None, None)
        .unwrap();
    let (pid, mid) = store.upsert_model("anthropic", "claude-x").unwrap();
    let call_a = store
        .insert_call(
            "live-a",
            "proxy",
            "api_request",
            Some(claude),
            Some(pid),
            Some(mid),
            None,
            Some("/v1/messages"),
        )
        .unwrap();
    let call_c = store
        .insert_call(
            "gone-c",
            "proxy",
            "api_request",
            Some(pi),
            None,
            None,
            None,
            Some("/v1/chat/completions"),
        )
        .unwrap();
    store
        .insert_usage("live-a", Some("claude-x"), "anthropic", 10, 1, 2, 3, call_a)
        .unwrap();
    store
        .insert_usage("live-a", Some("claude-x"), "anthropic", 20, 0, 5, 4, call_a)
        .unwrap();
    store
        .insert_usage("gone-c", Some("gpt-x"), "openai_chat", 7, 2, 0, 1, call_c)
        .unwrap();
    store
        .end_session("gone-c", rtok::log::now() as i64)
        .unwrap();
}

/// Every line cut at the header's `started` column: `run` and `seen` tick between two CLI
/// runs, and everything before `started` lines up the same when the rows match.
fn without_run_col(s: &str) -> String {
    let at = s
        .lines()
        .next()
        .and_then(|h| h.find("started"))
        .unwrap_or(0);
    s.lines()
        .map(|line| line.get(..at).unwrap_or(line).trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The four token columns of one data row: everything before the `started` column — the
/// only cell that contains a space — split on whitespace. The index comes from the
/// header, whose columns line up with the rows' by construction.
fn token_columns(line: &str, header: &str) -> Vec<i64> {
    let at = header.find("started").expect("header names started");
    line[..at]
        .split_whitespace()
        .skip(4) // agent, host, provider, model
        .map(|n| n.parse().expect("numbers before started"))
        .collect()
}

#[test]
fn two_live_and_one_ended_is_two_rows_three_with_all() {
    let h = home("rows");
    seed(&h);

    let out = rtok(&["agents", "sessions"], &h);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3, "header plus the two live rows:\n{out}");
    assert!(lines[0].starts_with("agent"), "header first:\n{out}");
    assert!(out.contains("claude") && out.contains("anthropic"), "{out}");
    assert!(
        !out.contains("gpt-x") && !out.contains("openai_chat"),
        "the ended session is hidden without --all:\n{out}"
    );

    let all = rtok(&["agents", "sessions", "--all"], &h);
    let all_lines: Vec<&str> = all.lines().collect();
    assert_eq!(all_lines.len(), 4, "the ended row joins:\n{all}");
    assert!(all.contains("gpt-x"), "{all}");

    // `agents` is the visible alias (P25): same rows. Drop the `run` duration so a
    // second ticking between the two invocations cannot flake `0s` vs `1s`.
    assert_eq!(
        without_run_col(&rtok(&["agents", "sessions", "--all"], &h)),
        without_run_col(&all)
    );
    let _ = fs::remove_dir_all(&h);
}

#[test]
fn the_token_columns_equal_rtok_stats_over_the_same_window() {
    let h = home("parity");
    seed(&h);

    let all = rtok(&["agents", "sessions", "--all"], &h);
    let mut lines = all.lines();
    let header = lines.next().unwrap().to_string();
    let mut sums = [0i64; 4];
    for line in lines {
        let cols = token_columns(line, &header);
        assert_eq!(cols.len(), 4, "in/out/cache_read/cache_create:\n{all}");
        for (sum, col) in sums.iter_mut().zip(cols) {
            *sum += col;
        }
    }
    // `stats`' `usage_*` counts the transcripts the fixture wrote to mirror the store's
    // `usage` rows — the same window `--all` (since = 0) lists, so the two definitions
    // of the numbers must agree on it.
    let stats: serde_json::Value =
        serde_json::from_str(&rtok(&["stats", "--json"], &h)).expect("stats json");
    assert_eq!(
        sums,
        [
            stats["usage_input"].as_i64().unwrap(),
            stats["usage_output"].as_i64().unwrap(),
            stats["usage_cache_read"].as_i64().unwrap(),
            stats["usage_cache_create"].as_i64().unwrap()
        ],
        "sessions' token columns vs stats' usage:\n{all}"
    );
    let _ = fs::remove_dir_all(&h);
}

#[test]
fn an_empty_store_prints_the_header_and_nothing_is_running() {
    let h = home("empty");
    let out = rtok(&["agents", "sessions"], &h);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "header plus the line:\n{out}");
    assert!(lines[0].starts_with("agent"), "header first:\n{out}");
    assert_eq!(lines[1], "nothing is running");
    let _ = fs::remove_dir_all(&h);
}

/// T25.3's Check, live: a session started while `watch` runs appears within one
/// interval and the table repeats plain (no escapes) whenever state changes —
/// including the duration ticking over with no new rows at all.
#[test]
fn watch_shows_a_session_started_mid_run_and_repeats_plain_tables() {
    use std::io::BufRead;
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    let h = home("watch");
    // One live session (`watch-old`) and one ended (`watch-gone`, hidden without
    // `--all`): the watch must show the first and never the second.
    {
        let cfg = rtok::config::Config::load_from(&h).expect("config");
        let store = rtok::store::Store::open(&cfg.core.db_path).expect("store");
        let claude = store.host_id("claude").unwrap().expect("0002 seeds claude");
        let pi = store.host_id("pi").unwrap().expect("0010 seeds pi");
        store
            .upsert_session("live-a", Some(claude), Some("rtok"), None, Some("proxy"))
            .unwrap();
        store
            .upsert_session("gone-c", Some(pi), None, None, None)
            .unwrap();
        let (pid, mid) = store.upsert_model("anthropic", "watch-old").unwrap();
        let call_a = store
            .insert_call(
                "live-a",
                "proxy",
                "api_request",
                Some(claude),
                Some(pid),
                Some(mid),
                None,
                Some("/v1/messages"),
            )
            .unwrap();
        let call_c = store
            .insert_call(
                "gone-c",
                "proxy",
                "api_request",
                Some(pi),
                None,
                None,
                None,
                Some("/v1/chat/completions"),
            )
            .unwrap();
        store
            .insert_usage(
                "live-a",
                Some("watch-old"),
                "anthropic",
                10,
                0,
                0,
                1,
                call_a,
            )
            .unwrap();
        store
            .insert_usage(
                "gone-c",
                Some("watch-gone"),
                "openai_chat",
                7,
                0,
                0,
                1,
                call_c,
            )
            .unwrap();
        store
            .end_session("gone-c", rtok::log::now() as i64)
            .unwrap();
    }

    let mut child = Command::new(bin())
        .args(["agents", "sessions", "watch"])
        .env("RTOK_HOME", &h)
        .env("HOME", &h)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            match line {
                Ok(l) => sink.lock().unwrap().push(l),
                Err(_) => break,
            }
        }
    });

    let wait_for = |want: &str| {
        let deadline = Instant::now() + rtok::log::WATCH_POLL * 20;
        loop {
            if seen.lock().unwrap().iter().any(|l| l.contains(want)) {
                return;
            }
            if Instant::now() > deadline {
                panic!("watch never showed {want:?}: {:?}", seen.lock().unwrap());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    };

    // The first screen: the live row, newest first, header first.
    wait_for("watch-old");
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("watch-gone")),
        "ended stays hidden without --all: {:?}",
        seen.lock().unwrap()
    );

    // Another process (this test) starts a session: it appears within one
    // interval (20 polls of slack for CI, not part of the contract).
    {
        let cfg = rtok::config::Config::load_from(&h).expect("config");
        let store = rtok::store::Store::open(&cfg.core.db_path).expect("store");
        let pi = store.host_id("pi").unwrap().expect("0010 seeds pi");
        store
            .upsert_session("live-b", Some(pi), Some("rtok"), None, None)
            .unwrap();
        let call_b = store
            .insert_call(
                "live-b",
                "proxy",
                "api_request",
                Some(pi),
                None,
                None,
                None,
                Some("/v1/chat/completions"),
            )
            .unwrap();
        store
            .insert_usage(
                "live-b",
                Some("watch-new"),
                "openai_chat",
                5,
                0,
                0,
                1,
                call_b,
            )
            .unwrap();
    }
    wait_for("watch-new");

    // The duration ticks over with no further writes: the table repeats (a new
    // header lands) because `run` advanced, which is the repaint state needs.
    let headers = || {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with("agent"))
            .count()
    };
    let before = headers();
    let deadline = Instant::now() + rtok::log::WATCH_POLL * 20;
    loop {
        if headers() > before {
            break;
        }
        if Instant::now() > deadline {
            panic!(
                "duration never repainted the table: {:?}",
                seen.lock().unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    child.kill().unwrap();
    child.wait().unwrap();
    let got = seen.lock().unwrap().join("\n");
    assert!(
        !got.contains('\x1b'),
        "a pipe gets plain repeated tables, not escapes: {got:?}"
    );
    assert!(
        got.contains("watch-old") && got.contains("watch-new"),
        "{got}"
    );
    assert!(!got.contains("watch-gone"), "ended never shown: {got}");
    assert!(headers() >= 3, "header repeats per table: {got}");
    let _ = fs::remove_dir_all(&h);
}

fn rtok_as(args: &[&str], home: &Path, agent: &str) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .env("RTOK_AGENT_ID", agent)
        .output()
        .unwrap()
}

/// T284: `sessions` carries each session's agent (short id in the table, full id and
/// paths in `--json`) with sub-agents indented under it; `show` finds an agent by id and
/// rejects a too-short prefix; `status` sets the caller's own status text. Ids are fresh
/// UUIDv4s per run, so this asserts rather than pins a golden.
#[test]
fn sessions_show_and_status_carry_the_agent_tree() {
    let h = home("t284");
    seed(&h);
    let repo = h.join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir_all(repo.join("src")).unwrap();
    let cwd = repo.join("src").to_string_lossy().into_owned();
    let (main, sub) = {
        let cfg = rtok::config::Config::load_from(&h).expect("config");
        let store = rtok::store::Store::open(&cfg.core.db_path).expect("store");
        let claude = store.host_id("claude").unwrap().unwrap();
        let main = store
            .register_agent(claude, "live-a", None, Some(&cwd), Some("Bash: cargo test"))
            .unwrap();
        let sub = store
            .register_agent(
                claude,
                "live-a",
                Some("sub-1"),
                Some(&cwd),
                Some("Read: a.rs"),
            )
            .unwrap();
        let gone = store
            .register_agent(claude, "live-a", Some("sub-2"), None, None)
            .unwrap();
        store.end_agent(&gone, rtok::log::now() as i64).unwrap();
        (main, sub)
    };

    let out = rtok(&["agents", "sessions"], &h);
    let lines: Vec<&str> = out.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.starts_with(&main[..8]))
        .unwrap_or_else(|| panic!("main agent row:\n{out}"));
    assert!(
        lines[at].contains("repo/src") && lines[at].contains("Bash: cargo test"),
        "{out}"
    );
    assert!(lines[at].contains(" live "), "{out}");
    assert!(
        lines[at + 1].starts_with(&format!("  {}", &sub[..8])),
        "sub indented:\n{out}"
    );
    // Ids minted in one second share their first 8 hex chars, so count the indented rows.
    let subs = |t: &str| t.lines().filter(|l| l.starts_with("  ")).count();
    assert_eq!(subs(&out), 1, "the ended sub-agent is hidden:\n{out}");
    assert!(
        lines.iter().any(|l| l.starts_with("- ")),
        "a session without an agent row still shows:\n{out}"
    );
    let all = rtok(&["agents", "sessions", "--all"], &h);
    assert_eq!(subs(&all), 2, "--all adds the ended sub-agent:\n{all}");
    assert!(all.contains(" ended "), "{all}");

    let json: serde_json::Value =
        serde_json::from_str(&rtok(&["agents", "sessions", "--json"], &h)).unwrap();
    let row = json
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "live-a")
        .expect("live-a");
    assert_eq!(row["agent"]["id"], main.as_str());
    assert_eq!(row["agent"]["cwd"], cwd.as_str());
    assert_eq!(row["agent"]["state"], "live");
    assert_eq!(row["agent"]["sub_agents"][0]["id"], sub.as_str());

    let set = rtok_as(&["agents", "status", "fixing\u{7} T284 "], &h, &main);
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    let shown: serde_json::Value =
        serde_json::from_str(&rtok(&["agents", "show", &main, "--json"], &h)).unwrap();
    assert_eq!(shown["status_text"], "fixing T284");
    assert_eq!(shown["model"], "claude-x");
    assert_eq!(shown["worktree"], "repo/src");
    let long = "y".repeat(121);
    let refused = rtok_as(&["agents", "status", &long], &h, &main);
    assert_eq!(refused.status.code(), Some(1), "over 120 chars is refused");
    assert!(String::from_utf8_lossy(&refused.stderr).contains("at most 120"));
    let kept: serde_json::Value =
        serde_json::from_str(&rtok(&["agents", "show", &main, "--json"], &h)).unwrap();
    assert_eq!(
        kept["status_text"], "fixing T284",
        "a refused text changes nothing"
    );
    let text = rtok(&["agents", "show", &sub], &h);
    assert!(text.contains(&format!("parent: {main}")), "{text}");

    // Ids are random (UUIDv4); the store test pins ambiguity, this pins the refusal.
    let short = rtok_as(&["agents", "show", &main[..3]], &h, "");
    assert_eq!(short.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&short.stderr).contains("at least 4"));
    let _ = fs::remove_dir_all(&h);
}
