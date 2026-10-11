// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `src/proxy` — the local API proxy (plan P5). `rtok proxy` serves here.
//!
//! T5.1 scope: passthrough. Every request is forwarded byte-identical to
//! `proxy.upstream` (default `https://api.anthropic.com`; point it at
//! `http://127.0.0.1:8788` to chain behind another proxy during A/B). SSE responses
//! stream through unchanged — chunks are tee'd into a buffer *while* the client
//! receives them, never before (a spawned task does the bookkeeping afterwards).
//! `/v1/chat/completions` goes to `proxy.openai_upstream` (T11.2), adding
//! `stream_options.include_usage` to streaming requests that omit it so the final
//! chunk reports usage. `/v1/responses` goes there too (T11.3) and needs no shaping —
//! it reports usage on its final `response.completed` event unasked.
//!
//! Which of those rewrites a request gets depends on its lane (T385.2, `lane::Lane::policy`):
//! the agent lane keeps every global switch as it was, `batch` and `files` are never
//! rewritten, and `[proxy.lanes.<lane>]` opens the others one switch at a time.
//!
//! Bookkeeping per request (all fail-open, logged, never alter the response):
//! one `calls` row (`kind = api_request`, or `api_request:<lane>` off the agent lane —
//! T385.1; `surface = proxy`) with provider+model
//! upserted from the request body; `call_io` with request/response bytes (inline
//! under `core.call_io_inline_bytes`, else archived); a `tokens` row
//! (`source = provider`) with the four counters; and one `usage` row whose
//! `call_id` points at the `calls` row. Session id: `metadata.user_id`, else the
//! `x-rtok-session` header, else the sha256 of the request body.
//!
//! When `proxy.enabled` or `core.enabled` is false the listener still serves, but every
//! request is a plain reverse proxy (no record / compress / prepare). Only killing the
//! process stops HTTP — flipping those flags never shuts the listener down.
//!
//! Every forwarded request carries `x-rtok-proxied: <hops>` (T442). One that arrives
//! already marked is forwarded the plain way with a `warn` log line, so a chain of rtok
//! proxies shapes and records it once; at `MAX_HOPS` the proxy answers 508 instead.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::header::{ACCEPT_ENCODING, CONTENT_TYPE, RETRY_AFTER};
use axum::http::{HeaderMap, HeaderName, Request, Response, StatusCode};
use axum::response::Response as AxumResponse;
use axum::routing::get;
use futures_util::StreamExt;
use futures_util::stream::unfold;
use reqwest::Client;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::config::{Config, LanePolicy};
use crate::plugin::{Ctx, Runtime};
use crate::plugins::Registry;
use crate::store::Store;
use rtok_plugin_sdk::Measurement;
use wire::{API_ANTHROPIC, Wire, WireRequest, api_of, join_upstream};

pub mod anthropic;
pub mod batch_results;
pub mod cli;
mod flex;
mod gate;
pub mod gemini;
pub mod lane;
pub mod live;
mod noise;
pub use live::LiveCall;
pub mod openai_chat;
pub mod openai_responses;
mod route;
pub mod semantic_cache;
pub mod tools_rewrite;
pub mod wire;

/// Request bodies are JSON and bounded by the Anthropic/OpenAI API limits; cap the
/// in-memory read well above them. Also caps how much of a streamed response the tee
/// task buffers for recording (see `handle`) — one constant for both, since both exist
/// only to bound memory, not to reject legitimate traffic.
const MAX_BODY_BYTES: usize = 256 * 1024 * 1024;

/// TCP connect deadline; `proxy.timeout_s` bounds reads only.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How many rtok proxies a request has passed through (T442). A second pass must not
/// rewrite or record the request again, and a proxy whose upstream is itself must not
/// recurse forever, so every forwarded request carries the count.
pub const HOP_HEADER: &str = "x-rtok-proxied";

/// A chain this long is a loop, not a deployment: answer 508 instead of forwarding.
const MAX_HOPS: u32 = 8;

/// OpenAI's `/v1/batches` and `/v1/files` carry no wire, so the path alone would send them to
/// the Anthropic upstream. Anthropic has a Files API on the same path, but every Anthropic
/// request has to carry `anthropic-version`, which tells the two apart.
fn is_openai_batch_path(path: &str, headers: &HeaderMap) -> bool {
    let under = |root: &str| {
        path.strip_prefix(root)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    };
    (under("/v1/batches") || under("/v1/files")) && !headers.contains_key("anthropic-version")
}

/// The incoming hop count. Anything but a number still says another rtok saw the
/// request, so it counts as one hop rather than none.
fn hops_of(headers: &HeaderMap) -> u32 {
    headers.get(HOP_HEADER).map_or(0, |v| {
        v.to_str()
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(1)
    })
}

/// A *read* timeout, not `Client::timeout`: the latter is a deadline on the whole exchange,
/// body stream included, so a turn that streams for longer than the budget (extended
/// thinking, many tool calls) was cut mid-SSE with the client left without a `message_stop`.
/// The budget bounds one read instead.
fn build_client(timeout_s: u64) -> Result<Client> {
    // Connecting is not streaming: a black-holed upstream used to hold the client for the
    // whole read budget (10 minutes by default) before it saw a 502.
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(Duration::from_secs(timeout_s.max(1)))
        // T53.3: webpki roots instead of reqwest's platform verifier
        // (Security.framework costs ~1.3–1.5 ms of dyld time per hook
        // spawn); corporate CAs arrive via `SSL_CERT_FILE` (see `tls`).
        .use_preconfigured_tls(crate::tls::preconfigured()?)
        .build()
        .context("reqwest client")
}

/// Shared server state: the DB, the upstream client and the effective `[proxy]` settings.
pub struct ProxyState {
    pub store: Store,
    client: Client,
    /// Clients for the lanes that set their own `timeout_s` (T385.2); the rest use `client`.
    lane_clients: HashMap<lane::Lane, Client>,
    /// Lanes sent to their own `[proxy.lanes.<lane>] upstream` (T385.7).
    lane_upstreams: HashMap<lane::Lane, String>,
    /// Lanes with a `max_in_flight` cap (T385.7); the rest, `agent` always, go straight on.
    gates: HashMap<lane::Lane, gate::Gate>,
    upstream: String,
    /// Where the OpenAI wires go; Anthropic paths keep using `upstream` (D11).
    openai_upstream: String,
    /// Where the Gemini wire goes (T51.3).
    gemini_upstream: String,
    host_id: Option<i32>,
    inline_cap: usize,
    archive_dir: Option<PathBuf>,
    pub mode: String,
    /// `compress` mode (T5.3): every enabled plugin's `proxy_filter` runs on `/v1/messages`.
    registry: Registry,
    cfg: Config,
    cache: Mutex<semantic_cache::Cache>,
}

impl ProxyState {
    pub fn new(cfg: &Config) -> Result<Self> {
        let store = Store::open(&cfg.core.db_path)?;
        store.set_store_raw(cfg.core.store_raw);
        let client = build_client(cfg.proxy.timeout_s)?;
        let mut lane_clients = HashMap::new();
        for lane in lane::Lane::ALL {
            let policy = lane.policy(&cfg.proxy.lanes);
            let secs = flex::lane_timeout_s(policy.flex, policy.timeout_s, cfg.proxy.timeout_s);
            if secs > 0 && secs != cfg.proxy.timeout_s {
                lane_clients.insert(lane, build_client(secs)?);
            }
        }
        let mut lane_upstreams = HashMap::new();
        let mut gates = HashMap::new();
        for lane in lane::Lane::ALL {
            let policy = lane.policy(&cfg.proxy.lanes);
            let base = policy.upstream.trim().trim_end_matches('/');
            if !base.is_empty() {
                lane_upstreams.insert(lane, base.to_string());
            }
            if let Some(gate) = gate::Gate::new(policy.max_in_flight, policy.max_queued) {
                gates.insert(lane, gate);
            }
        }
        // Host agent: the `[hook] host` setting (T5.1 says `core.host`, which T12 removed —
        // see plan.md §6 amendment in the T5.1 commit). Unknown slugs fall back to `other` (6).
        let host_id = store.host_id(&cfg.hook.host)?.or(Some(6));
        Ok(Self {
            store,
            client,
            lane_clients,
            lane_upstreams,
            gates,
            upstream: cfg.proxy.upstream.trim_end_matches('/').to_string(),
            openai_upstream: cfg.proxy.openai_upstream.trim_end_matches('/').to_string(),
            gemini_upstream: cfg.proxy.gemini_upstream.trim_end_matches('/').to_string(),
            host_id,
            inline_cap: cfg.core.call_io_inline_bytes as usize,
            archive_dir: Some(cfg.core.archive_dir.clone()),
            mode: cfg.proxy.mode.clone(),
            registry: Registry::new(cfg),
            cfg: cfg.clone(),
            cache: Mutex::new(semantic_cache::Cache::new()),
        })
    }

    /// The client for `lane`: its own read timeout when `[proxy.lanes.<lane>] timeout_s` sets one.
    fn client_for(&self, lane: lane::Lane) -> &Client {
        self.lane_clients.get(&lane).unwrap_or(&self.client)
    }

    /// Requests on `lane` waiting for an upstream slot right now (T385.7); 0 when uncapped.
    pub fn queued(&self, lane: lane::Lane) -> usize {
        self.gates.get(&lane).map_or(0, gate::Gate::waiting)
    }

    /// The lane's own upstream when it names one, else the upstream owning `wire`. Paths
    /// this build has no wire for keep the Anthropic default, which is what they did before
    /// P11 (`/v1/responses` until T11.3), except OpenAI's Batch and Files paths (T385.12.1).
    fn upstream_for(
        &self,
        lane: lane::Lane,
        wire: Option<&'static dyn Wire>,
        path: &str,
        headers: &HeaderMap,
    ) -> &str {
        if let Some(base) = self.lane_upstreams.get(&lane) {
            return base;
        }
        match wire.map(Wire::provider) {
            Some("openai") => &self.openai_upstream,
            Some("gemini") => &self.gemini_upstream,
            None if is_openai_batch_path(path, headers) => &self.openai_upstream,
            _ => &self.upstream,
        }
    }

    /// Config kill-switches: any of `proxy.enabled`, `core.enabled` or `plugins.proxy.enabled`
    /// false → byte-forward only. The listener stays up; only process exit stops HTTP. Flags
    /// are read from the Config loaded at start (restart picks up file changes).
    ///
    /// `plugins.proxy.enabled` is the plugin's own switch, and the plugin *is* usage capture:
    /// leaving it out of this check made it inert, so an operator who turned it off to stop
    /// prompt bodies being written kept every `calls`, `call_io` and `usage` row.
    pub fn plain(&self) -> bool {
        !self.cfg.proxy.enabled || !self.cfg.core.enabled || !self.cfg.plugins.proxy.enabled
    }
}

/// The axum app: `/health` plus a fallback that forwards every other path upstream.
pub fn app(state: Arc<ProxyState>) -> Router {
    Router::new()
        .route("/health", get(cli::health))
        .route("/live", get(cli::live_calls))
        .fallback(proxy)
        .with_state(state)
}

/// `rtok proxy` (cli.rs): run until killed.
pub fn serve_blocking(cfg: Config) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("tokio runtime")?;
    rt.block_on(serve(&cfg))
}

/// Bind `bind:port` and serve. Tests bind their own ephemeral listener and call
/// [`app`] directly instead.
pub async fn serve(cfg: &Config) -> Result<()> {
    let state = Arc::new(ProxyState::new(cfg)?);
    // Background, own connection: housekeeping must neither delay the listener nor die on a
    // contended store (T75, T352).
    crate::store::spawn_retention(cfg, "proxy");
    // A plain thread, not a task: a flush is blocking SQLite plus a blocking `flock`, and on
    // this runtime it stalled whichever worker also served live requests.
    crate::otel::export::spawn_ticker(cfg);
    let addr = format!("{}:{}", cfg.proxy.bind, cfg.proxy.port);
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("bind {addr}"))?;
    axum::serve(listener, app(state))
        .await
        .context("proxy server")
}

/// Upstream chunks buffered between the tee task and the client stream before the tee
/// waits for the client to catch up.
const TEE_CHANNEL_CHUNKS: usize = 32;

/// Milliseconds in a second, for the ledger's `ms` columns.
const MS_PER_SEC: f64 = 1000.0;

/// Wall time since `start`, in the milliseconds the ledger records.
fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * MS_PER_SEC
}

/// The request's `model` field, when the body is JSON and names one.
fn request_model(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.get("model").and_then(Value::as_str).map(str::to_string))
}

async fn proxy(State(state): State<Arc<ProxyState>>, req: Request<Body>) -> AxumResponse {
    handle(state, req).await
}

async fn handle(state: Arc<ProxyState>, req: Request<Body>) -> AxumResponse {
    let start = Instant::now();
    let method = req.method().clone();
    let query = req.uri().query().map(str::to_string);
    let (parts, body) = req.into_parts();
    let mut headers = parts.headers.clone();
    // The lane marker is for rtok, not the provider: it is read here and never forwarded.
    // A path prefix is stripped the same way; with lanes off both pass through untouched.
    let (req_lane, path) = if state.cfg.proxy.lanes.enabled {
        let c = lane::classify(parts.uri.path(), &headers);
        let classified = (c.lane, c.path.to_string());
        headers.remove(lane::HEADER);
        classified
    } else {
        (lane::Lane::Agent, parts.uri.path().to_string())
    };
    let hops = hops_of(&headers);
    headers.remove(HOP_HEADER);
    if hops >= MAX_HOPS {
        let msg = format!(
            "{method} {path}: {hops} rtok proxy hops, refusing a loop (check proxy.upstream)"
        );
        log_off_worker(&state, "error", msg.clone());
        return error_response(StatusCode::LOOP_DETECTED, &msg);
    }
    if hops > 0 {
        log_off_worker(
            &state,
            "warn",
            format!("{method} {path}: already through {hops} rtok proxy hop(s), forwarding as is"),
        );
    }

    // The lane's in-flight cap (T385.7), taken before the body is read so a queued request
    // holds no body in memory. Plain mode is rtok switched off, which never turns a request
    // away. The slot rides with the response stream and frees when that stream ends.
    let slot = match state.gates.get(&req_lane) {
        Some(gate) if !state.plain() => match gate.enter().await {
            Some(slot) => Some(slot),
            None => {
                let msg = format!(
                    "{method} {path}: {} lane at max_in_flight with a full queue, answered 429",
                    req_lane.name()
                );
                log_off_worker(&state, "warn", msg.clone());
                return queue_full(&msg);
            }
        },
        _ => None,
    };

    let request_body = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
        Ok(b) => b,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &e.to_string()),
    };

    let wire = wire::for_path(&path);
    // Plain mode (`proxy.enabled` / `core.enabled` false): byte-identical forward, no
    // bookkeeping, compress, or request shaping. Listener stays up until process exit.
    // An earlier rtok hop already shaped and recorded this request (T442).
    let plain = state.plain() || hops > 0;
    let (request_body, recorded, context_armed, flex_retry) = if plain {
        (request_body, None, false, None)
    } else {
        // Request bookkeeping is CPU-bound (serde parse of up to `MAX_BODY_BYTES`,
        // tokenizer estimates, sync store writes) — run it off the tokio worker so a
        // large body never delays `/health` or other in-flight streams (T205). `Bytes`
        // clone is O(1) (a refcounted view, not a copy), so keeping `original_body`
        // around for the fail-open fallback costs nothing.
        let state_bg = state.clone();
        let path_bg = path.clone();
        let headers_bg = headers.clone();
        let original_body = request_body.clone();
        match tokio::task::spawn_blocking(move || {
            shape_request(
                &state_bg,
                wire,
                &path_bg,
                &headers_bg,
                req_lane,
                request_body,
            )
        })
        .await
        {
            Ok(shaped) => shaped,
            // The task panicked or was cancelled: fail open exactly like `record` /
            // `compress` / `prepare` already do on their own internal errors — forward
            // the original bytes unmodified rather than failing the request.
            Err(_join_err) => (original_body, None, false, None),
        }
    };

    let sc = &state.cfg.plugins.proxy.semantic_cache;
    // A lane that does not use the cache neither reads nor fills it (T385.2).
    let cache_lane = req_lane.policy(&state.cfg.proxy.lanes).semantic_cache;
    // Who asked (T323): part of the cache key on lookup and, below, on store.
    let caller = semantic_cache::caller_identity(&headers);
    if sc.enabled
        && cache_lane
        && !plain
        && let (Some(wire), Ok(body)) = (wire, serde_json::from_slice::<Value>(&request_body))
        && semantic_cache::eligible(&body, sc)
        && let Some(prompt) =
            semantic_cache::build_prompt(wire, &body, sc).map(|p| p.with_caller(&caller))
    {
        let cache_hit = state
            .cache
            .lock()
            .ok()
            .and_then(|guard| guard.lookup(&prompt, sc));
        if let Some(hit) = cache_hit {
            return cache_response(state, Some(wire), recorded, &request_body, start, &hit);
        }
    }

    let target = match join_upstream(
        state.upstream_for(req_lane, wire, &path, &headers),
        &path,
        query.as_deref(),
    ) {
        Ok(u) => u,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &e.to_string()),
    };

    let mut rb = state.client_for(req_lane).request(method.clone(), target);
    // The client's `accept-encoding` is not honoured by this build: reqwest is linked
    // without its decompression features, so a compressed body would reach the client
    // intact but decode to no `usage` row, no `tokens` row and lossy text in `call_io`.
    // Asking upstream for `identity` is what keeps the tee readable — and it has to be the
    // only value: `header` appends, so copying the client's `gzip, deflate, br` first made
    // upstream see both and reply gzipped.
    for (name, value) in headers.iter() {
        if !hop_by_hop(name.as_str()) && name != ACCEPT_ENCODING {
            rb = rb.header(name, value);
        }
    }
    rb = rb.header(ACCEPT_ENCODING, "identity");
    rb = rb.header(HOP_HEADER, (hops + 1).to_string());
    // The platform path needs its beta (T51.2) — unless the client already opted in,
    // in which case appending a duplicate value is pointless.
    if context_armed
        && !headers.get_all("anthropic-beta").iter().any(|v| {
            v.to_str()
                .is_ok_and(|s| s.contains(anthropic::CONTEXT_BETA))
        })
    {
        rb = rb.header("anthropic-beta", anthropic::CONTEXT_BETA);
    }
    let flex::Sent {
        result,
        body: request_body,
        retries,
    } = flex::send(rb, request_body, flex_retry.as_ref()).await;
    if retries > 0 {
        let r = recorded.as_ref();
        log(
            &state,
            r.map_or("?", |r| r.session.as_str()),
            r.map(|r| r.call_id),
            "warn",
            &format!("flex 429 on {method} {path}: {retries} retry(ies)"),
        );
    }
    let upstream = match result {
        Ok(r) => r,
        Err(e) => {
            if plain {
                let model = request_model(&request_body);
                live::push(live::LiveCall {
                    ts: crate::log::now() as i64,
                    method: method.to_string(),
                    path: path.clone(),
                    provider: wire.map(Wire::provider).map(str::to_string),
                    model,
                    status: StatusCode::BAD_GATEWAY.as_u16(),
                    request_bytes: request_body.len(),
                    response_bytes: 0,
                    ms: elapsed_ms(start),
                });
            } else {
                let session = recorded.as_ref().map_or("?", |r| r.session.as_str());
                let call_id = recorded.as_ref().map(|r| r.call_id);
                if let Some(id) = call_id {
                    let _ = state.store.set_call_ms(id, elapsed_ms(start));
                }
                // Every request owes one usage row (T5.1): with no response there
                // are no counters. The `api` comes from the wire, like `finish`.
                if let (Some(wire), Some(r)) = (wire, recorded.as_ref()) {
                    r.usage(&state, wire, None);
                }
                log(
                    &state,
                    session,
                    call_id,
                    "error",
                    &format!("upstream {method} {path}: {e}"),
                );
            }
            return error_response(
                StatusCode::BAD_GATEWAY,
                &format!("rtok proxy: upstream error: {e}"),
            );
        }
    };

    let content_type = upstream
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let status = upstream.status();
    let mut out_headers: Vec<(HeaderName, axum::http::HeaderValue)> = Vec::new();
    for (name, value) in upstream.headers().iter() {
        if !hop_by_hop(name.as_str()) {
            out_headers.push((name.clone(), value.clone()));
        }
    }

    // Tee the upstream body: forward each chunk to the client *and* buffer it for the
    // `call_io`/`usage` rows, which the spawned task writes after the stream ends.
    let status_code = status.as_u16();
    let method_s = method.to_string();
    let path_s = path.clone();
    let req_len = request_body.len();
    let model_live = request_model(&request_body);
    let provider_live = wire.map(Wire::provider).map(str::to_string);
    // Batch results are read only after they were forwarded (T385.4), and only when asked.
    let results = (state.cfg.proxy.batch.parse_results && status.is_success())
        .then(|| batch_results::source(req_lane, &method, &path))
        .flatten();
    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(TEE_CHANNEL_CHUNKS);
    let recorder = state.clone();
    let body_stream = upstream.bytes_stream();
    tokio::spawn(async move {
        let mut buf: Vec<u8> = Vec::new();
        // True response size, independent of `buf`: every chunk still reaches the client
        // via `tx` once `buf` stops growing, so this is the only accurate byte count past
        // the cap (an upstream streaming gigabytes must not grow `buf` without bound).
        let mut total_bytes: usize = 0;
        // False once the stream was cut short — client gone or upstream error — so a
        // partial body is never taken for a response worth caching (T323).
        let mut complete = true;
        let mut stream = body_stream;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    total_bytes += bytes.len();
                    if buf.len() < MAX_BODY_BYTES {
                        let room = MAX_BODY_BYTES - buf.len();
                        buf.extend_from_slice(&bytes[..bytes.len().min(room)]);
                    }
                    if tx.send(Ok(bytes)).await.is_err() {
                        complete = false;
                        break; // client went away; record what we have
                    }
                }
                Err(e) => {
                    complete = false;
                    let _ = tx.send(Err(io::Error::other(e))).await;
                    break;
                }
            }
        }
        drop(tx);
        // Upstream is done with this request; bookkeeping below needs no slot.
        drop(slot);
        // A body past the `buf` cap is incomplete as recorded too.
        let complete = complete && total_bytes == buf.len();
        if plain {
            live::push(live::LiveCall {
                ts: crate::log::now() as i64,
                method: method_s,
                path: path_s,
                provider: provider_live,
                model: model_live,
                status: status_code,
                request_bytes: req_len,
                response_bytes: total_bytes,
                ms: elapsed_ms(start),
            });
        } else {
            // `finish`'s store writes are synchronous (T205); move them off the tokio
            // worker too. The response has already been fully streamed to the client by
            // this point, so a panicked/cancelled task only drops bookkeeping, never the
            // response itself — same fail-open shape as the rest of this module.
            let _ = tokio::task::spawn_blocking(move || {
                finish(
                    &recorder,
                    &recorded,
                    start,
                    wire,
                    status_code,
                    content_type.as_deref(),
                    &request_body,
                    &buf,
                    total_bytes,
                    complete,
                    cache_lane.then_some(caller.as_str()),
                    results,
                );
            })
            .await;
        }
    });

    let client_stream = unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    });
    let response_body = Body::from_stream(client_stream);

    let mut response = Response::builder().status(status);
    for (name, value) in out_headers {
        response = response.header(name, value);
    }
    match response.body(response_body) {
        Ok(r) => r,
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// One non-plain request's bookkeeping: `record`, `compress` (in `compress` mode), then
/// provider request shaping (`prepare`/`context_edits`/`rewrite_tools`). Runs on a
/// blocking-pool thread (T205, called via `spawn_blocking`) so its synchronous JSON
/// parse and tokenizer work never pins a tokio worker (fail-open: a DB error logs and
/// the request still goes through, same as each helper already does on its own).
fn shape_request(
    state: &ProxyState,
    wire: Option<&'static dyn Wire>,
    path: &str,
    headers: &HeaderMap,
    lane: lane::Lane,
    request_body: Bytes,
) -> (Bytes, Option<Recorded>, bool, Option<flex::Retry>) {
    // Each rewrite below runs only when its global switch and this lane's switch are both on.
    let policy = lane.policy(&state.cfg.proxy.lanes);
    // Before `record`, so the ledger names the model that serves the call (T385.10).
    let route = lane
        .route(&state.cfg.proxy.routing)
        .and_then(|rule| route::apply(rule, &request_body));
    let request_body = route.as_ref().map_or(request_body, |r| r.body.clone());
    let parsed = serde_json::from_slice::<Value>(&request_body).ok();
    let recorded = record(
        state,
        wire,
        path,
        parsed.as_ref(),
        headers,
        Tag {
            kind: lane.kind(),
            provider: route.as_ref().and_then(|r| r.provider.as_deref()),
        },
        &request_body,
    );
    if let (Some(route), Some(r)) = (route.as_ref(), recorded.as_ref()) {
        record_route(state, r, lane, route);
    }
    // From here on `request_body` is what upstream sees (and what `call_io` records).
    let compressing = state.mode == "compress" && policy.compress;
    let request_body = if compressing {
        wire.map_or(request_body.clone(), |wire| {
            compress(
                state,
                wire,
                parsed,
                recorded.as_ref(),
                &policy,
                request_body,
            )
        })
    } else {
        request_body
    };
    // T432: terminal noise only matters where the proxy rewrites at all (`compress`); a
    // passthrough request keeps every byte the client sent.
    let request_body = match wire {
        Some(wire) if compressing => noise::strip(wire, request_body),
        _ => request_body,
    };
    // Provider request shaping runs in both modes (T11.2: OpenAI `stream_options`;
    // T51.2: Anthropic `context_management`).
    let request_body = match wire {
        Some(wire) if !lane.passes_through() => prepare(state, wire, request_body),
        _ => request_body,
    };
    let (request_body, context_armed) = match wire {
        Some(wire) if policy.context_management => context_edits(state, wire, request_body),
        _ => (request_body, false),
    };
    if context_armed && let Some(r) = recorded.as_ref() {
        record_context_path(state, r, &request_body);
    }
    let (request_body, tools_delta) = if policy.tools_rewrite {
        rewrite_tools(state, request_body)
    } else {
        (request_body, None)
    };
    if let (Some(delta), Some(r)) = (tools_delta, recorded.as_ref()) {
        record_tools_rewrite(state, r, delta);
    }
    // Last, so the tier is the only thing that differs from what the other rewrites left.
    let flex_cfg = &state.cfg.proxy.flex;
    let (request_body, flex_retry) = match wire {
        Some(wire) if policy.flex => match flex::apply(wire.provider(), &request_body, flex_cfg) {
            Some(flexed) => (flexed, flex::Retry::new(request_body, flex_cfg)),
            None => (request_body, None),
        },
        _ => (request_body, None),
    };
    (request_body, recorded, context_armed, flex_retry)
}

/// `compress` mode: run every enabled plugin's `proxy_filter` over the parsed body and
/// forward the re-serialised result. Fail open: no parse, no `calls` row, a store error, or
/// no change at all → the original bytes go through untouched.
/// Let the wire shape the outgoing request (T11.2). Fail open: an unparseable or
/// unchanged body is forwarded exactly as it arrived.
fn prepare(state: &ProxyState, wire: &'static dyn Wire, original: Bytes) -> Bytes {
    let Ok(mut body) = serde_json::from_slice::<Value>(&original) else {
        return original;
    };
    if !wire.prepare_request(&mut body, state.cfg.proxy.include_usage) {
        return original;
    }
    serde_json::to_vec(&body).map_or(original, Bytes::from)
}

/// The platform path (T51.2): on the Anthropic wire with `[proxy] context_management`,
/// arm server-side clearing of old tool uses. Returns the body to forward and whether it
/// was armed. Fail open like `prepare`: unparseable, unchanged, non-Anthropic, or
/// disabled bodies go through exactly as they arrived.
fn context_edits(state: &ProxyState, wire: &'static dyn Wire, original: Bytes) -> (Bytes, bool) {
    if api_of(wire) != API_ANTHROPIC {
        return (original, false);
    }
    let Ok(mut body) = serde_json::from_slice::<Value>(&original) else {
        return (original, false);
    };
    if !anthropic::apply_context_edits(&mut body, state.cfg.proxy.context_management) {
        return (original, false);
    }
    let bytes = serde_json::to_vec(&body).map_or(original, Bytes::from);
    (bytes, true)
}

/// This request takes the platform path: the `context_management` field was added, so
/// the clearing happens server-side. Record which path it took — a zero-delta row on the
/// `semantic_cache_hit` precedent. The field adds bytes and the platform's saving is not
/// locally observable, so no saving is claimed (D3).
fn record_context_path(state: &ProxyState, r: &Recorded, request_body: &[u8]) {
    let nbytes = request_body.len();
    let est = (nbytes / 4).max(1) as u32;
    let m = Measurement {
        plugin: "proxy",
        kind: "context_management",
        before_bytes: nbytes as u64,
        after_bytes: nbytes as u64,
        est_before: est,
        est_after: est,
        ref_id: None,
        call_id: Some(r.call_id),
    };
    if let Err(e) = state.store.insert_measurement(&r.session, &m) {
        log(
            state,
            &r.session,
            Some(r.call_id),
            "error",
            &format!("context path: {e:#}"),
        );
    }
}

/// T385.10: a routed call is marked by a `model_route` row (both sizes the final body's, so
/// no token saving is claimed — the dollars are the `usage` rows' at the served model's
/// price), and the log row keeps the model the client asked for.
fn record_route(state: &ProxyState, r: &Recorded, lane: lane::Lane, route: &route::Routed) {
    let nbytes = route.body.len();
    let est = (nbytes / 4).max(1) as u32;
    let m = Measurement {
        plugin: "proxy",
        kind: "model_route",
        before_bytes: nbytes as u64,
        after_bytes: nbytes as u64,
        est_before: est,
        est_after: est,
        ref_id: None,
        call_id: Some(r.call_id),
    };
    if let Err(e) = state.store.insert_measurement(&r.session, &m) {
        log(
            state,
            &r.session,
            Some(r.call_id),
            "error",
            &format!("model route: {e:#}"),
        );
    }
    log(
        state,
        &r.session,
        Some(r.call_id),
        "info",
        &format!(
            "model route on the {} lane: {} -> {}",
            lane.name(),
            route.from,
            route.to
        ),
    );
}

/// T59.5: truncate / drop `tools[]` when enabled. Fail open; unchanged bodies keep original bytes.
fn rewrite_tools(state: &ProxyState, original: Bytes) -> (Bytes, Option<tools_rewrite::Delta>) {
    if !state.cfg.proxy.tools_rewrite.enabled {
        return (original, None);
    }
    let Ok(mut body) = serde_json::from_slice::<Value>(&original) else {
        return (original, None);
    };
    let Some(delta) = tools_rewrite::rewrite(
        &mut body,
        &state.cfg.proxy.tools_rewrite,
        &state.cfg.estimator,
    ) else {
        return (original, None);
    };
    if !delta.changed {
        return (original, Some(delta));
    }
    match serde_json::to_vec(&body) {
        Ok(b) => (Bytes::from(b), Some(delta)),
        Err(_) => (original, None),
    }
}

fn record_tools_rewrite(state: &ProxyState, r: &Recorded, delta: tools_rewrite::Delta) {
    let m = Measurement {
        plugin: "proxy",
        kind: "tools_rewrite",
        before_bytes: delta.before_bytes,
        after_bytes: delta.after_bytes,
        est_before: delta.est_before,
        est_after: delta.est_after,
        ref_id: None,
        call_id: Some(r.call_id),
    };
    if let Err(e) = state.store.insert_measurement(&r.session, &m) {
        log(
            state,
            &r.session,
            Some(r.call_id),
            "error",
            &format!("tools rewrite: {e:#}"),
        );
    }
}

fn compress(
    state: &ProxyState,
    wire: &'static dyn Wire,
    parsed: Option<Value>,
    recorded: Option<&Recorded>,
    policy: &LanePolicy,
    original: Bytes,
) -> Bytes {
    let (Some(mut body), Some(r)) = (parsed, recorded) else {
        return original;
    };
    let mut cx = match Runtime::open(state.cfg.clone(), r.session.clone()) {
        Ok(cx) => cx,
        Err(e) => {
            log(
                state,
                &r.session,
                Some(r.call_id),
                "error",
                &format!("ctx: {e}"),
            );
            return original;
        }
    };
    cx.call_id = Some(r.call_id);
    // The platform path (T51.2): with context edits armed on the Anthropic wire the
    // platform clears old tool uses server-side, so `archive` stands down for those
    // turns — rewriting them first would only churn the cache and double-shrink.
    let platform_clears = api_of(wire) == API_ANTHROPIC
        && state.cfg.proxy.context_management
        && policy.context_management;
    let changed = {
        let mut changed = false;
        let mut request = WireRequest::new(wire, &mut body);
        for p in state.registry.enabled() {
            if platform_clears && p.manifest().id == "archive" {
                continue;
            }
            if !policy.toon && p.manifest().id == "toon" {
                continue;
            }
            for m in p.proxy_filter(&mut request, &Ctx::new(&cx)) {
                changed = true;
                if let Err(e) = cx.record(&m) {
                    log(
                        state,
                        &r.session,
                        Some(r.call_id),
                        "error",
                        &format!("measurement: {e}"),
                    );
                }
            }
        }
        changed
    };
    if !changed {
        return original;
    }
    serde_json::to_vec(&body)
        .map(Bytes::from)
        .unwrap_or(original)
}

/// What `record` writes beside the request itself: the `calls.kind` (the lane) and, for a
/// routed call, the provider slug the rule names instead of the wire's own (T385.10).
struct Tag<'a> {
    kind: &'a str,
    provider: Option<&'a str>,
}

/// One request's bookkeeping: session, host, provider+model, and the `calls` row.
fn record(
    state: &ProxyState,
    wire: Option<&'static dyn Wire>,
    path: &str,
    body: Option<&Value>,
    headers: &HeaderMap,
    tag: Tag,
    raw: &[u8],
) -> Option<Recorded> {
    let Tag {
        kind,
        provider: provider_override,
    } = tag;
    let session = session_for(wire, body, headers, raw);
    // The model slug: each wire knows where its own lives (Gemini's travels in the
    // path, not the body — T51.3). Unwired paths keep the old body lookup.
    let model = wire.and_then(|wire| wire.model(path, body)).or_else(|| {
        body.and_then(|v| v.get("model").and_then(Value::as_str))
            .map(str::to_string)
    });
    let provider = provider_override.or_else(|| {
        wire.map(Wire::provider).or_else(|| {
            matches!(path, "/v1/chat/completions" | "/v1/responses").then_some("openai")
        })
    });
    let result = (|| -> Result<Recorded> {
        state
            .store
            .upsert_session(&session, state.host_id, None, None, Some("proxy"))?;
        let (provider_id, model_id) = match (provider, model.as_deref()) {
            (Some(p), Some(m)) => {
                let (pid, mid) = state.store.upsert_model(p, m)?;
                (Some(pid), Some(mid))
            }
            _ => (None, None),
        };
        let call_id = state.store.insert_call(
            &session,
            "proxy",
            kind,
            state.host_id,
            provider_id,
            model_id,
            None,
            Some(path),
        )?;
        Ok(Recorded {
            session: session.clone(),
            model,
            call_id,
        })
    })();
    match result {
        Ok(r) => Some(r),
        Err(e) => {
            log(
                state,
                &session,
                None,
                "error",
                &format!("record {path}: {e}"),
            );
            None
        }
    }
}

/// One `usage` row per request (T5.1 four counters) — `finish`, `cache_response` and
/// the upstream-error arm share it. `Some` also writes the provider `tokens` row;
/// `None` (error, non-2xx, usage-less body, unparseable cache hit) is an all-zero row
/// with no `tokens` row. Best-effort: a failure logs, the response is untouched.
fn record_usage(
    state: &ProxyState,
    session: &str,
    model: Option<&str>,
    api: &str,
    counters: Option<(i64, wire::Usage)>,
    call_id: i32,
) {
    let (total, usage) = counters.unwrap_or_default();
    if let Err(e) = state.store.insert_usage(
        session,
        model,
        api,
        usage.input,
        usage.cache_create,
        usage.cache_read,
        usage.output,
        call_id,
    ) {
        log(
            state,
            session,
            Some(call_id),
            "error",
            &format!("usage: {e:#}"),
        );
    }
    if counters.is_some()
        && let Err(e) = state.store.insert_provider_tokens(
            call_id,
            total,
            usage.input,
            usage.cache_create,
            usage.cache_read,
            usage.output,
        )
    {
        log(
            state,
            session,
            Some(call_id),
            "error",
            &format!("tokens: {e:#}"),
        );
    }
}

/// After the body was fully forwarded: `calls.ms`, `call_io`, the semantic cache, then
/// `usage` + provider `tokens` when the response carried a usage block. All best-effort.
///
/// `response_total_bytes` is the true response size; `response_body` may be a shorter,
/// capped buffer (see `handle`'s tee task and `MAX_BODY_BYTES`) — a truncated buffer means
/// `call_io` and usage parsing only see the retained prefix, never that they panic on it.
///
/// `complete` is whether the body arrived whole (T323): a cut or capped body is recorded but
/// never cached. `caller` is the [`semantic_cache::caller_identity`] the lookup used; `None`
/// when the request's lane does not use the cache (T385.2).
#[allow(clippy::too_many_arguments)]
fn finish(
    state: &ProxyState,
    recorded: &Option<Recorded>,
    start: Instant,
    wire: Option<&'static dyn Wire>,
    status_code: u16,
    content_type: Option<&str>,
    request_body: &[u8],
    response_body: &[u8],
    response_total_bytes: usize,
    complete: bool,
    caller: Option<&str>,
    results: Option<batch_results::Source>,
) {
    let Some(r) = recorded else { return };
    let session = r.session.clone();
    let log_err = |what: &str, e: anyhow::Error| {
        log(
            state,
            &session,
            Some(r.call_id),
            "error",
            &format!("{what}: {e:#}"),
        );
    };
    if let Err(e) = state.store.set_call_ms(r.call_id, elapsed_ms(start)) {
        log_err("set_call_ms", e.into());
    }
    if wire.is_some()
        && let Some(tier) = wire::service_tier_from_response(content_type, response_body)
        && let Err(e) = state.store.set_call_service_tier(r.call_id, &tier)
    {
        log_err("service_tier", e.into());
    }
    if response_total_bytes > response_body.len() {
        // The true size, since call_io's recorded response_bytes reflects only what was
        // retained under MAX_BODY_BYTES — surface the gap instead of under-reporting it.
        log(
            state,
            &session,
            Some(r.call_id),
            "error",
            &format!(
                "response body truncated for recording: {response_total_bytes} bytes \
                 received, {} retained (MAX_BODY_BYTES); call_io and usage reflect only \
                 the retained bytes",
                response_body.len()
            ),
        );
    }
    if let Err(e) = state.store.insert_call_io(
        r.call_id,
        Some(request_body),
        Some(response_body),
        state.inline_cap,
        state.archive_dir.as_deref(),
    ) {
        log_err("call_io", e.into());
    }
    // Fill the cache before the usage rows: a visible usage row then implies a warm cache.
    let sc = &state.cfg.plugins.proxy.semantic_cache;
    if sc.enabled
        && let Some(caller) = caller
        && let (Some(wire), Ok(body)) = (wire, serde_json::from_slice::<Value>(request_body))
        && semantic_cache::eligible(&body, sc)
        && complete
        && let Some(prompt) =
            semantic_cache::build_prompt(wire, &body, sc).map(|p| p.with_caller(caller))
        && let Ok(mut guard) = state.cache.lock()
    {
        guard.store(&prompt, sc, response_body, content_type, status_code);
    }
    match wire.and_then(|wire| {
        wire::usage_from_response(wire, content_type, response_body).map(|u| (wire, u))
    }) {
        Some((wire, usage)) => r.usage(state, wire, Some((wire.provider_total(usage), usage))),
        // A wire that carried no counters (non-2xx, usage-less 200) still owes its one
        // usage row (T5.1). Endpoints this build has no wire for (e.g. /v1/models)
        // never carry usage — logging there on every request would be noise.
        None if wire.is_some() => {
            if let Some(wire) = wire {
                r.usage(state, wire, None);
            }
            log(
                state,
                &session,
                Some(r.call_id),
                "info",
                "no usage in upstream response",
            );
        }
        None => {
            if let Some(source) = results
                && let Err(e) = batch_results::record(
                    &state.store,
                    &session,
                    r.call_id,
                    r.model.as_deref(),
                    source,
                    response_body,
                )
            {
                log_err("batch results", e);
            }
        }
    }
}

/// Semantic-cache hit: the same bookkeeping as `finish` minus the upstream round
/// trip — the measurement, `calls.ms`, `call_io` (with the request bytes: the request
/// is what the cache key came from), and the `usage` + provider `tokens` rows decoded
/// from the cached body. A cached body with no parseable usage still owes its one
/// usage row (counters unknown → zeros, no `tokens` row).
fn cache_response(
    state: Arc<ProxyState>,
    wire: Option<&'static dyn Wire>,
    recorded: Option<Recorded>,
    request_body: &[u8],
    start: Instant,
    hit: &semantic_cache::CacheHit,
) -> AxumResponse {
    let nbytes = hit.response.len();
    if let Some(r) = &recorded {
        let m = semantic_cache::measurement(hit, Some(r.call_id), nbytes);
        let session = r.session.clone();
        let call_id = r.call_id;
        let inline_cap = state.inline_cap;
        let archive_dir = state.archive_dir.clone();
        if let Err(e) = state.store.insert_measurement(&session, &m) {
            log(
                state.as_ref(),
                &session,
                Some(call_id),
                "error",
                &format!("cache hit: {e}"),
            );
        }
        let _ = state.store.set_call_ms(call_id, elapsed_ms(start));
        let _ = state.store.insert_call_io(
            call_id,
            Some(request_body),
            Some(hit.response.as_ref()),
            inline_cap,
            archive_dir.as_deref(),
        );
        // Every request owes one usage row (T5.1): decode the cached body's counters
        // through the wire, exactly like `finish` — zeros when unparseable.
        if let Some(wire) = wire {
            let counters =
                wire::usage_from_response(wire, hit.content_type.as_deref(), hit.response.as_ref())
                    .map(|usage| (wire.provider_total(usage), usage));
            r.usage(&state, wire, counters);
        }
    }
    let mut response = Response::builder().status(hit.status);
    if let Some(ct) = &hit.content_type {
        response = response.header(CONTENT_TYPE, ct);
    }
    response
        .body(Body::from(hit.response.clone()))
        .unwrap_or_else(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))
}

/// The proxy's log lines go through the one funnel (`log::record`), so the file line and
/// the `logs` row come out of the same call as the plugin path's — one writer, not two.
fn log(state: &ProxyState, session: &str, call_id: Option<i32>, level: &str, message: &str) {
    crate::log::record(
        &state.cfg,
        &state.store,
        Some(session),
        call_id,
        level,
        "module",
        "proxy",
        message,
    );
}

/// [`log`] from the async handler: its store write is synchronous, so it runs on the
/// blocking pool and the request does not wait for it.
fn log_off_worker(state: &Arc<ProxyState>, level: &'static str, message: String) {
    let state = Arc::clone(state);
    tokio::task::spawn_blocking(move || log(&state, "?", None, level, &message));
}

struct Recorded {
    session: String,
    model: Option<String>,
    call_id: i32,
}

impl Recorded {
    /// This request's `usage` row through the shared insert: `Some` counters also
    /// write the provider `tokens` row, `None` is the all-zero row.
    fn usage(&self, state: &ProxyState, wire: &dyn Wire, counters: Option<(i64, wire::Usage)>) {
        record_usage(
            state,
            &self.session,
            self.model.as_deref(),
            api_of(wire),
            counters,
            self.call_id,
        );
    }
}

fn session_for(
    wire: Option<&dyn Wire>,
    body: Option<&Value>,
    headers: &HeaderMap,
    raw: &[u8],
) -> String {
    // The wire owns the body's session field (`metadata.user_id`, OpenAI `user`).
    if let Some(session) = wire.and_then(|wire| body.and_then(|body| wire.session_id(body))) {
        return session.to_string();
    }
    for name in ["x-rtok-session", "x-session-id"] {
        if let Some(v) = headers
            .get(HeaderName::from_static(name))
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty())
        {
            return v.to_string();
        }
    }
    crate::store::hex_sha256(raw)
}

/// Headers that must not be forwarded (HTTP/1.1 hop-by-hop + framing). `content-length`
/// is dropped because the proxy re-chunks; the body bytes themselves are untouched.
fn hop_by_hop(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "content-length"
            | "host"
    )
}

/// A lane at its in-flight cap with a full queue (T385.7). `429` + `Retry-After` rather than
/// `503`: the provider SDKs already back off and retry on it, and it says "you, slow down",
/// which is what a lane over its own cap is.
fn queue_full(message: &str) -> AxumResponse {
    let mut response = error_response(StatusCode::TOO_MANY_REQUESTS, message);
    response
        .headers_mut()
        .insert(RETRY_AFTER, gate::RETRY_AFTER_S.into());
    response
}

fn error_response(status: StatusCode, message: &str) -> AxumResponse {
    let body = format!(
        r#"{{"type":"error","error":{{"type":"{}","message":{}}}}}"#,
        match status {
            StatusCode::BAD_GATEWAY => "upstream_error",
            StatusCode::TOO_MANY_REQUESTS => "rate_limit_error",
            _ => "invalid_request_error",
        },
        serde_json::to_string(message).unwrap_or_else(|_| "\"proxy error\"".to_string())
    );
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("static error response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("/v1/batches", false, true)]
    #[case("/v1/batches/batch_1/cancel", false, true)]
    #[case("/v1/files/file-1/content", false, true)]
    // Anthropic's Files API shares the path and is told apart by its mandatory header.
    #[case("/v1/files", true, false)]
    #[case("/v1/messages/batches", false, false)]
    #[case("/v1/batchesx", false, false)]
    fn openai_batch_paths_are_told_from_anthropic_ones(
        #[case] path: &str,
        #[case] anthropic: bool,
        #[case] openai: bool,
    ) {
        let mut headers = HeaderMap::new();
        if anthropic {
            headers.insert("anthropic-version", "2023-06-01".parse().unwrap());
        }
        assert_eq!(is_openai_batch_path(path, &headers), openai);
    }

    /// The proxy's rows come out of the same funnel as the plugin's, shaped exactly as
    /// `insert_log` left them (T24.1). `logs.call_id` is a foreign key, so the session and
    /// the `calls` row come first — the order `record` uses on every request.
    #[test]
    fn proxy_log_rows_keep_their_shape_through_the_funnel() {
        let dir = std::env::temp_dir().join(format!("rtok-proxy-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = crate::testutil::config_in(&dir);
        let state = ProxyState::new(&cfg).expect("proxy state");
        state
            .store
            .upsert_session("sess", None, None, None, None)
            .unwrap();
        let call = state
            .store
            .insert_call("sess", "proxy", "api_request", None, None, None, None, None)
            .unwrap();
        log(&state, "sess", Some(call), "error", "boom");
        let rows = state.store.logs_after(0, 10).unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        let r = &rows[0];
        assert_eq!(
            (r.level.as_str(), r.source.as_str(), r.name.as_str()),
            ("error", "module", "proxy")
        );
        assert_eq!(r.message, "boom");
        assert_eq!(r.session.as_deref(), Some("sess"));
        assert_eq!(r.call_id, Some(call));
        assert_eq!(r.plugin, None);
        drop(state);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[rstest]
    fn retention_runs_on_proxy_session_start() {
        let dir = std::env::temp_dir().join(format!("rtok-proxy-retain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = crate::testutil::config_in(&dir);
        cfg.core.retain_calls_days = 1;
        {
            let store = Store::open(&cfg.core.db_path).unwrap();
            store
                .upsert_session("sess", Some(1), None, None, Some("proxy"))
                .unwrap();
            let call = store
                .insert_call(
                    "sess",
                    "proxy",
                    "api_request",
                    Some(1),
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
            let body = vec![b'y'; 70 * 1024];
            store
                .insert_call_io(
                    call,
                    Some(&body),
                    None,
                    64 * 1024,
                    Some(&cfg.core.archive_dir),
                )
                .unwrap();
            store.set_call_ts(call, 0).unwrap();
        }
        let state = ProxyState::new(&cfg).expect("proxy state");
        state
            .store
            .run_retention(cfg.core.retain_calls_days, cfg.core.retain_hook_bodies_days)
            .unwrap();
        assert_eq!(state.store.count_calls().unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn semantic_cache_disabled_proxy_bytes_identical() {
        use httpmock::prelude::*;

        let server = MockServer::start();
        let body = r#"{"type":"message","usage":{"input_tokens":1,"output_tokens":2}}"#;
        let mock = server.mock(|when, then| {
            when.method(POST).path("/v1/messages");
            then.status(200)
                .header("content-type", "application/json")
                .body(body);
        });
        let dir = std::env::temp_dir().join(format!("rtok-proxy-sc-off-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = Config::load_from(&dir).expect("config");
        cfg.proxy.upstream = server.base_url();
        cfg.plugins.proxy.semantic_cache.enabled = false;
        let state = Arc::new(ProxyState::new(&cfg).expect("proxy state"));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let task = tokio::spawn(axum::serve(listener, app(state)).into_future());
        let req = r#"{"model":"claude-test","messages":[{"role":"user","content":"hi"}]}"#;
        let client = reqwest::Client::new();
        for _ in 0..2 {
            let resp = client
                .post(format!("http://{addr}/v1/messages"))
                .header("content-type", "application/json")
                .body(req)
                .send()
                .await
                .expect("request");
            assert_eq!(resp.bytes().await.expect("body"), body.as_bytes());
        }
        mock.assert_calls(2);
        task.abort();
    }

    #[tokio::test]
    async fn semantic_cache_enabled_direct_hit_skips_upstream() {
        use httpmock::prelude::*;

        let server = MockServer::start();
        let body = r#"{"type":"message","usage":{"input_tokens":1,"output_tokens":2}}"#;
        let mock = server.mock(|when, then| {
            when.method(POST).path("/v1/messages");
            then.status(200)
                .header("content-type", "application/json")
                .body(body);
        });
        let dir = std::env::temp_dir().join(format!("rtok-proxy-sc-on-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = Config::load_from(&dir).expect("config");
        cfg.proxy.upstream = server.base_url();
        cfg.plugins.proxy.semantic_cache.enabled = true;
        let state = Arc::new(ProxyState::new(&cfg).expect("proxy state"));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let task = tokio::spawn(axum::serve(listener, app(state.clone())).into_future());
        let req = r#"{"model":"claude-test","messages":[{"role":"user","content":"hi"}]}"#;
        let client = reqwest::Client::new();
        let url = format!("http://{addr}/v1/messages");
        let first = client
            .post(&url)
            .header("content-type", "application/json")
            .body(req)
            .send()
            .await
            .expect("request");
        assert_eq!(first.bytes().await.expect("body"), body.as_bytes());
        tokio::time::sleep(Duration::from_millis(50)).await;
        let second = client
            .post(&url)
            .header("content-type", "application/json")
            .body(req)
            .send()
            .await
            .expect("request");
        assert_eq!(second.bytes().await.expect("body"), body.as_bytes());
        mock.assert_calls(1);
        let n = state
            .store
            .measurement_count("proxy")
            .expect("measurements");
        assert_eq!(n, 1);
        // T45.2: the cache hit must also write a usage row.
        let sessions = state.store.usage_sessions().expect("usage sessions");
        assert!(
            !sessions.is_empty(),
            "cache hit must write a usage row (T5.1)"
        );
        task.abort();
    }

    /// T432: the upstream body carries no escapes, control or zero-width characters from a
    /// tool result, keeps its whitespace, and is the same bytes on the next turn.
    #[tokio::test]
    async fn compress_mode_strips_terminal_noise_byte_stably() {
        use httpmock::prelude::*;

        let noisy = "\u{1b}[31mFAIL\u{1b}[0m a\u{200b}b\u{7}  \r\n";
        let request = serde_json::json!({"model": "claude-test", "messages": [
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": noisy}
            ]}
        ]});
        let mut want = request.clone();
        want["messages"][0]["content"][0]["content"] = "FAIL ab  \r\n".into();
        let want = serde_json::to_string(&want).unwrap();

        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/v1/messages").body(&want);
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"type":"message","usage":{"input_tokens":1,"output_tokens":2}}"#);
        });
        let dir = std::env::temp_dir().join(format!("rtok-proxy-noise-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = Config::load_from(&dir).expect("config");
        cfg.proxy.upstream = server.base_url();
        cfg.proxy.mode = "compress".to_string();
        let state = Arc::new(ProxyState::new(&cfg).expect("proxy state"));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let task = tokio::spawn(axum::serve(listener, app(state)).into_future());
        let client = reqwest::Client::new();
        for _ in 0..2 {
            let resp = client
                .post(format!("http://{addr}/v1/messages"))
                .header("content-type", "application/json")
                .body(request.to_string())
                .send()
                .await
                .expect("request");
            assert_eq!(resp.status(), reqwest::StatusCode::OK);
        }
        mock.assert_calls(2);
        task.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn upstream_base_with_trailing_path_forwards_to_target() {
        use httpmock::prelude::*;

        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/prefix/v1/messages");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"type":"message","usage":{"input_tokens":1,"output_tokens":2}}"#);
        });
        let dir = std::env::temp_dir().join(format!("rtok-proxy-prefix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = Config::load_from(&dir).expect("config");
        cfg.proxy.upstream = format!("{}/prefix", server.base_url());
        cfg.proxy.mode = "passthrough".to_string();
        let state = Arc::new(ProxyState::new(&cfg).expect("proxy state"));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let task = tokio::spawn(axum::serve(listener, app(state.clone())).into_future());

        let resp = reqwest::Client::new()
            .post(format!("http://{addr}/v1/messages"))
            .header("content-type", "application/json")
            .body(r#"{"model":"test"}"#)
            .send()
            .await
            .expect("request");
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        mock.assert();
        task.abort();
    }

    /// The proxy in `compress` mode in front of `upstream`, listening on a free port.
    async fn compress_proxy(
        upstream: String,
        tag: &str,
    ) -> (
        String,
        Arc<ProxyState>,
        tokio::task::JoinHandle<io::Result<()>>,
    ) {
        let dir = std::env::temp_dir().join(format!("rtok-proxy-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = Config::load_from(&dir).expect("config");
        cfg.proxy.upstream = upstream;
        cfg.proxy.mode = "compress".to_string();
        let state = Arc::new(ProxyState::new(&cfg).expect("proxy state"));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let task = tokio::spawn(axum::serve(listener, app(state.clone())).into_future());
        (addr, state, task)
    }

    /// T442: an unmarked request is shaped and recorded and leaves marked as one hop; a
    /// marked one (a junk value counts as one hop) reaches upstream byte-identical, one hop
    /// further, with no `calls` row and a `warn` line.
    #[rstest]
    #[case::first_hop(None, "1", false)]
    #[case::second_hop(Some("2"), "3", true)]
    #[case::junk_value(Some("yes"), "2", true)]
    #[tokio::test]
    async fn hop_marker_counts_and_skips_shaping_on_a_later_pass(
        #[case] incoming: Option<&str>,
        #[case] outgoing: &str,
        #[case] passthrough: bool,
    ) {
        use httpmock::prelude::*;

        let noisy = serde_json::json!({"model": "claude-test", "messages": [
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": "\u{1b}[31mFAIL\u{1b}[0m"}
            ]}
        ]})
        .to_string();
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            let when = when
                .method(POST)
                .path("/v1/messages")
                .header(HOP_HEADER, outgoing);
            let _when = if passthrough {
                when.body(&noisy)
            } else {
                when.body_excludes("\u{1b}")
            };
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"type":"message","usage":{"input_tokens":1,"output_tokens":2}}"#);
        });
        let (addr, state, task) =
            compress_proxy(server.base_url(), &format!("hop-{outgoing}-{passthrough}")).await;
        let mut req = reqwest::Client::new()
            .post(format!("http://{addr}/v1/messages"))
            .header("content-type", "application/json")
            .body(noisy.clone());
        if let Some(v) = incoming {
            req = req.header(HOP_HEADER, v);
        }
        let resp = req.send().await.expect("request");
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        let _ = resp.bytes().await;
        mock.assert();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(state.store.count_calls().unwrap() == 0, passthrough);
        let warned = state
            .store
            .logs_after(0, 10)
            .unwrap()
            .iter()
            .any(|r| r.level == "warn" && r.message.contains("rtok proxy hop"));
        assert_eq!(warned, passthrough);
        task.abort();
    }

    /// T442: a request already `MAX_HOPS` deep is a loop — 508, never forwarded, an
    /// `error` line naming the setting to check.
    #[tokio::test]
    async fn hop_marker_at_the_cap_answers_loop_detected() {
        use httpmock::prelude::*;

        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.any_request();
            then.status(200);
        });
        let (addr, state, task) = compress_proxy(server.base_url(), "hop-cap").await;
        let resp = reqwest::Client::new()
            .post(format!("http://{addr}/v1/messages"))
            .header("content-type", "application/json")
            .header(HOP_HEADER, MAX_HOPS.to_string())
            .body(r#"{"model":"claude-test"}"#)
            .send()
            .await
            .expect("request");
        assert_eq!(resp.status(), reqwest::StatusCode::LOOP_DETECTED);
        mock.assert_calls(0);
        // The log row is written off the response path; a fixed sleep lost that race on a
        // loaded CI runner, so wait for it with a generous deadline instead.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !state
            .store
            .logs_after(0, 10)
            .unwrap()
            .iter()
            .any(|r| r.level == "error" && r.message.contains("proxy.upstream"))
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "no proxy.upstream error row"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        task.abort();
    }
}
