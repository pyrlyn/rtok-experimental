// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T216: `tests/trycmd/agents-list*.toml` match `stdout` against a bare `"""...\n"""` /
//! `[...]` wildcard, so a lost host row, a broken block header or a malformed `--json`
//! array all pass silently — "re-blessing" is a no-op. These two tests assert the actual
//! content instead: every host × variant in [`EXPECTED_VARIANTS`] must show up, in order, as
//! its own block header (text) or its own row (`--json`) — a Rust loop over concrete content
//! instead of one more giant snapshot.

mod common;

use common::agents::{rtok, tmp, write_cfg};
use rtok::agents::Kind;
use serde_json::Value;

/// `(host id, kind, variant name)` for every host × variant `agents list` prints today, in
/// `HOSTS` order — a literal table, not derived by calling `agents::HOSTS` /
/// `Agent::variants()` again. Deriving "expected" from the same functions the CLI reads
/// would let a lost host row or a deleted variant slip past unnoticed (both sides would
/// shrink together); this table only ever changes by a deliberate edit here, alongside the
/// host code (T216). Keep it in sync with `agents::HOSTS` and each host's `variants()`.
const EXPECTED_VARIANTS: &[(&str, Kind, &str)] = &[
    ("claude", Kind::Cli, "Claude Code"),
    ("claude", Kind::Desktop, "Claude Desktop"),
    ("cursor", Kind::Cli, "Cursor CLI"),
    ("cursor", Kind::Desktop, "Cursor"),
    ("codex", Kind::Cli, "Codex"),
    ("opencode", Kind::Cli, "OpenCode"),
    ("opencode", Kind::Desktop, "OpenCode Desktop"),
    ("kilo", Kind::Cli, "Kilo Code CLI"),
    ("kilo", Kind::Desktop, "Kilo Code for VS Code"),
    ("pi", Kind::Cli, "pi"),
    ("omp", Kind::Cli, "oh my pi"),
    ("zcode", Kind::Desktop, "ZCode"),
    ("kimi", Kind::Cli, "Kimi Code"),
    ("kimi", Kind::Desktop, "Kimi Code Desktop"),
    ("grok", Kind::Cli, "Grok Build"),
    ("vscode", Kind::Desktop, "VS Code"),
    ("vscode", Kind::Desktop, "VS Code - Insiders"),
    ("copilot", Kind::Cli, "Copilot CLI"),
    ("copilot", Kind::Desktop, "GitHub Copilot"),
    ("commandcode", Kind::Cli, "Command Code CLI"),
    ("aider", Kind::Cli, "aider"),
    ("windsurf", Kind::Desktop, "Windsurf"),
    ("zed", Kind::Cli, "Zed CLI"),
    ("zed", Kind::Desktop, "Zed"),
    ("cline", Kind::Cli, "Cline CLI"),
    ("cline", Kind::Desktop, "Cline for VS Code"),
    ("gemini", Kind::Cli, "Gemini CLI"),
    ("codewhale", Kind::Cli, "CodeWhale"),
    ("mimo", Kind::Cli, "MiMo Code"),
    ("antigravity", Kind::Cli, "Antigravity CLI"),
    ("antigravity", Kind::Desktop, "Antigravity"),
    ("devin", Kind::Cli, "Devin CLI"),
    ("devin", Kind::Desktop, "Devin"),
    ("roo", Kind::Desktop, "Roo Code"),
    ("qwen", Kind::Cli, "Qwen Code"),
    ("codebuddy", Kind::Cli, "CodeBuddy Code"),
    ("codebuddy", Kind::Desktop, "WorkBuddy"),
];

/// `agents list`'s text form: one `CLI: <name>` / `Desktop: <name>` header per variant, in
/// `HOSTS` order. Deleting a host from `HOSTS`, a variant from its `variants()`, or
/// reshaping `block()`'s header line changes this count or these prefixes.
#[test]
fn agents_list_text_headers_cover_every_host_variant() {
    let home = tmp("list-text");
    let cfg = write_cfg(&home);
    let out = rtok(&["agents", "list"], &cfg, &home);
    let headers: Vec<&str> = out
        .lines()
        .filter(|l| l.starts_with("CLI:") || l.starts_with("Desktop:"))
        .collect();
    let expected = EXPECTED_VARIANTS;
    assert_eq!(
        headers.len(),
        expected.len(),
        "expected {} host/variant headers, got {}:\n{out}",
        expected.len(),
        headers.len()
    );
    for (line, (host, kind, name)) in headers.iter().zip(expected.iter()) {
        let prefix = format!("{}: {name}", kind.label());
        assert!(
            line.starts_with(&prefix),
            "{host}: expected a header starting with {prefix:?}, got {line:?}\nfull output:\n{out}"
        );
    }
}

/// `agents list --json`'s rows: same coverage as the text form, over the structured
/// `host`/`kind`/`name` fields `AgentListRow` serializes. A malformed array (or one missing
/// a row) fails to zip against `EXPECTED_VARIANTS`.
#[test]
fn agents_list_json_rows_cover_every_host_variant() {
    let home = tmp("list-json");
    let cfg = write_cfg(&home);
    let out = rtok(&["agents", "list", "--json"], &cfg, &home);
    let rows: Vec<Value> = serde_json::from_str(&out)
        .unwrap_or_else(|e| panic!("agents list --json did not parse: {e}\n{out}"));
    let expected = EXPECTED_VARIANTS;
    assert_eq!(
        rows.len(),
        expected.len(),
        "expected {} rows, got {}:\n{out}",
        expected.len(),
        rows.len()
    );
    for (row, (host, kind, name)) in rows.iter().zip(expected.iter()) {
        assert_eq!(row["host"], Value::String(host.to_string()), "{row}");
        assert_eq!(
            row["kind"],
            Value::String(kind.as_str().to_string()),
            "{row}"
        );
        assert_eq!(row["name"], Value::String(name.to_string()), "{row}");
    }
}
