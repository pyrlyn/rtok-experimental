// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Comparative benches: native compress/ladder vs weak caveman / naive YAGNI baselines.
//!
//! # Why these baselines (not vendor marketing)
//!
//! - **caveman-lite**: only the weakest Sure! / I'd-be-happy strips — the part every terse
//!   mode does. We do not claim against caveman's unpublished full shrink pipeline or its
//!   65 % vendor figure (JetBrains measured 8.5 % agentic; research.md §4).
//! - **naive_always_minimum**: unstructured “be lazy” without a ladder — the failure mode of
//!   prompt-only YAGNI. We do not claim against ponytail's −54 % LOC own-bench (n=4, no
//!   independent check).
//!
//! Metrics: chars, estimated prose tokens (`Estimator` prose 4.2), fence identity, negation
//! retention, ladder rung accuracy. Never dollars — same Measurement philosophy as the rest
//! of rtok. Re-run: `cargo test --test mode_bench -- --nocapture`.
//! Published write-up: `research.md` (modes subsection), `docs/comparison.md` § Prompt-level.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use rtok::config::Estimator;
use rtok::modes::{
    CaveIntensity, LadderContext, LadderDecision, compress_prose, evaluate_ladder,
    naive_always_minimum,
};
use rtok::tokens::{self, Class};

const RATES: Estimator = Estimator {
    code: 3.5,
    prose: 4.2,
    json: 3.0,
    cjk: 1.0,
};

fn est(s: &str) -> u32 {
    tokens::estimate(s, Class::Prose, &RATES)
}

/// Weak caveman-lite baseline: only strip Sure! / I'd be happy to help.
fn caveman_lite(text: &str) -> String {
    let mut s = text.to_string();
    for p in [
        "I'd be happy to help you with that.",
        "I'd be happy to help you with that",
        "I'd be happy to help!",
        "I'd be happy to help.",
        "I'd be happy to help",
        "Sure!",
        "Sure.",
    ] {
        s = s.replace(p, "");
    }
    s.trim().to_string()
}

fn fence_bodies(s: &str) -> Vec<&str> {
    let mut bodies = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find("```") {
        let after = &rest[start + 3..];
        if let Some(end) = after.find("```") {
            bodies.push(&after[..end]);
            rest = &after[end + 3..];
        } else {
            break;
        }
    }
    bodies
}

/// ≥15 sample agent replies with fluff + code fences / critical negations.
fn prose_fixtures() -> Vec<&'static str> {
    vec![
        "Sure! I'd be happy to help. The issue is just really in the auth middleware.\n```rs\nfn check(t: u64) { assert!(t <= MAX); }\n```\nFix the comparison.",
        "Sure! Basically the cache is actually stale. Run `redis-cli PING` and check the error `NOAUTH Authentication required`.",
        "I'd be happy to help you with that. The function really just needs a guard. Do not drop the `null` check.",
        "Of course! The bug is simply that the parser never handles empty input.\n```py\ndef parse(s):\n    if not s:\n        return []\n```\nKeep the early return.",
        "Sure. I would be happy to help. Really the migration should not run twice. Never delete the backup.",
        "Happy to help! Actually we just need to patch the router. Error was `ECONNREFUSED`.\n```js\napp.get('/x', handler)\n```",
        "Certainly! The tests are basically failing because the fixture is really missing. Do not invent data.",
        "Sure! I'd be happy to help. Here is the diff.\n```diff\n- old\n+ new\n```\nApply it.",
        "I'd be happy to help. The API returns `401 Unauthorized` when the token is expired. Just refresh it.",
        "Sure! Really the HTML date picker is the right call — do not add a calendar lib.",
        "Of course. Basically a/an/the articles pad the reply. Keep `not` in 'do not deploy'.",
        "Sure! The heap dump shows the leak. Never ignore `OOMKilled` in the logs.\n```\nOOMKilled: true\n```",
        "I'd be happy to help you with that. Simply rewrite the loop. Actually no — reuse `itertools`.",
        "Sure. The schema has a unique constraint. Do not add an app-level duplicate check that can race.",
        "Happy to help. The flaky test is just a timing issue. Wait for the event; never sleep(5) as a fix.",
        "Certainly. I'd be happy to help. The README is really long. Trim filler; keep the install commands exact.",
        "Sure! Basically we should actually delete the dead code. Do not remove the feature flag yet.",
    ]
}

struct LadderFix {
    name: &'static str,
    ctx: LadderContext,
    expect: LadderDecision,
}

fn ladder_fixtures() -> Vec<LadderFix> {
    vec![
        LadderFix {
            name: "date_picker_native",
            ctx: LadderContext {
                native: true,
                installed_dep: true,
                ..Default::default()
            },
            expect: LadderDecision::NativePlatform,
        },
        LadderFix {
            name: "cache_stdlib",
            ctx: LadderContext {
                stdlib: true,
                one_liner: true,
                ..Default::default()
            },
            expect: LadderDecision::Stdlib,
        },
        LadderFix {
            name: "cache_one_liner",
            ctx: LadderContext {
                one_liner: true,
                ..Default::default()
            },
            expect: LadderDecision::OneLiner,
        },
        LadderFix {
            name: "speculative_redis",
            ctx: LadderContext {
                speculative: true,
                installed_dep: true,
                ..Default::default()
            },
            expect: LadderDecision::YagniSkip,
        },
        LadderFix {
            name: "security_validation",
            ctx: LadderContext {
                one_liner: true,
                must_not_simplify: true,
                ..Default::default()
            },
            expect: LadderDecision::Minimum,
        },
        LadderFix {
            name: "reuse_helper",
            ctx: LadderContext {
                existing_helper: true,
                stdlib: true,
                ..Default::default()
            },
            expect: LadderDecision::Reuse,
        },
        LadderFix {
            name: "installed_dep",
            ctx: LadderContext {
                installed_dep: true,
                ..Default::default()
            },
            expect: LadderDecision::InstalledDep,
        },
        LadderFix {
            name: "minimum_fallback",
            ctx: LadderContext::default(),
            expect: LadderDecision::Minimum,
        },
        LadderFix {
            name: "a11y_must_not",
            ctx: LadderContext {
                native: true,
                must_not_simplify: true,
                ..Default::default()
            },
            expect: LadderDecision::Minimum,
        },
        LadderFix {
            name: "data_loss_guard",
            ctx: LadderContext {
                speculative: false,
                must_not_simplify: true,
                stdlib: true,
                ..Default::default()
            },
            expect: LadderDecision::Minimum,
        },
        LadderFix {
            name: "speculative_beats_stdlib",
            ctx: LadderContext {
                speculative: true,
                stdlib: true,
                ..Default::default()
            },
            expect: LadderDecision::YagniSkip,
        },
        LadderFix {
            name: "reuse_beats_native",
            ctx: LadderContext {
                existing_helper: true,
                native: true,
                ..Default::default()
            },
            expect: LadderDecision::Reuse,
        },
        LadderFix {
            name: "stdlib_beats_dep",
            ctx: LadderContext {
                stdlib: true,
                installed_dep: true,
                ..Default::default()
            },
            expect: LadderDecision::Stdlib,
        },
        LadderFix {
            name: "native_date_only",
            ctx: LadderContext {
                native: true,
                ..Default::default()
            },
            expect: LadderDecision::NativePlatform,
        },
    ]
}

/// `fence_bodies` splits on raw triple backticks, so it cannot judge a four-backtick block.
/// This oracle is the block's literal text: whatever the fixture holds must come back verbatim.
#[test]
fn compress_keeps_longer_fences_and_identifier_suffixes_exact() {
    const OUTER: &str =
        "````md\nSure! Example:\n```rs\nlet the = 1;\n```\nThe really exact content\n````";
    const NAMES: [&str; 4] = ["foo_a", "/tmp/a", "item1a", "src/the"];
    let raw = format!(
        "Sure! The docs are really wrong about {}.\n{OUTER}\nUpdate the example.",
        NAMES.join(", ")
    );
    for intensity in [
        CaveIntensity::Lite,
        CaveIntensity::Full,
        CaveIntensity::Ultra,
    ] {
        let ours = compress_prose(&raw, intensity);
        assert!(ours.contains(OUTER), "{intensity:?}: fence changed\n{ours}");
        for name in NAMES {
            assert!(ours.contains(name), "{intensity:?}: lost {name} in {ours}");
        }
    }
    let full = compress_prose(&raw, CaveIntensity::Full);
    assert!(
        est(&full) < est(&raw),
        "prose outside the fence still compresses: {full}"
    );
}

#[test]
fn compress_beats_weak_caveman_and_identity() {
    let fixtures = prose_fixtures();
    assert!(
        fixtures.len() >= 15,
        "need ≥15 fixtures, got {}",
        fixtures.len()
    );

    let mut our_saved_chars = 0i64;
    let mut lite_saved_chars = 0i64;
    let mut our_saved_tok = 0i64;
    let mut lite_saved_tok = 0i64;

    for (i, raw) in fixtures.iter().enumerate() {
        let identity = (*raw).to_string();
        let weak = caveman_lite(raw);
        let ours = compress_prose(raw, CaveIntensity::Full);

        // Fence bodies byte-identical to input.
        let in_fences = fence_bodies(raw);
        let out_fences = fence_bodies(&ours);
        assert_eq!(
            in_fences, out_fences,
            "fixture {i}: fence body corrupted\nin={in_fences:?}\nout={out_fences:?}"
        );

        // Backtick error / command strings preserved when present.
        for chunk in raw.split('`').skip(1).step_by(2) {
            if chunk.chars().any(|c| c.is_ascii_uppercase()) || chunk.contains(' ') {
                // likely an error token or short command — must survive
                assert!(
                    ours.contains(&format!("`{chunk}`")) || ours.contains(chunk),
                    "fixture {i}: lost backtick span `{chunk}` in {ours}"
                );
            }
        }

        // Critical negations as whole words.
        for w in ["not", "never", "no "] {
            if raw.to_ascii_lowercase().contains(w) {
                assert!(
                    ours.to_ascii_lowercase().contains(w.trim()),
                    "fixture {i}: dropped critical word {w:?} → {ours}"
                );
            }
        }

        let before_c = identity.len() as i64;
        our_saved_chars += before_c - ours.len() as i64;
        lite_saved_chars += before_c - weak.len() as i64;
        our_saved_tok += est(&identity) as i64 - est(&ours) as i64;
        lite_saved_tok += est(&identity) as i64 - est(&weak) as i64;

        // Never expand vs identity on these fluff-heavy fixtures.
        assert!(
            ours.len() <= identity.len(),
            "fixture {i}: compress grew output"
        );
    }

    let n = fixtures.len() as f64;
    let our_avg = our_saved_chars as f64 / n;
    let lite_avg = lite_saved_chars as f64 / n;
    let our_pct =
        100.0 * our_saved_chars as f64 / fixtures.iter().map(|s| s.len()).sum::<usize>() as f64;
    let lite_pct =
        100.0 * lite_saved_chars as f64 / fixtures.iter().map(|s| s.len()).sum::<usize>() as f64;

    eprintln!(
        "compress: our_avg_chars_saved={our_avg:.1} lite_avg={lite_avg:.1} our_save%={our_pct:.1} lite_save%={lite_pct:.1} our_tok_saved={our_saved_tok} lite_tok_saved={lite_saved_tok}"
    );

    assert!(
        our_avg > lite_avg,
        "Full compress must save more chars on average than weak caveman-lite ({our_avg} vs {lite_avg})"
    );
    assert!(
        our_saved_tok > lite_saved_tok,
        "Full compress must save more estimated tokens than weak baseline ({our_saved_tok} vs {lite_saved_tok})"
    );
    assert!(
        our_saved_chars > 0,
        "identity baseline: our compress must save something"
    );
}

#[test]
fn ladder_accuracy_beats_naive_always_minimum() {
    let fixtures = ladder_fixtures();
    assert!(
        fixtures.len() >= 12,
        "need ≥12 ladder fixtures, got {}",
        fixtures.len()
    );

    let mut ours_ok = 0usize;
    let mut naive_ok = 0usize;
    for f in &fixtures {
        let got = evaluate_ladder(&f.ctx);
        assert_eq!(got, f.expect, "ladder wrong for {}", f.name);
        if got == f.expect {
            ours_ok += 1;
        }
        let naive = naive_always_minimum(&f.ctx);
        if naive == f.expect {
            naive_ok += 1;
        }
    }
    let our_acc = ours_ok as f64 / fixtures.len() as f64;
    let naive_acc = naive_ok as f64 / fixtures.len() as f64;
    eprintln!(
        "ladder: ours={ours_ok}/{} ({:.0}%) naive_minimum={naive_ok}/{} ({:.0}%)",
        fixtures.len(),
        our_acc * 100.0,
        fixtures.len(),
        naive_acc * 100.0
    );
    assert!(
        our_acc > naive_acc,
        "structured ladder must beat always-Minimum"
    );
    assert_eq!(
        ours_ok,
        fixtures.len(),
        "every fixture must match expected rung"
    );
}

#[test]
fn mode_markdown_budget_and_headings() {
    let terse = include_str!("../modes/terse.md");
    let yagni = include_str!("../modes/yagni.md");
    assert!(terse.starts_with("# terse"), "{terse}");
    assert!(yagni.starts_with("# yagni"), "{yagni}");
    assert!(est(terse) <= 250, "terse est={} > 250", est(terse));
    assert!(est(yagni) <= 250, "yagni est={} > 250", est(yagni));
    for label in ["Skipped:", "Not verified:", "Risks:"] {
        assert!(yagni.contains(label), "yagni missing tail label {label}");
    }
    eprintln!(
        "mode_budget: terse_tokens={} yagni_tokens={} terse_chars={} yagni_chars={}",
        est(terse),
        est(yagni),
        terse.len(),
        yagni.len()
    );
}

#[test]
fn inject_still_emits_enriched_modes() {
    let dir = std::env::temp_dir().join(format!("rtok-mode-bench-inject-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = rtok::testutil::config_in(&dir);
    cfg.plugins.inject.modes = vec!["terse".into(), "yagni".into()];
    let start = serde_json::json!({
        "hook_event_name": "SessionStart",
        "session_id": "mode-bench",
        "source": "startup"
    });
    let mut out = Vec::new();
    rtok::hooks::run("SessionStart", start.to_string().as_bytes(), &mut out, &cfg);
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let text = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(text.contains("# terse"), "{text}");
    assert!(text.contains("# yagni"), "{text}");
    assert!(
        text.contains("Auto-clarity") || text.contains("Pattern:"),
        "enriched terse missing signature phrases: {text}"
    );
    assert!(
        text.contains("ladder") || text.contains("YAGNI"),
        "enriched yagni missing ladder: {text}"
    );
    // Aliases resolve to the same builtins.
    cfg.plugins.inject.modes = vec!["cave".into(), "pony".into()];
    out.clear();
    rtok::hooks::run("SessionStart", start.to_string().as_bytes(), &mut out, &cfg);
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let text = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(text.contains("# terse"), "cave alias: {text}");
    assert!(text.contains("# yagni"), "pony alias: {text}");
}
