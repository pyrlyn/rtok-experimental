// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! What the GitHub (T441.7) and GitLab (T441.8) adapters share: the `rtok:<id>` label and
//! title naming, the token lookup, and one HTTP client that paces writes and waits out rate
//! limits (`research.md` §35.4).

use std::cell::Cell;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use reqwest::Method;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{CONTENT_TYPE, HeaderMap};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::TaskId;

/// Every rtok issue carries this label, so `list` asks the server for rtok's issues only
/// instead of paging through every issue and pull request in the repository.
pub const LABEL: &str = "rtok";

/// GitHub's floor between content-creating requests, also a safe pace for GitLab.
pub const WRITE_GAP: Duration = Duration::from_secs(1);

/// Tries per request when the server says to wait; a limit that outlasts them is an error.
const ATTEMPTS: usize = 3;

/// A CLI call or an MCP tool waits at most this long for a limit to reset; longer, the
/// agent is better told to come back later than left hanging.
const MAX_WAIT: Duration = Duration::from_secs(60);

/// The label naming one task: `rtok:R12`. Ids are ours, so the issue number never is one.
pub fn id_label(id: &TaskId) -> String {
    format!("{LABEL}:{id}")
}

/// The task a label names; `None` for any other label, `rtok:in-progress` included.
pub fn label_id(name: &str) -> Option<TaskId> {
    name.strip_prefix(LABEL)?.strip_prefix(':')?.parse().ok()
}

/// The issue title: `R12. Ship it`, so the id shows in every issue list.
pub fn issue_title(id: &TaskId, title: &str) -> String {
    format!("{id}. {}", title.trim())
}

/// The id an issue title still starts with (`R12. Ship it`), for an issue whose label is gone.
pub fn title_id(title: &str) -> Option<TaskId> {
    title.split_once(". ")?.0.trim().parse().ok()
}

/// The task title back from an issue title; one edited by hand keeps whatever it says.
pub fn task_title(id: &TaskId, title: &str) -> String {
    let lead = format!("{id}. ");
    match title.get(..lead.len()) {
        Some(head) if head.eq_ignore_ascii_case(&lead) => title[lead.len()..].trim().to_string(),
        _ => title.trim().to_string(),
    }
}

/// Unix seconds of an API timestamp (RFC 3339 on both providers); 0 when it does not parse.
pub fn secs(ts: &str) -> i64 {
    ts.parse::<jiff::Timestamp>().map_or(0, |t| t.as_second())
}

/// The highest of `ids` with `prefix`, any case: `max_id` over the issues a provider listed.
pub fn max_with_prefix(ids: impl IntoIterator<Item = TaskId>, prefix: &str) -> Option<TaskId> {
    let prefix = prefix.to_ascii_uppercase();
    ids.into_iter().filter(|id| id.prefix() == prefix).max()
}

/// The first non-empty variable of `vars`, else the trimmed output of the `cli` command
/// (`gh auth token`), the order the providers' own CLIs use.
pub fn token(adapter: &str, vars: &[&str], cli: &[&str]) -> Result<String> {
    token_with(|v| std::env::var(v).ok(), vars, cli).with_context(|| {
        // A provider with no CLI (Linear) has only the variable to offer.
        if cli.is_empty() {
            format!("{adapter}: no token; set {}", vars.join(" or "))
        } else {
            format!(
                "{adapter}: no token; set {} or run `{}`",
                vars.join(" or "),
                cli.join(" ")
            )
        }
    })
}

fn token_with(env: impl Fn(&str) -> Option<String>, vars: &[&str], cli: &[&str]) -> Result<String> {
    if let Some(t) = vars
        .iter()
        .filter_map(|v| env(v))
        .map(|t| t.trim().to_string())
        .find(|t| !t.is_empty())
    {
        return Ok(t);
    }
    let Some((program, args)) = cli.split_first() else {
        bail!("no token in the environment");
    };
    let out = Command::new(program).args(args).output()?;
    let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || t.is_empty() {
        bail!("`{}` printed no token", cli.join(" "));
    }
    Ok(t)
}

/// A blocking JSON client for one API base. The adapter trait is synchronous (CLI, stdio
/// MCP), so there is no runtime to hand an async client.
pub struct Http {
    adapter: &'static str,
    base: String,
    client: Client,
    gap: Duration,
    last_write: Cell<Option<Instant>>,
}

impl Http {
    /// `headers` carry the auth and API version every request sends; `gap` is the pause
    /// between writes ([`WRITE_GAP`], zero in tests).
    pub fn new(
        adapter: &'static str,
        base: &str,
        headers: HeaderMap,
        gap: Duration,
    ) -> Result<Self> {
        let client = Client::builder()
            .use_preconfigured_tls(crate::tls::preconfigured()?)
            .user_agent(concat!("rtok/", env!("CARGO_PKG_VERSION")))
            .default_headers(headers)
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            adapter,
            base: base.trim_end_matches('/').to_string(),
            client,
            gap,
            last_write: Cell::new(None),
        })
    }

    /// Every item of a paged list, following `Link: rel="next"` (GitHub and GitLab both send it).
    pub fn get_all<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Vec<T>> {
        let mut url =
            url::Url::parse_with_params(&format!("{}{path}", self.base), query)?.to_string();
        let mut out = Vec::new();
        loop {
            let res = self.exec(&Method::GET, &url, None)?;
            let next = next_link(res.headers());
            let page: Vec<T> = self.parse(&Method::GET, &url, res)?;
            out.extend(page);
            match next {
                // The token goes with every request, so a link off the API host is not followed.
                Some(n) if n.starts_with(&format!("{}/", self.base)) => url = n,
                _ => return Ok(out),
            }
        }
    }

    /// One write; the answer parsed as `T`.
    pub fn send<T: DeserializeOwned>(&self, method: Method, path: &str, body: &Value) -> Result<T> {
        let url = format!("{}{path}", self.base);
        let res = self.exec(&method, &url, Some(body))?;
        self.parse(&method, &url, res)
    }

    fn parse<T: DeserializeOwned>(&self, method: &Method, url: &str, res: Response) -> Result<T> {
        let bytes = res
            .bytes()
            .with_context(|| format!("{}: {method} {url}", self.adapter))?;
        serde_json::from_slice(&bytes)
            .with_context(|| format!("{}: {method} {url}: unexpected JSON", self.adapter))
    }

    fn exec(&self, method: &Method, url: &str, body: Option<&Value>) -> Result<Response> {
        for attempt in 1..=ATTEMPTS {
            if body.is_some() {
                self.pace();
            }
            let mut req: RequestBuilder = self.client.request(method.clone(), url);
            if let Some(b) = body {
                req = req
                    .header(CONTENT_TYPE, "application/json")
                    .body(serde_json::to_vec(b)?);
            }
            let res = req
                .send()
                .with_context(|| format!("{}: {method} {url}", self.adapter))?;
            let status = res.status();
            if status.is_success() {
                return Ok(res);
            }
            if let Some(wait) = rate_wait(status.as_u16(), res.headers(), unix_now()) {
                if wait > MAX_WAIT || attempt == ATTEMPTS {
                    bail!(
                        "{}: {method} {url}: rate limited; try again in {} s",
                        self.adapter,
                        wait.as_secs()
                    );
                }
                std::thread::sleep(wait);
                continue;
            }
            let text = res.text().unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                // GraphQL (Linear) puts the reason in `errors[0].message`, even for a 400.
                .and_then(|v| {
                    v.get("message")
                        .or_else(|| v.pointer("/errors/0/message"))?
                        .as_str()
                        .map(String::from)
                })
                .unwrap_or_else(|| text.chars().take(300).collect());
            bail!("{}: {method} {url}: {status}: {msg}", self.adapter);
        }
        unreachable!("the last attempt returns or bails")
    }

    /// Hold the next write until `gap` has passed since the last one.
    fn pace(&self) {
        if let Some(last) = self.last_write.get() {
            let since = last.elapsed();
            if since < self.gap {
                std::thread::sleep(self.gap - since);
            }
        }
        self.last_write.set(Some(Instant::now()));
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// How long a refused request should wait before its retry: `retry-after`, else the
/// primary limit's reset time, else a minute for a bare 429 (GitHub's best-practices order).
/// `None` when the refusal is not a rate limit (a 403 for a missing permission).
fn rate_wait(status: u16, headers: &HeaderMap, now: u64) -> Option<Duration> {
    if status != 403 && status != 429 {
        return None;
    }
    let header =
        |name: &str| -> Option<u64> { headers.get(name)?.to_str().ok()?.trim().parse().ok() };
    if let Some(secs) = header("retry-after") {
        return Some(Duration::from_secs(secs));
    }
    if header("x-ratelimit-remaining") == Some(0)
        && let Some(reset) = header("x-ratelimit-reset")
    {
        return Some(Duration::from_secs(reset.saturating_sub(now)));
    }
    (status == 429).then_some(MAX_WAIT)
}

/// The `rel="next"` URL of a `Link` header.
fn next_link(headers: &HeaderMap) -> Option<String> {
    let link = headers.get("link")?.to_str().ok()?;
    link.split(',').find_map(|part| {
        let (url, rel) = part.split_once(';')?;
        rel.contains("rel=\"next\"").then(|| {
            url.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_string()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn labels_and_titles_round_trip() {
        let id: TaskId = "R2.1".parse().unwrap();
        assert_eq!(id_label(&id), "rtok:R2.1");
        assert_eq!(label_id("rtok:r2.1"), Some(id.clone()));
        assert_eq!(label_id("rtok:in-progress"), None);
        assert_eq!(label_id("rtokR2"), None);
        assert_eq!(label_id("bug"), None);
        assert_eq!(issue_title(&id, " Ship it "), "R2.1. Ship it");
        assert_eq!(task_title(&id, "R2.1. Ship it"), "Ship it");
        assert_eq!(task_title(&id, "Renamed by hand"), "Renamed by hand");
        assert_eq!(
            task_title(&id, "Ж"),
            "Ж",
            "no panic on a short multibyte title"
        );
    }

    #[test]
    fn tokens_come_from_the_first_set_variable_then_the_cli() {
        let env = |v: &str| match v {
            "GITHUB_TOKEN" => Some("from-github".to_string()),
            "GH_TOKEN" => Some("  ".to_string()),
            _ => None,
        };
        let t = token_with(env, &["GH_TOKEN", "GITHUB_TOKEN"], &[]).unwrap();
        assert_eq!(t, "from-github");
        assert!(token_with(|_| None, &["GH_TOKEN"], &[]).is_err());
        // A CLI that is not installed is an error, not a panic.
        assert!(token_with(|_| None, &["GH_TOKEN"], &["rtok-no-such-cli", "auth"]).is_err());
    }

    #[test]
    fn rate_limits_wait_by_retry_after_then_reset() {
        let h = headers(&[("retry-after", "7")]);
        assert_eq!(rate_wait(429, &h, 0), Some(Duration::from_secs(7)));
        let h = headers(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "130")]);
        assert_eq!(rate_wait(403, &h, 100), Some(Duration::from_secs(30)));
        assert_eq!(rate_wait(429, &HeaderMap::new(), 0), Some(MAX_WAIT));
        assert_eq!(
            rate_wait(403, &HeaderMap::new(), 0),
            None,
            "a permission error"
        );
        assert_eq!(rate_wait(404, &h, 100), None);
    }

    #[test]
    fn next_links_are_read_from_the_link_header() {
        let h = headers(&[(
            "link",
            r#"<https://api.github.com/x?page=2>; rel="next", <https://api.github.com/x?page=5>; rel="last""#,
        )]);
        assert_eq!(
            next_link(&h).as_deref(),
            Some("https://api.github.com/x?page=2")
        );
        let h = headers(&[("link", r#"<https://api.github.com/x?page=1>; rel="prev""#)]);
        assert_eq!(next_link(&h), None);
    }

    #[test]
    fn a_limit_that_never_lifts_ends_in_an_error_and_pages_stay_on_the_host() {
        let server = httpmock::MockServer::start();
        let limited = server.mock(|when, then| {
            when.method("POST").path("/w");
            then.status(429).header("retry-after", "0");
        });
        let http = Http::new(
            "github",
            &server.base_url(),
            HeaderMap::new(),
            Duration::ZERO,
        )
        .unwrap();
        let err = http
            .send::<Value>(Method::POST, "/w", &serde_json::json!({}))
            .unwrap_err();
        assert!(err.to_string().contains("rate limited"), "{err}");
        limited.assert_calls(ATTEMPTS);

        server.mock(|when, then| {
            when.method("GET").path("/l").query_param("page", "1");
            then.status(200)
                .header(
                    "link",
                    format!("<{}/l?page=2>; rel=\"next\"", server.base_url()),
                )
                .body("[1]");
        });
        server.mock(|when, then| {
            when.method("GET").path("/l").query_param("page", "2");
            then.status(200)
                .header("link", "<https://elsewhere.example/l?page=3>; rel=\"next\"")
                .body("[2]");
        });
        let all: Vec<u32> = http.get_all("/l", &[("page", "1")]).unwrap();
        assert_eq!(all, [1, 2]);

        server.mock(|when, then| {
            when.method("GET").path("/denied");
            then.status(403)
                .body(r#"{"message":"Resource not accessible"}"#);
        });
        let err = http.get_all::<u32>("/denied", &[]).unwrap_err();
        assert!(err.to_string().contains("Resource not accessible"), "{err}");
    }
}
