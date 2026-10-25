// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Budgeted SessionStart / UserPromptSubmit injection (plan T2.4, D5).

use rtok_plugin_sdk::{
    Class, Ctx, DashboardPage, Injection, Manifest, Measurement, Plugin, PreCompact, SessionStart,
    Surface,
};
use std::io::Read as _;

/// Catalogue plugin `inject`.
pub struct InjectPlugin;

impl Plugin for InjectPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "inject",
            surfaces: &[Surface::Hook],
            default_on: true,
        }
    }

    fn dashboard_page(&self) -> DashboardPage {
        DashboardPage::new(
            "Inject",
            "Budgeted SessionStart / prompt context that stays byte-stable.",
            true,
        )
    }

    fn session_start(&self, ev: &SessionStart, cx: &Ctx) -> Option<Injection> {
        let mut text = String::new();
        let mut priority = 5u8;
        if ev.source == "compact"
            && let Some(c) = super::checkpoint::offer(cx)
        {
            text.push_str(&c.text);
            priority = 9;
        } else if ev.source == "startup" {
            let mem = cx.plugin_config::<crate::config::Memory>("memory");
            if mem.enabled
                && mem.startup_recall
                && let Some(c) = super::checkpoint::offer_session(cx)
            {
                text.push_str(&c.text);
                priority = 9;
            }
        }
        if let Some(m) = modes_text(cx) {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&m);
        }
        if text.is_empty() {
            None
        } else {
            Some(Injection {
                plugin: "inject",
                text,
                priority,
            })
        }
    }

    fn pre_compact(&self, ev: &PreCompact, cx: &Ctx) {
        let _ = super::checkpoint::save(ev.transcript_path, cx);
    }
}

const TERSE: &str = include_str!("../../../modes/terse.md");
const YAGNI: &str = include_str!("../../../modes/yagni.md");
const NUDGES: &str = include_str!("../../../modes/nudges.md");

/// Resolve canonical mode names and their compatibility aliases to embedded markdown.
fn builtin(name: &str) -> Option<&'static str> {
    match name {
        "terse" | "cave" => Some(TERSE),
        "yagni" | "pony" => Some(YAGNI),
        "nudges" => Some(NUDGES),
        _ => None,
    }
}

fn modes_text(cx: &Ctx) -> Option<String> {
    let cfg = cx.plugin_config::<crate::config::Inject>("inject");
    let setup = cx.config::<crate::config::Setup>("setup");
    let names = if cfg.modes.is_empty() {
        setup.modes.as_slice()
    } else {
        cfg.modes.as_slice()
    };
    if names.is_empty() {
        return None;
    }
    let budget = cfg.budget_tokens;
    let rate = cx
        .config::<crate::config::Estimator>("estimator")
        .prose
        .max(0.1);
    let dir = &cfg.modes_dir;
    let mut text = String::new();
    for name in names {
        let path = dir.join(format!("{name}.md"));
        let Some(body) = read_mode(&path, builtin(name), budget, rate) else {
            continue;
        };
        let body = crate::plugin::fit_budget(cx, &body, Class::Prose, budget);
        text.push_str(&body);
        if !body.ends_with('\n') {
            text.push('\n');
        }
    }
    Some(text)
}

fn read_mode(
    path: &std::path::Path,
    builtin: Option<&str>,
    budget: u32,
    rate: f32,
) -> Option<String> {
    let f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return builtin.map(str::to_string),
    };
    // The estimator budgets characters, not bytes: keep one character past what fits (so the
    // fitter still sees an overshoot) and allow UTF-8's widest scalar per character.
    let chars = (f64::from(budget) * f64::from(rate)).ceil() as usize + 1;
    let mut buf = Vec::new();
    let read = f.take(chars.saturating_mul(4) as u64).read_to_end(&mut buf);
    if read.is_err() {
        return builtin.map(str::to_string);
    }
    // A read that stops inside a scalar leaves a partial tail; drop it rather than let the
    // lossy decode turn it into a replacement character.
    if let Err(e) = std::str::from_utf8(&buf)
        && e.error_len().is_none()
    {
        buf.truncate(e.valid_up_to());
    }
    Some(String::from_utf8_lossy(&buf).chars().take(chars).collect())
}

/// Pick injections in priority order until the budget. A candidate that starts
/// under budget is emitted even if it overshoots (T2.4 Check: 500+500 at 800).
pub fn apply(cx: &Ctx, mut offered: Vec<Injection>) -> String {
    offered.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.plugin.cmp(b.plugin)));
    let budget = cx
        .plugin_config::<crate::config::Inject>("inject")
        .budget_tokens;
    let mut parts: Vec<String> = Vec::new();
    let mut dropped = Vec::new();
    let mut used = 0u32;
    let mut before = 0u32;
    let mut before_bytes = 0u64;
    for i in &offered {
        let t = cx.estimate(&i.text, Class::Prose);
        before += t;
        before_bytes += i.text.len() as u64;
        if used >= budget {
            dropped.push(format!("dropped:{}:{t}", i.plugin));
            continue;
        }
        if t <= budget {
            used += t;
            parts.push(i.text.clone());
            continue;
        }
        let marker = format!("dropped:{}:{t}", i.plugin);
        let marker_cost = cx.estimate(&format!("\n{marker}"), Class::Prose);
        let mut room = budget.saturating_sub(used).saturating_sub(marker_cost);
        if !parts.is_empty() {
            room = room.saturating_sub(1);
        }
        let prefix = crate::plugin::fit_budget(cx, &i.text, Class::Prose, room);
        if prefix.is_empty() {
            dropped.push(marker);
            used = budget;
            continue;
        }
        used = budget;
        parts.push(prefix);
        dropped.push(marker);
    }
    let mut text = parts.join("\n");
    if !dropped.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&dropped.join("\n"));
    }
    let after = cx.estimate(&text, Class::Prose);
    let _ = cx.record(&Measurement {
        plugin: "inject",
        kind: "inject",
        before_bytes,
        after_bytes: text.len() as u64,
        est_before: before,
        est_after: after,
        ref_id: dropped.first().cloned(),
        call_id: None,
    });
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    fn blob(tokens: u32, cx: &Ctx) -> String {
        let rate = cx
            .config::<crate::config::Estimator>("estimator")
            .prose
            .max(0.1);
        let mut s = "x".repeat((tokens as f32 * rate) as usize);
        while cx.estimate(&s, Class::Prose) < tokens {
            s.push('x');
        }
        while cx.estimate(&s, Class::Prose) > tokens && !s.is_empty() {
            s.pop();
        }
        s
    }

    #[test]
    fn three_500_budget_800_drops_one_and_is_byte_stable() {
        let cx = crate::plugin::Runtime::in_memory("inject-t24").unwrap();
        let text = blob(500, &Ctx::new(&cx));
        assert_eq!(cx.estimate(&text, Class::Prose), 500);
        let offered = vec![
            Injection {
                plugin: "a",
                text: text.clone(),
                priority: 3,
            },
            Injection {
                plugin: "b",
                text: text.clone(),
                priority: 2,
            },
            Injection {
                plugin: "c",
                text: text.clone(),
                priority: 1,
            },
        ];
        let once = apply(&Ctx::new(&cx), offered.clone());
        let twice = apply(&Ctx::new(&cx), offered);
        assert_eq!(once, twice);
        assert!(once.starts_with(&format!("{text}\n{text}\n")), "{once}");
        let drop = format!("dropped:c:{}", cx.estimate(&text, Class::Prose));
        assert!(once.lines().any(|l| l == drop), "{once}");
        assert_eq!(
            once.matches('\n').count(),
            2,
            "two emitted + one dropped line"
        );
        assert_eq!(cx.store.measurement_count("inject").unwrap(), 2);
    }

    #[test]
    fn single_10x_candidate_fits_budget_and_marks_dropped() {
        let cx = crate::plugin::Runtime::in_memory("inject-t188-huge").unwrap();
        let budget = cx.config.plugins.inject.budget_tokens;
        let text = blob(budget * 10, &Ctx::new(&cx));
        let t = cx.estimate(&text, Class::Prose);
        assert_eq!(t, budget * 10);
        let offered = vec![Injection {
            plugin: "huge",
            text,
            priority: 5,
        }];
        let once = apply(&Ctx::new(&cx), offered.clone());
        let twice = apply(&Ctx::new(&cx), offered);
        assert_eq!(once, twice);
        assert!(
            once.lines().any(|l| l == format!("dropped:huge:{t}")),
            "{once}"
        );
        assert!(cx.estimate(&once, Class::Prose) <= budget, "{once}");
    }

    #[test]
    fn one_mb_mode_file_stays_under_budget_and_stable() {
        let dir = std::env::temp_dir().join("rtok-t188-big-mode");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("big.md"), "y".repeat(1 << 20)).unwrap();
        let mut cfg = crate::testutil::config_in(&dir);
        cfg.plugins.inject.modes_dir = dir.clone();
        cfg.plugins.inject.modes = vec!["big".into()];
        let budget = cfg.plugins.inject.budget_tokens;
        let start = serde_json::json!({
            "hook_event_name": "SessionStart",
            "session_id": "t188",
            "source": "startup"
        });
        let run = || {
            let mut out = Vec::new();
            crate::hooks::run("SessionStart", start.to_string().as_bytes(), &mut out, &cfg);
            let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
            v["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let (once, twice) = (run(), run());
        assert_eq!(once, twice);
        let cx = crate::plugin::Runtime::in_memory("t188").unwrap();
        // T283: SessionStart also offers this session's own rtok agent id, a small line
        // ahead of `inject`'s own (priority 5) content — `apply`'s documented behavior is
        // that a candidate under budget on its own is emitted even if the combination
        // overshoots, so the true ceiling here is the mode budget plus that line's own
        // cost. Measured rather than hardcoded, so a wording change can't desync this.
        let mut bare_cfg = cfg.clone();
        bare_cfg.plugins.inject.modes.clear();
        let mut bare_out = Vec::new();
        crate::hooks::run(
            "SessionStart",
            start.to_string().as_bytes(),
            &mut bare_out,
            &bare_cfg,
        );
        let agent_line = serde_json::from_slice::<serde_json::Value>(&bare_out).unwrap()
            ["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let slack = cx.estimate(&agent_line, Class::Prose);
        assert!(cx.estimate(&once, Class::Prose) <= budget + slack, "{once}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T53.1: the opt-in nudge set is data (D7) inside the mode budget (≤250),
    /// appears once in SessionStart output, byte-stable across runs, and never
    /// in UserPromptSubmit output.
    #[test]
    fn session_start_has_nudges_once_and_stable() {
        let dir = std::env::temp_dir().join("rtok-t531-nudges");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = crate::testutil::config_in(&dir);
        cfg.plugins.inject.modes = vec!["nudges".into()];
        let start = serde_json::json!({
            "hook_event_name": "SessionStart",
            "session_id": "t531",
            "source": "startup"
        });
        let run = || {
            let mut out = Vec::new();
            crate::hooks::run("SessionStart", start.to_string().as_bytes(), &mut out, &cfg);
            let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
            v["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let (once, twice) = (run(), run());
        assert_eq!(once, twice, "nudge bytes must be stable");
        assert_eq!(once.matches("# nudges").count(), 1, "{once}");
        let cx = crate::plugin::Runtime::in_memory("t531").unwrap();
        let n = cx.estimate(NUDGES, Class::Prose);
        assert!(n <= 250, "nudges is {n} tokens");
        let mut out = Vec::new();
        let prompt = serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "t531",
            "prompt": "hi"
        });
        crate::hooks::run(
            "UserPromptSubmit",
            prompt.to_string().as_bytes(),
            &mut out,
            &cfg,
        );
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let ctx = v
            .pointer("/hookSpecificOutput/additionalContext")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        assert!(!ctx.contains("# nudges"), "{ctx}");
    }

    #[test]
    fn session_start_has_mode_once_prompt_submit_does_not() {
        let dir = std::env::temp_dir().join("rtok-t71-modes");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = crate::testutil::config_in(&dir);
        cfg.plugins.inject.modes = vec!["terse".into(), "yagni".into()];
        let start = serde_json::json!({
            "hook_event_name": "SessionStart",
            "session_id": "t71",
            "source": "startup"
        });
        let mut out = Vec::new();
        crate::hooks::run("SessionStart", start.to_string().as_bytes(), &mut out, &cfg);
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let text = v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(text.contains("# terse"), "{text}");
        assert!(text.contains("# yagni"), "{text}");
        assert_eq!(text.matches("# terse").count(), 1);
        let cx = crate::plugin::Runtime::in_memory("t71").unwrap();
        assert!(cx.estimate(TERSE, Class::Prose) <= 250);
        assert!(cx.estimate(YAGNI, Class::Prose) <= 250);
        out.clear();
        let prompt = serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "t71",
            "prompt": "hi"
        });
        crate::hooks::run(
            "UserPromptSubmit",
            prompt.to_string().as_bytes(),
            &mut out,
            &cfg,
        );
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let ctx = v
            .pointer("/hookSpecificOutput/additionalContext")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        assert!(!ctx.contains("# terse"), "{ctx}");
        assert!(!ctx.contains("# yagni"), "{ctx}");
    }

    fn mode_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rtok-t606-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The estimator counts characters, the old read cap counted bytes: a mode that fits the
    /// token budget lost its tail whenever its characters were wider than one byte.
    #[test]
    fn read_mode_keeps_a_fitting_multibyte_mode_whole() {
        let dir = mode_dir("read");
        let cx = crate::plugin::Runtime::in_memory("t606").unwrap();
        let tail = "\nKEEP_THIS_TAIL\n";
        for (label, unit, n) in [
            ("ascii", "x", 3000),
            ("two", "ж", 3000),
            ("three", "世", 3000),
            ("four", "😀", 3000),
        ] {
            let text = format!("{}{tail}", unit.repeat(n));
            assert!(cx.estimate(&text, Class::Prose) <= 800, "{label} must fit");
            let path = dir.join(format!("{label}.md"));
            std::fs::write(&path, &text).unwrap();
            assert_eq!(
                read_mode(&path, None, 800, 4.2).as_deref(),
                Some(text.as_str()),
                "{label}"
            );
        }
        let missing = dir.join("missing.md");
        assert_eq!(
            read_mode(&missing, Some("builtin"), 800, 4.2).as_deref(),
            Some("builtin")
        );
        assert_eq!(read_mode(&missing, None, 800, 4.2), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_mode_bounds_a_huge_multibyte_file_on_a_character_boundary() {
        let dir = mode_dir("huge");
        let path = dir.join("huge.md");
        std::fs::write(&path, "😀".repeat(1 << 18)).unwrap();
        // 800 tokens * 4.2 = 3360 characters, plus the one extra that lets the fitter see the overshoot.
        let body = read_mode(&path, None, 800, 4.2).unwrap();
        assert_eq!(body.chars().count(), 3361);
        assert!(!body.contains('\u{fffd}'), "cut inside a character");
        assert_eq!(read_mode(&path, None, 0, 4.2).unwrap().chars().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_start_keeps_the_tail_of_a_fitting_unicode_mode() {
        let dir = mode_dir("hook");
        let tail = "KEEP_THIS_TAIL";
        std::fs::write(
            dir.join("uni.md"),
            format!("{}\n{tail}\n", "😀".repeat(1000)),
        )
        .unwrap();
        let mut cfg = crate::testutil::config_in(&dir);
        cfg.plugins.inject.modes_dir = dir.clone();
        cfg.plugins.inject.modes = vec!["uni".into()];
        let run = |event: &str, extra: serde_json::Value| {
            let mut ev = serde_json::json!({"hook_event_name": event, "session_id": "t606"});
            ev.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let mut out = Vec::new();
            crate::hooks::run(event, ev.to_string().as_bytes(), &mut out, &cfg);
            serde_json::from_slice::<serde_json::Value>(&out)
                .ok()
                .and_then(|v| {
                    v.pointer("/hookSpecificOutput/additionalContext")?
                        .as_str()
                        .map(str::to_string)
                })
                .unwrap_or_default()
        };
        let start = serde_json::json!({"source": "startup"});
        let (once, twice) = (
            run("SessionStart", start.clone()),
            run("SessionStart", start),
        );
        assert_eq!(once, twice);
        assert!(once.contains(tail), "{once}");
        assert!(!once.contains("dropped:inject"), "{once}");
        let prompt = run("UserPromptSubmit", serde_json::json!({"prompt": "hi"}));
        assert!(!prompt.contains(tail), "{prompt}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
