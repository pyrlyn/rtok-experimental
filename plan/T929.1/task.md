---
status: todo
priority: P2
complexity: 3
readiness: 0%
agent:
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
