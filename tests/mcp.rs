// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T4.1 Check: `tools/list` over stdio lists `expand`.
#![allow(unexpected_cfgs)]

mod common;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

/// T263: `child.stdout` lines with a bounded wait, so a stalled exchange fails, not hangs.
struct LineReader {
    rx: std::sync::mpsc::Receiver<String>,
}

impl LineReader {
    fn new(stdout: std::process::ChildStdout) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match std::io::BufRead::read_line(&mut reader, &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if tx.send(line.trim_end().to_string()).is_err() => break,
                    Ok(_) => {}
                }
            }
        });
        Self { rx }
    }

    fn next_line(&self) -> String {
        self.rx
            .recv_timeout(common::scaled(std::time::Duration::from_secs(10)))
            .expect("rtok mcp did not answer in time")
    }
}

#[test]
fn tools_list_includes_expand() {
    let tmp = std::env::temp_dir().join(format!("rtok-mcp-list-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .arg("mcp")
        .env("RTOK_HOME", &tmp)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtok mcp");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "stderr {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("expand"), "{stdout}");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// T191: a malformed line answers `-32700` instead of silence, and the next valid
/// request on the same connection still answers.
#[test]
fn garbage_line_answers_parse_error_then_tools_list() {
    let tmp = std::env::temp_dir().join(format!("rtok-mcp-garbage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .arg("mcp")
        .env("RTOK_HOME", &tmp)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtok mcp");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(
            br#"{bad
{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "stderr {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut lines = stdout.lines();
    let first: serde_json::Value =
        serde_json::from_str(lines.next().unwrap_or_default()).expect("error line");
    assert_eq!(first["error"]["code"], -32700, "{stdout}");
    assert_eq!(first["id"], serde_json::Value::Null, "{stdout}");
    let rest: String = lines.collect::<Vec<_>>().join("\n");
    assert!(rest.contains("expand"), "{stdout}");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// T75: another process holding the store's write lock at spawn time must not kill the
/// server. The startup retention used to run a deferred read-then-write transaction that
/// came back "database is locked" (instantly on SQLITE_BUSY_SNAPSHOT, which skips the
/// busy handler, or after the steady 1 s) and the `?` took the whole process down — the
/// client saw "Server disconnected".
#[test]
fn mcp_serves_while_another_process_holds_the_store_writer() {
    let tmp = std::env::temp_dir().join(format!("rtok-mcp-locked-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let db = tmp.join("rtok.db");
    // Settle the store (migrations) and drop it: the binary's only startup write left
    // is the retention purge.
    drop(rtok::store::Store::open(&db).unwrap());
    // Hold the WAL writer lock for 2.5 s — past the steady 1 s busy timeout even
    // after the binary's own startup latency reaches the purge.
    let holder = common::hold_store_writer(&db, std::time::Duration::from_millis(2500));
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .arg("mcp")
        .env("RTOK_HOME", &tmp)
        .env("RTOK_CORE_DB_PATH", db.to_str().unwrap())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtok mcp");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        )
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    holder.join().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stderr {err}");
    assert!(!err.contains("database is locked"), "stderr {err}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("rtok"), "no serverInfo in {stdout}");
    assert!(
        stdout.contains("expand"),
        "no tools/list result in {stdout}"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// The handshake must introduce this server as rtok: `ServerInfo::default()` filled
/// `serverInfo` from rmcp's own build env, so clients saw `{"name":"rmcp"}`.
#[test]
fn initialize_names_the_server_rtok() {
    let tmp = std::env::temp_dir().join(format!("rtok-mcp-init-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .arg("mcp")
        .env("RTOK_HOME", &tmp)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtok mcp");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
        )
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().next().unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(line).expect("initialize response");
    assert_eq!(v["result"]["serverInfo"]["name"], "rtok", "{stdout}");
    assert!(
        v["result"]["serverInfo"]["version"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "{stdout}"
    );
    // T213: a client that requests a version this server supports gets that exact version
    // back (MCP lifecycle spec, "Initialization" — https://modelcontextprotocol.io/specification),
    // not whatever `ProtocolVersion::default()` happens to resolve to in the linked `rmcp`.
    assert_eq!(v["result"]["protocolVersion"], "2025-06-18", "{stdout}");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// T8.16: with the watcher on, `rtok mcp` exits promptly at stdin EOF.
#[test]
fn mcp_with_watcher_exits_on_stdin_eof() {
    let home = std::env::temp_dir().join(format!("rtok-mcp-watch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        "[plugins.graph]\nwatch = \"notify\"\n",
    )
    .unwrap();
    let repo = home.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("a.rs"), "fn a() {}\n").unwrap();
    let spawn = || {
        Command::new(env!("CARGO_BIN_EXE_rtok"))
            .arg("mcp")
            .env("RTOK_HOME", &home)
            .current_dir(&repo)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn rtok mcp")
    };
    // Warm-up: the first spawn on a machine pays dyld/Gatekeeper, not the watcher.
    let mut warm = spawn();
    drop(warm.stdin.take());
    let _ = warm.wait_with_output();
    let mut child = spawn();
    drop(child.stdin.take());
    let start = Instant::now();
    let out = child.wait_with_output().expect("wait");
    let ms = start.elapsed().as_millis();
    assert!(
        out.status.success(),
        "stderr {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The check is that EOF ends the watcher loop at all (it polls every 50 ms and used to be
    // joined forever), not how fast a shared runner spawns and tears down threads: the old 500 ms
    // wall-clock bound failed at 3355 ms on Windows CI (run 38073132494) with nothing wrong.
    let bound = common::scaled(std::time::Duration::from_secs(5)).as_millis();
    assert!(
        ms < bound,
        "mcp with watcher took {ms} ms to exit at EOF (bound {bound} ms)"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// T263: a client with the `roots` capability is asked `roots/list` after `initialized`;
/// the first `file://` root becomes the cwd, so `symbol` finds a repo the launch cwd is not.
#[test]
fn mcp_moves_into_the_first_file_root_after_roots_list() {
    let home = std::env::temp_dir().join(format!("rtok-mcp-roots-home-{}", std::process::id()));
    let outside = std::env::temp_dir().join(format!("rtok-mcp-roots-out-{}", std::process::id()));
    let repo = std::env::temp_dir().join(format!("rtok-mcp-roots-repo-{}", std::process::id()));
    for d in [&home, &outside, &repo] {
        let _ = std::fs::remove_dir_all(d);
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(repo.join("a.rs"), "fn alpha() {}\n").unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .arg("mcp")
        .env("RTOK_HOME", &home)
        .current_dir(&outside)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtok mcp");
    let mut stdin = child.stdin.take().expect("stdin");
    let lines = LineReader::new(child.stdout.take().expect("stdout"));

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18","capabilities":{{"roots":{{"listChanged":true}}}},"clientInfo":{{"name":"t","version":"1"}}}}}}"#
    )
    .unwrap();
    let init: serde_json::Value =
        serde_json::from_str(&lines.next_line()).expect("initialize response");
    assert_eq!(init["result"]["serverInfo"]["name"], "rtok", "{init}");

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    let roots_req: serde_json::Value =
        serde_json::from_str(&lines.next_line()).expect("roots/list request");
    assert_eq!(roots_req["method"], "roots/list", "{roots_req}");
    assert_eq!(roots_req["id"], "rtok-roots", "{roots_req}");

    let uri =
        url::Url::from_directory_path(repo.canonicalize().unwrap()).expect("file:// uri for repo");
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":"rtok-roots","result":{{"roots":[{{"uri":"{uri}","name":"r"}}]}}}}"#
    )
    .unwrap();

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"symbol","arguments":{{"name":"alpha"}}}}}}"#
    )
    .unwrap();
    let call: serde_json::Value =
        serde_json::from_str(&lines.next_line()).expect("tools/call response");
    assert_eq!(call["result"]["isError"], false, "{call}");
    assert!(
        call["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("a.rs"),
        "{call}"
    );

    drop(stdin);
    let _ = child.wait();
    for d in [&home, &outside, &repo] {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// T263: launched in `/` (Claude.app) with no roots, `symbol` refuses at once instead of
/// walking the disk.
#[test]
fn mcp_refuses_symbol_at_filesystem_root_without_walking() {
    let home = std::env::temp_dir().join(format!("rtok-mcp-noroot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .arg("mcp")
        .env("RTOK_HOME", &home)
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtok mcp");
    let mut stdin = child.stdin.take().expect("stdin");
    let lines = LineReader::new(child.stdout.take().expect("stdout"));

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18","capabilities":{{}},"clientInfo":{{"name":"t","version":"1"}}}}}}"#
    )
    .unwrap();
    let _init = lines.next_line();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();

    let start = Instant::now();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"symbol","arguments":{{"name":"alpha"}}}}}}"#
    )
    .unwrap();
    let call: serde_json::Value =
        serde_json::from_str(&lines.next_line()).expect("tools/call response");
    let ms = start.elapsed().as_millis();
    assert!(ms < 5000, "symbol at / took {ms} ms: must refuse, not walk");
    assert_eq!(call["result"]["isError"], true, "{call}");
    assert!(
        call["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("no project root"),
        "{call}"
    );

    drop(stdin);
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&home);
}

/// T351: a path in a sibling worktree of the cwd's repository and a path under any
/// `roots/list` root pass the guard of `read`, `search` and `outline`; everything else,
/// a scratchpad included, is still refused with `isError`.
#[test]
fn mcp_accepts_sibling_worktrees_and_every_client_root() {
    let base = std::env::temp_dir().join(format!("rtok-mcp-t351-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let names = ["home", "main", "extra", "scratchpad", "outside"];
    for n in names {
        std::fs::create_dir_all(base.join(n)).unwrap();
    }
    // macOS: `/var` is a symlink; the server compares canonical paths. `dunce`: Windows git
    // rejects the `\\?\` verbatim form `canonicalize` returns.
    let base = dunce::canonicalize(&base).unwrap();
    let [home, main, extra, scratch, outside] = names.map(|n| base.join(n));
    let wt = base.join("wt");
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&main)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    std::fs::write(main.join("m.txt"), "m\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "init"]);
    git(&["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "w"]);
    std::fs::write(wt.join("w.rs"), "fn in_worktree() {}\n").unwrap();
    for (dir, f) in [(&extra, "e.txt"), (&scratch, "s.txt"), (&outside, "o.txt")] {
        std::fs::write(dir.join(f), "x\n").unwrap();
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .arg("mcp")
        .env("RTOK_HOME", &home)
        .current_dir(&main)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtok mcp");
    let mut stdin = child.stdin.take().expect("stdin");
    let lines = LineReader::new(child.stdout.take().expect("stdout"));
    let mut send = |line: String| writeln!(stdin, "{line}").unwrap();
    send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{"roots":{}},"clientInfo":{"name":"t","version":"1"}}}"#.into());
    lines.next_line();
    send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.into());
    lines.next_line(); // our roots/list request
    let uri = |p: &Path| url::Url::from_directory_path(p).unwrap();
    send(format!(
        r#"{{"jsonrpc":"2.0","id":"rtok-roots","result":{{"roots":[{{"uri":"{}"}},{{"uri":"{}"}}]}}}}"#,
        uri(&main),
        uri(&extra)
    ));
    let mut call = |name: &str, args: serde_json::Value| -> (bool, String) {
        send(
            serde_json::json!({"jsonrpc":"2.0","id":7,"method":"tools/call",
                "params":{"name":name,"arguments":args}})
            .to_string(),
        );
        let v: serde_json::Value = serde_json::from_str(&lines.next_line()).expect("response");
        let text = v["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default();
        (v["result"]["isError"] == true, text.to_string())
    };
    let p = |d: &Path, f: &str| d.join(f).to_string_lossy().into_owned();

    let (err, text) = call("read", serde_json::json!({"path": p(&wt, "w.rs")}));
    assert!(!err && text.contains("in_worktree"), "{text}");
    let (err, text) = call(
        "search",
        serde_json::json!({"pattern": "in_worktree", "path": wt}),
    );
    assert!(!err && text.contains("w.rs"), "{text}");
    let (err, text) = call("outline", serde_json::json!({"path": p(&wt, "w.rs")}));
    assert!(!err && text.contains("in_worktree"), "{text}");
    let (err, text) = call("read", serde_json::json!({"path": p(&extra, "e.txt")}));
    assert!(!err, "second roots/list root: {text}");
    for (dir, f) in [(&scratch, "s.txt"), (&outside, "o.txt")] {
        let (err, text) = call("read", serde_json::json!({"path": p(dir, f)}));
        assert!(err && text.contains("path outside cwd"), "{text}");
    }

    drop(stdin);
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&base);
}
