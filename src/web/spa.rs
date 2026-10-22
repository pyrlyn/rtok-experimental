// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The React SPA `rtok web` serves (T310.9): the built `web/dist`, baked into the binary by
//! `rust-embed`, or a directory read at run time when `RTOK_WEB_DIST` names one.
//!
//! One handler serves both sources, so the headers cannot differ between a release binary and a
//! developer's local build:
//! - files under `assets/` are content-hashed by Vite, so they are `immutable` for a year;
//!   everything else (`index.html`, `theme-init.js`) is `no-cache` and revalidated by `ETag`;
//! - `web/scripts/precompress.mjs` writes `.br`/`.gz` beside each text asset, and the handler
//!   serves the one the browser accepts instead of compressing per request;
//! - a path with no file is a page route of the SPA (served `index.html`) unless it names an
//!   API surface (`/ws`, `/health`, `/api`, `/assets`) or has a file extension, which stay 404 so
//!   a typo in an asset URL is not answered with HTML;
//! - every response carries a CSP that forbids inline scripts and any non-same-origin load.

use std::borrow::Cow;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::header::{
    ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_SECURITY_POLICY, CONTENT_TYPE, ETAG,
    IF_NONE_MATCH, REFERRER_POLICY, VARY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use percent_encoding::percent_decode_str;
use rust_embed::RustEmbed;

/// `build.rs` points `RTOK_SPA_DIR` at `web/dist`, or at a one-page placeholder when the SPA was
/// not built (so `cargo build` works without npm).
#[derive(RustEmbed)]
#[folder = "$RTOK_SPA_DIR"]
struct Embedded;

/// Run-time override: a directory with a built SPA, served instead of the embedded copy. Not
/// `RTOK_WEB_*` from the config loader's namespace (`web.host`, `web.port`), which reads keys.
pub const DIST_ENV: &str = "RTOK_WEB_DIST";

/// True when this binary embeds the "UI not built" page instead of the SPA.
pub fn embedded_is_placeholder() -> bool {
    option_env!("RTOK_SPA_PLACEHOLDER").is_some()
}

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
    img-src 'self' data:; font-src 'self'; connect-src 'self'; object-src 'none'; \
    base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const REVALIDATE: &str = "no-cache";
/// First path segments that belong to the API, never to a page route.
const RESERVED: [&str; 5] = ["ws", "health", "auth", "api", "assets"];

/// Where the SPA's files come from.
#[derive(Debug, Clone)]
pub enum Assets {
    /// Compiled into this binary.
    Embedded,
    /// A built `dist/` on disk, read per request.
    Dir(PathBuf),
}

/// The source to serve and one line to print at startup when something is off. `dist` is the
/// value of [`DIST_ENV`]. A directory that is not one falls back to the embedded copy with a
/// warning: a typo in a dev override must not take the dashboard down (fail open).
pub fn resolve(dist: Option<OsString>) -> (Assets, Option<String>) {
    if let Some(dir) = dist {
        let dir = PathBuf::from(dir);
        if dir.join("index.html").is_file() {
            return (Assets::Dir(dir), None);
        }
        let note = format!(
            "rtok web: {DIST_ENV}={} has no index.html; serving the embedded UI instead",
            dir.display()
        );
        return (Assets::Embedded, Some(note));
    }
    let note = embedded_is_placeholder().then(|| {
        format!(
            "rtok web: this binary was built without the SPA (web/dist was missing), so the UI \
             cannot load (the API and /ws are up). Build it with `just web`, or point {DIST_ENV} \
             at a built dist/ directory."
        )
    });
    (Assets::Embedded, note)
}

/// The router for every path the API does not claim.
pub fn router(assets: Assets) -> Router {
    Router::new().fallback(serve).with_state(Arc::new(assets))
}

struct File {
    bytes: Cow<'static, [u8]>,
    /// Quoted hex digest; only the embedded copy has one (computed at compile time).
    etag: Option<String>,
}

impl Assets {
    async fn read(&self, name: &str) -> Option<File> {
        match self {
            Assets::Embedded => Embedded::get(name).map(|f| {
                let mut etag = String::from("\"");
                for b in f.metadata.sha256_hash() {
                    let _ = write!(etag, "{b:02x}");
                }
                etag.push('"');
                File {
                    bytes: f.data,
                    etag: Some(etag),
                }
            }),
            Assets::Dir(root) => {
                let bytes = tokio::fs::read(safe_join(root, name)?).await.ok()?;
                Some(File {
                    bytes: Cow::Owned(bytes),
                    etag: None,
                })
            }
        }
    }
}

/// Reject lexical traversal and symlinks that would expose files outside the SPA directory.
fn safe_join(root: &Path, name: &str) -> Option<PathBuf> {
    let rel = Path::new(name);
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    rtok_config::root::contain(root, rel).ok()
}

/// The file name a URL path asks for: each segment percent-decoded exactly once. `None` for text
/// that is not a file name: a malformed escape, bytes that are not UTF-8, or a segment that
/// decodes to a separator or NUL (`%2F` must not turn one segment into a nested path).
fn decode_path(path: &str) -> Option<String> {
    let segments = path
        .split('/')
        .map(|seg| {
            let well_formed = seg.match_indices('%').all(|(i, _)| {
                seg.as_bytes()
                    .get(i + 1..i + 3)
                    .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit))
            });
            let decoded = percent_decode_str(seg).decode_utf8().ok()?;
            (well_formed && !decoded.contains(['/', '\\', '\0'])).then_some(decoded)
        })
        .collect::<Option<Vec<_>>>()?;
    Some(segments.join("/"))
}

/// Whether the client accepts `coding` (`q=0` refuses it).
fn accepts(headers: &HeaderMap, coding: &str) -> bool {
    headers
        .get_all(ACCEPT_ENCODING)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|item| {
            let mut parts = item.split(';');
            let name = parts.next().unwrap_or("").trim();
            let q = parts
                .find_map(|p| p.trim().strip_prefix("q="))
                .and_then(|q| q.trim().parse::<f32>().ok())
                .unwrap_or(1.0);
            name.eq_ignore_ascii_case(coding) && q > 0.0
        })
}

/// A route of the SPA itself rather than a missing file or an API path.
fn is_page_route(path: &str) -> bool {
    let first = path.split('/').next().unwrap_or("");
    let last = path.rsplit('/').next().unwrap_or("");
    !RESERVED.contains(&first) && !last.contains('.')
}

struct Served {
    file: File,
    encoding: Option<&'static str>,
}

/// `name`, as the precompressed variant the client accepts when there is one.
async fn lookup(assets: &Assets, name: &str, headers: &HeaderMap) -> Option<Served> {
    // The variants are reachable only through negotiation, not by their own URL.
    if name.ends_with(".br") || name.ends_with(".gz") {
        return None;
    }
    for (coding, ext) in [("br", ".br"), ("gzip", ".gz")] {
        if accepts(headers, coding)
            && let Some(file) = assets.read(&format!("{name}{ext}")).await
        {
            return Some(Served {
                file,
                encoding: Some(coding),
            });
        }
    }
    let file = assets.read(name).await?;
    Some(Served {
        file,
        encoding: None,
    })
}

async fn serve(
    State(assets): State<Arc<Assets>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return secured(StatusCode::METHOD_NOT_ALLOWED.into_response());
    }
    let Some(path) = decode_path(uri.path()) else {
        return secured(StatusCode::BAD_REQUEST.into_response());
    };
    let path = path.trim_start_matches('/');
    let name = if path.is_empty() { "index.html" } else { path };
    let (name, served) = match lookup(&assets, name, &headers).await {
        Some(s) => (name, Some(s)),
        None if is_page_route(path) => {
            ("index.html", lookup(&assets, "index.html", &headers).await)
        }
        None => (name, None),
    };
    let Some(served) = served else {
        return secured(if is_page_route(path) {
            // A `dist/` directory without an index: say so rather than 404 the whole UI.
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("rtok web: no index.html in the SPA source; build it with `just web` or fix {DIST_ENV}"),
            )
                .into_response()
        } else {
            StatusCode::NOT_FOUND.into_response()
        });
    };
    secured(respond(name, served, &headers))
}

fn respond(name: &str, served: Served, request: &HeaderMap) -> Response {
    let cache = if name.starts_with("assets/") {
        IMMUTABLE
    } else {
        REVALIDATE
    };
    let mut res = if served.file.etag.as_deref().is_some_and(|etag| {
        request
            .get_all(IF_NONE_MATCH)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .any(|t| t.trim() == "*" || t.trim().trim_start_matches("W/") == etag)
    }) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let mime = mime_guess::from_path(name).first_or_octet_stream();
        let mut body = (StatusCode::OK, served.file.bytes.into_owned()).into_response();
        let kind = if mime.type_() == mime_guess::mime::TEXT {
            format!("{mime}; charset=utf-8")
        } else {
            mime.to_string()
        };
        if let Ok(v) = HeaderValue::from_str(&kind) {
            body.headers_mut().insert(CONTENT_TYPE, v);
        }
        if let Some(coding) = served.encoding {
            body.headers_mut()
                .insert(CONTENT_ENCODING, HeaderValue::from_static(coding));
        }
        body
    };
    let h = res.headers_mut();
    h.insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    h.insert(VARY, HeaderValue::from_static("Accept-Encoding"));
    if let Some(v) = served
        .file
        .etag
        .as_deref()
        .and_then(|e| HeaderValue::from_str(e).ok())
    {
        h.insert(ETAG, v);
    }
    res
}

/// The headers every UI response carries, errors included.
fn secured(mut res: Response) -> Response {
    let h = res.headers_mut();
    h.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    h.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accept(v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(ACCEPT_ENCODING, HeaderValue::from_str(v).unwrap());
        h
    }

    #[test]
    fn accept_encoding_honours_names_and_q_values() {
        assert!(accepts(&accept("gzip, br"), "br"));
        assert!(accepts(&accept("GZIP;q=0.5"), "gzip"));
        assert!(!accepts(&accept("br;q=0"), "br"));
        assert!(!accepts(&accept("gzip"), "br"));
        assert!(!accepts(&HeaderMap::new(), "gzip"));
    }

    #[test]
    fn page_routes_exclude_the_api_and_files() {
        assert!(is_page_route("overview"));
        assert!(is_page_route("sessions/42"));
        assert!(!is_page_route("ws"));
        assert!(!is_page_route("ws/x"));
        assert!(!is_page_route("auth"));
        assert!(!is_page_route("api/v1/anything"));
        assert!(!is_page_route("assets/missing"));
        assert!(!is_page_route("favicon.ico"));
        assert!(!is_page_route("a/b.js"));
    }

    #[test]
    fn url_paths_are_decoded_once_and_only_into_file_names() {
        assert_eq!(
            decode_path("/assets/a%20b.svg").as_deref(),
            Some("/assets/a b.svg")
        );
        assert_eq!(decode_path("/a%2520b").as_deref(), Some("/a%20b"));
        assert_eq!(decode_path("/caf%C3%A9").as_deref(), Some("/caf\u{e9}"));
        for bad in ["/a%2Fb", "/a%5cb", "/a%00", "/100%", "/%zz", "/%4", "/%FF"] {
            assert_eq!(decode_path(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_name_cannot_leave_the_served_directory() {
        let root = Path::new("/srv/dist");
        assert!(safe_join(root, "assets/app.js").is_some());
        assert!(safe_join(root, "../etc/passwd").is_none());
        assert!(safe_join(root, "assets/../../x").is_none());
        assert!(safe_join(root, "/etc/passwd").is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn directory_assets_refuse_symlink_escapes_but_keep_internal_links() {
        use std::os::unix::fs::symlink;

        let dir = crate::testutil::tmp_dir("spa-symlink-boundary");
        let root = dir.join("dist");
        let outside = dir.join("private");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("app.js"), "public").unwrap();
        std::fs::write(outside.join("secret.js"), "secret").unwrap();
        symlink(outside.join("secret.js"), root.join("leak.js")).unwrap();
        symlink(&outside, root.join("assets")).unwrap();
        symlink(outside.join("secret.js"), root.join("app.js.br")).unwrap();
        symlink(root.join("app.js"), root.join("alias.js")).unwrap();
        let assets = Assets::Dir(root);

        for name in ["leak.js", "assets/secret.js", "app.js.br"] {
            assert!(assets.read(name).await.is_none(), "served {name}");
        }
        for name in ["app.js", "alias.js"] {
            assert_eq!(assets.read(name).await.unwrap().bytes.as_ref(), b"public");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_bad_override_falls_back_to_the_embedded_ui_with_a_warning() {
        let (assets, note) = resolve(Some(OsString::from("/no/such/rtok-dist")));
        assert!(matches!(assets, Assets::Embedded));
        assert!(note.is_some_and(|n| n.contains(DIST_ENV)));
    }
}
