// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Mechanical observation of a tool call (T454).
//!
//! The shape follows agentmemory `src/functions/compress-synthetic.ts` (Apache-2.0):
//! a short title, a type inferred from the tool name, and a narrative capped at 400
//! characters. No model call. The full tool output stays in the archive.

use rtok_plugin_sdk::{Ctx, NewObservation, ObsHit, PostToolUse};
use serde_json::Value;

use super::scrub::strip_private;

const TITLE_MAX: usize = 80;
const NARRATIVE_MAX: usize = 400;
const RRF_K: f32 = 60.0;
/// At most this many hits from one session before the list backfills (agentmemory
/// `diversifyBySession`, `maxPerSession = 3`).
pub const MAX_PER_SESSION: usize = 3;

/// One compressed tool call, ready to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Synthesized {
    /// `file_read`, `command_run`, `error`, …
    pub obs_type: &'static str,
    /// Tool name, at most 80 characters.
    pub title: String,
    /// Scrubbed, at most 400 characters.
    pub narrative: String,
    /// Path-like strings taken from the tool input, not yet checked on disk.
    pub files: Vec<String>,
}

/// Map a tool name to an observation type. Word breaks follow
/// `compress-synthetic.ts` `inferType`.
pub fn infer_type(tool_name: Option<&str>, hook: &str, is_error: bool) -> &'static str {
    if is_error || hook == "post_tool_failure" {
        return "error";
    }
    if hook == "prompt_submit" {
        return "conversation";
    }
    let Some(tool_name) = tool_name.filter(|s| !s.is_empty()) else {
        return "other";
    };
    let n = split_name(tool_name);
    let has = |word: &str| {
        n == word
            || n.starts_with(&format!("{word}_"))
            || n.ends_with(&format!("_{word}"))
            || n.contains(&format!("_{word}_"))
    };
    if ["fetch", "http", "web"].iter().any(|w| has(w)) {
        return "web_fetch";
    }
    if ["grep", "search", "glob", "find"].iter().any(|w| has(w)) {
        return "search";
    }
    if ["bash", "shell", "exec", "run"].iter().any(|w| has(w)) {
        return "command_run";
    }
    if ["edit", "update", "patch", "replace"]
        .iter()
        .any(|w| has(w))
    {
        return "file_edit";
    }
    if ["write", "create"].iter().any(|w| has(w)) {
        return "file_write";
    }
    if ["read", "view"].iter().any(|w| has(w)) {
        return "file_read";
    }
    if ["task", "agent"].iter().any(|w| has(w)) {
        return "subagent";
    }
    "other"
}

fn split_name(tool_name: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = tool_name.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if c.is_uppercase() && i > 0 && chars[i - 1].is_lowercase() {
            out.push('_');
        }
        if *c == '-' || c.is_whitespace() {
            out.push('_');
        } else {
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

fn stringify(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let keep = n.saturating_sub(1);
    let mut t: String = s.chars().take(keep).collect();
    t.push('…');
    t
}

fn extract_files(tool_name: &str, input: &Value) -> Vec<String> {
    let Some(obj) = input.as_object() else {
        return Vec::new();
    };
    let search = tool_name.to_ascii_lowercase();
    let is_search = ["grep", "glob", "search", "find", "ls", "list"]
        .iter()
        .any(|w| search.contains(w))
        || obj.get("pattern").and_then(Value::as_str).is_some()
        || obj.get("query").and_then(Value::as_str).is_some();
    let keys: &[&str] = if is_search {
        &["file_path", "filepath", "filePath", "notebook_path"]
    } else {
        &[
            "file_path",
            "filepath",
            "path",
            "filePath",
            "file",
            "notebook_path",
        ]
    };
    let mut out = Vec::new();
    for key in keys {
        if let Some(v) = obj.get(*key).and_then(Value::as_str)
            && !v.is_empty()
            && v.len() < 512
            && !out.iter().any(|e| e == v)
        {
            out.push(v.to_string());
        }
    }
    out
}

/// Build the stored observation from a tool call. Input and output are scrubbed, then truncated
/// before they are joined so a large tool result is not copied into the note.
pub fn synthesize(tool_name: &str, hook: &str, input: &Value, output: &Value) -> Synthesized {
    let is_error = output.get("is_error").and_then(Value::as_bool) == Some(true)
        || output.get("error").is_some();
    let input_str = stringify(input);
    let output_str = stringify(output);
    // Scrub the whole field first: a cut after the scrubber would drop a private block's closing
    // tag or a secret's tail and leave the rest readable.
    let narrative_parts = [input_str, output_str]
        .into_iter()
        .map(|s| truncate(&strip_private(&s), 200))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    Synthesized {
        obs_type: infer_type(Some(tool_name), hook, is_error),
        title: truncate(
            if tool_name.is_empty() {
                "observation"
            } else {
                tool_name
            },
            TITLE_MAX,
        ),
        narrative: truncate(&narrative_parts, NARRATIVE_MAX),
        files: extract_files(tool_name, input),
    }
}

/// Reciprocal rank fusion over observation lists. `RRF_K` is 60, the same constant
/// as `src/store/embed.rs` and agentmemory `hybrid-search.ts`.
pub fn rrf_obs(lists: &[&[ObsHit]], limit: usize) -> Vec<ObsHit> {
    use std::collections::HashMap;
    let mut scores: HashMap<i32, f32> = HashMap::new();
    let mut hits: HashMap<i32, ObsHit> = HashMap::new();
    for list in lists {
        for (rank, h) in list.iter().enumerate() {
            *scores.entry(h.id).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
            hits.entry(h.id).or_insert_with(|| h.clone());
        }
    }
    let mut order: Vec<(i32, f32)> = scores.into_iter().collect();
    order.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    order
        .into_iter()
        .take(limit.max(1))
        .filter_map(|(id, _)| hits.remove(&id))
        .collect()
}

/// Keep at most `max_per_session` hits from one session, then backfill.
pub fn diversify_by_session(
    hits: Vec<ObsHit>,
    limit: usize,
    max_per_session: usize,
) -> Vec<ObsHit> {
    let mut selected = Vec::new();
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for h in &hits {
        let n = counts.entry(h.session_id.clone()).or_insert(0);
        if *n >= max_per_session {
            continue;
        }
        *n += 1;
        selected.push(h.clone());
        if selected.len() >= limit {
            break;
        }
    }
    if selected.len() < limit {
        for h in &hits {
            if selected.len() >= limit {
                break;
            }
            if !selected.iter().any(|s| s.id == h.id) {
                selected.push(h.clone());
            }
        }
    }
    selected
}

fn skip_tool(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("mem_")
        || n.starts_with("memory_")
        || n.contains("mem_save")
        || n.contains("mem_search")
}

/// Store one observation for this tool call. Returns the new id, or `None` when the
/// call is empty, a memory tool, a duplicate, or the store rejects it. Fail open.
pub fn capture(cx: &Ctx, ev: &PostToolUse) -> Option<i32> {
    if skip_tool(ev.tool_name) {
        return None;
    }
    let made = synthesize(
        ev.tool_name,
        "post_tool_use",
        ev.tool_input,
        ev.tool_response,
    );
    if made.narrative.is_empty() {
        return None;
    }
    let project = super::resolved_project(cx);
    let files = super::files::existing_rels(cx, &made.files);
    let dedup = crate::store::hex_sha256(
        format!("{}\0{}\0{}", cx.session(), ev.tool_name, made.narrative).as_bytes(),
    );
    let id = cx
        .insert_observation(&NewObservation {
            session_id: cx.session(),
            project: project.as_deref(),
            obs_type: made.obs_type,
            title: &made.title,
            narrative: &made.narrative,
            dedup: &dedup,
            files: &files,
        })
        .ok()
        .flatten()?;
    // No Measurement row on purpose (T927): the agent still receives the whole tool output, so
    // "output minus narrative" is not a saving, and the stored narrative costs it nothing until
    // `obs_get` or recall reads it, which meter themselves.
    Some(id)
}

/// Up to `n` observation index lines, `obs <id> <title> (<session prefix>)`.
pub fn index_lines(hits: &[ObsHit], n: usize) -> Vec<String> {
    hits.iter()
        .take(n)
        .map(|h| {
            let prefix: String = h.session_id.chars().take(8).collect();
            format!("obs {} {} ({prefix})", h.id, h.title)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// T617: the store now yields each file-linked observation once, so fusion keeps both
    /// distinct hits and still boosts one that is in the text list as well.
    #[test]
    fn fusion_keeps_distinct_file_hits_and_boosts_cross_list_ones() {
        let hit = |id| ObsHit {
            id,
            title: "t".into(),
            session_id: "s".into(),
            snippet: String::new(),
        };
        let linked = [hit(2), hit(1)];
        let ids =
            |lists: &[&[ObsHit]]| -> Vec<i32> { rrf_obs(lists, 10).iter().map(|h| h.id).collect() };
        assert_eq!(ids(&[&[], &linked]), vec![2, 1]);
        assert_eq!(ids(&[&[hit(1)], &linked]), vec![1, 2]);
    }

    #[test]
    fn bash_becomes_a_short_command_observation() {
        let s = synthesize(
            "Bash",
            "post_tool_use",
            &json!({"command": "cargo test"}),
            &json!("ok"),
        );
        assert_eq!(s.obs_type, "command_run");
        assert_eq!(s.title, "Bash");
        assert!(s.narrative.contains("cargo test"), "{}", s.narrative);
        assert!(s.narrative.chars().count() <= NARRATIVE_MAX);
    }

    /// T618: truncation used to cut a long private block's closing tag (and a secret's tail)
    /// before the scrubber ran, leaving the private prefix in the stored narrative.
    #[test]
    fn private_content_is_scrubbed_before_truncation_cuts_its_delimiters() {
        let long = |body: &str| format!("<private>{body}{}</private> public", "z".repeat(300));
        let secret = format!("{}sk-ant-{}", "x".repeat(190), "a".repeat(30));
        let cases = [
            (json!({"command": long("SYNTH_INPUT")}), json!("ok")),
            (json!({"command": "echo"}), json!(long("SYNTH_OUTPUT"))),
            (json!({"command": "echo"}), json!(secret)),
            (json!({"command": secret}), json!("ok")),
        ];
        for (input, output) in cases {
            let s = synthesize("Bash", "post_tool_use", &input, &output);
            for leak in ["SYNTH_", "<private>", "sk-ant-"] {
                assert!(!s.narrative.contains(leak), "{leak} in {}", s.narrative);
            }
            assert!(s.narrative.chars().count() <= NARRATIVE_MAX);
        }
        let s = synthesize(
            "Bash",
            "post_tool_use",
            &json!({"command": "echo"}),
            &json!(long("SYNTH_OUTPUT")),
        );
        assert!(s.narrative.contains("[REDACTED]"), "{}", s.narrative);
        assert!(s.narrative.contains("public"), "{}", s.narrative);
    }

    #[test]
    fn a_key_in_the_output_is_scrubbed_before_storage() {
        let s = synthesize(
            "Bash",
            "post_tool_use",
            &json!({"command": "echo"}),
            &json!("sk-ant-abcdefghijklmnopqrstuvwxyz"),
        );
        assert!(!s.narrative.contains("sk-ant-"), "{}", s.narrative);
        assert!(s.narrative.contains("[REDACTED]"), "{}", s.narrative);
    }

    #[test]
    fn narrative_stays_within_400_chars() {
        let huge = "x".repeat(5000);
        let s = synthesize(
            "Read",
            "post_tool_use",
            &json!({"file_path": "a.rs"}),
            &json!(huge),
        );
        assert!(
            s.narrative.chars().count() <= NARRATIVE_MAX,
            "{}",
            s.narrative.chars().count()
        );
        assert_eq!(s.obs_type, "file_read");
        assert_eq!(s.files, vec!["a.rs".to_string()]);
    }

    #[test]
    fn diversify_caps_one_session_then_backfills() {
        let hit = |id, session: &str| ObsHit {
            id,
            title: format!("t{id}"),
            session_id: session.into(),
            snippet: String::new(),
        };
        let hits = vec![
            hit(1, "aaaa"),
            hit(2, "aaaa"),
            hit(3, "aaaa"),
            hit(4, "aaaa"),
            hit(5, "bbbb"),
        ];
        let out = diversify_by_session(hits, 4, 3);
        let a = out.iter().filter(|h| h.session_id == "aaaa").count();
        assert!(a <= 3 || out.iter().any(|h| h.id == 5));
        assert_eq!(out.len(), 4);
        assert_eq!(out[3].id, 5);
    }
}
