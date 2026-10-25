// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `archive` — replace old, large tool-result blocks in the live zone with pointers (plan P5).
//!
//! Spec: the catalogue in `plan.md` §1 names the tools this replaces; none is a
//! dependency (D6) — the behaviour is re-implemented here.
//!
//! T5.3: in `proxy.mode = "compress"`, a tool-result block is rewritten when it is older
//! than `archive.keep_turns` (a turn = one `user` message, counted from the end) and above
//! `archive.min_tokens`. The original goes to the archive store; the block's `content`
//! becomes a pointer: `[archived <id>: N lines · T tokens · expand(<id>)]` followed by
//! `head_lines` … `tail_lines` of the original. `<id>` is the 8-character prefix (T909);
//! the archive row and the measurement keep the full id, and `expand` resolves the prefix.
//! The decision is persisted per
//! `tool_use_id` before the request is forwarded, so the pointer is byte-identical on every
//! later request (the frozen prefix stays cacheable) and an `expand`ed id (T5.4) is never
//! rewritten again. `system`, `tools` and the last `keep_turns` turns are never touched.
//! T11.4 applies the same rewrite through `Wire::tool_results` for Anthropic Messages,
//! OpenAI Chat Completions, and OpenAI Responses.

use serde_json::Value;

use rtok_plugin_sdk::{
    BlobRef, Class, Ctx, DashboardPage, Manifest, Measurement, Plugin, SkillRef, Surface,
    ToolResultRef, WireRequest,
};

pub mod pi;

pub struct ArchivePlugin;

impl Plugin for ArchivePlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "archive",
            // `cli`: `rtok archive rewrite --stdin` — the pi `context` carrier (T70.2).
            surfaces: &[Surface::Proxy, Surface::Mcp, Surface::Cli],
            default_on: true,
        }
    }

    fn dashboard_page(&self) -> DashboardPage {
        DashboardPage::new(
            "Archive",
            "Shrink old tool results in the live zone; pointers expand on demand.",
            true,
        )
    }

    fn proxy_filter(&self, req: &mut WireRequest<'_>, cx: &Ctx) -> Vec<Measurement> {
        if cx.config::<crate::config::Proxy>("proxy").mode != "compress" {
            return Vec::new();
        }
        let mut out = rewrite(req.tool_results(), cx);
        out.extend(rewrite_blobs(req.live_blobs(), cx));
        out.extend(rewrite_skills(req.skills(), cx));
        out
    }
}

/// The results a proxy filter may rewrite: never the last `[plugins.archive] keep_turns` turns,
/// which are what the model is working with now. `toon` obeys the same boundary, so it is
/// decided here once rather than re-read by every plugin that shrinks old results.
pub(crate) fn outside_live_zone<'a>(
    results: Vec<ToolResultRef<'a>>,
    cx: &Ctx,
) -> impl Iterator<Item = ToolResultRef<'a>> {
    let keep = cx
        .plugin_config::<crate::config::Archive>("archive")
        .keep_turns as usize;
    results.into_iter().filter(move |r| r.turn >= keep)
}

/// Rewrite every eligible wire-normalised result; one measurement per rewritten block, all
/// under one `plugin_run` child call. The wire owns the provider-specific request shape.
pub fn rewrite(results: Vec<ToolResultRef<'_>>, cx: &Ctx) -> Vec<Measurement> {
    let mut out: Vec<Measurement> = outside_live_zone(results, cx)
        .filter_map(|r| rewrite_block(&r.id, r.content, r.turn, cx))
        .collect();
    record_run(&mut out, cx);
    out
}

pub fn rewrite_skills(skills: Vec<SkillRef<'_>>, cx: &Ctx) -> Vec<Measurement> {
    if !cx.plugin_config::<crate::config::Archive>("archive").skills {
        return Vec::new();
    }
    let keep = cx
        .plugin_config::<crate::config::Archive>("archive")
        .keep_turns as usize;
    let mut out: Vec<Measurement> = skills
        .into_iter()
        .filter(|s| s.turn >= keep)
        .filter_map(|s| rewrite_skill(&s.id, &s.name, s.content, cx))
        .collect();
    record_run(&mut out, cx);
    out
}

fn rewrite_skill(
    id: &str,
    name: &str,
    content: &mut serde_json::Value,
    cx: &Ctx,
) -> Option<Measurement> {
    let text = content.as_str()?.to_owned();
    let key = format!("skill:{id}");
    let (archive_id, live) = match cx.archive_decision(&key) {
        Ok(Some(d)) if d.expanded => return None,
        Ok(Some(d)) => (d.archive_id, d.pointer),
        Ok(None) => {
            let archive_id = cx.put_archive(text.as_bytes()).ok()?;
            let n = text.lines().count();
            // `id` is a wire `tool_use_id`: not necessarily ASCII, so cut on chars.
            let short: String = id.chars().take(12).collect();
            let shown = crate::names::short_id(&archive_id);
            let live = format!("[archived {short}: skill {name} · {n} lines · expand({shown})]");
            cx.put_archive_decision(&key, &archive_id, &live).ok()?;
            (archive_id, live)
        }
        Err(_) => return None,
    };
    let m = Measurement {
        plugin: "archive",
        kind: "skill",
        before_bytes: text.len() as u64,
        after_bytes: live.len() as u64,
        est_before: cx.estimate(&text, Class::Prose),
        est_after: cx.estimate(&live, Class::Prose),
        ref_id: Some(archive_id),
        call_id: None,
    };
    *content = serde_json::Value::String(live);
    Some(m)
}

/// Shrink large non-result payloads inside the live zone (T51.1): nested JSON dumps
/// and `data:` blobs in user content blocks. Off by default (`live_blobs`) until a
/// bench shows cost per passed task does not rise. Eligible from turn 2 up — the
/// proxy invariant, not `keep_turns`: these blocks are re-sent whole every turn, so
/// anything older than the working edge is fair game once it is big and byte-stable.
pub fn rewrite_blobs(blobs: Vec<BlobRef<'_>>, cx: &Ctx) -> Vec<Measurement> {
    if !cx
        .plugin_config::<crate::config::Archive>("archive")
        .live_blobs
    {
        return Vec::new();
    }
    let mut out: Vec<Measurement> = blobs
        .into_iter()
        .filter(|b| b.turn >= 2)
        .filter_map(|b| rewrite_blob(b.content, cx))
        .collect();
    record_run(&mut out, cx);
    out
}

/// Ground truth for `stats`: est. tokens before/after, nested under the API request.
fn record_run(out: &mut Vec<Measurement>, cx: &Ctx) {
    if out.is_empty() {
        return;
    }
    match cx.record_plugin_run("proxy", "archive") {
        Ok(id) => {
            let before = out.iter().map(|m| i64::from(m.est_before)).sum();
            let after = out.iter().map(|m| i64::from(m.est_after)).sum();
            let _ = cx.record_tokens(id, Some("archive"), "before", "estimate", before);
            let _ = cx.record_tokens(id, Some("archive"), "after", "estimate", after);
            for m in &mut *out {
                m.call_id = Some(id);
            }
        }
        Err(e) => cx.log("error", "plugin", "archive", &format!("plugin_run: {e}")),
    }
}

/// Decide for one block: reuse the persisted pointer, skip an expanded or small block, or
/// archive it now. Any store error leaves the block alone (fail open).
fn rewrite_block(
    tool_use_id: &str,
    content: &mut Value,
    turn: usize,
    cx: &Ctx,
) -> Option<Measurement> {
    let text = block_text(content)?;
    let a = cx.plugin_config::<crate::config::Archive>("archive");
    let (archive_id, live, kind) = match cx.archive_decision(tool_use_id) {
        Ok(Some(d)) if d.expanded => return None,
        // `archive_decisions` is shared with `toon` (same `tool_use_id` key); replaying its
        // block here measured the saving twice, once under each plugin.
        Ok(Some(d)) if d.pointer.starts_with(crate::plugins::toon::PREFIX) => return None,
        // `json_tree` shares the same decision row and already measured the fold.
        #[cfg(feature = "json_tree")]
        Ok(Some(d)) if d.pointer.starts_with(crate::plugins::json_tree::PREFIX) => return None,
        Ok(Some(d)) => {
            let kind = if a.tiers {
                tier_kind(&d.pointer)
            } else {
                "pointer"
            };
            (d.archive_id, d.pointer, kind)
        }
        Ok(None) => {
            let est = cx.estimate(&text, Class::Code);
            if est < a.min_tokens {
                return None;
            }
            let archive_id = cx
                .put_archive(text.as_bytes())
                .map_err(|e| cx.log("error", "plugin", "archive", &format!("put: {e}")))
                .ok()?;
            let (live, kind) = if a.tiers {
                tier_live(&text, &archive_id, est, turn, &a)
            } else {
                (
                    pointer(
                        &text,
                        &archive_id,
                        est,
                        a.head_lines as usize,
                        a.tail_lines as usize,
                        a.signal_lines as usize,
                    ),
                    "pointer",
                )
            };
            cx.put_archive_decision(tool_use_id, &archive_id, &live)
                .map_err(|e| cx.log("error", "plugin", "archive", &format!("decision: {e}")))
                .ok()?;
            (archive_id, live, kind)
        }
        Err(e) => {
            cx.log("error", "plugin", "archive", &format!("decision: {e}"));
            return None;
        }
    };
    let m = Measurement {
        plugin: "archive",
        kind,
        before_bytes: text.len() as u64,
        after_bytes: live.len() as u64,
        est_before: cx.estimate(&text, Class::Code),
        est_after: cx.estimate(&live, Class::Code),
        ref_id: Some(archive_id),
        call_id: None,
    };
    *content = Value::String(live);
    Some(m)
}

/// Decide for one live-zone blob: reuse the persisted pointer, skip an expanded,
/// small or non-blob string, or archive it now. Decisions key on `blob:{sha256}`,
/// so identical bytes map to byte-identical pointers on every turn (the prompt
/// cache holds) and `expand` recovers the original. Any store error leaves the
/// block alone (fail open).
fn rewrite_blob(content: &mut Value, cx: &Ctx) -> Option<Measurement> {
    let text = content.as_str()?.to_owned();
    if !is_blob_candidate(&text) {
        return None;
    }
    let a = cx.plugin_config::<crate::config::Archive>("archive");
    let est = cx.estimate(&text, Class::Json);
    if est < a.min_tokens {
        return None;
    }
    let key = format!("blob:{}", crate::store::hex_sha256(text.as_bytes()));
    let (archive_id, live) = match cx.archive_decision(&key) {
        Ok(Some(d)) if d.expanded => return None,
        Ok(Some(d)) if d.pointer.starts_with(crate::plugins::toon::PREFIX) => return None,
        Ok(Some(d)) => (d.archive_id, d.pointer),
        Ok(None) => {
            let archive_id = cx
                .put_archive(text.as_bytes())
                .map_err(|e| cx.log("error", "plugin", "archive", &format!("put: {e}")))
                .ok()?;
            let live = pointer(
                &text,
                &archive_id,
                est,
                a.head_lines as usize,
                a.tail_lines as usize,
                0,
            );
            cx.put_archive_decision(&key, &archive_id, &live)
                .map_err(|e| cx.log("error", "plugin", "archive", &format!("decision: {e}")))
                .ok()?;
            (archive_id, live)
        }
        Err(e) => {
            cx.log("error", "plugin", "archive", &format!("decision: {e}"));
            return None;
        }
    };
    let m = Measurement {
        plugin: "archive",
        kind: "live_blob",
        before_bytes: text.len() as u64,
        after_bytes: live.len() as u64,
        est_before: est,
        est_after: cx.estimate(&live, Class::Json),
        ref_id: Some(archive_id),
        call_id: None,
    };
    *content = Value::String(live);
    Some(m)
}

/// A shrink candidate (T51.1): a `data:` URI, a bare base64 run (image/document
/// payloads travel without the prefix), or text that parses as JSON. Plain prose
/// and code stay — the model is working with them, and guessing wrong would hide
/// live context behind a pointer.
fn is_blob_candidate(text: &str) -> bool {
    let trimmed = text.trim_start();
    if trimmed.starts_with("data:") {
        return true;
    }
    match trimmed.as_bytes().first() {
        Some(b'{') | Some(b'[') => serde_json::from_str::<Value>(trimmed).is_ok(),
        _ => is_base64_run(trimmed),
    }
}

/// Long whitespace-free base64 alphabet runs are binary payloads, not prose.
/// The 4 KiB floor keeps hashes and short ids out; `min_tokens` still gates.
fn is_base64_run(text: &str) -> bool {
    text.len() > 4096
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
}

/// The text of a tool-result content: a string, or text blocks joined by newlines.
/// `None` when any part is not text (images stay as they are).
pub(crate) fn block_text(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let mut out = Vec::with_capacity(parts.len());
            for p in parts {
                if p["type"] != "text" {
                    return None;
                }
                out.push(p["text"].as_str()?);
            }
            Some(out.join("\n"))
        }
        _ => None,
    }
}

fn pointer_line(text: &str, id: &str, est: u32) -> String {
    let n = text.lines().count();
    // One 8-char id in the label and in `expand` (T909). A stored decision keeps the
    // text it was written with, so a prefix already in the cache does not change.
    let short = crate::names::short_id(id);
    format!("[archived {short}: {n} lines · {est} tokens · expand({short})]")
}

/// Widest line a pointer or L1 extract shows, in chars. Head/tail are counted in lines, so a
/// block of a few huge lines (minified JSON, one long log line) came back whole under the
/// pointer line: longer than the original, with a negative saving measured. The full line is
/// in the archive, so the cut stays lossless.
const LINE_CHARS: usize = 200;

pub(crate) fn clip(line: &str) -> std::borrow::Cow<'_, str> {
    match line.char_indices().nth(LINE_CHARS) {
        Some((at, _)) => format!("{}…", &line[..at]).into(),
        None => line.into(),
    }
}

/// Case-insensitive markers of the lines a reader usually comes back for with `expand` grep.
const SIGNALS: [&str; 6] = [
    "error",
    "panic",
    "fail",
    "warning",
    "exception",
    "traceback",
];

fn is_signal(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    SIGNALS.iter().any(|s| lower.contains(s))
}

/// The pointer line, `head` and `tail` lines, and up to `signal` elided lines that look like
/// errors or failures, numbered as `expand` grep prints them (`N:line`). Those lines are what
/// an agent most often expands an archive for, so inlining them saves that round-trip; the
/// output is a pure function of the text, so the frozen prefix stays byte-identical.
fn pointer(text: &str, id: &str, est: u32, head: usize, tail: usize, signal: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let n = lines.len();
    let mut s = pointer_line(text, id, est);
    if n <= head + tail {
        for l in &lines {
            s.push('\n');
            s.push_str(&clip(l));
        }
        return s;
    }
    for l in &lines[..head] {
        s.push('\n');
        s.push_str(&clip(l));
    }
    s.push_str(&format!("\n… {} lines …", n - head - tail));
    let hits: Vec<usize> = (head..n - tail).filter(|&i| is_signal(lines[i])).collect();
    if signal > 0 && !hits.is_empty() {
        let shown = hits.len().min(signal);
        s.push_str(&format!(
            "\n[{shown} of {} matching elided lines]",
            hits.len()
        ));
        for &i in &hits[..shown] {
            s.push_str(&format!("\n{}:{}", i + 1, clip(lines[i])));
        }
    }
    for l in &lines[n - tail..] {
        s.push('\n');
        s.push_str(&clip(l));
    }
    s
}

/// P33 L1: deterministic lossless extract (structure map + numbered head/tail). No model.
fn l1_extract(text: &str, head: usize, tail: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let n = lines.len();
    let mut s = format!("[tier L1: {n} lines · lossless extract]\n");
    for (i, line) in lines.iter().enumerate().take(head) {
        s.push_str(&format!("L{}: {}\n", i + 1, clip(line)));
    }
    if n > head + tail {
        s.push_str(&format!("… {} omitted lines …\n", n - head - tail));
    }
    // `.max(head)`: with fewer than `head + tail` lines the tail used to repeat lines the
    // head had already printed.
    for (i, line) in lines
        .iter()
        .enumerate()
        .skip(n.saturating_sub(tail).max(head))
    {
        s.push_str(&format!("L{}: {}\n", i + 1, clip(line)));
    }
    s.trim_end().to_string()
}

/// L0 = pointer line only; hot boundary (`turn == keep_turns`) promotes to L1 (v0.1 head/tail + extract).
fn tier_live(
    text: &str,
    id: &str,
    est: u32,
    turn: usize,
    a: &crate::config::Archive,
) -> (String, &'static str) {
    let head = a.head_lines as usize;
    let tail = a.tail_lines as usize;
    if turn == a.keep_turns as usize {
        let body = pointer(text, id, est, head, tail, a.signal_lines as usize);
        let extract = l1_extract(text, head * 3, tail * 2);
        (format!("{body}\n{extract}"), "tier_l1")
    } else {
        (pointer_line(text, id, est), "tier_l0")
    }
}

fn tier_kind(live: &str) -> &'static str {
    if live.contains("[tier L1:") {
        "tier_l1"
    } else {
        "tier_l0"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn big(tag: &str) -> String {
        (1..=400)
            .map(|i| format!("{tag} line {i}: some shell output with words"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn cx(name: &str) -> crate::plugin::Runtime {
        let dir = std::env::temp_dir().join(format!("rtok-archive-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cx = crate::plugin::Runtime::in_memory("s").unwrap();
        cx.config.core.archive_dir = dir;
        cx.config.proxy.mode = "compress".into();
        cx
    }

    fn refs<'a>(values: &'a mut [Value]) -> Vec<ToolResultRef<'a>> {
        let total = values.len();
        values
            .iter_mut()
            .enumerate()
            .map(|(index, content)| ToolResultRef {
                id: format!("tu-{}", index + 1),
                content,
                turn: total - index - 1,
            })
            .collect()
    }

    fn brews<'a>(values: &'a mut [Value]) -> Vec<BlobRef<'a>> {
        let total = values.len();
        values
            .iter_mut()
            .enumerate()
            .map(|(index, content)| BlobRef {
                content,
                turn: total - index - 1,
            })
            .collect()
    }

    fn brewed_cx(name: &str) -> crate::plugin::Runtime {
        let mut cx = cx(name);
        cx.config.plugins.archive.live_blobs = true;
        cx
    }

    /// Six kilobytes of stable JSON, the T51.1 live-zone payload.
    fn dump(tag: &str) -> String {
        let items: Vec<String> = (1..=200)
            .map(|i| format!(r#"{{"id":{i},"name":"{tag}-item-{i}","ok":true}}"#))
            .collect();
        format!("[{}]", items.join(","))
    }

    #[test]
    fn only_results_outside_the_live_tail_are_rewritten() {
        let cx = cx("turns");
        let mut values: Vec<Value> = (1..=6)
            .map(|n| Value::String(big(&format!("t{n}"))))
            .collect();
        let ms = rewrite(refs(&mut values), &Ctx::new(&cx));
        assert_eq!(ms.len(), 2);
        let first = values[0].as_str().unwrap();
        assert!(first.starts_with("[archived ") && first.contains("t1 line 400"));
        for (index, content) in values.iter().enumerate().skip(2) {
            assert_eq!(content, &Value::String(big(&format!("t{}", index + 1))));
        }
        assert_eq!(cx.store.count_kind("plugin_run").unwrap(), 1);
        let ids = cx.store.call_ids_of_kind("plugin_run").unwrap();
        assert_eq!(ids.len(), 1);
        let phases = cx.store.token_phases(ids[0]).unwrap();
        assert!(phases.iter().any(|p| p == "before"), "{phases:?}");
        assert!(phases.iter().any(|p| p == "after"), "{phases:?}");
    }

    #[test]
    fn decisions_are_deterministic_and_expanded_ids_stay_original() {
        let cx = cx("repeat");
        let mut first = vec![
            Value::String(big("one")),
            Value::String(big("two")),
            Value::String(big("three")),
            Value::String(big("four")),
            Value::String(big("five")),
            Value::String(big("six")),
        ];
        let first_ms = rewrite(refs(&mut first), &Ctx::new(&cx));
        let archive_id = first_ms[0].ref_id.clone().unwrap();
        let first_body = first.clone();
        let mut second = vec![
            Value::String(big("one")),
            Value::String(big("two")),
            Value::String(big("three")),
            Value::String(big("four")),
            Value::String(big("five")),
            Value::String(big("six")),
        ];
        rewrite(refs(&mut second), &Ctx::new(&cx));
        assert_eq!(first_body, second);
        assert_eq!(cx.store.mark_expanded(&archive_id).unwrap(), 1);
        let mut third = vec![
            Value::String(big("one")),
            Value::String(big("two")),
            Value::String(big("three")),
            Value::String(big("four")),
            Value::String(big("five")),
            Value::String(big("six")),
        ];
        assert_eq!(rewrite(refs(&mut third), &Ctx::new(&cx)).len(), 1);
        assert_eq!(third[0], Value::String(big("one")));
    }

    /// `archive` runs before `toon` and both key `archive_decisions` by `tool_use_id`, so
    /// a toon-encoded block used to come back as an `archive` measurement as well.
    #[test]
    fn a_toon_decision_is_not_replayed_as_an_archive_saving() {
        use rtok_plugin_sdk::Archive;
        let cx = cx("toon-scope");
        let id = cx.put_archive(big("t1").as_bytes()).unwrap();
        cx.store
            .put_archive_decision("tu-1", &id, "s", &format!("[toon {id}]\na,b,c"))
            .unwrap();
        let mut values: Vec<Value> = (1..=6)
            .map(|n| Value::String(big(&format!("t{n}"))))
            .collect();
        let ms = rewrite(refs(&mut values), &Ctx::new(&cx));
        assert_eq!(ms.len(), 1, "only tu-2 is archive's");
        assert_eq!(values[0], Value::String(big("t1")), "left for toon");
    }

    /// A pointer is cut from the payload it replaced, so it belongs to the session that made
    /// it. Replaying another session's decision overwrote a live tool result with unrelated
    /// lines — and never archived the payload it replaced, so `expand` could not recover it.
    #[test]
    fn another_sessions_decision_is_not_replayed() {
        use rtok_plugin_sdk::Archive;
        let cx = cx("session-scope");
        let foreign = cx.put_archive(b"another session's payload").unwrap();
        cx.store
            .put_archive_decision("tu-1", &foreign, "other-session", "[archived foreign]")
            .unwrap();
        let mut values: Vec<Value> = (1..=6)
            .map(|n| Value::String(big(&format!("t{n}"))))
            .collect();
        let ms = rewrite(refs(&mut values), &Ctx::new(&cx));
        assert_eq!(ms.len(), 2, "both candidates are archived afresh");
        let first = values[0].as_str().unwrap();
        assert!(first.contains("t1 line 400"), "{first}");
        assert!(!first.contains("archived foreign"), "{first}");
    }

    #[test]
    fn tiers_off_matches_v0_1_and_tiers_on_promotes_hot_block() {
        let cx_off = cx("tiers-off");
        let mut off: Vec<Value> = (1..=6)
            .map(|n| Value::String(big(&format!("t{n}"))))
            .collect();
        let ms_off = rewrite(refs(&mut off), &Ctx::new(&cx_off));
        let mut repeat: Vec<Value> = (1..=6)
            .map(|n| Value::String(big(&format!("t{n}"))))
            .collect();
        rewrite(refs(&mut repeat), &Ctx::new(&cx_off));
        assert_eq!(off, repeat, "tiers off is byte-identical across replays");
        let mut cx_on = cx("tiers-on");
        cx_on.config.plugins.archive.tiers = true;
        let mut on: Vec<Value> = (1..=6)
            .map(|n| Value::String(big(&format!("t{n}"))))
            .collect();
        let ms_on = rewrite(refs(&mut on), &Ctx::new(&cx_on));
        assert!(
            off[0].as_str().unwrap().contains("t1 line 400"),
            "v0.1 keeps head/tail inline"
        );
        assert!(
            !on[0].as_str().unwrap().contains("t1 line 400"),
            "cold block is L0 line-only"
        );
        assert!(
            on[1].as_str().unwrap().contains("[tier L1:"),
            "hot block promoted"
        );
        assert!(ms_off.iter().all(|m| m.kind == "pointer"));
        assert_eq!(ms_on.iter().filter(|m| m.kind == "tier_l1").count(), 1);
        assert_eq!(ms_on.iter().filter(|m| m.kind == "tier_l0").count(), 1);
    }

    #[test]
    fn gate_p33_tier_ctt_on_fixture() {
        use crate::measure::jsonl;
        let path = format!(
            "{}/tests/fixtures/tier_context/session.jsonl",
            env!("CARGO_MANIFEST_DIR")
        );
        let parsed = jsonl::parse_path(std::path::Path::new(&path)).expect("fixture");
        assert!(parsed.tool_results.len() >= 10);
        let n = parsed.turns;
        let turns: Vec<u32> = parsed.tool_results.iter().map(|r| r.turn).collect();
        let values: Vec<Value> = parsed
            .tool_results
            .iter()
            .map(|r| Value::String(r.content.clone()))
            .collect();
        let ctt = |vals: &[Value], tiers: bool| -> u64 {
            let dir = std::env::temp_dir().join(format!(
                "rtok-p33-{}-{}",
                if tiers { "on" } else { "off" },
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let mut cx = crate::plugin::Runtime::in_memory("p33").unwrap();
            cx.config.core.archive_dir = dir;
            cx.config.proxy.mode = "compress".into();
            cx.config.plugins.archive.tiers = tiers;
            let mut work: Vec<Value> = vals.to_vec();
            let total = work.len();
            let refs: Vec<ToolResultRef<'_>> = work
                .iter_mut()
                .enumerate()
                .map(|(index, content)| ToolResultRef {
                    id: format!("tu-{}", index),
                    content,
                    turn: total - index - 1,
                })
                .collect();
            let _ = rewrite(refs, &Ctx::new(&cx));
            work.iter()
                .enumerate()
                .map(|(index, v)| {
                    let text = v.as_str().unwrap();
                    let tokens = cx.estimate(text, Class::Code) as u64;
                    let remain = u64::from(n).saturating_sub(u64::from(turns[index]));
                    tokens * remain
                })
                .sum()
        };
        let baseline = ctt(&values, false);
        let treatment = ctt(&values, true);
        let pct = 100.0 * treatment as f64 / baseline as f64;
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr(),
            "Gate P33 CTT: baseline={baseline} treatment={treatment} ratio={pct:.1}%"
        );
        assert!(
            treatment < baseline,
            "L0 line-only cold blocks beat v0.1 CTT"
        );
    }

    #[test]
    fn text_blocks_join_and_images_are_skipped() {
        assert_eq!(
            block_text(&serde_json::json!([{"type":"text","text":"a"},{"type":"text","text":"b"}])),
            Some("a\nb".into())
        );
        assert_eq!(block_text(&serde_json::json!([{"type":"image"}])), None);
        assert_eq!(
            pointer("a\nb", "abc", 3, 8, 4, 8),
            "[archived abc: 2 lines · 3 tokens · expand(abc)]\na\nb"
        );
    }

    fn six_turn_wires() -> [(&'static str, &'static dyn crate::proxy::wire::Wire); 3] {
        use crate::proxy::anthropic::ANTHROPIC;
        use crate::proxy::openai_chat::OPENAI_CHAT;
        use crate::proxy::openai_responses::OPENAI_RESPONSES;
        [
            ("anthropic_messages_6turns.json", &ANTHROPIC),
            ("openai_chat_6turns.json", &OPENAI_CHAT),
            ("openai_responses_6turns.json", &OPENAI_RESPONSES),
        ]
    }

    fn load_six_turn(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/proxy/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_slice(&std::fs::read(&path).expect(name)).expect(name)
    }

    fn result_texts(wire: &dyn crate::proxy::wire::Wire, req: &mut Value) -> Vec<String> {
        wire.tool_results(req)
            .into_iter()
            .map(|result| result.content.as_str().expect("string payload").to_string())
            .collect()
    }

    #[test]
    fn six_turn_fixtures_archive_only_turns_1_and_2_on_every_wire() {
        for (name, wire) in six_turn_wires() {
            let cx = cx(name);
            let original = load_six_turn(name);
            let original_bytes = serde_json::to_vec(&original).unwrap();
            let mut first = original.clone();
            let ms = rewrite(wire.tool_results(&mut first), &Ctx::new(&cx));
            assert_eq!(ms.len(), 2, "{name}");
            let rewritten = serde_json::to_vec(&first).unwrap();
            let pos = rewritten
                .windows(10)
                .position(|window| window == b"[archived ")
                .expect(name);
            assert_eq!(&original_bytes[..pos], &rewritten[..pos], "{name} prefix");
            let mut second = original.clone();
            rewrite(wire.tool_results(&mut second), &Ctx::new(&cx));
            assert_eq!(first, second, "{name} deterministic");
            let texts = result_texts(wire, &mut first);
            assert_eq!(texts.len(), 6, "{name}");
            assert!(
                texts[0].starts_with("[archived ") && texts[1].starts_with("[archived "),
                "{name}"
            );
            for (index, text) in texts.iter().enumerate().skip(2) {
                assert!(
                    text.starts_with(&format!("t{} line 1:", index + 1)),
                    "{name} turn {}",
                    index + 1
                );
            }
            let archive_id = ms[0].ref_id.clone().unwrap();
            assert_eq!(cx.store.mark_expanded(&archive_id).unwrap(), 1);
            let mut expanded = original.clone();
            assert_eq!(
                rewrite(wire.tool_results(&mut expanded), &Ctx::new(&cx)).len(),
                1
            );
            let live = result_texts(wire, &mut expanded);
            assert!(live[0].starts_with("t1 line 1:"), "{name} expand");
            assert!(live[1].starts_with("[archived "), "{name} turn 2 stays");
        }
    }

    /// One 20k-char line came back whole under the pointer line: a longer "saving".
    #[test]
    fn a_pointer_over_one_huge_line_is_shorter_than_the_line() {
        let text = "é".repeat(20_000);
        let p = pointer(&text, "abc123", 5000, 8, 4, 8);
        assert!(p.len() < text.len() / 10, "{}", p.len());
        assert!(p.contains("expand(abc123)"), "{p}");
        let l1 = l1_extract(&text, 8, 4);
        assert!(l1.len() < text.len() / 10, "{}", l1.len());
    }

    /// T909: the printed id is 8 hex chars. The ledger `ref_id` stays the full id,
    /// and `expand` resolves the prefix back to that row.
    #[test]
    fn pointer_prints_an_8_char_id_and_the_ledger_keeps_the_full_one() {
        let mut cx = cx("short-id");
        cx.config.plugins.archive.keep_turns = 1;
        cx.config.plugins.archive.min_tokens = 1;
        let body = big("t1");
        let mut values = vec![Value::String(body.clone()), Value::String("live".into())];
        let ms = rewrite(refs(&mut values), &Ctx::new(&cx));
        assert_eq!(ms.len(), 1);
        let full = ms[0].ref_id.clone().unwrap();
        assert_eq!(full.len(), 64, "{full}");
        let short = crate::names::short_id(&full);
        assert_eq!(short.len(), crate::names::MIN_PREFIX);
        let shown = values[0].as_str().unwrap();
        assert!(
            shown.contains(&format!("[archived {short}:"))
                && shown.contains(&format!("expand({short})")),
            "{shown}"
        );
        assert!(!shown.contains(&full), "{shown}");
        let resolved = crate::expand::resolve_id(&cx, short).unwrap();
        assert_eq!(resolved, full);
        let back = cx
            .store
            .get_archive(&resolved, Some(&cx.config.core.archive_dir))
            .unwrap()
            .unwrap();
        assert_eq!(String::from_utf8(back).unwrap(), body);
    }

    /// A pointer written before the short id (full sha in `expand`) is replayed as stored.
    /// Shortening only new decisions keeps a cached prefix byte-stable.
    #[test]
    fn a_stored_full_id_pointer_is_replayed_verbatim() {
        let mut cx = cx("replay-old");
        cx.config.plugins.archive.keep_turns = 0;
        cx.config.plugins.archive.min_tokens = 1;
        let body = big("old");
        let full = Ctx::new(&cx).put_archive(body.as_bytes()).unwrap();
        let old = format!(
            "[archived {}: 400 lines · 9 tokens · expand({full})]",
            &full[..12]
        );
        cx.store
            .put_archive_decision("tu-1", &full, "s", &old)
            .unwrap();
        let mut values = vec![Value::String(body)];
        let ms = rewrite(refs(&mut values), &Ctx::new(&cx));
        assert_eq!(ms.len(), 1);
        assert_eq!(values[0].as_str().unwrap(), old);
        assert_eq!(ms[0].ref_id.as_deref(), Some(full.as_str()));
    }

    /// Three lines with head 2 + tail 2 printed L2 twice.
    #[test]
    fn a_short_l1_extract_prints_each_line_once() {
        let l1 = l1_extract("a\nb\nc", 2, 2);
        assert_eq!(
            l1,
            "[tier L1: 3 lines · lossless extract]\nL1: a\nL2: b\nL3: c"
        );
    }

    /// Off by default: the same blobs pass through untouched until the bench
    /// (T51.1 gate) says otherwise.
    #[test]
    fn live_blobs_stay_whole_while_the_flag_is_off() {
        let cx = cx("blobs-off");
        let mut values: Vec<Value> = (1..=6).map(|_| Value::String(dump("d"))).collect();
        let ms = rewrite_blobs(brews(&mut values), &Ctx::new(&cx));
        assert!(ms.is_empty());
        assert!(values.iter().all(|v| v.as_str().unwrap().starts_with('[')));
        assert_eq!(cx.store.measurement_count("archive").unwrap(), 0);
    }

    /// Stable JSON dumps shrink identically on every replay; the last two turns
    /// stay live; prose, code and small JSON pass through.
    #[test]
    fn live_blobs_shrink_stably_outside_the_working_edge() {
        let cx = brewed_cx("blobs-on");
        let json = dump("j");
        let code = "fn main() {\n    println!(\"hi\");\n}\n".repeat(200);
        let prose = "just some words ".repeat(600);
        assert!(code.len() > 4000 && prose.len() > 4000);
        let mut values: Vec<Value> = vec![
            Value::String(json.clone()),
            Value::String(json.clone()),
            Value::String(json.clone()),
            Value::String(json.clone()),
            Value::String(code.clone()),
            Value::String(prose.clone()),
        ];
        let ms = rewrite_blobs(brews(&mut values), &Ctx::new(&cx));
        assert_eq!(ms.len(), 4, "turns 5,4,3,2 shrink; turns 1,0 stay");
        assert!(ms.iter().all(|m| m.kind == "live_blob"));
        for v in values.iter().take(4) {
            let s = v.as_str().unwrap();
            assert!(
                s.starts_with("[archived ") && s.contains("expand("),
                "{s:.80}"
            );
        }
        assert_eq!(values[4], Value::String(code.clone()), "code is not a dump");
        assert_eq!(
            values[5],
            Value::String(prose.clone()),
            "prose is not a dump"
        );
        let first = values.clone();
        let mut second: Vec<Value> = vec![
            Value::String(json.clone()),
            Value::String(json.clone()),
            Value::String(json.clone()),
            Value::String(json.clone()),
            Value::String(code),
            Value::String(prose),
        ];
        rewrite_blobs(brews(&mut second), &Ctx::new(&cx));
        assert_eq!(first, second, "same bytes in → byte-identical pointers out");
        // Lossless: the archived original is the dump, byte for byte.
        let id = ms[0].ref_id.clone().unwrap();
        let back = crate::plugin::Ctx::new(&cx)
            .get_archive(&id)
            .unwrap()
            .expect("archived");
        assert_eq!(String::from_utf8(back).unwrap(), json);
    }

    /// `data:` URIs and bare base64 runs shrink; an expanded blob stays original.
    #[test]
    fn live_blobs_cover_data_uris_and_base64_and_expands() {
        let cx = brewed_cx("blobs-data");
        let uri = format!("data:image/png;base64,{}", "aB3dE5g7".repeat(900));
        let bare = "aB3dE5g7".repeat(900);
        // Six turns: eligible are 5..2 (keep last two live). Put the second uri in the
        // live edge so only turns 5 and 4 yield measurements.
        let mut values: Vec<Value> = vec![
            Value::String(uri.clone()),
            Value::String(bare.clone()),
            Value::String("small".into()),
            Value::String("also-small".into()),
            Value::String(uri.clone()),
            Value::String("live-two".into()),
        ];
        let ms = rewrite_blobs(brews(&mut values), &Ctx::new(&cx));
        assert_eq!(ms.len(), 2, "two stable blobs at turns 5,4");
        assert!(values[0].as_str().unwrap().starts_with("[archived "));
        assert!(values[1].as_str().unwrap().starts_with("[archived "));
        assert_eq!(values[2], Value::String("small".into()), "under min_tokens");
        let id = ms[0].ref_id.clone().unwrap();
        assert_eq!(cx.store.mark_expanded(&id).unwrap(), 1);
        let mut again: Vec<Value> = vec![
            Value::String(uri),
            Value::String(bare),
            Value::String("small".into()),
            Value::String("x".into()),
            Value::String("y".into()),
            Value::String("z".into()),
        ];
        let ms2 = rewrite_blobs(brews(&mut again), &Ctx::new(&cx));
        assert_eq!(ms2.len(), 1, "the expanded blob stays original");
        assert!(again[0].as_str().unwrap().starts_with("data:"));
    }

    /// T55.15: binary-bearing fields are never rewritten in place — pointer text
    /// inside `source.data` would make the request invalid (400) the moment
    /// `live_blobs` turns on. The very same bytes shrink as a *text* block in the
    /// same turn, so the only reason the image survives is its block type.
    #[test]
    fn image_source_data_is_never_rewritten() {
        let cx = brewed_cx("blobs-image");
        let uri = format!("data:image/png;base64,{}", "aB3dE5g7".repeat(900));
        let mut req = serde_json::json!({"messages":[
            {"role":"user","content":[
                {"type":"text","text": uri},
                {"type":"image","source":{"type":"base64","media_type":"image/png","data": uri}}
            ]},
            {"role":"assistant","content":"ok"},
            {"role":"user","content":[{"type":"text","text": uri}]},
            {"role":"assistant","content":"ok"},
            {"role":"user","content":[{"type":"text","text":"small"}]}
        ]});
        let original = req.clone();
        let ms = rewrite_blobs(
            crate::proxy::wire::ToolResults::live_blobs(
                &crate::proxy::anthropic::ANTHROPIC,
                &mut req,
            ),
            &Ctx::new(&cx),
        );
        assert_eq!(ms.len(), 1, "the text blob shrinks, the image never");
        assert!(ms.iter().all(|m| m.kind == "live_blob"));
        assert_eq!(
            req["messages"][0]["content"][1]["source"]["data"],
            original["messages"][0]["content"][1]["source"]["data"],
            "source.data is byte-identical through rewrite_blobs"
        );
        assert!(
            req["messages"][0]["content"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("[archived ")
        );
    }

    /// T55.15 on the Chat wire: `image_url.url` is byte-identical through
    /// `rewrite_blobs`; the text part beside it still shrinks.
    #[test]
    fn openai_image_url_is_never_rewritten() {
        let cx = brewed_cx("blobs-image-url");
        let uri = format!("data:image/png;base64,{}", "aB3dE5g7".repeat(900));
        let mut req = serde_json::json!({"messages":[
            {"role":"user","content":[
                {"type":"text","text": uri},
                {"type":"image_url","image_url":{"url": uri}}
            ]},
            {"role":"assistant","content":"ok"},
            {"role":"user","content":"mid-turn filler"},
            {"role":"assistant","content":"ok"},
            {"role":"user","content":"small"}
        ]});
        let original = req.clone();
        let ms = rewrite_blobs(
            crate::proxy::wire::ToolResults::live_blobs(
                &crate::proxy::openai_chat::OPENAI_CHAT,
                &mut req,
            ),
            &Ctx::new(&cx),
        );
        assert_eq!(ms.len(), 1, "the text part shrinks, the image_url never");
        assert_eq!(
            req["messages"][0]["content"][1]["image_url"]["url"],
            original["messages"][0]["content"][1]["image_url"]["url"],
            "image_url.url is byte-identical through rewrite_blobs"
        );
        assert!(
            req["messages"][0]["content"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("[archived ")
        );
    }

    fn skill_text(name: &str) -> String {
        format!(
            "Base directory for this skill: /s/{name}\n\n# {name}\n{}",
            big(name)
        )
    }

    fn srefs<'a>(values: &'a mut [Value]) -> Vec<SkillRef<'a>> {
        let total = values.len();
        values
            .iter_mut()
            .enumerate()
            .map(|(index, content)| SkillRef {
                id: format!("tu-{}", index + 1),
                name: "pixel".into(),
                content,
                turn: total - index - 1,
            })
            .collect()
    }

    #[test]
    fn skill_bodies_archive_outside_keep_turns_stably() {
        let mut cx = cx("skills-on");
        cx.config.plugins.archive.skills = true;
        cx.config.plugins.archive.keep_turns = 1;
        let body = skill_text("pixel");
        let mut values: Vec<Value> = (0..3).map(|_| Value::String(body.clone())).collect();
        let ms = rewrite_skills(srefs(&mut values), &Ctx::new(&cx));
        assert_eq!(ms.len(), 2, "turns 2 and 1 (keep_turns=1); turn 0 stays");
        assert!(ms.iter().all(|m| m.kind == "skill"));
        let shown = values[0].as_str().unwrap();
        let full = ms[0].ref_id.clone().unwrap();
        let short = crate::names::short_id(&full);
        assert!(
            shown.starts_with("[archived ")
                && shown.contains("skill pixel")
                && shown.contains(&format!("expand({short})"))
                && !shown.contains(&full),
            "{shown}"
        );
        assert!(values[1].as_str().unwrap().starts_with("[archived "));
        assert_eq!(
            values[2],
            Value::String(body.clone()),
            "live edge stays whole"
        );
        let first = values.clone();
        let mut again: Vec<Value> = (0..3).map(|_| Value::String(body.clone())).collect();
        let ms2 = rewrite_skills(srefs(&mut again), &Ctx::new(&cx));
        assert_eq!(first, again, "byte-identical pointers on replay");
        assert_eq!(ms2[0].ref_id, ms[0].ref_id, "the body never re-archives");
        let id = ms[0].ref_id.clone().unwrap();
        let back = crate::plugin::Ctx::new(&cx)
            .get_archive(&id)
            .unwrap()
            .expect("archived");
        assert_eq!(String::from_utf8(back).unwrap(), body);
    }

    /// A non-ASCII `tool_use_id` used to panic on a byte slice inside a UTF-8 char.
    #[test]
    fn a_non_ascii_skill_id_is_shortened_on_a_char_boundary() {
        let mut cx = cx("skills-utf8-id");
        cx.config.plugins.archive.skills = true;
        cx.config.plugins.archive.keep_turns = 0;
        let mut values = [Value::String(skill_text("pixel"))];
        let skills = vec![SkillRef {
            id: "tu-ключи-1".into(),
            name: "pixel".into(),
            content: &mut values[0],
            turn: 1,
        }];
        let ms = rewrite_skills(skills, &Ctx::new(&cx));
        assert_eq!(ms.len(), 1);
        assert!(
            values[0]
                .as_str()
                .unwrap()
                .starts_with("[archived tu-ключи-1: skill pixel"),
            "{}",
            values[0]
        );
    }

    #[test]
    fn skill_bodies_stay_whole_while_the_flag_is_off() {
        let mut cx = cx("skills-off");
        cx.config.plugins.archive.skills = false;
        cx.config.plugins.archive.keep_turns = 0;
        let body = skill_text("pixel");
        let mut values = vec![Value::String(body.clone())];
        let ms = rewrite_skills(srefs(&mut values), &Ctx::new(&cx));
        assert!(ms.is_empty());
        assert_eq!(values[0], Value::String(body));
    }

    /// An elided error line comes back under the pointer with its line number, so reading it
    /// needs no `expand` call; without such lines, or with `signal_lines = 0`, the pointer is
    /// what it was before.
    #[test]
    #[allow(clippy::print_stderr)]
    fn savings_elided_error_lines_are_inlined_and_save_an_expand() {
        let mut lines: Vec<String> = (1..=400)
            .map(|i| format!("compiling crate_{i} v0.1.0"))
            .collect();
        lines[199] = "error[E0308]: mismatched types at src/lib.rs:42".into();
        lines[249] = "test parse::tests::roundtrip ... FAILED".into();
        let text = lines.join("\n");
        let p = pointer(&text, "abc123", 5000, 8, 4, 8);
        assert!(p.contains("\n200:error[E0308]: mismatched types"), "{p}");
        assert!(
            p.contains("\n250:test parse::tests::roundtrip ... FAILED"),
            "{p}"
        );
        assert!(p.contains("[2 of 2 matching elided lines]"));
        assert_eq!(p, pointer(&text, "abc123", 5000, 8, 4, 8), "deterministic");
        assert!(!pointer(&text, "abc123", 5000, 8, 4, 0).contains("200:error"));
        // The archived block stays a small fraction of the original.
        assert!(p.len() * 10 <= text.len(), "{} vs {}", p.len(), text.len());
        // Expand round-trips needed to see the failure: 1 before (grep), 0 now.
        let before = pointer(&text, "abc123", 5000, 8, 4, 0);
        let needs_expand = |s: &str| !s.contains("mismatched types");
        assert!(needs_expand(&before) && !needs_expand(&p));
        eprintln!(
            "SAVINGS archive_signal: original {} bytes, pointer {} -> {} bytes, expand calls 1 -> 0",
            text.len(),
            before.len(),
            p.len()
        );
        let clean: String = (1..=400).map(|i| format!("ok {i}\n")).collect();
        assert_eq!(
            pointer(&clean, "a", 1, 8, 4, 8),
            pointer(&clean, "a", 1, 8, 4, 0)
        );
    }

    /// The cap holds: no more than `signal_lines` matches are inlined.
    #[test]
    fn signal_lines_are_capped() {
        let text: String = (1..=100).map(|i| format!("warning {i}\n")).collect();
        let p = pointer(&text, "a", 1, 2, 2, 3);
        assert!(p.contains("[3 of 96 matching elided lines]"), "{p}");
        assert_eq!(
            p.lines()
                .filter(|l| l.starts_with(char::is_numeric))
                .count(),
            3
        );
    }
}
