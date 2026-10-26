// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T20.1: `rtok demon` keeps a service up, and every verb tells the truth about it.
//!
//! Check: the supervised service here is `rtok mcp` with no stdin — it reaches EOF and exits at
//! once, which is the crash loop a supervisor exists for. Liveness is asked of the kernel
//! (`kill -0`), never read out of the state file, so a stale file cannot pass a test.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rtok")
}

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rtok-t201-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // `mcp` restarts in tens of milliseconds here; the shipped backoff is seconds.
    fs::write(
        dir.join("config.toml"),
        "[demon]\nservices = [\"mcp\"]\nbackoff_ms = 50\nmax_backoff_ms = 100\npoll_ms = 50\n",
    )
    .unwrap();
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

fn state(home: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(home.join("demon/mcp.json")).ok()?).ok()
}

/// The kernel's answer, not the state file's. `demon.rs` itself asks through
/// `rtok_sys::process_alive` (Unix `kill -0`, Windows `GetExitCodeProcess`); asking the same
/// way here — rather than shelling out to a `kill` binary that doesn't exist on Windows —
/// checks what the supervisor actually reads instead of merely a lookalike.
fn alive(pid: i64) -> bool {
    rtok_sys::process_alive(pid as i32)
}

/// Wait for the supervisor to have restarted the service at least `n` times.
fn wait_restarts(home: &Path, n: u64) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(v) = state(home)
            && v["restarts"].as_u64().unwrap_or(0) >= n
        {
            return v;
        }
        assert!(Instant::now() < deadline, "no {n} restarts within 10s");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_service_that_exits_comes_back_and_stop_takes_the_whole_tree_down() {
    let h = home("cycle");
    rtok(&["demon", "start"], &h); // no name: `[demon] services`
    let st = wait_restarts(&h, 2);
    let sup = st["supervisor"].as_i64().unwrap();
    assert!(alive(sup), "supervisor died");
    assert!(rtok(&["demon", "status"], &h).contains("running"), "status");

    rtok(&["demon", "stop"], &h);
    assert!(state(&h).is_none(), "stop left the state file behind");
    assert!(!alive(sup), "stop left the supervisor running");
    // The marker outlives the supervisor, so nothing can resurrect the service behind our back.
    assert!(h.join("demon/mcp.stop").exists());
    std::thread::sleep(Duration::from_millis(300));
    assert!(state(&h).is_none(), "something restarted after stop");
    let _ = fs::remove_dir_all(&h);
}

#[test]
fn status_asks_the_kernel_rather_than_believing_the_state_file() {
    let h = home("truth");
    rtok(&["demon", "start", "mcp"], &h);
    let st = wait_restarts(&h, 1);
    let sup = st["supervisor"].as_i64().unwrap();
    // Kill it the way a machine would — the state file stays, saying "supervisor <pid>".
    rtok_sys::process_kill(sup as i32);
    for _ in 0..40 {
        if !alive(sup) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        state(&h).is_some(),
        "the stale file is the point of the test"
    );
    // T237: liveness is the supervisor's lock, and Windows releases a killed process's locks
    // asynchronously, after its exit code already reads dead: `status` said `running` 0.2 s
    // after the kill (ci run 35938059153). A status that believed the file never turns.
    until("status reads the killed supervisor as stopped", || {
        rtok(&["demon", "status", "mcp"], &h).contains("stopped")
    });
    let out = rtok(&["demon", "status", "mcp"], &h);
    assert!(out.contains("stopped"), "{out}");
    assert!(!out.contains("running"), "{out}");
    rtok(&["demon", "kill", "mcp"], &h);
    assert!(state(&h).is_none(), "kill left the state file behind");
    let _ = fs::remove_dir_all(&h);
}

#[test]
fn a_second_start_is_refused_and_status_names_every_service() {
    let h = home("once");
    rtok(&["demon", "start", "mcp"], &h);
    let first = wait_restarts(&h, 1)["supervisor"].as_i64().unwrap();
    let out = rtok(&["demon", "start", "mcp"], &h);
    assert!(out.contains("already running"), "{out}");
    assert_eq!(
        state(&h).unwrap()["supervisor"].as_i64().unwrap(),
        first,
        "a second supervisor took the service over"
    );

    let status = rtok(&["demon", "status"], &h);
    for s in ["proxy", "mcp", "web", "hook"] {
        assert!(status.contains(s), "status is missing {s}:\n{status}");
    }
    // `proxy`, `web` and `hook` were never started, so they must read as stopped, not as absent.
    assert_eq!(status.matches("stopped").count(), 3, "{status}");

    let bad = Command::new(bin())
        .args(["demon", "start", "rm -rf /"])
        .env("RTOK_HOME", &h)
        .env("HOME", &h)
        .output()
        .unwrap();
    assert!(!bad.status.success(), "an arbitrary command was accepted");
    // The refusal is clap's, generated from the `Service` ValueEnum, and it names the choices.
    let err = String::from_utf8_lossy(&bad.stderr);
    assert!(err.contains("invalid value"), "{err}");
    assert!(err.contains("proxy") && err.contains("web"), "{err}");

    rtok(&["demon", "stop", "mcp"], &h);
    let _ = fs::remove_dir_all(&h);
}

#[test]
fn status_names_the_proxy_endpoint_running_or_stopped() {
    let h = home("endpoint");
    // A custom [proxy] port proves the row reads config, not a baked-in default.
    fs::write(
        h.join("config.toml"),
        "[demon]\nservices = [\"mcp\"]\nbackoff_ms = 50\nmax_backoff_ms = 100\npoll_ms = 50\n\
         [proxy]\nport = 8123\n",
    )
    .unwrap();

    // The proxy was never started; its row must still say where it would listen.
    let status = rtok(&["demon", "status"], &h);
    assert!(status.contains("127.0.0.1:8123"), "{status}");

    let json: Value = serde_json::from_str(&rtok(&["demon", "status", "--json"], &h)).unwrap();
    let proxy = json
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["service"] == "proxy")
        .unwrap();
    assert_eq!(proxy["endpoint"], "127.0.0.1:8123");
    let mcp = json
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["service"] == "mcp")
        .unwrap();
    assert!(mcp["endpoint"].is_null(), "mcp is stdio: {}", mcp);

    // T82: starting/stopping the demon process tree times out on windows-latest (180s) —
    // same family as the other `tests/demon.rs` exclusions. Keep the stopped-endpoint
    // checks above on every OS; the live listen round-trip stays Unix until T83.
    #[cfg(unix)]
    {
        // And the same address once it really is up: wait on the listener, not the state file.
        rtok(&["demon", "start", "proxy"], &h);
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 8123));
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
                break;
            }
            assert!(Instant::now() < deadline, "proxy never listened on 8123");
            std::thread::sleep(Duration::from_millis(50));
        }
        let up = rtok(&["demon", "status", "proxy"], &h);
        assert!(up.contains("running"), "{up}");
        assert!(up.contains("127.0.0.1:8123"), "{up}");
        rtok(&["demon", "stop", "proxy"], &h);
    }
    let _ = fs::remove_dir_all(&h);
}

#[cfg(unix)]
fn write_script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
fn upgrade(home: &Path, cmd: &Path) -> std::process::Output {
    Command::new(bin())
        .args(["demon", "upgrade"])
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .env("RTOK_UPDATE_CMD", cmd)
        .output()
        .unwrap()
}

#[cfg(unix)]
#[test]
fn upgrade_stops_before_replace_and_starts_after() {
    let h = home("upgrade-ok");
    rtok(&["demon", "start"], &h);
    wait_restarts(&h, 1);
    let script = h.join("replace.sh");
    write_script(
        &script,
        &format!(
            "#!/bin/sh\n[ ! -f '{}/demon/mcp.json' ] || exit 2\ntouch '{}'\n",
            h.display(),
            h.join("replaced").display()
        ),
    );
    let out = upgrade(&h, &script);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(h.join("replaced").exists(), "replace did not run");
    wait_restarts(&h, 1);
    rtok(&["demon", "stop"], &h);
    let _ = fs::remove_dir_all(&h);
}

#[cfg(unix)]
#[test]
fn upgrade_starts_again_when_replace_fails() {
    let h = home("upgrade-fail");
    rtok(&["demon", "start"], &h);
    wait_restarts(&h, 1);
    let script = h.join("replace.sh");
    write_script(&script, "#!/bin/sh\nexit 1\n");
    let out = upgrade(&h, &script);
    assert!(
        !out.status.success(),
        "failed replace must fail the command"
    );
    wait_restarts(&h, 1);
    rtok(&["demon", "stop"], &h);
    let _ = fs::remove_dir_all(&h);
}

/// Whether a resident answers on the home's endpoint (a connect, nothing sent).
fn listening(home: &Path) -> bool {
    rtok_hook::connect(&rtok_hook::endpoint(home).expect("endpoint")).is_some()
}

fn until(what: &str, f: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < deadline, "{what} within 10s");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn the_hook_service_runs_the_resident_and_stop_takes_it_down() {
    // Short: a Unix socket path must fit 104 bytes.
    let h = std::env::temp_dir().join(format!("rtd-{}", std::process::id()));
    let _ = fs::remove_dir_all(&h);
    fs::create_dir_all(&h).unwrap();
    rtok(&["demon", "start", "hook"], &h);
    until("the resident listens", || listening(&h));
    rtok(&["demon", "stop", "hook"], &h);
    until("the resident is gone", || !listening(&h));
    let _ = fs::remove_dir_all(&h);
}
