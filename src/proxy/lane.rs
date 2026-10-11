// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Request lanes (T385.1): which kind of traffic a proxied request is, so the ledger can tell
//! agent turns from bulk scripts, Batch jobs, uploads and rtok's own calls.
//!
//! The classifier is deliberately dumb. A path that names its own kind of traffic (Batch,
//! files, embeddings, models/token counting) decides the lane; an explicit `x-rtok-lane`
//! header or `/lane/<name>/` path prefix picks among what a path cannot say (a sync chat
//! call is an agent turn or a bulk script — only the caller knows). No heuristics.

use axum::http::HeaderMap;

use crate::config::{LanePolicy, Lanes, RouteRule, RoutingPolicy};

/// Request header naming the lane; stripped before the request goes upstream.
pub const HEADER: &str = "x-rtok-lane";
const PREFIX: &str = "/lane/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lane {
    Agent,
    Bulk,
    Batch,
    Files,
    Embeddings,
    Meta,
    Internal,
}

impl Lane {
    pub const ALL: [Lane; 7] = [
        Lane::Agent,
        Lane::Bulk,
        Lane::Batch,
        Lane::Files,
        Lane::Embeddings,
        Lane::Meta,
        Lane::Internal,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Lane::Agent => "agent",
            Lane::Bulk => "bulk",
            Lane::Batch => "batch",
            Lane::Files => "files",
            Lane::Embeddings => "embeddings",
            Lane::Meta => "meta",
            Lane::Internal => "internal",
        }
    }

    fn from_name(name: &str) -> Option<Lane> {
        let name = name.trim();
        Lane::ALL
            .into_iter()
            .find(|lane| lane.name().eq_ignore_ascii_case(name))
    }

    /// What the proxy may change on this lane (T385.2). The `agent` lane is the baseline:
    /// every rewrite is allowed and the global switches alone decide, which is how the proxy
    /// behaved before lanes. `batch` and `files` carry JSONL and uploads the proxy must never
    /// rewrite, so no setting can open them — nor move them to another upstream, since a
    /// Batch job's create, poll and results must all reach the provider that owns it. The
    /// rest read their `[proxy.lanes.<lane>]` table.
    pub fn policy(self, lanes: &Lanes) -> LanePolicy {
        match self {
            Lane::Agent => LanePolicy {
                compress: true,
                toon: true,
                tools_rewrite: true,
                context_management: true,
                semantic_cache: true,
                // Never silently: an agent turn is Flex only when the client asks for it.
                flex: false,
                timeout_s: 0,
                // The interactive lane is never capped or queued, and keeps the wire's
                // upstream: isolation exists so other lanes cannot slow it down (T385.7).
                upstream: String::new(),
                max_in_flight: 0,
                max_queued: 0,
            },
            Lane::Batch | Lane::Files => LanePolicy::default(),
            Lane::Bulk => lanes.bulk.clone(),
            Lane::Embeddings => lanes.embeddings.clone(),
            Lane::Meta => lanes.meta.clone(),
            Lane::Internal => lanes.internal.clone(),
        }
    }

    /// The model-routing rule of this lane (T385.10), `None` when routing is off or the lane
    /// cannot be routed. Only `internal` and `bulk` can: the `agent` lane is a live turn whose
    /// quality the operator chose, and the rest carry no chat body the proxy may rewrite.
    pub fn route(self, routing: &RoutingPolicy) -> Option<&RouteRule> {
        if !routing.enabled {
            return None;
        }
        match self {
            Lane::Bulk => Some(&routing.bulk),
            Lane::Internal => Some(&routing.internal),
            _ => None,
        }
    }

    /// True for the lanes whose bodies are forwarded verbatim whatever the config says —
    /// also past the usage-capture shaping (`stream_options`) that other lanes still get.
    pub fn passes_through(self) -> bool {
        matches!(self, Lane::Batch | Lane::Files)
    }

    /// The `calls.kind` this lane is recorded under. The agent lane keeps the bare
    /// `api_request` every existing row and report already uses, so default traffic leaves
    /// the ledger exactly as before; the rest are `api_request:<lane>`, which `is_api_request`
    /// still recognises.
    pub fn kind(self) -> &'static str {
        match self {
            Lane::Agent => crate::lane::KIND_AGENT,
            Lane::Bulk => crate::lane::KIND_BULK,
            Lane::Batch => crate::lane::KIND_BATCH,
            Lane::Files => crate::lane::KIND_FILES,
            Lane::Embeddings => crate::lane::KIND_EMBEDDINGS,
            Lane::Meta => crate::lane::KIND_META,
            Lane::Internal => crate::lane::KIND_INTERNAL,
        }
    }
}

pub use crate::lane::{is_api_request, lane_of_kind};

/// The lane of one request and the path to forward (the `/lane/<name>` prefix removed).
#[derive(Debug, PartialEq, Eq)]
pub struct Classified<'a> {
    pub lane: Lane,
    pub path: &'a str,
}

/// Classify `path` + `headers`. Unknown paths and sync chat wires fall to `agent` — what the
/// proxy treated them as before lanes existed.
pub fn classify<'a>(path: &'a str, headers: &HeaderMap) -> Classified<'a> {
    let (prefixed, path) = strip_prefix(path);
    let marked = prefixed.or_else(|| {
        headers
            .get(HEADER)
            .and_then(|v| v.to_str().ok())
            .and_then(Lane::from_name)
    });
    Classified {
        lane: by_path(path).or(marked).unwrap_or(Lane::Agent),
        path,
    }
}

/// `/lane/<name>/rest` → (`<name>`, `/rest`). An unknown name is not ours to strip.
fn strip_prefix(path: &str) -> (Option<Lane>, &str) {
    let Some(rest) = path.strip_prefix(PREFIX) else {
        return (None, path);
    };
    match rest.split_once('/') {
        Some((name, tail)) => match Lane::from_name(name) {
            // `tail` starts after the slash; the forwarded path keeps it.
            Some(lane) => (Some(lane), &path[path.len() - tail.len() - 1..]),
            None => (None, path),
        },
        None => (None, path),
    }
}

fn under(path: &str, base: &str) -> bool {
    path.strip_prefix(base)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// The lane a path names by itself, across the Anthropic, OpenAI and Gemini surfaces.
fn by_path(path: &str) -> Option<Lane> {
    // Gemini puts the verb after a colon on the model path.
    if (under(path, "/v1beta/models") || under(path, "/v1/models"))
        && let Some((_, action)) = path.rsplit_once(':')
    {
        return match action {
            "batchGenerateContent" | "asyncBatchEmbedContent" => Some(Lane::Batch),
            "embedContent" | "batchEmbedContents" => Some(Lane::Embeddings),
            "countTokens" => Some(Lane::Meta),
            _ => None,
        };
    }
    const BATCH: [&str; 3] = ["/v1/messages/batches", "/v1/batches", "/v1beta/batches"];
    const FILES: [&str; 3] = ["/v1/files", "/v1beta/files", "/upload/v1beta/files"];
    const META: [&str; 3] = ["/v1/messages/count_tokens", "/v1/models", "/v1beta/models"];
    let any = |bases: &[&str]| bases.iter().any(|base| under(path, base));
    if any(&BATCH) {
        Some(Lane::Batch)
    } else if any(&FILES) {
        Some(Lane::Files)
    } else if under(path, "/v1/embeddings") {
        Some(Lane::Embeddings)
    } else if any(&META) {
        Some(Lane::Meta)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use rstest::rstest;

    fn lane_of(path: &str, header: Option<&str>) -> (Lane, String) {
        let mut headers = HeaderMap::new();
        if let Some(v) = header {
            headers.insert(HEADER, HeaderValue::from_str(v).unwrap());
        }
        let c = classify(path, &headers);
        (c.lane, c.path.to_string())
    }

    #[rstest]
    #[case("/v1/messages", Lane::Agent)]
    #[case("/v1/chat/completions", Lane::Agent)]
    #[case("/v1beta/models/gemini-2.0-flash:generateContent", Lane::Agent)]
    #[case("/v1/something/new", Lane::Agent)]
    #[case("/v1/messages/batches", Lane::Batch)]
    #[case("/v1/messages/batches/msgbatch_1/results", Lane::Batch)]
    #[case("/v1/batches/batch_1/cancel", Lane::Batch)]
    #[case("/v1beta/models/gemini-2.0-flash:batchGenerateContent", Lane::Batch)]
    #[case("/v1/files", Lane::Files)]
    #[case("/v1/files/file_1/content", Lane::Files)]
    #[case("/upload/v1beta/files", Lane::Files)]
    #[case("/v1/embeddings", Lane::Embeddings)]
    #[case("/v1beta/models/text-embedding-004:embedContent", Lane::Embeddings)]
    #[case("/v1/models", Lane::Meta)]
    #[case("/v1/models/gpt-5", Lane::Meta)]
    #[case("/v1/messages/count_tokens", Lane::Meta)]
    #[case("/v1beta/models/gemini-2.0-flash:countTokens", Lane::Meta)]
    fn paths_name_their_lane(#[case] path: &str, #[case] lane: Lane) {
        assert_eq!(lane_of(path, None), (lane, path.to_string()));
    }

    #[test]
    fn marker_picks_among_what_a_path_cannot_say() {
        assert_eq!(lane_of("/v1/messages", Some("Bulk")).0, Lane::Bulk);
        assert_eq!(lane_of("/v1/responses", Some("internal")).0, Lane::Internal);
        // A structural path keeps its lane whatever the caller claims.
        assert_eq!(lane_of("/v1/messages/batches", Some("bulk")).0, Lane::Batch);
        // Not a lane: ignored, never an error.
        assert_eq!(lane_of("/v1/messages", Some("turbo")).0, Lane::Agent);
    }

    #[test]
    fn prefix_is_stripped_and_beats_the_header() {
        assert_eq!(
            lane_of("/lane/bulk/v1/messages", Some("internal")),
            (Lane::Bulk, "/v1/messages".to_string())
        );
        assert_eq!(
            lane_of("/lane/bulk/v1/files/f1", None),
            (Lane::Files, "/v1/files/f1".to_string())
        );
    }

    #[rstest]
    #[case("/lane/turbo/v1/messages")]
    #[case("/lane/bulk")]
    #[case("/lane/")]
    #[case("/lanes/bulk/v1/messages")]
    fn foreign_prefixes_are_forwarded_untouched(#[case] path: &str) {
        assert_eq!(lane_of(path, None), (Lane::Agent, path.to_string()));
    }

    #[test]
    fn only_the_agent_lane_rewrites_by_default() {
        let lanes = Lanes::default();
        for lane in Lane::ALL {
            let p = lane.policy(&lanes);
            let all_on =
                p.compress && p.toon && p.tools_rewrite && p.context_management && p.semantic_cache;
            assert_eq!(all_on, lane == Lane::Agent, "{}", lane.name());
            assert_eq!(p.timeout_s, 0, "{}", lane.name());
            if lane != Lane::Agent {
                assert_eq!(p, LanePolicy::default(), "{}", lane.name());
            }
        }
    }

    #[test]
    fn batch_and_files_ignore_the_config() {
        let on = LanePolicy {
            compress: true,
            tools_rewrite: true,
            ..LanePolicy::default()
        };
        let lanes = Lanes {
            bulk: on.clone(),
            embeddings: on.clone(),
            meta: on.clone(),
            internal: on.clone(),
            ..Lanes::default()
        };
        assert_eq!(Lane::Bulk.policy(&lanes), on);
        assert_eq!(Lane::Internal.policy(&lanes), on);
        for lane in [Lane::Batch, Lane::Files] {
            assert!(lane.passes_through());
            assert_eq!(lane.policy(&lanes), LanePolicy::default());
        }
        assert!(!Lane::Bulk.passes_through());
    }

    #[test]
    fn agent_batch_and_files_keep_their_upstream_and_no_cap() {
        let moved = LanePolicy {
            upstream: "http://elsewhere".into(),
            max_in_flight: 1,
            ..LanePolicy::default()
        };
        let lanes = Lanes {
            bulk: moved.clone(),
            embeddings: moved.clone(),
            meta: moved.clone(),
            internal: moved.clone(),
            ..Lanes::default()
        };
        for lane in [Lane::Agent, Lane::Batch, Lane::Files] {
            let p = lane.policy(&lanes);
            assert_eq!(
                (p.upstream.as_str(), p.max_in_flight),
                ("", 0),
                "{}",
                lane.name()
            );
        }
        assert_eq!(Lane::Bulk.policy(&lanes), moved);
    }

    #[test]
    fn only_internal_and_bulk_can_be_routed_and_only_when_enabled() {
        let rule = RouteRule {
            from: vec!["big".into()],
            to: "small".into(),
            gate: "bench".into(),
            ..RouteRule::default()
        };
        let mut routing = RoutingPolicy {
            bulk: rule.clone(),
            internal: rule.clone(),
            ..RoutingPolicy::default()
        };
        for lane in Lane::ALL {
            assert!(lane.route(&routing).is_none(), "{} while off", lane.name());
        }
        routing.enabled = true;
        for lane in Lane::ALL {
            let routable = matches!(lane, Lane::Bulk | Lane::Internal);
            assert_eq!(
                lane.route(&routing) == Some(&rule),
                routable,
                "{}",
                lane.name()
            );
        }
    }

    #[test]
    fn every_kind_reads_back_as_its_lane() {
        for lane in Lane::ALL {
            assert_eq!(lane_of_kind(lane.kind()), lane.name());
        }
    }

    #[test]
    fn kinds_stay_api_requests() {
        for lane in Lane::ALL {
            assert!(is_api_request(lane.kind()), "{}", lane.name());
        }
        assert!(!is_api_request("hook"));
        assert!(!is_api_request("api_requests"));
    }
}
