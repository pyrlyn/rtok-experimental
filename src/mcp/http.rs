// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok mcp --http` (T401): the stdio server's tools over MCP Streamable HTTP, for clients
//! that only reach remote servers (the Grok API calls MCP from xAI's own machines,
//! `docs/research/grok-cloud-mcp.md`).
//!
//! rmcp's `StreamableHttpService` owns the wire: POST/GET semantics, `MCP-Protocol-Version`,
//! the Host check against DNS rebinding and the Origin check
//! (https://modelcontextprotocol.io/specification/2025-11-25/basic/transports). `Http` only
//! hands `tools/list` and `tools/call` to [`Server`], so an HTTP call runs the same allow-list,
//! required-field gate, dispatch and `record` as a stdio one, and writes the same rows.

use std::borrow::Cow;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ListToolsResult, PaginatedRequestParams,
    ProtocolVersion, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::Value;
use subtle::ConstantTimeEq;

use super::{SUPPORTED_PROTOCOL_VERSIONS, Server, server_info};
use crate::config::Config;

/// The one MCP endpoint path the spec asks for.
pub const PATH: &str = "/mcp";

/// A shorter token is guessable by anyone who can reach the tunnel URL.
const MIN_TOKEN_CHARS: usize = 16;

/// Tools that act as "the agent this process serves" (`Server::agent`). One HTTP server answers
/// every client that holds the token and cannot tell whose session a call comes from, so these
/// would act as whoever started the server; they are never served here, whatever `http_tools` says.
const AGENT_BOUND: &[&str] = &[
    "whoami",
    "worktree_add",
    "worktree_remove",
    "worktree_finish",
    "worktree_adopt",
    "agent_send",
    "agent_inbox",
    "agent_status_set",
];

/// Hosts a loopback client sends; anything else must come from `[mcp] public_url`.
const LOOPBACK_HOSTS: &[&str] = &["localhost", "127.0.0.1", "::1"];

#[derive(Clone)]
struct Http(Arc<Server>);

impl ServerHandler for Http {
    fn get_info(&self) -> ServerConfig {
        let latest = SUPPORTED_PROTOCOL_VERSIONS.last().cloned();
        server_info(latest.unwrap_or(ProtocolVersion::LATEST))
    }

    // rmcp would otherwise agree to every revision it knows, including ones `Server` was
    // never tested against; this keeps HTTP and stdio on one list.
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(SUPPORTED_PROTOCOL_VERSIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.0.tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let server = self.0.clone();
        let name = request.name.into_owned();
        let args = request.arguments.map_or(Value::Null, Value::Object);
        // Tools read files and SQLite synchronously; off the async workers so one slow call
        // does not stall the listener.
        tokio::task::spawn_blocking(move || server.call_tool(&name, &args))
            .await
            .map(CallToolResponse::from)
            .map_err(|e| ErrorData::internal_error(format!("tool task failed: {e}"), None))
    }
}

/// The stdio server's tool set minus [`AGENT_BOUND`]; a dropped name is refused like any tool
/// the allow-list left out.
fn http_server(cfg: &Config) -> Result<Server> {
    let mut server = Server::new(cfg)?;
    // A reused PID must not revive read-cache pointers from a previous HTTP process.
    server.cx.session = format!("mcp-http-{}", uuid::Uuid::new_v4());
    server.cx.restrict_archives();
    server.listed.retain(|t| !AGENT_BOUND.contains(&t.def.name));
    Ok(server)
}

/// `rtok mcp --http <addr>`: bind, then serve until the process is killed.
pub fn serve_blocking(cfg: &Config, addr: &str) -> Result<()> {
    let addr: SocketAddr = addr
        .parse()
        .with_context(|| format!("rtok mcp --http: `{addr}` is not IP:PORT"))?;
    let token = bearer_token(&cfg.mcp.token)?;
    let public = public_url(&cfg.mcp.public_url)?;
    let cwd = std::env::current_dir().context("rtok mcp --http: current directory")?;
    exposure_guard(&cwd, std::env::home_dir().as_deref(), addr, public.as_ref())?;
    // Over HTTP the caller is remote: never hand it `.env`, keys or `.git/config`.
    #[cfg(feature = "read")]
    crate::plugins::read::roots::deny_secrets();
    let mut cfg = cfg.clone();
    // The HTTP surface gets its own allow-list through the same filter as stdio's `tools`.
    cfg.mcp.tools = cfg.mcp.http_tools.clone();
    let server = Arc::new(http_server(&cfg)?);
    crate::store::spawn_retention(&cfg, "mcp");
    crate::otel::export::spawn_ticker(&cfg);
    let app = router(server, token, transport_config(addr, public.as_ref()));
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("tokio runtime")?;
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .with_context(|| format!("bind {addr}"))?;
        let local = listener.local_addr().unwrap_or(addr);
        // The token is never printed: this line lands in terminals and logs.
        eprintln!("rtok mcp: serving http://{local}{PATH}");
        crate::log::append(
            &cfg,
            "info",
            "mcp",
            "http",
            &format!("http://{local}{PATH}"),
        );
        axum::serve(listener, app).await.context("rtok mcp --http")
    })
}

fn router(server: Arc<Server>, token: Arc<str>, config: StreamableHttpServerConfig) -> Router {
    let service = StreamableHttpService::new(
        move || Ok(Http(server.clone())),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    Router::new()
        .route_service(PATH, service)
        .layer(middleware::from_fn_with_state(token, require_token))
}

/// Stateless with JSON replies: no session table for internet callers to grow, and every
/// answer is one JSON body. Sessions are optional in the spec ("MAY assign a session ID").
fn transport_config(addr: SocketAddr, public: Option<&url::Url>) -> StreamableHttpServerConfig {
    let mut hosts: Vec<String> = LOOPBACK_HOSTS.iter().map(ToString::to_string).collect();
    if !addr.ip().is_unspecified() {
        hosts.push(addr.ip().to_string());
    }
    hosts.extend(public.and_then(url::Url::host_str).map(str::to_owned));
    let origins = public.map(|u| u.origin().ascii_serialization());
    StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_allowed_hosts(hosts)
        .with_allowed_origins(origins)
        // An empty list would switch the check off in rmcp; a browser page's Origin must be
        // refused unless it is the configured public one.
        .enforce_origin_validation()
}

/// Refuse a start that would expose more than one project over the network.
///
/// The `read` guard allows everything under the cwd, so a server started in `/` or `$HOME`
/// would serve `~/.ssh` to any token holder. A non-loopback bind sends the bearer token in
/// clear unless a TLS front (named by an `https` `public_url`) terminates the connection.
fn exposure_guard(
    cwd: &std::path::Path,
    home: Option<&std::path::Path>,
    addr: SocketAddr,
    public: Option<&url::Url>,
) -> Result<()> {
    if crate::fs::is_unwalkable_root(cwd, home) {
        bail!(
            "rtok mcp --http: refusing to serve {} (the filesystem root or home directory); \
             start it inside a project",
            cwd.display()
        );
    }
    if !addr.ip().is_loopback() && !public.is_some_and(|u| u.scheme() == "https") {
        bail!(
            "rtok mcp --http: {addr} is not loopback and `[mcp] public_url` is not https; the \
             bearer token would cross the network in clear. Bind 127.0.0.1 behind a TLS tunnel \
             and set public_url to its https URL"
        );
    }
    Ok(())
}

fn bearer_token(raw: &str) -> Result<Arc<str>> {
    if raw.chars().count() < MIN_TOKEN_CHARS || !raw.bytes().all(|b| b.is_ascii_graphic()) {
        bail!(
            "rtok mcp --http needs a bearer token of at least {MIN_TOKEN_CHARS} visible ASCII \
             characters in RTOK_MCP_TOKEN or `[mcp] token`"
        );
    }
    Ok(Arc::from(raw))
}

fn public_url(raw: &str) -> Result<Option<url::Url>> {
    if raw.is_empty() {
        return Ok(None);
    }
    let url = url::Url::parse(raw).with_context(|| format!("[mcp] public_url `{raw}`"))?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
        bail!("[mcp] public_url `{raw}` must be an http(s) URL with a host");
    }
    Ok(Some(url))
}

async fn require_token(State(token): State<Arc<str>>, req: Request, next: Next) -> Response {
    if bearer_matches(req.headers().get(header::AUTHORIZATION), &token) {
        return next.run(req).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        "Unauthorized",
    )
        .into_response()
}

/// RFC 6750 `Authorization: Bearer <token>`; the scheme name is case-insensitive
/// (RFC 9110 §11.1). The compare is constant-time so response timing does not reveal how
/// many leading bytes of a guess were right.
fn bearer_matches(value: Option<&HeaderValue>, token: &str) -> bool {
    let Some((scheme, got)) = value
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
    else {
        return false;
    };
    scheme.eq_ignore_ascii_case("bearer") && bool::from(got.as_bytes().ct_eq(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_expand_refuses_foreign_archives_but_keeps_its_own_lossless() {
        use crate::plugin::Archive;
        use serde_json::json;

        let (cfg, dir) = crate::testutil::config("http-archive-scope");
        let server = http_server(&cfg).unwrap();
        let body = b"private output from another session";
        let id = server
            .cx
            .store
            .put_archive("other-session", body, &cfg.core.archive_dir)
            .unwrap();
        for supplied in [&id[..8], id.as_str()] {
            let result = server.call_tool("expand", &json!({"id": supplied}));
            assert_eq!(result.is_error, Some(true), "{result:?}");
            let rendered = serde_json::to_string(&result).unwrap();
            assert!(!rendered.contains("private output"), "{rendered}");
        }
        // The store retains the first writer of identical bytes. A session-column filter
        // would reject a legitimate pointer even after this server archived those bytes.
        assert_eq!(server.cx.put_archive(body).unwrap(), id);
        for supplied in [&id[..8], id.as_str()] {
            let result = server.call_tool("expand", &json!({"id": supplied}));
            assert_ne!(result.is_error, Some(true), "{result:?}");
            assert!(
                serde_json::to_string(&result)
                    .unwrap()
                    .contains("private output")
            );
        }
        let restarted = http_server(&cfg).unwrap();
        assert_eq!(
            restarted.call_tool("expand", &json!({"id": id})).is_error,
            Some(true)
        );
        let stdio = Server::new(&cfg).unwrap();
        assert_ne!(
            stdio.call_tool("expand", &json!({"id": id})).is_error,
            Some(true)
        );
        drop((server, restarted, stdio));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn http_expand_resolves_collisions_only_among_granted_ids() {
        use crate::plugin::Archive;
        use serde_json::json;

        let (cfg, dir) = crate::testutil::config("http-archive-collision");
        let server = http_server(&cfg).unwrap();
        // Fixed SHA-256 fixtures collide in the eight hex characters printed in trailers.
        let first = b"rtok-t960-collision-22965";
        let second = b"rtok-t960-collision-44844";
        let own = server.cx.put_archive_for(first, Some("context")).unwrap();
        let foreign = server
            .cx
            .store
            .put_archive("other", second, &cfg.core.archive_dir)
            .unwrap();
        assert_eq!(&own[..8], &foreign[..8]);
        let prefix = &own[..8];
        let expanded = server.call_tool("expand", &json!({"id": prefix}));
        assert_ne!(expanded.is_error, Some(true), "{expanded:?}");
        assert!(
            serde_json::to_string(&expanded)
                .unwrap()
                .contains("collision-22965")
        );
        assert_eq!(server.cx.put_archive(second).unwrap(), foreign);
        let ambiguous = server.call_tool("expand", &json!({"id": prefix}));
        assert_eq!(ambiguous.is_error, Some(true));
        let message = serde_json::to_string(&ambiguous).unwrap();
        assert!(message.contains("ambiguous"), "{message}");
        for id in [&own, &foreign] {
            assert!(!message.contains(&id[..12]), "{message}");
            assert_ne!(
                server
                    .call_tool("expand", &json!({"id": &id[..12]}))
                    .is_error,
                Some(true)
            );
        }
        for invalid in ["", &own[..7], "../../archive", &format!("{own}0")] {
            assert_eq!(
                server.call_tool("expand", &json!({"id": invalid})).is_error,
                Some(true)
            );
        }
        drop(server);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bearer_needs_the_exact_token() {
        let token = "0123456789abcdef";
        let h = |s: &str| HeaderValue::from_str(s).unwrap();
        assert!(bearer_matches(Some(&h("Bearer 0123456789abcdef")), token));
        assert!(bearer_matches(Some(&h("bearer 0123456789abcdef")), token));
        assert!(!bearer_matches(Some(&h("Bearer 0123456789abcde")), token));
        assert!(!bearer_matches(Some(&h("Bearer 0123456789abcdefg")), token));
        assert!(!bearer_matches(Some(&h("Basic 0123456789abcdef")), token));
        assert!(!bearer_matches(Some(&h("0123456789abcdef")), token));
        assert!(!bearer_matches(None, token));
    }

    #[test]
    fn exposure_guard_refuses_home_root_and_plain_remote_binds() {
        let home = std::path::Path::new("/Users/someone");
        let project = home.join("src/app");
        let lo: SocketAddr = "127.0.0.1:8791".parse().unwrap();
        let wide: SocketAddr = "0.0.0.0:8791".parse().unwrap();
        assert!(exposure_guard(home, Some(home), lo, None).is_err());
        assert!(exposure_guard(std::path::Path::new("/"), Some(home), lo, None).is_err());
        assert!(exposure_guard(&project, Some(home), lo, None).is_ok());
        assert!(exposure_guard(&project, Some(home), wide, None).is_err());
        let http = public_url("http://example.com/mcp").unwrap();
        assert!(exposure_guard(&project, Some(home), wide, http.as_ref()).is_err());
        let https = public_url("https://example.com/mcp").unwrap();
        assert!(exposure_guard(&project, Some(home), wide, https.as_ref()).is_ok());
    }

    #[test]
    fn short_or_blank_tokens_refuse_to_start() {
        assert!(bearer_token("").is_err());
        assert!(bearer_token("short").is_err());
        assert!(bearer_token("has a space in it ok").is_err());
        assert!(bearer_token("0123456789abcdef").is_ok());
    }

    #[test]
    fn public_url_adds_only_its_own_host_and_origin() {
        let public = public_url("https://abc.trycloudflare.com/mcp").unwrap();
        let addr: SocketAddr = "127.0.0.1:8791".parse().unwrap();
        let c = transport_config(addr, public.as_ref());
        assert!(
            c.allowed_hosts
                .contains(&"abc.trycloudflare.com".to_string())
        );
        assert_eq!(c.allowed_origins, ["https://abc.trycloudflare.com"]);
        let c = transport_config("0.0.0.0:8791".parse().unwrap(), None);
        assert_eq!(c.allowed_hosts, LOOPBACK_HOSTS);
        assert!(c.allowed_origins.is_empty());
        assert!(public_url("ftp://x").is_err());
        assert!(public_url("").unwrap().is_none());
    }
}
