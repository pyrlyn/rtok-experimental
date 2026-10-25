# `memory`

One memory instead of two, with zero LLM cost: notes the agent writes, SQLite FTS5 search,
progressive disclosure.

| | |
|---|---|
| Surfaces | MCP `mem_save`, `mem_search`, `mem_pack`, `mem_get`, `mem_update`; PreCompact checkpoint; SessionStart recall |
| Spec | the **Replaces** column of [`docs/plugins.md`](../../../docs/plugins.md) |
| Default | on |

## Tools

- `mem_save(kind, title, body, project?)` — project defaults to the git root name of cwd.
  Admission gate (T671, `plugins.memory.admit`): a body under `admit_min_chars`, or `admit_similarity` percent
  trigram-alike to a stored note of the project, is refused with `{"refused": "too_short"|"duplicate", "id": <stored>}`.
  Same project+kind+title is a revision and always passes. Local text only; the `remember:` prompt path is not gated.
  The title is the topic key: a second save with the same project, kind and title updates
  that row (`{"id", "updated": true}`) instead of adding a stale twin to recall (T66.1).
  An explicit re-save of a retired topic revives it (clears the tombstone).
  `<private>`…`</private>` is stored as `[REDACTED]`. A body change keeps the previous
  text for `rtok memory history <id>`; SessionStart recall still shows the current title only.
- `mem_search(query, limit=5)` — ids, titles, 120-char snippets ranked by FTS5 `bm25`, re-ranked by use, age, pin and current project.
  Retired notes never appear.
- `mem_pack(query, limit=8, max_tokens=400)` — the same ranking, packed into one
  answer. Each hit starts at its abstract (title and snippet). Leftover budget
  deepens the best hits to the first paragraph, then the body. A tier that does
  not fit is skipped whole. `limit` is 1–20, `max_tokens` is 1–2000. The hook
  index is unchanged; this tool does not replace `mem_get`.
- `mem_get(id)` — full body. A retired note still returns its body, prefixed by one line
  `retired <ts>[, superseded by <id>]`.
- `mem_update(id, retire?, superseded_by?, pinned?)` — the lifecycle (T69.1): `retire`
  tombstones the id (never deletes), `superseded_by` names the replacement, `pinned`
  pins/unpins. The same operations run as `rtok memory retire|pin|unpin` and
  `rtok memory revise <id> --title --body` (revise = save the replacement through the
  `mem_save` path, then retire the old id naming it).
- `rtok memory history <id>` — the earlier title and body, oldest first, kept when an
  upsert changed the note (T472). `checkpoint:*` and `session:*` are not kept. Recall
  and `mem_get` stay on the current body. There is no MCP tool for history.

## Lifecycle

Retire, supersede, pin — never delete (D4). A retired note is skipped by recall and search
but keeps its body readable through `mem_get`; pinned notes lead the SessionStart recall
ahead of newest-first order; both orders are byte-stable for an unchanged store.

`search` takes the best bm25 hits and re-ranks them by use, age and pin (`rank_blend`): a
pinned or well-read note overtakes a neighbour with equal text, a clearly better match stays
ahead, and a store with no use signal ranks as before. `rtok memory forget` (a dry run unless
`--apply`) retires unpinned, never-read notes older than `forget_after_days`; `user` and
`decision` notes, checkpoints and sessions are never forgotten, and a retired body stays readable.

## Hooks

- PreCompact: extracts the last 20 user prompts (≤ 300 chars each), touched file paths and
  last error lines from the transcript into a `checkpoint` note. A prompt is what the human
  typed (T417): records the host injected are skipped — `isMeta` (skill bodies, sub-agent
  hand-backs), `isCompactSummary`, an `origin.kind` other than `human` (task
  notifications, peer messages), and text that opens with a host envelope
  (`<task-notification>`, `<ci-monitor-event>`, `<local-command-…>`, `<agent-message`,
  `[Request interrupted by user`). `<system-reminder>` blocks are cut out of a typed
  prompt. The same rule applies to the SessionEnd note and `handoff`, which reuse the
  extractor.
- Prompt quality (T419): each PreCompact and SessionEnd checkpoint also counts, over the
  whole transcript, the user-text records taken as typed prompts and the ones skipped as
  host-written. The counts go to plugin state (`kv` key `plugin:memory:<checkpoint kind>`
  via `Host::plugin_state_set`), never to the Measurement ledger and never into the note,
  so the restore stays byte-identical and no saving is claimed. The web and TUI Plugins
  pages show the totals as `checkpoint prompts typed` and `checkpoint host records
  skipped`; a session with both a PreCompact and a SessionEnd row counts once, by the
  larger row.
- SessionStart with `source == "compact"` (and PostCompact on Codex and Devin; Claude Code's
  PostCompact takes no context, T295): injects the latest checkpoint
  (≤ 400 tokens) through `inject`.
- SessionStart recall: last 5 note titles + ids for the project (≤ 200 tokens, priority 10),
  never bodies.

## Import and export

`rtok memory import <file.jsonl>` reads one note per line (`{kind, title, body, ts?, project?}`),
deduped by body sha256. Export your previous memory tool to that shape yourself; rtok knows
no third-party schema (D6).

`rtok memory export [--project <name>]` prints the same shape from the store — every note
but the session-local `checkpoint:*` rows — so notes move between machines through a file
you commit or copy (T66.2). An export piped into `import` on a second store inserts each
row once; a second import skips them all.

## Handoff

`handoff` is an MCP tool (`[plugins.memory] handoff = true`) that returns a budgeted session digest for sub-agents (T59.6). Set `handoff = false` to turn it off.

## Sync

`rtok memory sync [--file CLAUDE.md|AGENTS.md] [--budget N] [--dry-run] [--remove] [--force]`
writes pinned notes first, then remaining live notes by id desc, as `id title` lines between
`<!-- rtok:memory -->` / `<!-- /rtok:memory -->`. The block is ≤ `[plugins.memory] sync_tokens`
(default 300), byte-stable for an unchanged store, and created at the end of the file when
absent. `--remove` deletes only the block. A hand-edited block is refused unless `--force`.
This command is the only writer — no hook writes the file (fail-open). When the block exists
and hook recall is on, `sync` and `rtok doctor` print the same T59.7 overlap line.

## Recall bench (T69.3)

`cargo test --test memory_bench -- --nocapture` (2026-09-18). Seeded in-memory store, 20 planted facts, 5 revised. FTS5 and P29 hybrid both 20/20 at N=1/10/30/100; superseded returned 0. SessionStart recall is 95–100 bytes against 6 331–371 866 bytes of full live-body injection. `half_life_days` is N/A (T69.2 shipped no scorer). Numbers and the command live in `research.md` §14; never graymatter's 83 %.

## Tasks

See `roadmap.md` § `memory`. Checks in `plan/`.

T6.1 notes API · T2.5 checkpoint · T6.2 recall · T6.3 import · T66.1 upsert · T66.2 export · T69.1 lifecycle · T69.3 recall bench · T69.6 sync.

## Status

Manifest only. Schema (`notes`, `notes_fts`) exists since T0.3.
