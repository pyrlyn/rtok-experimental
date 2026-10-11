// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Model routing for the `internal` and `bulk` lanes (T385.10, D9, I-101).
//!
//! A rule swaps the request's `model` for a cheaper one before the request is recorded, so the
//! `calls` and `usage` rows name the model that served it and `rtok stats --price` costs the
//! lane at the price it was really billed. Nothing else about the body changes.
//!
//! The quality gate of I-101 cannot be run from here: whether the cheaper model answers well
//! enough is a bench result, not something a proxy can see. The mechanism is therefore closed
//! until the operator opts in and names that bench (`gate`), and it is closed to the `agent`
//! lane for good (`Lane::route`). rtok ships no rule and claims no saving from routing; the
//! dollars come from the `usage` rows of a run with and without the rule.
//!
//! Only wires that carry `model` in the JSON body are routed. Gemini names the model in the
//! path, which this step does not rewrite, so a Gemini call never matches.

use axum::body::Bytes;
use serde_json::Value;

use crate::config::RouteRule;

/// What a rule changed, for the ledger.
#[derive(Debug, PartialEq, Eq)]
pub struct Routed {
    pub body: Bytes,
    pub from: String,
    pub to: String,
    /// The provider slug to record, when the rule names one.
    pub provider: Option<String>,
}

/// A rule acts only when it names what it matches, where it goes and the bench behind it.
fn active(rule: &RouteRule) -> bool {
    !rule.from.is_empty() && !rule.to.trim().is_empty() && !rule.gate.trim().is_empty()
}

/// An exact id, or a prefix when the pattern ends in `*`.
fn matches(pattern: &str, model: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => model.starts_with(prefix),
        None => pattern == model,
    }
}

/// Re-point `body` at the rule's model, or `None` to forward it as it is: the rule is not
/// active, the body is not a JSON object naming one of `from`, or it already names `to`.
pub fn apply(rule: &RouteRule, body: &Bytes) -> Option<Routed> {
    if !active(rule) {
        return None;
    }
    let mut parsed: Value = serde_json::from_slice(body).ok()?;
    let from = parsed.get("model")?.as_str()?.to_string();
    let to = rule.to.trim();
    if from == to || !rule.from.iter().any(|p| matches(p, &from)) {
        return None;
    }
    parsed["model"] = to.into();
    Some(Routed {
        body: Bytes::from(serde_json::to_vec(&parsed).ok()?),
        from,
        to: to.to_string(),
        provider: Some(rule.provider.trim().to_string()).filter(|p| !p.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(from: &[&str], to: &str, gate: &str) -> RouteRule {
        RouteRule {
            from: from.iter().map(|s| s.to_string()).collect(),
            to: to.into(),
            gate: gate.into(),
            ..RouteRule::default()
        }
    }

    fn body(model: &str) -> Bytes {
        Bytes::from(format!(r#"{{"model":"{model}","messages":[]}}"#))
    }

    #[test]
    fn a_matching_model_is_swapped_and_nothing_else() {
        let r = apply(&rule(&["big"], "small", "bench-1"), &body("big")).unwrap();
        assert_eq!((r.from.as_str(), r.to.as_str()), ("big", "small"));
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v, serde_json::json!({"model": "small", "messages": []}));
        assert_eq!(r.provider, None);
    }

    #[test]
    fn a_star_matches_a_prefix_and_no_star_matches_exactly() {
        let r = rule(&["claude-opus-*", "exact"], "cheap", "g");
        assert!(apply(&r, &body("claude-opus-4-1-20250805")).is_some());
        assert!(apply(&r, &body("exact")).is_some());
        assert!(apply(&r, &body("exact-2")).is_none());
        assert!(apply(&r, &body("claude-sonnet-4")).is_none());
    }

    #[test]
    fn a_rule_missing_a_part_never_fires() {
        let b = body("big");
        assert!(apply(&rule(&[], "small", "g"), &b).is_none());
        assert!(apply(&rule(&["big"], "", "g"), &b).is_none());
        // No named bench, no routing: the quality gate is not rtok's to waive.
        assert!(apply(&rule(&["big"], "small", " "), &b).is_none());
    }

    #[test]
    fn bodies_without_a_model_or_json_pass_and_so_does_the_target_itself() {
        let r = rule(&["*"], "small", "g");
        assert!(apply(&r, &Bytes::from_static(b"{\"messages\":[]}")).is_none());
        assert!(apply(&r, &Bytes::from_static(b"not json")).is_none());
        assert!(apply(&r, &Bytes::from_static(b"{\"model\":7}")).is_none());
        assert!(apply(&r, &body("small")).is_none());
    }

    #[test]
    fn the_provider_is_reported_only_when_named() {
        let mut r = rule(&["big"], "small", "g");
        r.provider = " openrouter ".into();
        assert_eq!(
            apply(&r, &body("big")).unwrap().provider.as_deref(),
            Some("openrouter")
        );
    }
}
