// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Local operator dashboard: one process serves the WebSocket API and the React SPA.
//!
//! `rtok web --host --port`. Does not enter the hook path. The SPA (`web/`) is built to
//! `web/dist` and embedded by [`spa`] (T310.9), so the hook binary links no UI toolkit.
//! Every value served comes from [`model`], the operator model `rtok tui` renders too (D23).

#![allow(clippy::print_stdout, clippy::print_stderr)]

pub use crate::model;
pub mod calls_store;
pub mod calls_view;
pub mod live;
pub mod protocol;
pub mod spa;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::config::{Config, validate};
use crate::plugins::Registry;
use protocol::{ClientMessage, DoctorAction, DoctorRequest, JunkAction, JunkRequest, ServerFrame};

/// Builds one snapshot frame from a config. Production always uses [`frame`]; tests can
/// substitute a slower or instrumented builder to exercise T206's build coalescing (a real
/// busy store, or just a barrier) without waiting on a real busy store.
pub type BuildFn = Arc<dyn Fn(&Config) -> String + Send + Sync>;

/// T206: at most one snapshot build runs at a time; a tick that lands while one is already
/// running awaits its result instead of starting a redundant build.
enum BuildSlot {
    Idle,
    InFlight(watch::Receiver<Option<String>>),
}

pub struct DashState {
    cfg: Mutex<Config>,
    build: Mutex<BuildSlot>,
    build_fn: BuildFn,
    /// The graph call events every subscribed socket shares (T329.15).
    live: Arc<live::LiveCalls>,
}

impl DashState {
    pub fn new(cfg: Config) -> Self {
        Self::with_builder(cfg, Arc::new(frame))
    }

    /// For tests: swap the snapshot builder, e.g. to block it on a barrier so `/health` can be
    /// asserted to answer while a build is stuck (T206).
    pub fn with_builder(cfg: Config, build_fn: BuildFn) -> Self {
        let live = Arc::new(live::LiveCalls::new(&cfg));
        Self {
            cfg: Mutex::new(cfg),
            build: Mutex::new(BuildSlot::Idle),
            build_fn,
            live,
        }
    }

    /// One inbound `/ws` text frame (T60.5 / T60.4). `None` means ignore, or an
    /// accepted `set` whose next snapshot carries the write. `Some` is a message
    /// frame (refused `set`) or an `expand` payload frame.
    pub fn inbound(&self, text: &str) -> Option<String> {
        inbound(self, text)
    }

    /// A copy of the live configuration, taken under its lock.
    fn config(&self) -> Config {
        self.cfg
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The snapshot frame `/ws` sends next, after any accepted `set`.
    pub fn snapshot_json(&self) -> String {
        frame(&self.cfg.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The snapshot frame for one `/ws` tick (T206). The `Config` mutex is held only long
    /// enough to clone it — never across the build — and the build itself runs in
    /// `spawn_blocking` so it cannot stall the executor or `health()`/`inbound()`'s lock.
    /// Ticks that land while a build is already in flight share its result.
    async fn snapshot_shared(&self) -> String {
        // The check-and-set that either joins an in-flight build or claims the builder role
        // happens under one lock acquisition, so two ticks landing at once cannot both start a
        // build. A `MutexGuard` never lives across an `.await` (each is scoped to its own
        // block), so this future stays `Send` (required by `on_upgrade`).
        enum Claim {
            Join(watch::Receiver<Option<String>>),
            Build(watch::Sender<Option<String>>),
        }
        let claim = {
            let mut guard = self.build.lock().unwrap_or_else(PoisonError::into_inner);
            match &*guard {
                BuildSlot::InFlight(rx) => Claim::Join(rx.clone()),
                BuildSlot::Idle => {
                    let (tx, rx) = watch::channel(None);
                    *guard = BuildSlot::InFlight(rx);
                    Claim::Build(tx)
                }
            }
        };
        let tx = match claim {
            Claim::Build(tx) => tx,
            Claim::Join(mut rx) => {
                if rx.changed().await.is_ok()
                    && let Some(snap) = rx.borrow().clone()
                {
                    return snap;
                }
                // The build ahead of us never sent (it panicked): claim it ourselves instead
                // of hanging this socket forever.
                let mut guard = self.build.lock().unwrap_or_else(PoisonError::into_inner);
                let (tx, rx) = watch::channel(None);
                *guard = BuildSlot::InFlight(rx);
                tx
            }
        };

        let cfg = self
            .cfg
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let build_fn = self.build_fn.clone();
        let snap = tokio::task::spawn_blocking(move || build_fn(&cfg))
            .await
            .unwrap_or_else(|_| {
                json!({"type": "snapshot", "error": "snapshot build panicked"}).to_string()
            });

        *self.build.lock().unwrap_or_else(PoisonError::into_inner) = BuildSlot::Idle;
        let _ = tx.send(Some(snap.clone()));
        snap
    }
}

/// `rtok web`: bind `host:port` and serve until killed.
pub fn serve_blocking(cfg: Config) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("tokio runtime")?;
    rt.block_on(serve(cfg))
}

pub async fn serve(cfg: Config) -> Result<()> {
    let addr: SocketAddr = format!("{}:{}", cfg.web.host, cfg.web.port)
        .parse()
        .with_context(|| format!("dashboard bind {}:{}", cfg.web.host, cfg.web.port))?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr}"))?;
    eprintln!("rtok web http://{addr}  (ws://{addr}/ws)");
    crate::log::append(
        &cfg,
        "info",
        "web",
        "serve",
        &format!("http://{addr} websocket /ws"),
    );
    let (assets, notice) = spa::resolve(std::env::var_os(spa::DIST_ENV));
    if let Some(msg) = notice {
        eprintln!("{msg}");
        crate::log::append(&cfg, "warn", "web", "spa", &msg);
    }
    #[cfg(feature = "graph")]
    let health = health_check(&cfg);
    let served = axum::serve(
        listener,
        app_with_assets(Arc::new(DashState::new(cfg)), assets),
    )
    .await
    .context("dashboard server");
    #[cfg(feature = "graph")]
    health.store(true, std::sync::atomic::Ordering::Relaxed);
    served
}

/// T329.17: `rtok web` hosts graph, so it runs the health check too, over every registered
/// project. Its own thread and store handle: the check must never wait on a page request.
#[cfg(feature = "graph")]
fn health_check(cfg: &Config) -> Arc<std::sync::atomic::AtomicBool> {
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (cfg, flag) = (cfg.clone(), Arc::clone(&stop));
    std::thread::spawn(move || {
        if let Ok(rt) = crate::plugin::Runtime::open(cfg, "web-health") {
            crate::plugins::graph::health::run(&rt, None, &flag);
        }
    });
    stop
}

/// The router with the SPA embedded in this binary; tests and callers that must not read the
/// process environment use this, `serve` resolves `RTOK_WEB_DIST` itself.
pub fn app(state: Arc<DashState>) -> Router {
    app_with_assets(state, spa::Assets::Embedded)
}

/// `app` with the SPA's source already resolved, so tests cover every source without touching
/// the process environment. The API paths are explicit routes; everything else is the SPA's.
pub fn app_with_assets(state: Arc<DashState>, assets: spa::Assets) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_upgrade))
        .with_state(state)
        .fallback_service(spa::router(assets))
}

async fn health(State(state): State<Arc<DashState>>) -> Json<Value> {
    let cfg = state.cfg.lock().unwrap_or_else(|e| e.into_inner());
    Json(json!({
        "ok": true,
        "host": cfg.web.host,
        "port": cfg.web.port,
    }))
}

async fn ws_upgrade(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<Arc<DashState>>,
) -> impl IntoResponse {
    if !origin_allowed(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.on_upgrade(move |socket| socket_loop(socket, state))
        .into_response()
}

/// Browsers send `Origin` on cross-site WebSocket upgrades; non-browser clients
/// send none. Reject a present `Origin` whose host differs from `Host`, and one
/// whose `Host` is a DNS name other than `localhost`: a rebinding page
/// (`evil.example` resolved to 127.0.0.1) sends a matching `Origin` and `Host`,
/// and an IP literal or `localhost` is the only name it cannot point here.
fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let Some(host) = headers.get("host").and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let (Some(o), Some(h)) = (origin_authority(origin), host_authority(host)) else {
        return false;
    };
    // The port is part of the origin: another local app's page must not get in.
    let default = default_port(origin);
    let same_port = port_of(o).unwrap_or(default) == port_of(h).unwrap_or(default);
    let (o, h) = (strip_port(o), strip_port(h));
    o.eq_ignore_ascii_case(h) && same_port && rebind_safe(h)
}

const HTTP_DEFAULT_PORT: &str = "80";
const HTTPS_DEFAULT_PORT: &str = "443";

/// The port a browser implies for `origin`'s scheme when the authority names none.
fn default_port(origin: &str) -> &'static str {
    let scheme = origin.split("://").next().unwrap_or_default();
    if scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("wss") {
        HTTPS_DEFAULT_PORT
    } else {
        HTTP_DEFAULT_PORT
    }
}

fn rebind_safe(host: &str) -> bool {
    host.parse::<std::net::IpAddr>().is_ok()
        || host.eq_ignore_ascii_case("localhost")
        || host.to_ascii_lowercase().ends_with(".localhost")
}

fn origin_authority(origin: &str) -> Option<&str> {
    let rest = origin.split("://").nth(1)?;
    let authority = rest.split(['/', '?', '#']).next()?;
    authority.split('@').next_back()
}

fn host_authority(host: &str) -> Option<&str> {
    let host = host.split(',').next_back()?.trim();
    if host.is_empty() {
        return None;
    }
    host.split('@').next_back()
}

/// The explicit port of `authority` (`host:port` or `[v6]:port`), if any.
fn port_of(authority: &str) -> Option<&str> {
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split_once(']')?.1.strip_prefix(':');
    }
    if authority.matches(':').count() == 1 {
        return authority.split_once(':').map(|(_, port)| port);
    }
    None
}

fn strip_port(authority: &str) -> &str {
    if let Some(rest) = authority.strip_prefix('[')
        && let Some(end) = rest.find(']')
    {
        return &rest[..end];
    }
    if authority.matches(':').count() == 1
        && let Some((h, _)) = authority.split_once(':')
    {
        return h;
    }
    authority
}

/// One snapshot per tick, rendered from the operator model. Keys come out sorted, which
/// is the wire the P19 UI already reads. Public so the T15.10 parity test reads the
/// frame the surface actually sends on `/ws`.
pub fn frame(cfg: &Config) -> String {
    serde_json::to_value(model::snapshot(cfg))
        .unwrap_or_else(|_| json!({"type": "snapshot"}))
        .to_string()
}

/// The next calls frame of a subscribed socket; never resolves for one that is not. Every frame
/// is the whole state, so a socket that fell behind skips the ones in between and loses nothing.
async fn next_calls(calls: &mut Option<watch::Receiver<Arc<str>>>) -> Option<Arc<str>> {
    if let Some(rx) = calls
        && rx.changed().await.is_ok()
    {
        return Some(rx.borrow_and_update().clone());
    }
    std::future::pending().await
}

async fn socket_loop(mut socket: WebSocket, state: Arc<DashState>) {
    let mut calls: Option<watch::Receiver<Arc<str>>> = None;
    // A fixed period, not a sleep per pass: call frames arriving every few hundred
    // milliseconds must not keep pushing the next snapshot out.
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut snapshot_due = true;
    loop {
        if std::mem::take(&mut snapshot_due) {
            let snap = state.snapshot_shared().await;
            if socket.send(Message::text(snap)).await.is_err() {
                break;
            }
            tick.reset();
        }
        tokio::select! {
            frame = next_calls(&mut calls) => {
                if let Some(frame) = frame
                    && socket.send(Message::text(frame.as_ref())).await.is_err()
                {
                    break;
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Text(text))) => {
                        if let Some(on) = calls_subscription(&text) {
                            calls = None;
                            if on {
                                let (rx, armed) = state.live.subscribe(&state.config()).await;
                                if socket.send(Message::text(armed.as_ref())).await.is_err() {
                                    break;
                                }
                                calls = Some(rx);
                            }
                            continue;
                        }
                        snapshot_due = true;
                        // A registry write can index a project, so it runs off the executor.
                        let st = state.clone();
                        let text = text.to_string();
                        let reply = tokio::task::spawn_blocking(move || inbound(&st, &text))
                            .await
                            .ok()
                            .flatten();
                        if let Some(reply) = reply
                            && socket.send(Message::text(reply)).await.is_err()
                        {
                            break;
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
            _ = tick.tick() => snapshot_due = true,
        }
    }
}

/// `Some(on)` for a call-event subscription message (T329.15), which belongs to the socket and not
/// to [`inbound`]: it owns the receiver.
fn calls_subscription(text: &str) -> Option<bool> {
    match serde_json::from_str::<ClientMessage>(text) {
        Ok(ClientMessage::Calls { calls }) => Some(calls.subscribe),
        _ => None,
    }
}

fn inbound(state: &DashState, text: &str) -> Option<String> {
    let v: Value = serde_json::from_str(text).ok()?;
    let set = match ClientMessage::deserialize(&v) {
        Ok(ClientMessage::Expand { expand: id }) => {
            let cfg = state
                .cfg
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            return Some(match model::expand_payload(&cfg, &id, None) {
                Some(text) => ServerFrame::Expand { id, text }.to_json(),
                None => message_frame(&format!("unknown archive id: {id}")),
            });
        }
        Ok(ClientMessage::Project { project }) => {
            let cfg = state
                .cfg
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            return project_write(&cfg, project)
                .err()
                .map(|e| message_frame(&format!("{e:#}")));
        }
        Ok(ClientMessage::Set { set }) => set,
        // Handled by the socket loop, which owns the subscription; a stray one here is a no-op.
        Ok(ClientMessage::Calls { .. }) => return None,
        Ok(ClientMessage::Doctor { doctor }) => return Some(doctor_reply(state, &doctor)),
        Ok(ClientMessage::Junk { junk }) => return Some(junk_reply(state, &junk)),
        Ok(ClientMessage::Graph { graph }) => {
            let cfg = state
                .cfg
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            return Some(graph_reply(&cfg, &graph));
        }
        Ok(ClientMessage::Diff { diff }) => {
            let cfg = state
                .cfg
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            return Some(diff_reply(&cfg, diff));
        }
        Ok(ClientMessage::Export { export }) => {
            let cfg = state
                .cfg
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            return Some(export_reply(&cfg, &export));
        }
        Ok(ClientMessage::Import { import }) => return Some(import_reply(import)),
        Err(_) if v.get("export").is_some() => {
            return Some(message_frame(
                "export needs a project, a level and a format",
            ));
        }
        Err(_) if v.get("import").is_some() => {
            return Some(message_frame("import needs a name and the file's text"));
        }
        Err(_) if v.get("project").is_some() => {
            return Some(message_frame("unknown project request"));
        }
        Err(_) if v.get("set").is_some() => {
            return Some(message_frame("set needs a string key and a bool value"));
        }
        Err(_) if v.get("graph").is_some() => {
            return Some(message_frame("graph needs a project"));
        }
        Err(_) if v.get("diff").is_some() => {
            return Some(message_frame("diff needs a project"));
        }
        Err(_) if v.get("calls").is_some() => {
            return Some(message_frame("calls needs a subscribe flag"));
        }
        Err(_) if v.get("junk").is_some() => {
            return Some(message_frame("junk needs an action"));
        }
        Err(_) if v.get("doctor").is_some() => {
            return Some(message_frame("doctor needs an action and a selection"));
        }
        Err(_) => return None,
    };
    let (key, value) = (set.key.as_str(), set.value);
    let mut cfg = state.cfg.lock().unwrap_or_else(PoisonError::into_inner);
    if !allowlisted_plugin_enabled(&cfg, key) {
        return Some(message_frame(&format!("refused key {key}")));
    }
    match validate::set(&cfg.home, key, &value.to_string(), false) {
        Ok(_) => {
            if let Ok(reloaded) = Config::load_from(&cfg.home) {
                *cfg = reloaded;
            }
            None
        }
        Err(e) => Some(message_frame(&format!("config set {key}: {e:#}"))),
    }
}

#[cfg(feature = "graph")]
pub(crate) fn project_write(cfg: &Config, req: protocol::ProjectRequest) -> Result<String> {
    use crate::plugins::graph::projects::{Action, run};
    use protocol::ProjectRequest as R;
    let rt = crate::plugin::Runtime::open(cfg.clone(), "web-projects")?;
    let action = match req {
        R::Select { project } => Action::Select(project),
        R::Link { from, to, both } => Action::Link {
            to,
            from: Some(from),
            both,
            reason: None,
        },
        R::Unlink { from, to, both } => Action::Unlink {
            to,
            from: Some(from),
            both,
        },
    };
    run(&rt, action, false)
}

#[cfg(not(feature = "graph"))]
pub(crate) fn project_write(_cfg: &Config, _req: protocol::ProjectRequest) -> Result<String> {
    anyhow::bail!("the graph feature is not built in")
}

/// T329.14: one project's drill-down. Read-only, so unlike a registry write it needs no refusal
/// path beyond the error line.
#[cfg(feature = "graph")]
fn graph_reply(cfg: &Config, req: &model::DrillRequest) -> String {
    let drilled = crate::plugin::Runtime::open(cfg.clone(), "web-drill")
        .and_then(|rt| crate::plugins::graph::drill::run(&rt, req));
    match drilled {
        Ok(graph) => ServerFrame::Graph { graph }.to_json(),
        Err(e) => message_frame(&format!("{e:#}")),
    }
}

#[cfg(not(feature = "graph"))]
fn graph_reply(_cfg: &Config, req: &model::DrillRequest) -> String {
    match *req {}
}

/// T329.35: Compare mode. Read-only like `graph_reply`; the old side is a ref or the text the page
/// sent, never a path (see `DiffRequest`).
#[cfg(feature = "graph")]
fn diff_reply(cfg: &Config, req: model::DiffRequest) -> String {
    let project = req.project.clone();
    let diffed = crate::plugin::Runtime::open(cfg.clone(), "web-diff")
        .and_then(|rt| crate::plugins::graph::diff::page(&rt, &req));
    match diffed {
        Ok(diff) => ServerFrame::Diff { project, diff }.to_json(),
        Err(e) => message_frame(&format!("{e:#}")),
    }
}

#[cfg(not(feature = "graph"))]
fn diff_reply(_cfg: &Config, req: model::DiffRequest) -> String {
    match req {}
}

/// T329.40: the Export menu's download, from the one function `rtok graph export` runs.
#[cfg(feature = "graph")]
fn export_reply(cfg: &Config, req: &model::ExportRequest) -> String {
    let made = crate::plugin::Runtime::open(cfg.clone(), "web-export")
        .and_then(|rt| crate::plugins::graph::export::page_file(&rt, req));
    match made {
        Ok(file) => ServerFrame::Export { file }.to_json(),
        Err(e) => message_frame(&format!("{e:#}")),
    }
}

#[cfg(not(feature = "graph"))]
fn export_reply(_cfg: &Config, req: &model::ExportRequest) -> String {
    match *req {}
}

/// T329.40: the read-only import view. It only parses the text the page read, so no runtime is
/// opened and neither the registry nor the index is touched.
#[cfg(feature = "graph")]
fn import_reply(req: model::ImportRequest) -> String {
    match crate::plugins::graph::export::parse(&req.text, &req.name) {
        Ok(export) => ServerFrame::Imported {
            name: req.name,
            export,
        }
        .to_json(),
        Err(e) => message_frame(&format!("{e:#}")),
    }
}

#[cfg(not(feature = "graph"))]
fn import_reply(req: model::ImportRequest) -> String {
    match req {}
}

/// T331.12: the `doctor --fix` checklist for the page. The upgrade's origin guard covers it like
/// `set`; only `apply` writes, through the engine's backup and refusals.
fn doctor_reply(state: &DashState, r: &DoctorRequest) -> String {
    let cfg = state
        .cfg
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    match r.action {
        DoctorAction::Plan => ServerFrame::DoctorPlan {
            plan: crate::doctor::web::plan_here(&cfg, &r.selection),
        },
        DoctorAction::Apply => ServerFrame::DoctorFixed {
            fixed: crate::doctor::web::apply_here(&cfg, &r.selection),
        },
    }
    .to_json()
}

/// T330.7: "clear safe junk" for the Hosts page. The upgrade's origin guard covers it like `set`;
/// only `apply` removes, through the same plan, re-checks and refusals as `agents junk clear`.
fn junk_reply(state: &DashState, r: &JunkRequest) -> String {
    let cfg = state
        .cfg
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let report = crate::agents::junk::report(&cfg);
    match r.action {
        JunkAction::Plan => ServerFrame::JunkPlan {
            plan: crate::agents::junk_web::plan(&cfg, &report),
        },
        JunkAction::Apply => {
            let cleared = crate::agents::junk_web::apply(&cfg, &report, &r.paths);
            crate::model::forget_junk();
            ServerFrame::JunkCleared { cleared }
        }
    }
    .to_json()
}

/// `plugins.<id>.enabled` for a catalogue id (D23: Registry, not a second list).
fn allowlisted_plugin_enabled(cfg: &Config, key: &str) -> bool {
    let Some(rest) = key.strip_prefix("plugins.") else {
        return false;
    };
    let Some(id) = rest.strip_suffix(".enabled") else {
        return false;
    };
    if id.is_empty() || id.contains('.') {
        return false;
    }
    Registry::new(cfg)
        .manifests()
        .iter()
        .any(|(m, _)| m.id == id)
}

fn message_frame(text: &str) -> String {
    ServerFrame::Message { text: text.into() }.to_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(origin: Option<&str>, host: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("host", host.parse().unwrap());
        if let Some(o) = origin {
            h.insert("origin", o.parse().unwrap());
        }
        h
    }

    /// T193: same-origin loopback and header-less clients pass; a foreign page, an
    /// opaque `null` origin and a DNS-rebinding name (matching Origin and Host) do not.
    #[test]
    fn origin_gate_blocks_cross_site_and_rebinding() {
        for (origin, host) in [
            (None, "evil.example:4444"),
            (Some("http://127.0.0.1:4444"), "127.0.0.1:4444"),
            (Some("http://localhost:4444"), "localhost:4444"),
            (Some("http://[::1]:4444"), "[::1]:4444"),
            (Some("http://192.168.1.5:4444"), "192.168.1.5:4444"),
        ] {
            assert!(origin_allowed(&headers(origin, host)), "{origin:?} {host}");
        }
        for (origin, host) in [
            ("http://evil.example", "127.0.0.1:4444"),
            ("null", "127.0.0.1:4444"),
            ("http://evil.example:4444", "evil.example:4444"),
        ] {
            assert!(
                !origin_allowed(&headers(Some(origin), host)),
                "{origin} {host}"
            );
        }
    }

    /// The port is part of the origin: a page served by another local app
    /// (`http://localhost:3000`) must not open the dashboard socket on `:4444`, where it
    /// could read archived payloads through `expand` and toggle plugins through `set`.
    #[test]
    fn origin_gate_blocks_another_port_on_the_same_host() {
        for (origin, host) in [
            ("http://localhost:3000", "localhost:4444"),
            ("http://127.0.0.1:3000", "127.0.0.1:4444"),
            ("http://[::1]:3000", "[::1]:4444"),
            ("http://127.0.0.1", "127.0.0.1:4444"),
        ] {
            assert!(
                !origin_allowed(&headers(Some(origin), host)),
                "{origin} {host}"
            );
        }
        for (origin, host) in [
            ("http://127.0.0.1", "127.0.0.1"),
            ("http://127.0.0.1:80", "127.0.0.1"),
            ("https://localhost", "localhost:443"),
        ] {
            assert!(
                origin_allowed(&headers(Some(origin), host)),
                "{origin} {host}"
            );
        }
    }
}
