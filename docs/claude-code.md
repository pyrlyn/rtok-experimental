# Claude Code setup

What `rtok agents install claude` writes, how to update and remove it, and how to run the proxy next to it. Other hosts: [agents.md](agents.md).

## Start with Claude Code

Install rtok's eight hooks and MCP entry. The installer backs up the settings file before
writing it; inspect its changes first if preferred.

```bash
rtok agents install claude --dry-run
rtok agents install claude
rtok doctor
```

`--dry-run` prints exactly what it would write and touches nothing:

```text
+ PreToolUse Bash rtok hook PreToolUse
+ PreToolUse Read rtok hook PreToolUse
+ PostToolUse * rtok hook PostToolUse
+ UserPromptSubmit rtok hook UserPromptSubmit
+ SessionStart rtok hook SessionStart
+ PreCompact rtok hook PreCompact
+ PostCompact rtok hook PostCompact
+ SessionEnd rtok hook SessionEnd
8 additions
```

Each app gets one block: kind and name (`CLI: Claude Code`, `Desktop: Cursor`),
install path and version, config files touched, and the state of every rtok
module (`hooks`, `mcp`, `proxy`, `plugin`) as `✓ installed`,
`✗ not installed (--flag)`, or `− not supported: why`. rtok's own plugins are
listed the same way from the surfaces each one declares (`(off)` when disabled).

`claude` covers both Claude Code and Claude Desktop (Desktop is MCP only, in
`claude_desktop_config.json`). A second run of the same command says
`already installed` instead of repeating a diff. `rtok agents list` prints the
same blocks without writing; `rtok doctor` lists modules for every host under
`agents`.

In a folder Claude Code has not trusted ("This workspace isn't trusted"), it skips the
project's hooks and MCP servers, so rtok does nothing there. `rtok doctor` reports that as
`untrusted folder`, naming the folder Claude Code keys the trust on: the repository root, the
main checkout for a worktree. Trust it by running `claude` there and accepting the dialog, or
in the desktop app; `doctor` never sets the flag itself.

After upgrading rtok, `rtok agents update` refreshes every host rtok is
installed in (or only the hosts named): a hook or MCP entry written by an older
binary path or with an old timeout is rewritten in place, and a
host that has nothing of rtok is skipped rather than installed into.

Plugin versions and updates: [docs/plugin-versions.md](plugin-versions.md)

`rtok agents uninstall claude` takes it all back out: hook entries, MCP
registration, and the proxy variable. Both install and uninstall copy every file
they touch to `.backup_configs/<name>.bak-<ts>` first; a no-op run leaves no copy,
and identical content is never backed up twice. After a write, each requested
module is read back — a module that did not land is a warning, as is an
`rtok` that is not on `PATH`.

Run the proxy separately when you want provider usage rows and archive compression:

```bash
rtok proxy --mode passthrough
# In another shell, point the host at http://127.0.0.1:8790.
```

Two ways to shrink old tool results on the Anthropic wire, measured on the same
six-turn request (103 729 bytes upstream-bound; `tests/proxy.rs`
`proxy_compress_rewrites_old_tool_results_identically` and
`proxy_anthropic_context_edits_arm_platform_path`):

| path | upstream body | local rows |
| --- | --- | --- |
| `archive` (`mode = "compress"`) | 70 837 B (−31.7 %) | 4 `archive` Measurements, one per rewritten block |
| platform (`[proxy] context_management = true`) | 103 798 B (+69 B field) | 1 zero-delta `proxy/context_management` row naming the path; `archive` stands down (0 rows) |

The platform's own clearing happens server-side and is not locally observable, so by
D3 no saving is claimed for it — only the added field is measured. Keep `archive`
where the platform cannot clear (the OpenAI wires); where both apply, the platform
wins and `archive` skips those turns rather than shrinking twice.
