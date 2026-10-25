---
status: in progress
priority: P2
complexity: 2
readiness: 100%
agent: Claude Code / claude-sonnet-5-5
---

# T930. Read gate denies native Read for files `rtok read` cannot serve (images, paths outside roots)

The PreToolUse(Read) gate denied native Read over 32 KB 703 times in the rtok project alone
(by design), but 87 of those were paths outside the project (scratchpad logs, `tasks/*.output`,
`tool-results`, `~/.cargo/registry`) and 13 were PNG/PDF screenshots. `mcp__rtok__read` then
refuses the same path (`path outside cwd`), or cannot show an image, so the agent falls to
`cat`/`sed` and gets shortened again. The deny text "use rtok read" also sends agents to a
non-existent `rtok read` CLI (11 calls) and is shown as a "hook error".

Done when: the gate lets native Read through for images, PDFs and paths `rtok read` would
refuse, and its text names `mcp__rtok__read`.

Check: read-hook tests for an image, a scratchpad path and an in-project large file (still
denied); `just check`.

Plan: in `src/plugins/read/hook.rs`, after the size check, pass the native Read when the path
has an image or PDF extension, or when the MCP `read` root guard (`plugins::read::resolve`
with cwd and `allow_paths`) refuses it. Tests: image, outside-root scratchpad path, in-project
large file still denied; existing deny tests keep their files reachable via `allow_paths`.
The deny text already names `mcp__rtok__read` on Claude (T384); `rtok read` remains only for
hosts with unknown tool naming.
