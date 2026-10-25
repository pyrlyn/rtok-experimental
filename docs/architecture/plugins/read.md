# read plugin

![The agent either calls the MCP read, search and tree tools, which resolve the path under a root guard and answer with an outline, a dedup pointer, a diff or a capped body, or triggers the PreToolUse Read hook, which denies a large native Read in favour of rtok read](read.svg)

Source: [`read.d2`](read.d2). Plugin docs: [README](../../../src/plugins/read/README.md). Index: [all diagrams](../README.md).

`read` replaces the host's file tools with three MCP tools (`read`, `search`, `tree`) whose
output is shaped for tokens: outlines instead of whole files, a one-line pointer for a file the
model already has, a diff for a file that changed, and a cut with an archive id for anything
large. A PreToolUse hook steers the host away from a native `Read` of a large file towards
those tools. Every shortening is lossless through `expand <id>`.

## Walkthrough

1. **A tool call arrives.** `rtok mcp` routes `read`, `search` and `tree` to
   `plugins::read::read`, `search::search` and `search::tree` (`src/mcp.rs`).
2. **The root guard runs first.** `resolve_with` normalises the path (following symlinks),
   refuses credential files when the server is reachable over HTTP, and accepts only paths under
   the cwd, `[plugins.read] allow_paths`, and (in `rtok mcp`) sibling worktrees and the client's
   `roots/list` roots (`roots::dynamic`). Anything else is an error that names the path.
3. **The mode picks the view.** `read_with` reads the file, then chooses by `mode` (default
   `[plugins.read] default_mode`): `full` or `lines` with `range` numbers the lines;
   `map` and `signatures` call `outline::render` (tree-sitter tags, markdown headings, or the first
   lines plus a note for an unknown language); `stripped` drops comment nodes
   (`outline::stripped`); `density:X` calls `density::render`, which keeps the lines with the
   rarest words; `diff` is `full` with the delta path forced.
4. **Re-read check.** The key is `cache::key(path, mode, range)`. `cache::hit` compares the new
   bytes with the archived copy named in `read_cache`. Equal bytes return `unchanged id <8-char id>`
   (a `dedup` row). Changed bytes return a unified diff plus `previous <id>` and `expand <id>` when
   the diff is smaller than `delta_max_ratio` of the file (a `delta` row).
5. **Session-wide dedup.** Otherwise `plugin::identical_result` looks for the same sha256
   anywhere in the session and returns a pointer instead of the body (a `dedup` row).
   `cache::remember` then archives the bytes and writes the `read_cache` row.
6. **Cap.** `cap` or `cap_recording` cuts a body longer than `max_chars`, archives the full text
   and leaves `archived <id>` in the cut. A `Measurement` is written for `map`, `signatures`,
   `density` and `stripped` (raw file against returned text), and for `read_cap`, `search_cap`
   and `tree_cap` when a cut happened; at most one row per call.
7. **Search and tree.** `search` and `tree` walk with `ignore::WalkBuilder` (`.gitignore`
   respected, `.git` and linked worktrees skipped) and return `path:line: snippet` rows or
   `path size` rows, capped by `search_max` / `tree_depth`, then through `cap_recording`.
8. **The hook advises.** On PreToolUse for `Read`, `hook::pre_tool` returns nothing unless
   `advice` is on and the file is larger than `native_max_bytes`. A native `Read` whose `limit`
   is within `range_max_lines` passes, because the host's `Edit` needs a native `Read`. If an
   `Edit` or `Write` of the path sits in the last five PostToolUse rows, the call passes, or, with
   `delta` on and a cached read, is denied towards `read(mode=diff)`. Otherwise it is denied with the
   host's exact name for rtok's `read` tool. Each deny writes a `read`/`deny` cost row.
9. **Cache invalidation.** `Plugin::post_tool` calls `cache::invalidate`, which clears the
   cached key for an edited path only when `delta` is off; with `delta` on the stale entry is what
   the diff is computed against.

## Where it lives

| Box | Code |
| --- | --- |
| rtok hook, rtok mcp | `src/hooks/`, `src/mcp.rs` (`read_file`, `search_files`, `tree_files`) |
| Read advice | `src/plugins/read/hook.rs` (`pre_tool`, `recently_edited`, `deny`, `WINDOW_TOOL_CALLS`) |
| root guard | `src/plugins/read/mod.rs` (`resolve_with`), `src/plugins/read/roots.rs` (`dynamic`, `client_roots`) |
| mode switch | `src/plugins/read/mod.rs` (`read_with`) |
| outline | `src/plugins/read/outline.rs` (`render`, `stripped`, `tags`) |
| density | `src/plugins/read/density.rs` (`parse`, `render`) |
| cache::hit, delta | `src/plugins/read/cache.rs` (`hit`, `remember`, `invalidate`, `DEDUP_PREFIX`) |
| identical_result | `src/plugin.rs` (`identical_result`) |
| cap | `src/plugins/read/mod.rs` (`cap`, `cap_recording`, `cap_impl`) |
| search / tree | `src/plugins/read/search.rs` (`search`, `tree`, `text_files`) |
| Manifest | `src/plugins/read/mod.rs` (`ReadPlugin`: `Surface::Mcp`, `Surface::Hook`) |
| Config | `crates/rtok-config/` (`[plugins.read]`) |
| Tables | `crates/rtok-store/` (`read_cache`, `archive`, `measurements`) |

## Invariants

- Root guard first: no filesystem access outside the allowed roots; `allow_paths = [""]` grants nothing.
- Lossless: a capped or deduplicated result always carries an id that `expand` resolves.
- A dedup answer is shorter than the body it replaces; `identical_result` returns nothing
  when the pointer would not be shorter.
- The hook fails open: a store error, an unknown host or an unreadable file lets the `Read`
  through, and a ranged native `Read` is never denied.
- Every deny is recorded as a cost, never a saving, so `rtok stats --plugin read` nets it.
- Tool descriptions stay within their estimated-token budget (test in the T4.1 set).

## Related

[Overview](../overview.md) · [Plugin contract](../plugin-contract.md) · [Hook flow](../hook-flow.md) ·
[MCP flow](../mcp-flow.md) · [archive](archive.md) · [graph](graph.md)
