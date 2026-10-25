---
status: in progress
priority: P2
complexity: 3
readiness: 85%
agent: Cursor / grok 4.6
---

# T929.1. Tell MCP `read` which context window is calling

T929 stops MCP `read` from answering with a stub, a delta or a remembered body once the session
has a sub-agent, because the tool cannot see its caller. That fallback also costs the parent its
own legitimate re-read stubs after the first sub-agent starts. Give the tool the caller: a
PreToolUse hook on the host's MCP `read` tool adds the hidden `agent` argument (the sub-agent id
the hook payload carries, as the `--agent` flag of the Bash rewrite does), and MCP `read` scopes
its re-read cache, delta and `identical_result` lookups to `(session, agent)`. The T929 fallback
stays for a call that arrives without the argument (a host without the matcher, an older plugin).

Done when: a parent and a sub-agent each get a stub for their own repeat read and the whole body
for a read only the other made, with the argument present; without it the T929 behaviour holds.
The hook matcher is installed and checked by `agents install` and `doctor` for the hosts that
can rewrite an MCP call's input, and the hidden argument stays out of the tool's visible schema
or is documented as internal.

Check: MCP and hook tests with and without the `agent` argument; `just check`.

Execution plan:
1. Reuse `names::wrap_agent_id` (same shape as Bash `--agent`), `Ctx::with_agent`,
   `identical_result`/`put_archive_for` context, and `hook_events` (the one table installers
   and `tests/hook_manifests.rs` read). Do not add a second installer, move MCP, or put `agent`
   on the visible `read` schema (T924's `with_agent` is a different id).
2. PreToolUse on `mcp__rtok__read` / `mcp_rtok_read` rewrites `updatedInput` with `agent` from
   the payload (empty string = parent). Malformed ids are dropped (fail open). Matcher row only
   for hosts that honour MCP `updatedInput` (Claude, CodeBuddy); Gemini's BeforeTool has no
   matcher so it already fires. Kimi's `updatedInput` is unverified — leave it off.
3. MCP `read`: key present → skip the T929 `shared_session` fallback and scope cache / delta /
   `identical_result` to `(session, agent)`; key absent → T929.
4. Tests first (hook inject; parent vs sub-agent stubs; no-arg fallback), then `just check`.
