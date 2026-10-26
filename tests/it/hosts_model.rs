// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T231: `rtok agents list` / `agents info` render the D23 Hosts page. This spawns
//! the real `rtok web` binary on the same fixture matrix `tests/common/agents.rs`
//! builds for the install/list tests (fake claude/codex/copilot on PATH, T168; every
//! host's marker directory under a throwaway home), waits for its cached probe to
//! answer over `/ws`, and pins the page's text against `agents list --json` run on
//! the identical PATH/HOME — the same accessor T231's cache wraps (D27).

use crate::common;

use common::agents::{bin, fake_claude_path, write_cfg};
use futures_util::StreamExt;
use serde_json::Value;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-t231-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A `/ws` handshake carrying the token `rtok web` wrote under `rtok_home` (S2).
async fn connect_ws(
    addr: &str,
    rtok_home: &Path,
) -> Result<
    (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    ),
    tokio_tungstenite::tungstenite::Error,
> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let token = rtok::web::token::read(rtok_home).expect("web token file");
    let mut req = format!("ws://{addr}/ws")
        .into_client_request()
        .expect("ws request");
    req.headers_mut().insert(
        "authorization",
        format!("Bearer {token}").parse().expect("header"),
    );
    tokio_tungstenite::connect_async(req).await
}

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

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("free port")
        .port()
}

/// The shared env every subprocess in this file runs under: the same config, PATH
/// and HOME, so `agents list --json` and `rtok web`'s cached probe read the
/// identical fixture matrix (T168's shims, the per-host marker dirs) — only the
/// trailing args differ between the two calls.
fn cmd(cfg: &Path, home: &Path, path: &OsStr) -> Command {
    let mut c = Command::new(bin());
    c.arg("--config")
        .arg(cfg)
        .env("PATH", path)
        .env("HOME", home)
        .env("RTOK_HOME", home.join(".rtok"));
    c
}

async fn start(cfg: &Path, home: &Path, path: &OsStr) -> Web {
    let port = free_port().to_string();
    let child = cmd(cfg, home, path)
        .args(["web", "--host", "127.0.0.1", "--port", &port])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rtok web");
    let web = Web {
        child,
        addr: format!("127.0.0.1:{port}"),
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(r) = reqwest::get(format!("http://{}/health", web.addr)).await
            && r.status() == 200
        {
            return web;
        }
        assert!(Instant::now() < deadline, "rtok web never answered /health");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn agents_list_json(cfg: &Path, home: &Path, path: &OsStr) -> Vec<Value> {
    let out = cmd(cfg, home, path)
        .args(["agents", "list", "--json"])
        .output()
        .expect("agents list --json");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("json rows")
}

/// The Hosts page's text carries the same per-variant block `agents list --json`'s
/// rows describe — kind, name, config path — which is what T231's cache wraps.
/// Polls `/ws` snapshots (the 2 s tick) until the cache answers instead of "probing
/// hosts…" (T231: a cold cache never blocks the tick).
#[tokio::test]
async fn hosts_page_matches_agents_list_json_on_the_fixture_matrix() {
    let h = home("page");
    let cfg = write_cfg(&h);
    let path = fake_claude_path(&h);
    let rows = agents_list_json(&cfg, &h, &path);
    assert!(
        !rows.is_empty(),
        "the fixture matrix lists at least one host"
    );

    let web = start(&cfg, &h, &path).await;
    let (mut ws, _) = connect_ws(&web.addr, &h.join(".rtok"))
        .await
        .expect("ws connect");

    let deadline = Instant::now() + Duration::from_secs(60);
    let page = loop {
        assert!(Instant::now() < deadline, "the hosts probe never answered");
        let msg = tokio::time::timeout(Duration::from_secs(60), ws.next())
            .await
            .expect("frame within 15 s")
            .expect("socket open")
            .expect("frame");
        let Message::Text(t) = msg else { continue };
        let v: Value = serde_json::from_str(t.as_str()).expect("json frame");
        if v["type"] != "snapshot" {
            continue;
        }
        let text = v["hosts"].as_str().unwrap_or_default().to_string();
        if text != "probing hosts…\n" {
            break text;
        }
    };

    for row in &rows {
        let host = row["host"].as_str().unwrap();
        let name = row["name"].as_str().unwrap();
        assert!(
            page.contains(&format!(": {name}")),
            "page is missing the `{host}`/`{name}` block: {page}"
        );
        for cfg_path in row["config"].as_array().unwrap() {
            let cfg_path = cfg_path.as_str().unwrap();
            assert!(
                page.contains(cfg_path),
                "page is missing `{host}`'s config path {cfg_path}: {page}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&h);
}
