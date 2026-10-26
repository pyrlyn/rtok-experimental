// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T178 / D32: `rtok hook --serve` answers what `rtok hook` prints, refuses a client whose
//! environment or version differs, runs once per home, and exits on a newer client or a deleted
//! home. `rtok-hook` prints a resident's answer, runs `rtok hook` when none answers or one
//! refuses, and prints `{}` for one too slow. Every resident here is this test's own: a child in
//! its own temp home, or a fake listener.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rtok_hook::Request;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const CLIENT: &str = env!("CARGO_BIN_EXE_rtok-hook");

struct Home(PathBuf);

impl Home {
    /// A fresh temp home. Short: a Unix socket path must fit 104 bytes.
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rt{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// `bin` in this home with nothing else from the test's environment.
    fn cmd(&self, bin: &str, args: &[&str]) -> Command {
        let mut cmd = Command::new(bin);
        cmd.args(args)
            .env_clear()
            .envs(self.env())
            .current_dir(&self.0);
        for k in ["PATH", "SystemRoot"] {
            std::env::var_os(k).map(|v| cmd.env(k, v));
        }
        cmd
    }

    fn rtok(&self, args: &[&str]) -> Command {
        self.cmd(env!("CARGO_BIN_EXE_rtok"), args)
    }

    fn env(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        let dir = self.0.clone().into_os_string();
        vec![("RTOK_HOME".into(), dir.clone()), ("HOME".into(), dir)]
    }

    fn payload(&self) -> Vec<u8> {
        let tool_input = serde_json::json!({"command": "git status"});
        serde_json::json!({"hook_event_name": "PreToolUse", "session_id": "t178", "cwd": self.0,
            "tool_name": "Bash", "tool_input": tool_input})
        .to_string()
        .into_bytes()
    }

    /// `cmd`'s stdout for the payload on its stdin.
    fn output(&self, mut cmd: Command) -> Vec<u8> {
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let payload = self.payload();
        child.stdin.take().unwrap().write_all(&payload).unwrap();
        child.wait_with_output().unwrap().stdout
    }

    /// What `rtok hook PreToolUse` prints for the payload: a rewrite.
    fn expected(&self) -> Vec<u8> {
        let out = self.output(self.rtok(&["hook", "PreToolUse"]));
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains("rtok run"),
            "the control run rewrites: {text}"
        );
        out
    }

    fn call(&self, version: &str, fingerprint: u64) -> Option<Option<Vec<u8>>> {
        self.call_as(version, fingerprint, None)
    }

    fn call_as(
        &self,
        version: &str,
        fingerprint: u64,
        pid: Option<u32>,
    ) -> Option<Option<Vec<u8>>> {
        let endpoint = rtok_hook::endpoint(&self.0).expect("endpoint");
        let mut s = rtok_hook::connect(&endpoint)?;
        let (event, host, stdin) = ("PreToolUse".into(), String::new(), self.payload());
        let cwd = self.0.to_str().unwrap().into();
        let req = Request {
            version: version.into(),
            fingerprint,
            event,
            host,
            cwd,
            stdin,
            pid,
        };
        rtok_hook::exchange(&mut s, &req.encode())
    }

    /// A resident, once it answers (a refused probe).
    fn serve(&self) -> Child {
        let child = self.rtok(&["hook", "--serve"]).spawn().unwrap();
        let start = Instant::now();
        while self
            .call(VERSION, !rtok_hook::fingerprint(self.env()))
            .is_none()
        {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "resident never listened"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        child
    }
}

fn exits(child: &mut Child) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if child.try_wait().unwrap().is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn the_resident_answers_like_rtok_hook_and_refuses_or_exits_otherwise() {
    let home = Home::new("r");
    let expected = home.expected();
    let fp = rtok_hook::fingerprint(home.env());
    let mut resident = home.serve();
    assert_eq!(home.call(VERSION, fp), Some(Some(expected)));
    assert_eq!(
        home.call(VERSION, fp ^ 1),
        Some(None),
        "another environment is refused"
    );
    let second = home.rtok(&["hook", "--serve"]).status().unwrap();
    assert!(
        second.success(),
        "a second resident on the same home returns at once"
    );
    assert_eq!(
        home.call("999.0.0", fp),
        Some(None),
        "another version is refused"
    );
    assert!(exits(&mut resident), "and the stale resident exits");

    let mut resident = home.serve();
    // Windows keeps an open `hook.lock` from being deleted; there the test stops its
    // own child, waits for handles to release, then removes the home with a short retry
    // for ERROR_SHARING_VIOLATION (32).
    #[cfg(windows)]
    {
        resident.kill().unwrap();
        assert!(exits(&mut resident), "killed resident exits");
        let start = Instant::now();
        loop {
            match std::fs::remove_dir_all(&home.0) {
                Ok(()) => break,
                Err(e)
                    if e.raw_os_error() == Some(32) && start.elapsed() < Duration::from_secs(5) =>
                {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => panic!("remove home: {e}"),
            }
        }
    }
    #[cfg(not(windows))]
    {
        std::fs::remove_dir_all(&home.0).unwrap();
        assert!(
            exits(&mut resident),
            "a resident exits once its home is gone"
        );
    }
}

/// Agent rows' recorded ancestors in `home`'s store.
fn recorded_ancestors(home: &Home) -> Vec<Vec<i32>> {
    let cfg = rtok::config::Config::load_from(&home.0).expect("config");
    let store = rtok::store::Store::open(&cfg.core.db_path).expect("store");
    let rows = store.live_agents("30m").unwrap();
    rows.into_iter().map(|r| r.ancestors).collect()
}

/// T283.3: the resident walks the ancestors of the pid in the request and the hook stores them
/// on the agent row, nearest first; a request without a pid (an older client) stores none.
#[test]
fn the_resident_records_the_ancestors_of_the_request_pid() {
    let home = Home::new("a");
    let mut resident = home.serve();
    let fp = rtok_hook::fingerprint(home.env());
    let me = std::process::id();
    let none = home.call_as(VERSION, fp, None);
    let before = recorded_ancestors(&home);
    let some = home.call_as(VERSION, fp, Some(me));
    let after = recorded_ancestors(&home);
    let _ = resident.kill();
    let _ = resident.wait();
    assert!(none.is_some() && some.is_some());
    assert_eq!(before, [Vec::<i32>::new()], "no pid, no ancestors");
    let chain = rtok_sys::ancestors(i32::try_from(me).unwrap(), 3);
    assert_eq!(after, [chain], "this test's own ancestors");
}

/// The real client sends its own pid. It may miss the 50 ms answer window on a loaded machine
/// and then prints `{}` while the resident finds it gone, so retry until a call lands.
/// Not on Windows: `rtok_sys::parent_of` gives no answer there, so no row can name this test.
#[cfg(not(windows))]
#[test]
fn the_client_sends_its_own_pid() {
    let home = Home::new("p");
    let mut resident = home.serve();
    let me = i32::try_from(std::process::id()).unwrap();
    let landed = (0..40).any(|_| {
        home.output(home.cmd(CLIENT, &["PreToolUse"]));
        recorded_ancestors(&home)
            .iter()
            .any(|a| a.first() == Some(&me))
    });
    let _ = resident.kill();
    let _ = resident.wait();
    assert!(
        landed,
        "no agent row records this test as the client's parent"
    );
}

#[test]
fn with_no_resident_the_client_runs_rtok_hook() {
    let home = Home::new("n");
    let expected = home.expected();
    assert_eq!(home.output(home.cmd(CLIENT, &["PreToolUse"])), expected);
    let _ = std::fs::remove_dir_all(&home.0);
}

/// A fake resident: it answers each call with the next of `answers` (`None` holds the call past
/// the client's timeout) and hands back each request.
#[cfg(unix)]
fn fake(
    home: &Home,
    answers: Vec<Option<Option<&'static [u8]>>>,
) -> std::sync::mpsc::Receiver<Request> {
    let endpoint = rtok_hook::endpoint(&home.0).unwrap();
    let listener = std::os::unix::net::UnixListener::bind(endpoint).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for answer in answers {
            let (mut s, _) = listener.accept().unwrap();
            let req = Request::decode(&rtok_hook::read_frame(&mut s).unwrap()).unwrap();
            tx.send(req).unwrap();
            match answer {
                Some(out) => s.write_all(&rtok_hook::encode_response(out)).unwrap(),
                None => std::thread::sleep(Duration::from_secs(3)),
            }
        }
    });
    rx
}

#[cfg(unix)]
#[test]
fn the_client_prints_the_answer_runs_rtok_hook_on_refusal_and_fails_open_when_slow() {
    let home = Home::new("c");
    let expected = home.expected();
    let requests = fake(&home, vec![Some(Some(b"served")), Some(None), None]);

    let served = home.output(home.cmd(CLIENT, &["PreToolUse", "--host", "cursor"]));
    assert_eq!(served, b"served");
    let req = requests.recv().unwrap();
    assert_eq!(
        (req.version.as_str(), req.event.as_str(), req.host.as_str()),
        (VERSION, "PreToolUse", "cursor")
    );
    assert_eq!(req.fingerprint, rtok_hook::fingerprint(home.env()));
    assert_eq!(req.stdin, home.payload());

    let refused = home.output(home.cmd(CLIENT, &["PreToolUse"]));
    assert_eq!(refused, expected, "a refusal runs rtok hook");

    let start = Instant::now();
    let slow = home.output(home.cmd(CLIENT, &["PreToolUse"]));
    assert_eq!(slow, b"{}", "a slow resident fails open");
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
    let _ = std::fs::remove_dir_all(&home.0);
}

/// T459: the client has no config, so a body over the default cap is `rtok hook`'s to judge: the
/// resident is never sent it, and the client answers `{}` the way `rtok hook` does.
#[cfg(unix)]
#[test]
fn the_client_does_not_send_a_body_over_the_cap_to_the_resident() {
    let home = Home::new("o");
    let listener =
        std::os::unix::net::UnixListener::bind(rtok_hook::endpoint(&home.0).unwrap()).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let sent = rtok_hook::read_frame(&mut s)
            .ok()
            .and_then(|b| Request::decode(&b))
            .map(|r| r.stdin.len());
        let _ = s.write_all(&rtok_hook::encode_response(Some(b"served")));
        tx.send(sent).unwrap();
    });
    let mut child = home
        .cmd(CLIENT, &["PreToolUse"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut pipe = child.stdin.take().unwrap();
    // The client may stop reading once `rtok hook` has answered.
    let _ = pipe.write_all(&vec![
        b' ';
        rtok_hook::HOOK_MAX_INPUT_BYTES as usize + (1 << 20)
    ]);
    drop(pipe);
    assert_eq!(child.wait_with_output().unwrap().stdout, b"{}");
    assert_eq!(rx.recv().unwrap(), None, "the resident was sent the body");
    let _ = std::fs::remove_dir_all(&home.0);
}
