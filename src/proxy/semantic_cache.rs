// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Opt-in two-tier response cache (P31). Off when `enabled = false` or `embed_backend = "hash"`
//! (direct tier only). See `src/plugins/proxy/PLAN.md` v0.2.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use rtok_plugin_sdk::Measurement;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::config::SemanticCache;
use crate::proxy::wire::Wire;

#[derive(Debug, Clone, Serialize)]
pub struct CachePrompt {
    pub provider: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<(String, String)>,
    /// Digest of each whole message and of the whole `system` (T532). `messages` and `system`
    /// above are flattened text for the embedding; flattening drops `is_error`, `name`,
    /// `image_url.detail` and the block type, so the direct key covers the typed JSON too.
    pub typed: Vec<TypedMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_typed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools_fingerprint: Option<String>,
    /// Every other top-level request field (T212): `max_tokens`, `temperature`, `top_p`,
    /// `tool_choice`, `thinking`, stop sequences, and anything else neither excluded nor
    /// already covered by its own field above. Canonicalized (see `canonical_json`) so
    /// key order alone never splits the cache.
    pub params: Value,
    /// Who is asking (T323): [`caller_identity`] of the request headers that carry the
    /// credential or change the response. Empty only for a prompt built outside the proxy
    /// (the corpus audit), so two API keys never share an entry.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub caller: String,
}

/// `full` is the message's digest; `shape` is the same digest with its text blanked, which is
/// what the semantic tier may keep for the last user message: that text is the part it
/// compares fuzzily, while an image's `detail` or a block's type still has to match.
#[derive(Debug, Clone, Serialize)]
pub struct TypedMessage {
    full: String,
    shape: String,
}

pub struct CacheHit {
    pub response: Bytes,
    pub content_type: Option<String>,
    pub status: u16,
    pub similarity: f32,
    pub direct: bool,
    pub hash_hex: String,
}

struct Entry {
    hash: [u8; 32],
    response: Bytes,
    content_type: Option<String>,
    status: u16,
    at: Instant,
}

pub struct Cache {
    direct: HashMap<[u8; 32], Entry>,
    /// Vector of the last user message, [`scope_hash`] of the rest, entry.
    semantic: Vec<(Vec<f32>, [u8; 32], Entry)>,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

impl Cache {
    pub fn new() -> Self {
        Self {
            direct: HashMap::new(),
            semantic: Vec::new(),
        }
    }

    pub fn lookup(&self, prompt: &CachePrompt, cfg: &SemanticCache) -> Option<CacheHit> {
        let hash = canonical_hash(prompt);
        let ttl = Duration::from_secs(cfg.ttl_s.max(1));
        if let Some(e) = self.direct.get(&hash).filter(|e| e.at.elapsed() < ttl) {
            return Some(hit_from(e, 1.0, true, &hash));
        }
        if cfg.embed_backend == "hash" {
            return None;
        }
        let query = embed_vector(prompt);
        let scope = scope_hash(prompt);
        let mut best: Option<(f32, &Entry)> = None;
        for (emb, s, e) in &self.semantic {
            if e.at.elapsed() >= ttl || *s != scope {
                continue;
            }
            let sim = cosine(emb, &query);
            if sim >= cfg.threshold && best.is_none_or(|(s, _)| sim > s) {
                best = Some((sim, e));
            }
        }
        best.map(|(sim, e)| hit_from(e, sim, false, &e.hash))
    }

    pub fn store(
        &mut self,
        prompt: &CachePrompt,
        cfg: &SemanticCache,
        response: &[u8],
        content_type: Option<&str>,
        status: u16,
    ) {
        // A 429/529/500 replayed for `ttl_s` is an outage the cache would prolong.
        if !(200..300).contains(&status) {
            return;
        }
        let hash = canonical_hash(prompt);
        // `lookup` only skips expired entries, so without this a long-running proxy kept every
        // missed prompt's full response forever (and scanned all of them per request). The
        // same prompt stored again replaces its semantic row instead of stacking a second one.
        let ttl = Duration::from_secs(cfg.ttl_s.max(1));
        self.direct.retain(|_, e| e.at.elapsed() < ttl);
        self.semantic
            .retain(|(_, _, e)| e.hash != hash && e.at.elapsed() < ttl);
        let at = Instant::now();
        let response = Bytes::copy_from_slice(response);
        let content_type = content_type.map(str::to_string);
        let embedding = (cfg.embed_backend != "hash").then(|| embed_vector(prompt));
        if let Some(emb) = embedding {
            self.semantic.push((
                emb,
                scope_hash(prompt),
                Entry {
                    hash,
                    response: response.clone(),
                    content_type: content_type.clone(),
                    status,
                    at,
                },
            ));
        }
        self.direct.insert(
            hash,
            Entry {
                hash,
                response,
                content_type,
                status,
                at,
            },
        );
    }
}

fn hit_from(e: &Entry, similarity: f32, direct: bool, hash: &[u8; 32]) -> CacheHit {
    CacheHit {
        response: e.response.clone(),
        content_type: e.content_type.clone(),
        status: e.status,
        similarity,
        direct,
        hash_hex: hex8(hash),
    }
}

pub fn build_prompt(wire: &dyn Wire, body: &Value, cfg: &SemanticCache) -> Option<CachePrompt> {
    let model = body.get("model")?.as_str()?.to_string();
    Some(CachePrompt {
        provider: if cfg.cache_by_provider {
            wire.provider().to_string()
        } else {
            String::new()
        },
        model: if cfg.cache_by_model {
            model
        } else {
            Default::default()
        },
        system: system_text(body),
        messages: messages_text(body),
        typed: typed_messages(body),
        system_typed: body.get("system").map(|s| digest(s, false)),
        tools_fingerprint: tools_fingerprint(body),
        params: extra_params(body),
        caller: String::new(),
    })
}

impl CachePrompt {
    /// The same prompt asked by `caller` (see [`caller_identity`]).
    pub fn with_caller(mut self, caller: &str) -> Self {
        self.caller = caller.to_string();
        self
    }
}

/// Request headers that identify the caller or change the response for the same body.
/// `openai-project` picks the project a key bills and whose data controls apply, so two
/// projects sharing an org key must not share answers.
const CALLER_HEADERS: [&str; 7] = [
    "x-api-key",
    "authorization",
    "x-goog-api-key",
    "anthropic-version",
    "anthropic-beta",
    "openai-organization",
    "openai-project",
];

/// sha256 over the [`CALLER_HEADERS`] values (T323): the raw secret never reaches the
/// cache key. Lookup and store both call this on the client's own headers, so they agree.
pub fn caller_identity(headers: &axum::http::HeaderMap) -> String {
    let mut h = Sha256::new();
    for name in CALLER_HEADERS {
        h.update(name.as_bytes());
        for value in headers.get_all(name) {
            h.update([0]);
            h.update(value.as_bytes());
        }
        h.update([1]);
    }
    let digest: [u8; 32] = h.finalize().into();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// [`caller_identity`] plus the request's method and resolved upstream URL (T533), hashed so
/// a credential-bearing query string (Gemini's `?key=`) never reaches the key as text.
pub fn cache_scope(headers: &axum::http::HeaderMap, method: &str, target: &str) -> String {
    let mut h = Sha256::new();
    h.update(caller_identity(headers).as_bytes());
    for part in [method, target] {
        h.update([0]);
        h.update(part.as_bytes());
    }
    let digest: [u8; 32] = h.finalize().into();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn eligible(body: &Value, cfg: &SemanticCache) -> bool {
    if body.get("stream") == Some(&Value::Bool(true)) {
        return false;
    }
    let messages = body.get("messages").and_then(Value::as_array);
    let users = messages
        .map(|messages| {
            messages
                .iter()
                .filter(|m| m.get("role") == Some(&Value::String("user".into())))
                .count()
        })
        .unwrap_or(0);
    if users == 0 || users > cfg.max_messages as usize {
        return false;
    }
    if cfg.require_empty_tools
        && let Some(tools) = body.get("tools").and_then(Value::as_array)
        && !tools.is_empty()
    {
        return false;
    }
    true
}

pub fn measurement(hit: &CacheHit, call_id: Option<i32>, nbytes: usize) -> Measurement {
    let est = (nbytes / 4).max(1) as u32;
    Measurement {
        plugin: "proxy",
        kind: rtok_store::otel::SEMANTIC_CACHE_HIT_KIND,
        before_bytes: nbytes as u64,
        after_bytes: nbytes as u64,
        est_before: est,
        est_after: est,
        ref_id: Some(format!("{:.4}:{}", hit.similarity, hit.hash_hex)),
        call_id,
    }
}

#[derive(Debug)]
pub struct AuditReport {
    pub false_hit_pairs: u32,
    pub semantic_pairs: u32,
    pub hit_rate: f32,
}

pub fn audit_corpus(dir: &Path, cfg: &SemanticCache) -> Result<AuditReport, String> {
    let entries = load_corpus(dir)?;
    let n = entries.len();
    let mut false_hits = 0u32;
    let mut semantic = 0u32;
    for i in 0..n {
        for j in 0..n {
            if i == j {
                continue;
            }
            if entries[i].hash == entries[j].hash || entries[i].scope != entries[j].scope {
                continue;
            }
            if cfg.embed_backend == "hash" {
                continue;
            }
            let sim = cosine(&entries[i].embedding, &entries[j].embedding);
            if sim >= cfg.threshold {
                semantic += 1;
                if entries[i].response != entries[j].response {
                    false_hits += 1;
                }
            }
        }
    }
    // `n - 1` underflowed (a debug-build panic) on an empty corpus.
    let pairs = n * n.saturating_sub(1);
    Ok(AuditReport {
        false_hit_pairs: false_hits,
        semantic_pairs: semantic,
        hit_rate: if pairs == 0 {
            0.0
        } else {
            semantic as f32 / pairs as f32
        },
    })
}

struct CorpusEntry {
    hash: [u8; 32],
    scope: [u8; 32],
    embedding: Vec<f32>,
    response: Vec<u8>,
}

fn load_corpus(dir: &Path) -> Result<Vec<CorpusEntry>, String> {
    let manifest = dir.join("corpus.json");
    let raw = std::fs::read_to_string(&manifest)
        .map_err(|e| format!("read {}: {e}", manifest.display()))?;
    let doc: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let entries = doc
        .get("entries")
        .and_then(|v| v.as_array())
        .ok_or("corpus.json missing entries")?;
    entries
        .iter()
        .map(|row| {
            let id = row.get("id").and_then(|v| v.as_str()).ok_or("entry id")?;
            let req = dir.join(
                row.get("request")
                    .and_then(|v| v.as_str())
                    .ok_or("entry request")?,
            );
            let resp = dir.join(
                row.get("response")
                    .and_then(|v| v.as_str())
                    .ok_or("entry response")?,
            );
            let body: Value = serde_json::from_slice(
                &std::fs::read(&req).map_err(|e| format!("{id} request: {e}"))?,
            )
            .map_err(|e| format!("{id} request json: {e}"))?;
            let wire = crate::proxy::wire::for_path("/v1/messages").expect("wire");
            let cfg = SemanticCache::default();
            let prompt = build_prompt(wire, &body, &cfg).ok_or_else(|| format!("{id} prompt"))?;
            let hash = canonical_hash(&prompt);
            let response = std::fs::read(&resp).map_err(|e| format!("{id} response: {e}"))?;
            let embedding = row
                .get("embedding")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_f64().map(|f| f as f32))
                        .collect()
                })
                .unwrap_or_else(|| embed_vector(&prompt));
            Ok(CorpusEntry {
                hash,
                scope: scope_hash(&prompt),
                embedding,
                response,
            })
        })
        .collect()
}

pub fn canonical_hash(prompt: &CachePrompt) -> [u8; 32] {
    let json = serde_json::to_vec(prompt).unwrap_or_default();
    let mut h = Sha256::new();
    h.update(&json);
    h.finalize().into()
}

fn messages_text(body: &Value) -> Vec<(String, String)> {
    body.get("messages")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let role = m.get("role")?.as_str()?;
                    // A missing `content` (OpenAI Chat: an assistant message that is only
                    // `tool_calls`) is empty text, not a dropped message.
                    let mut text = m.get("content").map(content_text).unwrap_or_default();
                    // OpenAI Chat: an assistant message's `tool_calls` sits beside
                    // `content`, not inside it (T303) — fold it in so two requests
                    // differing only there don't collide.
                    if let Some(calls) = m.get("tool_calls") {
                        text.push_str(" tool_calls ");
                        text.push_str(
                            &serde_json::to_string(&canonical_json(calls)).unwrap_or_default(),
                        );
                    }
                    // OpenAI Chat: a `tool` message's `tool_call_id` (T303).
                    if let Some(id) = m.get("tool_call_id").and_then(Value::as_str) {
                        text.push_str(" tool_call_id ");
                        text.push_str(id);
                    }
                    Some((role.to_string(), text))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn typed_messages(body: &Value) -> Vec<TypedMessage> {
    body.get("messages")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                // Same filter as `messages_text`, so row i of each describes one message.
                .filter(|m| m.get("role").and_then(Value::as_str).is_some())
                .map(|m| TypedMessage {
                    full: digest(m, false),
                    shape: digest(m, true),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// sha256 of the canonical JSON. `cache_control` is dropped because clients move that marker
/// between turns and it does not change the answer; `blank_text` empties every `text` and
/// string `content` so only the structure around the words remains.
fn digest(v: &Value, blank_text: bool) -> String {
    fn walk(v: &Value, blank_text: bool) -> Value {
        match v {
            Value::Object(map) => Value::Object(
                map.iter()
                    .filter(|(k, _)| k.as_str() != "cache_control")
                    .map(|(k, val)| {
                        let blank = blank_text
                            && matches!(k.as_str(), "text" | "content")
                            && val.is_string();
                        let val = if blank {
                            Value::String(String::new())
                        } else {
                            walk(val, blank_text)
                        };
                        (k.clone(), val)
                    })
                    .collect(),
            ),
            Value::Array(items) => {
                Value::Array(items.iter().map(|i| walk(i, blank_text)).collect())
            }
            other => other.clone(),
        }
    }
    let json = serde_json::to_vec(&canonical_json(&walk(v, blank_text))).unwrap_or_default();
    crate::store::hex_sha256(&json)
}

fn system_text(body: &Value) -> Option<String> {
    body.get("system").map(content_text)
}

fn content_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks.iter().map(block_text).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// One content block's contribution to the cache key (T55.14, T303): text as-is; a
/// `tool_result`'s id plus nested content; a `tool_use`'s id, name and canonicalized
/// `input` (so two calls to the same tool with different arguments no longer collide);
/// binary-bearing blocks (the whole image/document `source`, OpenAI `image_url`) contribute
/// their sha256, so payloads never rendered as text still tell two requests apart; any
/// other block kind (`thinking`, `redacted_thinking`, `server_tool_use`,
/// `web_search_tool_result`, …) contributes the whole block canonicalized, so unknown
/// kinds no longer contribute nothing.
fn block_text(b: &Value) -> String {
    if let Some(text) = b.get("text").and_then(Value::as_str) {
        return text.to_string();
    }
    if let Some(s) = b.as_str() {
        return s.to_string();
    }
    let kind = b.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "tool_result" => {
            let id = b.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
            format!(
                "tool_result {id}: {}",
                content_text(b.get("content").unwrap_or(&Value::Null))
            )
        }
        "tool_use" => format!(
            "tool_use {} {} {}",
            b.get("id").and_then(Value::as_str).unwrap_or(""),
            b.get("name").and_then(Value::as_str).unwrap_or(""),
            serde_json::to_string(&canonical_json(b.get("input").unwrap_or(&Value::Null)))
                .unwrap_or_default()
        ),
        "image" | "document" => {
            // The whole `source`, canonicalized (T323): base64 `data`, but also a `url`, a
            // Files API `file_id` or inline `text`/`content`, none of which has `data`.
            let source =
                serde_json::to_string(&canonical_json(b.get("source").unwrap_or(&Value::Null)))
                    .unwrap_or_default();
            format!("{kind} {}", crate::store::hex_sha256(source.as_bytes()))
        }
        "image_url" => {
            let url = b
                .pointer("/image_url/url")
                .and_then(Value::as_str)
                .unwrap_or("");
            format!("image_url {}", crate::store::hex_sha256(url.as_bytes()))
        }
        // Unknown kind (thinking, redacted_thinking, server_tool_use,
        // web_search_tool_result, …): canonicalize the whole block rather than
        // contribute nothing, so it still tells two requests apart.
        _ => serde_json::to_string(&canonical_json(b)).unwrap_or_default(),
    }
}

/// Hashes each tool's full definition (T212), not just its name: Anthropic's
/// `{name, description, input_schema}` and OpenAI Chat's `{type, function: {name,
/// description, parameters}}` alike, so two tools sharing a name but differing in
/// schema no longer collide. Canonicalized per tool so key order does not split it.
fn tools_fingerprint(body: &Value) -> Option<String> {
    let tools = body.get("tools")?.as_array()?;
    if tools.is_empty() {
        return None;
    }
    let mut h = Sha256::new();
    for t in tools {
        h.update(serde_json::to_vec(&canonical_json(t)).unwrap_or_default());
        h.update([0]);
    }
    Some(hex8(&h.finalize().into()))
}

/// Top-level request fields excluded from `extra_params`: `messages` (hashed
/// separately, block-aware), `model`/`system`/`tools` (each already its own
/// `CachePrompt` field, `model` gated by `cache_by_model`), `stream` (requests that
/// set it never reach the cache — see `eligible`), and per-request tracking ids that
/// do not affect generation (Anthropic `metadata.user_id`, OpenAI `user`).
const EXCLUDED_PARAM_FIELDS: [&str; 7] = [
    "messages", "model", "system", "tools", "stream", "metadata", "user",
];

/// Every other top-level request field — `max_tokens`, `temperature`, `top_p`,
/// `tool_choice`, `thinking`, stop sequences, and so on — folded into the cache key
/// (T212), canonicalized so key order alone never splits the cache.
fn extra_params(body: &Value) -> Value {
    let Some(map) = body.as_object() else {
        return Value::Object(Default::default());
    };
    let filtered: serde_json::Map<String, Value> = map
        .iter()
        .filter(|(k, _)| !EXCLUDED_PARAM_FIELDS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    canonical_json(&Value::Object(filtered))
}

/// Recursively sorts object keys (array order is meaningful and left alone) so two
/// JSON payloads differing only in key order hash identically (T212).
fn canonical_json(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let sorted: std::collections::BTreeMap<&String, &Value> = map.iter().collect();
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, val) in sorted {
                out.insert(k.clone(), canonical_json(val));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_json).collect()),
        other => other.clone(),
    }
}

/// Everything but the last user message, hashed. The semantic tier compares only prompts
/// that share it: the vector covers the last user message alone, so the same question under
/// another system prompt, tool set or earlier turn used to get that other prompt's answer.
fn scope_hash(prompt: &CachePrompt) -> [u8; 32] {
    let mut rest = prompt.clone();
    if let Some(i) = rest.messages.iter().rposition(|(r, _)| r == "user") {
        rest.messages[i].1.clear();
        if let Some(t) = rest.typed.get_mut(i) {
            t.full.clear();
        }
    }
    canonical_hash(&rest)
}

/// Words, symbols and adjacent pairs of the last user message, feature-hashed. The old vector
/// summed raw 4-byte chunks into 8 all-positive slots, so unrelated prompts scored far above
/// zero; pairs keep "rename a to b" apart from "rename b to a", symbols keep `<` apart from `>`.
/// Case and spacing still match.
fn embed_vector(prompt: &CachePrompt) -> Vec<f32> {
    let last_user = prompt
        .messages
        .iter()
        .rev()
        .find(|(r, _)| r == "user")
        .map_or("", |(_, t)| t.as_str());
    let mut words = Vec::new();
    let mut word = String::new();
    for c in last_user.chars().chain([' ']) {
        if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
            continue;
        }
        if !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        if !c.is_whitespace() {
            words.push(format!("u{:x}", u32::from(c)));
        }
    }
    let pairs = words.windows(2).map(|w| format!("{}{}", w[0], w[1]));
    let text: Vec<String> = words.iter().cloned().chain(pairs).collect();
    crate::store::embed::hash_embed(&text.join(" "), 256)
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let dot = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum::<f32>();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

fn hex8(hash: &[u8; 32]) -> String {
    hash.iter().take(4).map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Audit 2026-10-19: one org key used from two OpenAI projects shared cached answers.
    #[test]
    fn openai_project_is_part_of_the_caller() {
        let with = |project: &str| {
            let mut h = axum::http::HeaderMap::new();
            h.insert("authorization", "Bearer sk-org".parse().unwrap());
            h.insert("openai-project", project.parse().unwrap());
            caller_identity(&h)
        };
        assert_ne!(with("proj_a"), with("proj_b"));
        assert_eq!(with("proj_a"), with("proj_a"));
    }
    use std::path::PathBuf;

    /// T55.14: tool_result text is part of the key — two requests whose results differ
    /// must not share one cache entry, and the shape stays eligible under defaults.
    #[test]
    fn tool_result_text_joins_the_cache_key() {
        let wire = crate::proxy::wire::for_path("/v1/messages").unwrap();
        let cfg = SemanticCache::default();
        let body = |result: &str| {
            serde_json::json!({
                "model": "m",
                "messages": [
                    {"role": "assistant", "content": [
                        {"type": "tool_use", "id": "t", "name": "run", "input": {}}
                    ]},
                    {"role": "user", "content": [
                        {"type": "tool_result", "tool_use_id": "t", "content": result},
                        {"type": "text", "text": "summarize the result"}
                    ]}
                ]
            })
        };
        let a = body("tests passed: 200 ok");
        let b = body("tests FAILED: 1 broken");
        for v in [&a, &b] {
            assert!(
                eligible(v, &cfg),
                "shape stays cache-eligible under defaults"
            );
        }
        let pa = build_prompt(wire, &a, &cfg).unwrap();
        let pb = build_prompt(wire, &b, &cfg).unwrap();
        assert_ne!(
            canonical_hash(&pa),
            canonical_hash(&pb),
            "different tool results must not share one cache entry"
        );
    }

    /// T55.14: the tool_use id joins the key even when the result text is identical.
    #[test]
    fn tool_use_ids_join_the_cache_key() {
        let wire = crate::proxy::wire::for_path("/v1/messages").unwrap();
        let cfg = SemanticCache::default();
        let body = |id: &str| {
            serde_json::json!({
                "model": "m",
                "messages": [
                    {"role": "assistant", "content": [
                        {"type": "tool_use", "id": id, "name": "run", "input": {}}
                    ]},
                    {"role": "user", "content": [
                        {"type": "tool_result", "tool_use_id": id, "content": "same output"}
                    ]}
                ]
            })
        };
        let pa = build_prompt(wire, &body("t-1"), &cfg).unwrap();
        let pb = build_prompt(wire, &body("t-2"), &cfg).unwrap();
        assert_ne!(canonical_hash(&pa), canonical_hash(&pb));
    }

    /// T303: a `tool_use` block's `input` (the call arguments) joins the key — two
    /// calls to the same tool differing only in arguments must not share one cache
    /// entry. Key order inside `input` alone must not split the cache.
    #[test]
    fn tool_use_input_joins_the_cache_key() {
        let wire = crate::proxy::wire::for_path("/v1/messages").unwrap();
        let cfg = SemanticCache::default();
        let body = |input: Value| {
            serde_json::json!({
                "model": "m",
                "messages": [
                    {"role": "assistant", "content": [
                        {"type": "tool_use", "id": "t", "name": "run", "input": input}
                    ]},
                    {"role": "user", "content": [
                        {"type": "tool_result", "tool_use_id": "t", "content": "same output"}
                    ]}
                ]
            })
        };
        let pa = build_prompt(wire, &body(serde_json::json!({"cmd": "ls"})), &cfg).unwrap();
        let pb = build_prompt(wire, &body(serde_json::json!({"cmd": "rm -rf /"})), &cfg).unwrap();
        assert_ne!(
            canonical_hash(&pa),
            canonical_hash(&pb),
            "different tool_use input must not share one cache entry"
        );

        let pc = build_prompt(wire, &body(serde_json::json!({"a": 1, "b": 2})), &cfg).unwrap();
        let pd = build_prompt(wire, &body(serde_json::json!({"b": 2, "a": 1})), &cfg).unwrap();
        assert_eq!(
            canonical_hash(&pc),
            canonical_hash(&pd),
            "key order inside tool_use input alone must not split the cache"
        );
    }

    /// T303: an unknown block kind (`thinking` here) used to contribute nothing to the
    /// key, so two requests differing only there hashed equal.
    #[test]
    fn thinking_block_joins_the_cache_key() {
        let wire = crate::proxy::wire::for_path("/v1/messages").unwrap();
        let cfg = SemanticCache::default();
        let body = |thinking: &str| {
            serde_json::json!({
                "model": "m",
                "messages": [{"role": "assistant", "content": [
                    {"type": "thinking", "thinking": thinking, "signature": "sig"},
                    {"type": "text", "text": "the answer"}
                ]}]
            })
        };
        let pa = build_prompt(wire, &body("maybe it's 4"), &cfg).unwrap();
        let pb = build_prompt(wire, &body("maybe it's 5"), &cfg).unwrap();
        assert_ne!(canonical_hash(&pa), canonical_hash(&pb));
    }

    /// T303: OpenAI Chat's assistant `tool_calls` sits beside `content`, not inside it,
    /// and used to be dropped entirely — two different tool calls hashed equal.
    #[test]
    fn openai_tool_calls_join_the_cache_key() {
        let wire = crate::proxy::wire::for_path("/v1/chat/completions").unwrap();
        let cfg = SemanticCache::default();
        let body = |args: &str| {
            serde_json::json!({
                "model": "m",
                "messages": [{"role": "assistant", "tool_calls": [
                    {"id": "c1", "type": "function", "function": {"name": "run", "arguments": args}}
                ]}]
            })
        };
        let pa = build_prompt(wire, &body("{\"cmd\":\"ls\"}"), &cfg).unwrap();
        let pb = build_prompt(wire, &body("{\"cmd\":\"rm -rf /\"}"), &cfg).unwrap();
        assert_ne!(canonical_hash(&pa), canonical_hash(&pb));
    }

    /// T55.14: image source data joins the key as its sha256, not rendered text.
    #[test]
    fn image_blocks_hash_into_the_cache_key() {
        let wire = crate::proxy::wire::for_path("/v1/messages").unwrap();
        let cfg = SemanticCache::default();
        let body = |data: &str| {
            serde_json::json!({
                "model": "m",
                "messages": [{"role": "user", "content": [
                    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": data}},
                    {"type": "text", "text": "what is in the picture?"}
                ]}]
            })
        };
        let pa = build_prompt(wire, &body(&"aB3dE5g7".repeat(600)), &cfg).unwrap();
        let pb = build_prompt(wire, &body(&"7g5E3dBa".repeat(600)), &cfg).unwrap();
        assert_ne!(canonical_hash(&pa), canonical_hash(&pb));
    }

    /// T323: an image/document source that is not base64 `data` (a URL, a Files API id,
    /// inline text) hashed as the empty string, so two requests differing only there
    /// shared a key and the second got the first one's answer.
    #[test]
    fn non_base64_image_and_document_sources_join_the_cache_key() {
        let wire = crate::proxy::wire::for_path("/v1/messages").unwrap();
        let cfg = SemanticCache::default();
        let body = |kind: &str, source: Value| {
            serde_json::json!({
                "model": "m",
                "messages": [{"role": "user", "content": [
                    {"type": kind, "source": source},
                    {"type": "text", "text": "describe it"}
                ]}]
            })
        };
        let key = |kind: &str, source: Value| {
            canonical_hash(&build_prompt(wire, &body(kind, source), &cfg).unwrap())
        };
        let url = |u: &str| serde_json::json!({"type": "url", "url": u});
        assert_ne!(
            key("image", url("https://a.test/1.png")),
            key("image", url("https://a.test/2.png"))
        );
        let file = |id: &str| serde_json::json!({"type": "file", "file_id": id});
        assert_ne!(
            key("document", file("file_1")),
            key("document", file("file_2"))
        );
        let text =
            |t: &str| serde_json::json!({"type": "text", "media_type": "text/plain", "data": t});
        assert_ne!(
            key("document", text("alpha")),
            key("document", text("beta"))
        );
        let inline = |t: &str| serde_json::json!({"type": "content", "content": t});
        assert_ne!(
            key("document", inline("alpha")),
            key("document", inline("beta"))
        );
        // The same source is still the same key, whatever its key order.
        let a = serde_json::json!({"type": "url", "url": "https://a.test/1.png"});
        let b = serde_json::json!({"url": "https://a.test/1.png", "type": "url"});
        assert_eq!(key("image", a), key("image", b));
    }

    /// T212: `max_tokens`/`temperature`/`top_p`/`tool_choice`/`thinking`/`stop_sequences`
    /// must join the cache key — a hit on these fields alone used to replay a wrong
    /// answer (a temperature-0 extraction sharing an entry with a temperature-1
    /// brainstorm). Key order and request-tracking ids must not, though.
    #[test]
    fn sampling_params_join_the_cache_key() {
        let wire = crate::proxy::wire::for_path("/v1/messages").unwrap();
        let cfg = SemanticCache::default();
        let body_with = |field: &str, value: Value| {
            let mut body = serde_json::json!({
                "model": "m",
                "messages": [{"role": "user", "content": "hi"}]
            });
            body[field] = value;
            body
        };
        let hash_of = |field: &str, value: Value| {
            canonical_hash(&build_prompt(wire, &body_with(field, value), &cfg).unwrap())
        };
        assert_ne!(
            hash_of("max_tokens", serde_json::json!(256)),
            hash_of("max_tokens", serde_json::json!(1024))
        );
        assert_ne!(
            hash_of("temperature", serde_json::json!(0.0)),
            hash_of("temperature", serde_json::json!(1.0))
        );
        assert_ne!(
            hash_of("top_p", serde_json::json!(0.1)),
            hash_of("top_p", serde_json::json!(0.9))
        );
        assert_ne!(
            hash_of("tool_choice", serde_json::json!({"type": "auto"})),
            hash_of("tool_choice", serde_json::json!({"type": "none"}))
        );
        assert_ne!(
            hash_of(
                "thinking",
                serde_json::json!({"type": "enabled", "budget_tokens": 1024})
            ),
            hash_of("thinking", serde_json::json!({"type": "disabled"}))
        );
        assert_ne!(
            hash_of("stop_sequences", serde_json::json!(["a"])),
            hash_of("stop_sequences", serde_json::json!(["b"]))
        );

        // Key order alone must not split the cache.
        let a = serde_json::json!({
            "model": "m", "max_tokens": 256, "temperature": 0.5,
            "messages": [{"role": "user", "content": "hi"}]
        });
        let b = serde_json::json!({
            "temperature": 0.5, "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 256, "model": "m"
        });
        assert_eq!(
            canonical_hash(&build_prompt(wire, &a, &cfg).unwrap()),
            canonical_hash(&build_prompt(wire, &b, &cfg).unwrap()),
            "key order alone must not split the cache"
        );

        // A request-tracking id that does not affect generation must not split it.
        let with_uid = |uid: &str| {
            serde_json::json!({
                "model": "m", "messages": [{"role": "user", "content": "hi"}],
                "metadata": {"user_id": uid}
            })
        };
        assert_eq!(
            canonical_hash(&build_prompt(wire, &with_uid("u1"), &cfg).unwrap()),
            canonical_hash(&build_prompt(wire, &with_uid("u2"), &cfg).unwrap()),
            "metadata.user_id must not split the cache"
        );
    }

    /// T212: `tools_fingerprint` used to hash tool *names* only, so two tools sharing
    /// a name but differing in schema hashed equal. Full definitions must join the
    /// key now, on both the Anthropic and OpenAI Chat tool shapes, order-independent.
    #[test]
    fn tool_schemas_join_the_cache_key() {
        let wire = crate::proxy::wire::for_path("/v1/messages").unwrap();
        let cfg = SemanticCache::default();
        let body = |schema: Value| {
            serde_json::json!({
                "model": "m",
                "messages": [{"role": "user", "content": "hi"}],
                "tools": [{"name": "run", "description": "run it", "input_schema": schema}]
            })
        };
        let narrow = serde_json::json!({"type": "object", "properties": {}});
        let wide = serde_json::json!({"type": "object", "properties": {"cmd": {"type": "string"}}});
        let pa = build_prompt(wire, &body(narrow.clone()), &cfg).unwrap();
        let pb = build_prompt(wire, &body(wide.clone()), &cfg).unwrap();
        assert_ne!(canonical_hash(&pa), canonical_hash(&pb));

        let chat = crate::proxy::wire::for_path("/v1/chat/completions").unwrap();
        let chat_body = |params: Value| {
            serde_json::json!({
                "model": "m",
                "messages": [{"role": "user", "content": "hi"}],
                "tools": [{"type": "function", "function": {"name": "run", "parameters": params}}]
            })
        };
        let ca = build_prompt(chat, &chat_body(narrow), &cfg).unwrap();
        let cb = build_prompt(chat, &chat_body(wide), &cfg).unwrap();
        assert_ne!(canonical_hash(&ca), canonical_hash(&cb));

        // Key order within a tool definition alone must not split the cache.
        let t1 = serde_json::json!(
            {"name": "run", "description": "d", "input_schema": {"type": "object"}}
        );
        let t2 = serde_json::json!(
            {"input_schema": {"type": "object"}, "name": "run", "description": "d"}
        );
        let pt1 = build_prompt(wire, &body_with_tools(vec![t1]), &cfg).unwrap();
        let pt2 = build_prompt(wire, &body_with_tools(vec![t2]), &cfg).unwrap();
        assert_eq!(canonical_hash(&pt1), canonical_hash(&pt2));
    }

    fn body_with_tools(tools: Vec<Value>) -> Value {
        serde_json::json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": tools
        })
    }

    #[test]
    fn direct_hit_on_exact_replay() {
        let body = serde_json::json!({
            "model": "claude-test",
            "messages": [{"role": "user", "content": "hello"}]
        });
        let cfg = SemanticCache::default();
        let wire = crate::proxy::wire::for_path("/v1/messages").unwrap();
        let prompt = build_prompt(wire, &body, &cfg).unwrap();
        let mut cache = Cache::new();
        cache.store(&prompt, &cfg, b"resp", Some("application/json"), 200);
        let hit = cache.lookup(&prompt, &cfg).unwrap();
        assert!(hit.direct);
        assert_eq!(hit.response.as_ref(), b"resp");
    }

    #[test]
    fn p9_fixture_audit_zero_false_hits() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/p9_semantic_cache_corpus");
        let cfg = SemanticCache {
            enabled: true,
            threshold: 0.99,
            ttl_s: 300,
            max_messages: 1,
            require_empty_tools: true,
            embed_backend: "fixture".into(),
            cache_by_model: true,
            cache_by_provider: true,
        };
        let report = audit_corpus(&dir, &cfg).expect("audit");
        assert_eq!(
            report.false_hit_pairs, 0,
            "false hits at {:.2} hit_rate",
            report.hit_rate
        );
    }

    #[test]
    fn an_error_response_is_never_cached() {
        let body = serde_json::json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hello"}]
        });
        let cfg = SemanticCache::default();
        let wire = crate::proxy::wire::for_path("/v1/messages").unwrap();
        let prompt = build_prompt(wire, &body, &cfg).unwrap();
        let mut cache = Cache::new();
        for status in [429, 500, 529] {
            cache.store(&prompt, &cfg, b"overloaded", None, status);
            assert!(cache.lookup(&prompt, &cfg).is_none(), "{status} was cached");
        }
    }

    #[test]
    fn hash_backend_skips_semantic_tier() {
        let a = serde_json::json!({
            "model": "m",
            "messages": [{"role": "user", "content": "a"}]
        });
        let b = serde_json::json!({
            "model": "m",
            "messages": [{"role": "user", "content": "b"}]
        });
        let cfg = SemanticCache::default();
        let wire = crate::proxy::wire::for_path("/v1/messages").unwrap();
        let pa = build_prompt(wire, &a, &cfg).unwrap();
        let pb = build_prompt(wire, &b, &cfg).unwrap();
        let mut cache = Cache::new();
        cache.store(&pa, &cfg, b"ra", None, 200);
        assert!(cache.lookup(&pb, &cfg).is_none());
    }

    /// T532: fields the flattened text drops still tell two requests apart, in both tiers; an
    /// exact replay, reordered keys and a moved `cache_control` marker still hit.
    #[rstest::rstest]
    #[case::image_detail(
        serde_json::json!([{"type": "image_url", "image_url": {"url": "u", "detail": "low"}}]),
        serde_json::json!([{"type": "image_url", "image_url": {"url": "u", "detail": "high"}}]),
        None, None,
    )]
    #[case::tool_result_error(
        serde_json::json!([{"type": "tool_result", "tool_use_id": "t", "content": "OK", "is_error": false}]),
        serde_json::json!([{"type": "tool_result", "tool_use_id": "t", "content": "OK", "is_error": true}]),
        None, None,
    )]
    #[case::participant_name(
        serde_json::json!("hi"), serde_json::json!("hi"), Some("alice"), Some("bob"),
    )]
    #[case::literal_text_versus_typed_block(
        serde_json::json!([{"type": "text", "text": "tool_result t: OK"}]),
        serde_json::json!([{"type": "tool_result", "tool_use_id": "t", "content": "OK"}]),
        None, None,
    )]
    fn typed_fields_join_the_cache_key(
        #[case] a: Value,
        #[case] b: Value,
        #[case] name_a: Option<&str>,
        #[case] name_b: Option<&str>,
    ) {
        let ask = |content: &Value, name: Option<&str>| {
            let mut m = serde_json::json!({"role": "user", "content": content});
            if let Some(n) = name {
                m["name"] = n.into();
            }
            fixture_prompt(serde_json::json!({"model": "m", "messages": [m]}))
        };
        let cfg = SemanticCache {
            embed_backend: "fixture".into(),
            ..SemanticCache::default()
        };
        let (pa, pb) = (ask(&a, name_a), ask(&b, name_b));
        assert_ne!(canonical_hash(&pa), canonical_hash(&pb));
        let mut cache = Cache::new();
        cache.store(&pa, &cfg, b"first", None, 200);
        assert!(cache.lookup(&pb, &cfg).is_none(), "both tiers miss");
        assert_eq!(&*cache.lookup(&pa, &cfg).unwrap().response, b"first");
    }

    /// T533: method, path and query each change the scope; the same request keeps it, and the
    /// raw target never appears in it.
    #[test]
    fn the_cache_scope_names_method_and_target() {
        let h = axum::http::HeaderMap::new();
        let base = cache_scope(&h, "POST", "https://a.test/v1/messages?key=secret");
        assert_eq!(
            base,
            cache_scope(&h, "POST", "https://a.test/v1/messages?key=secret")
        );
        for other in [
            ("GET", "https://a.test/v1/messages?key=secret"),
            ("POST", "https://b.test/v1/messages?key=secret"),
            ("POST", "https://a.test/v1/complete?key=secret"),
            ("POST", "https://a.test/v1/messages?key=other"),
        ] {
            assert_ne!(base, cache_scope(&h, other.0, other.1), "{other:?}");
        }
        assert!(!base.contains("secret"));
    }

    #[test]
    fn key_order_and_cache_control_do_not_split_the_key() {
        let ask = |block: Value| {
            fixture_prompt(
                serde_json::json!({"model": "m", "system": [{"type": "text", "text": "s"}],
                "messages": [{"role": "user", "content": [block]}]}),
            )
        };
        let plain = ask(serde_json::json!({"type": "text", "text": "hello"}));
        let reordered = ask(serde_json::json!({"text": "hello", "type": "text"}));
        let marked = ask(serde_json::json!({"type": "text", "text": "hello",
            "cache_control": {"type": "ephemeral"}}));
        assert_eq!(canonical_hash(&plain), canonical_hash(&reordered));
        assert_eq!(canonical_hash(&plain), canonical_hash(&marked));
    }

    #[test]
    fn store_drops_expired_entries_and_replaces_its_own() {
        let body = |t: &str| serde_json::json!({"model": "m", "messages": [{"role": "user", "content": t}]});
        let cfg = SemanticCache {
            embed_backend: "fixture".into(),
            ..SemanticCache::default()
        };
        let wire = crate::proxy::wire::for_path("/v1/messages").unwrap();
        let (pa, pb) = (
            build_prompt(wire, &body("a"), &cfg).unwrap(),
            build_prompt(wire, &body("b"), &cfg).unwrap(),
        );
        let mut cache = Cache::new();
        cache.store(&pa, &cfg, b"ra", None, 200);
        cache.store(&pa, &cfg, b"ra", None, 200);
        assert_eq!(
            cache.semantic.len(),
            1,
            "a replay replaces, it does not stack"
        );
        let old = Instant::now()
            .checked_sub(Duration::from_secs(cfg.ttl_s + 1))
            .unwrap();
        cache.direct.values_mut().for_each(|e| e.at = old);
        cache.semantic.iter_mut().for_each(|(_, _, e)| e.at = old);
        cache.store(&pb, &cfg, b"rb", None, 200);
        assert_eq!((cache.direct.len(), cache.semantic.len()), (1, 1));
    }

    fn fixture_prompt(body: Value) -> CachePrompt {
        let wire = crate::proxy::wire::for_path("/v1/messages").unwrap();
        build_prompt(wire, &body, &SemanticCache::default()).unwrap()
    }

    /// The vector covered only the last user message, so "hello" under a German system
    /// prompt got the cached French answer (similarity 1.0).
    #[test]
    fn the_same_question_under_another_system_prompt_misses() {
        let cfg = SemanticCache {
            embed_backend: "fixture".into(),
            ..SemanticCache::default()
        };
        let ask = |system: &str| {
            fixture_prompt(serde_json::json!({"model": "m", "system": system,
                "messages": [{"role": "user", "content": "hello"}]}))
        };
        let mut cache = Cache::new();
        cache.store(&ask("Answer in French"), &cfg, b"bonjour", None, 200);
        assert!(cache.lookup(&ask("Answer in German"), &cfg).is_none());
        assert!(cache.lookup(&ask("Answer in French"), &cfg).is_some());
    }

    /// Raw byte chunks in 8 positive slots scored unrelated prompts close to 1.
    #[rstest::rstest]
    #[case("add a test for the parser", "delete the build directory", false)]
    #[case("rename foo to bar", "rename bar to foo", false)]
    #[case("is x > 5", "is x < 5", false)]
    #[case("исправь баг в foo.rs", "удали foo.rs", false)]
    #[case("Fix the  bug", "fix the bug", true)]
    fn only_near_identical_prompts_clear_the_threshold(
        #[case] a: &str,
        #[case] b: &str,
        #[case] hit: bool,
    ) {
        let ask = |t: &str| {
            fixture_prompt(
                serde_json::json!({"model": "m", "messages": [{"role": "user", "content": t}]}),
            )
        };
        let sim = cosine(&embed_vector(&ask(a)), &embed_vector(&ask(b)));
        assert_eq!(sim >= SemanticCache::default().threshold, hit, "{sim}");
        assert!(hit || sim < 0.9, "{sim}");
    }

    #[test]
    fn empty_corpus_audits_to_zero() {
        let dir = std::env::temp_dir().join(format!("rtok-empty-corpus-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("corpus.json"), r#"{"entries": []}"#).unwrap();
        let report = audit_corpus(&dir, &SemanticCache::default());
        let _ = std::fs::remove_dir_all(&dir);
        let report = report.unwrap();
        assert_eq!((report.semantic_pairs, report.hit_rate), (0, 0.0));
    }

    /// Negative matrix: after one stored answer, every request that differs in anything that
    /// can change the answer misses (no foreign entry is ever served); the identical request
    /// and a moved `cache_control` marker (which does not change the answer) still hit.
    #[test]
    fn a_stored_answer_is_never_served_to_a_different_request() {
        use axum::http::{HeaderMap, HeaderValue};
        let cfg = SemanticCache {
            enabled: true,
            ..SemanticCache::default()
        };
        let wire = crate::proxy::wire::for_path("/v1/messages").unwrap();
        let base = serde_json::json!({
            "model": "m-1", "max_tokens": 100, "temperature": 0,
            "system": [{"type": "text", "text": "be brief"}],
            "messages": [{"role": "user", "content": [{"type": "text", "text": "2+2?"}]}]
        });
        let headers = |key: &str| {
            let mut h = HeaderMap::new();
            h.insert("x-api-key", HeaderValue::from_str(key).unwrap());
            h
        };
        let prompt = |body: &Value, key: &str| {
            build_prompt(wire, body, &cfg)
                .unwrap()
                .with_caller(&caller_identity(&headers(key)))
        };
        let mut cache = Cache::new();
        cache.store(&prompt(&base, "k1"), &cfg, b"4", None, 200);
        assert!(
            cache.lookup(&prompt(&base, "k1"), &cfg).is_some(),
            "identical hits"
        );
        let mut marked = base.clone();
        marked["messages"][0]["content"][0]["cache_control"] =
            serde_json::json!({"type": "ephemeral"});
        assert!(
            cache.lookup(&prompt(&marked, "k1"), &cfg).is_some(),
            "cache_control only"
        );

        let variant = |path: &str, v: Value| {
            let mut b = base.clone();
            *b.pointer_mut(path).unwrap() = v;
            b
        };
        let mut with_tool = base.clone();
        with_tool["tools"] = serde_json::json!([{"name": "t", "input_schema": {"type": "object"}}]);
        let mut with_stop = base.clone();
        with_stop["stop_sequences"] = serde_json::json!(["\\n"]);
        let cases = [
            (
                "prompt",
                variant("/messages/0/content/0/text", "2+3?".into()),
            ),
            (
                "prompt case",
                variant("/messages/0/content/0/text", "2+2?!".into()),
            ),
            ("system", variant("/system/0/text", "be verbose".into())),
            ("model", variant("/model", "m-2".into())),
            ("max_tokens", variant("/max_tokens", 101.into())),
            ("temperature", variant("/temperature", 1.into())),
            ("tools", with_tool),
            ("stop", with_stop),
        ];
        for (name, body) in &cases {
            assert!(
                cache.lookup(&prompt(body, "k1"), &cfg).is_none(),
                "{name} hit a foreign entry"
            );
        }
        assert!(
            cache.lookup(&prompt(&base, "k2"), &cfg).is_none(),
            "another caller"
        );
        let other_wire = crate::proxy::wire::for_path("/v1/chat/completions").unwrap();
        let chat =
            serde_json::json!({"model": "m-1", "messages": [{"role": "user", "content": "2+2?"}]});
        let other = build_prompt(other_wire, &chat, &cfg)
            .unwrap()
            .with_caller(&caller_identity(&headers("k1")));
        assert!(cache.lookup(&other, &cfg).is_none(), "another provider");
    }
}
