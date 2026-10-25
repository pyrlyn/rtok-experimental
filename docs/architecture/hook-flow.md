# Hook flow

![One hook call: the host runs rtok hook, the input is guarded, adapted and dispatched by event to the enabled plugins, and merged output goes back on stdout](hook-flow.svg)

Source: [`hook-flow.d2`](hook-flow.d2). Index: [all diagrams](README.md).

A host runs `rtok hook <event>` once per hook event, with the event's JSON on stdin. rtok
reads it, lets every enabled plugin react, merges what they return into one host-shaped
JSON object on stdout, and exits 0. Every failure path prints `{}`, so the host carries on as
if rtok were not installed (fail open, D1).

## Walkthrough

1. **Entry.** The host either starts `rtok hook <event> --host <h>` directly
   (`src/cli/hook.rs` → `hooks::run`) or starts the std-only `rtok-hook` client, which hands
   the call to the resident `rtok hook --serve` over a Unix socket or named pipe
   (`src/hooks/resident.rs`, D32). A refused or absent resident makes the client run
   `rtok hook` itself, so the resident is never required. Both paths end in `hooks::run`.
2. **Guard.** `hooks::run` reads at most `core.hook_max_input_bytes` and wraps the dispatch
   in `catch_unwind`. Oversized stdin, a read error or a panic logs the error and prints `{}`.
   With `hook.fail_open = false` (debugging only) the same cases panic instead.
3. **Adapt.** `dispatch_owned_strict` parses `HookInput` and rewrites a foreign host's payload
   into Claude's field names (`adapt_grok`, `adapt_cursor`, `adapt_gemini`, …), resolves the
   session id (stdin, then `$<core.session_env>`), and opens the `Runtime` (config + store).
   A store that stays locked past the wait passes the call through and defers `SessionEnd`.
4. **Record the call.** `hooks::dispatch` builds the `Registry`, registers or touches this
   session's agent row (`register_agent`), and writes one `calls` row for the event.
5. **Route by event.**
   - `PreToolUse` → `pre_tool`: each plugin may return `Deny` or `Rewrite`. The first `Deny`
     returns at once; otherwise the last `Rewrite` wins (`updatedInput`).
   - `PostToolUse` → `post_tool`: plugins can only add context. Strings are joined, pending
     agent messages go first, and `cap_budget` trims the result to
     `plugins.inject.budget_tokens`, leaving an `rtok expand` pointer for what it dropped.
   - `SessionStart`, `UserPromptSubmit`, `SubagentStart`, `PostCompact` → `inject_event`:
     every `Injection` (plus the agent id and claimed-worktree notes) goes to
     `inject::apply`, which orders them by priority and emits until its budget.
   - `PreCompact` → every plugin's `pre_compact`, then `memory::compact_context` as context.
   - `SessionEnd` → `checkpoint::save_session`, `end_agent` (which also unlocks the agent's
     worktree) and `end_session`.
6. **Isolate each plugin.** Every trait call runs inside its own `catch_unwind`; a panicking
   plugin is logged (`log_panic`) and dropped, the others still count.
7. **Answer.** The merged `HookOutput` is serialised; `copilot_output`, `cursor_output`,
   `gemini_output`, `codex_output` and friends reshape it for hosts with another envelope.
   The call's duration and its stdin/stdout go to `calls` / `call_io`; `Stop` and
   `SessionEnd` also start the OTLP export child.

## Where it lives

| Box | Code |
| --- | --- |
| `rtok hook <event>` | `src/cli/hook.rs`, `src/hooks/mod.rs` (`run`) |
| Resident and client | `src/hooks/resident.rs`, `crates/rtok-hook/` |
| Adapt | `src/hooks/mod.rs` (`dispatch_owned_strict`), `src/hooks/types.rs` (`HookInput::adapt_*`) |
| Dispatch | `src/hooks/mod.rs` (`dispatch`, `pre_tool`, `post_tool`, `inject_event`, `cap_budget`) |
| Agent messages pushed into context | `src/hooks/push.rs` |
| Injection budget | `src/plugins/inject/` (`apply`) |
| Ledger | `crates/rtok-store/` (`record_call`, `insert_hook_call_io`, `register_agent`) |

## Invariants

- Exit 0 within the 10 ms budget on every path; a hook never blocks the agent.
- PostToolUse adds context only; a tool result cannot be replaced from a hook on Claude Code.
- One plugin's panic never removes another plugin's output.
- A duplicate delivery of one tool call (plugin hook plus a leftover settings hook) is recorded
  once: `Runtime::once` keys measurements by `<event>:<tool_use_id>`.

## Related

[Overview](overview.md) · [Plugin contract](plugin-contract.md) · [Store](store.md) ·
[inject](plugins/inject.md) · [cmd](plugins/cmd.md) · [MCP flow](mcp-flow.md) ·
[Proxy flow](proxy-flow.md)
