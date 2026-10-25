# T973. Done

Scope as the creator narrowed it: only what T281 does not cover. Linking by PID and Codex session
lookup stay with T281.

- MCP `agent()` (`src/mcp.rs`): `Link::None` no longer answers a bare
  `not linked to an agent session`. That bare line was 58 of the 2026-10-22 MCP errors. With
  `[agents] enabled = false` it says the registry is off. Otherwise it says no live agent of the
  host ran a hook in this directory since the server started, and names the ways out: retry
  after the next tool call, `rtok doctor`, or the `agent` argument.
- `--agent <provider> / <model>` (`src/worktree/claim.rs` `caller`) says the flag wants an rtok
  agent id and names `rtok agents whoami`, instead of `unknown`.
- Ambiguous and `Outside` links already named the `agent` argument on main (T924). The
  `use RTOK_AGENT_ID` lines in the logs came from the installed alpha.2.

Checked: `cargo nextest run -p rtok --lib -E 'test(/mcp::|worktree::/)'`: 217 passed. That run
includes the new `an_agent_flag_holding_a_provider_and_model_says_it_wants_an_id` and three
whoami tests updated to the new texts.

PR: this branch.
