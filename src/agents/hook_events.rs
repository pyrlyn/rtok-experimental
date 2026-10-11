// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The one table of hook events rtok registers per host (T390, T390.1): which host event runs
//! which `rtok hook <event>`, under which matcher. Every installer reads its event list from
//! here and `tests/hook_manifests.rs` checks every plugin's manifest or script hook against it,
//! so a manifest and an installer cannot quietly diverge (Cursor once lacked two events its
//! manifest never carried, while `done.md` said it had them). Row order is the order an
//! installer writes its entries in, and so the order of the host's config file.

/// One host event and the `rtok hook <rtok_event>` it runs.
pub struct HookEvent {
    pub host: &'static str,
    pub host_event: &'static str,
    /// What follows `rtok hook`. Claude-shaped hosts pass the host's own event name here, so it
    /// equals `host_event` for them.
    pub rtok_event: &'static str,
    /// The host's tool matcher for this entry; empty omits the field. A host event may take
    /// several rows, one per matcher.
    pub matcher: &'static str,
    /// The host's own installer writes this row into the host's config. `false` marks a row only
    /// the plugin manifest carries; Claude's installer lists are read from its manifest instead.
    pub installer: bool,
}

const fn matched(
    host: &'static str,
    host_event: &'static str,
    rtok_event: &'static str,
    matcher: &'static str,
    installer: bool,
) -> HookEvent {
    HookEvent {
        host,
        host_event,
        rtok_event,
        matcher,
        installer,
    }
}

const fn row(
    host: &'static str,
    host_event: &'static str,
    rtok_event: &'static str,
    installer: bool,
) -> HookEvent {
    matched(host, host_event, rtok_event, "", installer)
}

/// Cursor's `subagentStart` stays out on purpose: its output schema has no context field, so a
/// hook there could not deliver the spawn brief (research.md §23). `beforeSubmitPrompt` stays out
/// for the same reason (T390.1): its output is only `{continue, user_message}`
/// (https://cursor.com/docs/hooks, checked 2026-10-04), so nothing it returns reaches the model,
/// and `sessionStart` already registers the agent.
pub const HOOK_EVENTS: &[HookEvent] = &[
    matched("claude", "PreToolUse", "PreToolUse", "Bash", false),
    matched("claude", "PreToolUse", "PreToolUse", "Read", false),
    matched("claude", "PreToolUse", "PreToolUse", "Skill", false),
    matched("claude", "PostToolUse", "PostToolUse", "*", false),
    row("claude", "UserPromptSubmit", "UserPromptSubmit", false),
    row("claude", "SessionStart", "SessionStart", false),
    row("claude", "PreCompact", "PreCompact", false),
    row("claude", "PostCompact", "PostCompact", false),
    row("claude", "SessionEnd", "SessionEnd", false),
    row("claude", "SubagentStart", "SubagentStart", false),
    // CodeBuddy takes Claude's rows minus SubagentStart: its hooks page (checked 2026-10-11,
    // https://www.codebuddy.ai/docs/cli/hooks) only names that event, with no input or output
    // shape to deliver the spawn brief through. The installer writes every row (T560.2).
    matched("codebuddy", "PreToolUse", "PreToolUse", "Bash", true),
    matched("codebuddy", "PreToolUse", "PreToolUse", "Read", true),
    matched("codebuddy", "PreToolUse", "PreToolUse", "Skill", true),
    matched("codebuddy", "PostToolUse", "PostToolUse", "*", true),
    row("codebuddy", "UserPromptSubmit", "UserPromptSubmit", true),
    row("codebuddy", "SessionStart", "SessionStart", true),
    row("codebuddy", "PreCompact", "PreCompact", true),
    row("codebuddy", "PostCompact", "PostCompact", true),
    row("codebuddy", "SessionEnd", "SessionEnd", true),
    row("cline", "PreToolUse", "PreToolUse", true),
    row("cline", "PostToolUse", "PostToolUse", true),
    row("cline", "TaskStart", "TaskStart", true),
    row("cline", "UserPromptSubmit", "UserPromptSubmit", true),
    row("cline", "SessionEnd", "SessionEnd", true),
    matched(
        "commandcode",
        "PreToolUse",
        "PreToolUse",
        "SHELL|READ|WRITE|EDIT",
        true,
    ),
    matched("commandcode", "PostToolUse", "PostToolUse", ".*", true),
    row("commandcode", "SessionStart", "SessionStart", true),
    row("commandcode", "Stop", "SessionEnd", true),
    row("codex", "PreCompact", "PreCompact", false),
    row("codex", "PostCompact", "PostCompact", false),
    row("copilot", "preToolUse", "PreToolUse", true),
    row("copilot", "postToolUse", "PostToolUse", true),
    row("copilot", "userPromptSubmitted", "UserPromptSubmit", true),
    row("copilot", "sessionStart", "SessionStart", true),
    row("copilot", "sessionEnd", "SessionEnd", true),
    row("copilot", "preCompact", "PreCompact", true),
    row("copilot", "subagentStart", "SubagentStart", true),
    row("cursor", "beforeShellExecution", "PreToolUse", true),
    row("cursor", "afterShellExecution", "PostToolUse", true),
    row("cursor", "sessionStart", "SessionStart", false),
    row("cursor", "sessionEnd", "SessionEnd", true),
    row("cursor", "preCompact", "PreCompact", true),
    row("cursor", "afterMCPExecution", "AfterMCPExecution", false),
    row("cursor", "postToolUse", "PostToolUse", false),
    matched("devin", "PreToolUse", "PreToolUse", "^exec$", true),
    matched("devin", "PreToolUse", "PreToolUse", "^read$", true),
    row("devin", "PostToolUse", "PostToolUse", true),
    row("devin", "UserPromptSubmit", "UserPromptSubmit", true),
    row("devin", "SessionStart", "SessionStart", true),
    row("devin", "PostCompaction", "PostCompaction", true),
    row("devin", "SessionEnd", "SessionEnd", true),
    row("gemini", "BeforeTool", "PreToolUse", true),
    row("gemini", "AfterTool", "PostToolUse", true),
    row("gemini", "BeforeAgent", "UserPromptSubmit", true),
    row("gemini", "SessionStart", "SessionStart", true),
    row("gemini", "SessionEnd", "SessionEnd", true),
    row("gemini", "PreCompress", "PreCompact", true),
    // Qwen documents Claude's event names and accepts Bash/Read/Skill as exact
    // tool aliases; `*` matches every tool. Gemini's BeforeTool rows would be
    // skipped as an invalid event name (research.md §32.2).
    matched("qwen", "PreToolUse", "PreToolUse", "Bash", true),
    matched("qwen", "PreToolUse", "PreToolUse", "Read", true),
    matched("qwen", "PreToolUse", "PreToolUse", "Skill", true),
    matched("qwen", "PostToolUse", "PostToolUse", "*", true),
    row("qwen", "UserPromptSubmit", "UserPromptSubmit", true),
    row("qwen", "SessionStart", "SessionStart", true),
    row("qwen", "PreCompact", "PreCompact", true),
    row("qwen", "PostCompact", "PostCompact", true),
    row("qwen", "SessionEnd", "SessionEnd", true),
    row("qwen", "SubagentStart", "SubagentStart", true),
    row("grok", "PreToolUse", "PreToolUse", false),
    row("grok", "PostToolUse", "PostToolUse", false),
    row("grok", "UserPromptSubmit", "UserPromptSubmit", false),
    row("grok", "SessionStart", "SessionStart", false),
    row("grok", "PreCompact", "PreCompact", false),
    row("grok", "PostCompact", "PostCompact", false),
    row("grok", "SessionEnd", "SessionEnd", false),
    matched("kimi", "PreToolUse", "PreToolUse", "Bash", true),
    matched("kimi", "PreToolUse", "PreToolUse", "Read", true),
    matched("kimi", "PreToolUse", "PreToolUse", "Skill", true),
    matched("kimi", "PostToolUse", "PostToolUse", "*", true),
    row("kimi", "UserPromptSubmit", "UserPromptSubmit", true),
    row("kimi", "SessionStart", "SessionStart", true),
    row("kimi", "PreCompact", "PreCompact", true),
    row("kimi", "PostCompact", "PostCompact", true),
    row("kimi", "SessionEnd", "SessionEnd", true),
    matched("zcode", "PreToolUse", "PreToolUse", "Bash", true),
    matched("zcode", "PreToolUse", "PreToolUse", "Read", true),
    matched("zcode", "PostToolUse", "PostToolUse", "*", true),
    row("zcode", "UserPromptSubmit", "UserPromptSubmit", true),
    row("zcode", "SessionStart", "SessionStart", true),
];

/// Every row of `host`, plugin-only ones included.
pub fn for_host(host: &str) -> impl Iterator<Item = &'static HookEvent> + '_ {
    HOOK_EVENTS.iter().filter(move |e| e.host == host)
}

/// The rows `host`'s own installer writes to its config.
pub fn installer_rows(host: &str) -> impl Iterator<Item = &'static HookEvent> + '_ {
    for_host(host).filter(|e| e.installer)
}

/// `(host event, rtok event)` for what `host`'s installer writes to its own config.
pub fn installed(host: &str) -> impl Iterator<Item = (&'static str, &'static str)> + '_ {
    installer_rows(host).map(|e| (e.host_event, e.rtok_event))
}

/// `(host event, matcher)` for every row of a Claude-shaped host (hooks keyed by event, each
/// with an optional matcher): the list `claude::insert_ours` and `strip_ours` walk.
pub fn entries(host: &str) -> Vec<(&'static str, &'static str)> {
    for_host(host).map(|e| (e.host_event, e.matcher)).collect()
}
