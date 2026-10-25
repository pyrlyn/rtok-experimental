---
status: todo
priority: P2
complexity: 2
readiness: 0%
agent:
---

# T979. `rtok doctor` probes MCP servers in parallel

From `ideas.md` I-94. `rtok doctor` probes each configured MCP server one after another (`src/doctor.rs`, `list_tools` per server), so one unreachable server costs a full `mcp_timeout_ms` and N of them cost N timeouts. The probes are IO wait, not CPU: run them on a `std::thread::scope` and join, with no new dependency. The output order stays the config order.

Done means: the worst case of the MCP section is one timeout, not N; the report is byte-identical to the sequential run for the same servers.

Check: a test with three fake MCP servers, one of which never answers, finishes in about one `mcp_timeout_ms` and prints the same section as before; `doctor` wall time before/after is recorded in `research.md`.
