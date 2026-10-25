# Agent notes — `memory`

**Owns** `src/plugins/memory/**` (`mod.rs`, `inject.rs`, `import.rs`, `sync.rs`), `src/plugins/checkpoint.rs`
if split out per T2.5.

**Contract**: the `Plugin` trait, `Ctx` and the host capabilities come from the published
`rtok-plugin-sdk` crate (`crates/rtok-plugin-sdk`), not from `crate::plugin` — import them as
`rtok_plugin_sdk::…`.

**Invariants**
- No LLM calls. Notes are written by the agent through `mem_save` or extracted mechanically.
- `<private>`…`</private>` is replaced with `[REDACTED]` before a note is stored (`strip_private`). An empty title after that is `missing `title``; an empty body is not stored. `mem_get` returns the stored text.
- `mem_save` is an upsert on `(project, kind, title)` — the title is the topic key (T66.1).
  The `notes_topic` UNIQUE index enforces one row per key at the database level (T209):
  checkpoints (`checkpoint.rs`) and the SDK's `insert_note` both upsert now too (newest
  body wins) so a repeat key never errors; only `memory import` still plain-inserts, and
  only when the key is free — see below.
- Lifecycle is retire/supersede/pin, never `DELETE` (T69.1): recall and search filter
  `retired IS NULL` in SQL; `mem_get` still returns the body with a `retired` prefix; an
  explicit re-save through `mem_save` clears the tombstone. Pinned rows lead recall
  (`ORDER BY pinned DESC, id DESC`).
- MCP `mem_update` and `rtok memory retire|pin|unpin|revise` call the same functions here —
  one call path per capability (D21); `revise` is `mem_save` + `retire_note`, nothing else.
- An upsert that changes the body keeps the previous title and body in `note_versions`
  (T472, migration `0035_note_versions`). `rtok memory history <id>` prints them oldest
  first. `checkpoint:*` and `session:*` are not versioned, and a same-body upsert writes
  nothing. Recall and `mem_get` stay on the current body. There is no MCP tool. Versions
  are not part of the JSONL export.
- Recall injects titles and ids only; bodies are fetched on demand with `mem_get`.
  MCP `mem_pack` (`pack.rs`) is the one budgeted exception: it may return a
  paragraph or a body, only inside `max_tokens`, and it never runs on a hook.
- Notes are linked to files (T374, `note_files`, `files.rs`): at `mem_save` and `remember:`,
  to the existing files the body names and the session checkpoint's paths, root-relative and
  never outside the root. `prompt_recall` fuses the text hits with the notes linked to files the
  session read (`read_cache`) or the prompt names (RRF); any error there leaves the text hits.
  Text hits are FTS, or RRF of FTS and vectors already stored when `embed.enabled && embed.hybrid`
  (`search_notes_hybrid_stored`, T454); the hook does not call `embed_stale`.
  `embed.provider = "http"` (T668, `src/store/http_embed.rs`) swaps the hash embedder for an
  OpenAI-compatible `/embeddings` endpoint (`url`, `api_key_env`, `max_chars`). Any failure
  returns the plain FTS search; with a remote set the hook reads FTS only (no network on the
  hook path); remote vectors live under their own model key. Needs `embed.enabled = true`.
  A file the prompt names (T452) also recalls up to 2 of its notes the text missed, taking slots
  from the same `prompt_recall` budget; a read file only re-ranks text hits (I-115 is the wider
  variant, off until a `Measurement` shows a saving).
- Checkpoint prompts are only what the human typed (T417, `checkpoint::user_prompt`):
  host-injected records and `<system-reminder>` blocks never take one of the 20 slots.
  A new host envelope goes into `HOST_OPENERS` and `HOST_FIXTURE`, not a second filter.
- Checkpoint paths carry what happened to them (T375): `path <p> (edited|created|deleted|read[ a-b])`,
  changed files first. The action is folded from the transcript's `tool_use` blocks in
  `checkpoint::Files`; a bare `path <p>` row from before decodes as a read, and a `Write` is
  `created` only when its result says so. New rendering must stay byte-stable.
- `memory export` and `memory import` share one JSONL shape; `checkpoint:*` rows never leave.
- Recall output is byte-stable across runs with unchanged notes and ≤ 200 tokens.
- Import reads only the generic JSONL shape (no third-party DB schemas, D6) and is idempotent
  (dedupe by sha256 of body). A line whose `(project, kind, title)` already names a local
  note is skipped too, whatever its body — import must never let an older export replace a
  newer local one (T209); `insert_note_if_absent` (`INSERT OR IGNORE`, no upsert).
- Search returns the right note first for the T6.1 fixture (three notes, one obvious match).
- A `PostToolUse` stores one mechanical observation (`observe.rs`): type, title, scrubbed
  narrative ≤ 400 characters, and existing root-relative files. The raw tool output is not
  copied. A repeat of the same session, tool and narrative within 5 seconds inserts nothing,
  and neither does the same hook call (`<event>:<tool_use_id>`) delivered again at any time.
  `mem_*` tools are not observed. `mem_get` with `obs` returns that narrative. An observation
  writes no `Measurement` row (T927): the agent still gets the whole tool output, so nothing
  was saved; reading it back (`obs_get`, recall) is what meters.
- `notes.uses` / `last_used` move on `mem_get` and on a prompt-recall injection. Recall order
  is pinned, then retention score, then newest id. A low score is never a delete.
- `prompt_recall` appends at most two `obs <id> <title> (<session>)` lines inside
  `recall_tokens`, after the note index, diversified to 3 hits per session before the cut.
- `PreCompact` adds that same title index plus this session's observations when either exists.
  The hook stays `{}` when both are empty.

**Schema** lives in `crates/rtok-store/migrations/0001_schema_v1/up.sql` (`notes`, `notes_fts` + triggers). Changing
it means a new migration directory, never an edit to `0001_schema_v1/up.sql`.

**Checks**: `plan/` T2.5, T6.1–T6.3.
