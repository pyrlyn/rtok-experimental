# `read`

Five MCP tools instead of seventy-eight, and no per-turn banner.

| | |
|---|---|
| Surfaces | MCP `read`, `search`, `tree`; PreToolUse(Read) advice |
| Spec | the **Replaces** column of [`docs/plugins.md`](../../../docs/plugins.md) |
| Default | on |

## Tools

- `read(path, mode=full|lines|map|signatures|diff|stripped|density:X, range?)` — numbered lines; `map` and
  `signatures` come from tree-sitter tags queries (Rust, TS, JS, Python, Dart, C, Go);
  `stripped` drops comment nodes (same grammars; unknown language or parse fail → `full`);
  unknown language for `map`/`signatures` → first 60 lines + note. Output over 20 K chars → head/tail + an 8-char archive id (`expand` resolves it, T908).
  `density:X` (0 < X ≤ 1, T670) keeps the most informative lines, by word rarity in the file, until about X of the bytes remain; line numbers are kept, `… N lines omitted` marks gaps (fetch them with `lines`).
  `diff` is the edit → verify form of the automatic delta below.
- `search(pattern, path, max=50)` — regex over files respecting `.gitignore`;
  `path:line: snippet` (≤ 120 chars).
- `tree(path, depth=2)` — compact listing with sizes.
- Re-read dedup: same session, same path, same sha256, same mode/range →
  `unchanged id <archive_id>` (8-char prefix; `expand` resolves it, T672/T906; ≤ 23 estimated tokens on the T58.1 fixture).
- Changed re-read (T58.1): a later `read` of a file that changed since the last archive
  returns a unified diff plus `previous <id>` / `expand <id>` of the full file, when the
  diff is below `delta_max_ratio` of the file (default 0.6). Measured 2026-09-18
  (`rtok stats --since 90d`, 959 sessions): 593 such re-reads, **7.3 %** of Read bytes.
- PreToolUse(Read) advice: native `Read` of a file > 32 K that was not edited in the last
  5 tool calls is denied with "use rtok read; before Edit run native Read(limit=1)", where
  `rtok read` is the host's exact MCP tool name (T384: `mcp__rtok__read` plus the
  `ToolSearch select:` line that loads it on Claude Code, `mcp_rtok_read` on Gemini CLI; any
  other host keeps `rtok read`). After an Edit of a file already in the read cache, the deny
  points at `read(mode=diff)` instead.
  Never for files under 32 K, and never for a native `Read` with `limit` 1..=`range_max_lines`
  (300; T127, T383): the host's `Edit` wants a native `Read` first and an MCP `read` does not
  count, so a ranged Read is the edit gate. Never for a path MCP `read` cannot serve (T930):
  an image or PDF, or a path its root guard refuses (scratchpad logs, task outputs,
  `~/.cargo/registry`), where a deny only pushes the agent to `cat`/`sed`. Every deny records
  a `read`/`deny` cost row.

Root guard: paths must be under cwd or `allow_paths`. In `rtok mcp` (T351) every worktree of the
cwd's repository (`git worktree list`, re-read at most every 30 s, only after a path failed the
cheap checks) and every `file://` root of the client's `roots/list` answer count too; host
scratchpads and any other directory stay outside, so `allow_paths` is the escape hatch.
The refusal names both ways out: the host's own file tool and `allow_paths` (T972). A missing
file under a directory named like one of those worktrees (an old worktree root) names where the
same file is now.

## Config

```toml
[plugins.read]
enabled = true
native_max_bytes = 32768     # PreToolUse(Read) deny threshold
range_max_lines = 300        # T383: native Read with limit 1..=N passes
allow_paths = []             # extra roots outside cwd
delta = true                 # T58.1: changed re-read → unified diff (7.3 % of Read bytes)
delta_max_ratio = 0.6        # full file when the diff is not below this fraction
```

## Tasks

See `roadmap.md` § `read`. Checks in `plan/`.

T4.2 full/lines · T4.3 map/signatures · T4.4 dedup · T4.5 search + tree · T4.6 Read advice · T50.3 stripped.

## Status

Manifest only. First task: T4.2 (after T4.1 `rtok mcp`).
