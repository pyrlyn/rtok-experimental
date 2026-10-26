// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Shared proxy-under-test harness (moved out of `tests/proxy.rs` T5.1 so `proxy_bench.rs`
//! can spin up the same real proxy against a fake upstream without duplicating it).

use std::sync::{Arc, Mutex};

use httpmock::HttpMockRequest;
use httpmock::prelude::*;
use rtok::config::Config;
use rtok::proxy::{ProxyState, app};

pub type Server = (
    String,
    Arc<ProxyState>,
    tokio::task::JoinHandle<std::io::Result<()>>,
);

/// One proxy on a fresh store and a throwaway config dir; `tune` points it at the
/// mock upstream and sets whatever else the case needs (mode, plugin flags).
pub async fn proxy_server(dir_tag: &str, tune: impl FnOnce(&mut Config)) -> Server {
    let dir = std::env::temp_dir().join(format!("rtok-proxy-{dir_tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cfg = Config::load_from(&dir).expect("config");
    // These cases assert `archive`'s own pointers; `compress` (on by default since T127)
    // would summarise them. Its path is covered in `plugins_e2e`; `tune` can turn it on.
    cfg.plugins.compress.enabled = false;
    tune(&mut cfg);
    let state = Arc::new(ProxyState::new(&cfg).expect("proxy state"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr").to_string();
    let task = tokio::spawn(axum::serve(listener, app(state.clone())).into_future());
    (addr, state, task)
}

/// The same throwaway dir `proxy_server` derived from `dir_tag`, for a caller that needs to
/// reopen it later (e.g. an `expand` runtime pointed at the same store, T55.11-style).
pub fn proxy_home_dir(dir_tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("rtok-proxy-{dir_tag}-{}", std::process::id()))
}

/// One request as upstream saw it: the body and whether it carried `anthropic-beta`.
pub type Seen = (Vec<u8>, bool);

/// An upstream that answers `{}` and keeps every request body and its `anthropic-beta` header.
pub struct Sink {
    server: MockServer,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Sink {
    pub fn new() -> Self {
        let server = MockServer::start();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        server.mock(move |when, then| {
            when.is_true(move |req: &HttpMockRequest| {
                let beta = req.headers().get("anthropic-beta").is_some();
                sink.lock().expect("sink").push((req.body_vec(), beta));
                true
            });
            then.status(200)
                .header("content-type", "application/json")
                .body("{}");
        });
        Self { server, seen }
    }

    pub fn base(&self) -> String {
        self.server.base_url()
    }

    /// Every request so far, oldest first.
    pub fn all(&self) -> Vec<Seen> {
        self.seen.lock().expect("sink").clone()
    }

    pub fn last(&self) -> Seen {
        self.seen
            .lock()
            .expect("sink")
            .last()
            .cloned()
            .expect("a request reached upstream")
    }
}
