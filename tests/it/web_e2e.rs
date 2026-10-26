// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! End to end: the real `rtok web` binary on a real port — the SPA it embeds (T310.9), the
//! `RTOK_WEB_DIST` override and a real WebSocket client on `/ws`. `tests/web.rs` drives the
//! router in-process; this is what a user's browser sees.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::ffi::OsStr;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::tungstenite::Message;

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
    home: PathBuf,
}

impl Drop for Web {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// The `host:port` in `rtok web`'s startup line, `rtok web http://<addr>  (ws://<addr>/ws)`.
fn bound_addr(line: &str) -> Option<String> {
    let rest = line.strip_prefix("rtok web http://")?;
    rest.split_whitespace().next().map(str::to_owned)
}

/// `rtok web` with `HOME`/`RTOK_HOME` in a temp dir, so the snapshot never reads this
/// machine's `~/.claude` (T74) and the `set` below writes a throwaway config.
async fn start(label: &str) -> Web {
    start_with(label, None).await
}

/// `dist` is the `RTOK_WEB_DIST` value, or `None` for the embedded SPA.
async fn start_with(label: &str, dist: Option<&OsStr>) -> Web {
    let home = std::env::temp_dir().join(format!("rtok-web-e2e-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("home");
    // Port 0: the kernel picks the port when `rtok web` binds and the startup line names it. A
    // port probed here and handed over can be taken by a parallel test in between.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rtok"));
    cmd.args(["web", "--host", "127.0.0.1", "--port", "0"])
        .env("RTOK_HOME", &home)
        .env("HOME", &home)
        .env_remove("RTOK_WEB_DIST");
    if let Some(dist) = dist {
        cmd.env("RTOK_WEB_DIST", dist);
    }
    // `home` is the cwd, so nothing here (no `web/dist`, no `dist/` beside the binary) can feed
    // the page except what the binary embeds.
    let mut child = cmd
        .current_dir(&home)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtok web");
    let stderr = child.stderr.take().expect("stderr");
    let (tx, rx) = std::sync::mpsc::channel();
    // Drains past the startup line too: a full pipe would block the server's next write.
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some(addr) = bound_addr(&line) {
                let _ = tx.send(addr);
            }
        }
    });
    let mut web = Web {
        child,
        addr: String::new(),
        home,
    };
    web.addr = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("rtok web never printed its address");
    let deadline = Instant::now() + Duration::from_secs(20);
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

fn built(rel: &str) -> Option<Vec<u8>> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("web/dist")
            .join(rel),
    )
    .ok()
}

/// The first `/assets/<file>` URL the page references with the given extension.
fn asset_url(html: &str, ext: &str) -> String {
    html.match_indices("/assets/")
        .map(|(i, _)| &html[i..i + html[i..].find('"').expect("closing quote")])
        .find(|url| url.ends_with(ext))
        .unwrap_or_else(|| panic!("no {ext} asset in the page"))
        .to_string()
}

/// T641: `--port 0` lets the kernel pick the port at bind time, so parallel servers never race
/// for a probed one; the startup line and `/health` name the port actually bound, not 0.
#[tokio::test]
async fn port_zero_reports_the_bound_port() {
    let web = start("port0").await;
    let port: u16 = web
        .addr
        .rsplit(':')
        .next()
        .and_then(|p| p.parse().ok())
        .expect("port in the startup line");
    assert_ne!(port, 0);
    let health = reqwest::get(format!("http://{}/health", web.addr))
        .await
        .expect("health")
        .text()
        .await
        .expect("body");
    let health: Value = serde_json::from_str(&health).expect("json");
    assert_eq!(health["port"], port, "{health}");
}

#[tokio::test]
async fn web_serves_the_embedded_spa_with_no_dist_on_disk() {
    let web = start("http").await;
    let base = format!("http://{}", web.addr);

    let health = reqwest::get(format!("{base}/health"))
        .await
        .expect("health")
        .text()
        .await
        .expect("body");
    let health: Value = serde_json::from_str(&health).expect("json");
    assert_eq!(health["ok"], true, "{health}");

    let html = reqwest::get(format!("{base}/")).await.expect("index");
    assert_eq!(html.status(), 200);
    assert!(html.headers().contains_key("content-security-policy"));
    let html = html.text().await.expect("html");
    // The page body is server-controlled input: keep it out of the assert message so a failure
    // cannot inject forged lines into the test log.
    let Some(index) = built("index.html") else {
        eprintln!("skip asset checks: built without web/dist — run `just spa-build`");
        assert!(
            html.contains("just web"),
            "placeholder does not say how to build"
        );
        return;
    };
    assert_eq!(
        html.as_bytes(),
        index.as_slice(),
        "index != web/dist/index.html"
    );

    // A deep link is the SPA's, not a 404 — a reload on /#/plugins never leaves the page.
    let deep = reqwest::get(format!("{base}/plugins"))
        .await
        .expect("deep link");
    assert_eq!(deep.status(), 200);

    let js_url = asset_url(&html, ".js");
    let js = reqwest::get(format!("{base}{js_url}")).await.expect("js");
    assert_eq!(js.status(), 200);
    assert!(
        js.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("javascript")
    );
    assert!(
        js.headers()["cache-control"]
            .to_str()
            .unwrap()
            .contains("immutable")
    );
    let raw = built(js_url.trim_start_matches('/')).expect("js in dist");
    assert_eq!(js.bytes().await.expect("body").as_ref(), raw.as_slice());

    // Precompressed: the brotli file in dist/ is what goes out, byte for byte.
    let br = reqwest::Client::new()
        .get(format!("{base}{js_url}"))
        .header("accept-encoding", "br")
        .send()
        .await
        .expect("br");
    assert_eq!(br.headers()["content-encoding"], "br");
    let raw_br = built(&format!("{}.br", js_url.trim_start_matches('/'))).expect("br in dist");
    assert_eq!(br.bytes().await.expect("body").as_ref(), raw_br.as_slice());
}

#[tokio::test]
async fn rtok_web_dist_overrides_the_embedded_spa() {
    let dist = std::env::temp_dir().join(format!("rtok-web-e2e-dist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dist);
    std::fs::create_dir_all(&dist).expect("dist");
    std::fs::write(dist.join("index.html"), "<!doctype html>OVERRIDE-MARKER").expect("index");
    let web = start_with("override", Some(dist.as_os_str())).await;
    let html = reqwest::get(format!("http://{}/", web.addr))
        .await
        .expect("index")
        .text()
        .await
        .expect("html");
    assert!(
        html.contains("OVERRIDE-MARKER"),
        "RTOK_WEB_DIST was ignored"
    );
    // The API is not the override's business.
    let (mut ws, _) = connect_ws(&web.addr, &web.home).await.expect("ws connect");
    assert_eq!(next_json(&mut ws).await["type"], "snapshot");
    let _ = std::fs::remove_dir_all(&dist);
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Next text frame as JSON, skipping pings; fails after 15 s.
async fn next_json(ws: &mut Ws) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(15), ws.next())
            .await
            .expect("frame within 15 s")
            .expect("socket open")
            .expect("frame");
        if let Message::Text(t) = msg {
            return serde_json::from_str(t.as_str()).expect("json frame");
        }
    }
}

/// Reads frames until one of `kind` arrives (snapshots keep ticking every 2 s).
async fn next_of(ws: &mut Ws, kind: &str) -> Value {
    loop {
        let v = next_json(ws).await;
        if v["type"] == kind {
            return v;
        }
    }
}

fn plugin_enabled(snap: &Value, id: &str) -> bool {
    snap["plugins"]
        .as_array()
        .expect("plugins")
        .iter()
        .find(|p| p["id"] == id)
        .unwrap_or_else(|| panic!("{id} row in {snap}"))["enabled"]
        .as_bool()
        .expect("enabled")
}

#[tokio::test]
async fn websocket_streams_snapshots_and_answers_commands() {
    let web = start("ws").await;
    let (mut ws, _) = connect_ws(&web.addr, &web.home).await.expect("ws connect");

    let first = next_json(&mut ws).await;
    assert_eq!(first["type"], "snapshot", "{first}");
    assert!(plugin_enabled(&first, "cmd"));

    // Unknown archive id → a message frame, and the socket stays up.
    ws.send(Message::text(r#"{"expand":"no-such-id"}"#))
        .await
        .expect("send expand");
    let msg = next_of(&mut ws, "message").await;
    assert!(
        msg["text"].as_str().unwrap().contains("unknown archive id"),
        "{msg}"
    );

    // T60.5 allow-list: a non-plugin key is refused over the wire.
    ws.send(Message::text(
        r#"{"set":{"key":"core.db_path","value":true}}"#,
    ))
    .await
    .expect("send refused set");
    let msg = next_of(&mut ws, "message").await;
    assert!(
        msg["text"].as_str().unwrap().contains("refused key"),
        "{msg}"
    );

    // An allow-listed toggle is written and the next snapshot carries it.
    ws.send(Message::text(
        r#"{"set":{"key":"plugins.cmd.enabled","value":false}}"#,
    ))
    .await
    .expect("send set");
    // A snapshot already in flight may predate the write; the next one carries it.
    let mut snap = next_of(&mut ws, "snapshot").await;
    if plugin_enabled(&snap, "cmd") {
        snap = next_of(&mut ws, "snapshot").await;
    }
    assert!(
        !plugin_enabled(&snap, "cmd"),
        "toggle not in the snapshot after the write"
    );
    let written = std::fs::read_to_string(web.home.join("config.toml")).expect("config written");
    assert!(written.contains("enabled = false"), "{written}");

    ws.close(None).await.expect("close");
}

/// T623: an answer carries the `req` of the request it answers — a plan, a refusal, a parse
/// error and a lookup miss alike — and a request without one is answered with none.
#[tokio::test]
async fn answers_echo_the_request_id_and_only_that_one() {
    let web = start("req").await;
    let (mut ws, _) = connect_ws(&web.addr, &web.home).await.expect("ws connect");
    let sent = [
        (r#"{"req":11,"junk":{"action":"plan"}}"#, "junkplan"),
        (r#"{"req":12,"junk":{"action":"burn"}}"#, "message"),
        (r#"{"req":13,"expand":"no-such-id"}"#, "message"),
        (
            r#"{"req":14,"set":{"key":"core.db_path","value":true}}"#,
            "message",
        ),
        (r#"{"req":15,"graph":{}}"#, "message"),
    ];
    for (text, kind) in sent {
        ws.send(Message::text(text)).await.expect("send");
        let want: Value = serde_json::from_str(text).unwrap();
        let reply = next_of(&mut ws, kind).await;
        assert_eq!(reply["req"], want["req"], "{text} -> {reply}");
    }
    ws.send(Message::text(r#"{"expand":"no-such-id"}"#))
        .await
        .expect("send bare");
    let bare = next_of(&mut ws, "message").await;
    assert!(bare.get("req").is_none(), "{bare}");
    ws.close(None).await.expect("close");
}

/// T331.12: the doctor fix over the real socket. A plan changes nothing; only `apply` writes,
/// and it leaves the `.backup_configs/` copy.
#[cfg(unix)] // POSIX hook command paths
#[tokio::test]
async fn doctor_plans_without_writing_and_applies_on_confirm() {
    let web = start("doctor").await;
    let settings = web.home.join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).expect("claude dir");
    let broken = r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "/nonexistent/old.sh"}]}]}}"#;
    std::fs::write(&settings, broken).expect("settings");
    let (mut ws, _) = connect_ws(&web.addr, &web.home).await.expect("ws connect");
    let none = r#""selection":{"keep":[],"toggled":[]}"#;

    ws.send(Message::text(format!(
        r#"{{"doctor":{{"action":"plan",{none}}}}}"#
    )))
    .await
    .expect("send plan");
    let plan = next_of(&mut ws, "doctorplan").await;
    let items = plan["plan"]["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{plan}");
    assert_eq!(items[0]["kind"], "broken-hook");
    assert_eq!(items[0]["selected"], true);
    assert!(plan["plan"]["diff"].as_str().unwrap().contains("old.sh"));
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), broken);

    // T576: an apply that does not name the previewed plan writes nothing and answers with one.
    ws.send(Message::text(format!(
        r#"{{"doctor":{{"action":"apply",{none}}}}}"#
    )))
    .await
    .expect("send unbound apply");
    let refused = next_of(&mut ws, "doctorfixed").await;
    assert_eq!(refused["fixed"]["code"], 1, "{refused}");
    assert_eq!(refused["fixed"]["replan"]["token"], plan["plan"]["token"]);
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), broken);

    let token = plan["plan"]["token"].as_str().expect("token");
    ws.send(Message::text(format!(
        r#"{{"doctor":{{"action":"apply",{none},"confirmed":"{token}"}}}}"#
    )))
    .await
    .expect("send apply");
    let fixed = next_of(&mut ws, "doctorfixed").await;
    assert_eq!(fixed["fixed"]["code"], 0, "{fixed}");
    assert!(
        !std::fs::read_to_string(&settings)
            .unwrap()
            .contains("old.sh")
    );
    // `Writer::backup` keeps the copy in a `.backup_configs/` next to the file it replaces.
    let backups = std::fs::read_dir(settings.parent().unwrap().join(".backup_configs"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert!(backups > 0, "no backup copy was kept");

    // A malformed doctor message is refused, not guessed at.
    ws.send(Message::text(r#"{"doctor":{"action":"burn"}}"#))
        .await
        .expect("send bad");
    let msg = next_of(&mut ws, "message").await;
    assert!(
        msg["text"].as_str().unwrap().contains("doctor needs"),
        "{msg}"
    );
    ws.close(None).await.expect("close");
}

/// T330.7: "clear safe junk" over the real socket. A plan removes nothing; `apply` removes
/// only a path the page confirmed, and a stale rtok log generation is the junk (the throwaway
/// home has no installed host).
#[tokio::test]
async fn junk_plans_without_removing_and_applies_only_confirmed_paths() {
    let web = start("junk").await;
    let logs = web.home.join("logs");
    std::fs::create_dir_all(&logs).expect("log dir");
    let (shown, unseen) = (logs.join("rtok.log.7"), logs.join("rtok.log.8"));
    let old = std::time::SystemTime::now() - Duration::from_secs(2 * 86_400);
    for f in [&shown, &unseen] {
        std::fs::write(f, b"stale").expect("stale log");
        std::fs::File::options()
            .write(true)
            .open(f)
            .and_then(|f| f.set_modified(old))
            .expect("age the log past the settle window");
    }
    let (mut ws, _) = connect_ws(&web.addr, &web.home).await.expect("ws connect");

    ws.send(Message::text(r#"{"junk":{"action":"plan"}}"#))
        .await
        .expect("send plan");
    let plan = next_of(&mut ws, "junkplan").await;
    let items = plan["plan"]["items"].as_array().expect("items");
    let listed: Vec<&str> = items.iter().filter_map(|i| i["path"].as_str()).collect();
    assert!(listed.contains(&shown.to_str().unwrap()), "{plan}");
    assert_eq!(plan["plan"]["yes"], false);
    assert!(
        shown.exists() && unseen.exists(),
        "a plan removed something"
    );

    let apply = serde_json::json!({"junk": {"action": "apply", "paths": [shown]}});
    ws.send(Message::text(apply.to_string()))
        .await
        .expect("send apply");
    let cleared = next_of(&mut ws, "junkcleared").await;
    assert_eq!(cleared["cleared"]["yes"], true, "{cleared}");
    assert!(!shown.exists(), "the confirmed path stayed");
    assert!(
        unseen.exists(),
        "a path the user never confirmed was removed"
    );

    ws.send(Message::text(r#"{"junk":{"action":"burn"}}"#))
        .await
        .expect("send bad");
    let msg = next_of(&mut ws, "message").await;
    assert!(
        msg["text"].as_str().unwrap().contains("junk needs"),
        "{msg}"
    );
    ws.close(None).await.expect("close");
}
