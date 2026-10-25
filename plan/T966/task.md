---
status: in progress
priority: P2
complexity: 3
readiness: 10%
agent: Claude Code / claude-opus-5-5
---

# T966. One agent lookup, and ask for the path when it finds nothing

The creator's request. rtok finds a host app (Claude Code, Codex, Cursor, …) in several ways
that drift apart: `agents::app_path`/`present` (bundle, then PATH), direct
`find_on_path("claude"|"codex"|"codebuddy")` checks inside host modules, `host_program`
for `run_cli` and `app_version`, and `doctor::detected_hosts`, which looks at config folders
(`~/.cursor`, `~/.config/opencode`, a Claude `settings.json`) instead of the app (T426 says a
folder alone does not count). None of them can be told where an app lives when it is not
where rtok looks, so `agents install <host>` just says "<host> is not installed".

Done: one function (`agents::locate`) is the only agent lookup, and every caller above uses
it; it reads an optional user path first (`[setup.paths.<host>] cli` / `desktop`, own TOML
through the config module, T238), then the app bundles, then PATH. When `agents install`,
`agents update <host>` or `agents info <host>` finds no app on an interactive terminal, rtok
asks for the path, validates it, shows what it resolved (agent, program path, config files),
asks "Is this correct? [y/N]", re-runs the command with it on yes, asks again on no, and
aborts cleanly on an empty answer; then it asks whether to save the path to the config file
(the creator's decision: a key, saved only on yes). Without a terminal (pipes, hooks, MCP,
`agents list`, `doctor`) it never asks: the error names the config key to set. Generic PATH
lookups of non-agent programs (`rtok`, `rtok-hook`, hook commands, LSP servers) stay on
`find_on_path`.

Check: unit tests for `locate` (user path, bundle, PATH, missing user path) and for the
prompt flow over scripted stdin (yes, no then yes, empty abort, invalid path, save yes/no);
`just check` green, including the config gates for the new key.

Plan:

1. `crates/rtok-config`: `SetupPath { cli, desktop }` section and `Setup.paths:
   BTreeMap<String, SetupPath>`; document the key in `default.toml`.
2. `src/agents/locate.rs`: `locate`, `present`, `program` (what to spawn), `cli`,
   `resolve_input` (typed path → variant), and the `NotFound` error naming the key.
3. Switch callers: `absent_hosts`, `block`, `apply`/update loops, `restart.rs`, `mcp.rs`,
   `mcp/ping.rs`, `model::*`, `run_cli` and the `find_on_path(host)` checks in
   `claude`/`codex`/`codebuddy`, `app_version`, `doctor::detected_hosts`.
4. Prompt flow over `BufRead`/`Write` (testable), wired into the `agents install|update|info`
   handlers in `src/cli/` behind a stdin+stdout terminal check; save through
   `validate::set_with` after a second yes. Reuse the SDK's y/N answer rule.
5. Tests; docs for the new key (`docs/`, ru/uk if the key list is translated); `just check`.
