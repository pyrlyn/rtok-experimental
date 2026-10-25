// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Strip secrets before an observation narrative is stored.
//!
//! The pattern list is adapted from agentmemory `src/functions/privacy.ts`
//! `stripPrivateData` (Apache-2.0). The rewrite is ours.

use std::sync::LazyLock;

use regex::Regex;

// `(?is).` and not `[\s\S]`: under `(?i)` that union class costs ~7 ms to compile (Unicode case
// folding of every codepoint), paid by every `PostToolUse` process (T920). Same language.
static PRIVATE_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<private>.*?</private>").expect("private tag"));
static PRIVATE_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----[\s\S]*?(?:-----END [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----|$)",
    )
    .expect("private key")
});
static URL_USERINFO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b([a-z][a-z0-9+.-]*://)[^\s/?#@:"'<>]*:[^\s/?#@"'<>]+@"#).expect("url")
});

static SECRETS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r#"(?i)(?:api[_-]?key|secret|token|password|credential|auth)[\s]*[=:]\s*["']?[A-Za-z0-9_\-/.+]{20,}["']?"#,
        r"(?i)Bearer\s+[A-Za-z0-9._\-+/=]{20,}",
        r"sk-proj-[A-Za-z0-9\-_]{20,}",
        r"(?:sk|pk|rk|ak)-[A-Za-z0-9][A-Za-z0-9\-_]{19,}",
        r"sk-ant-[A-Za-z0-9\-_]{20,}",
        r"gh[pus]_[A-Za-z0-9]{36,}",
        r"github_pat_[A-Za-z0-9_]{22,}",
        r"xoxb-[A-Za-z0-9\-]+",
        r"AKIA[0-9A-Z]{16}",
        r"AIza[A-Za-z0-9\-_]{35}",
        r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
        r"npm_[A-Za-z0-9]{36}",
        r"glpat-[A-Za-z0-9\-_]{20,}",
    ]
    .into_iter()
    .map(|s| Regex::new(s).expect("secret pattern"))
    .collect()
});

/// Replace private tags, keys and token-shaped strings with `[REDACTED]`.
pub fn strip_private(input: &str) -> String {
    let mut result = PRIVATE_TAG.replace_all(input, "[REDACTED]").into_owned();
    result = PRIVATE_KEY.replace_all(&result, "[REDACTED]").into_owned();
    result = URL_USERINFO
        .replace_all(&result, "$1[REDACTED]@")
        .into_owned();
    for pattern in SECRETS.iter() {
        result = pattern.replace_all(&result, "[REDACTED]").into_owned();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_private_tags_and_key_shaped_tokens() {
        let raw = "keep <private>nope</private> and sk-ant-abcdefghijklmnopqrstuvwxyz";
        let out = strip_private(raw);
        assert!(!out.contains("nope"), "{out}");
        assert!(!out.contains("sk-ant-"), "{out}");
        assert!(out.contains("[REDACTED]"), "{out}");
    }

    /// T920: the tag pattern spans lines and ignores case, as it did with `[\s\S]`.
    #[test]
    fn strips_a_multiline_private_tag_in_any_case() {
        let out = strip_private("a <PRIVATE>one\ntwo</Private> b");
        assert_eq!(out, "a [REDACTED] b");
    }

    #[test]
    fn strips_url_userinfo() {
        let out = strip_private("see https://user:secret@example.com/a");
        assert!(out.contains("https://[REDACTED]@example.com/a"), "{out}");
        assert!(!out.contains("secret"), "{out}");
    }
}
