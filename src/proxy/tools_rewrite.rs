// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Opt-in `tools[]` description rewrite (T59.5), plus schema prose compaction (T686). Off: the
//! request is untouched.

use serde_json::Value;

use crate::config::{Estimator, ToolsRewrite};
use crate::tokens::{self, Class};

/// Before/after description bytes and estimates for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Delta {
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub est_before: u32,
    pub est_after: u32,
    pub changed: bool,
    /// The `tools[]` array as received, serialized, when any of it changed (T686): the caller
    /// archives it so `expand <id>` returns the full catalog.
    pub original: Option<String>,
}

/// Truncate descriptions and drop allow/deny names. `None` if `tools[]` is absent.
/// `input_schema` / `parameters` is written only with `compact_schema`, and then only its
/// `description`, `default` and `examples` strings (see [`compact_schema`]). Unknown tool
/// objects are left as-is.
pub(super) fn rewrite(body: &mut Value, cfg: &ToolsRewrite, est: &Estimator) -> Option<Delta> {
    if !cfg.enabled {
        return None;
    }
    let tools = body.get_mut("tools")?.as_array_mut()?;
    let snapshot = tools.clone();
    let mut before_bytes = 0u64;
    let mut after_bytes = 0u64;
    let mut est_before = 0u32;
    let mut est_after = 0u32;
    let mut changed = false;
    let mut i = 0;
    while i < tools.len() {
        let Some(name) = tool_name(&tools[i]).map(str::to_string) else {
            i += 1;
            continue;
        };
        let drop = cfg.deny.iter().any(|d| d == &name)
            || (!cfg.allow.is_empty() && !cfg.allow.iter().any(|a| a == &name));
        let (b, eb) = tool_stats(&tools[i], cfg.compact_schema, est);
        before_bytes += b;
        est_before += eb;
        if drop {
            tools.remove(i);
            changed = true;
            continue;
        }
        if cfg.max_description_tokens > 0
            && let Some(desc) = tool_desc_mut(&mut tools[i])
        {
            let trimmed = truncate_at_sentence(desc, cfg.max_description_tokens, est);
            if trimmed != *desc {
                *desc = trimmed;
                changed = true;
            }
        }
        if cfg.compact_schema && compact_tool_schema(&mut tools[i]) {
            changed = true;
        }
        let (a, ea) = tool_stats(&tools[i], cfg.compact_schema, est);
        after_bytes += a;
        est_after += ea;
        i += 1;
    }
    Some(Delta {
        before_bytes,
        after_bytes,
        est_before,
        est_after,
        changed,
        original: if changed {
            serde_json::to_string(&snapshot).ok()
        } else {
            None
        },
    })
}

fn tool_name(tool: &Value) -> Option<&str> {
    tool.get("name")
        .and_then(Value::as_str)
        .or_else(|| tool.pointer("/function/name").and_then(Value::as_str))
}

fn tool_desc_mut(tool: &mut Value) -> Option<&mut String> {
    let path = if matches!(tool.get("description"), Some(Value::String(_))) {
        "/description"
    } else {
        "/function/description"
    };
    match tool.pointer_mut(path) {
        Some(Value::String(s)) => Some(s),
        _ => None,
    }
}

/// What the rewrite can shrink: the description, or the whole tool once the schema is in play.
fn tool_stats(tool: &Value, whole: bool, est: &Estimator) -> (u64, u32) {
    if whole {
        let json = tool.to_string();
        return (json.len() as u64, tokens::estimate(&json, Class::Json, est));
    }
    let d = tool
        .get("description")
        .and_then(Value::as_str)
        .or_else(|| {
            tool.pointer("/function/description")
                .and_then(Value::as_str)
        })
        .unwrap_or("");
    (d.len() as u64, tokens::estimate(d, Class::Prose, est))
}

const SCHEMA_PATHS: [&str; 3] = ["/input_schema", "/parameters", "/function/parameters"];
/// Keywords whose value maps names to subschemas. Matching by keyword keeps a property that is
/// itself called `description` or `default` out of reach.
const SCHEMA_MAPS: [&str; 4] = ["properties", "$defs", "definitions", "patternProperties"];
const SCHEMA_CHILDREN: [&str; 9] = [
    "items",
    "additionalProperties",
    "not",
    "if",
    "then",
    "else",
    "contains",
    "propertyNames",
    "unevaluatedProperties",
];
const SCHEMA_LISTS: [&str; 4] = ["anyOf", "oneOf", "allOf", "prefixItems"];

fn compact_tool_schema(tool: &mut Value) -> bool {
    let mut changed = false;
    for path in SCHEMA_PATHS {
        if let Some(schema) = tool.pointer_mut(path) {
            changed |= compact_schema(schema);
        }
    }
    changed
}

/// T686: first sentence of every `description`, and `default` / `examples` that only restate
/// the schema. Names, `type`, `required` and `enum` are never written.
fn compact_schema(node: &mut Value) -> bool {
    match node {
        Value::Array(list) => list.iter_mut().fold(false, |c, v| compact_schema(v) | c),
        Value::Object(m) => {
            let mut changed = false;
            if let Some(Value::String(d)) = m.get_mut("description")
                && let Some(first) = sentences(d).first().map(|s| s.trim())
                && !first.is_empty()
                && first != d.as_str()
            {
                *d = first.to_string();
                changed = true;
            }
            changed |= drop_restated(m);
            for key in SCHEMA_MAPS {
                if let Some(Value::Object(children)) = m.get_mut(key) {
                    changed |= children
                        .values_mut()
                        .fold(false, |c, v| compact_schema(v) | c);
                }
            }
            for key in SCHEMA_CHILDREN.into_iter().chain(SCHEMA_LISTS) {
                if let Some(child) = m.get_mut(key) {
                    changed |= compact_schema(child);
                }
            }
            changed
        }
        _ => false,
    }
}

/// Drop `examples` that all repeat `default` or an `enum` member, then a `default` that is the
/// type's zero value. Narrow on purpose: anything else may carry information the model uses.
fn drop_restated(m: &mut serde_json::Map<String, Value>) -> bool {
    let restated_examples = match m.get("examples") {
        Some(Value::Array(ex)) => {
            let default = m.get("default");
            let members = m.get("enum").and_then(Value::as_array);
            ex.iter()
                .all(|e| default == Some(e) || members.is_some_and(|ms| ms.contains(e)))
        }
        _ => false,
    };
    let zero_default = match (m.get("type").and_then(Value::as_str), m.get("default")) {
        (Some("boolean"), Some(Value::Bool(false))) => true,
        (Some("integer" | "number"), Some(Value::Number(n))) => n.as_f64() == Some(0.0),
        (Some("string"), Some(Value::String(s))) => s.is_empty(),
        (Some("array"), Some(Value::Array(a))) => a.is_empty(),
        (Some("object"), Some(Value::Object(o))) => o.is_empty(),
        _ => false,
    };
    if restated_examples {
        m.remove("examples");
    }
    if zero_default {
        m.remove("default");
    }
    restated_examples || zero_default
}

fn truncate_at_sentence(text: &str, max_tokens: u32, est: &Estimator) -> String {
    if tokens::estimate(text, Class::Prose, est) <= max_tokens {
        return text.to_string();
    }
    let mut out = String::new();
    for sentence in sentences(text) {
        let mut cand = out.clone();
        cand.push_str(sentence);
        if tokens::estimate(cand.trim(), Class::Prose, est) <= max_tokens {
            out = cand;
        } else if out.is_empty() {
            return prefix_tokens(text, max_tokens, est);
        } else {
            break;
        }
    }
    out.trim().to_string()
}

fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if matches!(b[i], b'.' | b'!' | b'?')
            && (i + 1 == b.len() || b[i + 1].is_ascii_whitespace())
            && !continues_sentence(text, start, i)
        {
            let end = i + 1;
            out.push(&text[start..end]);
            start = end;
        }
        i += 1;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// A full stop after `e.g.` / `i.e.` / `vs.` or before a lowercase word is not a sentence end:
/// cutting there would leave a dangling "e.g." (T686 relies on this for first sentences).
fn continues_sentence(text: &str, start: usize, dot: usize) -> bool {
    let word = text[start..dot]
        .rsplit(char::is_whitespace)
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(word.as_str(), "e.g" | "i.e" | "vs" | "cf") {
        return true;
    }
    text[dot + 1..]
        .trim_start()
        .chars()
        .next()
        .is_some_and(char::is_lowercase)
}

fn prefix_tokens(text: &str, max_tokens: u32, est: &Estimator) -> String {
    let mut prefix = String::new();
    for c in text.chars() {
        prefix.push(c);
        if tokens::estimate(&prefix, Class::Prose, est) > max_tokens {
            prefix.pop();
            break;
        }
    }
    prefix
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg(max: u32, allow: &[&str], deny: &[&str]) -> ToolsRewrite {
        ToolsRewrite {
            enabled: true,
            compact_schema: false,
            max_description_tokens: max,
            allow: allow.iter().map(|s| (*s).to_string()).collect(),
            deny: deny.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    fn est() -> Estimator {
        Estimator::default()
    }

    #[test]
    fn anthropic_truncates_description_not_schema() {
        let schema = json!({"type": "object", "properties": {"command": {"type": "string"}}});
        let mut body = json!({
            "tools": [{
                "name": "Bash",
                "description": "Short one. This second sentence is far too long to keep under the cap.",
                "input_schema": schema,
            }],
            "messages": [{"role": "user", "content": "hi"}],
        });
        let d = rewrite(&mut body, &cfg(5, &[], &[]), &est()).unwrap();
        assert!(d.changed && d.after_bytes < d.before_bytes);
        assert_eq!(body["tools"][0]["description"], "Short one.");
        assert_eq!(body["tools"][0]["input_schema"], schema);
        assert_eq!(body["messages"][0]["content"], "hi");
        let again = body.clone();
        rewrite(&mut body, &cfg(5, &[], &[]), &est()).unwrap();
        assert_eq!(body, again);
    }

    #[test]
    fn openai_chat_truncates_function_description_not_parameters() {
        let params = json!({"type": "object", "properties": {"q": {"type": "string"}}});
        let mut body = json!({
            "tools": [{
                "type": "function",
                "function": {
                    "name": "search",
                    "description": "Short one. This second sentence is far too long to keep under the cap.",
                    "parameters": params,
                }
            }],
        });
        let d = rewrite(&mut body, &cfg(5, &[], &[]), &est()).unwrap();
        assert!(d.changed);
        assert_eq!(body["tools"][0]["function"]["description"], "Short one.");
        assert_eq!(body["tools"][0]["function"]["parameters"], params);
    }

    #[test]
    fn deny_drops_tools_but_not_a_later_call() {
        let mut body = json!({
            "tools": [
                {"name": "keep", "description": "Stay.", "input_schema": {"type": "object"}},
                {"name": "drop_me", "description": "Gone.", "input_schema": {"type": "object"}},
            ],
            "messages": [{
                "role": "assistant",
                "content": [{"type": "tool_use", "id": "t1", "name": "drop_me", "input": {}}],
            }],
        });
        let d = rewrite(&mut body, &cfg(0, &[], &["drop_me"]), &est()).unwrap();
        assert!(d.changed);
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(body["tools"][0]["name"], "keep");
        assert_eq!(body["messages"][0]["content"][0]["name"], "drop_me");
    }

    #[test]
    fn off_or_missing_tools_is_a_no_op() {
        let mut body = json!({"tools": [{"name": "Bash", "description": "Hi there."}]});
        let mut off = cfg(5, &[], &[]);
        off.enabled = false;
        assert!(rewrite(&mut body, &off, &est()).is_none());
        assert_eq!(body["tools"][0]["description"], "Hi there.");
        let mut empty = json!({"messages": []});
        assert!(rewrite(&mut empty, &cfg(5, &[], &[]), &est()).is_none());
    }

    fn compact_cfg() -> ToolsRewrite {
        ToolsRewrite {
            compact_schema: true,
            ..cfg(60, &[], &[])
        }
    }

    /// `description`, `default` and `examples` are the only keys T686 may rewrite.
    fn without_prose(v: &Value) -> Value {
        match v {
            Value::Object(m) => Value::Object(
                m.iter()
                    .filter(|(k, _)| !matches!(k.as_str(), "description" | "default" | "examples"))
                    .map(|(k, v)| (k.clone(), without_prose(v)))
                    .collect(),
            ),
            Value::Array(a) => Value::Array(a.iter().map(without_prose).collect()),
            other => other.clone(),
        }
    }

    fn catalog_tokens(body: &Value) -> u32 {
        tokens::estimate(&body["tools"].to_string(), Class::Json, &est())
    }

    /// T686 gate: a realistic 12-tool catalog shrinks by at least 30 % of its tokens while every
    /// tool name, property name, type, `required` list and `enum` stays as it was, and the
    /// original is recoverable from `Delta::original`.
    #[test]
    fn compact_schema_cuts_a_realistic_catalog_by_30_percent_and_keeps_the_shape() {
        let original: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/mcp_catalog_github.json"))
                .unwrap();
        assert!(original["tools"].as_array().unwrap().len() >= 10);
        let mut body = original.clone();
        let d = rewrite(&mut body, &compact_cfg(), &est()).unwrap();
        assert!(d.changed);

        let (before, after) = (catalog_tokens(&original), catalog_tokens(&body));
        let cut = 100.0 * f64::from(before - after) / f64::from(before);
        assert!(
            cut >= 30.0,
            "{before} -> {after} tokens, cut {cut:.1} % < 30 %"
        );

        for (old, new) in original["tools"]
            .as_array()
            .unwrap()
            .iter()
            .zip(body["tools"].as_array().unwrap())
        {
            assert_eq!(old["name"], new["name"]);
            assert_eq!(
                old["input_schema"]["required"],
                new["input_schema"]["required"]
            );
            assert_eq!(without_prose(old), without_prose(new), "{}", old["name"]);
            let props = old["input_schema"]["properties"].as_object().unwrap();
            for (name, prop) in props {
                let now = &new["input_schema"]["properties"][name];
                assert_eq!(prop["type"], now["type"], "{name}");
                assert_eq!(prop.get("enum"), now.get("enum"), "{name}");
            }
        }

        let recovered: Value = serde_json::from_str(d.original.as_deref().unwrap()).unwrap();
        assert_eq!(recovered, original["tools"]);
    }

    #[test]
    fn compact_schema_is_byte_stable_and_idempotent() {
        let original: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/mcp_catalog_github.json"))
                .unwrap();
        let (mut a, mut b) = (original.clone(), original);
        rewrite(&mut a, &compact_cfg(), &est()).unwrap();
        rewrite(&mut b, &compact_cfg(), &est()).unwrap();
        assert_eq!(
            serde_json::to_vec(&a).unwrap(),
            serde_json::to_vec(&b).unwrap()
        );
        let again = rewrite(&mut b, &compact_cfg(), &est()).unwrap();
        assert!(!again.changed && again.original.is_none());
        assert_eq!(a, b);
    }

    #[test]
    fn compact_schema_off_leaves_input_schema_alone() {
        let original: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/mcp_catalog_github.json"))
                .unwrap();
        let mut body = original.clone();
        rewrite(&mut body, &cfg(60, &[], &[]), &est()).unwrap();
        for (old, new) in original["tools"]
            .as_array()
            .unwrap()
            .iter()
            .zip(body["tools"].as_array().unwrap())
        {
            assert_eq!(old["input_schema"], new["input_schema"]);
        }
    }

    #[test]
    fn property_descriptions_cut_to_first_sentence_recursively() {
        let mut body = json!({"tools": [{"name": "t", "description": "Hi.", "input_schema": {
        "type": "object",
        "description": "Root. Second.",
        "properties": {
            "a": {"type": "string", "description": "Path, e.g. /tmp/x. Never absolute."},
            "b": {"type": "array", "description": "List. More.",
                  "items": {"type": "object", "description": "Item. More.",
                            "properties": {"c": {"type": "string", "description": "Deep. Deeper."}}}},
            "d": {"anyOf": [{"type": "string", "description": "One. Two."}, {"type": "null"}]},
            "plain": {"type": "string", "description": "No terminator at all"}
        }}}]});
        let d = rewrite(&mut body, &compact_cfg(), &est()).unwrap();
        assert!(d.changed);
        let s = &body["tools"][0]["input_schema"];
        assert_eq!(s["description"], "Root.");
        assert_eq!(s["properties"]["a"]["description"], "Path, e.g. /tmp/x.");
        assert_eq!(s["properties"]["b"]["description"], "List.");
        assert_eq!(s["properties"]["b"]["items"]["description"], "Item.");
        assert_eq!(
            s["properties"]["b"]["items"]["properties"]["c"]["description"],
            "Deep."
        );
        assert_eq!(s["properties"]["d"]["anyOf"][0]["description"], "One.");
        assert_eq!(
            s["properties"]["plain"]["description"],
            "No terminator at all"
        );
    }

    #[test]
    fn zero_defaults_and_restating_examples_are_dropped_nothing_else() {
        let mut body = json!({"tools": [{"name": "t", "description": "Hi.", "input_schema": {
        "type": "object",
        "properties": {
            "flag": {"type": "boolean", "default": false},
            "n": {"type": "integer", "default": 0},
            "s": {"type": "string", "default": ""},
            "l": {"type": "array", "default": []},
            "kept_n": {"type": "integer", "default": 30},
            "kept_flag": {"type": "boolean", "default": true},
            "no_type": {"default": 0},
            "mode": {"type": "string", "enum": ["a", "b"], "default": "a", "examples": ["a", "b"]},
            "echo": {"type": "string", "default": "x", "examples": ["x"]},
            "real": {"type": "string", "examples": ["org:github"]},
            "mixed": {"type": "string", "enum": ["a", "b"], "examples": ["a", "zzz"]}
        }}}]});
        rewrite(&mut body, &compact_cfg(), &est()).unwrap();
        let p = &body["tools"][0]["input_schema"]["properties"];
        for gone in ["flag", "n", "s", "l"] {
            assert!(p[gone].get("default").is_none(), "{gone}");
        }
        assert_eq!(p["kept_n"]["default"], 30);
        assert_eq!(p["kept_flag"]["default"], true);
        assert_eq!(p["no_type"]["default"], 0);
        assert_eq!(p["mode"]["default"], "a");
        assert!(p["mode"].get("examples").is_none());
        assert_eq!(p["mode"]["enum"], json!(["a", "b"]));
        assert_eq!(p["echo"]["default"], "x");
        assert!(p["echo"].get("examples").is_none());
        assert_eq!(p["real"]["examples"], json!(["org:github"]));
        assert_eq!(p["mixed"]["examples"], json!(["a", "zzz"]));
    }

    #[test]
    fn properties_named_like_keywords_are_not_rewritten() {
        let schema = json!({"type": "object", "properties": {
            "description": {"type": "string"},
            "default": {"type": "boolean"},
            "examples": {"type": "array", "items": {"type": "string"}}
        }, "required": ["description"]});
        let mut body =
            json!({"tools": [{"name": "t", "description": "Hi.", "input_schema": schema}]});
        let d = rewrite(&mut body, &compact_cfg(), &est()).unwrap();
        assert!(!d.changed && d.original.is_none());
        assert_eq!(body["tools"][0]["input_schema"], schema);
    }

    #[test]
    fn compact_schema_covers_openai_function_parameters() {
        let mut body = json!({"tools": [{"type": "function", "function": {
            "name": "f", "description": "Hi.",
            "parameters": {"type": "object", "properties": {
                "q": {"type": "string", "description": "Query. Extra words.", "default": ""}}}}}]});
        let d = rewrite(&mut body, &compact_cfg(), &est()).unwrap();
        assert!(d.changed);
        let q = &body["tools"][0]["function"]["parameters"]["properties"]["q"];
        assert_eq!(q["description"], "Query.");
        assert!(q.get("default").is_none());
    }

    /// T59.5 truncation and `deny` lose text too: their original is recoverable as well.
    #[test]
    fn any_change_carries_the_original_tools() {
        let mut body = json!({"tools": [
            {"name": "keep", "description": "Short one. This second sentence is far too long to keep under the cap."},
            {"name": "gone", "description": "Gone."}]});
        let before = body["tools"].clone();
        let d = rewrite(&mut body, &cfg(5, &[], &["gone"]), &est()).unwrap();
        let recovered: Value = serde_json::from_str(d.original.as_deref().unwrap()).unwrap();
        assert_eq!(recovered, before);
        let mut same = json!({"tools": [{"name": "k", "description": "Hi."}]});
        assert!(
            rewrite(&mut same, &cfg(5, &[], &[]), &est())
                .unwrap()
                .original
                .is_none()
        );
    }

    /// Savings bench (`bench/token_savings.sh`): the shipped defaults rewrite the fixture
    /// catalog to at most 70 % of its tokens, deterministically, so the prompt prefix is
    /// the same on every request and the change is invisible to whatever model is behind it.
    #[test]
    #[allow(clippy::print_stderr)]
    fn savings_default_config_shrinks_the_fixture_catalog() {
        let cfg = ToolsRewrite::default();
        assert!(cfg.enabled && cfg.compact_schema, "tools_rewrite ships on");
        let original: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/mcp_catalog_github.json"))
                .unwrap();
        let mut once = original.clone();
        let d = rewrite(&mut once, &cfg, &est()).unwrap();
        let mut twice = original.clone();
        rewrite(&mut twice, &cfg, &est()).unwrap();
        assert_eq!(once.to_string(), twice.to_string(), "byte-stable prefix");
        let (before, after) = (catalog_tokens(&original), catalog_tokens(&once));
        let (b_bytes, a_bytes) = (
            original["tools"].to_string().len(),
            once["tools"].to_string().len(),
        );
        eprintln!(
            "SAVINGS tools_rewrite: {before} -> {after} est. tokens, {b_bytes} -> {a_bytes} bytes"
        );
        assert!(after * 100 <= before * 70, "{before} -> {after}");
        assert!(d.after_bytes < d.before_bytes);
    }

    /// Prompt-cache friendliness: a `cache_control` breakpoint on the last tool survives the
    /// rewrite untouched, and two independent rewrites of the same catalog (with the marker)
    /// produce byte-identical `tools[]`, so the provider cache prefix stays stable across calls.
    #[test]
    fn cache_control_marker_survives_and_prefix_is_byte_stable() {
        let cfg = ToolsRewrite::default();
        let mut original: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/mcp_catalog_github.json"))
                .unwrap();
        let marker = serde_json::json!({"type": "ephemeral"});
        let last = original["tools"].as_array().unwrap().len() - 1;
        original["tools"][last]["cache_control"] = marker.clone();
        let outs: Vec<String> = (0..3)
            .map(|_| {
                let mut b = original.clone();
                rewrite(&mut b, &cfg, &est()).unwrap();
                assert_eq!(b["tools"][last]["cache_control"], marker, "marker kept");
                let marked = b["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|t| t.get("cache_control").is_some())
                    .count();
                assert_eq!(marked, 1, "no marker added or moved");
                b["tools"].to_string()
            })
            .collect();
        assert!(outs.windows(2).all(|w| w[0] == w[1]), "byte-identical");
        // Rewriting an already rewritten catalog is a no-op on bytes (idempotent prefix).
        let mut again: Value =
            serde_json::json!({"tools": serde_json::from_str::<Value>(&outs[0]).unwrap()});
        let _ = rewrite(&mut again, &cfg, &est());
        assert_eq!(again["tools"].to_string(), outs[0], "idempotent");
    }
}
