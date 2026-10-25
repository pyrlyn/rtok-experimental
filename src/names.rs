// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Command and tool names shared by surfaces and plugins.
//!
//! These used to live on `agents` and `hooks`. Callers below those modules
//! (`measure`, `plugins`) need the same functions without an edge back up.

/// Basename of a command path — split on `/` and `\`, drop a trailing `.exe`
/// (case-insensitive). One definition for `agents`, `measure` and `cmd`
/// formatters (T55.10).
pub(crate) fn cmd_stem(path: &str) -> &str {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    // `get`, not `[..]`: the last 4 bytes of a non-ASCII name (`héllo`) can start mid-char.
    let cut = base.len().saturating_sub(4);
    if base.len() >= 4
        && base
            .get(cut..)
            .is_some_and(|e| e.eq_ignore_ascii_case(".exe"))
    {
        &base[..cut]
    } else {
        base
    }
}

/// T922: leads the `rtok run` wrapper on POSIX. The host's Bash may lack `rtok` on its PATH even
/// when the hook found one (hook lines also try `~/.ketch/bin`), and `command not found` would
/// fail every call; this defines a stand-in `rtok` that evals the last argument — the quoted
/// original command — in a subshell, like the real child shell. Never written on Windows, where
/// the rewrite is PowerShell-quoted and `command -v` does not exist.
pub(crate) fn run_guard() -> &'static str {
    if cfg!(windows) { "" } else { POSIX_RUN_GUARD }
}

const POSIX_RUN_GUARD: &str =
    "command -v rtok >/dev/null 2>&1 || rtok() ( for a; do :; done; eval \"$a\" ); ";

/// PreToolUse sees the user's command; PostToolUse often sees `rtok run -- '…'`,
/// or `rtok run --agent <id> -- '…'` when the dispatch carried a sub-agent id (T457).
pub(crate) fn strip_wrap(s: &str) -> String {
    let s = strip_run_wrap(s);
    if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        let inner = &s[1..s.len() - 1];
        // POSIX sh_quote embedding, or PowerShell doubled single-quotes (T55.4).
        if inner.contains("'\"'\"'") {
            inner.replace("'\"'\"'", "'")
        } else {
            inner.replace("''", "'")
        }
    } else {
        s.to_string()
    }
}

/// Drops one `rtok run` wrapper. `<id>` is the token `cmd` embeds: 1–64 bytes of ASCII
/// alnum, `_` or `-`. A lookalike stays intact, so it is not keyed as the inner command.
fn strip_run_wrap(s: &str) -> &str {
    let s = s.strip_prefix(POSIX_RUN_GUARD).unwrap_or(s);
    if let Some(rest) = s.strip_prefix("rtok run -- ") {
        return rest;
    }
    let Some(after) = s.strip_prefix("rtok run --agent ") else {
        return s;
    };
    let Some((id, rest)) = after.split_once(' ') else {
        return s;
    };
    if wrap_agent_id(id)
        && let Some(cmd) = rest.strip_prefix("-- ")
    {
        cmd
    } else {
        s
    }
}

/// Claude Code's MCP `read` tool name, and the PreToolUse matcher that injects the hidden
/// `agent` argument (T929.1). Gemini's spelling is `mcp_rtok_read`.
pub(crate) const MCP_READ: &str = "mcp__rtok__read";

/// Same shape as `cmd::hook::is_valid_agent_id`. Kept here so `guard`, `measure` and `read`
/// do not call back into `cmd` (that module already calls `guard::strip_cd_hop`).
pub(crate) fn wrap_agent_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Host names for rtok's MCP `read` (Claude, the retired plugin-scoped form, Gemini).
pub(crate) fn is_rtok_mcp_read(name: &str) -> bool {
    name == MCP_READ || name == "mcp_rtok_read" || name == "mcp__plugin_rtok_rtok__read"
}

/// Shortest archive-id prefix `rtok expand` resolves, and the one every pointer prints (T672).
pub(crate) const MIN_PREFIX: usize = 8;

/// T672: the id as a pointer prints it. Eight hex chars (32 bits) keep the link line to a
/// few tokens; the rare collision is answered by the ambiguity error in `expand`, which
/// names each candidate by a longer prefix, not by a longer id on every call.
pub fn short_id(id: &str) -> &str {
    &id[..id.len().min(MIN_PREFIX)]
}

/// Host tool names (`bash`, `read_file`, `edit`, …) to the Claude names `plugins::guard` matches.
pub(crate) fn canonical_tool_name(name: &str) -> String {
    let l = name.to_ascii_lowercase();
    if l == "exec"
        || l == "run_commands"
        || ["bash", "shell", "terminal", "powershell"]
            .iter()
            .any(|k| l.contains(k))
    {
        "Bash".into()
    } else if l.starts_with("read") || l.starts_with("view") {
        "Read".into()
    } else if l == "edit" || l == "replace" || l == "edit_file" {
        "Edit".into()
    } else if l == "write" || l == "write_file" {
        "Write".into()
    } else {
        name.to_string()
    }
}
