// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T2.2: spawn `rtok hook PreToolUse` 200×; p95 < 10 ms (release).
//! Gate P17 asks the same of `PostToolUse`, T288 of `UserPromptSubmit`; each prints p50/p95/max under `--nocapture`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

mod common;

const N: usize = 200;
const P95_MAX: Duration = Duration::from_millis(10);

fn p95_under_10ms(event: &str, fixture: &[u8]) {
    p95_with(event, "", fixture, |_| {}, |out| out == b"{}");
}

/// `tag` keeps two cases of one event apart, as the tests of this file run on parallel threads;
/// `setup` fills the home before the first spawn; `ok` checks every hook's stdout.
fn p95_with(
    event: &str,
    tag: &str,
    fixture: &[u8],
    setup: impl FnOnce(&std::path::Path),
    ok: impl Fn(&[u8]) -> bool,
) {
    let Some(samples) = sample(event, tag, fixture, setup, ok) else {
        return;
    };
    let p95 = common::p95(&samples);
    eprintln!(
        "{event}: n={N} p50 {:?} p95 {p95:?} max {:?}",
        samples[N / 2],
        samples[N - 1]
    );
    assert!(
        p95 < P95_MAX,
        "{event} p95 {p95:?} is not < {P95_MAX:?} (min {:?}, max {:?})",
        samples[0],
        samples[N - 1]
    );
}

/// `N` sorted wall times of `rtok hook <event>`, or `None` in a debug build (see the skip line).
fn sample(
    event: &str,
    tag: &str,
    fixture: &[u8],
    setup: impl FnOnce(&std::path::Path),
    ok: impl Fn(&[u8]) -> bool,
) -> Option<Vec<Duration>> {
    if cfg!(debug_assertions) {
        eprintln!("skip: T2.2 Check is `cargo test --release latency`");
        return None;
    }

    let bin = env!("CARGO_BIN_EXE_rtok");
    let tmp =
        std::env::temp_dir().join(format!("rtok-latency-{event}{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("temp home");
    setup(&tmp);

    let spawn = || {
        let mut child = Command::new(bin)
            .args(["hook", event])
            .env("RTOK_HOME", &tmp)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn rtok");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(fixture)
            .expect("write fixture");
        child.wait_with_output().expect("wait")
    };
    let warm = spawn();
    assert!(warm.status.success(), "warmup hook must exit 0");

    let mut samples = Vec::with_capacity(N);
    for _ in 0..N {
        let start = std::time::Instant::now();
        let out = spawn();
        samples.push(start.elapsed());
        assert!(out.status.success(), "hook must fail open with exit 0");
        // SessionStart injects the agent id and the memory line; the others stay `{}`.
        if event != "SessionStart" {
            assert!(ok(&out.stdout), "{}", String::from_utf8_lossy(&out.stdout));
        }
    }

    samples.sort();
    let _ = std::fs::remove_dir_all(&tmp);
    Some(samples)
}

#[test]
fn latency_hook_pre_tool_p95_under_10ms() {
    p95_under_10ms(
        "PreToolUse",
        include_bytes!("fixtures/hooks/pre_tool_read.json"),
    );
}

#[test]
fn latency_hook_post_tool_p95_under_10ms() {
    p95_under_10ms(
        "PostToolUse",
        include_bytes!("fixtures/hooks/post_tool.json"),
    );
}

/// T920: `PostToolUse` stores one observation per tool call, and the identical fixture above
/// stores only the first (the 5 s dedup window), so it never paid for one. Each spawn here is a
/// new call. The same spawn with a tool the memory plugin skips is the baseline: the hook's own
/// start-up and writes are in both, so the median gap is what an observation adds (7 ms in
/// alpha.2/3, all of it compiling the scrub patterns, the insert is ~0.3 ms). Interleaving keeps
/// machine load out of the gap.
#[test]
fn latency_hook_post_tool_observation_adds_under_4ms() {
    if cfg!(debug_assertions) {
        eprintln!("skip: T920 Check is `cargo test --release --test latency`");
        return;
    }
    const EXTRA_MAX: Duration = Duration::from_millis(4);
    let bin = env!("CARGO_BIN_EXE_rtok");
    let tmp = std::env::temp_dir().join(format!("rtok-latency-observe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("temp home");
    let session = "t920-observe";
    let fixture: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/hooks/post_tool.json")).unwrap();
    let spawn = |tool: &str, i: usize| {
        let mut v = fixture.clone();
        v["session_id"] = session.into();
        v["tool_name"] = tool.into();
        v["tool_use_id"] = format!("toolu_{tool}_{i}").into();
        v["tool_input"]["command"] = format!("echo call {i}").into();
        let start = std::time::Instant::now();
        let mut child = Command::new(bin)
            .args(["hook", "PostToolUse"])
            .env("RTOK_HOME", &tmp)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn rtok");
        common::feed_stdin(&mut child, v.to_string().as_bytes());
        let out = child.wait_with_output().expect("wait");
        let took = start.elapsed();
        assert!(out.status.success(), "hook must fail open with exit 0");
        assert_eq!(out.stdout, b"{}");
        took
    };
    // The first spawn of each kind creates the store and runs the migrations.
    spawn("Bash", usize::MAX);
    spawn("mem_save", usize::MAX);

    let (mut observed, mut skipped) = (Vec::new(), Vec::new());
    for i in 0..N {
        observed.push(spawn("Bash", i));
        skipped.push(spawn("mem_save", i));
    }
    observed.sort();
    skipped.sort();
    let extra = observed[N / 2].saturating_sub(skipped[N / 2]);
    eprintln!(
        "PostToolUse observation: n={N} p50 {:?} vs {:?} without one, extra {extra:?}",
        observed[N / 2],
        skipped[N / 2]
    );

    // The gap only means something if the observations were stored, one per call.
    let store = rtok::store::Store::open(&tmp.join("rtok.db")).expect("open store");
    let stored = store
        .recent_observations(session, 1000)
        .expect("observations");
    assert_eq!(stored.len(), N + 1, "one observation per Bash call");
    assert!(
        extra < EXTRA_MAX,
        "an observation adds {extra:?} to PostToolUse, not < {EXTRA_MAX:?}"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// T288: `UserPromptSubmit` also reads the caller's undelivered messages (one indexed
/// query); an empty inbox still prints `{}` inside the budget.
#[test]
fn latency_hook_user_prompt_submit_p95_under_10ms() {
    p95_under_10ms(
        "UserPromptSubmit",
        include_bytes!("fixtures/hooks/user_prompt_submit.json"),
    );
}

/// T921: Claude Code measured 39% of 1,354 `SessionEnd` runs over `max_ms` (p95 93 ms, from
/// reading a long transcript for the checkpoint). The median is what `max_ms` promises; a
/// session with a small transcript must stay under it, ending the agent and the session too.
#[test]
fn latency_hook_session_end_median_under_10ms() {
    let transcript =
        std::env::temp_dir().join(format!("rtok-latency-end-{}.jsonl", std::process::id()));
    std::fs::write(
        &transcript,
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"fix the build\"}}\n",
    )
    .unwrap();
    let fixture = serde_json::json!({
        "hook_event_name": "SessionEnd", "session_id": "t921-end", "reason": "other",
        "cwd": std::env::temp_dir(), "transcript_path": transcript,
    })
    .to_string();
    let Some(samples) = sample(
        "SessionEnd",
        "",
        fixture.as_bytes(),
        |_| {},
        |out| out == b"{}",
    ) else {
        return;
    };
    let p50 = samples[N / 2];
    eprintln!("SessionEnd: n={N} p50 {p50:?} max {:?}", samples[N - 1]);
    assert!(p50 < P95_MAX, "SessionEnd p50 {p50:?} is not < {P95_MAX:?}");
    let _ = std::fs::remove_file(&transcript);
}

/// T201: guard's `post_tool` used to sha256 + write to disk synchronously on every cached
/// Read/Bash, and the hook's stdin read was unbounded — a multi-MB `PostToolUse` body paid
/// two full hashing passes plus disk I/O on top of the JSON parse. With the archive cap in
/// `src/plugins/guard/mod.rs` and the sha skip in `Store::spill`, only the (unavoidable) JSON
/// parse is left, so an in-process dispatch of a 5 MB body stays well under this looser bound.
/// Same debug skip as the p95 gates above: under `just check`'s fully parallel `nextest run`
/// every logical CPU is busy with other tests, and even this 50 ms budget is not safe from
/// that contention (measured 152 ms under full-suite load, ~1 ms in isolation) — run with
/// `cargo test --release --test latency` for a real measurement.
#[test]
fn hook_dispatches_a_5mb_post_tool_body_under_50ms() {
    if cfg!(debug_assertions) {
        eprintln!("skip: T201 Check is `cargo test --release --test latency`");
        return;
    }
    let tmp = std::env::temp_dir().join(format!("rtok-latency-5mb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("temp home");
    let mut cfg = rtok::config::Config::default();
    cfg.core.db_path = tmp.join("rtok.db");
    cfg.core.archive_dir = tmp.join("archive");

    let raw = include_str!("fixtures/hooks/post_tool.json");
    let mut v: serde_json::Value = serde_json::from_str(raw).unwrap();
    v["tool_response"]["stdout"] = serde_json::Value::String("x".repeat(5 * 1024 * 1024));
    let stdin = serde_json::to_vec(&v).unwrap();

    let mut out = Vec::new();
    let start = std::time::Instant::now();
    rtok::hooks::run("PostToolUse", stdin.as_slice(), &mut out, &cfg);
    let took = start.elapsed();

    assert!(
        serde_json::from_slice::<serde_json::Value>(&out).is_ok(),
        "hook must print valid JSON"
    );
    assert!(
        took < Duration::from_millis(50),
        "5 MB PostToolUse dispatch took {took:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// T200: a second connection holding `BEGIN EXCLUSIVE` for 500 ms must not stall
/// the hook. `hooks::run` fails open through its few-ms lock bound and still
/// prints valid JSON, well before the holder lets go.
///
/// T237: the proof that it gave up rather than waited is the ledger — a hook that
/// outwaited the holder would have written its `calls` row. The wall bound stays as
/// the second check; on Windows it is 250 ms (still half the hold): the 5 ms busy
/// handler sleeps 1 + 2 + 2 ms and each Windows `Sleep` rounds up to the 15.6 ms
/// timer tick, so the `windows-latest` debug run took 107 ms (ci run 35947867095).
///
/// T309: a fixed ms bound measured the runner, not the hook: 102.7 ms on `macos-latest` and
/// 313.7 ms on `windows-latest` under suite load (2026-09-27), both green on rerun. The bound is
/// now half the hold on every platform — a hook that returns in `HOLD / 2` cannot have waited
/// for the release — and the test runs alone (`.config/nextest.toml`). The few-ms bound on the
/// wait itself is a compile-time assert on `LOCK_WAIT` in `src/hooks/mod.rs`.
///
/// The hold is 2 s: with 500 ms the bound was 250 ms, and `windows-latest` took 341 ms under
/// load (pyrlyn/rtok#871, 2026-10-08). A longer hold widens the bound without letting a hook
/// that waited for the release pass.
#[test]
fn hook_returns_despite_exclusive_lock() {
    const HOLD: Duration = Duration::from_millis(2000);

    let tmp = std::env::temp_dir().join(format!("rtok-latency-locked-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("temp home");
    let db = tmp.join("rtok.db");
    let cfg = rtok::testutil::config_in(&tmp);

    // Warm the store once so the locked run exercises contention, not migration.
    let fixture = include_bytes!("fixtures/hooks/pre_tool_read.json");
    let mut warm = Vec::new();
    rtok::hooks::run("PreToolUse", &fixture[..], &mut warm, &cfg);
    assert!(
        serde_json::from_slice::<serde_json::Value>(&warm).is_ok(),
        "warmup hook must print valid JSON"
    );
    let calls = || {
        let store = rtok::store::Store::open(&db).unwrap();
        store.recent_calls(i64::MAX).unwrap().len()
    };
    let before = calls();
    assert!(before > 0, "the warm hook must record its call");

    let holder = rtok::store::hold_write_lock(&db, rtok::store::HeldWrite::Exclusive, HOLD);

    let start = std::time::Instant::now();
    let mut out = Vec::new();
    rtok::hooks::run("PreToolUse", &fixture[..], &mut out, &cfg);
    let took = start.elapsed();
    holder.join().unwrap();

    let v: serde_json::Value =
        serde_json::from_slice(&out).expect("locked hook must print valid JSON");
    assert!(v.is_object(), "{v}");
    assert_eq!(
        calls(),
        before,
        "the hook outwaited the exclusive lock instead of failing open ({took:?})"
    );
    assert!(
        took < HOLD / 2,
        "hook waited {took:?} under a {HOLD:?} exclusive lock"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// T329.6 / T940: `SessionStart` with a real cwd (the fixture's own cwd does not exist
/// and would skip other work). Project registration waits for a tool; this path must
/// still stay under 10 ms.
#[test]
fn latency_hook_session_start_with_project_registration_p95_under_10ms() {
    let mut v: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/hooks/session_start.json")).unwrap();
    v["cwd"] = std::env::temp_dir().to_string_lossy().into_owned().into();
    p95_under_10ms("SessionStart", v.to_string().as_bytes());
}

/// T369: with `grep_symbol` on, a symbol-shaped `Grep` in an indexed project is answered on the
/// hook path (one index lookup, one small file read) inside the same 10 ms budget.
#[test]
fn latency_hook_grep_symbol_answer_p95_under_10ms() {
    let mut v: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/hooks/pre_tool_read.json")).unwrap();
    let project =
        std::env::temp_dir().join(format!("rtok-latency-grep-proj-{}", std::process::id()));
    v["cwd"] = project.to_string_lossy().into_owned().into();
    v["tool_name"] = "Grep".into();
    v["tool_input"] = serde_json::json!({"pattern": "fn parse_since"});
    let setup = |home: &std::path::Path| {
        std::fs::create_dir_all(&project).unwrap();
        let body = "pub fn parse_since(s: &str) -> u32 {\n    s.len() as u32\n}\n\npub fn run() -> u32 {\n    parse_since(\"1d\")\n}\n";
        std::fs::write(project.join("lib.rs"), body).unwrap();
        std::fs::write(
            home.join("config.toml"),
            "[plugins.guard]\ngrep_symbol = true\n",
        )
        .unwrap();
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_rtok"))
            .args(["graph", "index"])
            .arg(&project)
            .env("RTOK_HOME", home)
            .output()
            .expect("rtok graph index");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    // A denial prints the hook's deny JSON, not `{}`.
    let denied = |out: &[u8]| String::from_utf8_lossy(out).contains("answered from the rtok index");
    p95_with(
        "PreToolUse",
        "-grep-symbol",
        v.to_string().as_bytes(),
        setup,
        denied,
    );
    let _ = std::fs::remove_dir_all(&project);
}

/// T428: a real store holds note bodies of thousands of tokens and a `session:*` note, and
/// SessionStart reads the recall titles, each body's size and the newest session note
/// through them. An empty store (the case above) hides that cost.
#[test]
fn latency_hook_session_start_with_populated_notes_p95_under_10ms() {
    let mut v: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/hooks/session_start.json")).unwrap();
    v["cwd"] = std::env::temp_dir().to_string_lossy().into_owned().into();
    p95_with(
        "SessionStart",
        "-notes",
        v.to_string().as_bytes(),
        |home| {
            let store = rtok::store::Store::open(&home.join("rtok.db")).expect("open store");
            let body = "recalled body line\n".repeat(900);
            for i in 0..40 {
                store
                    .upsert_note(None, "note", &format!("note {i}"), &body)
                    .expect("seed note");
            }
            store
                .upsert_note(None, "session:seed", "compact", &body)
                .expect("seed session note");
        },
        |out| out == b"{}",
    );
}

/// T370: `map_rank = "pagerank"` reads one stored row and ranks in memory, with no scan of
/// `symbols` and no process. The graph is about this repo's size (900 files, 120k edges on
/// 2026-10-21) so the decode and the personalized iterations are paid in full.
#[test]
fn latency_hook_session_start_with_a_pagerank_map_p95_under_10ms() {
    p95_with(
        "SessionStart",
        "-rank",
        session_start_in_temp_dir().as_bytes(),
        seed_pagerank_home,
        |out| String::from_utf8_lossy(out).contains("repo map"),
    );
}

/// T370: the configured hook prints the map from the stored graph, whatever the build profile.
#[test]
fn session_start_prints_the_pagerank_map_from_the_stored_graph() {
    let home = std::env::temp_dir().join(format!("rtok-rank-map-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("temp home");
    seed_pagerank_home(&home);
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .args(["hook", "SessionStart"])
        .env("RTOK_HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtok");
    let stdin = child.stdin.as_mut().expect("stdin");
    stdin
        .write_all(session_start_in_temp_dir().as_bytes())
        .expect("write fixture");
    let out = child.wait_with_output().expect("wait");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("repo map"), "{text}");
    assert!(text.contains(".rs: sym"), "{text}");
    let _ = std::fs::remove_dir_all(&home);
}

fn session_start_in_temp_dir() -> String {
    let mut v: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/hooks/session_start.json")).unwrap();
    v["cwd"] = std::env::temp_dir().to_string_lossy().into_owned().into();
    v.to_string()
}

/// A config that turns the pagerank map on, and a stored graph about this repo's size (900
/// files, 120k edges on 2026-10-21) so the decode and the personalized iterations are paid in
/// full.
fn seed_pagerank_home(home: &std::path::Path) {
    std::fs::write(
        home.join("config.toml"),
        "[plugins.graph]\nmap_tokens = 1000\nmap_rank = \"pagerank\"\n",
    )
    .expect("write config");
    let (files, names) = (900, 3600);
    let mut scan = Vec::new();
    for k in 0..names {
        scan.push((format!("sym{k}"), format!("f{:03}.rs", k % files), true, 1));
    }
    for i in 0..files {
        for j in 0..135 {
            let k = (i * 31 + j * 17) % names;
            scan.push((
                format!("sym{k}"),
                format!("f{i:03}.rs"),
                false,
                1 + (j % 3) as i64,
            ));
        }
    }
    // Eight files edited an hour ago are a working set, so the hook runs the personalized
    // iterations instead of answering from the stored global ranks.
    let hour = 3_600_000_000_000;
    let mtimes = (0..files)
        .map(|i| {
            (
                format!("f{i:03}.rs"),
                if i < 8 { now_nanos() - hour } else { 1 },
            )
        })
        .collect();
    let graph = rtok::plugins::graph::rank::build(&scan, &mtimes, &[]);
    let store = rtok::store::Store::open(&home.join("rtok.db")).expect("open store");
    let root = rtok::store::canon_root(&std::env::temp_dir());
    store
        .file_rank_put(&root, &serde_json::to_string(&graph).unwrap())
        .expect("seed graph");
}

fn now_nanos() -> i64 {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    since.map_or(0, |d| d.as_nanos() as i64)
}
