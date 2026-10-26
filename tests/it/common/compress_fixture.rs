// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! A tool result and the mock compressor reply for it, shared by the P28 tests.

use rtok::plugins::compress::must_keep;
use serde_json::json;

/// A build log long enough that a short summary is a real saving, with spans of every kind.
pub fn log() -> String {
    let mut lines: Vec<String> = (1..=120)
        .map(|i| format!("   Compiling filler-crate-{i} v0.1.0 (/work/acme/crates/f{i})"))
        .collect();
    lines.splice(
        40..40,
        [
            "error[E0308]: mismatched types".to_string(),
            "   --> crates/ledger/src/posting.rs:142:23".to_string(),
            "settler: leg_0004 failed: BudgetExhausted after 5 attempts".to_string(),
            "run `acme settle --retry bat_01HZX4M8Q2` once; id 9c1e5ab07f2d44e8".to_string(),
        ],
    );
    lines.join("\n")
}

/// The summary a faithful model would write: every must-keep span of `text`, minus `skip`.
pub fn summary_with(text: &str, skip: Option<&str>) -> String {
    let kept: Vec<String> = must_keep::spans(text)
        .into_iter()
        .map(|s| s.text)
        .filter(|t| Some(t.as_str()) != skip)
        .collect();
    format!("cargo build failed.\n{}", kept.join("\n"))
}

/// An Anthropic Messages reply carrying `text`.
pub fn reply(text: &str) -> serde_json::Value {
    json!({"content": [{"type": "text", "text": text}], "stop_reason": "end_turn"})
}
