# rtok

https://github.com/pyrlyn/rtok

Token-reduction CLI for AI coding agents: hooks, MCP server, API proxy; measured reductions, pluggable methods.

## Cloud review findings (2026-10-08)

New bugs, dead code and moves from a read-only Cursor cloud review of `main` at `a060af58` (agent `bc-e07c8099-22ee-5d00-b1c7-3bd1ad623d52`; full report: `cloud/rtok.md` in the private `listepo/roadmap` repo). They take ids T457–T470, ordered P0, P1, P2, P3. **confirmed** means seen in the tree; **suspected** means plausible from the code but not proven (nothing was run on Windows). Line numbers are as of the review. The report's own labels T436–T449 are roadmap ids, not this plan's. T457, T458 and T459 are done (`done.md`). None of the rest is in the task table yet: to take one, add its row and a card with a `Check:` line.

| ID | Priority | Kind | Status | Where | Fix |
| --- | --- | --- | --- | --- | --- |
| T464 | P2 | move | compared | `crates/rtok-agent-sdk/src/lib.rs:84-163` (`backup`) → crates-packages `file-backup` | Not a drop-in (checked 2026-10-10): rtok writes to a `_backup/` folder, keeps N generations (`prune_backups`, `stale_backups`) and dedups against any file there; `file-backup` 0.1.1 writes beside the file and never prunes. The creator chose to extend `file-backup` first (crates-packages roadmap T12, with its T4 symlink fix, which the open crates-packages PR #34 also touches), then depend on it here. |
| T465 | P2 | move | compared | `tools/dist-generate.sh`, `tools/release.sh` → `pyrlyn/ci` | Not shared copies (diffed 2026-10-10): `dist-generate.sh` differs from ketch's and swarfr's in 239 and 102 lines; `release.sh` differs from ketch, cox, runa and swarfr in 130, 121, 63 and 75 lines. Nothing to fold until the repos agree on one release flow; no move without a new creator OK. |
| T466 | P2 | move | compared | `.github/actions/rustup-toolchain-cache` → `pyrlyn/ci/.github/actions/` | Only rtok uses it (`ci.yml`, `graph-review.yml`; checked 2026-10-10 across `apps/` and `packages/`). `pyrlyn/ci` already ships `setup-rust` (mise-pinned Rust and `rust-cache`); sharing would mean folding the toolchain cache into it. No move without a new creator OK. |
| T470 | P3 | move | suspected | `tools/test-changed.sh`, `tools/selective-check.sh` → `scoped-check` / `pyrlyn/ci` | Overlap was claimed but not diffed against cox or `pyrlyn/ci`. Compare first; move only what matches. |

Already tracked here, not added again: `src/render.rs` → `change-preview` is T416.1; per-host MCP code → `crates/rtok-mcp` and the unused `rtok_mcp::ops::apply` are T277; `OPERATION_ICONS` → a shared icon crate is T436.3; the test-only `VersionFile::write`/`::new` and the stale `#[allow(dead_code)]` on `read_installed` (`src/agents/plugin_version.rs:84`, `:112`, `:268`) are open on T279.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T124 | todo | P3 | 2 | 0% | |
| T131 | todo | P2 | 3 | 70% | |
| T132 | todo | P2 | 2 | 70% | |
| T134 | todo | P1 | 2 | 40% | |
| T156 | in progress | P3 | 3 | 50% | Claude Code / claude-opus-5-5 |
| T262.3 | todo | P2 | 2 | 0% | |
| T261 | in progress | P2 | 3 | 95% | Cursor / grok 4.7 |
| T275 | in progress | P1 | 4 | 90% | Claude Code / sonnet-5.5 |
| T275.1 | in progress | P2 | 3 | 80% | Cursor / grok 4.7 |
| T276 | in progress | P2 | 5 | 30% | Claude Code / sonnet-5.5 |
| T276.3 | todo | P2 | 4 | 0% | |
| T276.4 | todo | P2 | 4 | 0% | |
| T276.5 | todo | P2 | 3 | 0% | |
| T276.6 | todo | P2 | 3 | 0% | |
| T277 | in progress | P2 | 5 | 30% | Claude Code / sonnet-5.5 |
| T278 | in progress | P1 | 3 | 90% | Claude Code / claude-opus-5-5 |
| T279 | in progress | P1 | 5 | 90% | Claude Code / claude-opus-5-5 |
| T281 | in progress | P1 | 3 | 70% | Claude Code / claude-opus-5-5 |
| T289 | in progress | P2 | 4 | 75% | Claude Code / sonnet-5 |
| T329 | todo | P2 | 5 | 0% | |
| T329.50 | todo | P3 | 3 | 0% | |
| T356 | in progress | P1 | 2 | 5% | Claude Code / claude-opus-5-5 |
| T369.1 | todo | P3 | 1 | 0% | |
| T370 | in progress | P1 | 4 | 90% | Claude Code / sonnet-5.5 |
| T378 | todo | P3 | 3 | 0% | |
| T385 | in progress | P1 | 5 | 20% | Claude Code / opus-5-5 |
| T385.3 | todo | P1 | 3 | 20% | |
| T385.9 | todo | P3 | 5 | 10% | |
| T394 | todo | P2 | 2 | 20% | |
| T395 | todo | P3 | 2 | 20% | |
| T396 | todo | P3 | 2 | 20% | |
| T398 | todo | P3 | 1 | 30% | |
| T413 | todo | P2 | 3 | 0% | |
| T413.3 | in progress | P2 | 3 | 0% | Cursor / grok 4.7 |
| T413.4 | in progress | P2 | 3 | 0% | Cursor / grok 4.7 |
| T413.5 | in progress | P3 | 3 | 0% | Cursor / grok 4.7 |
| T413.6 | in progress | P3 | 2 | 0% | Cursor / grok 4.7 |
| T413.7 | in progress | P3 | 3 | 0% | Cursor / grok 4.7 |
| T413.8 | in progress | P3 | 3 | 0% | Cursor / grok 4.7 |
| T413.9 | in progress | P3 | 3 | 0% | Cursor / grok 4.7 |
| T413.10 | in progress | P3 | 2 | 0% | Cursor / grok 4.7 |
| T413.11 | in progress | P3 | 2 | 0% | Cursor / grok 4.7 |
| T413.12 | in progress | P3 | 2 | 0% | Cursor / grok 4.7 |
| T413.13 | in progress | P3 | 2 | 0% | Cursor / grok 4.7 |
| T413.14 | in progress | P3 | 2 | 0% | Cursor / grok 4.7 |
| T413.15 | in progress | P3 | 2 | 0% | Cursor / grok 4.7 |
| T414 | in progress | P1 | 4 | 20% | Claude Code / opus-5.5 |
| T414.7 | todo | P3 | 1 | 0% | |
| T416 | in progress | P1 | 3 | 70% | Claude Code / claude-opus-5-5 |
| T428 | in progress | P2 | 3 | 85% | Claude Code / sonnet-5.5 |
| T436.3 | todo | P2 | 2 | 0% | |
| T436.4 | todo | P3 | 2 | 0% | |
| T477 | todo | P3 | 3 | 0% | |
| T521 | todo | P2 | 2 | 0% | |
| T531 | todo | P2 | 2 | 0% | |
| T532 | todo | P1 | 3 | 0% | |
| T533 | todo | P1 | 3 | 0% | |
| T534 | todo | P2 | 2 | 0% | |
| T535 | todo | P2 | 2 | 0% | |
| T536 | todo | P2 | 2 | 0% | |
| T537 | todo | P2 | 2 | 0% | |
| T538 | todo | P2 | 2 | 0% | |
| T539 | todo | P2 | 2 | 0% | |
| T541 | todo | P1 | 2 | 0% | |
| T542 | todo | P2 | 3 | 0% | |
| T543 | todo | P2 | 2 | 0% | |
| T544 | todo | P2 | 2 | 0% | |
| T545 | todo | P2 | 3 | 0% | |
| T546 | todo | P2 | 2 | 0% | |
| T548 | todo | P1 | 2 | 0% | |
| T549 | todo | P2 | 3 | 0% | |
| T550 | todo | P2 | 2 | 0% | |
| T551 | todo | P2 | 2 | 0% | |
| T552 | todo | P2 | 2 | 0% | |
| T553 | todo | P2 | 2 | 0% | |
| T555 | todo | P2 | 2 | 0% | |
| T556 | todo | P2 | 3 | 0% | |
| T557 | todo | P2 | 3 | 0% | |
| T558 | todo | P2 | 2 | 0% | |
| T559 | todo | P2 | 2 | 0% | |
| T560 | todo | P3 | 2 | 0% | |
| T561 | todo | P3 | 3 | 0% | |
| T562 | todo | P1 | 2 | 0% | |
| T563 | todo | P2 | 2 | 0% | |
| T564 | todo | P2 | 2 | 0% | |
| T565 | todo | P2 | 3 | 0% | |
| T566 | todo | P2 | 2 | 0% | |
| T567 | todo | P2 | 2 | 0% | |
| T569 | todo | P2 | 3 | 0% | |
| T570 | todo | P2 | 2 | 0% | |
| T571 | todo | P2 | 2 | 0% | |
| T573 | todo | P2 | 3 | 0% | |
| T574 | todo | P2 | 2 | 0% | |
| T576 | todo | P2 | 3 | 0% | |
| T577 | todo | P2 | 3 | 0% | |
| T579 | todo | P2 | 3 | 0% | |
| T580 | todo | P2 | 2 | 0% | |
| T581 | todo | P2 | 2 | 0% | |
| T583 | todo | P2 | 3 | 0% | |
| T584 | todo | P2 | 2 | 0% | |
| T586 | todo | P2 | 3 | 0% | |
| T587 | todo | P2 | 2 | 0% | |
| T589 | todo | P2 | 3 | 0% | |
| T590 | todo | P2 | 3 | 0% | |
| T592 | todo | P2 | 3 | 0% | |
| T594 | todo | P2 | 3 | 0% | |
| T595 | todo | P2 | 3 | 0% | |
| T597 | todo | P1 | 3 | 0% | |
| T599 | todo | P3 | 2 | 0% | |
| T601 | todo | P2 | 3 | 0% | |
| T604 | todo | P2 | 3 | 0% | |
| T606 | todo | P2 | 3 | 0% | |
| T608 | todo | P3 | 3 | 0% | |
| T609 | todo | P3 | 3 | 0% | |
| T611 | todo | P2 | 3 | 0% | |
| T613 | todo | P2 | 3 | 0% | |
| T615 | todo | P2 | 3 | 0% | |
| T617 | todo | P2 | 3 | 0% | |
| T618 | todo | P1 | 3 | 0% | |
| T619 | todo | P2 | 3 | 0% | |
| T620 | todo | P1 | 5 | 0% | |
| T621 | todo | P2 | 5 | 0% | |
| T622 | todo | P2 | 3 | 0% | |
| T623 | todo | P2 | 5 | 0% | |
| T624 | todo | P3 | 2 | 0% | |
| T625 | todo | P1 | 3 | 0% | |
| T626 | todo | P2 | 3 | 0% | |
| T627 | todo | P1 | 3 | 0% | |
| T628 | todo | P2 | 3 | 0% | |
| T629 | todo | P2 | 3 | 0% | |
| T630 | todo | P2 | 3 | 0% | |
| T631 | todo | P3 | 3 | 0% | |
| T632 | todo | P2 | 3 | 0% | |
| T633 | todo | P3 | 2 | 0% | |
| T634 | todo | P2 | 2 | 0% | |
| T635 | todo | P2 | 3 | 0% | |
| T636 | todo | P2 | 3 | 0% | |
| T637 | todo | P2 | 3 | 0% | |
| T638 | todo | P2 | 3 | 0% | |
| T639 | todo | P2 | 3 | 0% | |



**Source audit (T530).**

T531–T539 are confirmed source defects on `0b14b88e`, filed for implementation rather
than fixed during this audit. Start with the incorrect-response cases T532/T533, then
the remaining independent P2 fixes. Each card distinguishes executed function probes
from source-derived reproduction steps and gives its own implementation/check sequence.
Existing tasks, including uncommitted T522–T527 in the shared checkout, were checked for
overlap and were not copied or reassigned. This is a bounded review of the core data
paths, not a claim that every bug or host/platform combination has been exhausted.

### T531. Make the archive reference returned by read dedup expandable

Source-confirmed on `0b14b88e`: [`dedup_line`](src/plugins/read/cache.rs#L65) formats a 64-character archive id, then always shortens it to eight characters to meet the 80-character output limit. Even a one-line file produces a full form longer than 80 characters. [`Store::get_archive`](crates/rtok-store/src/archive_rows.rs#L402) performs an exact primary-key lookup; neither CLI nor MCP expand resolves prefixes. Read a file twice in one session, copy the id from `unchanged since <8 chars>`, then expand it: the lookup has no matching row. The bytes exist, but the returned reference cannot retrieve them. T4.4 introduced the short display; T354 documents full ids but does not make this reference usable.

Execution plan:

1. Add a regression using the id parsed from the second read response, not the full id fetched from the database.
2. Prefer a compact dedup message containing the full id within the existing budget; reuse the archive reference convention instead of introducing ambiguous prefix lookup.
3. Exercise CLI and MCP expansion of that exact reference, including large line counts; retain one dedup Measurement.

Check: the second read's reference expands to the archived bytes, the response meets its length contract, and `just check` passes.

### T532. Preserve typed content and message fields in the direct cache key

Source-confirmed on `0b14b88e`: [`messages_text` / `block_text`](src/proxy/semantic_cache.rs#L368) flatten structured requests into strings before hashing. `tool_result.is_error`, message `name`, and `image_url.detail` are omitted. For example, otherwise identical single-user requests with the same image URL and `detail: "low"` versus `"high"` produce identical CachePrompts. A text block containing `tool_result t: OK` also has the same flattened text as `{type:"tool_result", tool_use_id:"t", content:"OK"}`. With semantic cache enabled, the direct tier returns the first response for the second request before an upstream call. This is distinct from completed T55.14/T212/T303/T323: those added fields but retained lossy flattening.

Validation: an offline Rust harness compiled verbatim `messages_text`, `content_text`, `block_text` and `canonical_json` from this revision; all four differing pairs above returned equal message fields. Other CachePrompt inputs were held equal. This proves the normalization collision, not an end-to-end proxy run.

Execution plan:

1. Add paired-request fixtures for image detail, tool-result error status, participant name and literal text versus typed tool content. Assert distinct direct keys and two upstream calls.
2. Build the direct key from canonical structured message/content values, keeping type tags, field boundaries and response-affecting fields. Keep semantic text extraction separate.
3. Retain key-order equivalence and the existing caller/model/tool-schema isolation tests; do not broaden cache eligibility.

Check: all differing fixture pairs miss the direct cache, exact replay still hits, reordered object keys retain the same key, and `just check` passes.

### T533. Scope cached responses to the resolved upstream request target

Source-confirmed on `0b14b88e`: [`handle`](src/proxy/mod.rs#L407) looks up a shared cache before resolving the target at line 430. [`CachePrompt`](src/proxy/semantic_cache.rs#L22) includes provider/model/body/caller but no upstream URL, path, query or HTTP method. [`upstream_for`](src/proxy/mod.rs#L224) supports different upstreams per lane. Two cache-enabled lanes with identical shaping policy, body and credentials but different upstreams therefore share a direct key: warming endpoint A can cause a request intended for B to return A's answer without contacting B. This remains possible with `cache_by_provider` and `cache_by_model` both enabled; T323 isolated callers, not routing destinations.

Execution plan:

1. Reproduce with two local upstream servers returning distinct sentinel responses and two lanes that permit cache reads/fills.
2. Resolve the effective target before lookup and include a consistent target/method namespace in both lookup and fill. Avoid retaining raw credentials or credential-bearing query strings in cache metadata; reuse hashing.
3. Test differing paths/queries as well as lane upstreams; exact replay to one endpoint must still hit.

Check: B receives its first request after A was warmed, each endpoint returns its own bytes, same-target replay hits, and `just check` passes.

### T534. Validate expand ranges against the payload before freezing decisions

Source-confirmed on `0b14b88e`: [`expand::run`](src/expand.rs#L200) and [`mcp::expand_text`](src/mcp.rs#L837) validate ranges against `usize::MAX`, call `fetch`, and only then validate against the actual line count. [`fetch`](src/expand.rs#L26) commits the expanded flag and its Measurement. Expanding a one-line live-zone archive with `lines: "2-3"` therefore returns an error after permanently freezing the compression decision and charging an expansion that returned no bytes. The earlier past-EOF fix correctly rejects the range; this task addresses its remaining side effects, not range syntax (T522).

Execution plan:

1. Seed an unexpanded decision and a one-line archive, request `2-3` through both CLI and MCP, and assert an error with unchanged decision and measurement rows.
2. Separate retrieval/validation from successful-expansion bookkeeping; share the sequence across both surfaces and preserve the atomic freeze-plus-measurement transaction.
3. Cover empty archives and valid filtered expansion, and retain malformed-range checks.

Check: all failed ranges leave state unchanged; successful expansion still freezes once and records the intended cost; `just check` passes.

### T535. Record the first full or ranged read when its output is capped

Source-confirmed on `0b14b88e`: [`read_with`](src/plugins/read/mod.rs#L171) calls `cap`, which passes `None` to [`cap_impl`](src/plugins/read/mod.rs#L270) and writes no Measurement. The subsequent render measurement only handles map/signatures/stripped modes. A fresh `full` or `lines` read exceeding `read.max_chars` is archived and shortened but produces no read saving row. Generic MCP token rows compare the arguments with the response and do not repair this missing Measurement. T300 covers outlines and stripped output; search/tree already use `cap_recording`.

Execution plan:

1. In a fresh session, read a fixture above the cap in full and ranged modes; assert one Measurement with the full rendered view as before and the returned text, including its marker, as after.
2. Reuse the existing cap recorder at the responsible path; retain a single measurement for map/signatures/stripped, dedup and delta paths.
3. Verify an uncapped full read does not acquire a fictitious saving, and the capped archive remains expandable.

Check: capped fresh full/lines reads have exactly one correct Measurement, existing rendering/dedup accounting does not double count, and `just check` passes.

### T536. Preserve raw command bytes when storage fails open

Source-confirmed on `0b14b88e`: [`emit_filtered_to`](src/plugins/cmd/run.rs#L323) creates a lossy UTF-8 view before opening the runtime. Both the runtime-open failure at line 332 and the archive-write failure at line 393 print that lossy view. With a body containing byte `0xff` and an unavailable database/archive, the fallback outputs UTF-8 replacement bytes instead; no usable archive can restore the input. The normal successful archive path already preserves binary bytes, so its tests do not cover this fallback.

Execution plan:

1. Inject runtime-open failure and archive-write failure using a temporary store/path and non-UTF-8 input. Compare output bytes, not decoded strings.
2. Share a raw-byte fallback writer for both failures, preserving the CLI's established trailing-newline convention and wrapped exit code.
3. Keep the successful formatter/archive path and tiny-body behavior unchanged; verify no dead archive pointer is emitted on failure.

Check: the original bytes survive both failure branches, the command status is preserved, and `just check` passes.

### T537. Keep MCP pending calls when the server sends its own request

Source-confirmed on `0b14b88e`: [`take_call`](src/mcp/wrap.rs#L207) removes a client pending call for any server frame with the same `id`, without checking whether the frame is a response. After client `tools/call` id 7, a server `sampling/createMessage` request id 7 consumes the tool name; the eventual tool result id 7 is then forwarded without compression or its archive/measurement. Request ids in opposite directions must not be treated as one response namespace. The existing `pending_matches_tool_call_ids_only` test covers numeric versus string ids, not reverse requests.

Validation: an offline Rust harness compiled verbatim `note_call` and `take_call` from this revision. The server request returned the pending tool name; the later response returned `None`. Full transport validation remains part of the fix.

Execution plan:

1. Extend the framing fixture with an in-flight client tool call, a server request using the same numeric id, and the eventual tool response.
2. Consume pending state only for response-shaped frames; preserve server requests/notifications byte-for-byte.
3. Cover server requests with string ids, error responses and unrelated notifications; a genuine success/error response must drain its pending entry once.

Check: the server request does not remove the client call, the later large result is shortened and expandable with one measurement, and `just check` passes.

### T538. Do not let skipped memory imports reserve unseen body hashes

Source-confirmed on `0b14b88e`: [`memory::import::run`](src/plugins/memory/import.rs#L94) inserts a body's hash into `seen` before checking whether the topic key is already occupied. Seed local topic A with body `old`; import A with body `new`, then free topic B with body `new`. A is correctly skipped to protect the local note, but B is also skipped because A already reserved `new` in memory, although no stored note has that body. The reported result is zero insertions; reversing the input order permits B. The same false skip affects dry-run.

Execution plan:

1. Add this two-line fixture in both orders, with an existing local A, for dry-run and actual import.
2. Check body/topic eligibility before reserving either; reserve both only for an accepted simulated or successful insertion. Handle the concurrent insert returning `None` without poisoning later rows.
3. Preserve idempotency, the local-topic-wins rule and intentional dedup against bodies that really exist.

Check: B is imported regardless of the skipped A's position, A keeps `old`, dry-run matches actual import, a second import is idempotent, and `just check` passes.

### T539. Preserve a global note's project scope when revising it

Source-confirmed on `0b14b88e`: [`mem_revise`](src/plugins/memory/mod.rs#L598) passes `old.project.as_deref()` to `mem_save`. For a global note (`project = NULL`), that becomes `None`, which [`mem_save`](src/plugins/memory/mod.rs#L430) interprets as infer the current working directory's project. Global notes are supported by [`memory import`](src/plugins/memory/import.rs#L30), whose optional project is stored without cwd inference. Import a global note, then revise it while inside a repository: the replacement becomes repository-scoped and the global note is retired, even when the title is unchanged. It may also upsert an existing same-title project note.

Execution plan:

1. Seed a global note and a separate same-title project note. Revise the global id from a repository cwd with both unchanged and changed titles.
2. Separate an already-resolved optional project from the user-facing infer-project default; reuse one save implementation while preserving the old note's exact scope.
3. Cover global, explicit-project and inferred-project creation plus revision; keep privacy stripping, embeddings and note history on the same write path.

Check: global revisions stay global, unrelated project notes are untouched, ordinary creation still infers cwd, and `just check` passes.

**Docs/checkpoint audit (T547).** T548–T553 are confirmed against `99b6753d`
(product source unchanged from `0b14b88e`). Address the private-text persistence
case first; the remaining fixes are independent except that T549/T550 share the
local documentation ingestion path. Related completed feature tasks are cited
below; these are newly identified failure cases, not copies of those features.

### T548. Redact private text before storing and restoring checkpoints

Confirmed in source and a function probe: [user_prompt](src/plugins/checkpoint.rs#L311) keeps private-tagged text, [render](src/plugins/checkpoint.rs#L122) reproduces it, and [write](src/plugins/checkpoint.rs#L687) passes the rendered body directly to upsert_note. Neither [Runtime::upsert_note](src/plugin.rs#L468) nor [Store::upsert_note](crates/rtok-store/src/note_store.rs#L155) strips it. A compact user record containing `use <private>PRIVATE_SENTINEL</private> briefly` therefore persists the sentinel in checkpoint/session notes and exposes it again on restoration. This violates the private-tag invariant in `src/plugins/memory/AGENTS.md`, despite ordinary mem_save/import already redacting.

Validation: an offline Rust probe compiled the verbatim checkpoint extraction/rendering code and asserted that the private sentinel survives. Persistence/restoration is source-confirmed through write → upsert → render_offer; no live hook or database integration was run.

Execution plan:

1. Add PreCompact and SessionEnd fixtures with private text in prompts and captured errors; assert the stored note and restored injection never contain the sentinel.
2. Reuse the existing private-tag scrubber at the checkpoint text boundary before truncating captured strings, so a closing tag beyond the prompt/error length limit cannot evade redaction.
3. Cover mixed-case, multiline and long private blocks; retain ordinary prompt/path/action output, budgets and byte stability for nonprivate fixtures.

Check: private content is absent from both persisted and restored checkpoint/session bodies, ordinary memory redaction remains intact, and `just check` passes.

### T549. Scope local llms documentation to its workspace

Source-confirmed: [ingest_llms](src/plugins/docs/mod.rs#L191) writes every workspace to the same `("llms", "local")` cache key. [docs_resolve/query](src/plugins/docs/mod.rs#L91) reads that key even when ingestion finds no local file. [replace_doc_crate](crates/rtok-store/src/docs.rs#L118) replaces all items with that name/version, with no workspace dimension. Query workspace A's llms.txt, then query `llms` in B with the same rtok home and no llms file: B receives A's documentation. If B has its own file, it overwrites A's cache. T455 introduced workspace docs but did not isolate their storage.

Execution plan:

1. Use two temporary workspace roots sharing one store: A has a unique local marker, B initially has no file and then a different marker. Assert neither resolve nor query in B exposes A's content.
2. Give local documents a canonical workspace namespace, consistently applied during ingestion, cache checks and queries; keep downloaded crate/version documents globally reusable.
3. Reconcile missing or empty local sources within their namespace and test workspace switching, deletion and recreation. Keep old unscoped local rows from being treated as the current workspace's docs.

Check: each workspace sees only its own local documents, absence reports an uncached/empty result according to the tool contract, shared crate caches still work, and `just check` passes. Reproduction is source-derived.

### T550. Preserve returned doc ids when unchanged local docs are re-ingested

Source-confirmed: every [ingest_llms](src/plugins/docs/mod.rs#L200) call calculates a SHA but unconditionally calls replace_doc_crate. That method deletes and reinserts the items; [doc_items](crates/rtok-store/migrations/0039_docs_fts/up.sql#L12) uses generated integer primary keys. Seed one local section (id 1), fetch a crate whose item gets id 2, then run docs_resolve or docs_query with the unchanged local file present: the local section is recreated at id 3. [docs_get](src/plugins/docs/mod.rs#L177) on the previously returned id 1 now fails. An unrelated read invalidates the handle even though its source bytes did not change. This is distinct from T549's cross-workspace namespace.

Execution plan:

1. Add the interleaved local-ingest/crate-fetch/local-resolve fixture, retaining the id from the first local query and calling docs_get afterward.
2. Compare the current source SHA and scope with cached metadata and make unchanged ingestion a no-op, with the check/write sequence safe against concurrent ingestion.
3. Verify repeated resolves and unrelated crate queries preserve local item ids and FTS rows; changed bytes must still refresh the affected document. Coordinate the cache key with T549.

Check: docs_get keeps resolving ids from unchanged local documents across repeated reads, actual edits are searchable, and `just check` passes. Source-derived reproduction; generated-id behavior is visible in the schema and replacement path.

### T551. Estimate docs-query savings against the same full content as byte accounting

Source-confirmed: [docs_query](src/plugins/docs/mod.rs#L138) sets before_bytes from the full documentation bodies of all considered hits, but est_before at line 154 estimates only their already-truncated snippets. est_after estimates the returned JSON. For one long document with a short snippet, most of the reduction is absent from token accounting and JSON metadata can make the reported token saving negative while bytes show a large reduction. T455 requires a query Measurement; this card fixes the inconsistent before basis rather than adding another row.

Execution plan:

1. Seed a long cached document with a small snippet limit and query it under a fixed estimator. Assert before_bytes and est_before describe the same full set of considered bodies, and after values describe the actual response.
2. Obtain full-body token estimates through the existing store/runtime boundary before snippet and output-budget trimming; avoid duplicating the query Measurement.
3. Cover multiple hits, budget-trimmed hits and no-result queries; retain the documented definition of considered content and the current query output.

Check: token and byte baselines refer to the same pre-trim content, exactly one row records a returned query result, and `just check` passes. Source-derived accounting defect; no measured saving is claimed by this audit.

### T552. Keep checkpoint prefilter equivalent for spaced and mixed-content JSONL

Confirmed by a function probe: [worth_parsing](src/plugins/checkpoint.rs#L261) requires the literal substring `"type":"user"` and rejects any otherwise ordinary user record containing `"tool_use_id"`. A valid one-line `{"type": "user", "message": {"content": "ship the patch"}}` loses its prompt, as does a compact user record containing both a tool_result and a typed text block. [user_text](src/plugins/checkpoint.rs#L386) correctly extracts text from both if the prefilter is bypassed. Completed T417 filters host-written prompts; these are uncovered false negatives in the prefilter's promised superset, not a request to undo that host filter.

Validation: an offline Rust probe compiled the verbatim extraction code and compared filter=true/false for both records. Only the filtered run lost the prompt; the equivalent compact text-only record survived.

Execution plan:

1. Extend the prefilter equivalence fixtures with optional JSON whitespace, mixed text/tool-result blocks, reordered keys and escaped field names.
2. Make the cheap filter conservative: ambiguous records must reach the structural parser; do not use tool_use_id presence as proof that no typed text exists.
3. Compare extracted prompts, counts, paths, skills and errors with filtering disabled; preserve T417's host-envelope exclusions and the existing large-transcript performance check.

Check: semantically equivalent valid JSONL yields identical checkpoint content regardless of formatting, mixed typed text survives, host-only content remains excluded, and `just check` passes.

### T553. Use the shared project resolver for session checkpoint handoff

Source-confirmed: [project_of_cx](src/plugins/checkpoint.rs#L737) returns git_root's directory basename, while [project::project_name](src/project.rs#L19) resolves origin names and linked worktrees to the main project's identity. Both save_session and offer_session use the former, and [latest_session_note](crates/rtok-store/src/note_store.rs#L320) requires an exact project match. End a session in main checkout `repo`, then start one in its linked worktree `repo-fix`: the saved `repo` handoff is not found under `repo-fix`, although normal memory/session attribution treats them as one project. T133/T154 implemented the shared resolver, but this handoff path still bypasses it.

Execution plan:

1. Add a main/linked-worktree fixture and an origin name different from the checkout basename. Save a session checkpoint in one checkout and restore it from the other.
2. Reuse project::project_name for both saving and looking up session handoffs. Define a narrow compatibility path for existing basename-scoped handoffs without searching unrelated projects.
3. Exercise both handoff directions, subdirectories and non-git cwd; preserve the opt-in handoff flag, session-local compact restore and injection budget.

Check: linked worktrees share the intended project handoff consistently with ordinary memory, unrelated project notes remain excluded, and `just check` passes. Source-derived reproduction; no real agent session was started.

**Guard/encoding audit (T554).** T555–T560 were confirmed against `eb13dd7f`
(product source unchanged from `0b14b88e`). T555–T559 are guard behavior defects;
T560 is explicitly a test-oracle defect, not a production decoder failure.
Each was compared with completed T55.8/T55.9/T57.1/T322/T445/T62.1/T468 and
the active backlog. The related features do not track these exact triggers.

### T555. Preserve quoted whitespace in guard command identities

Confirmed by an actual-source function probe: [cache_key](src/plugins/guard/mod.rs#L255) passes command text through [norm_cmd/collapse](src/plugins/guard/mod.rs#L409), whose split_whitespace/join normalizes inside quotes too. `cat 'a  b'` and `cat 'a b'` therefore produce the same nonempty Bash key in one cwd, although they read distinct files. After the first result is archived, [pre_tool](src/plugins/guard/mod.rs#L57) can deny the first execution of the second command and offer the wrong file's result. T55.9/T445 preserved cd/cwd distinctions but not quoted argument bytes.

Execution plan:

1. Add two files with different contents and one versus two spaces in their names; cache the first command's result and assert the second command is allowed.
2. Preserve quoted and escaped argument bytes when normalizing keys; only collapse syntactically insignificant whitespace. Fail open when the existing shell helpers cannot establish a safe identity.
3. Cover single/double quotes, literal tabs/newlines inside arguments, escaped spaces and supported rtok run wrappers; retain dedup for equivalent outside-quote whitespace.

Check: different paths never collide, ordinary equivalent repeats still deny with the correct archive, and `just check` passes. The probe confirmed key equality; full hook regression is part of the fix.

### T556. Resolve guard Read and invalidation paths against the same cwd

Confirmed by an actual-source key probe and source tracing: [cache_key's Read arm](src/plugins/guard/mod.rs#L230) stores only the trimmed input path and slice, ignoring cwd. A Read of `file.txt` in /A and one in /B produce the same key. An intervening absolute `cd /B && ls` only clears Bash keys in [post_tool](src/plugins/guard/mod.rs#L108), so it does not prevent the false Read denial. Separately, Edit/Write clears the literal `read\t{path}` prefix at line 149: an absolute edit does not invalidate a prior relative/./ spelling of the same file. T55.8 added invalidation and T322 added slices; neither establishes shared path identity.

Execution plan:

1. Add same-session fixtures for equal relative names under two cwd values and for Read("./file.txt") followed by Edit of its absolute path.
2. Reuse one path-identity function for Read keys and Edit/Write invalidation, resolving relative paths against the hook cwd without reading file bodies. Preserve meaningful filename whitespace; fail open when relative identity cannot be established.
3. Keep slice and agent/window namespaces distinct, ensure equivalent path spellings invalidate together, and retain the hook's metadata-only budget.

Check: changing cwd allows the new file's first read, editing through an alias invalidates all affected slices, unrelated files retain their cache, and `just check` passes.

### T557. Reject side-effect variants of nominally read-only shell commands

Confirmed by actual-source classification probes: [stem_read_only/writer_marker](src/plugins/guard/mod.rs#L324) accepts `sed 'w output' input`, `find . -fprint output`, and `git diff --output=output` as cacheable. It only checks selected writer markers such as sed -i, find -delete/-exec and redirection. These accepted variants write files, so their PostToolUse stores a dedup key instead of invalidating earlier reads; repeating them can be denied and previous file results can remain stale. T57.1/T322 cover other writer forms, not these options/scripts.

Validation: all three variants returned nonempty keys in an offline actual-source Rust probe. A separate system sed run in a temporary directory created a nonempty output file from the first example. The find/git side effects were not executed during the audit.

Execution plan:

1. Add classification regressions for sed's write/execute script commands, find output actions and git output-file options, plus a hook fixture proving a writer invalidates an earlier Read result.
2. Tighten eligibility at the shared classification layer: use a conservative supported read-only subset and reject write/execute forms or ambiguous script/option parsing. Reuse existing shell parsing helpers.
3. Retain common safe sed/grep/find/git listing cases; assert writer variants are allowed to execute again and clear stale read/Bash keys.

Check: all demonstrated writers return no dedup key, their post-tool events invalidate prior results, safe readers retain expected dedup, and `just check` passes.

### T558. Fail open for a sole skill installation scoped to another project

Confirmed by an actual-source function probe: [plugin_root](src/plugins/guard/skill.rs#L116) first looks for the current project or a user installation, then accepts any single remaining installation. A manifest containing only `scope: "project", projectPath: "/A", installPath: "/private-A-plugin"` returns that path even for cwd /B. With skill digest enabled and a sufficiently large file there, the guard archives A's skill body and denies B's invocation using the wrong installation. The single-entry fallback intended for legacy unscoped records does not distinguish an explicitly foreign project. T62.1's existing legacy and multi-install tests do not cover this case.

Execution plan:

1. Add a sole foreign-project installation fixture and assert resolution fails open; include a large skill file to prove no archive/denial is generated for it.
2. Restrict the legacy fallback to records without contradictory scope/project metadata; never select an explicitly nonmatching project just because it is the only entry.
3. Retain matching-project precedence, user fallback, genuine legacy single entries and ambiguous-marketplace fail-open behavior.

Check: an unrelated project cannot supply the denied skill's body, supported matching/legacy cases still resolve, and `just check` passes. The probe tested path selection; the full digest check belongs to the fix.

### T559. Recognize YAML-quoted host keys before digesting skills

Confirmed by an actual-source probe: [host_keys](src/plugins/guard/skill.rs#L157) checks raw line prefixes, so valid frontmatter such as `"model": value` or `'allowed-tools': value` bypasses the exemption. [decide](src/plugins/guard/skill.rs#L125) then archives and denies an oversized skill whose host-applied model/tool/context behavior must be preserved. The T62.1 contract is semantic frontmatter key presence, not a particular YAML spelling.

Validation: the probe parsed quoted model/allowed-tools keys with the repository's serde-saphyr version and confirmed host_keys returns false for the same valid YAML.

Execution plan:

1. Add oversized-skill fixtures with single/double-quoted top-level host keys and assert no digest denial or saving row.
2. Inspect the frontmatter mapping with the existing YAML parser, preserving host behavior for allowed-tools/model/context/agent; malformed or ambiguous frontmatter must fail open.
3. Cover unquoted keys, CRLF and ordinary prose mentioning those words, while retaining digest behavior for valid frontmatter without host-affecting keys.

Check: equivalent YAML spellings receive the same exemption, host behavior is not lost through a digest denial, and `just check` passes.

### T560. Make the TOON test decoder handle quoted comma cells

Confirmed by an actual-source round-trip probe: [encode_cell](src/plugins/toon/mod.rs#L188) correctly quotes a string containing a comma, but the test-only [decode](src/plugins/toon/mod.rs#L225) unconditionally splits each row on every comma. The accepted table `[{"a":"x,y","b":1,"c":true}]` encodes to a three-column row and decodes to None because the oracle sees four cells. The round-trip cases at line 375 omit commas. T468 intentionally kept this decoder as the losslessness oracle after removing it from release code; this task repairs that verification path and makes no production-decoder/data-loss claim.

Execution plan:

1. Add the quoted-comma table to the round-trip cases and verify the current oracle fails on it.
2. Parse row cell boundaries with quote/escape awareness in the test decoder, matching the existing encoder's grammar; do not weaken the encoder or reject valid input to satisfy a broken oracle.
3. Add generated round-trip coverage combining commas, quotes, backslashes, control characters, empty strings and scalar-looking strings, and check declared row/column counts.

Check: decode(encode(table)) equals the original for all accepted generated fixtures, valid comma cells pass, malformed rows are rejected, and `just check` passes. This is a P3 test-quality defect.


### T562. Abort memory sync when an existing host file cannot be read

Source-confirmed: [sync read](src/plugins/memory/sync.rs#L119) uses read_to_string(...).unwrap_or_default(), treating every read error as an empty file. An existing CLAUDE.md containing an invalid UTF-8 byte therefore bypasses the managed-block guard; a normal sync prepares only the new memory block and [backs up and overwrites](src/plugins/memory/sync.rs#L138) the original, losing all outside content from the active host file. A backup does not make this replacement correct. A missing file is the only read failure that should enter the create path.

Execution plan:

1. Add a scratch host-file fixture with ordinary outside text and an invalid UTF-8 byte; assert sync fails and preserves the original bytes and stored digest.
2. Handle NotFound explicitly as a new file; propagate all other read failures before rendering, backups, writes or digest updates.
3. Cover unreadable files where permissions allow a reliable fixture, dry-run/remove/force variants, and successful creation of a genuinely missing file.

Check: failed reads never replace or remove host content or change sync metadata; existing valid managed-block behavior and `just check` pass. The isolated read/splice probe establishes the lossy replacement text; the live CLI write regression remains part of this fix.

### T563. Key memory-sync protection by the complete destination path

Source-confirmed: [kv_key](src/plugins/memory/sync.rs#L28) stores a block digest under `memory.sync.sha:<basename>`. Sync A/CLAUDE.md with one note set, then B/CLAUDE.md with another, using the same store. The second sync replaces the first file's digest. Re-syncing unchanged A now [fails as hand-edited](src/plugins/memory/sync.rs#L93); removing B also deletes the shared guard state. T69.6 established the hand-edit guard but does not cover this cross-file identity collision.

Execution plan:

1. Add a two-directory fixture sharing one store; sync different blocks and verify each file can be re-synced without force.
2. Derive the state key from a stable normalized absolute destination identity, consistently handling aliases and not falling back to another file's basename digest.
3. Cover removal of either destination, same-file relative/absolute spellings and legacy-key handling without falsely trusting unrelated host content.

Check: one destination's sync/removal cannot invalidate another's protection; genuine hand edits still require force; `just check` passes.

### T564. Keep memory-sync note titles from becoming block delimiters

Source-confirmed: [render](src/plugins/memory/sync.rs#L36) interpolates saved titles verbatim between its markers; [mem_save](src/plugins/memory/mod.rs#L420) accepts nonempty titles containing those markers. A title containing `<!-- /rtok:memory -->` creates an early END. The whole rendered block is hashed when saved, but [block_range](src/plugins/memory/sync.rs#L53) stops at the title's END: the next ordinary sync rejects its own unchanged output as hand-edited. A forced sync leaves the old title tail and actual closing marker behind. A title containing START instead makes the generated file look like multiple blocks.

Execution plan:

1. Add render/save/resync fixtures with START, END and multiline titles; demonstrate rejection of an unchanged generated block and the leftover tail after force.
2. Encode titles in the host-file presentation so they cannot emit structural markers; preserve the stored note title and define a deterministic single-line representation.
3. Validate generated block structure before writing and cover repeat sync, remove, force, budget accounting and preservation of bytes outside the genuine block.

Check: generated output has exactly one structural marker pair, normal repeated sync succeeds, and force/remove leave no generated tail; `just check` passes.

### T565. Include serialized metadata and separators in mem_pack's budget

Source-confirmed: [pack_notes](src/plugins/memory/pack.rs#L104) budgets tier text and a fixed separator cost, but [render](src/plugins/memory/pack.rs#L256) adds an id/tier header and separators afterwards. [mem_pack](src/plugins/memory/mod.rs#L461) returns that larger string without enforcing max_tokens again, contrary to the memory plugin invariant. With the four-characters-per-token estimator, hit (1, "a", "b"), no body and max_tokens=2, the pack reports 2 used tokens while the rendered text "1\tabstract\n1 a\nb" costs 4. T460 covers tier selection, not final serialization overhead.

Execution plan:

1. Add the minimal budget-2 fixture and multi-note cases asserting the estimate of the complete returned string stays within the requested effective budget.
2. Budget the same serialized representation that will be returned, including headers, newlines and separators, when admitting or deepening entries.
3. Derive used_tokens from the final representation; cover all tiers, Unicode, estimator settings and budgets too small for even one complete entry.

Check: estimate(render(pack)) <= max_tokens for every fixture, used_tokens agrees with the returned payload, note entries remain complete, and `just check` passes.

### T566. Preserve leading dots in memory note-file mentions

Source-confirmed: [clean](src/plugins/memory/files.rs#L56) strips periods from both ends of a path token. ".env" becomes "env" and is discarded by [mentioned](src/plugins/memory/files.rs#L95); "./src/lib.rs" becomes "/src/lib.rs" and is rejected as outside the fixture root. Worse, ".config/file" becomes "config/file" and links the wrong existing sibling if both exist. This affects note linking and prompt-driven recall. T374/T452 specify those features but do not track this normalization defect.

Execution plan:

1. Create existing .env, src/lib.rs, .config/file and config/file fixtures; assert hidden and dot-relative mentions resolve to the named file only.
2. Separate prose punctuation handling from path syntax so leading dots and dot-relative prefixes survive; preserve existing root confinement and position-suffix handling.
3. Cover Markdown/backtick mentions, trailing punctuation, line ranges, hidden-directory collisions and traversal rejection in both note linking and prompt recall.

Check: no-dot siblings are never substituted for explicitly named dot paths, valid mentions remain root-relative, outside paths stay excluded, and `just check` passes.

### T567. Apply negative pnpm workspace patterns before reading members

Source-confirmed: [members](src/project/refs.rs#L122) discards patterns beginning with ! instead of subtracting their matches. With packages/* and !packages/skip, an excluded packages/skip/package.json is still read by [npm discovery](src/project/refs/npm.rs#L44); its external file: dependency is returned and can be automatically registered by graph following. The [pnpm packages specification](https://pnpm.io/settings#packages) explicitly supports negative exclusions. This reproduces with single-level globs, independently of the recursive-glob limitation already acknowledged by T329.7.

Execution plan:

1. Add included and excluded sibling packages with distinct external dependencies; verify discovery returns only the included package's dependency.
2. Apply negative pnpm patterns to the selected member set before recording members or reading their dependencies, following pnpm semantics rather than silently dropping exclusions.
3. Cover multiple positives/negatives, overlapping patterns, nested exclusions and root-package inclusion; keep behavior of other manifest formats explicit.

Check: excluded members contribute neither references nor missing-reference warnings/auto-projects, included members still resolve, and `just check` passes.

### T569. Preserve each host file's undo bytes across backup deduplication and pruning

Confirmed with actual SDK functions: [backup_at](crates/rtok-agent-sdk/src/lib.rs#L181) skips a copy if any file in the shared _backup directory has identical bytes, regardless of basename. [prune_backups](crates/rtok-agent-sdk/src/lib.rs#L103) subsequently prunes generations of one basename without knowing other files depend on that copy. Create A.json and B.json with the same original text; with backup_files=1, write A1 to A, B1 to B, then A2 to A. B's backup was skipped, and the final A write removes the only copy of B's original bytes. T72 intentionally introduced basename-agnostic dedup and T249 per-basename pruning, but neither tracks this interaction.

Execution plan:

1. Add the two-file/three-write fixture using SDK write with backup enabled and a retention cap of 1; assert both files retain their required undo content.
2. Make deduplication and retention share an ownership model. Prefer independent per-basename backup identity unless cross-file ownership is explicitly tracked; never treat a foreign file's prunable copy as sufficient protection.
3. Cover repeated same-content writes, different basenames, same-second suffixes, lowered caps and cleanup via stale_backups; preserve intentional no-op behavior without losing another file's recovery bytes.

Check: pruning A cannot delete B's only required undo content; repeated installs remain idempotent, the retention cap remains well-defined per file, and `just check` passes. The isolated fixture exercised real SDK backup/write/prune functions against scratch files.

### T570. Fail SDK writes on symlink cycles instead of replacing a link

Confirmed on Unix with actual SDK functions: [link_target](crates/rtok-agent-sdk/src/lib.rs#L327) stops after 40 unsuccessful resolution attempts and returns the still-unresolved symlink. [write](crates/rtok-agent-sdk/src/lib.rs#L309) then calls [write_atomic](crates/rtok-agent-sdk/src/lib.rs#L352), whose rename replaces that symlink with a regular file. For cycle-a -> cycle-b -> cycle-a, write with backups enabled succeeds, replaces cycle-a, and makes cycle-b point to the new content; backup was skipped because exists() follows the cycle and returns false. This violates the SDK promise to preserve dotfile-manager symlinks. Existing dangling-link support does not cover loops.

Execution plan:

1. Add self-loop and two-link-cycle scratch fixtures; assert SDK write returns an error and preserves every link target byte-for-byte.
2. Return a Result from target resolution and reject cycles, exhausted hop limits and unresolved non-missing-path errors before atomic replacement; distinguish a supported dangling target from an unresolved link.
3. Cover valid relative/absolute link chains, a dangling final target with an existing parent, a missing target parent, dry-run and backup-enabled writes. Do not weaken valid dotfile-manager support.

Check: cyclic or over-limit chains never become regular files and produce a clear error, supported acyclic targets still update without replacing their links, and `just check` passes. The destructive behavior was reproduced only on isolated Unix fixtures.

### T571. Redact graph-export scope metadata and suggested download names

Confirmed by the actual redaction helper: [collect](src/plugins/graph/export.rs#L335) copies member names into meta.scope, but [redact](src/plugins/graph/export.rs#L351) rewrites projects[].name without touching meta.scope. With home /Users/alice and project alias alice, the default export reports `projects[0].name = <user>` and meta.redacted = true while meta.scope still contains alice. The page's [download name](src/plugins/graph/export.rs#L587) independently uses the original scope name, yielding rtok-graph-alice-overview.json even when export contents are redacted. T329.16/T329.40 promise default redaction and downloads but do not track these remaining copies.

Execution plan:

1. Add the username-as-project-alias fixture and assert the complete serialized redacted export contains no original alias; add a page-file assertion for its suggested download name.
2. Apply the same redaction policy to scope labels and derive the download name from a redacted display identity. Keep IDs, edges and non-sensitive names stable.
3. Cover CLI default output, MCP, page downloads, saved-export re-rendering and explicit CLI no-redact; verify JSON and image metadata use consistent labels.

Check: neither redacted payload metadata nor the suggested filename exposes the username alias, explicit no-redact retains its documented behavior, and `just check` passes. The scope leak was reproduced with the actual helper; the filename leak is source-confirmed and its end-to-end page regression belongs to this fix.

### T573. Apply terminal call state even when the live event list is capped

Confirmed with actual coalesce/fold/sweep functions: [coalesce](src/web/live.rs#L134) removes the oldest events beyond the display cap, including End events for calls already present in running. [CallsStore::fold](src/web/calls_store.rs#L213) clears running entries only from the retained events. Deliver Start A and Start B, then successful End A and End B in one batch with cap=1: the summary counts two completed calls, but A remains running because its End was discarded. At 120001 ms [sweep](src/web/calls_store.rs#L282) falsely marks A interrupted. The reader advances past the dropped End, so the next poll cannot repair it. Web and [TUI](src/tui/live_calls.rs#L118) share this state path. T329.15/T329.26 intentionally allow capped display rows and sampled tool/backend/latency bars; they do not permit a completed call to become a false interruption.

Execution plan:

1. Add the two-start/two-end fixture and an uncapped control; assert every completed call leaves running even when its End is not a displayed row.
2. Preserve terminal call identities independently of the capped display list, or apply lifecycle transitions before truncation. Keep bounded processing and the existing intentional display/statistics sampling contract.
3. Cover multiple omitted ends, ends without visible starts, interleaved progress, genuinely unfinished calls, late ends after a real timeout, and web/TUI consumption of the shared batch.

Check: successful omitted ends never create stale running or interrupted rows, pending calls remain visible, summaries and display caps remain correct, and `just check` passes. The isolated actual-source probe reproduced the false interruption and showed the uncapped control clearing both calls.

### T574. Decode SPA asset URL paths before looking up filenames

Confirmed through the actual Axum SPA router with both directory and embedded assets: [serve](src/web/spa.rs#L198) passes uri.path() directly to lookup, preserving percent escapes as literal filename text. A present assets/logo space.svg is requested as /assets/logo%20space.svg and returns 404; the same happens for filenames containing a hash, a percent sign or Unicode. An ordinary assets/plain.svg control returns 200. Neither [directory lookup](src/web/spa.rs#L117) nor embedded lookup decodes the name. T310.9 covers both sources but has no fix for encoded asset names.

Execution plan:

1. Add directory/embedded router fixtures for space, hash, percent and Unicode filenames, requesting their percent-encoded URLs and asserting the original bytes and MIME type.
2. Decode the URL path exactly once before filename lookup and route classification, then validate the decoded relative path with the existing confinement rules. Handle malformed escapes/invalid text explicitly rather than silently changing a filename.
3. Cover encoded traversal and separators, literal percent sequences without double decoding, missing assets, page fallback and negotiated compressed variants; retain the existing GET/HEAD behavior.

Check: encoded URLs resolve the intended existing asset in both sources, traversal remains rejected and unknown asset URLs stay 404, and `just check` passes. Actual router probes returned 404 for all four encoded-name fixtures and 200 for the plain-name control; HEAD controls correctly returned no body.

### T576. Bind Doctor web writes to the confirmed preview

Confirmed in the selection path: [Selection](src/doctor/web.rs#L31) carries only kept-copy swaps and toggles, and [prepare](src/doctor/web.rs#L80) rebuilds candidates and default selections on both plan and apply. Preview a removable broken hook A in a user-level settings file, then add another removable broken hook B before confirming the original preview. With the original empty selection delta, B is also selected during apply although the page confirmed only A. [DoctorFix confirmation](web/src/pages/DoctorFix.tsx#L38) sends the deltas without preview identities or file versions. [fix_found](src/doctor/fix.rs#L284) edits the fresh selected findings; [fix_file's reread](src/doctor/fix.rs#L366) compares bytes read within that apply request, not those shown in the earlier preview. T331.12's fresh rescan prevents use of stale findings but does not bind writes to the confirmed diff.

Execution plan:

1. Add a fake-filesystem plan/apply regression: preview A, introduce B, submit the original confirmation and assert B is untouched. Cover insertion in the same file and a newly discovered file, plus an unchanged-preview success control.
2. Carry an explicit set of confirmed identities and content/version evidence from the preview into apply. Revalidate the selected entries and affected files before writing; refuse a changed preview and return a fresh plan requiring confirmation. Do not auto-select findings discovered after the preview or treat a reused array position as the same entry.
3. Wire the preview evidence through the web protocol and page state; cover toggles, kept-copy swaps, shared-file defaults, reordered/replaced entries, stale requests and backup/write failures. Retain the immediate pre-write reread as a separate safeguard.

Check: apply never removes an entry absent from the confirmed preview or silently changes the confirmed diff, unchanged selections still work, and `just check` passes. The isolated probe compiled actual prepare/shared/default-selection functions with fixture discovery/removability: the same selection chose A before the rescan and A+B after it. Host parsing and writes were source-traced, not executed; full fake-filesystem and page regressions belong to this fix.

### T577. Protect nested package-manager lockfiles during junk cleanup

Confirmed with an isolated directory fixture: [recheck](src/agents/junk_clear.rs#L278) compares PACKAGE_LOCKS only with the candidate's own basename. Its protected-path list contains the store and host settings paths, not discovered descendant lockfiles. An eligible explicitly configured cache containing nested/Cargo.lock passes recheck, then [remove](src/agents/junk_clear.rs#L334) recursively discards its children and deletes that lockfile. The cache root remains, while the same Cargo.lock checked directly is refused. [extra_items](src/agents/junk_review.rs#L169) accepts the configured cache root; the plan's D36/age/running gates do not add a descendant-lockfile check. This contradicts completed T330.4's promise to refuse a path that is or holds a package-manager lockfile; T330.5.1's explicit exclude globs do not supply the missing automatic protection.

Execution plan:

1. Add scratch-tree cleanup regressions for an eligible extra cache with direct-child and deeper Cargo.lock files. Assert the parent is refused and all contents remain; retain controls for a direct lockfile, a cache without protected descendants, settings/store descendants and CACHEDIR.TAG.
2. Share a bounded descendant-protection check between planning and the immediate pre-delete recheck. Match the existing PACKAGE_LOCKS set, never follow symlinks, and refuse deletion when inspection fails or times out. Protect the entire removal candidate if it contains a protected lockfile.
3. Cover tagged build/cache roots, extra cache roots and recursively removed temp entries, a lockfile appearing after preview, all supported package lock names, unreadable subtrees and trash/permanent-delete paths. Preserve existing store/settings and exclude protections and clear refusal reasons.

Check: no planned recursive deletion or cache emptying removes a protected descendant lockfile, unknown inspection results remain non-destructive, and `just check` passes. The actual recheck/newest/usage-walk/remove functions and permanent-delete branch reproduced the loss; a synthetic clock made the fixture older than the settle window. This was not a full report/CLI or trash integration test.

### T579. Count each transcript once when scanning directory aliases

Confirmed by the actual [paths_with_ext](src/measure/codex.rs#L28) walker: a root with real/s.jsonl returns one path; adding alias pointing to real returns two paths that canonicalize to the same file. The walker follows directory symlinks through Path::is_dir and has no visited-directory or file-identity set. [Codex requests](src/measure/codex.rs#L77), [Claude usage](src/measure/usage.rs#L115) and [stats collect](src/measure/stats.rs#L1098) parse and accumulate each returned path; a path-keyed transcript cache does not remove these aliases. Thus a single recorded request/session can contribute twice to usage and statistics. T135 addresses avoiding repeated parsing of unchanged paths, not alias deduplication.

Execution plan:

1. Add a filesystem fixture with one transcript, a directory alias and an unlinked control. Assert the discovered physical file and its usage/session contribution are counted once; include a symlink cycle and a symlink to a file where supported.
2. Define one shared traversal policy for transcript readers: resolve the configured root, then either avoid following nested symlinks or track canonical directory/file identities before traversal and emission. Keep legitimate distinct transcripts and supported linked roots readable, preserve cutoff filtering and deterministic ordering, and fail open on inaccessible entries.
3. Cover Claude/Codex usage, stats and transcript-cache callers, and consolidate the separate jsonl::parse_dir traversal so it cannot reintroduce alias double counting. Verify subagent attribution, missing targets and out-of-root links under the chosen policy.

Check: adding an alias never changes totals for the same physical transcript and cycles terminate without repeated parsing; `just check` passes. The scratch probe exercised the actual walker and proved duplicate canonical paths; downstream accumulation was source-traced, not run through a complete host parser or CLI.

### T580. Include report metadata and anchors in the AI output budget

Confirmed with the actual [AI render loop](src/report/ai.rs#L41) and [token estimator](src/tokens.rs#L22): admission subtracts only section-body estimates, then appends the report header, date range/store path, dropped-section list and section anchors outside the budget. Eight controlled bodies admitted at an 808-token budget produced a final 840-token estimate. A longer store path adds more unbudgeted output. T22.4 and report.budget_tokens describe a cap on the AI report; existing tight-budget tests check whole-section omission but never estimate the final output. T565 covers a similar issue in mem_pack, not this renderer.

Execution plan:

1. Add final-output estimator assertions using real Document fixtures at a tight budget and exact admission boundaries, including long store paths, all-kept and partially dropped sections.
2. Budget the complete serialized report, including anchors and the actual dropped list. Recompute admission when dropping a section changes metadata; preserve whole sections, their order and byte-stable output.
3. Define explicit behavior when even the minimal metadata does not fit, and cover zero/tiny budgets, non-default estimator rates, Unicode and both TOON and fallback table rendering. Do not silently emit output above the configured cap.

Check: every successful AI report stays within report.budget_tokens according to the configured estimator, while tables remain whole and drops are accurately named; `just check` passes. The isolated probe used actual render/admission and estimator functions with controlled section bodies and minimal model/config stubs, not the full document renderer.

### T581. Filter actionable report sinks before applying the top-ten limit

Confirmed through actual aggregation and recommendation functions: [report_sinks](src/model/report.rs#L284) groups all catalogue plugins, sorts by before_bytes and truncates to ten. Only afterwards [top_sinks](src/report/advice.rs#L253) skips classes other than read/cmd/mcp. Ten larger archive rows with distinct references therefore displace a smaller Read sink, yielding no top-sinks recommendation; the Read-only control yields one. The archive rows are valid measurements, but deliberately have no actionable switch in this rule. T59.8 and docs/report.md promise the ten largest file/command/MCP sinks; T302 changed ranking units, not filtering before truncation.

Execution plan:

1. Add an integration fixture with ten distinct positive-byte archive measurements and a smaller Read measurement. Assert that the Read recommendation remains; include cmd and MCP wrap controls and more than ten eligible sinks.
2. Apply the actionable-class predicate before truncating to ten, or retain all grouped sinks until the recommendation layer filters and limits them. Share the eligibility rule so aggregation and presentation cannot drift; preserve deterministic tie ordering and existing switch labels.
3. Cover mixed actionable/non-actionable classes, repeated references that aggregate into one sink, nonpositive byte counts and empty ledgers. Verify that recommendations expose at most ten eligible sinks and keep their Measurement evidence.

Check: unrelated archive/injection rows cannot hide eligible read/cmd/MCP recommendations, the largest eligible sinks remain ranked correctly, and `just check` passes. The scratch probe compiled actual report_sinks/sink_label/sink_switch and top_sinks with an in-memory Store fixture and a catalogue subset containing the relevant real plugins; it was not a Diesel or CLI integration test.

### T583. Make Batch result accounting idempotent across downloads

Confirmed in the recording path: [batch_results::record](src/proxy/batch_results.rs#L56) inserts every parsed successful result under the current GET call_id. [ResultUsage](src/proxy/batch_results.rs#L90) retains model/API/counters but drops the result/custom_id, and record receives no batch or output-file identity. Fetching the same result again therefore adds the same counters under a new call. [Store insertion](crates/rtok-store/src/call_log.rs#L261) is a plain INSERT and [lane totals](crates/rtok-store/src/call_log.rs#L509) sum both copies; Batch model pricing also consumes these rows. In the isolated probe, one result with 100 input and 20 output tokens produced 120 after the first recording and 240 after replay, without another inference request. T385.4 covers recording result lines and T385.12.1 prices Batch usage, but neither implements replay identity. Refusing an existing CLI output path does not prevent another GET or a fetch to another file.

Execution plan:

1. Add store/proxy regressions that download the same successful Batch artifact twice under distinct call IDs and assert one usage contribution per original result. Keep both download calls observable; cover Anthropic results and OpenAI output-file downloads.
2. Preserve a stable, provider-scoped batch/file/result identity through parsing and recording. Scope it to the upstream/account as needed so identical custom IDs in different jobs are not collapsed; do not deduplicate solely by usage values or response text.
3. Persist the result identity and usage contribution atomically through Diesel with a uniqueness rule that survives concurrent fetches and restarts. Let later downloads add previously unrecorded complete results after a partial/truncated download; keep malformed/error lines uncounted and preserve response bytes.
4. Verify lane/model/pricing aggregates remain unchanged on replay but grow for distinct results and jobs. Define compatibility for older rows lacking result identity without guessing which historical entries represent the same request.

Check: downloading existing results cannot inflate provider-token totals or estimated Batch costs; distinct billed requests still count independently, and `just check` passes. The probe used actual Batch record/parse functions with a plain-append Store fixture matching the source INSERT semantics; Diesel, HTTP and provider integration were source-traced, not executed.

### T584. Preserve complete SSE usage events before an incomplete UTF-8 tail

Confirmed with actual wire parsers: [usage_from_response](src/proxy/wire.rs#L370) and [service_tier_from_response](src/proxy/wire.rs#L395) validate the entire captured buffer as UTF-8 before reading any events. A complete Anthropic message_start containing input_tokens=100 and service_tier=standard is lost when a later content delta ends after only the first byte of a multibyte character. Both parsers return None; the same prefix cut before that character and the full-body control preserve the earlier values. [The tee](src/proxy/mod.rs#L558) retains arbitrary byte prefixes on a capture cap, upstream error or client disconnect. [finish](src/proxy/mod.rs#L1082) then records an all-zero usage row and omits the tier despite having a complete earlier usage event. T323's acknowledged capture limit and no-partial-cache rule do not require discarding complete events already retained; T326 covers terminal provider events, not this decoding failure.

Execution plan:

1. Add parser regressions with a complete usage/tier event followed by each possible split within a multibyte SSE data character. Compare full response, ASCII-safe incomplete tail, malformed later line and no-usage controls.
2. Decode complete SSE events independently from captured bytes, retaining valid preceding events while ignoring an incomplete trailing event. Share framing between usage and service-tier extraction; do not make incomplete ordinary JSON responses valid or invent missing usage counters.
3. Add a mock streaming regression ending after a split UTF-8 chunk, plus a small configurable capture-limit fixture. Verify retained counters/tier reach the store, missing final output counters remain explicitly partial, original response bytes are forwarded unchanged and truncated responses still cannot enter the semantic cache.

Check: a damaged or incomplete trailing SSE event cannot erase earlier complete usage/tier events, genuinely absent counters remain unknown, and `just check` passes. The isolated probe exercised actual wire/Anthropic extraction functions with synthetic byte prefixes; transport cancellation, the 256 MiB cap and store persistence were source-traced rather than exercised end to end.

### T586. Resolve local relative remotes before deriving project identity

Confirmed with separate Git repositories: [project_key_with](src/project.rs#L42) passes the origin string into [canonical_url](src/project.rs#L120) without its checkout context. Two checkouts at a/checkout and b/checkout, each configured with origin ../remote.git, target different bare repositories but both receive the key ../remote. Git ls-remote controls returned different fixture refs, while the actual project resolver returned equal keys. Replacing A's relative URL with the absolute path of the same remote also changed its key. [Task projects](src/tasks/run.rs#L56) use this key for counters, and [claim injection](src/hooks/mod.rs#L162) uses it to select project claims. Unrelated repositories can therefore share a namespace, while alternate spellings of one local remote split it. T441.3's canonical origin identity has no relative-path exception or existing fix for this case.

Execution plan:

1. Add two-checkout fixtures with identical relative origin strings resolving to different repositories; assert distinct keys. Add relative/absolute/file-URL spellings of the same local remote and assert one identity, using real Git resolution as a control.
2. Distinguish local filesystem remotes from network URL and scp forms before normalization. Resolve relative local paths with the proper checkout context, canonicalize existing destinations and provide a stable absolute fallback for missing ones. Preserve filesystem case semantics and validate linked-worktree behavior against the existing shared-project contract.
3. Cover nested cwd, dot segments, symlinks, spaces and supported Windows path forms without regressing network remote normalization. Verify task-counter and claim lookups remain isolated between different repositories.
4. Handle persisted old keys explicitly: seed/reconcile counters from the intended project's adapter as needed, and avoid blindly migrating an ambiguous shared relative key into every project. Test compatibility without silently resetting IDs or moving unrelated claim rows.

Check: different resolved local remotes never collide solely because their relative strings match, equivalent local spellings share a key, existing state is handled deliberately, and `just check` passes. The isolated harness compiled actual project resolver/normalization functions with a standard filesystem adapter; Git controls used only scratch local repositories. Store counter/claim behavior was source-traced, not exercised end to end.

### T587. Let an explicit report --format md override configuration

Confirmed through actual Clap argument parsing and [report_flags](src/cli/report.rs#L32): [Args.format](src/cli/report.rs#L63) defaults to Md, and the flag layer treats every Md value as omitted. With report.format=html in a lower-precedence config layer, explicitly passing --format md produces no format override and the effective value remains html; --format pdf correctly overrides it. [run](src/cli/report.rs#L89) merges this flag layer and selects the renderer using cfg.report.format. Thus the explicit CLI request can produce HTML or PDF instead of Markdown. D12 and docs/config.md promise flags override configuration; T22.1/T22.2 and T60 do not cover explicit selection of the built-in default.

Execution plan:

1. Add argument/config regressions for configured html and pdf with omitted --format, explicit md and explicit pdf/html. Assert omission preserves configuration but every explicit format wins; include combinations with --out and --since.
2. Preserve whether --format was supplied, for example with an Option<ReportFormat>, and insert every explicitly supplied value into the flag layer. Keep md as the configuration default and preserve the documented --ai precedence.
3. Cover environment and project/user config layers, help/completion output, and a CLI rendering fixture that verifies Markdown bytes for an explicit --format md over configured HTML/PDF.

Check: explicit format flags always select the requested renderer, an omitted flag still respects lower-precedence settings, and `just check` passes. The scratch probe used actual ReportFormat/Args/report_flags with Clap and a Figment config layer; full report construction and file output were not run.

### T601. Prevent an expired task-lock guard from deleting its successor's lock

Confirmed with actual [lock_claim](src/tasks/disk.rs#L219) and [DiskGuard::drop](src/tasks/disk.rs#L64): A acquires `.claim.lock`; after the real stale interval, B takes over while A's guard still exists. Dropping A unconditionally unlinks the shared pathname, deleting B's fresh lock. C then acquires immediately while B still holds its guard, bypassing the normal rejection of fresh-lock contenders. A slow or paused holder can trigger this; no process crash is required. [Project::claim](src/tasks/run.rs#L224) relies on that guard to protect read/check/save, so the broken exclusion can permit conflicting task writes.

T442 explicitly chose ten-second stale takeover; that timeout policy alone is not filed again. The uncovered defect is the previous owner's release removing its successor's lock. The [existing stale/fresh test](src/tasks/disk.rs#L517) ages an ownerless file and never keeps the displaced guard alive. T541 concerns filename/frontmatter mismatches, not lock ownership.

Execution plan:

1. Add a deterministic A/B/C regression around takeover and delayed release. Assert A's release cannot remove B's lock and C cannot enter B's protected section. Keep fresh-contention and release/reacquisition controls.
2. Use an ownership-safe cross-process lock, preferably the existing platform file-lock abstraction, with a stable lock file and guard-owned handle. If leases remain necessary, add ownership and write fencing; a pathname existence check followed by unlink alone still races with replacement.
3. Define recovery for paused and crashed holders so an expired writer cannot resume protected writes concurrently with its successor. Preserve bounded wait/error behavior and coordinate claim, release, status and other disk mutations through the same mechanism.
4. Add isolated multi-process claim/save tests with delayed and terminated owners. Verify only one claimant wins and later guard release cannot expose another writer's critical section, then run task/disk tests and `just check`.

Check: releasing an older guard never removes a newer holder's exclusion, and protected task writes remain serialized. The scratch executable compiled actual guard/acquisition/staleness functions and used real temporary lock files and an 11-second wait; it held A/B/C guards in one process to reproduce the exact interleaving. Full task writes, multiple OS processes and remote adapters were not exercised.

### T604. Keep literal strings distinct from JSON-tree variable references

Confirmed on `e06ada0852e3cc28e5dcaa04d26faa6bdc2bc5ab` with the actual folding functions. In a root object's `children`, give nodes `a` and `b` the same object-valued `value`, then give node `literal` a string equal to that object's generated `V-<sha1-8>` identifier. [insert_field](src/plugins/json_tree/mod.rs#L323) represents a hoisted object as `Value::String(id)`, exactly like the original literal. [collect_bodies](src/plugins/json_tree/mod.rs#L361) and [stamp](src/plugins/json_tree/mod.rs#L372) consequently assign all three nodes the same element template. The probe produced `EL-a5ce585c: {"type":"ITEM","value":"V-65c37664"}` for all three, although the third input value is a string and the other two are objects. Changing the literal to `ordinary-literal` leaves only the two object-valued nodes templated.

This is a collision between user data and the encoder's reference representation, not a SHA-1 hash collision. The folded preview incorrectly conflates values and cannot tell a reader which occurrence is literal. The archived original remains recoverable; no irreversible archive loss is claimed. T475 implements hoisting and T554 inspected general folding behavior, but neither covers this case. T532's typed cache-key collision concerns a separate mechanism.

Execution plan:

1. Add a deterministic regression using two identical object-valued fields and one literal equal to their generated variable id. Assert that the literal retains its string meaning and is not assigned the same element body as the references. Retain the ordinary-literal control and insertion-order determinism checks.
2. Keep generated references distinct from JSON strings in the internal folded representation, including the body identity used for template counting and stamping. Do not infer a reference solely from a string's spelling; preserve the source JSON value until the encoder explicitly replaces it with a typed reference.
3. Render an unambiguous distinction between a reference and literal reference-like text in both inline fields and `ELEMENTS` bodies. Make variable-use collection follow actual references, covering nested user strings, identity extras and reserved-looking literals without treating them as encoder metadata.
4. Exercise proxy and MCP folding with the collision fixture, including their size gates and Measurements. Verify that folding retains the distinction, stored-pointer replay is stable, and `expand` still returns the exact original bytes. Run focused JSON-tree/MCP tests and `just check`.

Check: the reproduced object/string fields remain distinguishable in the folded output while repeated object bodies still share templates. The offline probe compiled extracted production fold/render/hash functions and the actual TOON eligibility helper. It also executed the extracted MCP fold wrapper with a minimal enabled config and recording/estimation adapters: the collision fixture was accepted at 1,075 output bytes versus 1,932 original bytes. Full CLI/proxy transport, archive storage and a production tokenizer were not executed.

### T606. Preserve fitting Unicode mode files when bounding injection reads

Confirmed with the actual [read_mode](src/plugins/inject/mod.rs#L119), [fit_budget](src/plugin.rs#L37), [apply](src/plugins/inject/mod.rs#L148) and [token estimator](src/tokens.rs#L22). `read_mode` computes `ceil(budget * prose_rate) + 1` and reads that many **bytes**, while the estimator divides the number of Unicode **characters** by the prose rate. At the defaults of 800 tokens and rate 4.2, a mode containing 1,000 four-byte characters followed by `\nKEEP_THIS_TAIL\n` is only 242 estimated tokens (1,016 characters), but exceeds the 3,361-byte read cap. The reader returns 3,360 bytes and loses the tail; the final injection has no `dropped:` marker. This example even fits the README's 250-token per-mode guidance. A 2,016-character Cyrillic mode likewise loses its tail despite fitting the shared 800-token budget; the same-length ASCII control remains intact.

The truncation happens before [modes_text](src/plugins/inject/mod.rs#L87) passes the mode to `fit_budget` and before aggregate injection accounting, so neither stage can recover the unread instructions. This is unnecessary truncation of content that fits the configured estimator budget, not a claim about actual model token billing. T188 introduced bounded mode reads to fix oversized input and tested a 1 MB ASCII mode; it does not test Unicode modes that already fit the token budget. T189 addresses PostToolUse archive references; neither covers this byte/character mismatch while loading mode files.

Execution plan:

1. Add file-backed regressions for two-, three- and four-byte UTF-8 text that fits the configured estimated-token budget but exceeds the current byte cap. Include a distinctive tail and assert that the entire fitting mode survives; retain ASCII and missing-file builtin controls.
2. Bound file I/O in a way consistent with the estimator's character budget. Account for UTF-8's maximum bytes per scalar or decode incrementally with a bounded character allowance, using checked arithmetic and preserving character boundaries. Keep large files bounded rather than reading them all into memory.
3. Run the decoded text through the shared budget fitter. Preserve fitting content and define how genuine truncation is reported/accounted for without changing T2.4's intentional candidate-selection policy. Check zero/small budgets, partial UTF-8 at the read boundary and multiple mode files.
4. Add SessionStart coverage with an actual Unicode mode file: assert the fitting tail survives, repeated unchanged runs remain byte-identical, and mode text is absent from UserPromptSubmit. Run focused injection/token tests and `just check`.

Check: the reproduced 242-token mode survives in full under budget 800; genuinely oversized files remain bounded and existing priority/budget behavior stays intact. The isolated executable compiled the production read, fit, apply and estimate functions, used real scratch files and minimal config/Measurement adapters, and observed stable repeated output. ASCII and missing-file builtin controls retained all fitting text. Full hook dispatch, checkpoint composition and persistent Measurement storage were source-traced or excluded, not executed.

### T608. Preserve identifier and path suffixes in prose word stripping

The public [compress_prose](src/modes/compress.rs#L82) helper changes `Edit foo_a now` to `Edit foo_ now`, `Open /tmp/a now` to `Open /tmp/ now`, and `Keep item1a here` to `Keep item1 here` at Full intensity. [strip_words](src/modes/compress.rs#L259) starts a removable word after an underscore, slash or digit but checks only the character following that word for identifier/path glue. Thus the final `a` is mistaken for an article and deleted. These are changes to names, not whitespace or filler compression. The existing `a.txt`/`a-b`/`a/b`/`a1` tests cover following glue, not preceding glue.

Execution plan:

1. Add Full and Ultra regressions for `foo_a`, `/tmp/a`, `item1a`, and analogous suffixes matching filler words. Assert preservation of the entire name, not merely the surrounding sentence. Retain standalone-article removal and existing right-boundary cases.
2. Decide whether a token is standalone from both original input boundaries. Treat preceding identifier/path punctuation and digits consistently with the current following-boundary protection, without making the decision from already edited output.
3. Cover combinations of left/right glue, Unicode neighbors, negation words and protected inline/fenced code; retain Lite behavior and deterministic output. Keep genuine prose article/filler removal working.
4. Add realistic identifier/path fixtures to the mode comparison tests, checking exact retained names before recording compression measurements. Run focused mode unit/benchmark tests and `just check`.

Check: the three reproduced names remain byte-identical while `Edit the file` still becomes `Edit file`. The isolated executable imported the actual compression module by path and reproduced all three changes; Lite and `a.txt` controls preserved their names. This is a P3 library/benchmark correctness defect: these native helpers are currently used by tests and comparisons, not wired into production hook compression.

### T609. Preserve code inside longer Markdown backtick fences

The public [compress_prose](src/modes/compress.rs#L82) helper closes a fence at the next occurrence of exactly three backticks, without remembering the opening delimiter length or requiring a closing fence line. For a four-backtick Markdown block containing a triple-backtick example followed by `The really exact content`, Full compression removes `The really` inside the outer code block. The source promise to preserve fenced code verbatim is therefore violated. This is distinct from inline code-span protection and from stripping identifiers outside code.

Execution plan:

1. Add a golden regression for a four-backtick outer fence containing a triple-backtick example and filler-like code text. Assert exact preservation of the whole fenced region, including delimiters, spacing and newlines, at every intensity.
2. Recognize fence openings and closures by line position, delimiter kind and opening run length. A shorter internal run or an occurrence inside a code line must not close the block. Preserve unclosed blocks through the end as today; define and test supported Markdown fence forms.
3. Add controls for ordinary triple fences, longer matching closers, code lines containing backtick runs, and prose around the fence. Verify that inline-span handling remains independent and that prose outside the block still compresses.
4. Extend `tests/mode_bench.rs` with the reproduced fixture and an independent preservation oracle: its current `fence_bodies` helper also splits on raw triple-backtick substrings and cannot establish correctness for this case. Run focused mode tests, comparison tests and `just check`.

Check: the complete reproduced outer code block is byte-for-byte unchanged. An isolated executable imported the actual compression module by path, reproduced the four-backtick mutation, and passed ordinary triple-fence and double-backtick inline controls. This is a P3 library/benchmark defect, not observed production hook corruption; the native compression helper is not currently wired into those hooks.

### T611. Decode serialized Copilot tool arguments before hook dispatch

GitHub's [Copilot CLI hook tutorial](https://docs.github.com/en/copilot/tutorials/copilot-cli-hooks#understand-the-pretooluse-input), checked 2026-10-11, documents `toolArgs` as a JSON string requiring parsing before accessing its fields. [HookInput::adapt_copilot](src/hooks/types.rs#L134) instead assigns that `Value` directly to `tool_input`. For `toolName: "bash"` and `toolArgs: "{\"command\":\"git status\"}"`, adaptation produces `tool_name = "Bash"` but leaves `tool_input` a string. The [main-argument reader](src/hooks/mod.rs#L138) returns `None`; similarly, a serialized `view` input with `path` gives [guard::cache_key](src/plugins/guard/mod.rs#L219) no Read key. Object-valued versions of the same fixtures work. Consequently the documented CLI payload bypasses argument-dependent duplicate-read handling and loses command/path activity detail even though the tool name was normalized correctly.

This is distinct from T262.5, which stores already adapted host input for ledger consumers: storing this adapter's output still stores a string instead of parsed arguments. The existing Copilot adapter tests use object-valued `toolArgs`, so they miss the documented CLI representation. No claim is made that all Copilot versions or transports send identical argument types.

Execution plan:

1. Add adapter regressions for JSON-string and object `toolArgs` on PreToolUse and PostToolUse. Assert equal normalized argument objects for equivalent inputs, including `command` and `path`, while existing `tool_input` retains precedence.
2. Decode serialized arguments at the host adapter boundary, reusing the existing JSON-like field handling where appropriate instead of duplicating it. Preserve object inputs, unknown fields and fail-open behavior for malformed/non-object arguments; do not replace a valid existing normalized input.
3. Add dispatch-level coverage showing that a serialized read can populate and consult the guard cache and that command/path activity summaries retain their main argument. Check that ledger storage receives the normalized object, retaining T262.5's behavior.
4. Retain malformed-string, absent-argument and already-normalized controls, cover the installed Copilot event spellings, then run focused hook/guard tests and `just check`.

Check: equivalent documented string and supported object payloads reach consumers with equivalent argument objects. The isolated executable compiled the actual HookInput struct/serde, Copilot adapter and naming helpers, main-argument reader, and Read cache-key branch. The unused Bash-only key helpers were panic-only stubs and were never called. No real Copilot process, complete hook dispatch, persistent store or end-to-end guard denial was exercised; those effects were source-traced, with integration tests prescribed above.

### T613. Attach sub-agents when their main agent is registered later

Confirmed on `2d91199fdb507af71eedbcf1b7ed127a06b39784` using the actual in-memory Store. Register a sub-agent with `register_agent(host, session, Some("sub-1"), ...)` before that session's main row, then register the main agent and repeat the sub-agent registration. [register_agent](crates/rtok-store/src/agents.rs#L202) initially obtains no parent from `main_agent_id`, inserts `parent_id = NULL`, and its conflict-update fields never repair that parent. The repeated call returns the same sub-agent id but leaves it unattached; `agent_children(main)` is empty. Hook dispatch directly passes the input's sub-agent key to this method, so the first observed event of a session being a sub-agent event is not rejected or repaired by the caller.

The broken relation also affects [session_views](src/model/agents.rs#L73). Both rows become roots, and the first root matching the host/session is selected as the session's main agent. The probe registered the child more than one second earlier to make the store's ordering deterministic: `agents_of_sessions` returned it first, and the actual model selected it instead of the real main row. This can display the wrong agent identity/activity/state and omit the intended child relationship. T282's existing parent test registers the main row first; T284 covers ordinary nesting, and T324 covers revival on repeat registration. None covers a late main row repairing an existing child.

Execution plan:

1. Add store regressions for sub-agent-first registration, subsequent main registration and child re-registration. Assert stable ids, exactly one main row and correct `parent_id`/`agent_children` results. Retain the main-first control and isolate host/session identities.
2. Establish the parent relation in the registry layer for either arrival order. Ensure or reconcile the main row and repair existing unparented sub-agent rows through typed Diesel operations, without changing a legitimate relationship or clearing activity/status fields. Make the operation safe under concurrent registrations and partial failures.
3. Add model and hook-dispatch regressions for a first observed sub-agent event followed by a main event. Assert that the true main agent is selected and the child is nested; include ended/live filtering so an orphan's state cannot hide or mislabel the session. Preserve same-id repeat behavior and existing T324 revival semantics.
4. Cover pre-existing orphan rows through the chosen reconciliation path without requiring their ids to change. Run focused store/agent/model/hook tests and `just check`.

Check: event arrival order no longer changes parentage or which agent represents the session. The isolated executable linked the actual `rtok-store` crate, used its in-memory migrations and public methods, imported actual agent-view types and compiled the production pure model functions. The two orders and repeated-registration controls were executed without raw SQL or mocked store behavior. SessionTotals were synthetic; full CLI output, live hosts, real agents and concurrent OS processes were not exercised.

### T615. Preserve claim order when tasks are claimed in the same second

Confirmed on `e8d36a0c6fdfb27f37ca97ceb33e6230bee545d1` with the actual in-memory Store. [upsert_task_claim](crates/rtok-store/src/task_claims.rs#L36) records `since` in whole Unix seconds, while [latest_task_claim](crates/rtok-store/src/task_claims.rs#L78) sorts by `since DESC, task_id ASC`. Claim `R1` and then `R2` for the same project/agent within one second: the method returns `R1` and its old title. Re-claiming `R2` within that second still returns `R1`. The existing test claims `R1`, `R2`, then `R1` again, so its expected result coincides with the lexical tie-break and misses the defect.

The task runner writes this row after a successful claim and on a same-agent re-claim ([claim_held](src/tasks/run.rs#L245)); it does not restrict an agent to one active task. [claim_injection](src/hooks/mod.rs#L159) uses the result directly for the SessionStart/PostCompact task line, despite its explicit newest-row contract. Fast successive claims can therefore inject the earlier task instead of the task most recently selected. This is separate from T601's disk lock ownership race and T592's OTel cursor ordering.

Execution plan:

1. Add deterministic store regressions for equal-second `R1` then `R2`, the reverse order, and re-claiming either existing row. Assert last successful claim order, not lexical ID order; keep separate-second, project/agent isolation and release-fallback controls.
2. Persist an ordering value that advances on every successful claim and re-claim, including updates of existing rows. Use typed Diesel operations and a transaction-safe ordering scheme; retain `since` for elapsed-time/display semantics. Include a compatible migration and deterministic handling of pre-existing ties.
3. Cover the runner-to-hook path so SessionStart/PostCompact select the latest claimed task and its current first-line title, including rapid re-claims. Preserve release/finish cleanup and the injection budget.
4. Run focused task-claim, runner and hook tests and `just check`.

Check: selecting the latest claim is independent of task ID spelling when timestamps tie. The isolated probe used real Store migrations and public APIs, checked that all reproducing writes occurred in one wall-clock second, and exercised re-claim, release fallback, project/agent isolation and separate-second controls. No raw SQL, full hook dispatch, live host or concurrent process was exercised; the injected-text effect was source-traced.

### T617. Limit file-linked observation recall after deduplicating observations

Confirmed on `2c7c40b2d8af0673f085d71d8d3e1da0051084e5` with actual Store migrations and public APIs. [observations_for_files](crates/rtok-store/src/observations.rs#L122) joins `observation_files` to `observations`, then orders and limits joined rows without making observation IDs distinct. Insert an older observation linked to `a.rs`, then a newer one linked to both `a.rs` and `b.rs`; querying both paths with limit 2 returns the newer ID twice and omits the older matching observation. [obs_for_prompt](src/plugins/memory/mod.rs#L299) feeds these rows into [rrf_obs](src/plugins/memory/observe.rs#L180), which both gives duplicate IDs extra rank contributions and merges them after the store limit has already discarded other hits. The real ranking helper reduces this fixture to one hit despite two eligible observations.

Execution plan:

1. Add store regressions with multiple file links per observation and several distinct observations. Assert unique IDs, newest-first ordering and a limit counted in observations; include project isolation, overlapping paths and single-path controls.
2. Express an existence/subquery or distinct projection through Diesel so each observation participates once before ordering and LIMIT. Preserve global/project visibility and the existing empty-path behavior.
3. Add recall/ranking coverage proving extra matched files do not create duplicate contributions within one result list or starve older distinct hits. Preserve the intentional boost when an observation appears in separate FTS and file-recall lists.
4. Run focused store observation and memory recall tests, then `just check`.

Check: the two-path fixture returns both distinct observations, and fusion retains both. Executed evidence used the real in-memory Store and the verbatim production `rrf_obs`; single-path and other-project controls passed. Full hook dispatch and live host integration were not run.

### T618. Scrub private observation content before truncation removes its delimiters

Confirmed on `2c7c40b2d8af0673f085d71d8d3e1da0051084e5` with the production [synthesize](src/plugins/memory/observe.rs#L153) and [strip_private](src/plugins/memory/scrub.rs#L48). A complete output `<private>SYNTHETIC_PRIVATE_MARKER` + 300 filler characters + `</private> public` is redacted by `strip_private` when passed whole. `synthesize` first truncates each input/output string to 200 characters, then the combined narrative to 400, and only then scrubs. That removes the closing tag; the paired-tag regex no longer matches, so the private marker remains in the narrative. The real Store persisted and returned that narrative in the isolated probe.

[capture](src/plugins/memory/observe.rs#L245) passes this synthesized narrative straight to `insert_observation`. Short complete blocks are covered by existing tests and redact correctly; the long complete-block case is not. T548 covers checkpoint prompts bypassing private-tag filtering entirely; this is a separate observation path where truncation defeats an existing scrubber.

Execution plan:

1. Add synthetic private-block regressions crossing the 200-character per-field and 400-character combined limits, for both tool input and response. Assert private text never reaches the stored narrative; retain short-block, ordinary-text and Unicode-cap controls.
2. Scrub complete field content before applying lossy truncation, using the shared privacy function. Ensure any bounded implementation tracks private spans across its cut rather than retaining an unmatched private prefix. Preserve the final narrative character cap.
3. Exercise capture through the memory host and Store retrieval/search so private payloads cannot reappear in recall. Cover other supported secret patterns crossing a cut, without real credentials in fixtures.
4. Run focused privacy, observation/capture and memory recall tests and `just check`.

Check: a complete private block remains private regardless of its position or length. The probe compiled the verbatim production synthesizer, imported the actual scrubber, and used real Store migrations/public methods; the full-block scrub, short-block and Unicode-cap controls passed. The full hook/capture integration was source-traced, not executed.

### T619. Scroll wrapped TUI panel results by rendered rows

Confirmed on `2c7c40b2d8af0673f085d71d8d3e1da0051084e5` with the actual [Panel](src/tui/panel.rs#L144) and Ratatui's test backend. The shown-result key handler passes `lines.len()` to `scroll_key`, which caps the offset at the number of logical lines minus one. [render](src/tui/panel.rs#L199) enables paragraph wrapping and applies that offset in rendered rows. A one-line result that wraps beyond the viewport therefore has a maximum offset of zero: even 100 PageDown events leave the same first rows visible and its tail inaccessible. A directly scrolled Paragraph renders the tail, and the unwrapped multi-line control scrolls normally. Compare/export result and error text share this panel.

Execution plan:

1. Add a small-viewport regression with one long result line and a distinctive tail. Send Down/PageDown and assert the tail becomes visible; retain the ordinary multi-line control.
2. Calculate or maintain the scroll range from wrapped rows at the current content width and viewport height. Share that geometry between rendering and navigation rather than capping against the logical input line count.
3. Clamp the offset when resizing or replacing the result; cover Unicode, blank lines, narrow terminals, short results, Up/PageUp and the compare/export result paths.
4. Run focused TUI panel and caller tests, then `just check`.

Check: every rendered result row is reachable using the existing navigation keys. The isolated probe imported the production panel with small surrounding UI shims and used a real Ratatui buffer; it did not execute an interactive terminal session or graph backend.

### T620. Enforce LSP request deadlines during blocking transport reads

Confirmed on `2c7c40b2d8af0673f085d71d8d3e1da0051084e5` in [Session::request](src/plugins/graph/lsp.rs#L461). Its deadline is checked before `read_msg`, but that function performs blocking pipe reads without a deadline. A matching response returns immediately without checking the elapsed deadline. An isolated probe compiled the verbatim request/framing methods against a synthetic subprocess peer: with `ready()` set to 10 ms and the peer delaying its reply by 250 ms, the request still succeeded after 675 ms wall time. A peer that stays silent can leave the call blocked instead of reaching the documented fallback.

[with_session](src/plugins/graph/lsp.rs#L544) holds the shared session mutex through the call; [capability::ask](src/plugins/graph/capability.rs#L197) invokes the server closure synchronously. T376 adds fallback after a returned error, but that does not enforce a deadline on an unread pipe.

Execution plan:

1. Add deterministic fake-peer regressions for no reply, a delayed reply, a partial header/body and intervening notifications. Assert bounded completion and timeout classification, with a timely-response control.
2. Make framing reads deadline-aware using one transport owner and bounded cancellation. Include writes in the request budget so a peer that stops consuming input cannot bypass the same bound.
3. On timeout, invalidate or safely resynchronize the session, release locks and reach the existing fallback path. Verify a later request and shutdown cannot be trapped behind the expired request or consume its late response.
4. Run focused LSP transport, capability/fallback and shutdown tests, then `just check`.

Check: an unresponsive peer cannot hold an LSP request beyond its configured budget plus a bounded cleanup allowance. Executed evidence used extracted production request/framing code, a synthetic peer and a 10 ms timeout shim; real language servers, the full capability dispatcher and silent-peer indefinite waiting were not executed.

### T621. Refresh opened LSP documents after their files change

Confirmed on `2c7c40b2d8af0673f085d71d8d3e1da0051084e5` in [Session::did_open](src/plugins/graph/lsp.rs#L511). The first query reads a file and sends `textDocument/didOpen` with version 1. Subsequent calls return immediately if the URI is already in `opened`, before reading the file again. [with_session](src/plugins/graph/lsp.rs#L544) reuses the same live server for the workspace; the module sends no document-change or close notification. Editing a file between queries therefore leaves the peer with the original open-document text.

The isolated probe opened a temporary file containing `old_symbol`, changed it to `new_symbol`, and called the actual `did_open` implementation again. A synthetic protocol peer reported only one notification, still carrying `old_symbol`. This affects repeated queries in a resident MCP session; restarting a server happens to refresh the initial snapshot. T376's lifecycle/fallback work does not cover edits to an already opened document.

Execution plan:

1. Add a fake-peer test that opens, edits and re-queries one URI in the same session; assert the peer receives the new content before the query. Keep an unchanged-file control.
2. Track each opened document's content/version and send the appropriate full-content change or close/reopen sequence when it changes, respecting the negotiated synchronization capability. Do not mark a document as synchronized before its notification succeeds.
3. Cover consecutive edits, Unicode, deletion/read failure and session replacement; ensure query results and source ranges cannot silently refer to the old snapshot.
4. Run focused document synchronization and repeated graph-query tests, then `just check`.

Check: a second query after a saved edit uses the current document text without restarting the MCP process. Executed evidence compiled the production document-open/framing methods against a synthetic peer; actual language-server analysis and full MCP integration were not run.

### T622. Preserve non-text blocks when the pi extension compresses tool output

Confirmed on `2c7c40b2d8af0673f085d71d8d3e1da0051084e5` in [pi tool_result](plugins/pi/extensions/rtok.ts#L143). The handler joins only text blocks for `rtok filter`, then returns a replacement content array containing only the compressed text. A successful transformation of a text-plus-image result therefore drops the image; its bytes were never sent to the filter and cannot be recovered from that text's archive. The omp installer links the same package. T70.1 covers text replacement, not preservation of mixed content.

A Vitest probe imported the actual extension with a synthetic registrar and mocked child-process transport. A text/image fixture returned only compressed text; unchanged-output and subprocess-failure controls returned no replacement.

Execution plan:

1. Add a mixed-content regression for text, image and other host-supported blocks, alongside text-only, unchanged and failure controls.
2. Preserve every non-text block and its ordering when replacing text, or conservatively pass mixed content through until a lossless mapping is available. Do not claim that archiving only the joined text preserves the whole result.
3. Cover multiple text blocks separated by images, empty text, unknown block types and the shared omp package path.
4. Run extension/package parity tests and `just check`.

Check: successful compression cannot remove or alter non-text tool-result blocks. Evidence used the actual extension under Vitest; no real pi/omp process or model was invoked.

### T623. Correlate web replies across request timeouts and retries

Confirmed on `2c7c40b2d8af0673f085d71d8d3e1da0051084e5` in [askDoctor and settleDoctor](web/src/api/query.tsx#L180). Pending requests are matched by answer kind and optional project key. A timeout removes the old entry, but its eventual answer still matches the next request of that kind. The client then resolves the new request with the old payload and ignores the actual new answer. Sequential server processing does not prevent this when the first operation outlasts the client timer.

A Vitest fake-clock/transport probe exercised the verbatim `createApi` implementation: plan A timed out, plan B was sent, and the late A frame resolved B with A's selection/diff. A timely-response control passed. [DoctorFix](web/src/pages/DoctorFix.tsx#L29) protects component effect generations, but cannot detect a response already assigned to the wrong API promise. This is separate from T576's server-side preview/apply selection drift and the already documented snapshot/write acknowledgement behavior.

Execution plan:

1. Add deterministic timeout/retry tests for two requests with different selections; the late first reply must never resolve the second promise. Include same-project graph/diff and export/import variants sharing this queue.
2. Add request identifiers to the client/server protocol and echo them in successful and failed answers. Match pending promises by identifier and operation; update generated types and protocol tests together.
3. Ignore replies for expired requests, isolate reconnect generations and retain bounded pending-state cleanup. Verify timely requests, concurrent requests and connection closure.
4. Run focused API/protocol and Doctor UI tests, then `just check`.

Check: every resolved promise receives only its own request's answer, including after a timeout. Executed evidence used extracted production API code with a fake query cache, clock and connection; no full browser/server integration was run.

### T624. Retain the trace ID in the OTel integration-check extractor

Confirmed on `2c7c40b2d8af0673f085d71d8d3e1da0051084e5` in [otel-check.sh](tools/otel-check.sh#L72). After `rg` matches a complete `"traceID":"<32 hex digits>"` field, `sed 's/.*"//;s/"$//'` greedily removes everything through the final quote. Even a valid trace therefore becomes an empty ID and the following check fails before trace validation. Running the exact pipeline on `0123456789abcdef0123456789abcdef` produced an empty result; the malformed-ID control was also empty. T53.4 introduced the check but does not cover this extraction defect.

Execution plan:

1. Add an offline fixture test proving that a valid Jaeger response yields the exact trace ID and a missing/invalid ID fails clearly.
2. Parse the response structurally using an existing available JSON parser, and explicitly select a valid trace ID rather than depending on quote stripping or compact JSON formatting.
3. Cover compact/pretty JSON, multiple traces, empty data and malformed responses; keep these checks independent of Docker.
4. Run the extractor tests and `just check`; run the existing Docker-backed OTel check when that environment is available.

Check: a valid returned trace can reach the existing trace-validation stage. Only the exact extraction pipeline was executed with synthetic JSON; the Docker/collector/Jaeger integration was not run.

### T625. Preserve invalid Grok configuration when installing MCP

Confirmed on `2c7c40b2d8af0673f085d71d8d3e1da0051084e5` in [Grok load](src/agents/grok/mod.rs#L295): a TOML parse error becomes a default empty document. When Claude MCP does not already cover Grok, [register_mcp](src/agents/grok/mod.rs#L209) inserts its server into that empty document and writes it over the existing configuration. An unrelated syntax error therefore discards the user's provider/settings text instead of stopping the edit.

An isolated probe compiled the verbatim registration/loader functions with a temporary config path and the real agent SDK writer. A fixture containing a user provider followed by an unfinished array was replaced by an MCP-only document. The original bytes survived in the enabled backup; a valid-provider control retained its content. Configuration and Claude-coverage dependencies were shimmed, and no real host was started.

Execution plan:

1. Add a malformed-existing-TOML regression through the Grok registration path. Require an error and byte-for-byte preservation of the original file; retain missing-file and valid-file controls.
2. Make loading distinguish a missing config from read/UTF-8/parse errors, propagating errors with the path instead of substituting an empty document. Reuse the existing strict TOML loader where its contract fits.
3. Verify install/remove/compatibility checks cannot turn a read failure into an empty writable config; test backup-disabled operation, permission failures where supported and idempotent valid installs.
4. Run focused Grok and host-config preservation tests, then `just check`.

Check: an invalid existing Grok file is never replaced by generated configuration. The executed probe covered parsing, registration and the real SDK filesystem write with backups; real CLI dispatch and OS permission failures were not exercised.

### T626. Match Aider proxy ownership by URL authority

Confirmed on `2c7c40b2d8af0673f085d71d8d3e1da0051084e5` in [is_ours_value](src/agents/aider/mod.rs#L149). Ownership accepts `localhost` or `127.0.0.1` anywhere in the value, plus the configured port anywhere with a non-digit boundary. [strip_ours](src/agents/aider/mod.rs#L213) consequently deletes an unrelated `openai-api-base: https://api.example/localhost:8790/v1` when the rtok port is 8790, even though the URL authority is `api.example`. The same predicate reports that endpoint as rtok-installed.

An isolated Rust probe compiled the actual parser, predicate and remover: the foreign endpoint disappeared while an unrelated `model` line remained. A conventional foreign endpoint was kept and rtok's exact loopback endpoint was removed. T559 concerns quoted YAML keys, not URL ownership; the existing foreign-base-URL test does not cover loopback text outside the authority.

Execution plan:

1. Add ownership/removal regressions for loopback strings and port-like text in paths, queries, userinfo and foreign hostnames. Assert foreign configuration remains byte-for-byte unchanged.
2. Parse the endpoint and compare its actual scheme, host and port against the supported rtok ownership contract. Preserve exact configured URL recognition and intentional loopback aliases; do not infer ownership from arbitrary substrings.
3. Use the same predicate for installed-state detection and removal. Cover IPv6 loopback if supported, malformed URLs, port-prefix collisions and quoted YAML values without duplicating T559's parser work.
4. Run focused Aider detection/removal tests and `just check`.

Check: a foreign URL containing loopback-looking text outside its authority is neither reported as rtok's proxy nor removed. Executed evidence used extracted production parsing/removal code; full CLI file edits and real Aider were not run.

### T627. Preserve JSONC separators when removing an item with a trailing comment

Confirmed on `ea999781d7a48733e3a5c92e711cfad291c13536` using the verbatim production JSONC editor. [excise_member](src/agents/jsonc.rs#L241) skips whitespace but not comments when looking for the following comma. Removing `rtok` from `{"servers":{"rtok":1 /*keep*/,"other":2}}` returns `{"servers":{ /*keep*/,"other":2}}`, which no longer parses. The array equivalent, removing `"rtok worktree adopt" /*keep*/,` before another command, also leaves a leading comma. [Cursor post-create removal](src/agents/post_create.rs#L129) passes that result to `save` without validating it again; the ordinary MCP `remove_entry` wrapper instead rejects it before writing. T328 fixed replacement token boundaries, and T331.5 introduced shared removal coverage; neither records this comment-before-separator counterexample.

Execution plan:

1. Add object and array regressions with first/middle/last owned elements, block and line comments, trailing commas and multiple removals. Assert valid output, preserved foreign values and comments, and byte stability outside the removed span.
2. Make separator selection aware of JSONC trivia through the existing parser/scanner; share the correction between object and array removals.
3. Validate the final edited document before post-create settings can be written, preserving original bytes on errors. Exercise Cursor and the Devin/Windsurf caller with temporary files.
4. Run JSONC, Doctor removal and post-create preservation tests, then `just check`.

Check: removal leaves a parseable document containing the unrelated entry/command. Executed evidence compiled the actual editor and passed no-comment and last-element controls; the post-create filesystem write was source-traced, not executed against a real host.

### T628. Select cached plugin versions by version precedence

Confirmed on `ea999781d7a48733e3a5c92e711cfad291c13536`: [newest_child_dir](src/agents/plugin_install.rs#L225) sorts directory paths lexicographically and pops the last. An isolated fixture containing `0.9.0` and `0.10.0` selects `0.9.0`. [cache_probe](src/agents/plugin_install.rs#L190) uses this for Codex and CodeBuddy installed-version detection, so the outdated/list paths can report an older cached copy even when the newer copy exists. T328 corrected the separate `ketch_store_plugin` resolver; this still-unfixed cache selector does not use that ordering.

Execution plan:

1. Add installed-version probes containing `0.9.0`, `0.10.0`, prereleases, build metadata, malformed names and unrelated files; assert which manifest/version record supplies the result.
2. Prefer an authoritative active-install record wherever the host supplies one. For the documented newest-cache fallback, apply SemVer precedence and an explicit deterministic fallback for unparseable names.
3. Reuse the existing version parsing/order policy rather than adding another independent comparator. Verify Codex and CodeBuddy list/outdated behavior and keep receipt precedence intact.
4. Run plugin-install/version/outdated tests and `just check`.

Check: a newer semantic version is not hidden by the lexical order of cache folder names. Executed evidence used the exact selector over real temporary directories; host activation and update commands were not run.

### T629. Reopen previously closed hosts when a later preflight fails

Confirmed on `ea999781d7a48733e3a5c92e711cfad291c13536` in [with_restart](src/agents/restart.rs#L289). The loop closes each running desktop immediately after its `would_change` check. If a later host's check returns an error, `?` exits before the reopen loop. A probe compiling this exact orchestration with fake processes recorded only `quit:First` when the second host's preflight failed; the write callback was never reached. A write-error control recorded both quits and both reopens. T141/T434 cover ordinary restart and write/quit failures, not a later preflight error after partial shutdown.

Execution plan:

1. Add an orchestration regression with two running desktop hosts and an error in the second preflight. Assert no host remains closed and no write occurs.
2. Complete fallible preflight checks before shutting down any app, or use guaranteed cleanup for every successfully closed app on every exit path. Preserve existing warnings and best-effort reopen semantics.
3. Cover failures on first/middle/last hosts, dry run, no-restart, CLI-only hosts and write errors without launching real applications.
4. Run restart/install orchestration tests and `just check`.

Check: every successfully closed app is reopened even if a subsequent host cannot be planned. Executed evidence used the real orchestration function with injected process/config seams; OS process control was not exercised.

### T630. Preserve the selected config file in operator pages

Confirmed on `ea999781d7a48733e3a5c92e711cfad291c13536` with the actual `rtok-config` crate. [page](src/model/tasks.rs#L156) reloads layers using `(&home, None, None)` and [configured](src/model/tasks.rs#L184) repeats that selection, dropping `cfg.loaded_from`. A custom file supplied through `--config` with task prefix `Z` loads correctly, but the verbatim `configured` helper returns false when the default file has no tasks section. If the default file instead specifies prefix `A`, the page's reload selects `A`; a control passing `loaded_from` retains `Z`. The board can consequently report an unconfigured project or use different task settings from the CLI launched with the same explicit config. The Config page's [config_page_text](src/model/mod.rs#L1019) calls the same `figment(&cfg.home, None, None)`, so it also displays values and provenance from the wrong file (source-traced). T441.13.7's refresh behavior does not cover this source-selection case.

Execution plan:

1. Add task-page refresh regressions for an explicit config file with no default file and with a conflicting default task configuration. Compare the board's settings and rows, and the Config page's values/provenance, with the CLI using that same config.
2. Carry the original selected config path through both refresh and provenance checks, reusing `Config::loaded_from`. Preserve reloading of project-level `.rtok.toml` changes.
3. Cover default-file, `RTOK_CONFIG`, explicit `--config`, missing/invalid files and fallback behavior without process-global environment mutation in tests.
4. Run config-layer/task-page tests and `just check`.

Check: task-page refresh and Config page rendering use the same selected user file as the process that created them. Executed evidence used real config extraction and the exact provenance predicate; background polling and browser rendering were source-traced, not run.

### T631. Track the real Git metadata when embedding a worktree build SHA

Confirmed on `ea999781d7a48733e3a5c92e711cfad291c13536` using the actual [build script](build.rs#L39) in a minimal temporary Cargo package and a real linked Git worktree. `.git` is a file there, so none of `.git/HEAD`, `.git/refs/heads`, `.git/packed-refs` exists and no Git rerun hint is emitted. After an initial build and an empty commit advancing HEAD, a second `cargo run` still prints the old embedded SHA. The fixture reported new HEAD `86a42649a` while the binary retained `81b95850e`. Other rerun directives keep Cargo from using its broad default source scan. T10.4's version embedding and T310's SPA hints do not cover linked-worktree Git metadata.

Execution plan:

1. Add an isolated incremental-build regression with a linked worktree: build, advance HEAD without touching sources, build again and compare the embedded SHA with Git.
2. Resolve the actual per-worktree Git directory and common directory through Git, and watch HEAD plus the current loose ref/packed refs as appropriate. Support detached HEAD and ordinary checkouts.
3. Keep source archives without Git working, avoid always-dirty missing-path hints, and cover checkout/ref changes as well as ordinary commits.
4. Run build/version tests and `just check`.

Check: an incremental worktree build embeds its current HEAD rather than a stale commit. The reproduction ran Cargo and Git with the production build.rs; the full rtok binary and release packaging were not rebuilt.

### T632. Use rendered chart height when paginating PDF reports

Source-confirmed on `ea999781d7a48733e3a5c92e711cfad291c13536` in [pagination](src/report/pdf.rs#L77) and [bar rendering](src/report/pdf.rs#L243). A chart is budgeted as `pairs.len() + 2` lines at `LEAD = 4.6` mm, but each drawn bar advances 5.2 mm. The chunker accepts 56 bars as a 58-line page. Starting at TOP 282 mm, after the chart title the last bar baseline is `282 - 6 - 55 * 5.2 = -10` mm, below the page itself and well below the 15 mm bottom margin. The existing oversized-chart guard and T167's orphan-heading fix do not account for the different physical bar spacing.

Execution plan:

1. Add layout regressions at the maximum chart size, with and without a preceding heading/paragraph. Inspect the emitted positions and require every row to stay within the content area.
2. Express block costs and splitting in the same physical units used for rendering, including chart title and trailing spacing. Derive the maximum rows per chart from available height.
3. Preserve chart continuation, heading placement and progress guarantees for oversized and empty charts; check mixed charts and paragraphs.
4. Render representative reports for visual inspection, run PDF/report parity tests and `just check`.

Check: no chart row crosses the bottom margin. The executed probe checked the current source constants and arithmetic; an actual PDF renderer/viewer was not run in this audit.

### T633. Include the contents page in printed PDF page numbers

Source-confirmed on `ea999781d7a48733e3a5c92e711cfad291c13536`: [render](src/report/pdf.rs#L52) prepends one contents page, and section bookmarks use `start + 2`, but [toc_page](src/report/pdf.rs#L171) prints `start + 1`. A section starting on content index zero is physically on page 2 while the contents says page 1; subsequent section numbers have the same offset. This is separate from T632's chart-height calculation.

Execution plan:

1. Add a PDF contents regression with sections starting on different content pages, checking printed page numbers against their physical pages.
2. Share the conversion from zero-based content index to physical page number between the contents and bookmark construction, honoring the PDF library's index convention.
3. Verify first, middle and final sections plus chart continuations with extracted text/bookmarks and a rendered sample.
4. Run PDF/report tests and `just check`.

Check: every contents entry points to the page that actually starts the section. The source-arithmetic probe confirmed the one-page discrepancy; viewer navigation was not executed.

### T634. Keep PyPI dry-run behavior independent of flag order

Confirmed on `ea999781d7a48733e3a5c92e711cfad291c13536` by running the actual [publisher](tools/pypi-publish.sh#L18) with temporary artifacts and fake git/cargo/twine executables. Both options overwrite one `mode` variable: `--dry-run --testpypi` invokes `twine upload --repository-url https://test.pypi.org/legacy/`, whereas the reverse order and `--dry-run` alone only invoke `twine check`. A dry run can therefore upload if the repository-selection flag follows it. No network upload occurred in the probe.

Execution plan:

1. Add a fake-twine command test for both flag orders, repeated flags and each standalone option. Require every invocation containing `--dry-run` to perform no upload.
2. Store repository selection and dry-run state independently, or reject the combination explicitly before any publishing command. Keep the target repository visible in dry-run output.
3. Preserve version/artifact validation and production wheel completeness checks for the chosen target; verify validation failures never upload.
4. Run publisher script checks and `just check`.

Check: flag order cannot turn a requested dry run into a publishing operation. The actual shell script ran with synthetic distribution files and a logging-only twine stub; package validity and registry authentication were not exercised.

### T635. Decode JSON paths in Claude worktree fallback hooks

Confirmed on `ea999781d7a48733e3a5c92e711cfad291c13536` in [field](plugins/claude/scripts/worktree.sh#L26). The sed expression copies the JSON string's encoded bytes and stops at an escaped quote. Given a JSON-encoded cwd `/tmp/quote"dir`, it returns `/tmp/quote\`; a cwd containing one backslash retains the JSON escape pair. [WorktreeCreate](plugins/claude/scripts/worktree.sh#L49) passes that wrong path to `git -C` in its fallback. Removal's directory check can silently skip the actual worktree for the same reason. D31/T159 require a functioning fallback, and absolute paths are not restricted to characters that avoid JSON escaping.

Execution plan:

1. Add script regressions with plain, spaced, quoted, backslash-containing and Unicode paths, using a missing/old rtok fallback and fake Git argv capture.
2. Decode the hook payload with a real JSON parser available under the supported fallback contract; do not replace one partial unescaper with another. Explicitly handle unavailable parsing and malformed fields.
3. Test create cwd/name and remove cwd/worktree_path independently, preserving slug sanitation, dirty-worktree refusal and stdout's single-path contract.
4. Run hook-script/fallback tests and `just check` on supported shell platforms.

Check: Git receives the decoded path exactly and removal does not silently skip an escaped path. Executed evidence ran the verbatim `field` function through POSIX sh with synthetic JSON; real Git worktree creation/removal and Windows were not run.

### T636. Limit Claude legacy proxy migration to owned endpoints

Confirmed on `ea999781d7a48733e3a5c92e711cfad291c13536` using the actual [retarget_proxy](src/agents/claude/migrate.rs#L94). `agents install claude --replace --yes` rewrites any `ANTHROPIC_BASE_URL` containing `8788` or `8787`, wherever those digits occur. The foreign URL `https://api.example/v1/8788` becomes `https://api.example/v1/8790`. The migration also hardcodes its destination instead of receiving the configured endpoint. T10.9's normal removal contract is related, but this is the separate legacy migration entry point; T626 affects Aider's ownership predicate.

Execution plan:

1. Add migration fixtures for genuine legacy loopback endpoints and foreign URLs with matching digits in hosts, paths, queries and userinfo. Assert foreign values are unchanged.
2. Parse and verify the legacy endpoint's actual authority before retargeting, using the established ownership policy and supported loopback aliases.
3. Pass the effective configured proxy destination into migration, preserving endpoint structure deliberately rather than globally replacing digit strings. Cover nondefault ports and idempotence.
4. Run Claude migration and host-config preservation tests and `just check`.

Check: migration retargets only recognized legacy rtok-related endpoints to the effective destination. The extracted production helper reproduced foreign-path mutation and passed an ordinary foreign-URL control; full installer dispatch and live Claude were not run.

### T637. Route filter text before global TUI shortcuts

Confirmed on `ea999781d7a48733e3a5c92e711cfad291c13536` in [App::key](src/tui/app.rs#L715). The global `r`/`?` branches run before the Config and Calls filter handlers. An isolated compiled probe using the verbatim routing prefix and [config_key](src/tui/app.rs#L483) typed `proxy?` into an active Config filter and obtained `poxy`, one refresh and an opened help overlay. Calling the filter handler directly preserves `proxy?`. The Calls filter is reached after the same global branches (source-traced). T60.8 establishes global help/refresh outside text entry; its tests do not exercise active filters. The existing compare/export typing exceptions already demonstrate the needed precedence. This is separate from T619's wrapped-row scrolling.

Execution plan:

1. Add App-level regressions typing `proxy?` and ordinary text into Config and Calls filters; assert exact text and no refresh/help side effects.
2. Dispatch active text-entry handlers before ordinary global shortcuts, keeping Ctrl+C and the documented finish/cancel keys deliberate. Reuse the compare/export routing convention.
3. Keep `r` and `?` functional outside text entry and cover closing a filter, reopening it, Backspace and switching pages.
4. Run TUI state/rendering tests and `just check`.

Check: every printable filter character reaches its input without firing an unrelated command. The executed probe compiled the actual routing prefix and Config handler with unrelated page/model methods stubbed; a full interactive terminal and Calls input were not run.

### T638. Bound benchmark output draining after the child exits

Confirmed on `ea999781d7a48733e3a5c92e711cfad291c13536` in [run_bounded](src/bench.rs#L402). The deadline controls only child polling; the subsequent `reader.join()` waits without a limit for stdout EOF. With the actual extracted function, a synthetic `sh -c 'sleep 0.8 & printf parent; exit 0'` and a 100 ms limit took 816 ms, waiting for the descendant after the direct child had exited. The ordinary `printf control` case returned its complete output. A longer-lived inherited pipe can stall the benchmark indefinitely. T235.1 and the host-version follow-up fixed command capture through `src/proc.rs`; this separate benchmark helper still implements its own unbounded drain. T620 concerns LSP request framing, not this process reader.

Execution plan:

1. Add hermetic subprocess regressions for an exited child whose descendant holds stdout, a child exceeding its deadline, nonzero exit and ordinary complete output. Never launch a real agent.
2. Reuse the shared bounded capture mechanism in `src/proc.rs`, preserving the benchmark's success/timeout result semantics and imposing a finite post-exit drain.
3. Reap the direct child on errors and timeout, and check both ordinary and graph benchmark callers. Keep capture output bounded where the shared API supports it.
4. Run benchmark/process tests and `just check`.

Check: the benchmark finishes within its deadline plus a documented finite drain allowance even when descendants retain stdout. The reproduction compiled the actual helper and ran synthetic shell children on macOS; no live benchmark agent or Windows process path was run.

### T639. Keep the selected Calls row visible in the TUI

Confirmed on `ea999781d7a48733e3a5c92e711cfad291c13536` in [render_calls](src/tui/view.rs#L609) and [calls_table](src/tui/view.rs#L654). The cursor advances through all snapshot rows, but the renderer uses a stateless `Table` with no offset; selection only changes a row's style. A Ratatui 0.30.2 TestBackend probe of the actual `calls_table`, with 15 rows in a 100-by-5 area, renders row 0 and highlights it when selected. Selecting row 10 still renders row 0, hides row 10 and leaves no highlighted row on screen. T15.5 added the Calls cursor and details; T60.10 fixed viewport tracking for Sessions, not Calls. T619 is the independent wrapped-result panel issue.

Execution plan:

1. Add a Calls rendering regression navigating beyond a short viewport and back, asserting that the selected row remains visible and highlighted.
2. Use Ratatui table state or the existing derived-offset convention to scroll the Calls list to its cursor, accounting for its header and border.
3. Cover terminal resize, snapshot shrink, empty rows and opening the detail/expand panes, which reduce the list height. Keep selection and details on the same call.
4. Run TUI state/rendering tests and `just check`.

Check: keyboard navigation never leaves the Calls cursor outside its visible list. The executed probe used the production table function and real TestBackend; row data and formatting/theme dependencies were small fixtures, and full App navigation was source-traced.

### T599. Make stored-body cleaning stable when noise removal reveals a wrapper

Confirmed with the actual [text](crates/rtok-store/src/sanitize.rs#L70) and [body](crates/rtok-store/src/sanitize.rs#L34) functions: put ANSI reset `ESC [ 0 m` or U+200B between `system-` and `reminder` in an opening tag, followed by `hidden</system-reminder>\nvisible`. The first pass removes the noise but leaves `<system-reminder>hidden</system-reminder>\nvisible`; the second returns only `visible`. JSON string values show the same change on a second body pass. `text` removes wrappers before character cleanup, so cleanup can create a wrapper after the removal stage has finished. This violates T431's explicit idempotence requirement and the [property test's invariant](crates/rtok-store/src/sanitize.rs#L290). No existing card covers this composition case; T432's upstream cleaner intentionally retains wrappers and is a separate contract.

Execution plan:

1. Add deterministic text and JSON regressions for both reproduced tag/noise combinations, asserting one-pass normalization and `clean(clean(x)) == clean(x)`. Retain plain-wrapper, unchanged-byte and invalid-UTF-8 controls.
2. Order or combine character and wrapper processing so the result reaches the documented canonical form in one call. Account for transformations that expose new syntax; avoid an unbounded repeated-regex loop over large inputs.
3. Extend property inputs to deliberately combine noise inside wrapper names and adjacent/nested wrapper boundaries instead of relying on accidental random coverage. Preserve non-wrapper text and keep `terminal_noise`'s distinct whitespace/wrapper behavior.
4. Add storage-path coverage for ordinary and split hook bodies with `store_raw` on/off. Ensure persisted bytes, sizes and hashes refer to the final cleaned representation, then run sanitizer/store tests and `just check`.

Check: repeated stored-body sanitation no longer changes the reproduced inputs, raw storage stays byte-for-byte and the terminal-only cleaner keeps wrappers. The offline probe imported the actual sanitizer module without modification; no database, production replay, host record or upstream request was exercised. This is a P3 normalization-contract defect, not evidence of a privacy leak or observed production data loss.

### T597. Reject stale demon process identities before stopping services

Confirmed in [stop](src/demon.rs#L208): a saved state file is enough to send TERM or KILL to its supervisor and child PIDs; the timestamp is never checked. A state left over from an earlier boot can therefore cause `demon stop`, `demon kill`, or the stop phase of `demon restart` to terminate unrelated processes after those positive PIDs are reused. [start](src/demon.rs#L154) and [quiesce](src/demon.rs#L263) already guard stale retirement with the boot time, but stop bypasses that protection. [process_term/process_kill](crates/rtok-sys/src/lib.rs#L48) validate positive PIDs, not process identity. T73 covers start/quiesce around upgrade; no active card covers this stop-path gap.

Execution plan:

1. Add deterministic tests with pre-boot state and reused positive PID identities for normal stop, forced kill and restart. Use a recording process adapter and assert no signal reaches unrelated processes; retain missing-state and valid-live-service controls.
2. Apply one shared stale-state/identity check before any initial or escalation signal. Reject pre-boot records and handle unknown boot information conservatively; do not let force mode bypass ownership checks. For current-boot PID reuse, verify a process birth token or equivalent platform identity rather than treating liveness alone as ownership.
3. Retire stale metadata with an explicit result while preserving any concurrently published new supervisor state. Keep stop-marker ordering and the valid-service shutdown/escalation behavior; revalidate identity before escalation to avoid signalling a newly reused PID.
4. Cover restart and upgrade callers, unavailable identity information, malformed state, nonpositive PIDs and concurrent service replacement using fakes. Rerun demon/platform tests and `just check`.

Check: stale state never authorizes a signal to a different process, including forced shutdown; valid owned services still stop and restart. The scratch probe compiled actual Service/State, file/read, targets and stop functions with a recording-only process adapter. State `since=1` sent TERM to both simulated reused PIDs in normal mode and KILL in force mode, then removed the state; the missing-state control sent nothing. No OS process was signalled, and real kernel PID reuse or full CLI integration was not exercised.

### T594. Validate structured config types before persisting edits

Confirmed through actual config APIs: `set_with(..., "stats.prices", "42", false)` writes the scalar and `issues` returns no errors, but Figment extraction into `Config` rejects it as an expected map. Likewise `plugins.read.allow_paths = [1]` is written and passes `issues`, then typed extraction rejects element 0 as not a path string. [check_table](crates/rtok-config/src/validate.rs#L545) skips open-map keys entirely through `is_open`, while [check_leaf](crates/rtok-config/src/validate.rs#L663) checks only that an array exists, not its element types. [set_all_with](crates/rtok-config/src/validate.rs#L100) uses those checks as its sole pre-write gate. An accepted edit can therefore leave the user file unloadable, contrary to the setter's contract. These are one incomplete structured-type validation issue; T363–365 cover other constraints, not these shapes.

Execution plan:

1. Add rejection tests for scalar open maps, wrongly typed map entries, and array elements, including the two reproduced commands. Assert both real and dry-run setters reject them and preserve an existing file byte-for-byte.
2. Validate the candidate document against the actual Config field types before persistence, using a pure default-plus-candidate extraction so project/environment overrides cannot mask invalid file values. Retain semantic/range checks and useful key/line diagnostics; do not add a second independently maintained type schema.
3. Keep free-form map names accepted while checking their container and value types. Cover valid custom model prices, benchmark paths, graph mappings, empty arrays and heterogeneous-invalid arrays; apply consistent checks to file validation.
4. Add CLI coverage proving a rejected set leaves subsequent loading functional and valid edits still preserve comments. Rerun config/layer tests and `just check`.

Check: every successful set produces a document accepted by typed config extraction, invalid structured values never replace the existing file, and free-form names remain supported. The scratch probe used the actual config crate's setter/validator plus Figment default-and-file extraction; full CLI and environment layering were not executed.

### T595. Keep commented-default discovery outside TOML string payloads

Confirmed with a valid multiline literal string: `[otel] service_name` contains the lines `[proxy]` and `# port = 12`, followed by the real `[proxy]` table with a commented port default. Setting `proxy.port` to 9000 updates the requested key but also removes `# ` from the unrelated service name's payload. [reveal_commented_key](crates/rtok-config/src/validate.rs#L185) calls [reveal_assignment](crates/rtok-config/src/validate.rs#L198), whose line scan treats the payload's table/header and comment as document syntax before the actual TOML edit. The output remains valid, so validation does not detect the unintended value change. Dry-run leaves disk untouched but previews the same proposed mutation. T381's default/comment preservation tests do not cover multiline string payloads.

Execution plan:

1. Add the multiline literal fixture and an equivalent multiline basic string, with header-shaped and assignment-shaped content. Assert that setting a different key changes only that key and preserves the parsed string value and its original bytes.
2. Use TOML-aware source spans/token context for commented-default discovery, or restrict revival to comments proven to be outside string values. Share that context with live-key detection; avoid interpreting raw payload lines as table headers or assignments.
3. Cover quoted/dotted table keys, inline comments, escaped quotes, CRLF, existing live keys and batch setters. Preserve the intended uncomment-in-place behavior and trailing comment/padding guarantees.
4. Verify real and dry-run edits propose the same safe result, with dry-run writing nothing; rerun existing comment-preservation tests and `just check`.

Check: arbitrary valid TOML string content cannot be modified by setting an unrelated key. The actual setter changed the fixture service name from `[proxy]\n# port = 12\n` to `[proxy]\nport = 12\n`; the requested port and ordinary-default control both became 9000. Only scratch configuration files were used.

### T592. Keep late same-second sessions from moving behind the OTel cursor

Confirmed using the actual Store and [advance_sessions](src/otel/export.rs#L315): create sessions `z-first` and `a-late`, end and export `z-first` at timestamp 1700000000, then end `a-late` at that same timestamp. The next pending batch contains `z-first` again instead of `a-late`; after advancing, the following batch is empty, so `a-late` is never exported. Reversing the lexical ID order exports the late session correctly. [SessionsPending](crates/rtok-store/src/sql_ext.rs#L18) recomputes `ROW_NUMBER` ordered by session ID, while [sessions_pending_export](crates/rtok-store/src/otel.rs#L262) resumes using a count at the watermark second. A late lower ID shifts the already-exported row after the count and hides the new row before it. The flush lock serializes exporters, not session writers.

This is an uncovered case of completed T36.12's same-second delivery guarantee, not another active copy of that task. Its [integration test](tests/otel.rs#L178) uses `s1` then `s2` and checks counts, so the row-identity failure is missed. No claim is made that the backend displays duplicates: the repeated span has a stable ID, but the late session's root span is absent altogether.

Execution plan:

1. Add store and exporter regressions with both sessions already present, a later completion at the watermark second, and lexical ID orders in both directions. Inspect emitted session/span IDs, not just batch lengths or POST counts; the late session must be delivered and the next flush must drain.
2. Replace the mutable rank/count cursor with durable delivery state that cannot move behind already-exported rows. Prefer a monotonic completion/outbox sequence or explicit acknowledged session identities; a timestamp plus lexical last-ID cursor alone still skips late lower IDs. Keep database access in Store and use Diesel under the existing SQL policy.
3. Advance the new delivery state only after successful export and update related cursor state atomically. Define migration from existing timestamp/tail marks without assuming that a count identifies which tied sessions were delivered; allow deliberate replay using stable span IDs where needed to avoid loss.
4. Cover more than one BATCH of equal timestamps, session writes during a flush, HTTP failure/retry and restart with persisted marks. Retain stream isolation and bounded batches, and rerun the existing T36.12/T16 exporter tests.

Check: every newly ended session is eventually selected regardless of lexical ID order at an already-covered timestamp, acknowledged deliveries survive restart, and `just check` passes. The offline scratch executable linked the actual `rtok-store` crate, exercised an in-memory database and compiled the actual cursor-advance function; it used no SQL stubs, real host data or collector. HTTP/CLI integration was source-traced, not executed.

### T589. Track log generations without using their first line as identity

Confirmed with actual [Follow::poll](src/log.rs#L399): matching first lines and sufficient line count make a rotated replacement look like the previous file. Prime a reader with `same\nold-shown\n`, append `old-unseen`, rotate to `.1`, then create `same\nnew-unseen\n`: polling returns nothing, losing the old tail and both replacement rows. A second manifestation occurs when the replacement exists but is empty: polling carries the old tail and resets `first` to None; after the replacement receives its first line, polling replays the whole rotated file, including previously shown rows. [State replacement](src/log.rs#L430) loses the generation boundary. T24.3 covers ordinary rotation, missing-path gaps and storms with distinct first lines, not either fixture. These are one identity/state defect, not two cards.

Execution plan:

1. Add deterministic follow tests for matching first lines across rotations and an empty or partial-line replacement between polls. Assert every complete line is returned once in chronological order, including the unread tail of the old file.
2. Track file generations with a reliable filesystem identity or retained file handle and consumed position, using the existing platform abstractions where possible. Do not infer identity from log text; retain the carry boundary after observing an empty replacement.
3. Cover consecutive rotations, retention exhaustion, missing/unreadable intervals, truncation and partial writes. Preserve the shared initial-screen snapshot contract and define any unavoidable loss when the old generation has already been deleted.
4. Exercise watch integration with a scratch rotating writer and rerun the existing rotation/first-screen tests on supported platforms.

Check: retained generations neither lose nor replay complete rows in either fixture, existing follow ordering remains intact, and `just check` passes. The offline probe compiled actual Follow and whole-line functions with local scratch files; full CLI/watch scheduling was source-traced, not executed.

### T590. Preserve filesystem semantics for parent segments after symlinks

Confirmed in [resolve_with](src/plugins/read/mod.rs#L205): the request is lexically normalized before filesystem canonicalization at lines 218–224. With `root/link -> root/a/nested`, both `root/a/target.txt` and `root/target.txt` present and containing different text, native lookup of `link/../target.txt` reads `root/a/target.txt`, but rtok resolves and reads `root/target.txt`. Both targets are inside the allowed root; this is a wrong-file result, not a demonstrated root escape. Read, search and tree share this resolver. T56's Vfs/symlink-escape coverage and T430's Windows path identity do not cover parent traversal through an existing symlink.

Execution plan:

1. Add a fixture with distinct target contents and an in-root directory symlink followed by `..`; compare relative and absolute requests with native filesystem lookup. Keep a direct-path control.
2. Preserve symlink-sensitive traversal when resolving an existing request instead of collapsing parent components first. Apply the root guard to the actual resolved target and return a path naming that same target; handle missing paths explicitly without weakening confinement.
3. Cover allowed extra/dynamic roots, symlinks leaving a root, parent segments after those links, missing components and multiple links. Align the Vfs adapter with the host contract and retain Windows prefix/case coverage.
4. Add representative read/search/tree integration checks proving both selected content and root rejection, then rerun existing root-guard and path tests.

Check: existing paths identify the same file as native lookup, outside-root targets remain rejected, and `just check` passes. The Unix scratch probe compiled actual resolve/normalize functions with a standard filesystem adapter, equivalent Unix prefix checks and inactive dynamic roots; full MCP and Windows behavior were not executed.

**Continued source audit (T540).** T541–T546 were confirmed against source at
`efbb220b` (product code unchanged from `0b14b88e`). Fix the disk data-loss
case T541 first. T542/T543 concern index freshness; T544/T545 concern filtering
and should share eligibility logic; T546 concerns the derived rank document.
Completed T68.10/T35.5 and active T370 describe intended behavior, but do not
track these concrete failure cases. These cards record defects, not implemented fixes.

### T541. Reject disk task filename and frontmatter identity mismatches

Source-confirmed: [disk scan/find](src/tasks/disk.rs#L81) accepts a valid task id in the filename without comparing it to the parsed frontmatter id. Lookup selects by filename, but [place/write](src/tasks/disk.rs#L116) chooses the destination using the frontmatter id and title, then deletes the original file. Create a valid `R2 - target.md`, then a manually copied `R1 - copy.md` whose frontmatter says `id: R2`, `title: target` and has a different body. Setting R1 to in-progress overwrites R2's file with the copied body and removes R1. A malformed identity must be rejected before any mutation. T441.4 shipped the adapter; it does not validate this consistency boundary.

Execution plan:

1. Add a two-file fixture with the mismatch above; assert get/list/status reject the inconsistent task and preserve both files byte-for-byte.
2. Validate filename id against frontmatter id in the shared disk read path and return an error naming the offending path and ids before write/move operations.
3. Cover mismatches in active and done directories, status changes and claim/save; retain valid renames, moves and case-normalized ids.

Check: no operation on the malformed R1 overwrites R2 or removes either fixture; valid task lifecycle tests and `just check` pass. This reproduction is source-derived; an end-to-end regression remains part of the fix.

### T542. Invalidate graph rows when an extension's grammar mapping changes

Source-confirmed: [extractor_fingerprint](src/plugins/graph/index.rs#L417) hashes compiled queries and INDEX_VERSION but no configured extension map. [run_with](src/plugins/graph/index.rs#L111) therefore reuses old stats after changing `plugins.graph.extensions`; its stat gate skips unchanged files. Even touching the file reaches the [SHA gate](src/plugins/graph/index.rs#L346), which returns Same before consulting the new grammar. Index `sample.tpl` containing `fn old() {}` with `tpl = "rust"`, change only the mapping to `tpl = "py"`, then index again: the old Rust definition survives. T35.5 covers extractor upgrades; T68.10 added configurable mappings without including them in freshness.

Execution plan:

1. Add a fixture that changes the mapping after an initial index, with unchanged source bytes; exercise both unchanged and changed mtime.
2. Include a deterministic representation of the effective extension-to-grammar mapping in index freshness and invalidate the affected cached rows before either skip gate. Reuse the same policy for full and incremental entry points.
3. Verify equivalent map insertion orders do not rebuild, while mapping changes re-extract and remove obsolete symbols; retain warm-index skipping.

Check: the mapped file's symbols match the current grammar after reindex without a source edit; unchanged configuration still skips; `just check` passes. Source-derived reproduction, not an executed integration test.

### T543. Make stale-fingerprint dry runs predict the actual cold reindex

Source-confirmed: [run_with](src/plugins/graph/index.rs#L113) clears stale symbol rows only when `!dry_run`, then reads the existing stats unconditionally. With an old stored fingerprint and unchanged indexed files, dry-run takes the stat skip gate and reports no new rows; an actual run clears those stats and parses the same files. This violates the dry-run's would-be indexing report independently of T542's missing configuration input.

Execution plan:

1. Seed an indexed fixture and a deliberately obsolete fingerprint. Snapshot the store, run dry-run, then run the actual index on the same fixture.
2. When the fingerprint is stale, bypass old stat/SHA reuse for the simulated run without deleting rows or changing metadata.
3. Compare predicted indexed/inserted counts with the real cold run and assert the dry-run left symbols, fingerprint, timestamps and rank unchanged; retain warm dry-run behavior.

Check: stale-fingerprint dry-run predicts the real reindex and performs no writes; `just check` passes. Source-derived reproduction.

### T544. Keep unrelated gitignored files excluded when graph include is nonempty

Confirmed in source and an isolated Rust probe: [Matcher::new](src/plugins/graph/walk.rs#L27) adds the positive override `**` before any configured includes. Override whitelists take precedence over gitignore, so this revives every ignored file, not only the requested subtree. With `.gitignore` containing `wanted/` and `unwanted/`, and include containing only `wanted/`, the full walk returns both `wanted/a.rs` and `unwanted/a.rs`. This can index large ignored trees and pollute graph results. T68.10 intended selective inclusion, not disabling ignore rules globally.

Validation: the offline probe compiled this revision's actual `walk.rs` with the real `ignore` crate, plus minimal configuration/extension-support stubs. It asserted both paths were returned. No repository index or live watcher was run.

Execution plan:

1. Add a real-repository fixture with two ignored subtrees, one explicit include and ordinary nonignored source.
2. Replace the blanket whitelist with eligibility handling that preserves normal gitignore behavior while admitting explicit includes; share the override interpretation with T545.
3. Cover no includes, nested patterns and include/exclude precedence without restricting ordinary supported source to the include list.

Check: only the explicitly included ignored subtree returns, ordinary source remains eligible, unrelated ignored source stays out, and `just check` passes.

### T545. Apply full-walk include and glob-exclude rules to watcher events

Confirmed in source and an isolated Rust probe: [absorb_event](src/plugins/graph/watch.rs#L193) rejects gitignored paths before considering configured includes. Conversely, [Matcher::is_excluded](src/plugins/graph/walk.rs#L69) treats exclude patterns as literal prefixes, while the full walker interprets globs. Thus a file explicitly included from a gitignored subtree is indexed initially but its edits are ignored; a path matching `**/generated/**` is excluded from the full index but accepted as a watcher event and passed to incremental indexing. These are the event-path regressions of T68.10, distinct from T544's blanket full-walk whitelist.

Validation: the same offline probe compiled verbatim `gitignore`, `git_path`, `relevant` and `absorb_event` functions alongside actual `walk.rs`. It asserted rejection of the explicitly included file and acceptance of the glob-excluded file. This tests event filtering, not OS notification delivery or a database update.

Execution plan:

1. Add paired full-walk/event fixtures for an included ignored file and a glob-excluded supported file.
2. Reuse a consistent eligibility decision for walking, event intake and incremental indexing, with the same include/exclude precedence; preserve directory-rescan and .git exclusions.
3. Exercise edit and deletion events: included paths refresh/remove their symbols, excluded paths never enter the index, and irrelevant files do not reset the quiet window.

Check: full and incremental paths agree on eligibility for both fixtures, existing watcher debounce tests and `just check` pass.

### T546. Refresh stored file ranks after deletion or zero-symbol replacement

Source-confirmed: [run_changed](src/plugins/graph/index.rs#L238) removes deleted files' symbol rows but refreshes the derived rank document only when `report.inserted > 0` (line 290). Replacing the last definition with an empty file also reports zero inserted symbols: [replace_one](crates/rtok-store/src/symbols.rs#L234) deletes old symbols, writes a freshness sentinel and returns zero. [run_with](src/plugins/graph/index.rs#L192) has the same zero-symbol replacement gap: its later missing-file deletion count is zero because the path remains in the keep set. [rank::map/render](src/plugins/graph/rank.rs#L221) renders definitions from the stored document without checking current symbols or files. Index a sole `a.rs` defining `old`, then delete it via the incremental path or empty it and reindex: the map still lists `a.rs: old`. A later unchanged full scan does not repair the already-existing rank document. The symbols themselves are removed correctly; this is not T324's rejected stale-symbol candidate.

Execution plan:

1. Seed symbols and a rank document; separately test an incremental deletion and full/incremental replacement with zero extracted rows.
2. Track whether index contents changed independently of the number of inserted rows, and refresh or invalidate the derived document after successful removals/replacements.
3. Assert the rendered map and stored rank paths/definitions drop obsolete entries immediately; retain warm no-change behavior and the existing rank-rebuild failure fallback.

Check: the map never serves the removed definition after a successful update, subsequent warm indexing stays consistent, and `just check` passes. Source-derived reproduction; no full index runtime was executed during the audit.

### T131. Measure the spawn brief: cost row and on/off re-read share
Rule: a saving that is not a `Measurement` row does not exist, and the brief is a cost first. Needs T128 and T130.2.
Plan: T130.1's hook records a `Measurement` (`plugin: "memory"`, `kind: "brief"`) with the tokens it added (before = 0, after = brief) so the cost shows as negative saving; `rtok stats` `subagents` row splits the re-read share and sub-agent input tokens by "spawned with a brief" (the brief's archive id in the sub-agent's first user message) vs without.
Check: fixture with one briefed and one plain sub-agent asserts the split; after a dated window with the flag on, `research.md` §17 gets the measured net; default flips to on only if net tokens saved > 0 — otherwise the card closes with the number and T130 stays off.
Progress (2026-09-25): the cost row was already recorded by T130.1 (`memory`/`brief`, before 0, after = brief tokens); `tests/hook_spawn_brief.rs` now asserts one row per fired brief and none otherwise. `rtok stats` splits the `subagents` re-read share and input tokens into `brief` / `no brief`, detected by `measure::subagents::SPAWN_BRIEF_MARKER` in a sub-agent's first user message (a `handoff.rs` test pins it inside the brief's `INSTRUCTIONS`); fixture test `briefed_and_plain_subagents_split_the_reread_share`. Left: the dated window with the flag on, the net in `research.md` §17, and the default decision.
### T132. Ship a Haiku scout agent definition with the Claude Code plugin
`research.md` §17.3(4). Make the cheap path the default one: `plugins/claude/agents/rtok-scout.md` with `model: haiku`, `tools` limited to the rtok MCP `read`, `search`, `outline`, `explore`, `expand`, and a short system prompt — ranged reads only, never a whole file over the outline threshold, answer with `path:line` citations and no file dumps. Verify the plugin `agents/` directory format against the current Claude Code docs first and add the link to the `## Docs` list in `plugins/claude/README.md`.
Check: `rtok agents install claude` offers the agent file and removal takes it away (host matrix e2e); `tests/host_docs.rs` and `tests/agents_doc.rs` (`RTOK_BLESS=1`) green; T128's per-`agentType` split is the measurement — record `rtok-scout` vs `Explore`/`general-purpose` read bytes per sub-agent in `research.md` §17 after a dated window.
Progress (2026-09-24): `plugins/claude/agents/rtok-scout.md` ships with the plugin (`model: haiku`, the five tools under the plugin-scoped names `mcp__plugin_rtok_rtok__<tool>`); unit test on the frontmatter, install/remove e2e in `tests/claude_plugin.rs`. Left: the dated `rtok-scout` vs `Explore`/`general-purpose` read-bytes row in `research.md` §17 once a window of sessions has run with it.
### T134. Probe: does a CLI command hook's `PostToolUse` `updatedToolOutput` replace native tool output?
Gate for I-91 (`research.md` §17.2). The Agent SDK hooks page says `updatedToolOutput` "works for any tool"; rtok's standing rule says PostToolUse can only add context. If the CLI honours it, native Read/Bash output could be shrunk in place (pointer + `expand <id>`) instead of wrapped or denied — that changes the design of `cmd`, `read` and `guard`, so it is a creator decision, not a silent change. No product code in this task.
Plan: throwaway hook script (scratch, not committed) returning `hookSpecificOutput.updatedToolOutput` for `Read` and `Bash` on the current Claude Code; run one Read and one Bash; check what the model received in the transcript. Repeat for an MCP tool.
Check: a dated row in `research.md` §3 with the Claude Code version, the payload sent and what the transcript shows, per tool kind. Honoured → the `AGENTS.md` rule line and I-91 are put to the creator with the row; not honoured → I-91 closes with the date.
Progress (2026-09-25, `research.md` §3 "T134"): Claude Code 2.1.267; the CLI hooks page lists no `updatedToolOutput` for `PostToolUse` (only `additionalContext`, `systemMessage`, `terminalSequence`), the Agent SDK page does. The live run is blocked in agent sessions (`claude -p` fails with an expired OAuth session, as in T53.1); left: the creator runs the scratch probe (hook logging the payload and returning `updatedToolOutput`, one Bash / Read / MCP call on `--model haiku`) from a real terminal, and the per-tool verdict goes into the row.
### T124. Realized `tools_rewrite` saving as a dated `research.md` row
6.2 % (T59.5) is the ceiling, not a saving: no dated row shows what `[proxy.tools_rewrite]` removes with the default `max_description_tokens = 60`. Precondition, by the creator: turn it on for this machine's proxy for at least 20 sessions. Then sum the `kind = tools_rewrite` Measurement rows against session input for the same window (`rtok stats` / `rtok gain`, dated command in the row), and write one row into `research.md` §2 next to the T59.5 row; update `docs/comparison.md` only if it cites the number. If the realized share is under the 3 % gate, say so in the row and leave the default off.
Check: the row cites the command, date, sessions, before/after tokens and the share; no number in prose without it.

### T156. Probe: `WorktreeCreate`/`WorktreeRemove` hooks and reflink-seeded `target/`

No product code. Two open questions from `research.md` §18.3–18.4: (1) Claude Code's `WorktreeCreate`/`WorktreeRemove` hooks replace the default create/remove — can rtok own location, naming and the ownership record there, and what do the desktop app and sub-agent `isolation: worktree` actually send; (2) does seeding a new worktree's `target/` by reflink (`reflink-copy`, APFS `clonefile`) save build time and disk after a real task, or does cargo rewrite most of it anyway.

Plan: throwaway hook script (scratch, not committed) that logs the payloads for `claude --worktree`, a sub-agent worktree and the desktop app, and returns a path under `_worktrees/`. For (2): two fresh worktrees of this repo, one seeded with `cp -c -R target`, one cold; record wall time of `just check` and physical disk delta (`df`, not `du` — clones are double-counted) for each. Write the payloads, the numbers and the dated commands into `research.md` §18. `reflink-copy` is a new dependency: adopting it is a creator decision taken on those numbers, not part of this task.

Progress (2026-09-25, `research.md` §18.4 second data point): part (2) measured. Cold `just check` took 185 s and +6.28 GiB; seeded took 371 s and +3.35 GiB. The clone skipped every dependency rebuild (≈ 23 s saved), but T236's `dunnage` pass then compressed the cloned files (≈ 215 s). Parked as I-99, with no follow-up task. Lead (unmeasured, 2026-10-08): the installed `dunnage` 0.1.0 has its own `seed` and `worktree add` subcommands (`dunnage --help`), so seeding may not need `reflink-copy`. Part (1), the hook payloads from `claude --worktree`, a sub-agent worktree and the desktop app, is still open: it needs live sessions of the creator's.

Check: `research.md` §18 gains the hook payloads and a dated table (cold vs seeded: seconds, bytes); T159's card is corrected against the recorded payloads; seeding gets a follow-up task or an `ideas.md` entry from the numbers; no file under `src/` changes.

Execution (2026-09-27): (1) a probe kit in the session scratchpad (never committed), like T281, that logs `WorktreeCreate`/`WorktreeRemove` payloads and returns a `_worktrees/` path; the creator runs it with `claude --worktree`, a sub-agent `isolation: worktree` and the desktop app. The documented payload fields go into `research.md` §18.3 now, with sources. (2) Seeded (`cp -c -R target`) and cold worktrees of this repo: wall time of `just check` and physical disk delta (`df` before/after, not `du`). The cold run only happens with ≥ 30 GiB free; otherwise the row says so. Result: a dated row in `research.md` §18.4.

### T262.3. Codex: spawn brief on `SubagentStart`

`research.md` §23: Codex fires `SubagentStart` and adds the hook's stdout (or its hook-specific context) to the subagent as developer context. Add `SubagentStart` to `plugins/codex/hooks/hooks.json` and the Codex installer's list, and make `rtok hook SubagentStart` answer in the shape Codex reads.

Blocked (found 2026-09-24 while claiming): the brief is built from `PreToolUse` rows whose `tool_name` is `Read|Edit|Write` (`ledger()` in `src/plugins/memory/handoff.rs`), and rtok installs no `PreToolUse` hook for Codex (only `PreCompact`/`PostCompact`), so a Codex brief would always be empty. Needs Codex `PreToolUse` wiring first (idea I-88), which the creator has not approved.

Check: a Codex `SubagentStart` payload through `rtok hook` returns the brief in Codex's shape (test); `just check` green.

### T261. CI takes ~9.5 min on macOS; the webui check recompiles 183 crates every run

Creator request 2026-09-24: find what makes CI and the tests slow, try fixes in a draft PR, do not merge. Measured on #326:
- `check (macos-latest)` is the critical path. Of its ~9.5 min: mise 53 s, cache restore 65 s, fmt 3 s, clippy 46 s, nextest's test-profile build 1 min 47 s, 1733 tests 79 s, then about 39 s of `build-min`, jscpd, oxlint and pytest.
- `just webui-check` takes 2 min 6 s on macOS and 1 min 33 s on ubuntu. `crates/rtok-webui` is excluded from the workspace, so its `target/` is not in `rust-cache`, and 183 crates compile from scratch on every run.
- 78 `tests/*.rs` files mean 78 test binaries to link.

Plan (a draft PR; each change measured with `workflow_dispatch` runs on the branch, warm cache):
1. `rust-cache` also caches `crates/rtok-webui/target`.
2. A `lint` job on ubuntu takes fmt, clippy, `build-min`, jscpd, JS, Python and `webui-check`. The `check` matrix keeps its name and runs only the tests and examples, so macOS stops paying for lint. Trade-off to report: clippy no longer runs on macOS-only `cfg` code.
3. `CARGO_PROFILE_DEV_DEBUG=0` in CI only (smaller test binaries, less linking).
4. Estimate folding `tests/*.rs` into one integration binary (78 links become 1); report it, do not do it here.

Check: warm-cache `workflow_dispatch` runs on the draft PR are green, and a PR comment gives before/after timings per job.

Progress (Cursor / grok 4.7): items 1–3 already on `origin/main` via #332 — do not redo. Draft PR https://github.com/pyrlyn/rtok/pull/411 (`t261-ci-timings`). Two `workflow_dispatch` runs posted timings in a PR comment; both runs **failed** (Check not met — card stays open).

**Blocker (exact errors from the logs):**
1. `lint` — `just … dup` / jscpd: `ERROR: jscpd found too many duplicates (2.1%) over threshold (2.0%)` (`.jscpd.json` `threshold: 2`; 198 clones). Same on both runs. `webui-check` / `publish-dry` and lint’s rust-cache save were skipped because of this.
2. `windows` — `cargo nextest` exit 1. Failures (warm run [36202660499](https://github.com/pyrlyn/rtok/actions/runs/36202660499)): `agents::devin::tests::plugin_manifest_matches_the_installer`; `agents_install::{dry_run_setup_creates_nothing_and_copies_nothing, setup_twice_takes_one_backup_and_says_already_installed, remove_twice_says_no_changes_and_the_second_takes_no_backup, list_reports_installed_modules_per_host}` (5 failed; cold run [36201850046](https://github.com/pyrlyn/rtok/actions/runs/36201850046) also failed `plugins::checkpoint::tests::session_end_on_a_large_transcript_is_bounded`).

`check (ubuntu-latest)` and `check (macos-latest)` were green both times; cold→warm job wall: ubuntu 161 s → 146 s, macOS 362 s → 222 s (see PR #411 comment).

Fold estimate (not folding): **82** `tests/*.rs` bins. macOS log `Finished test profile`: cold **2 m 05 s**, warm **1 m 16 s**. No per-binary `Linking` lines in the CI log. Fold not in this PR (82-file mechanical move; own task).

### T275. Install/update removes rtok's MCP entry from an agent's config while a plugin serves MCP (every host)

Observed by the creator on 2026-09-27: rtok 0.10.0, Claude Code 2.1.267, `rtok@rtok` plugin installed. After `rtok agents update claude` the plugin (commit `12c7e91`) and its hooks are current, but rtok's MCP server is missing from Claude Desktop's MCP settings. It must be available in both Claude Desktop and Claude Code.

Evidence on that machine:
- `~/Library/Application Support/Claude/claude_desktop_config.json` has no `mcpServers` key. The backup written by the same run (`_backup/claude_desktop_config.json.bak-1790511161`, 15:12) still holds `mcpServers.rtok` = `/Users/<user>/.ketch/bin/rtok mcp`, so the update removed it.
- Claude Code gets rtok's MCP only from the plugin (`.mcp.json` runs `scripts/mcp.sh`, which runs `rtok mcp`). `~/.claude.json` and `~/.claude/.mcp.json` have no `mcpServers.rtok`.
- `rtok agents info claude` still prints `✓ mcp installed` for Claude Desktop.

Cause (code on `main`, `12c7e91`). The D21 singleton rule ("while a plugin serves MCP, the config-file entry goes") is applied on every install and update, and the Claude Desktop case extends it to a different app:
- `src/agents/claude/mod.rs:724-737`, `Claude::apply` for `Kind::Desktop`: when `code_serves_mcp(cfg)` is true it calls `unregister_mcp_ours(cfg, &desktop_path(), "rtok")` instead of `register_mcp`. `Mode::Update` takes the same branch as install.
- `src/agents/claude/mod.rs:422-426`, `code_serves_mcp` is true whenever the Claude Code plugin is installed, so the desktop entry is always removed.
- T243/T244 assumed the desktop file only feeds the desktop app's Code tab. It is also the only place Claude Desktop's own chat reads MCP servers from, so removing it takes rtok out of Claude Desktop entirely.
- The read side hides it. `Claude::installed(Kind::Desktop)` (`mod.rs:688-694`) reports `mcp` when the file has `"rtok"` *or* `code_serves_mcp`. `Claude::installed(Kind::Cli)` (`mod.rs:700-706`) and most hosts below count `mcp` as installed when the plugin is present.

Hosts with the same pattern (install/update removes, or never writes, the agent-config MCP entry while a plugin serves MCP):
- Claude Desktop: `claude/mod.rs:724-737`, removes `mcpServers.rtok` from `claude_desktop_config.json` while the Claude Code plugin is installed.
- Claude Code: `claude/mod.rs:764-767`, removes `mcpServers.rtok` from `~/.claude.json` while the plugin is installed.
- Cursor: `cursor/mod.rs:113-117` with `plugin_is_mcp` (`:246`), never writes `~/.cursor/mcp.json` while the plugin is linked.
- Copilot CLI and Gemini CLI: the shared `d21_plugin_apply` (`agents/mod.rs`, the `if remove || plugin_installed(cfg)` branch), removes `mcpServers.rtok` from `~/.copilot/mcp-config.json` and from Gemini's settings.
- Codex: `codex/mod.rs:334-337`, `run(cfg, true)` removes `[mcp_servers.rtok]` from `~/.codex/config.toml` while the plugin is enabled.
- VS Code and VS Code Insiders: `vscode/mod.rs:140-148`, removes `servers.rtok` from `mcp.json` while the plugin is linked (T196).
- ZCode: `zcode/mod.rs:257-260`, removes `mcp.servers.rtok` while `plugin_serves`.
- Kimi: `kimi/mod.rs:127-130` (`plugin_detected`), removes rtok's tables.
- Grok Build: `grok/mod.rs:97-101` (`plugin_detected`), removes `[mcp_servers.rtok]`.
- Not affected (MCP is written on install/update regardless of a plugin): Kilo, OpenCode, Cline, Devin, Windsurf, Zed, MiMo, CodeWhale. Check omp, pi and Antigravity (they link a plugin; verify whether their plugin serves MCP and whether their config entry is skipped) and add them here if they match.

Decision (record as a decision row; it amends D21 for MCP and replaces T271's "sweep" item and the MCP half of T243/T244):
1. Install and update always write rtok's MCP entry into each agent's own config. Only `remove` takes it out. A plugin being installed no longer suppresses or strips it.
2. Duplicates with the same name are left to the agent to merge (Claude Code merges same-name servers across scopes). All config-file entries keep the name `rtok`.
3. Where two entries could conflict by name on one surface (the agent errors or refuses on a duplicate name instead of merging), use a different name per surface, recorded in the host's README row.
4. Hooks keep the D21 singleton rule; this task changes MCP only.

Architecture principle (applies to every step below and to T275.1):
- One shared MCP core for all hosts: install, update, remove, status (`installed`), the `doctor` check and `ping` are one implementation in `src/agents/mod.rs` (or a new `src/agents/mcp.rs`), parameterised only by data: the host's config file path(s), the JSON/TOML key path (`mcpServers`, `servers`, `context_servers`, `mcp.servers`, `[mcp_servers]`), the server name(s) per surface, the command form (bare `rtok` or absolute path), and the entry shape (`type`, `args`, `env`).
- Host-specific differences live in one table, not in code branches: a per-host `McpSpec` entry (for example a `const` slice or a method on `Agent` that returns data) with those fields plus how the agent treats duplicate names (merges same-name entries, overrides by scope, errors, or shows two servers), whether a plugin also serves MCP and under what name, and the headless ping command for T275.1. `docs/agents.md` is generated from that table.
- No per-host `if plugin …` / `if kind == Desktop …` branches for MCP. The host `apply` / `installed` code calls the shared core with its spec. A new host is added by adding a table row and a Vfs test row, not new MCP logic. The per-host MCP functions this replaces (`register_mcp`, `unregister_mcp`, `plugin_is_mcp`, `code_serves_mcp`, the MCP half of `d21_plugin_apply`) are deleted once every host uses the core.
- Tests are table-driven too: one Vfs test iterates every row and runs install, update, remove, status and ping against it.

Fix:
1. For each host above, verify against the agent's docs and one real run how it treats two MCP servers named `rtok` from different sources (merge, override by scope, error, or two servers), and whether a plugin-served server is namespaced (Claude: `plugin_rtok_rtok`, so it never merges with `rtok`). Record per host in `research.md` with the agent version and a docs link. This decides where rule 3 applies.
2. Write side: change each listed `apply` so that install and update call `register_mcp` whenever `cfg.setup.mcp` is set, and `unregister_mcp` only on `Mode::Remove`. For the Claude Desktop case, drop the `code_serves_mcp` branch in `claude/mod.rs:724-737`. For Copilot and Gemini, change `d21_plugin_apply` once. Keep T246's ownership check, so a user-edited entry is left and reported.
3. Name conflicts: where step 1 shows a surface sees both a plugin server and the config entry as two servers (for example the Claude desktop Code tab, T271), give that pair distinct names or drop the plugin's MCP server (keeping its hooks) so the agent merges one `rtok`. Pick per host from step 1 and note it in the decision row.
4. Honest status: `installed()` reports `mcp` only when the agent's own config file really holds rtok's entry. Remove the `|| code_serves_mcp(cfg)` fallback in `claude/mod.rs:688-694` and the "plugin implies mcp" shortcut in the other hosts; show a plugin-served MCP as its own line (for example `✓ mcp (plugin)`) so it never stands in for the config entry.
5. `doctor`: warn per host when the agent is installed but its config has no rtok MCP entry, naming `rtok agents install <host>`. Keep T171's duplicate check only where step 1 shows the agent does not merge.
6. Tests (Vfs, no host disk, D29), for every listed host: (a) plugin installed plus `install`, then the config has rtok's MCP entry with the right command and args; (b) the same after `update`; (c) `remove` takes it out; (d) config without an entry plus the plugin, then `installed()` does not report `mcp` from the config and `doctor` warns; (e) a user-edited entry is kept with a `leave` line; (f) the T244 surface count is updated to the new rule: at most one merged rtok server per surface, or distinct names where step 3 applies. Re-bless `docs/agents.md` and each host's README MCP row with `RTOK_BLESS=1`.

Check: the tests above pass; on the creator's machine `rtok agents update claude` leaves `mcpServers.rtok` in `claude_desktop_config.json`, Claude Desktop's MCP settings list rtok, the Code tab and Claude Code list one rtok server, `rtok agents info|list` reports every host truthfully, and a Cursor/Codex/Copilot update keeps their config entries; `just check`.

Execution plan (lands before T277; T277 then moves the core into its crate):
1. Worktree `_worktrees/rtok-T275`, branch `t275-mcp-entry-always`. Research (Fix 1) per host into `research.md`, with version and docs link.
2. PR A, Claude Code and Desktop: shared MCP core in `src/agents/mcp.rs` (write/remove/status by spec), decision row amending D21, drop `code_serves_mcp` in `apply` and `installed`, Vfs tests (a)-(f) for Claude.
3. PRs B-D, the other affected hosts in groups of at most 10 files: Copilot and Gemini (`d21_plugin_apply`); Cursor, Codex, VS Code; ZCode, Kimi, Grok. Each deletes that host's MCP branch and adds its table rows to the shared Vfs test.
4. PR E: `doctor` warning (Fix 5), re-bless `docs/agents.md` and README MCP rows, then the creator-machine Check.
Progress (2026-09-28): PR A and B-D merged for Claude, Copilot, Gemini, Codex, Cursor (#444), VS Code (#455), ZCode (#459), Kimi (#467); Grok is #468. PR E merged (#483, doctor warning). Command Code, the last host that gated MCP on its plugin, is T275.2 (2026-10-11). Left: the creator-machine Check.
Each PR listed here becomes a subtask (`Tn.m`, D16) when it is claimed; this card stays the epic.

### T275.1. `rtok mcp ping <agent>`: prove the agent's rtok MCP server is alive and answering

`installed` only says a config entry exists (and today not even that, see T275). This command checks the real path: the agent starts rtok's MCP server from its own config, calls a tool, and writes the answer into its chat.

Syntax: `rtok mcp ping <agent> [--cli|--desktop] [--timeout <secs>] [--json]`. `<agent>` takes the same host names as `rtok agents install|info` (`claude`, `cursor`, `codex`, `copilot`, `gemini`, `vscode`, …); `--cli` / `--desktop` picks the variant like `agents install`; no agent means every host `agents list` shows with MCP installed. Place it in `src/cli.rs` as a `ping` subcommand of `rtok mcp`, next to `--call`. The foreign-server wrap stays behind `--` (`rtok mcp -- <argv>`), so `rtok mcp ping …` must not be parsed as a wrap argv: add a clap test for both forms. Add a matching line to `rtok agents info` help that points at it.

What it sends:
1. A new MCP tool `ping` in rtok's own server (`src/mcp`), taking `{"agent": "<display name>"}` and returning exactly `MCP <display name> жив`, where the display name is the host name `agents list` prints (for example `MCP Claude Desktop жив`, `MCP Claude Code жив`, `MCP Cursor CLI жив`). The call is logged like any MCP call, with no measurement row.
2. For hosts with a headless CLI, rtok runs the agent once with one prompt: `Call the rtok MCP tool "ping" with agent="<display name>" and reply with its result only, nothing else.` For example `claude -p …`, `codex exec …`, `cursor-agent -p …`, `copilot -p …`, `gemini -p …`. The exact non-interactive flag per host is verified against its docs and recorded in the host README.
3. For desktop-only hosts (Claude Desktop, VS Code chat, Windsurf, Zed …) there is no documented way to send a prompt from outside. rtok prints the same prompt for the user to paste into the app's chat, then does the server-side part itself: it spawns the MCP command exactly as written in that host's config (`command` / `args` / `env`), runs `initialize`, `tools/list` and `tools/call ping`, and checks the reply. That proves the configured server starts and answers, but not that the app loaded it; the output says which of the two was checked.

Success: the agent's chat reply (captured stdout for headless hosts) is exactly `MCP <display name> жив`, after trimming whitespace. Anything else is a failure: no rtok entry in the host's config, the server fails to start, the `ping` tool is missing, a timeout (default 60 s), or a different reply. Each failure prints its reason and the fix (for a missing entry, `rtok agents install <agent>`). Exit code 0 only when every checked host succeeds. `--json` prints one object per host: `agent`, `variant`, `mode` (`chat` or `server-only`), `reply`, `ok`, `reason`.

Tests: unit test for the `ping` tool reply text; clap tests for `rtok mcp ping` vs `rtok mcp -- …`; a Vfs test that the server-only check reads `command` / `args` from each host's config (rtok's entry, not a hardcoded path); an e2e that spawns `rtok mcp` through a fake host config and gets `MCP Test жив`; headless hosts are covered by a fake agent binary on PATH that runs the MCP call and echoes the result.

Check: on the creator's machine `rtok mcp ping claude --cli` prints `MCP Claude Code жив` from `claude -p`, `rtok mcp ping claude --desktop` checks the server from `claude_desktop_config.json` and prints the prompt, and after T275 both succeed; `just check`.

Execution plan:
1. `src/mcp.rs`: tool `ping`, always listed, reply `MCP <agent> жив`, logged through the existing call row (no measurement).
2. `src/cli.rs`: `ping` subcommand of `rtok mcp` beside `--call`; wrap stays behind `--`. `agents info` long help points at `rtok mcp ping`.
3. `src/mcp/ping.rs`: headless rows for claude (`-p`), codex (`exec`), cursor (`cursor-agent -p`), copilot (`-p`), gemini (`-p`), each recorded in that host README with the docs link. Every other variant is server-only: read `command` / `args` / `env` from `Agent::files`, spawn through `doctor`'s MCP round-trip (`initialize`, `tools/list`, `tools/call ping`).
4. Tests: ping reply text; clap `mcp ping` vs `mcp --`; Vfs bodies for every host's MCP shape; e2e spawn of `rtok mcp` from a fake config (`MCP Test жив`); fake `claude` on PATH.
5. Verify with `just check`. The creator-machine `claude --cli` / `--desktop` run stays open.

### T276. Spinner audit: every place rtok runs an external command and the user waits for its output

Goal: whenever rtok starts an external process and a person at a terminal waits for its result, a spinner is on screen from the moment the process starts until its output arrives, with a message that says what is happening (`installing plugin via claude…`, `probing MCP server context7…`, `creating worktree…`). Use the existing helpers in `src/render.rs` (`loader`, `spinner`, both silent when stderr is not a TTY) and `with_loader` in `src/cli.rs`; no new spinner code.

Audit (first pass, `rg 'Command::new|tokio::process'` on origin/main `12c7e91`): 39 call sites in 16 production files (about 10 more are in tests and do not count). No `tokio::process` use. Step 1 of the fix confirms this list.

Already covered (spinner shown today):
- `rtok agents list` / `rtok agents info`: `app_version` (`agents/mod.rs:297`, `<bin> --version`) runs inside `with_loader("listing hosts" / "reading host")`.
- `rtok agents install/update/remove` when stdin is not a TTY: the 10 calls in `agents/restart.rs` (`tasklist`, `pgrep`, `osascript` x2, `taskkill`, `killall`, `open`, `cmd`, direct spawn, `xdg-open`) and the host plugin CLI calls through `run_cli` / `spawn_cli` (`agents/mod.rs:248,252`) run inside `with_loader("updating host")`.
- `rtok graph index`: LSP servers spawned in `graph/lsp.rs:202` run under `render::spinner("indexing")`.

Missing, output awaited, spinner to add:
1. `rtok agents install/update/remove` in an interactive terminal (`apply_hosts`, `cli.rs`): T81 turned the loader off entirely so it never draws over the plugin question. Instead, show a loader per step (`installing plugin via <bin>…`, `closing <app>…`, `reopening <app>…`) and stop it before any prompt and restart it after the answer. Covers the same `restart.rs` and `run_cli` sites.
2. `rtok doctor` MCP probe (`doctor.rs:924-938`, `spawn_mcp` / `mcp_command`): starts each configured MCP server (often `npx` / `uvx`, several seconds cold) and waits for `tools/list`, with nothing on screen. Add `probing MCP server <name>…` per server.
3. `rtok worktree add/gc/clean` (`worktree/git.rs:11`, the shared `git` helper): `git worktree add` and removal can take seconds on a large repo. Add a loader around the long `git` calls (`creating worktree…`, `removing worktree <id>…`); keep quick reads (`rev-parse`, `worktree list`) without one.
4. `rtok bench` (`bench.rs:268`, `bench.rs:378`, `claude -p`): each run takes tens of seconds to minutes and prints nothing until it ends. Add a loader with the arm, the prompt number and elapsed time (`bench mcp 3/10…`).
5. `rtok run <command>` (`plugins/cmd/run.rs:150`, `shell_command`): if the output is captured and printed only when the command ends, add `running <command>…`; if it streams live, add nothing (a spinner would interleave with it). Decide from the code in step 1.

No spinner needed (checked, with the reason):
- No person waits, or no terminal: `build.rs:12` (build time), `demon.rs:182,518` (supervised children), `bin/rtok-hook.rs:85` and `hooks/mod.rs:467` (hooks run by the agent), `otel/export.rs:496` (detached flush), `mcp/wrap.rs:44` (stdio proxy), `graph/watch.rs:720,740,759` (background watcher).
- The child owns the terminal: `log.rs:196` (`tspin` viewer), `demon.rs:315,317,326` (`rtok demon upgrade` hands the terminal to `ketch` / `rtok-update`, which print their own progress).
- Returns in milliseconds: `demon.rs:603` (`sysctl`), `bench.rs:88` (`sh` probe), `graph/lsp.rs:21,32` (`on_path`, `rustup which`), `graph/mod.rs:755` (`git diff --name-only`).

Architecture: one `ProgressRunner` for every external command (replaces adding spinners site by site):
- Where it lives: a `proc` module (`mod.rs` for the runner, `indicator.rs`, `parse.rs`) in the existing crate `crates/rtok-sys` (OS process shims), which both `rtok` and `crates/rtok-mcp` (T277) depend on, so the MCP crate needs neither the `rtok` crate nor its own `Command::new`; no new crate. It is the only place in the workspace that calls `std::process::Command::new`; `clippy.toml` gets `disallowed-methods = ["std::process::Command::new"]` with an `#[allow]` only inside `crates/rtok-sys`, so a new call site that bypasses the runner, in `rtok` or in `crates/`, fails `just check`. `TtyIndicator`'s `indicatif` dependency moves to that crate with its `toolchain.md` row. The Windows shim logic now in `spawn_cli` (`agents/mod.rs:246`) and `mcp_command` (`doctor.rs:932`) moves into the runner too.
- Interface:
  - `ProgressRunner::new(label: &str, program, args)` returns a builder with `.cwd()`, `.env()`, `.stdin()`, `.timeout()`, `.progress(Progress)`, `.output(Output)`, then `.run() -> Result<Captured>` or `.spawn() -> Result<Running>`.
  - `enum Output { Capture, Stream, Inherit, Detached }`. `Capture` keeps stdout/stderr and shows an indicator until exit; `Stream` forwards lines live and suspends the indicator around each write; `Inherit` hands the terminal to the child (`tspin`, `ketch` upgrade) with no indicator; `Detached` is for background children (demon, otel flush, deferred hook) with no indicator.
  - `enum Progress { Unknown, Parse(Box<dyn ProgressParser>), Callback }`. `Unknown` draws a spinner. The other two draw a progress bar as soon as the first measurable update arrives and fall back to the spinner until then.
  - `trait ProgressParser: Send { fn feed(&mut self, stream: Stream, line: &str) -> Option<Update>; }` with `struct Update { pos: u64, total: Option<u64>, unit: Unit, msg: Option<String> }` and `enum Unit { Bytes, Percent, Items }`. Bytes render as MB with speed and ETA, percent as a 0-100 bar, items as `pos/total`.
  - `trait Indicator { fn set(&self, u: &Update); fn message(&self, m: &str); fn suspend<R>(&self, f: impl FnOnce() -> R) -> R; fn finish(&self); }` with two implementations: `TtyIndicator` (indicatif, on stderr) and `Hidden`. The runner picks `Hidden` when stderr is not a TTY, under `--json`, inside MCP and hook contexts, and when `RTOK_NO_PROGRESS=1`, so their output stays byte-identical.
- How a process reports progress:
  - Parsing: the runner reads stderr and stdout line by line (splitting on `\r` too, which is how `git`, `curl` and `cargo` redraw) and passes each line to the parser. Built-in parsers in `parse.rs` are `GitProgress` (`Receiving objects: 45% (120/267)`, `Updating files`), `Percent` (any `NN%`), `CargoProgress` (`Building [==> ] 120/300`), and `JsonLines` (a line `{"progress":{"pos":..,"total":..,"unit":".."}}`, for rtok's own child processes and plugin CLIs). The runner adds the flag that turns progress on where the tool has one (`git --progress`).
  - Callback: for work rtok measures itself, `ProgressRunner::batch(label, total, Unit::Items)` returns a `Batch` whose `.step(msg)` moves the bar; each command inside the batch runs with its own spinner line under the bar. `rtok bench` (runs), `rtok doctor` (servers), `rtok worktree gc/clean` (worktrees) and `rtok agents install/update/remove` over several hosts use this.
  - Prompts: `proc::suspend(|| ask(...))` hides every live indicator while rtok waits for an answer and redraws it after. This replaces the T81 rule that turned the loader off for the whole interactive run.
- Result: a new host, plugin CLI or subcommand that runs a program through `ProgressRunner` gets the right indicator with no extra code; the point fixes 1-5 above become one-line `label` / `progress` / `output` choices.

Migration plan:
1. Add the `proc` module in `crates/rtok-sys` with the runner, both indicators, the four parsers and `Batch`; unit tests feed recorded `git` / `cargo` / `curl` stderr into the parsers, and a test asserts `Hidden` writes nothing.
2. Move the shared helpers first: `worktree/git.rs::git`, `agents/mod.rs::run_cli` / `spawn_cli` / `app_version`, `doctor.rs::spawn_mcp` / `mcp_command`. That converts most user-facing sites at once.
3. Convert the remaining sites file by file, choosing `Output` per the audit: `agents/restart.rs` (10, `Capture`), `bench.rs` (3, `Batch` plus `Capture`), `plugins/cmd/run.rs` (`Stream` or `Capture` per step 1), `graph/lsp.rs` and `graph/mod.rs` (`Capture`, long-lived LSP children via `.spawn()`), `graph/watch.rs` (`Capture`, hidden in the watcher), `demon.rs` (`Detached` for children, `Inherit` for upgrade, `Capture` for `sysctl`), `log.rs` (`Inherit`), `otel/export.rs`, `hooks/mod.rs`, `bin/rtok-hook.rs`, `mcp/wrap.rs` (`Detached` / `Stream`, always hidden). `build.rs` stays on `Command` (build scripts cannot use the crate) and is the one listed exception.
4. Delete the point indicators: `with_loader` and the T81 `interactive` switch in `cli.rs`, and `render::loader` / `render::spinner` once `graph index` uses a `Batch` over files. `render.rs` keeps only styles.
5. Turn on the `clippy.toml` ban and fix whatever it still finds, including `crates/`.
6. Update `docs/` (contributor notes: "run external programs only through `proc::ProgressRunner`") and `CHANGELOG.md`.

Fix:
1. Confirm the audit list above with `rg` over the whole workspace (including `crates/`), and record each site as covered / add / not needed in a table in this card.
2. Implement the `ProgressRunner` migration above; items 1-5 are covered by it, not by separate spinners. Every message names the action and the target in present tense and ends with `…`; the loader is cleared (`finish_and_clear`) before the command's own output or any error is printed.
3. Interactive rule (T81) stays: a loader never runs while rtok is waiting for an answer. Put the pause and resume in one helper so every prompt uses it.
4. Nothing changes when stderr is not a TTY: pipes, CI, `--json` and MCP output stay byte-identical.
5. Progress bar over spinner: when an operation can report progress (megabytes, percent, files or items done out of a known total), show an `indicatif` progress bar with that count, not a spinner. A spinner is only for a wait that cannot be measured, where the command gives no intermediate data until its output arrives. For the items above this means: `rtok bench` shows a bar over runs (`3/10`), `rtok doctor` a bar over servers when it probes more than one, `rtok worktree gc/clean` a bar over worktrees removed; a single `claude -p` run, one MCP server start or one `git worktree add` stays a spinner.

Check: on a TTY, each of items 1-5 shows its message from the moment the process starts until output appears; `rtok agents update claude` in a terminal shows step loaders and the plugin question is readable; `rtok doctor 2>/dev/null` and `--json` output are unchanged; a Vfs or snapshot test asserts no spinner bytes on non-TTY stderr; every operation with a known total shows a progress bar with its count and a spinner appears only on unmeasurable waits; `just check`.

Execution plan (after T275, T277 and T279, which touch the same `agents` spawn helpers):
1. Worktree `_worktrees/rtok-T276`. Confirm the audit with `rg` over the workspace and write the site table into this card (Fix 1).
2. PR 1: the `proc` module in `crates/rtok-sys` with `ProgressRunner`, `TtyIndicator` / `Hidden`, the four parsers, `Batch`, `proc::suspend`; parser tests on recorded stderr and a no-bytes test for `Hidden`.
3. PR 2: the shared helpers (`worktree/git.rs::git`, `run_cli` / `spawn_cli` / `app_version`, `spawn_mcp` / `mcp_command`), with `proc::suspend` replacing the T81 switch.
4. PRs 3-4: the remaining sites file by file, per the audit's `Output` choice.
5. PR 5: delete `with_loader` / `render::loader` / `render::spinner`, turn on the `clippy.toml` ban, contributor docs and `CHANGELOG.md`.
Each PR listed here becomes a subtask (`Tn.m`, D16) when it is claimed; this card stays the epic.

### T276.3. Shared agent helpers on `ProgressRunner`: `run_cli`, `spawn_cli`, `app_version`, doctor MCP probe

Split from T276 (migration plan step 2, 2026-10-11, D16): `agents/mod.rs::run_cli` / `spawn_cli` / `app_version` and `doctor.rs::spawn_mcp` / `mcp_command` go through `ProgressRunner`; `proc::suspend` replaces the T81 `interactive` switch so `rtok agents install/update/remove` shows step loaders and the plugin question stays readable; `probing MCP server <name>…` with a bar when more than one server. Starts after T275, T277 and T279 land (same spawn helpers). Unblocks T436.4. Depends on T276.1, T276.2.

Check: the T276 card's Check lines for items 1 and 2; a non-TTY test asserts no spinner bytes; `just check`.

### T276.4. `ProgressRunner` in `bench`, `plugins/cmd/run` and `agents/restart`

Split from T276 (migration plan step 3, 2026-10-11, D16): `bench.rs` (3 sites, a `Batch` over runs plus `Capture`), `plugins/cmd/run.rs` (`Stream` or `Capture`, decided from the code), `agents/restart.rs` (10 sites, `Capture`). Depends on T276.1, T276.2.

Check: the T276 card's Check lines for items 4 and 5; `just check`.

### T276.5. `ProgressRunner` in `graph`, `demon`, `log`, `otel`, hooks and the MCP wrapper

Split from T276 (migration plan step 3, 2026-10-11, D16): `graph/lsp.rs`, `graph/mod.rs`, `graph/watch.rs` (`Capture`, long-lived LSP children through `.spawn()`), `demon.rs` (`Detached` / `Inherit` / `Capture`), `log.rs` (`Inherit`), `otel/export.rs`, `hooks/mod.rs`, `bin/rtok-hook.rs`, `mcp/wrap.rs` (`Detached` / `Stream`, always hidden). Depends on T276.1.

Check: hook latency budget unchanged (fail open, 10 ms); `just check`.

### T276.6. Retire the point indicators, turn on the `Command::new` ban, docs and changelog

Split from T276 (migration plan steps 4-6, 2026-10-11, D16): delete `with_loader` and the T81 switch in `cli.rs`, `render::loader` / `render::spinner` (`graph index` moves to a `Batch` over files), drop `indicatif` from the root crate, add `disallowed-methods = ["std::process::Command::new"]` to `clippy.toml` with an `#[allow]` only inside `crates/rtok-sys`, update the contributor notes in `docs/` and `CHANGELOG.md`. Closes T276. Depends on T276.2 to T276.5.

Check: `just check` fails on a stray `Command::new` outside `crates/rtok-sys`.

### T277. Move rtok's MCP core into its own crate `crates/rtok-mcp`

Problem: install, update, remove, status and ping of rtok's MCP entry are written separately in each host (`register_mcp` / `unregister_mcp` / `installed` in 20+ `src/agents/<host>/mod.rs`, plus `plugin_is_mcp`, `code_serves_mcp`, `installed_mcp_only` and the MCP half of `d21_plugin_apply`). That is how the Claude Desktop entry removal (T275) was repeated on Cursor, Codex, VS Code, ZCode, Kimi, Grok, Copilot and Gemini. T275's architecture principle states the rule; this task makes it a crate boundary so a host cannot grow its own MCP logic again.

Crate layout (`crates/rtok-mcp`, workspace member, not published, no dependency on the `rtok` crate; it depends on `rtok-sys` for T276's `ProgressRunner`):
- `spec.rs`: `McpSpec` (the per-host data from T275): config path(s) per surface, format (`Json`, `Jsonc`, `Toml`), key path (`mcpServers`, `servers`, `context_servers`, `mcp.servers`, `mcp_servers`), server name, command form (bare or absolute), entry shape (`type`, `args`, `env`), duplicate-name behaviour (`Merges`, `OverridesByScope`, `Errors`, `ShowsBoth`), plugin-served name, headless ping command. `Surface { Cli, Desktop, Ide }`.
- `registry.rs`: the one list of rtok's MCP servers (today `rtok`, plus graph or plugin servers when they get their own entry): name, command, args, env. Hosts never build an entry by hand.
- `config.rs`: read and write the host config through a `Fs` trait (so the existing `Vfs` tests plug in): `write_entry`, `remove_entry`, `read_entry`, keeping unrelated keys, comments in JSONC and TOML formatting, with a backup like today.
- `status.rs`: `McpStatus { surface, entry: Present | Missing | Stale(diff), plugin_serves: Option<name> }`; the truth source for T278.
- `ping.rs`: T275.1's ping: headless run through the spec's command, or spawn the configured server and call the `ping` tool.
- `doctor.rs`: the per-host MCP check `rtok doctor` runs (entry present, command resolves, server starts, `tools/list` answers).
- `ops.rs`: `apply(spec, Mode::{Install, Update, Remove}, fs) -> Report`, the only entry point hosts call.

What moves: from `src/agents/mod.rs` the MCP read/write helpers, `installed_mcp_only`, the MCP half of `d21_plugin_apply`; from every host its `register_mcp` / `unregister_mcp` and the MCP branch of `installed`; `plugin_is_mcp` (cursor) and `code_serves_mcp` (claude); from `src/doctor.rs` the MCP probe (`spawn_mcp`, `mcp_command`, using T276's `ProgressRunner` from `crates/rtok-sys` for the spawn, so the crate never calls `Command::new` itself); the `ping` tool body for T275.1. What stays in `rtok`: hooks, proxy, plugin install via host CLIs, desktop restart, and each host's `McpSpec` value.

Migration by host:
1. Create the crate with `spec`, `registry`, `config`, `status`, `ops` and a table-driven `Vfs` test over sample specs. No host uses it yet.
2. Claude (Code and Desktop) first, because it is where T275 was found: implement its `McpSpec`, route `apply` and `installed` through `rtok_mcp::ops` / `status`, delete `code_serves_mcp`. Land with T275's fix and T278's status.
3. The hosts T275 lists as affected: Cursor, Copilot, Gemini, Codex, VS Code and Insiders, ZCode, Kimi, Grok. One commit per host, each deleting that host's MCP functions.
4. The unaffected hosts: Kilo, OpenCode, Cline, Devin, Windsurf, Zed, MiMo, CodeWhale, omp, pi, Antigravity.
5. Move `doctor`'s MCP probe and add `ping` (T275.1) on top of the crate.
6. Delete the now-empty helpers in `src/agents/mod.rs`, add a test that fails when a file under `src/agents/` touches an MCP key directly, generate the host table in `docs/agents.md` from the specs.

Check: every host's install, update, remove, status and ping pass the same table-driven test; `rg 'mcpServers|context_servers|mcp_servers' src/agents` finds only `McpSpec` values; `just check`.

Execution plan (after T275 PR A, which gives the core in `src/agents/mcp.rs`):
1. Worktree `_worktrees/rtok-T277`. PR 1: crate `crates/rtok-mcp` with `spec`, `registry`, `config` (behind an `Fs` trait the `Vfs` implements), `status`, `ops`, and the table-driven test over sample specs; `toolchain.md` row.
2. PR 2: Claude moves onto the crate; `src/agents/mcp.rs` shrinks to spec rows.
3. PRs 3-4: the other hosts, one commit per host, at most 10 files per PR.
4. PR 5: `doctor`'s MCP probe into the crate; `ping` joins when T275.1 lands.
5. PR 6: delete leftover helpers, add the guard test against MCP keys under `src/agents/`, generate the `docs/agents.md` host table.
Each PR listed here becomes a subtask (`Tn.m`, D16) when it is claimed; this card stays the epic.
Progress: PR 1 merged (#437); PR 2 (Claude Code and Desktop) done as T277.1. Next: PR 3, the other hosts.

### T278. `rtok agents info <agent>` reports the real MCP state, not "mcp installed" by assumption

Problem: `agents info claude` shows `mcp installed` for Claude Desktop with no `rtok` entry in `claude_desktop_config.json`, because `installed` (`claude/mod.rs:688-694`) returns `mcp` when the Code plugin is present (`code_serves_mcp`), and matches the text `"rtok"` anywhere in the file. That hid today's bug. Other hosts do the same when a plugin is installed (`files.contains("mcp") || plugin`).

Fix:
1. Status comes from `rtok_mcp::status` (T277), or from an equivalent function in `src/agents/mod.rs` if T278 lands first. It parses the config and looks up the exact key path and server name; no substring match.
2. `agents info` prints MCP per surface on its own line, for example `mcp  desktop  missing (claude_desktop_config.json has no "rtok")`, `mcp  cli  plugin rtok@rtok (serves "rtok")`, `mcp  cli  entry ~/.claude.json`. A stale entry (wrong command or args) prints `stale` and the difference. Hooks keep their current line.
3. A plugin only counts for the surface it actually serves. The Claude Code plugin never makes Desktop show as installed.
4. `--json` and the web UI model (`web/model.rs`) carry the same fields: `surface`, `entry` (`present`, `missing`, `stale`), `plugin`.
5. `agents list` and `installed_hosts` use the same status, so "installed" means the same thing everywhere.

Tests: a table-driven `Vfs` test per host writes each combination (no entry, entry, stale entry, plugin only, entry and plugin) and asserts the printed and `--json` status matches the file content; a regression test for today's case (Code plugin installed, Desktop file without `rtok`) expects `desktop missing`.

Check: on the creator's machine, before T275 is fixed `rtok agents info claude` shows Desktop `missing`; after `rtok agents install claude` it shows `present`; `just check`.

Execution (2026-09-27): one PR off main. Status comes from `rtok_mcp::status` (T277, merged in #437) per surface. `agents info`, `agents list`, `installed_hosts` and `web/model.rs` all read it, so a plugin only counts for the surface it serves. Tests: one table-driven `Vfs` test per host, plus the regression "Code plugin installed, Desktop file without `rtok`" → `desktop missing`. The T275 per-host PRs still open (VS Code, ZCode, Kimi, Grok) touch the same host modules, and whichever lands second rebases.
Progress (2026-09-28): merged in #464 (per-surface `mcp` lines, `--json` and web `mcp: [{surface, file, entry, diff?, plugin}]`, table-driven test over every host, the `desktop missing` regression). Left: the creator-machine Check.

### T279. One plugin version scheme for every install source (GitHub, local, marketplace), and `agents update` that skips an up-to-date plugin

Problem: every plugin manifest is still `0.0.1` while rtok is at `0.10.0` (tag `v0.10.0`): `plugins/claude/.claude-plugin/plugin.json` (`rtok@rtok`), `plugins/codex/.codex-plugin/plugin.json`, `plugins/cursor/plugin.json` and `.cursor-plugin/plugin.json`, `plugins/copilot/plugin.json`, `plugins/gemini/gemini-extension.json`, `plugins/kimi/kimi.plugin.json`, `plugins/pi/package.json`. Claude Code caches a plugin by its manifest version (`~/.claude/plugins/cache/rtok/rtok/0.0.1/`, `installed_plugins.json` records `"version": "0.0.1"` for commit `12c7e91`), so a new build with the same number is not a new version to it. rtok itself has no way to tell which plugin build is installed or where it came from, so `agents update` either reinstalls every time or trusts the host. A plugin reaches a user from three sources, and the scheme has to work for all of them:
- GitHub: the host installs straight from the repo (`claude plugin marketplace add pyrlyn/rtok`, Gemini `extensions install https://github.com/pyrlyn/rtok`, similar for others), at a branch or tag.
- Local: installed from the plugin tree of an rtok checkout or install (`plugins/<host>`; Claude via `claude plugin marketplace add <path>`, the pre-T139 flow), used for development and offline installs.
- Marketplace: the host's own catalog entry, which updates when a push to the repo changes the committed catalog (`.claude-plugin/marketplace.json`, `.agents/plugins/marketplace.json`, T183's `marketplace.yml` verifies them).

1. Version file: format and location.
   - Each plugin tree carries `plugins/<host>/.rtok-plugin-version`, committed, one JSON object: `{"schema":1,"plugin":"claude","version":"0.10.0"}`. `version` is SemVer and equals the rtok version at the commit. It sits at the plugin root, so every source copies it with the plugin: the GitHub and marketplace installs land it in the host's install dir (for Claude `installPath` from `installed_plugins.json`), a local install copies or links it.
   - Local installs from a git checkout add build metadata when rtok writes the installed copy: `"version":"0.10.0+g12c7e91"`, and `+g12c7e91.dirty` for uncommitted changes, plus `"source":"local"`. rtok writes the same string into the installed copy's manifest `version`, so hosts that cache by version (Claude) also see a new build.
   - rtok keeps an install receipt per host at `$XDG_STATE_HOME/rtok/plugins.json` (`~/Library/Application Support/rtok/plugins.json` on macOS, `%LOCALAPPDATA%\rtok\plugins.json` on Windows): `{"claude":{"source":"github","ref":"v0.10.0","marketplace":"rtok","path":"<installPath>","version":"0.10.0","installed_at":"…"}}`. For a local install `ref` is the checkout path and commit; for a marketplace install it is the marketplace name and its source.
2. How update finds the source and the new version.
   - Source: the receipt first. With no receipt, the host's own records: for Claude, `known_marketplaces.json` (`"source":"github","repo":"pyrlyn/rtok"` means GitHub or marketplace, a `directory` source means local) and `installed_plugins.json` (`installPath`, `version`, `gitCommitSha`); each host's lookup is a field in its plugin spec, next to T275's `McpSpec`, not a code branch.
   - New version, by source. GitHub: read `plugins/<host>/.rtok-plugin-version` at the tag that matches the running binary (`v{CARGO_PKG_VERSION}`), so plugin and binary stay in step; with `--channel main` read it from `main`. Local: read the file in the local plugin tree and add `+g<sha>[.dirty]` from `git describe`. Marketplace: refresh the host catalog first (`claude plugin marketplace update rtok` and the equivalents), then read the file inside the refreshed catalog checkout (for Claude `~/.claude/plugins/marketplaces/rtok/plugins/claude/.rtok-plugin-version`).
   - Installed version: the `.rtok-plugin-version` inside the installed copy, falling back to the receipt, then the host record's `version`.
3. Comparison and decision.
   - Parse both as SemVer. Equal version and equal build metadata: skip, print `plugin rtok@rtok 0.10.0 up to date (github)`. New is greater: update in place through the host (`claude plugin update`, T242.3's reinstall fallback), then rewrite the receipt. New is lower: skip with a warning naming both versions; `--force` (step 6) reinstalls anyway.
   - Same base version but different build metadata (local builds): update, since SemVer ignores metadata and the build did change.
   - A source change (the receipt says local, the user now installs from GitHub, or T139's stale marketplace) is always an update: reinstall from the new source.
   - `--force` bypasses this table entirely (step 6); `--dry-run` prints the decision for each host without acting. The decision table is one pure function with unit tests.
4. Installs without a version file (everything before this task).
   - The installed version comes from the host record if it has one (`0.0.1` today), otherwise it is treated as `0.0.0`. Either is lower than any real release, so the first `agents update` after this task updates once, writes the version file into the new copy and creates the receipt; from then on the normal rule applies.
   - If that update fails, the old plugin stays, the receipt is not written, and the report says `plugin rtok@rtok: legacy install, update failed: …` so the next run tries again.
   - `agents info` shows `legacy (no version file)` for such installs until then (ties into T278).
5. Release-plz and bump.
   - `tools/plugin-versions.sh --set <version>` writes the version into every `.rtok-plugin-version` and every manifest `version` above; `--check <version>` lists each file that differs and exits non-zero.
   - `tools/release.sh` (the version commit `bump.yml` makes) calls `--set` in the same commit that edits `Cargo.toml` and `Cargo.lock`, and the version files join the list of files that commit may touch. `release-plz` only edits Cargo files and today releases only `rtok-plugin-sdk`, so the plugin files never go through it; if `release-plz` ever opens the rtok release PR, that PR runs `--set` in a follow-up step, and the check below blocks it until it does.
   - CI: `ci.yml` runs `--check` against the `Cargo.toml` version on every PR; `release.yml` runs `--check` against the tag (`v` stripped) before building and fails the release on a mismatch. A Rust test asserts every version file and manifest equals `env!("CARGO_PKG_VERSION")`, so `just check` catches drift locally.
   - Raise every manifest and add every version file at `0.10.0` in this task's PR.

6. `--force`: unconditional reinstall.
   - Syntax: `rtok agents update <agent>[,<agent>…] --force`, and `rtok agents update --force` with no host for every host rtok is installed in (the existing "host omitted" rule; `--all` already means "all variants, CLI and desktop" in `UpdateArgs`, so it keeps that meaning and does not also mean "all hosts"). Combines with `--cli` / `--desktop`, `--no-restart` and `--dry-run`.
   - Behaviour: no version lookup, no comparison, no skip. For each selected host rtok removes the installed plugin through the host (`claude plugin uninstall rtok@rtok` and the equivalents, plus a stale marketplace entry as in T139), then installs it again from the source: the receipt's source, else the one step 2 detects, else the default for that host (GitHub at `v{CARGO_PKG_VERSION}`). `--force --source github|local|marketplace` picks the source explicitly and replaces the recorded one.
   - Plugin not installed: `--force` installs it from scratch (the uninstall step is skipped, not an error), then writes the version file and receipt like a normal install.
   - Version file and receipt: after a successful reinstall rtok reads `.rtok-plugin-version` from the new installed copy (writing the `+g<sha>` form for a local source) and rewrites the receipt with the new source, ref, path, version and time. A downgrade or an identical version is reinstalled all the same.
   - Failure: if the uninstall succeeds and the install fails, the report says so plainly (`plugin rtok@rtok removed, reinstall failed: …`), the receipt is deleted so the next `agents update` treats the host as "not installed" and installs, and the exit code is non-zero. MCP entries and hooks follow T275 (always rewritten), not the plugin step.
   - `--dry-run --force` prints `would reinstall rtok@rtok from github v0.10.0 (forced)` per host and changes nothing.
   - Tests (with a fake host CLI on `PATH` that logs its calls, plus `Vfs`): equal versions with `--force` still run uninstall then install; without `--force` they run nothing; a not-installed host with `--force` runs install only and writes the receipt; a newer installed version with `--force` is replaced by the older source version; `--force --source local` switches the receipt source; install failure after uninstall deletes the receipt and exits non-zero; `--force` with no host touches every installed host and only those; `--dry-run --force` calls no CLI and writes no file; the forced path never calls the version comparison function.

7. Documentation (required; T279 and T279.1 are not done without it).
   - A new page `docs/plugin-versions.md`, written for a developer who has not read the code. Sections, in this order:
     1. Why: hosts cache plugins by version, three install sources, what went wrong with `0.0.1`.
     2. The version file: location `plugins/<host>/.rtok-plugin-version`, the JSON fields (`schema`, `plugin`, `version`, `source`), who writes it (committed by the release, rewritten with `+g<sha>[.dirty]` for local installs), where it ends up after install for each host.
     3. The receipt `plugins.json`: path per OS, every field, when it is written, rewritten and deleted.
     4. Sources: GitHub, local, marketplace; how each is detected (receipt, then host records) and where the new version is read from for each, with the Claude paths as the worked example.
     5. The decision: a table of every case (equal, newer, older, same base with different build metadata, source change, legacy without a file) and its result (skip, update, reinstall, warn), then `--force`, `--source` and `--dry-run`, and what happens when a reinstall fails.
     6. `rtok agents outdated` and `rtok agents update --check`: what is listed and what is hidden, the two "nothing to do" messages, the `--json` schema, `--exit-code`, why it works offline.
     7. Releasing: `tools/plugin-versions.sh --set` / `--check`, where `tools/release.sh` and `bump.yml` call it, the CI and `release.yml` checks, why `release-plz` does not touch these files and what happens if it ever opens the rtok release PR.
     8. Troubleshooting: "plugin stays old after update", "legacy install", "version mismatch in CI", each with the command to run.
   - Each section has a short command example with real output copied from a run, not invented.
   - Links: README gets a line under the plugin/agents section, `Plugin versions and updates: [docs/plugin-versions.md](docs/plugin-versions.md)`; `docs/release.md` links the Releasing section; `docs/agents.md` links it next to `agents update`; `CHANGELOG.md` mentions the page. The landing site picks the page up through the existing docs sync (owned separately; this task does not edit `sync-docs.yml`).
8. Tests (required; the full set for T279 and T279.1).
   - Unit tests on the pure decision function (no files, no processes), one case each: equal versions skip; newer available updates; older available skips with a warning; same base with different `+g<sha>` updates; same base with `.dirty` against clean updates; identical build metadata skips; source change reinstalls; legacy (no version) against any release updates; `--force` returns reinstall for every one of these inputs; invalid SemVer in a version file is an error that names the file.
   - Unit tests on the version file and receipt: parse and write round-trip; unknown `schema` rejected; receipt paths per OS; local version string from a clean and a dirty `git describe`.
   - Integration tests in `tests/plugin_versions.rs` with `Vfs` fixtures and a fake host CLI on `PATH` that logs every call:
     - update: equal versions run no CLI; a newer version runs `plugin update` and rewrites the receipt; an older one runs nothing and warns.
     - force: equal versions still run uninstall then install; a not-installed host runs install only and writes the receipt; a newer installed plugin is replaced by the older source version; the forced path never calls the comparison.
     - source change: `--force --source local` over a GitHub receipt reinstalls from the local tree and switches the receipt source; a stale marketplace (T139) triggers reinstall.
     - failure: the fake CLI fails install after a successful uninstall; the receipt is deleted, the report says `removed, reinstall failed`, the exit code is non-zero, and the next `update` installs.
     - dry-run: `--dry-run` and `--dry-run --force` spawn no CLI and change no file; their printed decisions match what a real run then does.
     - legacy: an install without a version file and with host record `0.0.1` updates once, then the second run skips.
     - outdated: only outdated rows are printed; all current prints the up-to-date line; nothing installed prints `no rtok plugins installed`; legacy rows show `legacy`; newer and same-version-with-metadata rows are hidden; `--json` matches the schema in T279.1; `--exit-code` returns 10 when something is outdated and 0 otherwise; `agents update --check` output is byte-identical to `agents outdated`.
     - offline: `agents outdated` runs with the network disabled (proxy env pointed at a closed port) and the fake CLI logs no call.
   - Release checks: `plugin-versions.sh --check` passes on the tree and fails after one manifest or version file is edited (a shell test in `tools/`); the Rust test that compares every manifest and version file with `CARGO_PKG_VERSION`.
   - All of it runs in `just check` and CI on macOS, Linux and Windows.

Check: `rtok agents update claude` on today's install (0.0.1, no file) updates once to `0.10.0` and writes the receipt; a second run prints `up to date` and runs no `claude` command; a local install from a dirty checkout shows `0.10.0+g<sha>.dirty` and a new commit triggers an update; `--dry-run` lists the decision per host; editing one version file to `0.0.2` fails the CI check and the test; `rtok agents update claude --force` reinstalls even when up to date and rewrites the receipt; `just check`.

Execution plan:
1. Worktree `_worktrees/rtok-T279`. PR 1: `.rtok-plugin-version` files and every manifest at `0.10.0`, `tools/plugin-versions.sh --set/--check` with its shell test, the `CARGO_PKG_VERSION` Rust test, `--check` in `ci.yml`, `release.yml`, `tools/release.sh`.
2. PR 2: version file and receipt types, the installed/new version lookup, and the pure decision function with its unit tests (step 8, first two groups). No behaviour change yet.
3. PR 3: `agents update` uses the decision; `--force`, `--dry-run`, `--source`, the legacy path, failure handling; integration tests in `tests/plugin_versions.rs` with a fake host CLI.
4. PR 4: `docs/plugin-versions.md` with real command output, and its links (step 7).
Each PR listed here becomes a subtask (`Tn.m`, D16) when it is claimed; this card stays the epic.
Progress (2026-09-28): PRs 1-4 merged (docs: #465, today's behaviour). Open against this card, for the creator: the installed copy's `.rtok-plugin-version` is never read or written (`read_installed` and `VersionFile::write` are unused), so there is no legacy line from it; a missing `claude` on `PATH` prints "already current"; the dry-run reinstall wording differs from step 5; `--source local` fails from a release install; the marketplace source does not read the catalog. Only Claude is wired; Codex, Copilot and Gemini follow.

### T281. Probe: tie a host session's hooks and its rtok MCP server to one agent

Creator request 2026-09-27 (T281–T290, D34): every agent working through rtok gets one rtok agent id, visible to the user in the terminal and to the agent over MCP. Hooks see the host's session id (`research.md` §26), but `rtok mcp` is started by the host with only `["mcp"]` and its cwd (`src/mcp.rs:592`, host from `[hook] host`); nothing today tells the MCP process which session it serves. Without that link an agent's MCP calls (`whoami`, `worktree_add`, `agent_send`) cannot be attributed to its agent id. No product code in this task.

Plan:
1. For each host with hooks and MCP (Claude Code CLI and desktop, Cursor, Codex, Copilot CLI, Grok, Gemini, Kimi, ZCode, CodeWhale): a throwaway MCP wrapper and hook script in the scratchpad (never committed) log, per process: pid, ppid chain up to the host process, cwd, every env var whose name contains `SESSION`, `CONVERSATION`, `THREAD` or the host name, MCP `initialize.params.clientInfo` and `_meta`, and the hook payload's session field.
2. Run one session per host by hand, on the creator's machine only; real agents are for manual debugging, never tests (T280). Include one sub-agent per host that has them.
3. Pick the link rule per host, in this order of preference: a session id env var the MCP process inherits (e.g. `GROK_SESSION_ID`, `GEMINI_SESSION_ID`); the nearest common host ancestor pid shared by hook processes and the MCP process; cwd + host + start time as the last resort, marked ambiguous when two sessions share a cwd.
4. Check whether one MCP process serves several sessions (the desktop apps may share one) and whether it outlives its session.

Check: `research.md` §26 gains a dated table host → host version → the MCP process's link to the session (env var name / ppid rule / cwd only / none) → sub-agent behaviour → one MCP process per session yes/no, each row with its source (the probe log or the vendor docs URL). The T283 card is updated with the rule per host.

Execution (2026-09-27): agent sessions may not read their own process environment (blocked by the host's policy), so the live runs are the creator's. (1) Worktree `_worktrees/rtok-t281`. (2) Vendor docs first, per host: which env vars an MCP server process inherits, the hook payload's session field, whether one MCP process serves one session; each fact cited with URL and date. (3) A probe kit in the scratchpad, never committed: an MCP stdio wrapper that logs pid, ppid chain, cwd, the filtered env var names and values, `initialize` params, then execs `rtok mcp`; a hook script that logs the same next to the payload's session field; a one-page run sheet for the creator. (4) `research.md` §26 gets the table from the docs now, with a "probe" column left `pending` until the creator's logs arrive; T283's card gets the rule per host once they do.

### T289. Worktrees the host creates join rtok: `rtok worktree adopt` and the post-create hooks

Depends on T285, T286. Only Claude Code can redirect worktree creation (T159). Cursor (`.cursor/worktrees.json` `setup-worktree*`), Kilo (`.kilo/setup-script`) and Devin/Windsurf (`post_setup_worktree`) only run a script after they create a worktree in their own pool (`research.md` §26). For worktrees to behave the same on every host, those must still get an owner, an agent id and rtok's remove / gc / clean.

Plan:
1. Worktree `_worktrees/rtok-T289`.
2. `rtok worktree adopt [<path>] [--task <id>] [--agent]` and MCP `worktree_adopt`: lock a host-made worktree with the v2 reason (T285), record the claim and `source: <host>`; the directory stays where the host put it. First confirm on each host whether a locked worktree breaks the host's own eviction (Cursor's cap of 25, Devin/Windsurf LRU); where it does, adopt records the claim in the store only, with no git lock, and the card says so.
3. Wire the post-create scripts: `rtok agents install <host> --project` writes rtok's entry into the project file (`.cursor/worktrees.json`, `.kilo/setup-script`, Devin/Windsurf's hook config), our entry only, the rest byte-for-byte (host-config rule); removal takes it out.
4. Hosts with native worktrees and no hook (Codex, Grok Build, MiMo, omp, Antigravity): the skill tells the agent to call `worktree_adopt` when it finds itself in a host-made worktree.
5. `rtok worktree list` already shows every registered worktree of the repo (git knows them wherever they are); add `source` (`rtok`, `claude`, `cursor`, …) from the path pool.

Check: adopt e2e in a scratch repo with a worktree under a fake `~/.cursor/worktrees/`; install/remove e2e per host writing only our entry; list shows `source`; `just check`.

Execution (2026-10-03, Claude Code / sonnet-5): split into four PRs, each at most about 300 LOC, stacked on T288 step 4. Design decisions the card left open:
- `adopt` is `claim` with three differences: the path defaults to the caller's worktree (cwd), `--task` names the task when the branch cannot (a detached HEAD such as Codex's `thread-N`; the directory name is the last fallback), and a worktree in a pool whose host evicts by itself (Cursor, Windsurf/Devin, Codex) gets **no git lock**, only the claim row. Reason: the card says to confirm per host whether a locked worktree breaks the host's eviction, and that needs a live run on each host (the creator's probe, like T281); until it is confirmed, a lock could stop Cursor's cap of 25 from evicting, so the safe choice is the store-only claim. `worktree list` and `gc` already read an unlocked worktree's claim row (T285), so a live agent's adopted worktree is still never collected. Flip the pool table once a host is confirmed.
- The origin of a worktree is derived from its path (`~/.cursor/worktrees/`, `~/.windsurf/worktrees/`, `<repo>/.claude/worktrees/`, `<repo>/.kilo/worktrees/`, `$CODEX_HOME/worktrees`, `~/conductor/workspaces/`; else `rtok` under `[worktree] root`, else `other`), so no migration. The `worktree list` table already has a `source` column (source bytes), so the new field is named `origin` in the table and in `--json`.

### T329. Graph page: project selector, auto-added projects and linked projects (epic)

Ivan, 2026-10-01: in the web UI's graph tab, the graph is built for a project the user picks. The page always shows which project is selected. Projects the user needs are added automatically. Other projects can be linked to the selected one, and the graph then traverses into them as if everything were one project. If the selected project references other projects, those are added, indexed and linked automatically, so an agent working in the current project can follow the graph across them right away.

Today the graph plugin (`src/plugins/graph/`) always works on one root: the process's current directory. The index is keyed by that root (`index::canon(root)` in `src/store/symbols.rs`), and the MCP tools `symbol`, `callers`, `impact`, `outline` and `explore`, plus `dead` and `affected`, only see that root. The graph page shows the same single root (`root .`). There is no way to pick another project and no way to follow a call into a dependency's source.

Split (2026-10-03, complexity 5): one subtask = one PR, T329.1 to T329.21 in dependency order (T329.1 to T329.3, T329.6, T329.12, T329.13, T329.14, T329.20, T329.22 and T329.23 are already in `done.md`). This card stays the specification; each subtask reads the section it names and updates `docs/` (en, ru, uk) for its own part. T337, which gated T329.11/T329.17, is settled (requests never re-probe; the health check does); T334, which gated T329.9, is settled (`tags` stays the default, `auto` is opt-in); T336, which gated T329.4, is settled (the cwd, not the web selection).

#### Terms

- **Project**: a directory rtok indexes as one unit, identified by its canonical root path. Display name defaults to the directory name (or the package name from the manifest when there is one); the user can rename it.
- **Selected project**: the project the graph page answers for. The CLI and MCP tools do not follow it: without `project` they answer for the caller's current directory and its links (T336).
- **Link**: a directed edge "project A sees into project B". A link is either **manual** (the user made it) or **auto** (rtok made it from a reference, see 4).
- **Graph scope**: the selected project plus every project reachable through its links (transitively). All graph queries run over the scope.

#### 1. Project registry

- A `projects` table in the rtok store holds id, canonical root, display name, origin (`manual`, `session`, `worktree`, `mcp`, `reference`), created and last-used times, and per-project index status (rows, files, pending, `indexed_at`, watch state, last error).
- Each project keeps its own symbol index, keyed by its canonical root as today, so switching projects never re-indexes the others and never mixes their rows.
- Two paths that canonicalize to the same directory (symlinks, `..`, case on macOS) are the same project; registering one twice is a no-op that only updates last-used.
- A project whose root no longer exists stays in the registry marked **missing**: it is greyed out in the selector, excluded from the scope, and its links are kept so they come back if the directory returns. The user can remove it.
- Removing a project drops rtok's index rows, its links in both directions and its registry row. It never touches the project's files.
- The registry and links migrate forward with the store schema; an existing store starts with one project, the root it already indexed, selected.

#### 2. Project selector on the graph page

- The page header has a project selector listing every known project: name, root path, index status and a link count. It is searchable when there are more than about ten projects.
- Picking a project switches the whole page to that project's scope: summary counts, dead symbols, the symbol/callers/impact views and the graph drawing.
- The selection is stored in the rtok store, so it survives page reloads, other browser tabs (they update over `/ws`) and `rtok web` restarts.
- When `rtok web` starts in a directory that is a known project and nothing is selected yet, that project is selected. A stored selection that is now missing falls back to the current directory's project, with a notice.

#### 3. Current-project indicator

- The selected project's name and root path are always visible in the page header, together with its index status: rows, files, pending files, last indexed time and watch state.
- When the scope includes linked projects, the header says so ("+ 3 linked") and expands to list them, each with its own status.
- States the page must show clearly: **not indexed yet** (empty state with an "Index now" action), **indexing** (progress, the page stays usable on the old data), **stale** (pending files, same banner the tools already use), **failed** (the error and a retry action), **missing** (root gone).

#### 4. Automatic adding

Projects are added to the registry, without the user asking, in two ways.

**4a. Projects rtok sees in use.** The working directory of a hooked agent session, a worktree created or adopted through `rtok worktree` (T285, T289), and the root of any graph MCP call are registered when first seen. A worktree is registered as its own project (its files differ from the main checkout) with its display name showing the branch.

**4b. Projects the selected project references.** When a project is indexed, rtok reads its manifests and collects references to code that lives outside its root but on this machine. Each referenced directory is registered as a project (origin `reference`), indexed, and auto-linked from the referencing project. Reference sources, in this order:

- Cargo: `path = "..."` dependencies and `[patch]` entries, and workspace members outside the root.
- npm/pnpm/yarn: `file:`, `link:` and `workspace:` dependencies that resolve outside the root.
- Go: `replace` directives with a local path in `go.mod`, and `go.work` `use` entries.
- Python: path dependencies in `pyproject.toml` (`{ path = "..." }`, editable installs).
- Git submodules (`.gitmodules`) whose checkout is present.
- Anything else the indexer finds while resolving imports: an import that resolves to a file outside the root (through an LSP server or the language's resolver) adds that file's project root (nearest directory with a manifest or `.git`).

Rules for 4b:

- References are followed transitively: if B (referenced by A) references C, C is added, indexed and linked from B, so A's scope includes C. A depth limit (`[plugins.graph] reference_depth`, default 3) and a project cap (`max_auto_projects`, default 20) stop runaway chains; hitting either is shown on the page and logged, never silent.
- Registry dependencies that are not local source (crates.io, npm registry, PyPI, Go module cache) are not followed by default, so the scope stays the user's own code. An opt-in setting (`include_registry_deps = false`) can add them later; it is out of scope for the first PR.
- A reference to a path that does not exist is recorded on the referencing project as a warning ("references ../foo, not found") and nothing is added.
- Auto-indexing runs in the background with the existing index code; the selected project is usable while its references are still indexing, and results from a reference that is not indexed yet are marked incomplete rather than missing.
- Re-indexing a project re-reads its manifests: a new reference adds and links a project; a removed reference removes the auto link (the project stays in the registry until the user removes it). Manual links are never removed automatically.
- If the user unlinks an auto link, rtok remembers that and does not re-create it on the next index.

**Turning it off.** `[plugins.graph] auto_add_projects = true` controls 4a and `auto_link_references = true` controls 4b, both on by default, documented in `docs/config.md`. With both off, the registry changes only through the page and the CLI.

#### 5. Manual links

- From the selected project the user can link any known project and unlink any linked one; the page lists links with their kind (manual or auto) and the reason for auto links (for example "Cargo path dependency `../ketch-core`").
- Linking a project that is not indexed yet starts indexing it.
- Links are directional: linking B into A puts B in A's scope, not A in B's. The page offers "link both ways" as a shortcut that creates two links.
- Cycles are allowed (A to B to A). Scope building visits each project once, so cycles never loop or duplicate rows.
- A project cannot link to itself, and linking an already-linked project is a no-op.

#### 6. Cross-project traversal

- Every graph query runs over the scope as one graph: `symbol`, `callers`, `impact` (with `depth` and `to`), `explore`, `affected` and `dead`.
- A reference from a call site in one project to a definition in a linked project resolves and is followed, in both directions: `callers` of a function in B include call sites in A when A links B, and `impact` from a change in B walks up into A.
- Each result row shows which project it belongs to (project name badge on the page, a `project` field in JSON, a `[name]` prefix in text output).
- Ambiguity: when the same symbol name is defined in several projects in the scope, results are grouped by project and marked ambiguous, using the same banner the single-project path already uses. A definition in the selected project ranks first.
- `dead` is computed over the scope: a symbol in B used only from A is not dead while A links B. Dead symbols are still reported per project.
- `affected` with git changes reads `git diff` in every project in the scope that is a git repo, and maps test commands per project.
- Output caps and token budgets apply to the whole scoped answer, not per project, so linking projects does not multiply the size of an MCP reply.
- Watching: when `watch` is on, file changes in any project in the scope update its index and refresh the page over `/ws`.

#### 6a. Backends: `tags` by default; `auto` (LSP, then tree-sitter, then text) opt-in

Decision (2026-10-09, creator, T334): `[plugins.graph] backend` keeps `tags` (tree-sitter tags index) as the default, so the default answers stay byte-identical (Gate P30, `graph_contract.rs`). `backend = "lsp"` already falls back to tags since T376: when the server is missing, not ready, dead or answers "nothing", the answer is the tags one headed `(tags; lsp: <reason>)` with an `lsp_fallback` `Measurement` row (`docs/lsp.md`, "Without the server"). T329 adds `backend = "auto"` as an opt-in value: LSP first, tree-sitter second, plain text search last. `"tags"` and `"text"` pin one mode with no fallback; `"lsp"` keeps the T376 fallback to tags. Changing the default later needs a new gate with recorded `lsp.*` cold and warm latency rows, the `lsp_fallback` rate, and a creator decision that supersedes P30.

Backends are chosen per project and per language, not once per process: in a scope where A is Rust with rust-analyzer installed and B is Go with no server, A's rows come from LSP and B's from tree-sitter, in the same answer.

**Mode 1: LSP (first under `auto` and `lsp`).**

- Used when a server for the project's language is configured and works: the marker file is found (`Cargo.toml`, `compile_commands.json`, `tsconfig.json`, `pubspec.yaml`, plus any added later) and the server binary is on `PATH` (or resolved through `rustup which rust-analyzer`, as today), spawns over stdio, and answers `initialize`.
- Gives the most precise answers: type-position references, trait/interface implementations, re-exports and macro-expanded calls that tags miss.
- Cross-project traversal uses each project's own server; a definition location the server returns inside a linked project's root is mapped to that project and labelled with it.
- A server that starts but crashes or times out mid-session (default per-request timeout 10 s) marks that project's LSP as failed, answers the current request from the next mode and says so in the result (see "Which mode answered" below). It is not respawned on every request (see 6b).
- Indexing on large projects: while the server is still indexing (`$/progress` not finished), requests wait up to the timeout; on timeout they fall back to tree-sitter for that request only and keep LSP as the working mode.

**Mode 2: tree-sitter (fallback).**

- Used when LSP is not configured or not working for that project, and a tree-sitter grammar for the project's languages is compiled into rtok (the existing tags index in SQLite).
- Answers from the existing per-project tags index: definitions, call sites by name, outlines. It indexes the project on first use if needed, with the usual stale banner for pending files.
- Known limits, stated in the result when they matter: name-based resolution (several definitions with the same name are reported as ambiguous), no type-position references, no macro expansion.
- Cross-project traversal joins the tags indexes of every project in the scope by symbol name, preferring a definition in the selected project, then in directly linked projects, then transitively linked ones.

**Mode 3: plain text search (last resort).**

- Used when neither LSP nor a tree-sitter grammar is available for the project (for example a language rtok has no grammar for).
- Searches in process with the crates rtok already uses for its `search` tool (`ignore` for the walk and the ignore rules, `regex` for the matching; T4.5), so no `rg`, `grep` or `ssh` is spawned (D6, D18), with word-boundary patterns built from the symbol name and simple per-language definition patterns (`fn name`, `def name`, `function name`, `class name`, `func name`). Roots are local paths; `ssh://` roots are not supported (ideas.md I-118).
- Answers are best effort: `symbol` returns matching definition lines, `callers` returns lines that mention the name outside its definition, `outline` returns definition-pattern matches in the file, `impact` is limited to one level, and `dead` is not offered (the page and the tool say "not available in text mode" instead of guessing).
- Every text-mode result says it came from text search and may include false positives (comments, strings, same-named symbols). Output is capped the same way as other modes.
- Respects `.gitignore` and the project's ignore settings; never searches outside the project roots in the scope.

**When no mode works.** If all three fail for a project (no server, no grammar, or the root cannot be read), that project is dropped from the answer with one clear line ("project B: no graph backend available: ...") and the other projects still answer. If it is the only project, the tool returns that error.

**Which mode answered.** Every result says which mode answered for each project (page: a small LSP / tree-sitter / text tag next to the project badge; JSON: `backend` per project; text output: one header line). `Measurement` rows keep `kind = "lsp.*"` for LSP and gain `tags.*` and `text.*` kinds, so `rtok stats` shows how often each mode is used.

**Config.** `[plugins.graph] backend = "auto" | "lsp" | "tags" | "text"` (default `tags`), `lsp_timeout_ms = 40000`, and per-language overrides (`[plugins.graph.backend_by_language] go = "tags"`), documented in `docs/config.md` and `docs/lsp.md` (whose "Without the server" section already describes the T376 fallback).

#### 6b. Capability cache: check once; requests never re-probe; the health check re-checks failed records

Decision (2026-10-09, creator, T337): requests never probe; only the §8d background health check re-probes, so a backend-down or unreachable alert can clear without a restart while the request path keeps zero probes.

- The first graph request for a project (and language) runs the capability check: find the LSP marker and server binary and try to start it; check for a tree-sitter grammar; text search runs in process and needs only a readable root. The result is a per-project record such as "LSP works", or "LSP: rust-analyzer not on PATH; tree-sitter works", or "LSP and tree-sitter unavailable; text works".
- Later requests use that record directly: they go straight to the working mode and do not re-probe the modes that failed. No `PATH` lookup, no server spawn attempt and no grammar check runs again on each request.
- The record lives in memory for the hot path and is mirrored into the store (a per-project row, the same pattern as the co-change state), so separate processes (the MCP server, `rtok web`, the CLI, `rtok doctor`) see the same record. A new process reads the mirrored record instead of probing again. A request does not re-probe a failed record; the §8d health check does (below).
- A working mode that later breaks (server crash, repeated timeouts) is downgraded in the record once, and the next mode becomes the chosen one for that project until the health check sees the failed mode work again (two consecutive good checks); it is not re-probed per request.
- Changing `[plugins.graph] backend` or the per-language overrides in config clears the record for the affected projects (the config watcher already reloads settings); config changes and the §8d health check are the only things that update it.
- Adding a new project (manually, by session or by reference) runs the check once for that project only; existing records are untouched.
- `rtok graph projects --json` and the page show each project's record from the store, with `checked_at` and `next_probe_at`, so the user can see why a mode was chosen and when it is checked next.
- Concurrent first requests for the same project share one check (single-flight); they do not spawn several servers.

#### 7. CLI and MCP

- `rtok graph projects` lists projects, `rtok graph projects add <path>`, `remove <id|path>`, `select <id|path>`, `link <id|path>`, `unlink <id|path>`; all support `--json`.
- Every graph command and every graph MCP tool takes an optional `project` (id or path). Without it, the project is the caller's current directory (agents keep today's behaviour) and the scope includes that project's links, so an agent working in A automatically sees into the projects A references.
- MCP results carry the same `project` field per row as the CLI's JSON.

#### 8. Web UI

- Lands on the React SPA graph page (T310.8): selector, indicator, links panel, project badges in every list and in the graph drawing (one colour per project, with a legend).
- New `/ws` messages: project list, selection changed, links changed, per-project index progress.
- If T310.8 has not landed when the backend is ready, ship the registry, links, references, traversal, CLI, MCP and `/ws` first, and the page with T310.8.

#### 8a. Visual graph: projects overview and drill-down into one project

The graph page draws two levels of graph, both interactive (pan, zoom, drag, click), rendered from data sent over `/ws`.

**Level 1: projects overview (the page's landing view).**

- Header counters: total known projects, projects in the current scope, linked pairs, and projects with problems (missing, failed, no backend).
- A node per project, labelled with its name, sized by indexed symbol count, coloured per project (the same colour used for project badges everywhere), with a small backend tag (LSP / tree-sitter / text) and a state marker (indexing, stale, failed, missing).
- An edge per link, drawn as an arrow from the linking project to the linked one. Manual and auto links look different (solid vs dashed); hovering an auto link shows its reason (for example "Cargo path dependency `../ketch-core`"). Edge thickness reflects the number of cross-project references actually found between the two projects; a link with zero references found is drawn thin and grey with a tooltip saying so.
- The selected project is highlighted and its scope (everything reachable through links) is emphasised; projects outside the scope are dimmed but still shown.
- Interactions: click a node to select it as the current project; double-click (or an "Open" button) to drill into it; right-click or a node menu to link, unlink, re-index or remove; a filter box hides projects by name; a toggle shows only the current scope.
- Edge cases: one project only shows a single node and a hint about linking; cycles are drawn normally (no infinite layout); more than about 50 projects switches to a clustered layout grouped by origin, with a list view fallback; missing projects are drawn hollow and cannot be opened.

**Level 2: inside one project (drill-down).**

- Opening a project shows the relationships inside it as a graph: files, modules, types and functions/methods as nodes; "contains", "calls", "implements" and "imports" as edges. A breadcrumb (`All projects / rtok / src/plugins/graph`) leads back up, and the browser back button works (the drill-down state is in the URL).
- It starts at file/module level (files grouped by directory, edges are aggregated call/import counts between files) so a large project stays readable. Clicking a file expands it into its functions, methods and types; clicking a function focuses on it and shows its callers and callees (depth 1 by default, adjustable up to the same limit `impact` uses).
- Calls that leave the project into a linked project end at a node for that project (in its colour); clicking that node opens the target symbol inside the linked project, so the user can follow a call chain across projects visually, matching what cross-project traversal (6) returns.
- A side panel shows the selected node's details: path and line, signature, callers and callees lists, and "open in editor" (the `vscode://` / `file://` link rtok already uses where available).
- Search: typing a symbol name finds it in the project (and the scope) and focuses it on the graph.
- What the graph shows depends on the backend answering for that project (6a): LSP and tree-sitter give full call and containment edges; text mode shows files and definition-pattern matches only, with a banner saying call edges are not available in text mode.
- Large graphs: nodes beyond a cap (default 500 visible) are collapsed into "+N more" groups that expand on click; layout runs in a web worker so the page never freezes; the page shows a spinner while the graph data streams in.
- Live updates: when `watch` is on and files change, the affected nodes and edges update in place over `/ws` without resetting the layout or the user's zoom.
- Edge cases: an unindexed project shows the "Index now" empty state instead of an empty canvas; a project still indexing shows what is indexed so far, marked partial; dead symbols (when available) can be highlighted with a toggle; a file with parse errors is shown with a warning marker and its known nodes.

**Rendering: 3D with Three.js.**

- Both levels are drawn as a 3D graph in WebGL with Three.js. The preferred stack is `3d-force-graph` / `react-force-graph-3d` (Three.js plus a d3-force-3d layout) or `@react-three/fiber` with `@react-three/drei` if more control is needed; pick one in the first PR and record the choice and bundle size in `toolchain.md`. Library versions are pinned like other SPA dependencies.
- Camera: orbit (rotate, pan, zoom) with mouse, trackpad and touch; double-click a node flies the camera to it; a "reset view" button and a "fit all" button; the camera position is kept when data updates live.
- Nodes are spheres (projects) or smaller shapes per kind inside a project (file: cube, type: octahedron, function/method: sphere), coloured per project, with text labels as sprites that face the camera and hide past a zoom distance so the scene stays readable. Edges are lines with arrowheads (or directional particles for calls) and the same solid/dashed and thickness rules as above.
- Layout runs as a 3D force simulation in a web worker; it settles and then stops (no constant CPU use when idle). Expanding a file or project adds nodes near their parent instead of re-laying out the whole scene.
- Selection, hover tooltips, the side panel, search-to-focus and the right-click menu work the same as described above, using Three.js raycasting for picking.
- A 2D toggle shows the same graph flat (same library in 2D mode, or a 2D canvas renderer) for users who prefer it; the choice is remembered.
- Performance targets: 60 fps orbiting with 500 visible nodes and 2,000 edges on a 2020 laptop's integrated GPU; above the visible cap nodes are grouped (as above). Instanced meshes are used for nodes when counts are high.
- Fallbacks and edge cases: no WebGL (blocked, old browser, headless without GPU) switches to the 2D renderer with a notice; a lost WebGL context is restored automatically or falls back to 2D; `prefers-reduced-motion` disables camera fly-to and particle animation; the keyboard list view stays available in 3D mode; the 3D scene is disposed (geometries, materials, renderer) when leaving the page so memory does not grow when switching tabs.
- Tests: Vitest for the data-to-scene mapping (nodes, edges, colours, grouping) without WebGL; Playwright with software WebGL (SwiftShader) checks the canvas renders, a node click selects it, and the no-WebGL path shows the 2D fallback; Storybook stories for both levels with fixture data.

**Accessibility and themes.** Both levels work in dark and light themes at 375 and 1280 px; every graph has a keyboard-navigable list view with the same data (nodes, edges, counts) for screen readers and small screens; colours are not the only signal (shapes and labels carry the same meaning).

#### 8b. Two-part graph UI: interactive explorer and read-only live graph

The graph page is split into two parts that show the same graph data side by side (stacked on narrow screens).

**Part 1: interactive explorer.** Everything described in 8a: the user clicks, selects, drills down, expands, searches, links and unlinks, and moves the camera. Nothing happening in the background moves this view; it changes only when the user acts (or when indexed data changes under `watch`).

**Part 2: live graph (read-only).** The same graph, rendered with the same layout, colours and shapes, but purely for watching:

- No interaction at all: no click, hover menus, selection, drag, expand, search or link actions; no tooltips that need hovering. Pointer and keyboard events on the canvas are ignored, and the cursor stays the default arrow so it never looks clickable.
- The camera is driven automatically: it frames whatever is being queried right now and eases back to an overview when activity stops. The user cannot move it.
- It follows the level shown in part 1 (projects overview, or the project the user drilled into) so both parts show the same part of the graph; the live graph does not drill down by itself. Queries on symbols outside what part 1 shows light up the nearest visible ancestor with a counter.
- It shows only what the graph is being asked right now and how the data changes: every `symbol`, `callers`, `impact`, `explore`, `outline`, `affected` and `dead` call from MCP, the CLI or part 1 itself.

**Layout controls (outside the canvases).** A splitter between the parts (drag to resize, double-click to reset to 50/50), buttons to maximise either part, and a "Hide live graph" toggle; the choice is remembered. On screens narrower than 900 px the parts stack, live graph below, collapsed to its metrics strip until expanded. These controls are the only things the user operates for part 2; the live canvas itself stays read-only.

**What the live graph shows on the canvas.**

- When a call starts, the queried symbol's node (and its project node on the overview) pulses in an "in progress" colour; when it ends, the nodes in the answer (callers, callees, impact chain, explore hits) flash and the traversed edges animate along the path the query took, including edges crossing into linked projects.
- Each running call gets a small floating label next to its node with its tool name and a live counter of symbols returned so far; the label fades a few seconds after the call ends.
- Nodes queried often build up a heat glow that decays over a window (default 5 minutes), so hot spots are visible at a glance.
- Several concurrent calls are shown at once, each in its own accent so their paths can be told apart; more than 8 concurrent calls are merged into one "busy" pulse with a count.

**Live metric displays (read-only, updating in real time).** Arranged as a strip above the live canvas and a feed beside it; every number updates as events arrive, with a short count-up animation (disabled under `prefers-reduced-motion`).

- **Now running:** count of calls in progress, and for each: tool, symbol or query, caller (agent id and host from T283/T284, or "web" / "cli"), project, backend answering (LSP / tree-sitter / text), elapsed time ticking up.
- **Symbols requested:** for the current call, the size of its target set (for example the symbols an `impact` at depth 3 expands to); for the window, the running total. Shown as a number with a sparkline of the last 60 s.
- **Symbols returned:** same layout; the per-call value counts up while the answer streams; the ratio returned/requested is shown as a small bar.
- **Tokens sent / tokens without rtok / saved:** for the last call and for the window: answer size, the size of the unreduced answer (what a plain read or grep of the same data would have returned), and the saving in tokens and percent, shown as a large number with a sparkline. Values come from the same `Measurement` rows `rtok stats` uses, so they match `rtok stats` exactly.
- **Latency:** last call, p50 and p95 for the window, as numbers with a sparkline.
- **Files touched and projects crossed:** per call and window totals.
- **Per-tool breakdown:** a live bar per tool (`callers`, `impact`, ...) with call counts and tokens saved in the window.
- **Backend use:** live shares of LSP / tree-sitter / text answers and the number of fallbacks in the window.
- **Cache and caps:** how many answers were cut by a cap and how many fell back, as live counters.
- **Call feed:** newest first, one row per finished call with tool, symbol, caller, project, backend, requested, returned, tokens saved and latency; failed calls in red with the error; interrupted calls marked as such. The feed scrolls by itself and keeps the last 200 rows; it is read-only like the canvas (no click to replay), but it can be filtered by agent, tool and project with controls above it.
- **Window selector:** totals cover the last 1, 5 or 15 minutes, or "since `rtok web` started"; changing it recomputes from the store, not from what the browser happened to receive.
- **Freeze button:** stops the live canvas and the displays updating so a moment can be read; events keep arriving in the background and the view catches up on unfreeze. Totals never drop events.

**Data path.**

- The graph plugin emits a start event and an end event per call (with the numbers above, and a progress event while a long answer streams) on the existing `/ws` stream. The page subscribes only while part 2 is visible, so a hidden or collapsed live graph costs nothing.
- Events from the MCP server process, CLI runs and `rtok web` all reach the page through the store (or the daemon channel the web UI already uses), so an agent's call in another process shows up within one second.
- Payloads carry ids, symbol names, paths and numbers, never source text.
- Rendering is batched per animation frame; a burst (for example 200 calls per second) is coalesced for display, while counters and totals still count every call.

**Edge cases.**

- No activity yet: the live graph shows the static graph dimmed and "Waiting for graph calls"; the displays show zeros, not blanks.
- A failing call (no backend, timeout) pulses red on its node and appears red in the feed with the error.
- A call still running when its process exits is marked "interrupted" after a timeout and stops counting as running.
- Calls on a project outside the current scope are counted in the totals and listed in the feed (marked "outside scope") but do not light up the canvas.
- `/ws` drops: the live part shows "reconnecting", then resumes; missed events are shown as a count and the totals are refreshed from the store.
- Part 1 drills into a project while calls are running: the live graph switches level with it and re-attaches running calls to the new view.
- Several browser tabs: each live graph receives the stream; closing or hiding them stops the subscription.
- WebGL unavailable: the live graph uses the same 2D fallback as part 1; the metric displays do not depend on WebGL.
- Live events stay local to `rtok web` (localhost by default); nothing leaves the machine.

**Config.** `[plugins.graph] live_heat_window_s = 300`, `live_max_events_per_s = 50` (rendering cap only), `live_feed_rows = 200`, documented in `docs/config.md`.

#### 8c. Export: graph as an image or JSON

- **What can be exported:** the projects overview, the current drill-down view, or a focused subgraph (a symbol with its callers/callees/impact at the chosen depth), from part 1. The live graph (part 2) can export a snapshot of its current frame as an image only.
- **Formats:**
  - PNG at 1x/2x/4x, transparent or theme background, with a legend (project colours, node shapes, edge styles) and a footer (project names, scope, backend per project, `indexed_at`, rtok version, export time).
  - SVG for the 2D rendering (vector, editable); in 3D mode SVG exports the current camera projection flattened to 2D.
  - JSON, versioned schema (`"schema": "rtok.graph.v1"`): `projects` (id, name, root as a path relative to the user's home or redacted, origin, backend, health), `links` (from, to, kind, reason, reference count), `nodes` (id, project, kind, name, path, line), `edges` (from, to, kind), and `meta` (scope, level, focus, depth, filters, export time, rtok version). The schema is documented in `docs/plugins.md` and checked by a JSON Schema file in the repo.
- **Where:** an "Export" menu on the page; `rtok graph export --format png|svg|json [--project ID] [--focus SYMBOL --depth N] [-o FILE]`; and an MCP tool `graph_export` (JSON only) so an agent can hand a graph to another agent or attach it to a PR.
- **Sharing safety:** absolute paths, the home directory and the user name are redacted by default (`--no-redact` to keep them); source text is never included; file names and symbol names are, and the export dialog says so.
- **Edge cases:** an export larger than the visible cap includes every node in JSON but only the visible ones in images, and the image footer says "N nodes hidden"; exporting while indexing marks the export partial in `meta` and the footer; text-mode projects export without call edges and say so; images render offscreen at the requested size, not a screenshot of the window, so the result does not depend on window size; no WebGL means PNG comes from the 2D renderer.
- **Import (read-only):** the page can open an exported JSON to view it (no live data, banner "viewing export from ..."), which is also how diffs against a saved export work (8e).

#### 8d. Alerts: linked project down or unreachable

- **What raises an alert:** a project in the current scope (including auto-linked references) becomes **missing** (root deleted or moved), **unreachable** (a network share stops answering, an external disk is unmounted), **backend down** (its working backend from 6b fails and no fallback works), **index failing** (re-index errors three times in a row), or **link broken** (a manifest reference now points to a path that does not exist).
- **Detection:** the `watch` loop and every graph query update project state; a light background check runs every 60 s (`[plugins.graph] health_check_interval_s`) in every process that hosts graph (the MCP server and `rtok web`), for projects in an open scope. It is the only code that re-probes (requests never do, 6b), and it reads the capability record (6b) from the store. The cheap tier runs every interval: the project root exists (missing, unmounted), the server binary has appeared on `PATH` (path and mtime), a running server is still alive (`try_wait`). A failed or degraded record is restarted (server spawn plus `initialize`) only when the cheap tier sees a change (new binary, root back) or on a backoff that starts at 60 s, doubles, and is capped (the cap is fixed in T329.17). A state must persist for two checks before it alerts, to avoid flapping on a brief unmount.
- **Where alerts show:** a red badge on the project node and link edges in both parts, a toast and an alerts list on the graph page, a line in `rtok doctor`, `rtok graph projects` output (`state` and `alert` fields in `--json`), and a short notice in graph MCP answers that touch an affected project ("project B unreachable since 14:02; results exclude B"). Agents therefore learn about it in the answer they are already reading.
- **Optional push:** if T288 (push unread messages to hooked agents) is available, an alert is delivered once to agents whose current scope includes the project; repeated failures do not repeat the message.
- **Recovery:** when the project comes back, the alert clears automatically after two consecutive good checks (the same two-check rule as raising), the capability record upgrades, a "recovered" entry is logged, and the project is re-indexed if files changed while it was away.
- **Edge cases:** a project removed on purpose from the registry never alerts; unlinking a broken project clears its alert for that scope; an alert on a project that is only transitively linked names the chain ("A to B to C: C missing"); many simultaneous alerts (for example a whole disk unmounted) collapse into one grouped alert.
- **Config:** `[plugins.graph] alerts = true`, `health_check_interval_s = 60`, documented in `docs/config.md`.

#### 8e. Diff: compare the graph before and after a change

- **What can be compared:** the current graph against (a) a git ref (`HEAD~1`, a branch, a commit, the merge base of a PR branch), (b) the working tree versus `HEAD` (uncommitted changes), or (c) a saved export (8c). Diffs work over the whole scope, so a change in B that affects A's call sites shows up in A.
- **How the "before" side is built:** for git refs, rtok indexes the files at that ref from the object database into a temporary index (no checkout, no change to the user's working tree), using the same backend chain (LSP is skipped for the old side when it would need a separate checkout; tree-sitter is used instead and the diff says so).
- **What the diff reports:** symbols added, removed, renamed (same body hash, different name or path), moved between files or projects, and changed (signature or body); call edges added and removed; links added and removed; and, for each changed symbol, its callers that are affected (the `impact` set), which is the part agents need for review.
- **Where:**
  - Page: a "Compare" mode in part 1 colours nodes and edges (added green, removed red, changed amber, moved blue) and lists changes in a side panel; the live graph is unaffected.
  - CLI: `rtok graph diff [--from REF|--from-export FILE] [--to REF|working] [--project ID] [--json]`.
  - MCP: `graph_diff` returning a capped summary (counts, top changed symbols with affected callers) and an id to page through details, so an agent reviewing a PR gets a short answer by default.
- **Edge cases:** a ref that does not exist returns a clear error; a diff spanning projects at different git states diffs each project against its own ref (a `--from` per project is allowed); generated or vendored files follow the project's ignore rules; very large diffs are capped like other answers with a "more" id; renames are detected only when unambiguous, otherwise shown as remove plus add; binary or unparsed files are listed as "changed, not analysed".

#### 8f. Health score per project

- **Score:** 0 to 100 per project, shown as a coloured ring on the project node (green 80+, amber 50 to 79, red below 50) with the breakdown on hover in part 1 and in the project list, and as `health` in `rtok graph projects --json` and MCP answers.
- **Components (weights in brackets, each 0 to 1):**
  - **Index freshness [40%]:** 1 when no files are pending and the last index is newer than the last file change; drops with the share of pending files and with age (0 when more than 20% of files are pending or the index is older than 24 hours with changes since).
  - **Backend alive [30%]:** 1 when the configured backend works (so the default `tags` working scores 1); under `auto` or `lsp`, 1 when LSP works, 0.6 when running on the tree-sitter fallback and 0.3 on the text fallback; 0 when no backend works. Reads the cached capability record (6b) plus recent query failures.
  - **Links not broken [30%]:** the share of the project's links whose target is present, reachable and indexed; a project with no links scores 1 here.
- **Explained, not just a number:** each score comes with the reasons that lowered it ("12 files pending", "rust-analyzer not on PATH, using tree-sitter", "link to ../foo broken"), and a suggested fix for each (re-index, install the server, which the health check picks up within one interval or on restart, fix or remove the link).
- **Scope score:** the selected project's scope shows its lowest project score (the weakest link decides), not an average.
- **Agents:** graph MCP answers include a one-line health note when the scope's score is below 80, so an agent knows when results may be incomplete; `rtok doctor` lists every project under 80 with its reasons.
- **Edge cases:** a project being indexed for the first time shows "indexing" instead of a score; a missing project scores 0 and shows "missing"; text-only languages are not penalised beyond the backend component; scores update live as state changes and are recomputed at most once per second per project.

#### 9. Docs

`docs/plugins.md` (graph section: projects, links, references, scope, backends), `docs/lsp.md` (fallback chain and capability cache) and `docs/config.md` (the new `[plugins.graph]` keys), with `docs/ru/` and `docs/uk/` updated in the same change.

#### 10. Delivery

As PRs, backend first; do not merge them. Each PR becomes a subtask (`Tn.m`, D16) when it is claimed.

Dependencies: T310.8 for the page; T285 and T289 for worktree-based adding; the existing graph index and LSP integration.

Check: fixture repos under `tests/fixtures`, no network:

- Repo A has a Cargo path dependency on B; B has one on C; D is unrelated.
- Indexing A registers B and C (origin `reference`), indexes them and creates auto links A to B and B to C; D is not added.
- Selecting A shows A in the header with "+ 2 linked"; `callers` of a function defined in C returns call sites in A and B, each labelled with its project; `impact` from that function walks up into A.
- `dead` over A's scope does not report B's function that only A calls; selecting B alone does.
- Unlinking B from A removes B and C from A's scope, survives a re-index (no re-link), and `callers` no longer crosses projects.
- A manual link A to D adds D to the scope; a cycle (D links A) does not loop or duplicate rows.
- Removing the path dependency from A's manifest and re-indexing removes the auto link but keeps B in the registry.
- A reference to a missing path shows a warning and adds nothing; a deleted project root shows as missing and drops out of the scope.
- `reference_depth = 1` stops at B; `max_auto_projects` limits are reported on the page and in logs.
- A new agent session in a new directory registers it when `auto_add_projects` is on and not when it is off; `auto_link_references = false` adds no reference projects.
- The selection survives an `rtok web` restart and syncs between two browser tabs.
- MCP `callers` without `project` from A's directory crosses into B and C; with `project` set to D it does not.
- Backends, with `backend = "auto"`: with rust-analyzer on `PATH`, A answers from LSP (result tagged LSP) and finds a type-position reference tags would miss; with it removed from `PATH` and the MCP server restarted, A answers from tree-sitter (tagged tree-sitter); a fixture project in a language with no grammar answers from text search (tagged text, `dead` reported as not available); a scope mixing all three labels each project with its own mode.
- `backend = "lsp"` with no server answers from tags with the T376 `(tags; lsp: <reason>)` header and an `lsp_fallback` row; the default `tags` answers stay byte-identical (`graph_contract.rs`).
- A server that crashes mid-session: the current request is answered from tree-sitter with a notice, and later requests go straight to tree-sitter until the health check sees the server work again.
- Capability cache: a test counts probes; 100 requests to the same project after the first run zero further `PATH` lookups or spawn attempts; installing the server is picked up by the health check within one interval (or by a restart), while requests still run zero probes; changing `backend` in config re-checks only the affected projects; two concurrent first requests run one check.
- Visual graph, level 1: with A, B, C, D the page shows 4 projects, 3 in A's scope and 3 linked pairs (A to B, B to C, A to D); the A-to-B edge is dashed with the Cargo reason on hover, A-to-D is solid; clicking B selects it; a missing project is drawn hollow and cannot be opened.
- Visual graph, level 2: opening A shows its files with aggregated edges; expanding a file shows its functions; focusing the function that calls into C shows the edge ending at a C node, and clicking it opens the target symbol inside C; the breadcrumb and browser back return to the overview; a text-mode project shows the "call edges not available" banner; editing a file with `watch` on updates the node without resetting zoom; a fixture with more than 500 nodes shows "+N more" groups and the page stays responsive.
- 3D: both levels render in Three.js (Playwright with SwiftShader sees a non-empty canvas and can select a node by click); disabling WebGL shows the 2D fallback with a notice; the 2D/3D toggle is remembered across reloads; orbiting the 500-node fixture stays smooth and the layout stops when settled; leaving the page releases the WebGL context.
- Two-part UI: an MCP `callers` call from a separate process lights up the target node in the live graph within one second, animates the path into a linked project and adds a feed row whose symbols requested/returned, tokens and saving equal the matching `Measurement` row and `rtok stats`; part 1's camera and selection do not move; clicking, dragging, hovering and keyboard input on the live canvas change nothing (Playwright asserts no selection or camera change); drilling into a project in part 1 switches the live graph to it; freeze then unfreeze catches up without losing totals; a burst of 500 calls in 5 s keeps both parts responsive and the totals exact; a failing call shows red with its error; dropping and restoring `/ws` shows "reconnecting" and refreshes totals from the store; with the live part hidden, no live events are serialised; on a 375 px screen the live part stacks below as a metrics strip.
- Export: PNG, SVG and JSON exports of A's scope open correctly; the JSON validates against the schema; absolute paths and the user name are redacted by default; a 2,000-node scope exports every node to JSON and the PNG footer notes hidden nodes; `rtok graph export` and MCP `graph_export` produce the same JSON; importing the JSON shows it read-only.
- Alerts: unmounting (or renaming) B's directory raises "B missing" after two checks on the page, in `rtok doctor`, in `rtok graph projects --json` and as a notice in an MCP `callers` answer from A; restoring it clears the alert and re-indexes; a backend-down alert clears after the server is restored, without a restart; a broken manifest path raises "link broken"; unmounting several projects at once shows one grouped alert; a removed project never alerts.
- Diff: changing a function signature in B and running `rtok graph diff --from HEAD` from A reports the change and lists A's affected call sites; the working tree is untouched by building the old side; a rename is reported as a rename; an unknown ref errors clearly; MCP `graph_diff` returns a capped summary with a paging id.
- Health: a fully indexed A with LSP and intact links scores 100; with 30% of files pending it drops below 80 with the reason shown; on tree-sitter fallback under `auto` the backend component reads 0.6, and the default `tags` scores 1; a broken link lowers the links component; the scope shows the lowest score; an MCP answer from a scope under 80 includes the health note.
- Playwright covers the selector, the indicator and its states, link/unlink, project badges, backend tags, both graph levels, export, alerts, compare mode, health rings, 3D and 2D modes, the two-part layout with the read-only live graph and its metric displays, and the list-view fallback; `just check`.

### T329.50. Graph page: running labels with elapsed counters on the live canvas

Left over from T329.32. Each running call gets a small floating label next to its node (2D and 3D) with its tool name and an elapsed counter that ticks up (the metrics list already shows the elapsed seconds); the label fades a few seconds after the call ends. A call folded into "+N more" labels the node that holds the count (T329.32). Calls outside the scope get no label.

Check: a running call shows its label and elapsed counter on its node in 2D and 3D; the label fades after the call ends; no label for a call outside the scope; Vitest, stories (axe) and Playwright; `just check`.

### T521. MiMo Desktop variant of the `mimo` host

Split from T520. No documented config path for MiMo Desktop (early access); a third-party README (https://github.com/akaradje/mimo-desktop, unverified) says Desktop reads `~/.config/mimocode/mimocode.jsonc`. Waits for the creator's run of the read-only probe `scratchpad/mimo-desktop-probe/run.sh`; done = a Desktop variant (detection plus the same config family or its own) with tests.

Check: with a probe result in hand, `rtok agents list` shows a MiMo Desktop variant detected from its app, install writes the config the probe found, tests cover it; `just check`.

### T356. Never index `$HOME` or `/` as a graph root

Found 2026-10-02 (T352 research): `symbols` holds 617,319 rows (~120 MB plus indexes) under the root `/Users/listepo` — `.config/amp/plugins`, `.cursor/extensions`, `.grok/bundled`, `go/pkg/mod`, `.motive/node_modules`. The graph root is the `rtok mcp` process cwd (`std::env::current_dir()` in `src/plugins/graph/mod.rs`), so a server launched in the home directory (no `roots/list` answer yet, or a host without roots) walks the whole home on its first `symbol`/`callers`/`explore` call, and the rows never leave: `delete_symbols_missing` runs only when that same root is re-indexed.

Done when: the graph refuses a root that is the home directory or the filesystem root (an error that says to pass a path or open a project, no walk), and existing `symbols`/`symbol_stale` rows of such roots are dropped once (store retention or a migration). Indexing of real projects is unchanged.

Check: graph tests for both refused roots and an accepted project root; a store test that drops the home-root rows; `symbols` size on a copy of the real store before and after, recorded in this card.

Execution plan (after T352 lands — it adds `drop_vanished_symbol_roots` to retention): reuse `plugins::read::walk_root_ok` (T263: refuses `/` and the home directory for the watcher) in the graph's root choice (`src/plugins/graph/mod.rs`, the `current_dir()` call sites) and return `no project: pass path or open a project (roots)`; extend T352's root drop so a root that fails `walk_root_ok` is dropped like a vanished one. Tests: graph refuses home and `/`, accepts a temp project; retention drops a home-root row.

### T369.1. Measure grep_symbol follow-up rate after an opt-in window

Turn `plugins.guard.grep_symbol` on for a dated window and measure the share of follow-up Grep/Read on the same name within 3 calls (`guard/grep_symbol` rows against the transcripts). At most 25 % means propose default-on to the creator; above it, keep the flag opt-in and record why. Also re-measure the PreToolUse hook p95 on an idle machine (`cargo test --release --test latency -- --test-threads=1`; the flag-on test is `latency_hook_grep_symbol_answer_p95_under_10ms`); on 2026-10-06 the load average was 46 and even the baseline run failed the 10 ms gate. Record the result as a dated row in `research.md` §29.5.

Check: a dated `research.md` §29.5 row with the follow-up share and the idle-machine p95 (under 10 ms); `just check`.

### T370. SessionStart repo map ranked by file-level personalized PageRank

From the Empryo study (idea-only, clean-room; Empryo `repo-map.ts` PageRank and render, `repo-map-utils.ts:479-493`: damping 0.85, 20 iterations, budget 1500 / 2500 / 4000 tokens). T52.3 / I-28 ranks definitions by raw reference count (`symbol_top_refs`, `src/store/symbols.rs:614`), off by default and without a live A/B; `repo_map` (`src/plugins/graph/mod.rs:1090`) pops and re-estimates the rendered text in a loop, quadratic in the number of rows. A file graph (edge = file A references a name defined in file B, weighted by T368's IDF) with PageRank puts hub files first, and a personalization vector moves the map toward what the session is about.

Plan: new `src/plugins/graph/rank.rs` — CSR adjacency from one `symbols` scan, power iteration (0.85, 20 iterations or L1 delta < 1e-6), dangling mass spread uniformly. Global ranks persisted per root in a `file_rank` table (new migration), recomputed at index end; personalized ranks are computed at SessionStart and never persisted (Empryo issue #228: a stored personalized rank leaked one session's focus into the next). Personalization: git-dirty files, the last checkpoint's paths on `compact` (`src/plugins/checkpoint.rs`), and the SessionStart `source` (`crates/rtok-plugin-sdk/src/lib.rs:274`). Render top files with their top definitions; fill the budget in one pass with a running token estimate instead of pop-and-re-estimate. Host trait method in `crates/rtok-plugin-sdk/src/host.rs` and its Runtime impl in `src/plugin.rs`. Config `plugins.graph.map_rank = "refs" | "pagerank"`, default `refs` until the check passes.

Done when: with `map_rank = "pagerank"`, the SessionStart map lists files by rank under `map_tokens`, personalized by dirty files, and a stored rank never contains session state.

Check: unit tests for PageRank on a 4-node graph (known stationary vector, sum = 1 ± 1e-9) and for personalization; offline backtest over the last 200 commits of this repo: recall of the commit's touched files in a `map_tokens = 1000` map, `pagerank` minus `refs` ≥ 15 pp; SessionStart hook p95 ≤ 30 ms on this repo (divan bench); `just check`.

Execution plan (Claude Code / sonnet-5.5; fits one task, no split):
1. `src/plugins/graph/rank.rs`: file graph from one grouped `symbols` scan (edge A to B when A references a name defined in B, weight = refs x ln(1 + files / files-referencing-name) / files-defining-name; T368's shared IDF table replaces it when it lands), power iteration (0.85, 20 iterations or L1 < 1e-6, dangling mass uniform) over CSR, JSON encode/decode of the graph, and a one-pass renderer with a running token estimate. No crate: the maintained graph crates have no personalized PageRank.
2. Migration `0030_file_rank` (`file_rank (root PRIMARY KEY, graph TEXT)`): paths, global ranks, indexed mtimes, top defs per file and the edges, written at index end; a project removal drops it. Three `Symbols` host methods with default bodies (`symbol_file_scan`, `file_rank_get`, `file_rank_put`), Diesel impls in `src/store/symbols.rs`, Runtime in `src/plugin.rs`.
3. SessionStart reads the one row and never spawns a process (D1, T428's 10 ms): personalization seeds are files whose indexed mtime is under 24 h old (only when that set is a working set, 32 files at most; a fresh clone seeds nothing) plus, on `compact`, the paths of the session's last checkpoint (`checkpoint::last_paths`). The personalized rank is computed in memory and never stored.
4. Config `plugins.graph.map_rank = "refs" | "pagerank"` (default `refs`), `default.toml`, `docs/config.md`, trycmd and config_coverage goldens; `repo_map` branches on it, `refs` output unchanged.
5. Tests: 4-node stationary vector, personalization, no session state in the stored row, codec round trip, budget fill, store round trip and purge, SessionStart through the hook. An ignored backtest (`cargo test --lib backtest -- --ignored --nocapture`) reproduces the numbers recorded in `research.md` section 34; a latency test measures the hook with a populated row.

Progress (2026-10-06, Claude Code / sonnet-5.5): steps 1 to 5 are in. Backtest over the last 200 commits, 1000-token map: `refs` 26.7 %, `pagerank` 44.1 %, so +17.4 pp (card asks 15 pp; both halves of the history clear it). Open: the hook latency check. The host ran at a load of 30 to 50, the unmapped SessionStart hook itself missed 10 ms there (p95 13.9 ms), and the pagerank map added about 2 ms at p50 (decode 1.4 ms of a 700 KB stored graph, 20 iterations 0.33 ms). Re-run `cargo test --release --test latency session_start` on a quiet machine; the default stays `refs` until it passes.

### T378. Trigram prefilter for `search` (only if I-95 shows p95 > 200 ms)

From the Empryo study (idea-only, clean-room; Empryo `trigram.ts`: per-file trigram sets, candidate files = intersection of the query's literal trigrams). `search` (`src/plugins/read/search.rs:92`) walks and scans every file. Gate: I-95 (parallel walk) measures `search` p95 on a large repo first; if it is ≤ 200 ms, close this card with the number.

Plan: per-root trigram posting lists (`file_id` bitmaps, `roaring` only with a `Cargo.toml` justification, else sorted `Vec<u32>`) built in the graph index pass for text files ≤ 1 MB; queries with a literal of ≥ 3 bytes scan only candidate files; regexes without a usable literal take the full scan.

Done when: a literal `search` on a 50k-file repo scans only candidate files and returns the same hits.

Check: equivalence test (prefiltered vs full scan) over a fixture; divan bench `search` p95 −50 % on the large repo; index size growth recorded in the card; `just check`.

### T385. Proxy lanes and the optimization plan from `docs/research/optimization.md`

From `research.md` §31 and `docs/research/optimization.md` §6 (2026-10-04): a 12-step plan, "proposal, nothing built", which says to promote items into `plan.md` before coding. `src/proxy/` has no lane code. This card is the spec; split it into sub-tasks (T385.1 …) when claiming, one PR each, in this order:

1. Lane classifier and a `calls.kind` tag (P1).
2. Per-lane policy: bulk and batch bodies stay byte-identical in `compress` mode.
3. Defaults bench for `compress` mode, context editing, skills and `live_blobs`, with a dated `research.md` row (`tools_rewrite` stays in T124).
4. Batch observe, with `parse_results` into `usage`.
5. Flex on bulk and internal lanes, with a 429 policy.
6. Per-lane cache-hit ledger and a replay byte-stability test.
7. Isolation: per-lane upstream and an in-flight cap.
8. P28 Phase 1 measurement.
9. P28 Phase 2 async compressor on the `internal` lane.
10. Routing for internal and bulk calls (I-101's quality gate applies).
11. Deferred tool schemas and thinking replay (I-85, I-86 evidence first).
12. `rtok batch` CLI and a lane breakdown in `rtok report`.

Also: a cross-session read-dedup measurement from `calls` (optimization.md §5, "measure first"). Related ideas: I-84, I-85, I-86, I-101, I-102, I-21. The optimization doc's own ids (T250–T259) collide with `done.md`; use T385.x.

Plan: split 2026-10-04 into T385.1–T385.13 below, one PR each, taken in id order (T385.3, T385.8 and T385.13 are measurements and may run in parallel with the build steps). Boundaries from `docs/batch-flex.md` hold throughout: never convert a live agent turn into a Batch job, never rewrite Batch JSONL, everything lives under `src/proxy/`.

Check: each sub-task carries its own Check; this card closes when every step is done or dropped with its number in `research.md`.

### T385.3. Defaults bench: `compress` mode, context editing, skills, `live_blobs`

optimization.md §5. `rtok bench` cost per passed task for each setting on and off, recorded with a date in `research.md` (`tools_rewrite` is T124). Settings whose row shows a net saving with the pass rate held become default-on in a follow-up; the rest stay off with their number. Branch `t128-proxy-compress-default` (PR #562, `df2a13ba`) is prior art. Needs the creator's API spend for live arms (see T394).

Check: one dated `research.md` row per setting.

### T385.9. P28 Phase 2: async compressor on the `internal` lane

optimization.md §3.3. An LLMLingua-class compressor (arXiv 2403.12968; ACON arXiv 2510.00615 as the agent-context variant) runs on the `internal` lane, asynchronously, cached per archive id; the agent lane never waits for it; the original stays expandable. Only if T385.8 clears Gate P28.

Check: Gate P28 — the must-keep fixture survives, the bench beats v0.1 lossless on cost per passed task; `just check`.

### T394. Run the paid live benches and record them

From `research.md` §3 (coaching nudges A/B) and §2 (A/B bench T9.2, `graph` T68.9): every live arm is unrun ("No live token, cache, or USD rows … the gate is do not enable"; T68.9 "Live API clause remains open"). Gate P9 and P8b clause 4 stay open until they run. Needs the creator's session and API spend: an agent sandbox cannot run them (OAuth expired).

Done means: one dated row per suite in §2 (nudges off/on, T9.2 legacy vs rtok, T68.9 graph MCP vs native) with the command, `claude --version`, mean input/cache/output tokens, `stats --price` cost and pass rate. Nudges become default-on only if cost per passed task falls and the pass rate holds; otherwise the row says "stays off". Gate P9 and P8b clause 4 close or fail on these rows.

Check: the rows are in §2 and `docs/comparison.md`; the gates are marked.

### T395. One real session as one OpenTelemetry trace in SigNoz and Maple

From `research.md` §2 "OpenTelemetry export (Gate P16)": still open is one real Claude Code session (hooks, MCP and proxy) exported as a single trace with an `invoke_agent` root and `chat {model}` spans, checked in SigNoz and Maple. `roadmap.md` says these clauses are Gate P18, but P18 in `done.md` has no such clause, so the item is orphaned. Needs a SigNoz or Maple account.

Done means: the `docs/otel.md` recipe for each backend is run with the proxy in front of a live session; the trace shape and `rtok_tokens_total` are recorded in §2 with a date, or the clause is dropped explicitly with the reason.

Check: dated rows in §2 for both backends, or the dropped clause in `roadmap.md`.

### T396. Verify the usage readers against real files

From `research.md` §30 (T358.3, T358.4): the Kilo and Gemini CLI readers are "not run on real data"; Copilot CLI `inputTokens` vs its `tokenDetails` block disagree and are marked unverified; whether a resumed Copilot session writes a second shutdown with run-only or cumulative totals is unverified (the reader sums them); pi forked-session id copying is unverified. Grok's `signals.json` reader waits on the creator accepting observed keys as a source (§30.7) — ask when claiming.

Done means: each reader runs once on a real file from a machine with that host; the Copilot resume semantics are confirmed or the reader stops summing duplicate shutdowns; each `unverified` tag in §30 is resolved or kept with a date.

Check: a fixture per confirmed shape; §30 updated with dates; `just check`.

### T398. Probe editors on a `worktree.useRelativePaths` worktree

From `research.md` §18.2 (T157): every non-interactive reader opens a relative-link worktree, but VS Code, Zed, Cursor and lazygit are untested, so the setting stays opt-in. It is the setting that would have prevented the 18 GB orphaned `graph-perf` worktree.

Done means: one dated row per editor in §18.2. If all pass, the `worktrees` skill and `AGENTS.md` gain the setting (T157's Check); if any fails, the failure is recorded and the setting stays off.

Check: the §18.2 table has four dated rows.

### T413. More agent hosts: popular agents rtok does not install into yet

Creator request 2026-10-05: add installers for popular coding agents missing from `src/agents/` (22 hosts on `9d6557c7`). One sub-task per host, one PR each. The list below is a lead, not evidence: every fact a sub-task relies on (binary, config paths, MCP entry format, hook events and payloads, plugin or extension API) is first written into `research.md` with its primary source (vendor docs or the repository's own files, URL plus date checked).

Shared shape for every sub-task, per `architecture.md` "New host" and D21:
1. Research: a `research.md` row for the host with the sources above; if a field is not documented, it stays out of the installer (never guessed).
2. `src/agents/<host>/mod.rs` implementing `Agent` (variants, detection, files, installed modules, apply) and `README.md` with the module table and `## Docs`; registered in `HOSTS` and `host()`; `[setup.<host>]` paths in `config/default.toml`, `src/config/mod.rs`, `docs/config.md`.
3. Reuse first: a host that forks or mirrors an existing one (named per card) shares that host's code through a helper, never a copy. Host configs: check only rtok's entry, keep the rest byte-for-byte; backups and `--dry-run` from `rtok-agent-sdk`.
4. `docs/agents.md` row; `agents_install` matrix row (install twice / remove twice / list) against fake homes, never the real agent.

5. Per-host check: the targeted tests above, `agents_doc` and `host_docs` green, `just check` green; one manual `rtok agents install <host> --dry-run` on this machine if the host is installed here, otherwise say "not installed here" in `done.md`.

Check: every sub-task below is closed in `done.md` with its per-host check.

### T413.3. `rtok agents install droid` — Factory Droid

`rtok agents usage` already lists Droid as `unsupported` (`research.md` §30.4). This adds the installer: MCP, and hooks or plugins if Factory documents them.

Check: the T413 per-host check (step 5) for this host.

Plan: primary Factory docs for the MCP entry and any hook file; write only documented keys; `src/agents/droid/` plus the host matrix.

### T413.4. `rtok agents install kiro` — Kiro IDE and CLI

Two variants (IDE and CLI) if both are documented. Hooks and MCP; `research.md` already cites Kiro specs, which this task does not touch.

Check: the T413 per-host check (step 5) for this host.

Plan: vendor docs for IDE and CLI config paths; one variant per documented file; share a helper if the JSON matches an existing host.

### T413.5. `rtok agents install amp` — Amp

Amp (Sourcegraph). MCP plus its plugin system if documented (`~/.config/amp/plugins` exists on this machine, `plan.md` T352 note). A plugin follows the `agents::plugin::HostPlugin` shape.

Check: the T413 per-host check (step 5) for this host.

Plan: Sourcegraph docs for the MCP entry and the plugin directory; `HostPlugin` only if the load path is documented.

### T413.6. `rtok agents install goose` — Goose

Goose (Block), CLI and desktop. MCP-native, so the minimum is the MCP entry; hooks only if documented.

Check: the T413 per-host check (step 5) for this host.

Plan: Block's docs for the MCP config file; hooks only if a payload is documented. MCP-only host via the existing stdio or local helper, whichever the file matches.

### T413.7. `rtok agents install continue` — Continue

VS Code / JetBrains extension and the `cn` CLI. MCP and rules; variants per documented config location.

Check: the T413 per-host check (step 5) for this host.

Plan: Continue docs for each config location; one variant per documented file; no guessed rules path.

### T413.8. `rtok agents install augment` — Augment Code and Auggie CLI

IDE extension and Auggie CLI. MCP and hooks if documented.

Check: the T413 per-host check (step 5) for this host.

Plan: Augment and Auggie docs for MCP and hooks; skip a surface whose file is not documented.

### T413.9. `rtok agents install junie` — JetBrains Junie

Junie in JetBrains IDEs and its CLI if one is documented. MCP first.

Check: the T413 per-host check (step 5) for this host.

Plan: JetBrains docs for the MCP file; a CLI variant only if its config path is documented.

### T413.10. `rtok agents install amazonq` — Amazon Q Developer CLI

Verify first whether Amazon Q Developer CLI is still maintained or replaced by the Kiro CLI; if replaced, close this sub-task into T413.4 and say so in `done.md`.

Check: the T413 per-host check (step 5) for this host.

Plan: AWS docs first. If the CLI is the Kiro CLI, close into T413.4 with the citation. Otherwise an MCP installer for the documented file only.

### T413.11. `rtok agents install crush` — Crush

Crush (Charm), open-source terminal agent. MCP and any documented hooks.

Check: the T413 per-host check (step 5) for this host.

Plan: Charm's docs and the Crush repo for the MCP config; hooks only with a documented payload.

### T413.12. `rtok agents install warp` — Warp

Warp terminal's agent. MCP and rules if they live in a file setup can edit; a setting stored only in the app's database is out of scope.

Check: the T413 per-host check (step 5) for this host.

Plan: Warp docs for a file-based MCP or rules path. If the setting lives only in the app database, close with that finding and no installer writes.

### T413.13. `rtok agents install trae` — Trae

Trae IDE (ByteDance). MCP; check whether its config mirrors the VS Code layout `src/agents/vscode/` already handles.

Check: the T413 per-host check (step 5) for this host.

Plan: Trae docs for the MCP file. Reuse the VS Code helper only if the key and path match; otherwise a thin host of its own.

### T413.14. `rtok agents install openhands` — OpenHands

OpenHands CLI (the local variant only; the cloud service is out of scope). MCP and any documented hooks.

Check: the T413 per-host check (step 5) for this host.

Plan: OpenHands local CLI docs for the MCP file. Cloud config stays out. Hooks only with a documented payload.

### T413.15. `rtok agents install reasonix` — DeepSeek Reasonix

DeepSeek-Reasonix (https://github.com/esengine/DeepSeek-Reasonix), CLI plus editor extension. Its docs list MCP, skills, memory and hooks in `~/.reasonix/config.json` (lead, checked 2026-10-05: https://esengine.github.io/DeepSeek-Reasonix/configuration.html); confirm the keys before writing them.

Check: the T413 per-host check (step 5) for this host.

Plan: re-read configuration.html and write only the keys it names. Skills and memory stay out unless that page says setup may edit them.

### T414. Web dashboard restyle on the brand pack, built from `brand/` sources

Creator request 2026-10-05: a new look for the `rtok web` SPA from the brand pack in `brand/` (tokens, logos, icons, `DESIGN.md`, on top of the `@pyrlyn/brand` base it pins). Creator decisions 2026-10-05: a restyle by the brandbook (pages and navigation stay; shell, panels, tables, KPIs and charts change); a mockup first, approved before the pages; our own `web/src/ui` React components on the brand roles (`--pyr-*`), not the base `.pyr-*` classes. Creator decisions 2026-10-05, later: UI/UX additions (grouped sidebar, command palette and shortcuts, clickable KPIs, live status with pause) and features (Δtok savings trend, table filters in the URL, CSV/JSON export), one sub-task each (T414.8–T414.14). Creator decisions 2026-10-05, later still: charts draw on canvas through ECharts 6, behind our own chart abstraction so the library can be swapped; hover shows a tooltip, and other places change live with the hover where that does not repeat the tooltip; small marks (budget grid, plugin bitset, token mix, share bars) stay DOM and share the same tooltip (T414.15, T414.16). Creator decision 2026-10-05, after the charts: open-source UI toolkits are allowed; behaviour (focus, keyboard, overlays, menus, dialogs, listboxes) comes from React Aria Components, styled by us on the `--pyr-*` roles, so the look stays ours (from T414.9 on; its `Autocomplete` serves the palette, so no `cmdk`).

Source rule: `web/` holds no copy of a brand file. Tokens, fonts, icons, logos and illustrations are imported from `brand/` and `brand/node_modules/@pyrlyn/brand` at build time; anything derived (CSS, raster sizes) is produced by a program in the build, never committed by hand. `brand/README.md` "Known gaps" and `PROVENANCE.md` "Adopting in each surface" list today's copies.

Check: every sub-task below is closed in `done.md`, and no file under `web/` is byte-identical to a file under `brand/` or `brand/node_modules/@pyrlyn/brand/base/`.

### T414.7. Re-shoot `web/screenshots/`; close the web admin gap in `brand/`

Regenerate `web/screenshots/` with the existing script; in `brand/README.md` "Known gaps" and `PROVENANCE.md` "Adopting in each surface" mark the web admin as adopted.

Check: `just check` green; `brand/README.md` no longer says the web admin ships its own copies.

### T416. Shared `change-preview` crate for dry-run output

Every command that changes the disk should preview it the same way, and ketch and cox carry the same need (ketch has its own dry-run paths; cox depends on `similar` and `diffy`). The renderer moves out of `src/render.rs` into a crate with a neutral name in `packages/crates` (`listepo/crates-packages`, tracked there as T1), released to crates.io by that repo's release-plz pipeline; rtok then depends on the crates.io version, because a path outside this repository does not resolve in CI. Blocks T416.1-T416.4.

The crate renders two kinds of change. A file edit (path, before, after) is a unified diff with the `a/` and `b/` headers `render::unified_diff` prints today, or in stat mode a `path | 7 +++--` line; a removal (path, bytes, files) is `- path  12.1 GB  48 213 files`. Both modes end with a totals line: `3 files changed, 12 insertions(+), 4 deletions(-)` for edits, `15 paths, 1 204 311 files, -106.2 GB` for removals. Colour goes through owo-colors' `if_supports_color` exactly as `render::paint` does; byte sizes through the crate's `human_bytes` (the same binary units as `info::human_bytes`); everything derives `Serialize` for `--json`.

Plan: in `packages/crates` (branch `t1-change-preview`), add the crate's project files (`AGENTS.md`, `plan.md`, `todo.md`, `done.md`, `roadmap.md`, `ideas.md`, `toolchain.md`) and the crate as a workspace member, with unit tests for both modes, the totals and the colour switch; `bump.yml` gains a `package` choice and `release.yml` reads the crate from the tag, so the pipeline that releases `file-backup` releases this crate too; one PR there; then `bump.yml -f package=change-preview` publishes it. The crates.io token must allow publishing the new crate name.

Check: `cargo test` and `cargo clippy -- -D warnings` green in `packages/crates`; the crate is on crates.io.

### T428. SessionStart hook back under the 10 ms budget

Renumbered from T418 on 2026-10-06: T418 is the finished `rtok worktree gc` task in `done.md`.

`rtok hook session-start` on the rtok repo took 12.7, 34.2, 13.4 and 65.0 ms on four manual runs (rtok 0.15.1, 2026-10-05) and warned `hook SessionStart slow … over max_ms 10 ms`. The output stayed correct and the exit code 0, but the fail-open rule asks for ≤ 10 ms.

Done: per-plugin timing of SessionStart on a real store shows where the time goes (memory recall, checkpoint restore, agent id, or store open); the slow part is fixed or moved off the hook path; `tests/latency.rs` covers SessionStart with a populated notes store and stays under its p95 bound.

Check: `tests/latency.rs` SessionStart case green; five manual `rtok hook session-start` runs on the rtok repo print no `slow` warning; `just check`.

Execution plan:

1. Time each phase of the hook (store open, `register_agent`, `record_call`, project upsert, each plugin's `session_start`, `inject::apply`, tail) on a copy of the real store (485 MB, 525 notes), idle and with six hooks racing; temporary trace, not committed.
2. Fix what the numbers show: SessionStart does about eleven write commits (against six for PreToolUse) and `memory::recall` reads every note body twice. Queue the three SessionStart measurements and write them in one transaction (`Runtime::defer_measurements` / `flush_measurements`, `Store::insert_measurements_once`); read each body once in `memory/mod.rs`.
3. `tests/latency.rs`: a SessionStart case with 40 notes of ~4k tokens and a `session:*` note.
4. Verify: A/B of the traced binaries, the latency test in release, five manual runs, `just check`.

Status: steps 1-3 done. On the rtok repo SessionStart dispatch is 2-4 ms on an idle host; the `slow` warnings come from write-lock waits and host load, so the fix cuts commits and reads. Left: re-run the `tests/latency.rs` release gate on a quiet host (it fails for every event at load average 35-60 because the spawn floor is already about 9-10 ms) and the five manual runs.

### T436.3. Shared operation-icon crate for rtok and ketch

Split from T436 (2026-10-08), item 5: `OPERATION_ICONS`, `icon()`, `ICON_WIDTH` and the gutter padding are the same code in `apps/ketch/src/ui.rs` and rtok's `src/ui/style.rs` (only ketch's `Tone` vs rtok's `Kind` differs). Extract them into a crate with a neutral name in `packages/crates` (released by that repository's release-plz pipeline, as `change-preview` is in T416), then use it from both. No output change in either tool.

Check: the crate's unit tests (icon per verb, fallback, width); rtok's `src/ui/style.rs` and `tests/ui_style.rs` green on the crate; ketch's own tests green; `just check`.

### T436.4. Spinner on `agents install/update` through T276's `ProgressRunner`

Split from T436.2 (2026-10-08): T436.2 shipped the remaining waits and the operation icons on the `agents install/update/remove` header, but the card's install/update spinner rests on T276's `ProgressRunner`, which does not exist yet; the existing `with_loader("updating host")` stays until then. Depends on T276.

Check: `agents install` and `agents update` show one spinner per host on a TTY and nothing on a pipe (non-TTY test); trycmd snapshots unchanged; the creator's manual run of `rtok agents install` in a terminal.

### T477. TUI: re-index and remove projects (counterpart of the web actions)

T329 plans web actions to re-index ("Index now") and remove a project on the graph page; D27 (amended 2026-10-10, T346) requires the same actions in the TUI. Depends on the T329 subtask that adds those web actions and on T476 (done: the TUI project keys, picker and confirm stage live in `src/tui/projects.rs`, and `web::project_write` runs on the key loop, so the re-index here needs a worker to keep the screen alive). Done means: the TUI graph page has re-index and remove keys that call the same functions as the web actions and the CLI commands, with the same guards: remove shows the plan (what leaves the registry, that no file is deleted) and needs a confirm key; re-index shows progress on the status line and leaves the old data usable while it runs.

Check: a TUI test on a fixture project: re-index brings a stale project to indexed and equals the CLI result, remove drops it from the registry only after the confirm, declining changes nothing; `tests/surface_parity.rs` lists both actions on both surfaces; `just check`.

## Reference

Historical phase notes (P0–P39) live in `done.md`. Companion evidence: `research.md`, `architecture.md`. Per-plugin plan: `roadmap.md`. Unapproved propositions: `ideas.md`.

Claim a `todo` row before work: set Status to `in progress` and Agent to `Provider / model`. Before stopping unfinished work, set Status to `todo` and clear Agent. When the Check passes, move the task entirely to `done.md` (Do/Check + Check result) and drop it from this table, its card, and `todo.md`.

### Decisions (read before any task)

| # | Decision | Why (evidence in `research.md`) |
| --- | --- | --- |
| D1 | **Rust, one static binary, in-tree plugins behind one trait + Cargo features.** No WASM, no subprocess plugins, no daemon **on the hook path in v0.1** (`rtok demon` supervises the long-running surfaces only, D22). A later WASM plugin host is P32 (landed); this repo still does not wrap third-party tools (D6). | Hooks run on every tool call; Rust cold start vs Python hooks. |
| D2 | **One binary, three surfaces:** `rtok hook` (Claude Code hooks), `rtok mcp` (MCP server), `rtok proxy` (`ANTHROPIC_BASE_URL`). | PostToolUse hooks cannot modify tool results. Only PreToolUse rewrite, MCP tool replacement, or a proxy can shrink what the model sees. |
| D3 | **Measurement first.** Nothing ships until `rtok stats` reads real usage from session logs and the proxy. Every plugin logs before/after. Metric = *context-token-turns* (tokens × turns they stay in context), plus output tokens. | Vendor claims vs measured savings; nobody in the stack measures end to end. |
| D4 | **Lossless by default.** Every compression keeps the original retrievable via `rtok expand <id>` / MCP `expand`. Lossy only where the source is regenerable (re-run the command). | Trust is the product. |
| D5 | **Injection budget.** All SessionStart/UserPromptSubmit injections go through one plugin with a per-turn token cap (default 800) and a byte-stable prefix. | Injections are re-read (cached) every turn. |
| D6 | **Every plugin is native, written from scratch in this repo. No third-party plugins.** A plugin never spawns, links, imports, or reads the data of another tool. Third parties extend rtok from outside through the public plugin API (`rtok-plugin-sdk`, `Registry::from_plugins`, `docs/plugin-authoring.md`), never through this repo. | One code path per method is what `Measurement` can attribute. |
| D7 | **Prompt "modes" (terse, YAGNI) are data files, not code.** | Measured effect must be A/B tested, not assumed. |
| D8 | **One SQLite file** (`~/.rtok/rtok.db`, WAL). Schema is D13. Raw archived payloads on disk under `~/.rtok/archive/` (D4); the DB holds indexes and inline JSON under a size cap. | Field tools converge on SQLite (+FTS5). |
| D9 | **Agents are provider-agnostic.** Route by job: low-cost for mechanical work; mid-tier for coding; high-performance for research only after the user confirms. Host names are products, not the implementer. | User constraint: cost-aware routing, any provider. |
| D10 | **Retire, don't stack.** Phase 9 replaces the legacy hook pile with ≤ 8 and drops every tool the A/B bench cannot justify. | Duplicated responsibilities across bash/read/memory/graph tools. |
| D11 | **The proxy speaks both wire formats.** Anthropic Messages and OpenAI Chat Completions / Responses are `Wire` adapters behind one proxy. Hosts point `ANTHROPIC_BASE_URL` or `OPENAI_BASE_URL` at rtok. | One proxy, two parsers is cheaper than two proxies. |
| D12 | **One config file holds every setting; every CLI flag is a config key.** Precedence: defaults < user file < `<git root>/.rtok.toml` < `RTOK_<SECTION>_<KEY>` env < flags. `rtok config show --sources` tells where each value came from. | Hooks, long-running servers, and benches need one precedence rule. |
| D13 | **Core persists through a sync ORM on bundled SQLite.** Diesel (`sqlite` + bundled `libsqlite3-sys` with FTS5). Plugins never write SQL; `Store` is the only DB owner. Hook path: metadata always, body only if under the inline cap — never archive, never fail the hook (D1). | Diesel is sync, so the ≤ 10 ms hook path stays blocking and fail-open. |
| D14 | **CLI is clap 4 (derive); config layers are figment; TOML writes are toml_edit.** Env is `RTOK_<SECTION>_<KEY>` looked up in a leaf table from `Config::default()`. | Clap owns the subcommand tree; Figment tracks per-key provenance. |
| D15 | **Every plugin is designed against alternatives before it is built.** Each catalogue plugin has `src/plugins/<id>/PLAN.md`. | From-scratch only pays if the design beats what it retires. |
| D16 | **One task = one PR.** Each task gets its own branch (or worktree) off `origin/main` and lands through its own pull request; never commit to `main` directly. The PR carries the `<task-id>: <title>` commit and the `plan.md` → `done.md` move. Delete the branch after merge. A task that needs several PRs is split into subtasks `Tn.m`, one PR each; the parent card stays as the epic and moves to `done.md` with its last subtask. | Every change passes CI before it reaches `main`; concurrent agents stop colliding in one checkout. |
| D18 | **The graph index lives in SQLite with the ledgers (D8).** LadybugDB and Grafeo were gated, frozen, then removed (P39). No live `lbug` / `graph-lbug` / `symbols_lbug.rs` / `grafeo` feature flags. SQL for symbols lives only in `src/store/symbols.rs`. D6 holds: no spawned graph tool. | Both graph-store candidates were priced and deleted per the gate. Survey archive: `src/plugins/graph/PLAN.md`. |
| D19 | **Observability is a projection of the ledgers, never a second recorder.** OpenTelemetry export reads existing rows and posts OTLP/HTTP JSON. Nothing runs on the hook path. | Delivery is at-least-once behind a per-stream watermark. |
| D20 | **Local web UI is an operator surface, not a catalogue plugin.** `rtok web` serves axum + the embedded React SPA (`web/`, T310). The SPA is a static bundle, not linked code, and `rtok hook` never loads it. | Pulling a UI stack into `rtok hook` would fail the size/latency gate. |
| D21 | **Every new plugin is plugin and MCP as one unit, a singleton, with one call path per capability.** Host plugins load in that host's desktop app and its CLI. Missing `rtok`: fail open and print that it must be installed with ketch. | Duplicate MCP processes and duplicate call paths break D18 and measurement. |
| D22 | **`rtok demon` supervises rtok's own long-running surfaces**, not a catalogue plugin. Allow-list names (`proxy`, `mcp`, `dashboard`). Nothing in it is on the hook path. | The proxy is the wire hop; when it dies every host silently loses it. |
| D23 | **`rtok tui` and `rtok web` are two renderings of one operator model.** A page that exists on one surface and not the other is a defect. | Two independently built surfaces drift. |
| D24 | **`rtok report` renders; it never computes a number of its own.** It reads the D23 operator model. Recommendations are rules over those rows, never an LLM call. | A saving that is not a `Measurement` row does not exist (D3). |
| D25 | **The plugin contract is `rtok-plugin-sdk`.** Required methods are explicit; event methods keep no-op defaults. `rtok` is the only dispatcher. | One contract, no in-tree shortcut. |
| D26 | **One log with two readers:** a rotating text file and the `logs` table OTel exports. Bounded (`max_bytes` 1 MiB, `files` 5). | An unbounded log on a long-running proxy is a disk-full bug. |
| D27 | **Anything a command prints, or the store keeps, is a page on `rtok web` and `rtok tui`.** Writing actions may appear on both, each calling the same function as its CLI command with the same guards (dry-run plan, then confirm); every write action on one UI surface needs its counterpart on the other (D23 applies to writes too; creator decision 2026-10-10, T346). | The two surfaces plus CLI must not disagree about what a session is, or about what a write does. |
| D28 | **The agent-host contract is `rtok-agent-sdk`.** Installers go through it; host-specific code stays in `src/setup/<host>.rs`. | One write cycle, one plugin-offer body. |
| D29 | **Unit tests prefer a virtual filesystem (`testutil::Vfs`) over host TempDir/std::fs.** Pure path/content/size logic must not require real disk; Windows/macOS quirks are simulated in Vfs. Migrate hottest suites first (read/search/cmd/setup) as T56.x — not a big-bang rewrite of e2e. | Hermetic tests; reproducible CI; path-case and spaced-path bugs (T55) need a simulated FS. |
| D30 | **HTTPS uses webpki Mozilla roots (`use_preconfigured_tls`); one binary.** Corporate CAs via `SSL_CERT_FILE` (curl parity, fail closed). reqwest 0.13 `rustls` still links `rustls-platform-verifier`; `otool` showed Security.framework still present (T53.3). A second hook binary was rejected. | I-32: 1.3–1.5 ms dyld; dropping the `rustls` feature does not compile. |
| D31 | **Claude Code's `WorktreeCreate`/`WorktreeRemove` hooks route through `rtok worktree` and are exempt from the 10 ms rule (T159).** The 10 ms / unmodified-input budget (D1) binds the per-tool-call hot path. These two events fire once per worktree, replace the host's own create/remove, and must spawn git (a fetch and `git worktree add`). Fail open means the host never loses the ability to work: on any rtok error `WorktreeCreate` still prints a path, made by plain git at the host's own default (`<repo>/.claude/worktrees/<name>`, branch `worktree-<name>`); `WorktreeRemove` deletes nothing on error, never forces, and exits non-zero so the host keeps the directory and shows the reason (the host counts exit 0 as removed). Plugin only: `scripts/worktree.sh` carries the fallback, so `settings.json` never gets these entries. | The host's contract (`research.md` §18.3, re-checked 2026-10-02): a create hook that prints no path fails the session, so "print nothing, exit 0" is not available here. |
| D32 | **An optional resident hook process (T178).** `rtok hook --serve` answers `rtok-hook`, a std-only client, over a Unix socket (Windows: a named pipe); `rtok demon` supervises it as the service `hook`, or the hook starts it detached, rate-limited by a lock file. This supersedes D1's "no daemon on the hook path" and D22's "nothing in it is on the hook path" for the `hook` service only. Without it everything works as today: the client runs `rtok hook` when the resident is absent or refuses (another version or config environment), and prints `{}` when it does not answer within 50 ms. | Process start is ~11 ms of the ~14 ms Claude Code waits per hook (research.md §19); a fresh process cannot meet the 10 ms budget. |
| D33 | **rtok's MCP lives in each agent's own config, not in its plugins (T275, amends D21 for MCP).** Install and update always write the config entry `rtok`; only `remove` takes it out, and a plugin no longer suppresses or strips it. Where an agent would show a plugin server next to the config entry (Claude Code and Desktop, Cursor, Copilot, Codex, VS Code, ZCode, Kimi, Grok; `research.md` §25), the rtok plugin ships no MCP server and keeps its hooks, skills and agents. Gemini keeps both, since settings.json wins over an extension's same-name server. Same-name entries across one agent's files are left to the agent to merge. Hooks keep D21 unchanged. `rtok doctor --fix` never removes the `rtok` config entry (T332). |
| D34 | **rtok gives every agent session its own id and owns its worktrees the same way on every host (T281–T290, creator request 2026-09-27).** The agent id is a random UUIDv4 issued by rtok per host session (sub-agents get their own, with a parent), shown as its first 8 hex chars; any unique prefix of 4+ chars is accepted. Not UUIDv7: its leading hex is a timestamp, so agents started within the same minute would share the short id (found 2026-09-27; `started_at` keeps the order). The host's session id is kept alongside but never used as the identity: it collides across hosts and is missing on several (`research.md` §26). A worktree is bound to one agent by the git lock reason `<owner> \| <task-id> \| <date> \| agent <uuid>` (the old 3-field form stays valid) and a store row; the lock is the source of truth. Every host gets the same root, naming, lock, list, remove and gc: Claude Code redirects its own worktrees through `WorktreeCreate`/`WorktreeRemove` (T159), hosts with a post-create script adopt theirs (T289), all others use the skill and the MCP tools. Messages between agents and from the user are local, capped, framed as information from another agent and never as instructions. |
| D35 | **Smart memory is on by default for every agent, from one event module (T291–T293, creator 2026-09-27).** `prompt_recall` (5), `startup_recall`, `handoff` and `spawn_brief` default on. This overrides the T131 gate that would have left `spawn_brief` off until a measured net saving. Bodies stay out of the always-on prompt; the index names `mem_get`. One module under `src/agents/` generates every host manifest from that host's event-name map; a host with no equivalent event is MCP-only for it. Hook path stays sync, ≤10 ms, fail-open, no LLM (D13). No vector read while `[plugins.memory.embed] enabled` is false (the default). T454: when `enabled` and `hybrid` are both true, `prompt_recall` reads vectors already stored (`search_notes_hybrid_stored`) and does not call `embed_stale`. SQLite stays the only store (D8). A later private-repo sync is outside this repo; T294 only makes an export row able to carry a tombstone. Evidence: `research.md` §27. T390 shipped the table (`src/agents/hook_events.rs`) and a manifest drift test in place of generating the manifests. |
| D36 | **Agent junk deletes only what has evidence; history and credentials are never cleared by default (T338, T339).** `rtok agents junk clear` deletes a path only when `research.md` §22 documents it (official docs or source), when it carries a valid `CACHEDIR.TAG`, or when the user names it in `[agents.junk] extra`; platform cache roots and Electron subfolders without a §22 row are listed read-only with their size, never cleared. Credentials and token files are never read for expiry or deleted. Session history and snapshots are `never` for default and `--include review` runs; `--kind sessions` removes a whole session unit only on hosts whose §22 row documents it and its index; `stale_session_days` defaults to 30. Creator approved (C for T338, C for T339, 30 days) on 2026-10-03. | `research.md` §22.1, §22.2: hosts refresh tokens and prune their own sessions (Claude Code, Gemini: 30 days), sessions share trees with memory and indexes, and undocumented paths (Cursor) sit next to chat history. |

### Architecture

```
                    ┌──────────────── rtok (one binary) ────────────────┐
 Claude Code ──hook─┤ hook <event>  → EventBus → plugins (Pre/Post/...) │
 Cursor/OpenCode ───┤ mcp           → tools: read search tree run expand │
                    │                 mem_save mem_search symbol callers │
 ANTHROPIC_BASE_URL ┤ proxy         → usage capture, live-zone rewrite   │──▶ api.anthropic.com
                    │ stats | bench | doctor | setup | run | expand      │
                    └──────────┬─────────────────────┬──────────────────┘
                         ~/.rtok/rtok.db        ~/.rtok/archive/
```

`Ctx` gives every plugin: the DB handle, the token estimator, the archive store, config, session id. `Measurement { plugin, kind, before_bytes, after_bytes, est_before, est_after, ref_id }` is the only way savings enter the DB.

Plugin catalogue (v0.1). Every plugin is native Rust written from scratch here (D6).

| id | spec (replaces; evidence in research.md) | surface | mechanism |
| --- | --- | --- | --- |
| `measure` | rtk gain, headroom savings, lean-ctx gain | `stats`, `bench`, proxy | session JSONL ingest + proxy `usage`; context-token-turns |
| `cmd` | rtk hook, lean-ctx ctx_shell | PreToolUse(Bash) → `rtok run` | archive raw output; per-family formatters + TOML rules; pointer trailer |
| `read` | lean-ctx ctx_read/search/tree | MCP `read`,`search`,`tree` + PreToolUse(Read) | modes via tree-sitter-tags; re-read dedup |
| `archive` | token-optimizer archive_result, headroom CCR | proxy live zone + `expand` | replace old large `tool_result` with pointer + head/tail |
| `proxy` | headroom proxy | `ANTHROPIC_BASE_URL`, `OPENAI_BASE_URL` | passthrough + SSE; usage capture; never touches system, tools, or last 2 turns |
| `inject` | caveman shrink-hook, ponytail/caveman modes | SessionStart, UserPromptSubmit | budgeted, byte-stable prefix; modes as markdown |
| `guard` | token-optimizer refetch_guard | PreToolUse | identical read/command within N turns → deny |
| `memory` | engram, claude-mem | MCP `mem_save/search/get` | agent-written notes, SQLite FTS5 |
| `graph` | codebase-memory-mcp, serena, codegraph | MCP `symbol`,`callers`,`outline` | tree-sitter-tags index in SQLite; optional LSP is T30.2 |
| `toon` (on by default) | caveman toon, TOON | proxy/MCP | tabular JSON → TOON |

### Working agreement

- Graph backends: do not claim tasks that reintroduce `lbug` / `graph-lbug` / `symbols_lbug.rs` / `grafeo` / `graph-grafeo` / cmake-for-liblbug. Symbol index is SQLite only.
- One task = one branch = one PR (D16). Never skip the Check.
- Read `research.md` §3 (hook contract) before any hook task. Hook input is JSON on stdin; output is JSON on stdout; exit 0. Exit 2 blocks (PreToolUse only). PostToolUse can only add context.
- Fail open: any plugin error → log to DB and return the unmodified input/empty output. A hook that crashes must still exit 0 in ≤ 10 ms.
- No new dependency without a one-line justification in the commit message.
- Don't duplicate code or logic: find the existing helper and reuse it, or extract one shared helper at the responsible layer.
- Code style: `cargo fmt`, `cargo clippy -D warnings`, `cargo nextest run` green before every Check.
- Anything unmeasurable is a bug in the plan: add a `Measurement` before adding a feature.
- Every new CLI flag gets a key in `config/default.toml` and a row in `docs/config.md` in the same commit (D12).
- No plugin shells out to, links, imports from, or reads the data of a third-party tool (D6).
- Every new plugin obeys D21.
---

## Review 2026-09-17 — bug hunt (post #37–#47)

Scope: `origin/main` after cross-platform agent fixes #37–#47. Local WIP from other agents was stashed (`preserve-other-agents-wip-before-docs-review-bugs-plan`) and not reviewed. No code fixes in this pass — findings tracked as T55.x.

### Blockers

None for the macOS/Linux happy path on current main. Windows correctness gaps below are P1.

### Should fix

1. ~~**T55.1**~~ — done in #49 (`display_rel` case-insensitive strip).
2. ~~**T55.2**~~ — done in #49 (`never_wrap` case-insensitive stem).
3. ~~**T55.3**~~ — done in #49 (`file_uri` percent-encoding).
4. ~~**T55.4**~~ — done in #49 (host-shell-safe wrap / cmd quoting).
5. ~~**T55.5**~~ — done in #49 (`search_max_bytes`).
6. ~~**T55.6**~~ — done in #49 (`call` in mcp.cmd).

### Nits

1. ~~**T55.7**~~ — done (`skip_word`; quoted `cd` paths bucket by family).
2. ~~**`expand::parse_range` when start > line count**~~ — done 2026-09-23: a start past the last line errors (`start exceeds line count N`) instead of printing nothing with exit 0.
3. **`guard::strip_wrap`** — updated in #49 for PowerShell `''`.
4. **T55.8 / T55.9 / T55.10** — filed from the T55.7 code read: guard `read:` keys survive a mutating Bash, guard Bash key cwd-blind, three copies of `cmd_stem`.

### Residual still open (called out before)

- T55.1–T55.6 closed by #49.
- PATH / single-quote parsers — rtok: T55.7 closed; larger residual in ketch.
- Guard false denies — T55.8 (P2), T55.9; flag-aware read-only classes promoted from I-38 as T57.1.
- Test VFS migration — D29 / T56.x (`Vfs` helper in #49).
- T48.3 still todo: Cursor plugin mcp.json still bare `rtok mcp`.

### Second pass (2026-09-17, later the same day)

Scope: hook dispatcher/types, guard, cmd (hook/run/rules/formatters), read (mod/cache/hook/search), archive, proxy (mod/wire/anthropic/openai_chat/semantic_cache), expand, store (archive/decisions/read_cache paths), mcp session handling. Four findings reproduced with a scratch integration test (written, run, deleted — `cargo nextest run --test zz_review_repro` → 4 failed exactly as predicted); two filed from code read. No code fixes in this pass — findings tracked as T55.11–T55.16, propositions as I-39/I-40.

- ~~**T55.11 (P2)**~~ — done: `expand` never froze the owning session's pointer (every expand caller runs under session `expand` / `mcp-<pid>`, decisions belong to the proxy session; expand rate stayed 0, toon attribution dead). Reproduced.
- **T55.12 (P2)** — Windows `wrap_quote` `''` quoting is wrong under Git Bash (Claude Code's Windows shell): apostrophes silently dropped from the rewritten command. Code read.
- ~~**T55.13 (P3)**~~ — done: Copilot `Read` inputs use `path`; `guard::cache_key` and `read::hook` match `file_path` only → dedup and advice never fire on that host. Reproduced.
- ~~**T55.14 (P3)**~~ — done: the semantic-cache key ignored tool_result text; two requests differing only in tool results hashed identically and were both eligible under defaults. Reproduced.
- **T55.15 (P3)** — `live_blobs` overwrites image/document `source.data` / `image_url.url` with pointer text → invalid request when the flag is on. Reproduced.
- ~~**T55.16 (P3)**~~ — done: the guard deny read the full archived body (and estimated over it) on the ≤ 10 ms PreToolUse path. Code read.

### Out of scope this pass

Concurrent agent WIP on local `main` (stashed as `preserve-other-agents-wip-before-docs-review-bugs-plan`). Half-finished T48–T53 cards on that WIP were not judged as shipped bugs.

---

## Note 2026-09-17 — testing library candidates

Shared catalog: [`listepo/rust.md`](../../rust.md) → *Testing candidates*.
Catalog only — no blanket `Cargo.toml` adds (Working agreement: justify each dep).

Fits for rtok (1–3):

1. `vfs` (crates.io) — evaluate against in-house T56 `src/testutil.rs` `Vfs`
   before adopting; mandate stays "prefer VFS over host TempDir".
2. `mockall` — trait mocks for provider / plugin host seams when hand fakes
   get noisy (`httpmock` stays for HTTP).
3. `tokio-test` — async unit helpers beyond `#[tokio::test]` where time/task
   control matters.

Already covered: `assert_cmd`, `divan`, `httpmock`, `insta`, `rstest`,
`trycmd`, `similar`. Skip `test-case` / `expect-test` / `mockito` duplicates;
`testcontainers` / `bolero`/`honggfuzz` only if a measured e2e/fuzz gap appears.

---

## Note 2026-10-02 — Empryo-derived tasks (T368–T378)

Source: study of [proxysoul/Empryo](https://github.com/proxysoul/Empryo) (formerly SoulForge) at `669ff91`. **Idea-only, clean-room: Empryo is BSL 1.1, no code copied.** Every card cites Empryo only for the idea; implementations are written from the card.

Take first, in order: T369, T370, T372, T373, T374. Then T375, T377, T378 (gated on I-95). T368, T371 and T376 are done.

The 14 portable ideas and where each landed:

1. PageRank repo map — T370 (extends T52.3 / I-28).
2. Edge confidence / name IDF — T368.
3. Git co-change — T371 (done).
4. Blast radius under a budget — T377.
5. Trigram index — T378 (gated on I-95).
6. Clone detection (MinHash) — not now: T52.4 / I-29 (dead code) has no measured question that duplicates answer, and Empryo's own perf PR #220 shows the cold-scan cost; revisit after T68.9.
7. Grep → symbol intercept — T369 (builds on I-08 / T50.4).
8. Post-edit diagnostics delta — does not fit: rtok does not run edits; it is the agent loop's job (cox).
9. Backend fallback / LSP hygiene — T376.
10. Compound tools (rename, project check) — does not fit: they need the agent's edit loop and approvals; rtok stays read/measure.
11. Deterministic compaction state — T375 (checkpoint); LLM compaction itself is the host's.
12. Memory RRF / file affinity — T373, T374.
13. Edit robustness (fuzzy `old_string`) — does not fit now: I-43 / T58.3 measured 1.3 % `old_string` misses; below the gate.
14. Shell output compress / tee — already covered by the `cmd` rules (`src/plugins/cmd/rules.rs`) and the archive (`expand <id>`).

### T561. TUI: folded-target counter in a one-project view

Left over from T500. `CallsStore::folded_counters(drawn, more, scope, now)` and the pane's "folded calls" lines (`{label} · {n} folded`) exist, but the TUI has no one-project drill view, so nothing fills `LiveCalls.drawn`/`more` and the counter never shows. Give the TUI a one-project view that supplies the drawn nodes and `more`, then wire it in and drop the `#[cfg(test)]` `set_drawn`.

Check: in the one-project view the pane lists folded targets on their nearest drawn ancestor, else on the "+N more" group; TUI test; `tests/surface_parity.rs`; `just check`.
