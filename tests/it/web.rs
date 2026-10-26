// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Dashboard HTTP + WebSocket smoke (P19), the T60.5 plugin `set` allow-list,
//! and T60.4 inbound `{"expand": id}`.

use std::future::IntoFuture;
use std::sync::Arc;

use rtok::config::Config;
use rtok::testutil::config_file_in;
use rtok::web::spa::{self, Assets};
use rtok::web::{DashState, app, app_with_assets};

async fn serve(
    label: &str,
) -> (
    String,
    Arc<DashState>,
    std::path::PathBuf,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let dir = std::env::temp_dir().join(format!("rtok-dash-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let cfg = config_file_in(&dir);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let state = Arc::new(DashState::new(cfg));
    let task = tokio::spawn(axum::serve(listener, app(state.clone())).into_future());
    (addr, state, dir, task)
}

#[tokio::test]
async fn web_health_and_index() {
    let (addr, _state, _dir, task) = serve("health").await;
    let body = reqwest::Client::new()
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .expect("health")
        .text()
        .await
        .expect("body");
    let health: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(health["ok"], true, "{body}");
    let html = reqwest::Client::new()
        .get(format!("http://{addr}/"))
        .send()
        .await
        .expect("index")
        .text()
        .await
        .expect("html");
    assert!(html.contains("id=\"root\"") || spa::embedded_is_placeholder());
    task.abort();
}

/// T206: `socket_loop` used to build every tick's snapshot synchronously while holding
/// `DashState::cfg`'s mutex, so one slow build (doctor probes, a whole-transcript parse, a
/// blocking fetch) froze `/health` and every other socket along with it. A builder blocked on
/// a barrier stands in for that slow build without actually waiting on one; several WS clients
/// tick at once (coalescing onto the one in-flight build), and `/health` must still answer
/// while the build is stuck — bounded generously so slow CI runners do not flake.
#[tokio::test]
async fn health_answers_during_a_snapshot_build() {
    use std::sync::Barrier;
    use std::time::Duration;

    let dir = std::env::temp_dir().join(format!("rtok-dash-slowbuild-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let cfg = config_file_in(&dir);

    let barrier = Arc::new(Barrier::new(2));
    let build_barrier = barrier.clone();
    let build_fn: rtok::web::BuildFn = Arc::new(move |cfg| {
        build_barrier.wait();
        rtok::web::frame(cfg)
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let state = Arc::new(DashState::with_builder(cfg, build_fn));
    let task = tokio::spawn(axum::serve(listener, app(state)).into_future());

    // Several sockets ticking at once must coalesce onto the one in-flight (barrier-stuck)
    // build rather than each running their own.
    let mut sockets = Vec::new();
    for _ in 0..3 {
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
            .await
            .expect("ws connect");
        sockets.push(ws);
    }
    // Give the sockets' first tick time to reach `spawn_blocking` and hit the barrier.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let health = tokio::time::timeout(
        Duration::from_secs(5),
        reqwest::Client::new()
            .get(format!("http://{addr}/health"))
            .send(),
    )
    .await
    .expect("/health must not wait on the in-flight snapshot build")
    .expect("health request");
    assert_eq!(health.status(), 200);
    let body = health.text().await.expect("body");
    let body: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(body["ok"], true, "{body}");

    // Release the stuck build so the server task can finish cleanly.
    barrier.wait();
    drop(sockets);
    task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn snapshot_error_when_store_path_is_a_directory() {
    let dir = std::env::temp_dir().join(format!("rtok-web-store-err-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = config_file_in(&dir);
    cfg.core.db_path = dir.join("not-a-db");
    std::fs::create_dir_all(&cfg.core.db_path).unwrap();
    let snap = rtok::web::model::snapshot(&cfg);
    assert!(snap.error.is_some(), "{:?}", snap.error);
    let v = serde_json::to_value(&snap).unwrap();
    assert!(v.get("error").is_some(), "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// T228: `config get` and the Config page share `model::config_entries` (D27), so
/// the CLI seeing a `RTOK_*` var as D12's `env` source proves the page would too.
/// `unsafe_code = "forbid"` rules out `std::env::set_var` on this process; a spawned
/// process's env is safe to set (`Command::env`, `dry_run.rs`'s pattern).
#[test]
fn config_page_source_reflects_an_env_override() {
    let dir = std::env::temp_dir().join(format!("rtok-web-config-env-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rtok"))
        .args(["config", "get", "proxy.port", "--json"])
        .env("RTOK_HOME", &dir)
        .env("RTOK_PROXY_PORT", "9911")
        .output()
        .expect("run rtok config get");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v["source"], "env", "{v}");
    assert_eq!(v["value"], "9911", "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// T229: the Services page folds `demon status`'s per-service rows (no state file
/// on a fresh fixture, so every service reads `stopped`, like `demon status` itself)
/// with `otel status`'s pending count — one inserted `logs` row past the (zero) mark
/// is the "non-zero watermark" the plan card asks for, through the same
/// `Store::otel_pending`/`last_log` `otel_status` already reads (D27, no second
/// reader).
#[test]
fn services_page_reflects_a_stopped_service_and_a_pending_otel_row() {
    let (cfg, dir) = rtok::testutil::config("services-fixture");
    let store = rtok::store::Store::open(&cfg.core.db_path).expect("open store");
    store
        .insert_log("error", "otel", "flush", "boom", None, None, None)
        .expect("insert log");
    let snap = rtok::web::model::snapshot(&cfg);
    let text = snap.services.expect("services page answers");
    assert!(text.contains("stopped"), "{text}");
    assert!(text.contains("logs_pending=1"), "{text}");
    assert!(text.contains("boom"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn ws_set_accepts_plugin_enabled() {
    let (_addr, state, dir, task) = serve("set-ok").await;
    let before: serde_json::Value = serde_json::from_str(&state.snapshot_json()).expect("snap");
    let cmd = before["plugins"]
        .as_array()
        .expect("plugins")
        .iter()
        .find(|p| p["id"] == "cmd")
        .expect("cmd row");
    assert_eq!(cmd["enabled"], true, "{cmd}");

    let msg = r#"{"set":{"key":"plugins.cmd.enabled","value":false}}"#;
    assert!(state.inbound(msg).is_none(), "allow-listed set is accepted");
    assert!(
        !Config::load_from(&dir).unwrap().plugin_enabled("cmd", true),
        "the write went through config set's writer"
    );
    let after: serde_json::Value = serde_json::from_str(&state.snapshot_json()).expect("snap");
    let cmd = after["plugins"]
        .as_array()
        .expect("plugins")
        .iter()
        .find(|p| p["id"] == "cmd")
        .expect("cmd row");
    assert_eq!(cmd["enabled"], false, "next snapshot reflects the toggle");
    task.abort();
}

#[tokio::test]
async fn ws_set_refuses_other_keys() {
    let (_addr, state, dir, task) = serve("set-no").await;
    let before = std::fs::read_to_string(dir.join("config.toml")).unwrap_or_default();
    let reply = state
        .inbound(r#"{"set":{"key":"proxy.port","value":true}}"#)
        .expect("refused");
    let v: serde_json::Value = serde_json::from_str(&reply).expect("message frame");
    assert_eq!(v["type"], "message", "{reply}");
    assert!(
        v["text"].as_str().unwrap_or("").contains("refused key"),
        "{reply}"
    );
    let after = std::fs::read_to_string(dir.join("config.toml")).unwrap_or_default();
    assert_eq!(after, before, "a refused key must not write the file");
    let unknown = state
        .inbound(r#"{"set":{"key":"plugins.nope.enabled","value":true}}"#)
        .expect("unknown plugin id is not allow-listed");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&unknown).unwrap()["type"],
        "message"
    );
    task.abort();
}

#[tokio::test]
async fn ws_expand_returns_payload_and_unknown_id() {
    let (_addr, state, dir, task) = serve("expand-ok").await;
    let cfg = Config::load_from(&dir).expect("cfg");
    let cx = rtok::plugin::Runtime::open(cfg, "s").expect("runtime");
    let id = cx
        .store
        .put_archive("s", b"alpha\nNEEDLE\n", &cx.config.core.archive_dir)
        .expect("archive");
    drop(cx);

    let reply = state
        .inbound(&format!(r#"{{"expand":"{id}"}}"#))
        .expect("expand frame");
    let v: serde_json::Value = serde_json::from_str(&reply).expect("json");
    assert_eq!(v["type"], "expand", "{reply}");
    assert_eq!(v["id"], id, "{reply}");
    assert!(
        v["text"].as_str().unwrap_or("").contains("NEEDLE"),
        "{reply}"
    );

    let missing = state
        .inbound(r#"{"expand":"no-such-id"}"#)
        .expect("unknown");
    let v: serde_json::Value = serde_json::from_str(&missing).expect("json");
    assert_eq!(v["type"], "message", "{missing}");
    assert!(
        v["text"]
            .as_str()
            .unwrap_or("")
            .contains("unknown archive id"),
        "{missing}"
    );

    assert!(
        state
            .inbound(r#"{"set":{"key":"plugins.cmd.enabled","value":false}}"#)
            .is_none(),
        "expand must not break the T60.5 set allow-list"
    );
    task.abort();
}

/// T193: a cross-site page must not upgrade `/ws`; same-origin and header-less
/// clients (tests/CLI) keep working. Raw HTTP: asserting the upgrade status
/// needs no WebSocket client dependency.
#[tokio::test]
async fn ws_upgrade_rejects_foreign_origin() {
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    async fn upgrade_status(addr: &str, origin: Option<&str>) -> u16 {
        let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let mut req = format!(
            "GET /ws HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
              Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n"
        );
        if let Some(o) = origin {
            req.push_str(&format!("Origin: {o}\r\n"));
        }
        req.push_str("\r\n");
        stream.write_all(req.as_bytes()).await.expect("write");
        let mut line = String::new();
        tokio::time::timeout(
            Duration::from_secs(10),
            BufReader::new(&mut stream).read_line(&mut line),
        )
        .await
        .expect("status within 10 s")
        .expect("status");
        line.split_whitespace()
            .nth(1)
            .expect("code")
            .parse()
            .expect("u16")
    }

    let (addr, _state, _dir, task) = serve("origin").await;
    assert_eq!(
        upgrade_status(&addr, Some("http://evil.example")).await,
        403,
        "foreign origin refused"
    );
    assert_eq!(
        upgrade_status(&addr, Some(&format!("http://{addr}"))).await,
        101,
        "same origin upgrades"
    );
    assert_eq!(
        upgrade_status(&addr, None).await,
        101,
        "header-less clients keep working"
    );
    task.abort();
}

/// T310.9: the SPA's source is injected, so every serving rule is checked against a fixture
/// `dist/` (the layout `vite build` + `precompress.mjs` write) without reading
/// the process environment or depending on a built `web/dist`.
const INDEX_HTML: &str = "<!doctype html><div id=\"root\"></div>";

fn fixture_dist(label: &str) -> std::path::PathBuf {
    let dist = std::env::temp_dir().join(format!("rtok-dist-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dist);
    std::fs::create_dir_all(dist.join("assets")).expect("dist dir");
    std::fs::write(dist.join("index.html"), INDEX_HTML).expect("index");
    std::fs::write(dist.join("assets/app-1a2b3c.js"), "export default 1;\n").expect("js");
    std::fs::write(dist.join("assets/app-1a2b3c.js.br"), b"BR-BYTES").expect("br");
    std::fs::write(dist.join("assets/app-1a2b3c.js.gz"), b"GZ-BYTES").expect("gz");
    dist
}

async fn serve_assets(
    label: &str,
    assets: Assets,
) -> (String, tokio::task::JoinHandle<std::io::Result<()>>) {
    let dir = std::env::temp_dir().join(format!("rtok-spa-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let cfg = config_file_in(&dir);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let app = app_with_assets(Arc::new(DashState::new(cfg)), assets);
    let task = tokio::spawn(axum::serve(listener, app).into_future());
    (addr, task)
}

#[tokio::test]
async fn the_index_is_revalidated_and_carries_a_csp() {
    let dist = fixture_dist("index");
    let (addr, task) = serve_assets("index", Assets::Dir(dist.clone())).await;
    let res = reqwest::get(format!("http://{addr}/"))
        .await
        .expect("index");
    assert_eq!(res.status(), 200);
    let h = res.headers();
    assert!(h["content-type"].to_str().unwrap().starts_with("text/html"));
    assert_eq!(h["cache-control"], "no-cache");
    assert_eq!(h["x-content-type-options"], "nosniff");
    let csp = h["content-security-policy"].to_str().unwrap().to_string();
    assert!(
        csp.contains("script-src 'self';"),
        "no inline scripts: {csp}"
    );
    assert!(csp.contains("connect-src 'self'") && csp.contains("frame-ancestors 'none'"));
    assert_eq!(res.text().await.expect("body"), INDEX_HTML);
    task.abort();
    let _ = std::fs::remove_dir_all(&dist);
}

#[tokio::test]
async fn hashed_assets_are_immutable_and_precompressed_variants_follow_the_client() {
    let dist = fixture_dist("assets");
    let (addr, task) = serve_assets("assets", Assets::Dir(dist.clone())).await;
    let url = format!("http://{addr}/assets/app-1a2b3c.js");
    let client = reqwest::Client::new();
    let get = |accept: Option<&'static str>| {
        let mut req = client.get(&url);
        if let Some(a) = accept {
            req = req.header("accept-encoding", a);
        }
        async move { req.send().await.expect("asset") }
    };

    let plain = get(None).await;
    assert_eq!(plain.status(), 200);
    assert!(
        plain.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("javascript")
    );
    assert_eq!(
        plain.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    assert!(plain.headers().get("content-encoding").is_none());
    assert_eq!(
        plain.bytes().await.unwrap().as_ref(),
        b"export default 1;\n"
    );

    let br = get(Some("gzip, br")).await;
    assert_eq!(br.headers()["content-encoding"], "br");
    assert!(
        br.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("javascript")
    );
    assert_eq!(br.headers()["vary"], "Accept-Encoding");
    assert_eq!(br.bytes().await.unwrap().as_ref(), b"BR-BYTES");

    let gz = get(Some("gzip")).await;
    assert_eq!(gz.headers()["content-encoding"], "gzip");
    assert_eq!(gz.bytes().await.unwrap().as_ref(), b"GZ-BYTES");

    let refused = get(Some("br;q=0, gzip;q=0")).await;
    assert!(refused.headers().get("content-encoding").is_none());

    // The variants exist only through negotiation.
    let direct = reqwest::get(format!("{url}.br")).await.expect("variant");
    assert_eq!(direct.status(), 404);
    task.abort();
    let _ = std::fs::remove_dir_all(&dist);
}

#[tokio::test]
async fn page_routes_fall_back_to_the_index_but_api_and_files_do_not() {
    let dist = fixture_dist("fallback");
    let (addr, task) = serve_assets("fallback", Assets::Dir(dist.clone())).await;
    let status = |path: &'static str| {
        let url = format!("http://{addr}{path}");
        async move { reqwest::get(url).await.expect("get") }
    };
    let page = status("/sessions/abc").await;
    assert_eq!(page.status(), 200);
    assert_eq!(page.headers()["cache-control"], "no-cache");
    assert_eq!(page.text().await.unwrap(), INDEX_HTML);
    for path in [
        "/ws/x",
        "/api/nope",
        "/assets/missing",
        "/missing.png",
        "/a/b.js",
    ] {
        assert_eq!(status(path).await.status(), 404, "{path}");
    }
    let health = status("/health").await;
    assert_eq!(health.status(), 200);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&health.text().await.unwrap()).unwrap()["ok"],
        true
    );
    let post = reqwest::Client::new()
        .post(format!("http://{addr}/"))
        .send()
        .await
        .expect("post");
    assert_eq!(post.status(), 405);
    task.abort();
    let _ = std::fs::remove_dir_all(&dist);
}

/// T574: a browser sends `assets/logo space.svg` as `logo%20space.svg`; the lookup needs the file
/// name, not the escape. Raw requests where the client library would normalise the path itself.
async fn raw_status(addr: &str, method: &str, target: &str) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let req = format!("{method} {target} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.expect("write");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.expect("read");
    String::from_utf8_lossy(&raw)
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("status line")
}

#[tokio::test]
async fn percent_encoded_asset_urls_resolve_the_file_they_name() {
    let dist = fixture_dist("encoded");
    let names = [
        "logo space.svg",
        "a#b.svg",
        "100%.svg",
        "caf\u{e9}.svg",
        "a%20b.svg",
        "plain.svg",
    ];
    for name in names {
        std::fs::write(
            dist.join("assets").join(name),
            format!("<svg id=\"{name}\"/>"),
        )
        .unwrap();
    }
    std::fs::write(dist.join("assets/logo space.svg.br"), b"BR-LOGO").unwrap();
    let secret = std::env::temp_dir().join(format!("rtok-secret-{}.txt", std::process::id()));
    std::fs::write(&secret, "secret").unwrap();
    let (addr, task) = serve_assets("encoded", Assets::Dir(dist.clone())).await;
    let get = |path: &str| reqwest::get(format!("http://{addr}{path}"));

    for (url, name) in [
        ("/assets/plain.svg", "plain.svg"),
        ("/assets/logo%20space.svg", "logo space.svg"),
        ("/assets/a%23b.svg", "a#b.svg"),
        ("/assets/100%25.svg", "100%.svg"),
        ("/assets/caf%C3%A9.svg", "caf\u{e9}.svg"),
        // Decoded once: `%2520` is the literal text `%20`, not a space.
        ("/assets/a%2520b.svg", "a%20b.svg"),
    ] {
        let res = get(url).await.expect(url);
        assert_eq!(res.status(), 200, "{url}");
        assert_eq!(res.headers()["content-type"], "image/svg+xml", "{url}");
        assert_eq!(
            res.text().await.unwrap(),
            format!("<svg id=\"{name}\"/>"),
            "{url}"
        );
    }
    // `%20` names "a b.svg", which does not exist, even though "a%20b.svg" does.
    assert_eq!(get("/assets/a%20b.svg").await.unwrap().status(), 404);
    assert_eq!(get("/assets/no%20such.svg").await.unwrap().status(), 404);

    let br = reqwest::Client::new()
        .get(format!("http://{addr}/assets/logo%20space.svg"))
        .header("accept-encoding", "br")
        .send()
        .await
        .unwrap();
    assert_eq!(br.headers()["content-encoding"], "br");
    assert_eq!(br.bytes().await.unwrap().as_ref(), b"BR-LOGO");
    assert_eq!(
        raw_status(&addr, "HEAD", "/assets/logo%20space.svg").await,
        200
    );

    // Encoded traversal and separators never leave `assets/`; malformed text is refused outright.
    let outside = format!("rtok-secret-{}.txt", std::process::id());
    assert_eq!(
        raw_status(&addr, "GET", &format!("/%2e%2e/{outside}")).await,
        404
    );
    assert_eq!(
        raw_status(&addr, "GET", &format!("/assets/%2E%2E/%2e%2e/{outside}")).await,
        404
    );
    for bad in [
        "/assets%2Fplain.svg",
        "/assets/..%2F..%2Fplain.svg",
        "/assets%5Cplain.svg",
        "/assets/plain.svg%00",
        "/assets/100%.svg",
        "/assets/%zz.svg",
        "/assets/%FF.svg",
    ] {
        assert_eq!(raw_status(&addr, "GET", bad).await, 400, "{bad}");
    }
    task.abort();
    let _ = std::fs::remove_dir_all(&dist);
    let _ = std::fs::remove_file(&secret);
}

#[tokio::test]
async fn a_dist_without_an_index_says_so_and_keeps_the_api_up() {
    let dist = fixture_dist("noindex");
    std::fs::remove_file(dist.join("index.html")).expect("rm index");
    let (addr, task) = serve_assets("noindex", Assets::Dir(dist.clone())).await;
    let res = reqwest::get(format!("http://{addr}/"))
        .await
        .expect("index");
    assert_eq!(res.status(), 503);
    // A fixed check: echoing the body into the panic is CodeQL `rust/log-injection`.
    assert!(res.text().await.expect("body").contains("RTOK_WEB_DIST"));
    let health = reqwest::get(format!("http://{addr}/health"))
        .await
        .expect("health");
    assert_eq!(health.status(), 200, "the API stays up without the UI");
    task.abort();
    let _ = std::fs::remove_dir_all(&dist);
}

/// The embedded copy is what a ketch install serves. Its `ETag` is the compile-time digest, so
/// a repeat request revalidates to a 304; when `web/dist` is built, `/` is that exact file.
#[tokio::test]
async fn the_embedded_ui_revalidates_by_etag_and_matches_web_dist() {
    let (addr, task) = serve_assets("embedded", Assets::Embedded).await;
    let url = format!("http://{addr}/");
    let first = reqwest::get(&url).await.expect("index");
    assert_eq!(first.status(), 200);
    let etag = first.headers()["etag"].to_str().unwrap().to_string();
    let body = first.bytes().await.expect("body");
    let again = reqwest::Client::new()
        .get(&url)
        .header("if-none-match", &etag)
        .send()
        .await
        .expect("again");
    assert_eq!(again.status(), 304);
    assert!(again.headers()["content-security-policy"].to_str().is_ok());

    let built = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web/dist/index.html");
    match std::fs::read(&built) {
        Ok(file) => {
            assert!(
                !spa::embedded_is_placeholder(),
                "web/dist exists but was not embedded"
            );
            assert_eq!(
                body.as_ref(),
                file.as_slice(),
                "embedded index != web/dist/index.html"
            );
        }
        Err(_) => {
            assert!(spa::embedded_is_placeholder());
            assert!(String::from_utf8_lossy(&body).contains("just web"));
        }
    }
    task.abort();
}

#[tokio::test]
async fn ws_project_select_link_and_unlink_reach_the_next_snapshot_and_a_bad_id_is_refused() {
    let (_addr, state, dir, task) = serve("projects").await;
    let roots: Vec<_> = ["a", "b"]
        .iter()
        .map(|n| {
            let p = dir.join(n);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("lib.rs"), "fn used() {}\n").unwrap();
            p
        })
        .collect();
    let rt = rtok::plugin::Runtime::open(Config::load_from(&dir).unwrap(), "web-test").unwrap();
    let ids: Vec<i32> = roots
        .iter()
        .map(|p| {
            rt.store
                .register_project(p, rtok::store::Origin::Manual)
                .unwrap()
                .id
        })
        .collect();
    let rows = |state: &DashState| -> Vec<serde_json::Value> {
        let snap: serde_json::Value = serde_json::from_str(&state.snapshot_json()).unwrap();
        snap["projects"].as_array().expect("projects").clone()
    };
    assert_eq!(rows(&state).len(), 2, "the snapshot lists the registry");

    let send = |m: serde_json::Value| state.inbound(&m.to_string());
    let (a, b) = (ids[0].to_string(), ids[1].to_string());
    assert!(send(serde_json::json!({"project": {"action": "select", "project": b}})).is_none());
    let sel: Vec<_> = rows(&state).iter().map(|r| r["selected"].clone()).collect();
    assert_eq!(
        sel,
        [false, true],
        "the selection reaches the next snapshot"
    );

    let link = serde_json::json!({"project": {"action": "link", "from": a, "to": b, "both": true}});
    assert!(send(link).is_none());
    let links = |r: &serde_json::Value| r["links"].as_array().unwrap().len();
    assert_eq!(rows(&state).iter().map(links).collect::<Vec<_>>(), [1, 1]);

    let unlink = serde_json::json!({"project": {"action": "unlink", "from": a, "to": b}});
    assert!(send(unlink).is_none());
    assert_eq!(rows(&state).iter().map(links).collect::<Vec<_>>(), [0, 1]);

    let bad = send(serde_json::json!({"project": {"action": "select", "project": "9999"}}));
    let bad: serde_json::Value = serde_json::from_str(&bad.expect("refused")).unwrap();
    assert_eq!(bad["type"], "message");
    task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

/// T329.54: re-index answers at once and the row reads `indexing` then `ok`; remove answers the
/// dry run first and drops the project only for the confirmed request.
#[tokio::test]
async fn ws_reindex_runs_on_a_worker_and_remove_needs_its_confirmation() {
    let (_addr, state, dir, task) = serve("reindex-remove").await;
    let root = dir.join("a");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("lib.rs"), "fn used() {}\nfn also() {}\n").unwrap();
    let rt = rtok::plugin::Runtime::open(Config::load_from(&dir).unwrap(), "web-test").unwrap();
    let id = rt
        .store
        .register_project(&root, rtok::store::Origin::Manual)
        .unwrap()
        .id
        .to_string();
    let rows = |state: &DashState| -> Vec<serde_json::Value> {
        let snap: serde_json::Value = serde_json::from_str(&state.snapshot_json()).unwrap();
        snap["projects"].as_array().expect("projects").clone()
    };
    let send = |m: serde_json::Value| state.inbound(&m.to_string());
    assert_eq!(rows(&state)[0]["state"], "not indexed");

    let started = send(serde_json::json!({"project": {"action": "reindex", "project": id}}));
    assert!(started.is_none(), "a started job answers with the snapshot");
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while rows(&state)[0]["state"] == "indexing" {
        assert!(std::time::Instant::now() < end, "the worker never finished");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(rows(&state)[0]["state"], "ok");
    assert!(rows(&state)[0]["index"]["rows"].as_i64().unwrap() >= 2);

    let plan = send(serde_json::json!({"project": {"action": "remove", "project": id}}));
    let plan: serde_json::Value = serde_json::from_str(&plan.expect("the plan")).unwrap();
    assert_eq!(plan["type"], "projectplan");
    assert_eq!(plan["project"], id);
    assert!(plan["plan"].as_str().unwrap().contains("files untouched"));
    assert_eq!(
        rows(&state).len(),
        1,
        "an unconfirmed remove writes nothing"
    );

    let gone = send(
        serde_json::json!({"project": {"action": "remove", "project": id, "confirmed": true}}),
    );
    assert!(gone.is_none());
    assert!(rows(&state).is_empty());
    assert!(root.join("lib.rs").exists(), "files are never touched");

    let bad = send(serde_json::json!({"project": {"action": "reindex", "project": "9999"}}));
    let bad: serde_json::Value = serde_json::from_str(&bad.expect("refused")).unwrap();
    assert_eq!(bad["type"], "message");
    task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

/// T329.14: the drill-down request answers with a `graph` frame; an unknown project and a request
/// that names none are a `message`, and nothing is written.
#[tokio::test]
async fn ws_graph_answers_a_project_with_its_files_and_refuses_an_unknown_one() {
    let (_addr, state, dir, task) = serve("drill").await;
    let root = dir.join("a");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.rs"), "fn one() {\n    two();\n}\n").unwrap();
    std::fs::write(root.join("b.rs"), "fn two() {}\n").unwrap();
    let rt = rtok::plugin::Runtime::open(Config::load_from(&dir).unwrap(), "web-test").unwrap();
    rtok::plugins::graph::index::run(&rtok_plugin_sdk::Ctx::new(&rt), &root, false).unwrap();
    let id = rt
        .store
        .register_project(&root, rtok::store::Origin::Manual)
        .unwrap()
        .id;

    let ask = |m: serde_json::Value| -> serde_json::Value {
        serde_json::from_str(&state.inbound(&m.to_string()).expect("a reply")).unwrap()
    };
    let g = ask(serde_json::json!({"graph": {"project": id.to_string(), "expand": ["b.rs"]}}));
    assert_eq!(g["type"], "graph");
    assert_eq!(g["graph"]["state"], "ok");
    let ids: Vec<_> = g["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.contains(&"f:a.rs".to_string()), "{ids:?}");
    assert!(ids.iter().any(|i| i.starts_with("s:b.rs:1:two")), "{ids:?}");

    let bad = ask(serde_json::json!({"graph": {"project": "9999"}}));
    assert_eq!(bad["type"], "message");
    let bad = ask(serde_json::json!({"graph": {}}));
    assert_eq!(bad["type"], "message");
    task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}
