// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! S2: the `/ws` access token, against the real `rtok web` binary. The token file is created
//! owner-only, a client holding it connects, one without it (or with another) gets 401, a restart
//! replaces the token, and the non-loopback bind stays denied without the opt-in.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

struct Web {
    child: Child,
    addr: String,
}

impl Drop for Web {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn temp_home(label: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("rtok-web-token-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("home");
    home
}

fn command(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rtok"));
    cmd.env("RTOK_HOME", home)
        .env("HOME", home)
        .env_remove("RTOK_WEB_ALLOW_REMOTE")
        .current_dir(home);
    cmd
}

/// Start `rtok web` on a kernel-picked port and wait for its startup line.
fn start(home: &Path) -> Web {
    let mut child = command(home)
        .args(["web", "--host", "127.0.0.1", "--port", "0"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtok web");
    let stderr = child.stderr.take().expect("stderr");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some(rest) = line.strip_prefix("rtok web http://")
                && let Some(addr) = rest.split_whitespace().next()
            {
                let _ = tx.send(addr.to_owned());
            }
        }
    });
    let addr = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("rtok web never printed its address");
    Web { child, addr }
}

fn token_file(home: &Path) -> PathBuf {
    rtok::web::token::token_path(home)
}

fn read_token(home: &Path) -> String {
    rtok::web::token::read(home).expect("token file")
}

/// A `/ws` handshake presenting `token` as a bearer header, or nothing.
async fn connect(addr: &str, token: Option<&str>) -> Result<(), WsError> {
    let mut req = format!("ws://{addr}/ws")
        .into_client_request()
        .expect("request");
    if let Some(t) = token {
        req.headers_mut().insert(
            "authorization",
            format!("Bearer {t}").parse().expect("header"),
        );
    }
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await?;
    let first = tokio::time::timeout(Duration::from_secs(30), ws.next())
        .await
        .expect("first frame in time")
        .expect("socket open")
        .expect("frame");
    let Message::Text(t) = first else {
        panic!("expected a text frame")
    };
    assert!(
        t.as_str().contains("\"snapshot\""),
        "first frame is a snapshot"
    );
    Ok(())
}

fn unauthorized(r: Result<(), WsError>) -> bool {
    matches!(r, Err(WsError::Http(resp)) if resp.status() == 401)
}

/// 1: the token file exists once the server is up, with owner-only permissions.
#[cfg(unix)]
#[test]
fn token_file_is_created_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let home = temp_home("mode");
    let _web = start(&home);
    let mode = std::fs::metadata(token_file(&home))
        .expect("token file")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "token file mode is {:o}", mode & 0o777);
    assert_eq!(
        mode & 0o077,
        0,
        "token file must not be group/world accessible"
    );
    assert_eq!(read_token(&home).len(), 64);
}

/// Windows: inheritance is cut and exactly one principal (the creating user) holds an ACE.
#[cfg(windows)]
#[test]
fn token_file_acl_names_only_the_current_user() {
    let home = temp_home("acl");
    let _web = start(&home);
    let out = Command::new("icacls")
        .arg(token_file(&home))
        .output()
        .expect("icacls");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let user = std::env::var("USERNAME").expect("USERNAME");
    assert_eq!(
        text.matches(":(").count(),
        1,
        "exactly one ACE expected:\n{text}"
    );
    assert!(
        text.to_lowercase().contains(&user.to_lowercase()),
        "ACE is not the user's:\n{text}"
    );
    for broad in ["Everyone", "BUILTIN\\Users", "Authenticated Users"] {
        assert!(
            !text.contains(broad),
            "{broad} must have no access:\n{text}"
        );
    }
}

/// Another account cannot read the file. Mode bits are the portable check (see the test above);
/// a real open as another user is only possible as root, otherwise this reports a skip.
#[cfg(unix)]
#[test]
fn another_user_cannot_open_the_token_file() {
    let home = temp_home("other");
    let _web = start(&home);
    let file = token_file(&home);
    let id = Command::new("id").arg("-u").output().expect("id");
    if String::from_utf8_lossy(&id.stdout).trim() != "0" {
        eprintln!(
            "SKIP real other-user open: needs root to switch user (mode bits asserted instead)"
        );
        return;
    }
    // The home sits under a world-traversable temp dir, so only the file's mode can stop `nobody`.
    let _ = Command::new("chmod").arg("755").arg(&home).status();
    let out = Command::new("su")
        .args(["-s", "/bin/sh", "nobody", "-c"])
        .arg(format!("cat '{}'", file.display()))
        .output();
    match out {
        Ok(o) => assert!(!o.status.success(), "nobody read the token file"),
        Err(e) => eprintln!("SKIP real other-user open: su unavailable ({e})"),
    }
}

/// 2: a client that reads the token file connects, and the strict file does not stop the server
/// from serving `/ws` (this runs on every platform CI covers).
#[tokio::test]
async fn client_with_the_file_token_connects() {
    let home = temp_home("ok");
    let web = start(&home);
    connect(&web.addr, Some(&read_token(&home)))
        .await
        .expect("connect with token");
}

/// 2b: the browser fallback, `?token=`.
#[tokio::test]
async fn query_token_connects_for_browsers() {
    let home = temp_home("query");
    let web = start(&home);
    let url = format!("ws://{}/ws?token={}", web.addr, read_token(&home));
    let (_ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("connect with ?token=");
}

/// 3: no token, a wrong token and a wrong scheme are all refused with 401.
#[tokio::test]
async fn missing_or_wrong_token_is_rejected() {
    let home = temp_home("deny");
    let web = start(&home);
    let real = read_token(&home);
    assert!(unauthorized(connect(&web.addr, None).await));
    assert!(unauthorized(connect(&web.addr, Some("wrong")).await));
    assert!(unauthorized(
        connect(&web.addr, Some(&real[..real.len() - 1])).await
    ));
    let url = format!("ws://{}/ws?token=wrong", web.addr);
    assert!(matches!(
        tokio_tungstenite::connect_async(url).await,
        Err(WsError::Http(r)) if r.status() == 401
    ));
}

/// 4: a restart issues a new token; the old one stops working.
#[tokio::test]
async fn restart_rotates_the_token() {
    let home = temp_home("rotate");
    let old = {
        let web = start(&home);
        let old = read_token(&home);
        connect(&web.addr, Some(&old))
            .await
            .expect("first run accepts its token");
        old
    };
    let web = start(&home);
    let new = read_token(&home);
    assert_ne!(old, new, "token must change on restart");
    assert!(unauthorized(connect(&web.addr, Some(&old)).await));
    connect(&web.addr, Some(&new))
        .await
        .expect("new token works");
}

/// T944: `/auth` runs the `/ws` checks without an upgrade. A browser WebSocket hides the 401, so
/// this is how the SPA tells a missing or stale token from a server that is down.
#[tokio::test]
async fn auth_probe_answers_the_ws_checks() {
    let home = temp_home("probe");
    let web = start(&home);
    let url = format!("http://{}/auth", web.addr);
    let client = reqwest::Client::new();
    let status = |req: reqwest::RequestBuilder| async move {
        req.send().await.expect("probe answered").status().as_u16()
    };
    let real = read_token(&home);
    assert_eq!(status(client.get(&url)).await, 401);
    assert_eq!(status(client.get(&url).bearer_auth("wrong")).await, 401);
    assert_eq!(status(client.get(&url).bearer_auth(&real)).await, 204);
    let foreign = client
        .get(&url)
        .bearer_auth(&real)
        .header("origin", "http://evil.example");
    assert_eq!(status(foreign).await, 403);
}

/// 5: a non-loopback bind is refused without `RTOK_WEB_ALLOW_REMOTE=1`, and no token is issued.
#[test]
fn non_loopback_bind_is_denied_without_opt_in() {
    let home = temp_home("remote");
    let out = command(&home)
        .args(["web", "--host", "0.0.0.0", "--port", "0"])
        .stdin(Stdio::null())
        .output()
        .expect("run rtok web");
    assert!(!out.status.success(), "0.0.0.0 bind must fail");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("refusing to bind"), "unexpected stderr: {err}");
    assert!(
        !token_file(&home).exists(),
        "a refused start must not leave a token"
    );
}
