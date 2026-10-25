// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `compress` — optional semantic shrink after the lossless `archive` lane (P28).
//!
//! On by default (`[plugins.compress] enabled`). In `proxy.mode = "compress"`, blocks
//! already archived by `archive` are replaced in-context with a deterministic extractive
//! summary. The archive row is unchanged; `expand <id>` always returns the original bytes.

use std::cell::RefCell;

use serde_json::Value;

pub mod llm;
pub mod must_keep;
pub mod queue;

use rtok_plugin_sdk::{
    Class, Ctx, DashboardPage, Manifest, Measurement, Plugin, Surface, ToolResultRef, WireRequest,
};

pub struct CompressPlugin;

thread_local! {
    // The proxy filter runs on one blocking thread per request and has no handle to the proxy,
    // so the ids it wants summarised wait here until `take_offered` hands them to the queue.
    static OFFERED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// The archive ids this thread's last proxy filter pass found without a stored summary.
pub(crate) fn take_offered() -> Vec<String> {
    OFFERED.with(|o| std::mem::take(&mut *o.borrow_mut()))
}

impl Plugin for CompressPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "compress",
            surfaces: &[Surface::Proxy],
            default_on: true,
        }
    }

    fn dashboard_page(&self) -> DashboardPage {
        DashboardPage::new(
            "Compress",
            "Extractive summaries for archived tool output; expand returns the original.",
            true,
        )
    }

    fn proxy_filter(&self, req: &mut WireRequest<'_>, cx: &Ctx) -> Vec<Measurement> {
        if !cx
            .plugin_config::<crate::config::Compress>("compress")
            .enabled
        {
            return Vec::new();
        }
        if cx.config::<crate::config::Proxy>("proxy").mode != "compress" {
            return Vec::new();
        }
        rewrite(req.tool_results(), cx)
    }
}

fn rewrite(results: Vec<ToolResultRef<'_>>, cx: &Ctx) -> Vec<Measurement> {
    crate::plugins::archive::outside_live_zone(results, cx)
        .filter_map(|r| summarize_block(&r.id, r.content, cx))
        .collect()
}

fn summarize_block(tool_use_id: &str, content: &mut Value, cx: &Ctx) -> Option<Measurement> {
    let pointer = content.as_str()?;
    let d = match cx.archive_decision(tool_use_id) {
        Ok(Some(d)) if !d.expanded && d.pointer.starts_with("[archived ") => d,
        Ok(_) => return None,
        Err(e) => {
            cx.log("error", "plugin", "compress", &format!("decision: {e}"));
            return None;
        }
    };
    if pointer != d.pointer {
        return None;
    }
    let summary = match stored_view(&d.archive_id, pointer.len(), cx) {
        Some(view) => view,
        None => {
            let bytes = cx
                .get_archive(&d.archive_id)
                .map_err(|e| cx.log("error", "plugin", "compress", &format!("get: {e}")))
                .ok()?
                .ok_or_else(|| {
                    cx.log("error", "plugin", "compress", "missing archive payload");
                })
                .ok()?;
            extractive_summary(&d.archive_id, &String::from_utf8_lossy(&bytes))
        }
    };
    if summary.len() >= pointer.len() {
        return None;
    }
    let m = Measurement {
        plugin: "compress",
        kind: "summary",
        before_bytes: pointer.len() as u64,
        after_bytes: summary.len() as u64,
        est_before: cx.estimate(pointer, Class::Code),
        est_after: cx.estimate(&summary, Class::Code),
        ref_id: Some(d.archive_id.clone()),
        call_id: None,
    };
    *content = Value::String(summary);
    Some(m)
}

/// The model summary of `archive_id`, as the block the agent sees (T385.9.2). A summary never
/// changes once stored, so the block is byte-stable from the turn it first appears. While none
/// exists the id is offered to the background queue and the caller keeps the extractive view;
/// a summary that would not be shorter than the pointer is no saving and is not used.
fn stored_view(archive_id: &str, pointer_len: usize, cx: &Ctx) -> Option<String> {
    let llm = cx.plugin_config::<crate::config::Compress>("compress").llm;
    if !llm.enabled {
        return None;
    }
    let summary = match cx.archive_summary(archive_id, &llm::version(&llm.model)) {
        Ok(Some(s)) => s,
        Ok(None) => {
            OFFERED.with(|o| o.borrow_mut().push(archive_id.to_string()));
            return None;
        }
        Err(e) => {
            cx.log("error", "plugin", "compress", &format!("summary: {e}"));
            return None;
        }
    };
    let short = crate::names::short_id(archive_id);
    let view =
        format!("[compress {short}]\ntype: tool_output\nsummary: {summary}\nexpand({short})");
    (view.len() < pointer_len).then_some(view)
}

/// Deterministic extractive ranker — no network, no LLM (D12). Each source line shows up at
/// most once, clipped like an archive pointer: a repeated or minified line used to outgrow the
/// pointer, and the block then kept the pointer with no saving.
pub(crate) fn extractive_summary(id: &str, text: &str) -> String {
    use crate::plugins::archive::clip;
    let mut seen = std::collections::HashSet::new();
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty() && seen.insert(*l))
        .collect();
    let title = lines
        .first()
        .map(|l| l.chars().take(80).collect::<String>());
    let narrative = lines
        .iter()
        .skip(1)
        .take(3)
        .map(|l| l.chars().take(60).collect::<String>())
        .collect::<Vec<_>>()
        .join("; ");
    let rest = || lines.iter().skip(4).copied();
    let is_error = |l: &str| l.to_ascii_lowercase().contains("error");
    // Errors first: nearly every line has a `:`, so they used to crowd errors out.
    let facts: Vec<&str> = rest()
        .filter(|l| is_error(l))
        .chain(rest().filter(|l| !is_error(l) && l.contains(':')))
        .take(3)
        .collect();
    let files: Vec<&str> = rest()
        .filter(|l| !facts.contains(l) && (l.contains('/') || l.contains('\\')))
        .take(3)
        .collect();
    let short = crate::names::short_id(id);
    let mut s = format!(
        "[compress {short}]\ntype: tool_output\ntitle: {}\nnarrative: {}",
        title.unwrap_or_else(|| "tool result".into()),
        narrative
    );
    if !facts.is_empty() {
        s.push_str("\nfacts:");
        for f in facts.iter().copied() {
            s.push_str("\n- ");
            s.push_str(&clip(f));
        }
    }
    if !files.is_empty() {
        s.push_str("\nfiles:");
        for f in files {
            s.push_str("\n- ");
            s.push_str(&clip(f));
        }
    }
    s.push_str(&format!("\nexpand({short})"));
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::archive::rewrite as archive_rewrite;
    use rtok_plugin_sdk::Archive;

    fn big(tag: &str) -> String {
        (1..=400)
            .map(|i| format!("{tag} line {i}: src/foo.rs: some shell output with words"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn cx(name: &str, compress_on: bool) -> crate::plugin::Runtime {
        let dir = std::env::temp_dir().join(format!("rtok-compress-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cx = crate::plugin::Runtime::in_memory("s").unwrap();
        cx.config.core.archive_dir = dir;
        cx.config.proxy.mode = "compress".into();
        cx.config.plugins.compress.enabled = compress_on;
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

    #[test]
    fn default_off_leaves_bytes_identical() {
        use crate::proxy::anthropic::ANTHROPIC;
        use rtok_plugin_sdk::ToolResults;
        let cx = cx("off", false);
        let content = big("t1");
        let mut body = serde_json::json!({
            "messages": (1..=6).map(|i| serde_json::json!({
                "role": "user",
                "content": [{"type":"tool_result","tool_use_id": format!("tu-{i}"), "content": &content}]
            })).collect::<Vec<_>>()
        });
        archive_rewrite(ANTHROPIC.tool_results(&mut body), &Ctx::new(&cx));
        let after_archive = body.clone();
        assert!(
            CompressPlugin
                .proxy_filter(&mut WireRequest::new(&ANTHROPIC, &mut body), &Ctx::new(&cx))
                .is_empty()
        );
        assert_eq!(body, after_archive);
    }

    #[test]
    fn fixture_compresses_and_expand_recovers() {
        let cx = cx("on", true);
        let mut values: Vec<Value> = (1..=6)
            .map(|n| Value::String(big(&format!("t{n}"))))
            .collect();
        archive_rewrite(refs(&mut values), &Ctx::new(&cx));
        let ms = rewrite(refs(&mut values), &Ctx::new(&cx));
        assert_eq!(ms.len(), 2);
        assert!(ms[0].after_bytes < ms[0].before_bytes);
        let summary = values[0].as_str().unwrap();
        assert!(summary.starts_with("[compress "));
        let archive_id = ms[0].ref_id.clone().unwrap();
        let recovered = cx.get_archive(&archive_id).unwrap().unwrap();
        assert_eq!(recovered, big("t1").into_bytes());
        assert_eq!(cx.store.count_kind("measurement").unwrap(), 0);
    }

    #[test]
    fn summary_is_deterministic() {
        let cx = cx("det", true);
        let mut first = vec![
            Value::String(big("one")),
            Value::String(big("two")),
            Value::String(big("three")),
            Value::String(big("four")),
            Value::String(big("five")),
            Value::String(big("six")),
        ];
        archive_rewrite(refs(&mut first), &Ctx::new(&cx));
        rewrite(refs(&mut first), &Ctx::new(&cx));
        let body = first.clone();
        let mut second = vec![
            Value::String(big("one")),
            Value::String(big("two")),
            Value::String(big("three")),
            Value::String(big("four")),
            Value::String(big("five")),
            Value::String(big("six")),
        ];
        archive_rewrite(refs(&mut second), &Ctx::new(&cx));
        rewrite(refs(&mut second), &Ctx::new(&cx));
        assert_eq!(body, second);
    }

    #[test]
    fn extractive_summary_shape() {
        let s = extractive_summary("abc123", "title line\nsrc/a.rs: ok\nerr: fail");
        assert!(s.contains("type: tool_output"));
        assert!(s.contains("title: title line"));
        assert!(s.contains("expand(abc123)"));
    }

    #[test]
    fn summary_puts_errors_first_and_clips_each_line_once() {
        let long = format!("src/big.rs: {}", "x".repeat(5000));
        let text =
            format!("head\nn1\nn2\nn3\na: 1\nb: 2\nc: 3\nERROR: boom\n{long}\n{long}\nsrc/z.rs");
        let s = extractive_summary("id", &text);
        let facts = s.split("\nfacts:").nth(1).unwrap();
        assert!(facts.starts_with("\n- ERROR: boom"), "{s}");
        assert!(s.len() < 1000, "long lines must be clipped: {}", s.len());
        assert_eq!(s.matches("src/big.rs").count(), 1, "{s}");
    }

    #[test]
    fn a_stored_summary_replaces_the_extractive_view_and_a_missing_one_is_offered() {
        let mut cx = cx("llm", true);
        cx.config.plugins.compress.llm.enabled = true;
        let mut values: Vec<Value> = (1..=6)
            .map(|n| Value::String(big(&format!("t{n}"))))
            .collect();
        archive_rewrite(refs(&mut values), &Ctx::new(&cx));
        let id = cx
            .archive_decision("tu-1")
            .unwrap()
            .expect("decision")
            .archive_id;
        let _ = take_offered();

        let mut waiting = values.clone();
        rewrite(refs(&mut waiting), &Ctx::new(&cx));
        assert!(waiting[0].as_str().unwrap().contains("narrative:"));
        assert!(take_offered().contains(&id), "no summary yet: offered");

        let version = llm::version(&cx.config.plugins.compress.llm.model);
        cx.store
            .put_archive_summary(&id, &version, "ran 400 lines of shell output")
            .unwrap();
        let mut ready = values.clone();
        rewrite(refs(&mut ready), &Ctx::new(&cx));
        let view = ready[0].as_str().unwrap();
        assert!(
            view.contains("summary: ran 400 lines of shell output"),
            "{view}"
        );
        let short = crate::names::short_id(&id);
        assert!(view.ends_with(&format!("expand({short})")), "{view}");
        assert!(!view.contains(&id), "{view}");
        assert!(
            !take_offered().contains(&id),
            "a summarised id is not offered"
        );

        let mut again = values.clone();
        rewrite(refs(&mut again), &Ctx::new(&cx));
        assert_eq!(again, ready, "the view is byte-stable");
    }
}
