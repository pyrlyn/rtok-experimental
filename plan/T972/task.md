---
status: in progress
priority: P2
complexity: 2
readiness: 100%
agent: Claude Code / claude-opus-5-5
---

# T972. MCP `read` and `outline` reach host skills, scratchpads and parent instruction files

About 40 `mcp/read: path outside cwd` errors on 2026-10-22: `~/.agents/skills/*/SKILL.md`,
`/private/tmp/claude-*` scratchpads, and the parent `AGENTS.md` / `CLAUDE.md` above the repo.
About 10 more reads used the old `_worktrees/` path. `plugins.read.allow_paths` exists, but its
defaults do not cover these. T930 (not in the installed alpha.2) may cover the scratchpads.

Done when: on main, first list which of these paths still fail. Then add the rest to the default
roots, read-only: host skill dirs, the host scratchpad pattern, and `AGENTS.md` / `CLAUDE.md` in
the repo's ancestors. A missing path under a moved worktree root names the current path. The
error for other outside paths names `allow_paths`.

Check: `read` / `outline` tests over a `Vfs` for each root and for the moved-path hint; `just check`.

Plan (scope narrowed by the creator on 2026-10-23: hints only, the default roots stay as they
are, since `src/plugins/read/README.md` keeps scratchpads outside on purpose and T930 already
lets the native Read through for them):
1. `resolve_with` (`src/plugins/read/mod.rs`): the outside-cwd refusal names `allow_paths` next
   to the host's own file tool.
2. `roots::moved_hint`: a missing absolute path with a directory named like a known worktree or
   client root, where the same file exists, gets ` (moved? it exists at <path>)`. `read_with` and
   `graph::outline_in` append it. It returns nothing unless `rtok mcp` enabled the roots, so
   hooks never spawn git.
3. A unit test in `roots.rs` over a real git repo with a linked worktree (git worktrees cannot be
   faked in `Vfs`); the read README says what the errors name.
