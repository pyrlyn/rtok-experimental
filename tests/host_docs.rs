// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Every host plugin (`plugins/<host>/`) and host installer (`src/agents/<host>/`) documents
//! itself: a `README.md` with a `## Docs` list of live links to the host's config and plugin
//! documentation (AGENTS.md, D21).

use std::fs;
use std::path::PathBuf;

fn hosts(sub: &str) -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(sub);
    let mut dirs: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("{}: {e}", root.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "no hosts under {}", root.display());
    dirs
}

/// The text between a `## Docs` heading and the next `## ` heading, or end of file if
/// `## Docs` is the last section. Stopping here (instead of running to end of file) matters:
/// a `## Docs` list left empty by a bad edit used to read as non-empty whenever any later
/// section happened to carry an `https://` line (T216).
fn docs_section(text: &str) -> Option<&str> {
    let start = text.find("## Docs")?;
    let after = &text[start + "## Docs".len()..];
    Some(match after.find("\n## ") {
        Some(end) => &after[..end],
        None => after,
    })
}

/// Each host's official docs domain, exactly as it already appears in that host's own
/// `## Docs` links (never invented) — `plugins/<id>` and `src/agents/<id>` are checked
/// separately because a few hosts point their plugin docs and their installer docs at
/// different domains (Kimi: kimi.com vs. moonshotai.github.io). Extends the `SKILL_HOSTS`
/// pattern: a generic `>= 1` link count cannot tell the host's own docs from a stray link
/// copy-pasted from another host or a later section.
const DOC_DOMAINS: &[(&str, &str)] = &[
    ("plugins/antigravity", "antigravity.google"),
    ("plugins/claude", "code.claude.com"),
    ("plugins/codebuddy", "codebuddy.ai"),
    ("plugins/cline", "docs.cline.bot"),
    ("plugins/codex", "chatgpt.com"),
    ("plugins/commandcode", "commandcode.ai"),
    ("plugins/copilot", "docs.github.com"),
    ("plugins/cursor", "cursor.com"),
    ("plugins/devin", "docs.devin.ai"),
    ("plugins/gemini", "geminicli.com"),
    ("plugins/grok", "docs.x.ai"),
    ("plugins/kimi", "kimi.com"),
    ("plugins/opencode", "opencode.ai"),
    ("plugins/pi", "pi.dev"),
    ("plugins/qwen", "qwenlm.github.io"),
    ("plugins/zcode", "zcode.z.ai"),
    ("src/agents/aider", "aider.chat"),
    ("src/agents/antigravity", "antigravity.google"),
    ("src/agents/claude", "code.claude.com"),
    ("src/agents/cline", "docs.cline.bot"),
    ("src/agents/codebuddy", "codebuddy.ai"),
    ("src/agents/codewhale", "github.com"),
    ("src/agents/codex", "chatgpt.com"),
    ("src/agents/commandcode", "commandcode.ai"),
    ("src/agents/copilot", "docs.github.com"),
    ("src/agents/cursor", "cursor.com"),
    ("src/agents/devin", "docs.devin.ai"),
    ("src/agents/gemini", "geminicli.com"),
    ("src/agents/grok", "docs.x.ai"),
    ("src/agents/kilo", "kilo.ai"),
    ("src/agents/kimi", "moonshotai.github.io"),
    ("src/agents/mimo", "mimo.xiaomi.com"),
    ("src/agents/omp", "github.com"),
    ("src/agents/opencode", "opencode.ai"),
    ("src/agents/pi", "pi.dev"),
    ("src/agents/qwen", "qwenlm.github.io"),
    ("src/agents/roo", "docs.roocode.com"),
    ("src/agents/vscode", "code.visualstudio.com"),
    ("src/agents/windsurf", "docs.windsurf.com"),
    ("src/agents/zcode", "zcode.z.ai"),
    ("src/agents/zed", "zed.dev"),
];

#[test]
fn every_host_readme_links_its_docs() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for dir in hosts("plugins").into_iter().chain(hosts("src/agents")) {
        let readme = dir.join("README.md");
        let text =
            fs::read_to_string(&readme).unwrap_or_else(|e| panic!("{}: {e}", readme.display()));
        let docs = docs_section(&text)
            .unwrap_or_else(|| panic!("{}: no `## Docs` section", readme.display()));
        let links: Vec<&str> = docs
            .lines()
            .filter(|l| l.trim_start().starts_with("- ") && l.contains("https://"))
            .collect();
        assert!(
            links.len() >= 2,
            "{}: `## Docs` needs at least 2 `- … https://` links, found {}",
            readme.display(),
            links.len()
        );
        let key = dir
            .strip_prefix(&root)
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        let domain = DOC_DOMAINS
            .iter()
            .find(|(k, _)| *k == key)
            .unwrap_or_else(|| panic!("{}: add its docs domain to DOC_DOMAINS", readme.display()))
            .1;
        let hits = links.iter().filter(|l| l.contains(domain)).count();
        assert!(
            hits >= 2,
            "{}: `## Docs` must link {domain} (its own docs) at least twice, found {hits} of {} links",
            readme.display(),
            links.len()
        );
    }
}

/// Hosts that receive `skills/rtok/` on install must link their skill-root docs (T71.3).
const SKILL_HOSTS: &[(&str, &str)] = &[
    ("claude", "code.claude.com/docs/en/skills"),
    ("cursor", "cursor.com/docs/skills"),
    ("codex", "agentskills.io"),
    ("opencode", "opencode.ai/docs/skills"),
    ("copilot", "copilot/concepts/agents/about-agent-skills"),
    ("pi", "pi.dev/docs/latest/skills"),
    ("commandcode", "CommandCodeAI/agent-skills"),
];

#[test]
fn skill_hosts_link_their_skill_root_docs() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/agents");
    for (id, needle) in SKILL_HOSTS {
        let readme = root.join(id).join("README.md");
        let text =
            fs::read_to_string(&readme).unwrap_or_else(|e| panic!("{}: {e}", readme.display()));
        assert!(
            text.contains(needle),
            "{}: `## Docs` must link the host skill root ({needle})",
            readme.display()
        );
    }
}

/// T217: `AGENTS.md` is loaded into every agent session and promises to stay under
/// 350 tokens. Words × 4/3 is the usual English token rate; bytes / 4 catches a file
/// that stays short in words but grows in tables, paths and code spans.
#[test]
fn agents_md_stays_under_its_350_token_budget() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let text = fs::read_to_string(root.join("AGENTS.md")).unwrap();
    let by_words = text.split_whitespace().count() * 4 / 3;
    let by_bytes = text.len() / 4;
    assert!(
        by_words < 350 && by_bytes < 350,
        "AGENTS.md ≈ {by_words} tokens by words, {by_bytes} by bytes; the budget is 350"
    );
    assert!(
        text.contains("under 350 tokens"),
        "the budget line must stay in the file"
    );
    #[cfg(unix)]
    assert_eq!(
        fs::read_link(root.join("CLAUDE.md")).unwrap(),
        PathBuf::from("AGENTS.md"),
        "CLAUDE.md must stay a symlink to AGENTS.md"
    );
}
