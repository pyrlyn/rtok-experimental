# memory plugin

![Notes enter through mem_save, a remember prefix, an opt-in extractor or a checkpoint, pass a redaction and admission gate into SQLite notes with FTS5 and file links, and come back as a capped title index from SessionStart, UserPromptSubmit and PreCompact hooks or as mem_search, mem_pack and mem_get tool results](memory.svg)

Source: [`memory.d2`](memory.d2). Plugin docs: [README](../../../src/plugins/memory/README.md). Index: [all diagrams](../README.md).

`memory` is a local note store with no model calls. Notes arrive from the agent (`mem_save`), from
the human (`remember:` prompts), from a mechanical observer of tool calls, or from an opt-in
extractor that runs a program the user names. Recall injects only a title index, with a token
cost for each body, so the model pays for a body only when it asks (`mem_get`, `mem_pack`).
Nothing is ever deleted: notes are retired, superseded or pinned.

## Walkthrough

1. **Save.** MCP `mem_save` calls `mem_save_gated`. `strip_private` replaces each
   `<private>`...`</private>` span with `[REDACTED]`; an empty title or body is rejected.
2. **Admission gate.** With `[plugins.memory] admit` on, `admit::judge` refuses a body shorter than
   `admit_min_chars`, or one at least `admit_similarity` percent trigram-alike to a stored note of
   the project, and returns `{"refused": "too_short"|"duplicate", "id": <stored>}`. A save with an
   existing `(project, kind, title)` is a revision and always passes.
3. **Upsert and links.** `save_scoped` calls `upsert_note`: the title is the topic key, so a repeat
   updates the row (the old title and body go to `note_versions`) and clears a tombstone. The
   `notes_fts` index is kept in step by triggers. `files::link_note` records the existing,
   root-relative files the body names in `note_files`, and `upsert_note_embedding` refreshes the
   vector row for the opt-in `embed` settings.
4. **Other writers.** A first prompt line starting `remember:` is saved as kind `user` by
   `remember_save` (redacted, not gated). `rtok memory extract` runs `extract::run`, which pipes the
   redacted transcript tail to `[plugins.memory.extract] command` and saves each candidate as a plain
   note through the same gated save; it needs `extract.enabled` and no hook calls it. The inject
   plugin's `checkpoint.rs` upserts the PreCompact and SessionEnd checkpoint note.
5. **Observe.** On PostToolUse, `observe::capture` skips `mem_*` tools, builds a typed title, a
   scrubbed narrative capped at 400 characters and the files from the tool input, and inserts one
   row into `observations` (and `observation_files`), deduplicated by a session, tool and
   narrative hash. The raw tool output is not copied.
6. **Recall by hook.** SessionStart runs `recall`: `list_note_titles` for the project (pinned
   first), then `render_title_index`, which keeps the index inside `recall_tokens` by dropping the
   newest entries. UserPromptSubmit runs `prompt_recall` on the first 24 words: `search_notes` (FTS,
   or the stored hybrid leg when `embed.enabled` and `embed.hybrid`), `files::recall_hits` fuses notes
   linked to files the session read (`read_cache`) or the prompt names, and at most two `obs` lines
   follow. PreCompact adds `compact_context` (titles plus this session's observations) to the
   host's context, and SubagentStart adds `handoff::build_brief` when `spawn_brief` is on.
7. **Recall by tool.** `mem_search` is FTS5 bm25 re-ranked by use, age, pin and project. `mem_pack`
   packs the same ranking into a token budget, tier by tier (abstract, first paragraph, body),
   skipping a tier that does not fit. `mem_get` returns a body (a retired note carries a
   `retired <ts>` line) and counts a use. `mem_update` retires, supersedes or pins.
8. **Every path is measured.** `recall`, `prompt_recall`, `pre_compact`, `mem_pack`, `mem_get`,
   `observe` and `brief` write `Measurement` rows whose "before" is the bodies the index replaced.

## Where it lives

| Box | Code |
| --- | --- |
| Surfaces | `src/mcp.rs` (`mem_save`, `mem_pack`, `mem_get`, `mem_update`), `src/hooks/mod.rs` (PreCompact), `src/cli/memory.rs` |
| strip_private | `src/plugins/memory/mod.rs` (`strip_private`, `redact_private`) |
| admit::judge | `src/plugins/memory/admit.rs` (`judge`, `Verdict`) |
| upsert_note, save path | `src/plugins/memory/mod.rs` (`mem_save_gated`, `save_scoped`) |
| files::link_note | `src/plugins/memory/files.rs` (`link_note`, `recall_hits`, `existing_rels`) |
| remember: prefix | `src/plugins/memory/mod.rs` (`remember_save`) |
| extract::run | `src/plugins/memory/extract.rs` (`run`, `run_configured`, `CommandExtractor`) |
| observe::capture | `src/plugins/memory/observe.rs` (`capture`, `synthesize`), `scrub.rs` |
| checkpoint | `src/plugins/checkpoint.rs` (`save`, `write`), used by `src/plugins/inject/` |
| recall, prompt_recall | `src/plugins/memory/mod.rs` (`recall`, `prompt_recall`, `render_title_index`, `compact_context`) |
| build_brief | `src/plugins/memory/handoff.rs` (`build_brief`, `handoff`) |
| mem_search, mem_pack, mem_get, mem_update | `src/plugins/memory/mod.rs`, `src/plugins/memory/pack.rs` (`pack_notes`) |
| import, export, sync | `src/plugins/memory/import.rs`, `export.rs`, `sync.rs` (CLI only, not drawn) |
| Tables | `crates/rtok-store/` (`notes`, `notes_fts`, `note_versions`, `note_embeddings`, `note_files`, `observations`, `observations_fts`, `observation_files`, `read_cache`, `measurements`) |
| Config | `crates/rtok-config/` (`[plugins.memory]`, `[plugins.memory.embed]`, `[plugins.memory.extract]`) |

## Invariants

- No model calls: notes come from the agent, the human, a mechanical observer or a user-named command.
- Lossless lifecycle: retire, supersede and pin, never delete; a retired body stays readable
  through `mem_get`.
- `<private>` spans are redacted before anything is stored, and observation narratives are scrubbed
  before they are cut.
- One row per `(project, kind, title)`; a same-body upsert writes no version.
- Recall injects titles and ids only, within `recall_tokens`, byte-stable for an unchanged store;
  only `mem_pack` may return a paragraph or body, and it never runs on a hook.
- Fail open: a store or search error leaves the hook output unchanged, and no network call is made on
  the hook path.
- No `Measurement` row, no saving claim; the checkpoint prompt counts go to plugin state, not the ledger.

## Related

[Overview](../overview.md) · [Plugin contract](../plugin-contract.md) · [Hook flow](../hook-flow.md) ·
[MCP flow](../mcp-flow.md) · [Store](../store.md) · [inject](inject.md) · [read](read.md)
