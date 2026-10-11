# Token-reduction tools for AI coding agents — research, comparison, evidence

Date: 2026-09-01. Method: 8 Haiku research agents (compression tools, code graphs, memory, host surfaces, architecture, techniques, token-optimizer plugin, session-log measurement) + 2 Haiku adversarial fact-check agents (GitHub API metadata for 19 repos; 27 documentation/blog claims). Local ground truth from `~/.claude/settings.json`, `rtk gain`, `headroom savings`, `lean-ctx gain`, `caveman status`, claude-mem banner, and 17 session transcripts (Jul 29 – Sep 1). Raw reports live in the session scratchpad `token-research/`. Features those tools have that rtok has not scheduled live in `ideas.md`.

## 1. Verdict

1. **Your stack stacks ten tools, 81 hooks and two chained proxies, and nobody measures end to end.** Vendor claims are 60–99 %; your own meters show 3–40 % on the slice each tool touches; the only independent measurement (JetBrains) found rtk cost +7.6 % to 0 % and caveman 8.5 % on agentic work.
2. **The cost lever is context × turns, not single outputs.** Cache hit rate is 98.1 %; every token that enters context is re-read (at 0.1× price, 0.025× on Fable/Mythos 5.1) on every later turn. A 17 K-token Read on turn 10 of 60 costs ~1.25 × 17 K to write and 0.1 × 17 K × 50 to keep. Shrinking early and clearing old results beats shrinking a little everywhere.
3. **Build one Rust binary with plugins, measurement first, and retire what does not pay.** Hooks cannot modify tool results after the fact, so the binary needs three surfaces: hook (PreToolUse rewrite), MCP (tool replacement), proxy (cache-safe live-zone rewriting + ground-truth usage). Plan: `plan.md` (project `~/GitHub/rtok`).

## 2. Where your tokens go (17 sessions, 43,609 transcript lines)

Estimator: 4 chars/token (heuristic). Usage counters are real API numbers.

| Item | Value |
|------|-------|
| Tool results, est. tokens | 2.83 M total; Bash 1.00 M (35 %), Read 0.41 M (15 %), Agent 23 K, MCP engram 8.8 K, MCP lean-ctx 6.2 K |
| Largest single results | Reads of 38–68 K chars (9.5–17 K tokens each, top 8 all `Read`) |
| Bash by family | `cd …&&` chains 435 K (hides the real command), sed 217 K, grep 131 K, cat 17 K, ls 13 K, pnpm 35 K, python3 34 K |
| Assistant output | 8.6 M output tokens; text is 4 % of assistant content, 96 % is tool input (the code it writes) |
| Read delta re-reads (T58.1, 2026-09-18, `rtok stats --since 90d`, 959 sessions) | 593 native `Read` calls of a path already read in the same session with an Edit/Write/MultiEdit of that path in between; 1.79 MB of 24.61 MB Read result bytes (**7.3 %**) — above the 3 % gate, so MCP `read` returns a unified diff against the archived previous content. |
| Content-hash repeats (T65.1, 2026-09-18, `rtok stats --since 30d`, 924 sessions) | 6,649 later tool_results whose SHA-256 equalled an earlier result in the same session; 1.83 MB of 95.25 MB result bytes (**1.9 %**) — above the 1 % gate, so `cmd::run` and `read` return a pointer at the earlier archive instead of the body. |
| Extra `read` modes (T50.3, 2026-09-18, this repo's 38–68 K char sources) | 11 Rust files, 534 894 B → tree-sitter comments-stripped 433 997 B (**18.9 %**, ~25 K est. tokens) with function/type bodies kept. imports-only is 0.1–2.1 % of each file and drops those bodies. `app.slint` (38 064 B, no grammar) stays `full`. |
| Edit `old_string` (T58.3, 2026-09-17, `rtok stats --since 90d`, 925 sessions) | 5,990 Edit/MultiEdit calls; `old_string` 1.61 MB, `new_string` 3.60 MB; `old_string` = 3.8 % of tool-input bytes, ≈ 1.3 % of output tokens (bytes/4 against API output) — under the 10 % gate, so the anchored `patch` tool (T58.4) was not built. Caveat: this machine already routes many edits through lean-ctx `ctx_patch`, so the share is a lower bound for a plain-`Edit` workload. |
| Compactions (T58.2, 2026-09-18, `rtok stats --since 30d`) | 923 transcript sessions, 271 `subtype=compact_boundary` events (75 sessions compacted at least once). T2.5 fixture checkpoint body = 144 B before archive-id lines; `plugins.memory.checkpoint_tokens` = 400. Command: `rtok stats --since 30d` (header `sessions N  compact N`); detector is `compact_boundary` only so the paired `isCompactSummary` line is not double-counted. |
| Sub-agents Agent/Task (T59.6, 2026-09-18, `rtok stats --since 30d`, 939 sessions) | `rtok stats --since 30d` (2026-09-18): 42 of 939 sessions used `Agent` (449 calls); `Task` 0. Agent in 1,042,386 B / out 632,586 B (158,299 est. tokens); table line `result/tool_tokens 0.7%  input/tool_input 2.7%` (JSON 0.662 % of 23,905,777 tool-result tokens; 2.699 % of 38,626,178 tool-input bytes). Under the 5 % gate, so the `handoff` MCP tool was not built. |
| Assistant thinking blocks (T125, 2026-09-21, `rtok stats --since 30d`, 827 sessions) | 28,072 `thinking`/`redacted_thinking` blocks (unique `message.id`), 7,266,318 B ≈ 1,816,580 est. tokens = **0.0297 %** of session input (uncached + cache_create + cache_read) — two orders of magnitude under the 3 % gate. The replay share is lower still: the Anthropic API strips thinking blocks of earlier assistant turns from the context (https://docs.claude.com/en/docs/build-with-claude/extended-thinking). I-86 rejected. |
| Session checkpoints (T71.2, 2026-09-18, `rtok stats --since 30d`) | 939 transcript sessions, 0 with a `checkpoint:<id>` or `session:<id>` note, 939 without. Store has 25 legacy `kind=checkpoint` rows (no session suffix, not joinable to a stem). Header `sessions N  compact N  checkpoint N  no_checkpoint N`. Command: `rtok stats --since 30d`. |
| Foreign MCP results (T59.4, 2026-09-17, `rtok stats --since 30d`, 885 sessions) | `rtok stats --since 30d` (2026-09-17, 885 sessions), `mcp` table: lean-ctx 8,232 calls, 19.66 MB result bytes, mean 2.4 KB, p95 45.7 KB (≈ 4.9 M est. tokens) — ≈ 27 % of the 71.8 MB in the tool table; rtok 579 KB, engram 279 KB, t3-code 73 KB (mean 18 KB), Claude_Browser 45 KB. lean-ctx is above the 5 % gate, so the wrapper is justified for this workload; caveat: lean-ctx already compresses its own results, so the win is in the p95 tail, not the mean. |
| MCP description tokens × turns (T59.5, 2026-09-18, host without Tool Search) | `rtok doctor` (2026-09-18): 11 MCP servers, 8,951 description tokens (caveman 485, code-review-graph 2,295, engram 1,865, mobile 1,555, serena 1,494, lean-ctx 697, headroom 161, rtok 143+143, jscpd 113, codebase-memory-mcp 0); `mcp_tool_search likely disabled` because `ANTHROPIC_BASE_URL` is set. Transcripts `~/.claude/projects/**/*.jsonl` mtime ≥ 30 d, unique `message.id` (same rule as `measure::jsonl`): 936 sessions, 40,402 API turns, session input 5.834 B (uncached 0.670 M + cache_create 147.1 M + cache_read 5.686 B). 8,951 × 40,402 / 5.834 B = **6.2 %** of session input. Above the 3 % gate → `proxy.tools_rewrite` ships, off by default. Native-tool descriptions are extra, so 6.2 % is a lower bound. |
| Cache | read 1,367 M, creation 26.7 M, uncached input 42 K → 98.1 % hit rate |
| Median final context | 167 K tokens per session |
| rtk-wrapped commands visible in transcripts | 3 of 3,658 (the PreToolUse rewrite happens after the transcript records the call, so this under-counts) |
| Replay bench: saving over a session mix (T241, 2026-09-24, `cargo test --test replay_bench -- --nocapture`) | `tests/fixtures/replay/session.jsonl`: 30 hand-written events (23 Bash cargo/git/grep/ls/npm/pytest-style + 5 MCP `read` + 2 `search`), tool-mix shares from `rtok stats --since 7d --json` on this machine (129 sessions, 21,046 tool_use calls: Bash 64.7 %, Read 7.3 %; MCP share raised to cover `read`'s `stripped`/`dedup` paths — see the fixture's header comment). Replayed through the real hook/run/mcp surfaces: `cmd` 23 calls, 23,075 → 3,648 est. tokens (**84.2 %**), `read` 5 calls, 2,482 → 1,183 (**52.3 %**), total 25,557 → 4,831 (**81.1 %**) — `tests/replay_bench.rs`'s floor is 76 %. `search` records no row on these two calls: since T300 (2026-09-28) `search` records a `search_cap` row only when `max_chars` cuts its output, and neither fixture search is cut (was: never called `cx.record`). The first run left `emit_filtered`'s `[rtok <id> · N lines · expand …]` trailer out of `after_bytes`/`est_after`, so `cmd`'s 84.2 % was an upper bound; T247 (`bc33fd13b`, 2026-09-24) counts it, which is the 82.7 % below. Mutation check: disabling `[plugins.cmd]` drops the total to `read`-only 52.3 %, below the floor, confirming the floor is load-bearing. Rerun 2026-09-25 on main (0bb62f4f): `cmd` 23,075 → 3,994 (**82.7 %**), `read` unchanged, total 25,557 → 5,177 (**79.7 %**) — still above the floor. Rerun 2026-10-08 (T397) on `1d6961541`, debug test profile, `RTOK_HOST_SANDBOX=1`, three runs, identical each time: `cmd` 23 calls 23,075 → 3,994 (**82.7 %**), `read` 5 calls 2,482 → 1,183 (**52.3 %**), total 28 calls 25,557 → 5,177 (**79.7 %**); the 2026-09-25 numbers still hold, so nothing after T247 and T300 moved this fixture. |

### `guard` read-only stems (T57.1, 2026-09-18)

Dated command: `measure::stats::collect` walk (`jsonl_paths` over `[stats] transcripts_dir`, default `~/.claude/projects`, 968 `*.jsonl`). Repeat = the same Bash `command` string (whitespace-collapsed) inside `plugins.guard.window_turns` (default 8) transcript turns. First-word family is `bash_family` (the `collect` path). Flag/subcommand counts are a token scan of those commands, not a shell grammar.

| Stem / marker | All Bash | Exact repeats in window | Action |
|---------------|----------|-------------------------|--------|
| `sed` | 2,417 | 5 | **add** (`sed -n` 2,238; `sed -i` 169 stay mutating) |
| `jq` | 20 | 0 | **add** (repeats are path/filter variants; the stem is present) |
| `awk` | 359 | 0 | **add** |
| `git rev-parse` | 8 | 0 | **add** (`git` all 1,370 / repeats 53, mostly `status` 285) |
| `cargo metadata` | 9 | 0 | **add** (`cargo` all 216 / repeats 2, none metadata) |
| `ls` `cat` `head` `tail` `grep` `rg` `find` `wc` | 1,072 / 1,793 / 147 / 308 / 3,646 / 28 / 343 / 206 | 3 / 9 / 0 / 3 / 8 / 0 / 0 / 5 | keep |
| `find -delete` / `-exec` | 0 / 12 | — | writer marker |
| `tail -f` | 8 | — | writer marker |
| `\| tee` | 44 | — | writer marker |
| redirect `>` / `>>` (token) | 13,802 | — | writer marker (greedy: fail-open vs false deny) |

No existing stem was removed: each still has a count. `tree` is 0 on this machine and stays in the list (already shipped).

Hook e2e fixture (one session, `window_turns = 64` so the fixture is not capped; PostToolUse then PreToolUse of each command, plus `ls` → `find . -delete` → `ls`). `Measurement` rows `plugin = guard`, `kind = guard`:

| | count | `before_bytes` sum |
|---|---|---|
| before (stem list, flags ignored) | 9 | 52 |
| after (flag-aware) | 8 | 32 |

Before denied writers (`find -delete`, `cat a > b`, `grep x > out`, `ls \| xargs rm`, `tail -f`) and false-denied the follow-up `ls`. After those take the mutating path (0 denies) and the new read-only repeats (`sed -n`, `jq`, `awk`, `git rev-parse`, `cargo metadata`) deny. `cat a \| grep b` and `cat f \| wc -l` deny in both. `sed -i` is never keyed.

### Extra `read` modes (T50.3, 2026-09-18)

Dated command: tree-sitter comment-node strip (newlines kept) and import-kind bytes on this
repo's files in the 38–68 K char Read-tail class (`research.md` §2), worktree `t50.3` based
on `t65.1`, 2026-09-18. Estimator 4 chars/token.

| File | full B | stripped B | save | bodies kept | imports remaining |
|------|--------|------------|------|-------------|-------------------|
| tests/proxy.rs | 59 352 | 54 433 | 8.3 % | yes | 0.4 % |
| src/agents/mod.rs | 55 932 | 44 869 | 19.8 % | yes | 0.6 % |
| src/config/mod.rs | 52 035 | 39 522 | 24.0 % | yes | 0.5 % |
| src/plugins/graph/mod.rs | 51 030 | 41 104 | 19.5 % | yes | 0.7 % |
| src/doctor.rs | 48 890 | 41 590 | 14.9 % | yes | 0.8 % |
| src/measure/stats.rs | 48 369 | 41 227 | 14.8 % | yes | 0.8 % |
| src/web/model.rs | 46 958 | 33 749 | 28.1 % | yes | 0.9 % |
| src/cli.rs | 44 534 | 34 402 | 22.8 % | yes | 1.1 % |
| src/tui/view.rs | 43 277 | 34 122 | 21.2 % | yes | 1.2 % |
| src/proxy/mod.rs | 43 052 | 33 219 | 22.8 % | yes | 2.1 % |
| src/plugins/cmd/rules.rs | 41 465 | 35 760 | 13.8 % | yes | 0.1 % |
| crates/rtok-webui/ui/app.slint | 38 064 | 38 064 | 0 % (no grammar → `full`) | n/a | 0 % |
| **11 Rust files** | **534 894** | **433 997** | **18.9 %** | **yes** | 0.1–2.1 % |

Verdict: `mode=stripped` wins; imports-only skipped (loses the bodies a Read of these files is for).

### Whole-file native Reads an outline could answer (T136, 2026-09-24)

`rtok stats --since 30d` on this machine (367 sessions, 423 630 transcript lines), `read_whole` row: native `Read` with no `offset`/`limit`, of a file `read::outline::supported` has a grammar for, result ≥ `[plugins.read] native_max_bytes` (32 768 B).

| Metric | Value |
| --- | --- |
| Calls | 6 |
| Bytes | 216 364 |
| Share of Read bytes (14 150 009) | 1.5 % |
| Share of all tool-result bytes | 0.3 % |
| Followed by an Edit of the path within `[plugins.guard] window_turns` (8) | 0 |

Under the 5 % gate: I-82 (deny such Reads, point at `outline`) is rejected with this number. The `read` plugin's advice deny (T4.6) already turns most such Reads away before they produce a result; these six got through.

### Image blocks in the live zone (T137, 2026-09-24)

`rtok stats --since 30d` on this machine (367 sessions, 424 122 transcript lines), `images` row: every `image` content block in a tool result or a user message. Bytes are decoded base64; pixel size comes from the PNG `IHDR` / JPEG `SOF` header; tokens are `⌈w/28⌉ × ⌈h/28⌉` at the high-resolution tier (long edge 2576 px, 4784-token cap), per the Anthropic vision docs ("Resolution and token cost", read 2026-09-24). `resident` multiplies each block's tokens by the API requests at or after its turn — an upper bound, since compaction drops images earlier.

| Source | Blocks | Bytes | Est. tokens |
| --- | --- | --- | --- |
| `Read` of an image file | 182 | 21 258 233 | 159 978 |
| Browser pane (`browser_batch`, `computer`) | 23 | 674 729 | 11 610 |
| iOS Simulator `control` | 3 | 606 872 | 7 128 |
| Other MCP screenshots (2 tools) | 2 | 234 584 | 2 036 |
| Pasted into a prompt | 1 | 243 375 | 370 |
| **Total** | **211** | **23 017 793** | **181 122** |

Resident: 56 020 282 tokens, **0.91 %** of session input; every block had a PNG or JPEG header. Under the 5 % gate: the multimodal token gate (§16.3 #9) stays unbuilt, with this number. Most image tokens come from the agent's own `Read` of image files, not from browser or simulator screenshots.

### Repeat native Reads by class (T179, 2026-09-24)

`rtok stats --since 30d --json` on this machine (367 sessions, 426 092 transcript lines), the new `repeat_reads` row: every native `Read` of a path already read this session (or, for `subagent`, by its parent/an earlier sibling — T127), not the first read of it, classified by why `read/dedup`+`read/delta` (the MCP `read` cache) did not shorten it. Priority order: a result already carrying a dedup/delta/archive marker → `fired`; else an Edit/Write/MultiEdit/Bash that *changed* the path since the last read → `changed`; else `offset`/`limit` differing from the last read → `ranged`; else, if nothing in the session carries any rtok marker → `hook_absent` (T174: `rtok` likely not on `PATH`); else → `declined`. `subagent` is a sub-agent's re-read of a path its parent or an earlier sibling already read — invisible to the per-session walk, so it is filled from `Subagents::reread_calls`/`reread_bytes` (T128) instead.

`changed`'s Bash signal was revised after the first pass: naming the path in a Bash command was not enough (`cat`/`grep`/`sed -n`/`head` of the path are reads and are common), so `bash_touches` now requires a write signal — output redirection, `sed -i`/`perl -i`/`-pi`, `tee`, `mv`/`cp`/`rm`/`touch`/`patch` naming the path, or a "blanket writer" (`git checkout`/`restore`/`stash`/`reset`/`rebase`/`merge`/`pull`/`apply`/`cherry-pick`, `cargo fmt`/`rustfmt`/`oxfmt`/`prettier --write`) that can rewrite any tracked file without naming it. The numbers below are the re-measurement under that narrower rule.

| Class | Calls | Share of calls | Bytes | Share of bytes |
| --- | --- | --- | --- | --- |
| `subagent` (d) | 1 017 | 41.0 % | 5 909 394 | 58.2 % |
| `changed` (b) | 732 | 29.5 % | 2 141 409 | 21.1 % |
| `ranged` (c) | 524 | 21.1 % | 1 976 031 | 19.5 % |
| `hook_absent` (a) | 111 | 4.5 % | 53 192 | 0.5 % |
| `fired` (e) | 55 | 2.2 % | 6 900 | 0.1 % |
| `declined` (f) | 44 | 1.8 % | 58 386 | 0.6 % |
| **Total** | **2 483** | **100 %** | **10 145 312** | **100 %** |

Same total calls and bytes as the first pass — only reclassification moved: 118 reads that the old, name-only `bash_touches` had called `changed` are no longer flagged as touched by a plain read-only Bash, and fall through to `ranged` (+119) or, in a few cases, `hook_absent`/`declined`; `fired` and `subagent`, untouched by the Bash signal, are unchanged. The 2026-09-22 ad hoc audit's "367 same-session re-reads / ~272 dedup+delta fires" was a narrower count (no sub-agent transcripts, and evidently a different window); this run's per-session-file walk alone (everything but `subagent`) still finds 1 466 repeats, of which only 55 (`fired`) show a dedup/delta/archive marker — consistent with that audit's headline gap even though the totals do not match exactly.

Largest class: `subagent` (d), 1 017 calls / 5.9 MB — a sub-agent re-reading a path its parent or an earlier sibling already read. Per plan, not fixed here: it is class (d), T127's territory (attribution/sharing across agent transcripts, not a `read` plugin bug), and any fix (e.g. seeding a sub-agent's dedup cache from its parent's reads) is a design decision — cross-process cache sharing, and whether a sub-agent *should* trust its parent's read of a path it cannot verify is still current — not a small, clear patch inside `src/plugins/read/**`.

Largest non-(d) class both before and after the narrower `bash_touches`: `changed` (b), now 732 calls / 2.1 MB (was 850 / 2.6 MB) — still expected behaviour by construction (an Edit/Write/MultiEdit/Bash write touched the path since the last read), not a miss to fix. `ranged` (c) likewise: a different `offset`/`limit` is a different read. `hook_absent` (a) is T174's territory (`rtok` not on `PATH`), already tracked there. The only class that is a same-session, same-path, same-range repeat the cache should have caught and did not — `declined` (f) — is also the smallest both times (44 calls now, 0.6 % of bytes): too small to justify a code change, and `--json`-scale transcript text alone cannot show *why* the MCP `read` cache missed these 44 (a native `Read` never even consults it; these are native Reads that stayed native through the read-advice gate — under `native_max_bytes`, or within the recently-edited early return of `plugins/read/hook.rs::pre_tool` that returns `None` with no marker). The class ordering did not change under the narrower Bash signal, so the fix decision stands: no class here is both largest and a small, clear `read`-plugin fix, so none was made.

### `graph` index accuracy (T8.8, 2026-09-04)

30 symbols of this repo labelled by hand from a plain-text scan, independent of the index that is
being scored: definition files complete per symbol, reference files a must-appear subset, no line
numbers. `tests/graph_truth.rs` re-measures on every run.

| Metric | 2026-09-04 (T8.8) | 2026-10-04 (T387, after T52.5 re-land) |
|--------|-------|-------|
| Definitions found | 30 / 30, recall 1.000, precision 1.000 | 43 / 44, recall 0.977, precision 1.000 |
| References found | 40 / 114, recall 0.351 | 97 / 105, recall 0.924 (33 / 105, 0.314 at HEAD before the re-land) |
| All sites | 70 / 144, recall 0.486 | 140 / 149, recall 0.940 |
| Cause of every miss | type positions 64, macro bodies 9, path-qualified calls 1 | macro bodies 8, one definition label (`upsert_note`, crates/) |

The 2026-09-04 reference number was a property of the tree-sitter Rust tags query, not of rtok's
storage: it captures plain calls, field-expression method calls, macro invocations and `impl`
items, nothing else. T52.5 added rtok's own `RUST_EXTRA_REF` / `TS_CALL_TYPE_REF` queries
(type positions, scoped calls, path segments) and measured 0.914 on 2026-09-17, but the
auto-revert `c217b8f2` removed them the same day: main CI failed on `ef6c6ff` (T51.4) in
`agents::tests::list_prints_one_block_per_app_with_kind_name_app_and_config`, a host-list test
unrelated to the graph, and the revert took the whole stack `2b8c266..ef6c6ff` with it,
including the innocent T52.5 commit; T387 re-landed the queries. Command (2026-10-04): `cargo test -p rtok --test graph_truth -- --nocapture`;
the test floor is 0.92. `src/plugins/graph/PLAN.md` lists the constructs under "Known misses".

### `graph` import-edge index time (T68.6, 2026-09-18)

Same tree (this repo checkout, 172 tagged files). Release `rtok graph index` into a fresh
`RTOK_HOME`, twice each; the second run is the comparable pair (CPU warm, still a cold store).

Command: `RTOK_HOME=$(mktemp -d) <bin> graph index <repo>`

| | Binary | Files | Rows | Cold s (1st / 2nd) |
|---|---|---|---|---|
| Before | t68.5 release | 172 | 33 312 | 0.437 / 0.270 |
| After | t68.6 release | 172 | 35 017 | 0.922 / 0.325 |

T8.8 `tests/graph_truth.rs` `labelled_symbols_are_found` (2026-09-18, after): definition recall
1.000, precision 1.000; reference recall 0.305 (floor 0.30; the T52.5 queries were reverted at
that time, see the T387 re-measure above). Imports are `kind = import` and
are excluded from `symbol_refs`.

### `graph` extra grammars and index payload size (T52.2, 2026-09-18)

This-repo debug `rtok graph index` into a fresh `RTOK_HOME` (172 tagged files, 35 231 symbol rows). The new grammars add no rows here — the tree has no Java/Kotlin/Swift/C#/Ruby/PHP sources.

Command (2026-09-18): `RTOK_HOME=/tmp/t52.2-index-home rtok graph index <worktree>` then `stat`, `sqlite3` `dbstat` / `LENGTH(...)`, `gzip -n`.

| | Bytes |
|---|---|
| `rtok.db` | 14 077 952 |
| VACUUM copy | 13 402 112 |
| `symbols` table pages | 7 503 872 |
| `symbols` indexes | 6 356 992 |
| Concatenated TEXT columns | 6 067 201 |
| gzip -n of those columns (one stream) | 197 682 |
| gzip -n of the whole db | 1 150 703 |

Most TEXT bytes are `file_sha` (2 254 784) and `root` (1 972 936) repeated on every row. One-stream gzip looks like a 30× win because it shares a dictionary across 35 231 rows; per-row gzip would grow those short fields (64-byte sha + gzip header). Live payload compression is skipped.

### `graph` v0.2 surface and latency (Gate P8b, 2026-09-04; surface re-measured 2026-09-18, T68.1)

Release build. The 3 000-file repo is generated, each file one function calling two others.

| Measurement | Value | P8b bar |
|-------------|-------|---------|
| Tools, description tokens | 5, 127 | ≤ 150 |
| Cold index, 3 000 files / 9 000 rows | 22.1 s | not gated |
| Warm `symbol` / `callers` / `impact` | 23 / 24 / 26 ms | < 100 ms |
| Definition recall, precision | 1.000, 1.000 | ≥ 0.9 |
| Reference recall | 0.351 (2026-09-04; 0.924 on 2026-10-04 after T387, see the T8.8 table above) | published, not gated |

The fourth clause — fewer tool calls per multi-file task on the P9 set — is not measured, so
the gate is open. `callers("estimate")` on this repo fell from 1 959 bytes at v0.1 to 793.

T68.9 (2026-09-18) adds `rtok bench --suite graph`: 12 architecture questions, three repos
(this tree, `bench/repos/mini-rs`, `bench/repos/mini-py`), each run twice (rtok MCP on vs
native Read/Grep only) through the T9.1 `claude -p` harness. Dry-run (no spend):
`rtok bench --suite graph --runs 1 --dry-run`. The live API run is not done (needs the
creator's go); until a dated live result lands here, codegraph's −88 % tool calls / −62 %
tokens stay a vendor claim in the §4 matrix, not an rtok number.

### `graph` composite-query chains (T52.1, 2026-09-17)

Dated command: three ad-hoc scans over all 172 transcripts in
`~/.claude/projects/*/*.jsonl` (157 sessions with tool calls; transcripts
predate the `graph` tools, so the chains are `ctx_search`/shell-`rg` shaped).

| Measurement | Value |
|-------------|-------|
| `ctx_search` calls | 615, of which 610 carry a path scope |
| Search→search refinement chains (≤3 calls apart) | 252, in 33 sessions |
| Search→read chains (≤5 calls apart) | 528, in 46 sessions |
| Shell `rg` calls with a path arg | 41 of 98 |

Verdict: GO — "X inside path Y" is the measured composite, so `symbol` /
`callers` / `impact` grew optional `path` (substring) args and `symbol` an
optional `kind` (exact) arg on the existing tools — no new tool. Surface after:
4 tools, 94 description tokens (still ≤ 150; each description ≤ 60).

Cost split with p_out = 5 × p_in (input-token equivalents). The ratio is the real one on every current Claude 5 model: Fable 5.1 $10 / $50, Opus 5.5 $4 / $20, Sonnet 5.5 $2 / $10 per MTok input / output (https://platform.claude.com/docs/en/about-claude/pricing, checked 2026-10-06, T389):

| Component | Standard cache read 0.1× | Fable/Mythos 5.1 read 0.025× |
|-----------|--------------------------|------------------------------|
| Cache reads | 137 M (64 %) | 34 M (31 %) |
| Cache writes (1.25×) | 33 M (16 %) | 33 M (30 %) |
| Output (×5) | 43 M (20 %) | 43 M (39 %) |

Reading: on standard models, context volume dominates → compress tool results and clear old ones. On Fable/Mythos, output tokens dominate → fewer lines written (ponytail-style), fewer turns, terse prose.

### `graph` LadybugDB vs SQLite (Gate P8c, T8.14, 2026-09-08)

Release build, this machine (macOS arm64). Same generated 3 000-file repo as P8b (one `fn` calling
two others, 9 000 rows). Fan-out-10 depth-4 fixture: 11 110 call edges (10+100+1 000+10 000).
Numbers from `cargo test --release --test graph_bench -- --ignored --nocapture`.

| Measurement | default (SQLite) | `--features graph-lbug` | Bar |
|-------------|------------------|-------------------------|-----|
| (1) `tests/graph_contract.rs` | 3 passed | 3 passed | unchanged, both |
| (2) `rtok hook PostToolUse` p95, n=100 | 8.07 ms | 96.6 ms | ≤ 10 ms |
| (3) warm `symbol` / `callers` / `impact(2)` | 17.9 / 17.5 / 26.8 ms | 797 / 776 / 873 ms | < 100 ms |
| (3) cold index, 3 000 files | 13.8 s; 341 ms after T35.1, 172 ms after T35.2 (2026-09-11). **Re-measured 2026-10-08 (T397)** on `1d6961541`, release, Apple M3 Max, load average 80–200 from other agents' builds: `cargo test --release --test graph_bench -- --ignored --nocapture p8c_numbers` five times gave 656 / 909 / 911 / 1,130 / 9,177 ms (**median 911 ms**); the same fixture and `index::run` call fifteen times in one process (scratch test, not committed) gave **median 479 ms** (min 301, max 2,226). Slower than 172 ms, but not comparable: the 172 ms run's load was not recorded, and the cold run now also rebuilds the symbol IDF (T368) and refreshes the rank. **T59.3's 200-file batch never shipped**: the code kept a hard-coded 64 (T35.3, `eb92c9dba`; `45c4531` only added a doc comment) until **T451** introduced `SYMBOL_BATCH_FILES`. T451 (2026-10-08, base `60cef680b`, M3 Max 16 cores/64 GB, the same `p8c_numbers` run, 16 interleaved pairs, load average 40-88 from other agents' builds): cold index median 64 files/txn **360 ms**, 200 files/txn **384 ms** (range 207-3544 ms vs 209-1703 ms; the last 10 pairs at load 40-85: 280 ms vs 324 ms; 200 was slower in 9 pairs, faster in 4, 3 within 2 %). The gap is inside the load noise, so 200 stays (fewer transactions); re-measure on an idle machine before tuning further | 33.7 s | not gated |
| (4) `impact(4)` on fan-out fixture | CTE 28.5 s | path 371 ms (**77×**) | lbug ≥ 2× CTE |
| (4) same fixture, Rust BFS | 2.61 s | 2.35 s | baseline |
| (5) `just check` (liblbug already built) | 16.9 s | same command (clippy `--all-features`) | ≤ 2× default |
| (6) release `rtok` bytes | 20 668 544 (19.7 MiB) | 34 002 576 (32.4 MiB) | published |
| (6) store after 3 000-file index | `rtok.db` 3.98 MB | `rtok.db` 200 KB + `graph.lbdb` 6.10 MB | published |

Clause (4) won. Clauses (2) and (3) fail on the `graph-lbug` binary (spawn/link cost, and every
warm tool call opens LadybugDB). Default SQLite meets (2) and (3). Incremental `just check` is
not 2× a default `cargo test` (18.9 s); the C++ cmake cost is paid once (T8.11: 3 min 21 s
debug from source, pinned). **Archive (P39, 2026-09-12):** after the freeze and the Grafeo
negative spike below, **LadybugDB was deleted** (`graph-lbug` / `lbug` / `symbols_lbug.rs`).
Numbers above are historical only — no live feature flag.

### `graph` Grafeo vs SQLite (P8e spike, T8.20, 2026-09-12) — abandoned / removed

Release build, this machine (macOS arm64). Same `tests/graph_bench.rs` harness as T8.14 plus
focused `p8e_impact4_*` tests. `grafeo` 0.5.42 (`edge`+`wal`+`grafeo-file`, no ONNX/AI).
Spike lived on `feat/graph-grafeo` (PR #21 draft / PR #22 abandon); **not merged**; code removed
with P39.

| Measurement | default (SQLite) | `--features graph-grafeo` | Bar |
|-------------|------------------|---------------------------|-----|
| (1) `tests/graph_contract.rs` | 3 passed | 3 passed (debug) | unchanged, both |
| (2) `rtok hook PostToolUse` p95, n=100 | 11.7 ms | 76.9 ms | ≤ 10 ms |
| (3) warm `symbol` / `callers` / `impact(2)` | 15.4 / 15.6 / 22.8 ms | 39.3 / 42.1 / **498.6 s** | < 100 ms |
| (3) cold index, 3 000 files | 127 ms | 19.0 s | not gated |
| (4) `impact(4)` on fan-out fixture | CTE 30.5 s | path query **DNF >14 min** | grafeo ≥ 2× CTE |
| (4) same fixture, Rust BFS | 2.65 s | 59.4 s | baseline |
| (5) build | default features | pure Rust, no cmake C++ | not catastrophic |
| (6) release `rtok` bytes | 23 733 888 (22.6 MiB) | 27 626 000 (26.3 MiB) | published |
| (6) store after 3 000-file index | `rtok.db` 4.08 MB | `rtok.db` 213 KB + `graph.grafeo` 3.72 MB | published |

Clause (1) and the cmake-free build were the only wins. Warm `impact(2)` ~22 000× slower than
SQLite (CALLS re-materialized per call); fan-out path query never finished in 14 min.
**Decision: abandon** — then **delete** under P39 (SQLite only).

### `graph` watcher idle cost (Gate P8d (2), T8.16, 2026-09-08)

Release binary, macOS arm64, this machine. `rtok mcp` idle for 60 s (stdin held open, no
requests), 50-file fixture root, `ps -o time=,rss=` sampled at t+60 s. Watcher thread inside
the server process (D18 one-writer rule).

| Measurement | `watch = "off"` | `watch = "notify"` | Bar |
|---|---|---|---|
| CPU time over 60 s idle | 0:00.03 (30 ms) | 0:00.02 (20 ms) | Δ ≤ 50 ms |
| RSS at t+60 s | 11 552 KB (11.3 MB) | 12 720 KB (12.4 MB) | Δ ≤ 2 MB |

Clause (2) **passed**: the FSEvents-backed watcher costs less CPU than the noise floor of the
measurement and +1.14 MB RSS. Per the Gate P8d decision rule the watcher is not forced to
`"off"`; it stays opt-in anyway because T8.15 shipped `watch = "off"` as the default and no
task in P8d changes it.

### `graph` watchman backend (Gate P8d (3)+(5), T8.17, 2026-09-09)

Release binary, macOS arm64, this machine, `watchman 2026.07.27.00` on PATH.
Gate P8d (1) under `watch = "watchman"`: daemon edit visible in `symbol`
within 1 s while the call reads 0 files — `watchman_sees_daemon_edit_within_1s_reading_nothing`
green (`tests/` poll the daemon's `watch-list` for the root before the timed
write, so the check measures delivery, not connect + subscribe).
Gate P8d (3): `mcp_watchman_watch_list_names_the_root` green — the MCP cwd
registers with the daemon (the first version slept a fixed 400 ms and flaked
when the daemon needed longer; it now polls `watch-list` ≤ 5 s); with no
socket the server prints exactly one `watchman: … falling back to notify`
line and serves through `notify` (`mcp_watchman_without_daemon_falls_back_once`).
Gate P8d (1) latency, single probes (debug binary, one live `rtok mcp` per
backend, `auto_index = false`, write-then-poll to `symbol`): `notify` ~250 ms,
`watchman` ~500 ms — both under the 1 s bar; the daemon path pays connect +
round-trip on top of the same 250 ms quiet period, so it cannot beat `notify`
here. Not a bench (n=1 each); the committed tests assert the bar, not the gap.
Gate P8d (4): hook path is untouched by this phase (no `hooks/` file changed;
the default binary links neither `notify` nor `watchman_client`), so no new
p95 is owed by the change itself. Measured anyway on this machine:
`cargo test --release --test latency` p95 14.8/14.9 ms (Pre/PostToolUse,
n=200) at load ~9, and 10.1/10.6 ms at load ~11 — both over the 10 ms bar
with p50 ~8.3 ms and 633 ms scheduler-stall maxima, i.e. machine load, as in
Gate P17 (7–8 ms p95 at load ~3). Resolved 2026-09-09: the straddle is the
harness itself — cargo runs both latency tests in parallel (2×200 spawns
contend). Serialized (`-- --test-threads=1`) at load ~4–6: Pre p50 7.22 ms
p95 8.25 ms, Post p50 7.18 ms p95 8.25 ms — pass with margin (row in the
Gate P17 section). Gate P8d (4) takes that row.
Gate P8d (5): release `rtok` 19 764 144 B pre-`notify` (scratch worktree at
`54f2445^`) vs 19 867 968 B (18.9 MiB) default with `notify` (+103 824 B,
+0.5 %) vs 20 429 392 B (19.5 MiB) with `--features graph-watchman`
(+561 424 B over default, +2.8 %). The feature is **not** in `default`:
(3) passes but watchman does not beat `notify` on (1) latency — same ≤ 1 s
bar, same quiet-period loop, plus a daemon the user must run — so per the
Gate P8d decision rule the `watchman_client` crate stays behind opt-in
`graph-watchman` and `watchman` stays a documented value, not the default.

### OpenTelemetry export (Gate P16, 2026-09-04)

Release binary, macOS arm64, 100 runs per event, spawn-to-exit measured from Python.
"Endpoint set" points at a local OTLP receiver that answers 200.

| Hook event | No endpoint | Endpoint set | Delta | Bar |
|---|---|---|---|---|
| `PostToolUse` p95 | 8.89 ms | 9.70 ms | +0.81 ms | 10 ms |
| `Stop` p95 | 8.36 ms | 8.97 ms | +0.61 ms | 10 ms |

`Stop` is the event that spawns the detached `rtok otel flush`; 0.61 ms is what that spawn
costs. `PostToolUse` never touches the exporter, so its delta is the extra `[otel]` section in
the config plus noise.

Payload, checked by an independent receiver that re-implements the OTLP JSON rules
(`tools/otlp_validator.py`, not the Rust encoder): **0 problems** over one session's
traffic — 205 spans, 3 metric streams, ids 32/16 lowercase hex, every int64 a decimal string,
numeric `kind` / `severityNumber` / `aggregationTemporality`. Bodies: 93.8 KB for the first
batch, 889 B per incremental flush, 1.4 KB per metrics post. Span names seen:
`execute_tool Read`, `hook UserPromptSubmit`, `hook Stop`, `hook SessionEnd`,
`invoke_agent agent` (the root, once `SessionEnd` sets `ended_at`).

**Gate P16 clause (3), 2026-09-04: open.** Jaeger, Grafana, SigNoz and Maple were not
exercised: Docker was blocked by this machine's shell allowlist. What was proven is that the
bytes satisfy the OTLP/HTTP JSON spec as an independent implementation reads it.

**Clause (3), 2026-09-07: two of four backends verified; two remain.** Docker allowed
(`lean-ctx allow docker`), the two `docs/otel.md` recipes run against a copy of the live
ledger (the `p17-bench` session: 780 hook calls, no `usage` or measurement rows), one
`rtok otel flush` each, checked through the backends' own APIs rather than by eye:

| Backend | Traces | Logs | Metrics | Flush report |
|---|---|---|---|---|
| Jaeger 2.11.0 (`jaegertracing/jaeger`) | 2 traces, 780 spans; `execute_tool` spans carry `gen_ai.tool.call.arguments` / `result` | 404 | 404 | `780 spans · 0 logs · 0 metric points · 1 posts · not served: logs, metrics` |
| Grafana `otel-lgtm` (Tempo, Loki, Prometheus) | trace `ca4564baad2314a1f53c431f7a5dc802` in Tempo | accepted, 0 rows sent (the ledger had none pending) | `rtok_calls_total` in Prometheus; no `rtok_tokens_total` because the ledger has no `usage` rows | `780 spans · 0 logs · 2 metric points · 2 posts` |

Two findings. Jaeger 2.x has no logs or metrics pipeline over OTLP/HTTP: `/v1/logs` and
`/v1/metrics` answer 404. Before this run the exporter treated that as a failure, logged one
`logs` row per flush, and re-sent that row on the next flush — pending grew by one per flush
(every 5 s under `mcp` / `proxy`), unboundedly. A 404 is now "stream not served": skipped,
watermark kept, nothing logged, named in the report (`tests/otel.rs`,
`a_404_stream_is_skipped_not_logged`). Second, `rtok_tokens_total` needs `usage` rows, which
only the proxy or an imported transcript write; hooks alone produce `rtok_calls_total`.

Still open: the clause asks for one real Claude Code session (hooks + MCP + proxy) as one
trace with an `invoke_agent` root and `chat {model}` spans — this ledger has no ended session
and no proxy traffic, and rtok is not on this machine's PATH — and for SigNoz and Maple, which
need an account or an API key.


### WASM bundle (`rtok web`, T60.7, 2026-09-18)

| What (date, command) | Result |
| --- | --- |
| Before (`ls -l crates/rtok-webui/pkg/rtok_webui_bg.wasm`, 2026-09-17) | 10,560,601 B, default `wasm-pack --release`, no `wasm-opt` |
| After (`wasm-pack --release` + wasm-opt -Oz, 2026-09-18) | 4,130,017 B |
| After the T227–T232 pages (release.yml build-local-artifacts, run 36165796413, 2026-09-25) | 4,522,155 B, over the 4,500,000 B gate; gate raised to 5,000,000 B |
| Same commit, wasm-pack `wasm-opt = ["-Oz", "--converge"]`, no binaryen on PATH (as on the release runners), macOS arm64, 2026-09-25 | 4,351,421 B (−170,734 B, −3.8 %) |
| Same, with Homebrew binaryen 132 on PATH | 4,344,475 B |

**Why the release build was bigger than every local measurement (2026-09-25).** wasm-pack runs
wasm-opt with `-O` unless `[package.metadata.wasm-pack.profile.release]` says otherwise.
`tools/webui-bundle.sh` then ran a second `wasm-opt -Oz`, but only when binaryen was on PATH.
It was on PATH locally and not on the release runners, so local builds got `-Oz` and the
runners got `-O`. Replaying the same module through binaryen 132: `-O` 4,510,120 B, `-Oz`
4,351,672 B, `-Oz --converge` 4,344,475 B. The fix moves the flags into the webui
`Cargo.toml` and drops the host-only pass, so every machine runs the same wasm-opt the same way.

Other options measured and not taken:

| Option | Result |
| --- | --- |
| `opt-level = "s"` instead of `"z"` (then `-Oz --converge`) | 4,738,453 B, worse |
| `-O4 -Oz` | 4,430,802 B, worse |
| `-Oz -Oz`, `-Oz --gufa -Oz` | 4,344,535 B and 4,344,604 B, no gain over `--converge` |
| `--strip-producers --strip-target-features` | −265 B, noise |
| `--low-memory-unused` | −8.5 KB, but only safe if nothing uses addresses below 1024; not worth the risk |

**What the module is made of** (4,351,421 B shipped build; `twiggy top` on the pre-wasm-opt
`target/wasm32-unknown-unknown/release/rtok_webui.wasm`, which keeps its name section):

| Part | Size | Notes |
| --- | --- | --- |
| Code section | ~2.65 MB | Before wasm-opt: `rtok_webui` 772 KB (742 KB of that is Slint-generated `InnerComponent_*` for the conditional pages, which is what the T227–T232 pages added), `i_slint_core` 543 KB, `read_fonts` 408 KB, `core` 330 KB, `skrifa` 243 KB, `harfrust` 204 KB, `alloc` 167 KB, `zeno` 111 KB, `winit` 105 KB |
| Data section | ~1.65 MB | IBM Plex Mono Regular/SemiBold/Bold (522,756 B, `ui/fonts`), Slint's Inter fallback font that `i-slint-common` always embeds on wasm (503,796 B), shaping and Unicode tables, strings |
| Custom sections | 260 B | `producers` and `target_features` only; no `name` or DWARF |

The three Plex weights (all used: `font-weight: 600` and `700` appear in the UI) could be
subset to the scripts the UI needs. That would change which glyphs render in Plex and which
fall back to Inter, and femtovg hints glyph outlines, so dropping hinting tables would change
rendering too. Not done here; it is the next lever if the gate is hit again.

**Regression checks** (`tests/web_wasm.rs`). The artifact checks skip when the bundle is not built:
the gate itself, separate code (3,100,000 B) and data (1,800,000 B) budgets, and an allowlist of
custom sections (a `name` section means wasm-opt did not run). Each size failure prints the
section sizes, the largest data segments and the largest function bodies, with names when the
module still has them. The source checks always run: the webui release profile (`opt-level = "z"`,
`lto = true`, `codegen-units = 1`, `panic = "abort"`, no debug info), `-Oz` in the wasm-pack
wasm-opt flags, no `--no-opt`/`--dev`/`--profiling` in the script's `wasm-pack build`, an
allowlist of Slint features, a 540,000 B budget for `ui/fonts`, and every font file imported.

### Build size (T17.1, Gate P17, 2026-09-04)

macOS arm64, this machine. Every "before" is a cold build of the same commit with the profile
lines removed; every "after" a cold build with them in. Sizes are `stat -f%z` bytes.

**Release, default features** (`[profile.release] strip = "symbols"`):

| Artifact | Before | After | Δ |
|---|---|---|---|
| `target/release/rtok` | 22 374 576 B (21.34 MiB) | 19 157 712 B (18.27 MiB) | **−14.4 %** |

Release build time is unchanged at 1m30s — stripping happens after linking.

Latency, since Gate P17 asks for it: `rtok hook PostToolUse`, 100 spawn-to-exit runs after 10
warmups, the two binaries interleaved run-for-run so they see the same machine.

| Release binary | p50 | p95 | max |
|---|---|---|---|
| stripped (this profile) | 8.16 ms | 10.07 ms | 19.21 ms |
| unstripped (same commit, `strip = "none"`) | 8.33 ms | 9.83 ms | 12.91 ms |

Stripping does not cost latency — it is marginally faster at the median, and the p95 gap is inside
the noise (four repeat rounds of the stripped binary gave p95 9.83, 10.07, 10.70, 10.83, 10.91 ms).
What the table does not show is a comfortable margin: p95 sits *on* the 10 ms bar today, against
8.89 ms recorded by the same harness at Gate P16 earlier the same day. The interleaved A/B places
that drift outside the profile change — the unstripped binary drifted with it — so it is the
machine or the grown `rtok.db`, and it is the p95 that P8/P16 own, not P17.

Re-measured 2026-09-05 with `cargo test --release --test latency -- --nocapture`, which now times
`PostToolUse` beside `PreToolUse` (200 runs each, fresh `RTOK_HOME`, so the grown database is out
of the picture). Three rounds at a 1-minute load average of 14 → 36 (other sessions compiling):

| Event | p50 | p95 | max |
|---|---|---|---|
| `PostToolUse` | 8.51 / 7.93 / 7.58 ms | 13.58 / 11.06 / 11.98 ms | 39.9 / 19.4 / 16.5 ms |
| `PreToolUse` | 7.45 / 8.09 / 7.73 ms | 10.29 / 11.65 / 12.28 ms | 22.3 / 21.0 / 17.0 ms |

The median holds where T17.1 measured it and the empty-home `PreToolUse` test drifts by the same
amount as `PostToolUse`, so the drift is the machine, not the database and not the profile.

**Gate P17 p95 clause: passed 2026-09-07** on a quiet machine (1-minute load 2.9–3.4), same
test, three rounds, release profile:

| Event | p50 | p95 | max |
|---|---|---|---|
| `PostToolUse` | 5.63 / 5.60 / 5.64 ms | **8.17 / 7.24 / 7.86 ms** | 16.2 / 10.3 / 17.9 ms |
| `PreToolUse` | 5.49 / 5.54 / 5.46 ms | **6.41 / 6.94 / 5.79 ms** | 8.6 / 13.4 / 6.7 ms |

Re-measured 2026-09-09: the straddle is the harness running both tests in
parallel (cargo default; 2×200 spawns contend with each other). Serialized
(`-- --test-threads=1`), one round at load ~4–6, release, fresh homes:
`PreToolUse` p50 7.22 ms p95 8.25 ms max 14.5 ms, `PostToolUse` p50 7.18 ms
p95 8.25 ms max 10.6 ms — both pass with margin. Parallel rounds on the
same machine straddle the bar (9.5–11.2 ms): scheduler noise, not the
binary — the hook path is unchanged since Gate P17 passed it. Gate P8d (4)
takes this row: hook p95 ≤ 10 ms holds when the harness does not load the
machine it measures.

Where a hook's milliseconds go (spawn-to-exit p50, same harness, same quiet machine; the
in-process figures are `Instant` around the call):

| Step | p50 | How measured |
|---|---|---|
| harness floor (`/usr/bin/true`) | 1.3–1.5 ms | same `Command` + three pipes |
| empty Rust binary | 1.9 ms | scratch crate, `strip = "symbols"` |
| … linking Security.framework + CoreFoundation | **3.2–3.5 ms** | same crate, one `#[link]` block |
| `rtok --version` (release) | 3.3–4.2 ms | exits inside `Cli::parse` |
| `rtok --version` (dist: thin LTO, 1 cgu, 17.4 MB) | 3.3 ms | −0.1 to −0.2 ms vs release |
| `Config::load_lenient`, real 7 KB file | 0.26 ms | in-process |
| `Store::open` + close (WAL) | 0.52–0.59 ms | in-process; `TRUNCATE` would be 0.24 ms |
| `hooks::run PostToolUse` (in-process, whole hook) | 1.07–1.30 ms | includes the store line |
| `rtok hook PostToolUse` (release, spawn-to-exit) | 5.5–5.7 ms | |

Reading: the hook's own work is ~1.3 ms; process startup is ~4 ms, of which 1.3–1.5 ms is the
dyld cost of Security.framework and CoreFoundation, linked because reqwest 0.13's `rustls`
feature hard-depends on `rustls-platform-verifier` (the crate has no webpki-roots feature any
more). Nothing on the hook path uses them. Dropping the link means a different TLS root story
for `proxy` and `otel` — an `ideas.md` entry (I-32), not a P17 task. Config loading and the
store are already small; `WAL` costs 0.3 ms per short-lived process over `TRUNCATE`, kept
because `mcp` and `proxy` write concurrently with hooks.

**T53.3 (2026-09-18).** Decision D30: one binary, webpki Mozilla roots via
`ClientBuilder::use_preconfigured_tls` — not a second hook binary. reqwest
0.13.4's `rustls` feature still references `rustls_platform_verifier::Verifier::new`
ungated under `__rustls`, so the verifier is never-called-but-linked.

`otool -L` on release `rtok`, `cargo build --release --bin rtok`, this machine:

| Binary | bytes | Security.framework |
|---|---|---|
| before (`533d68a`, pre-webpki) | 25,124,800 | linked (also CoreFoundation, CoreServices) |
| after (HEAD `0731efd` + T53.3 crates already in tree) | 25,562,032 | linked (same dylib set) |

`nm -u` on the after binary still lists `SecTrustCreateWithCertificates` and
the other `SecTrust*` imports. Security.framework did **not** disappear.

Hook spawn, n=200, nearest-rank p95, fresh `RTOK_HOME`, same spawn-to-exit
harness as `tests/latency.rs`, sequential arms, 2026-09-18, 1-minute load 50
(other agents compiling — not a P17 quiet run):

| Event | before p50 / p95 | after p50 / p95 |
|---|---|---|
| `PreToolUse` | 34.14 / 79.71 ms | 32.91 / 80.82 ms |
| `PostToolUse` | 28.99 / 66.47 ms | 47.33 / 92.60 ms |

No dyld win, as expected while the frameworks stay linked. Absolute p95 is
scheduler noise against the quiet P17 row (2026-09-07: Pre 5.79–6.94 ms, Post
7.24–8.17 ms). `cargo test --release --test latency -- --nocapture --test-threads=1`
on the after binary during the same compile storm: Pre p95 235 ms, Post 336 ms
(gate fail; load, not the hook path). Corporate CAs: `SSL_CERT_FILE`
(`docs/config.md`, TLS and corporate CAs).

**Dev, `--features graph-lbug` (archived; feature removed P39).** The whole debug footprint was one C++ library. `lbug` builds
`liblbug` through `cmake-rs`, which reads `OPT_LEVEL`/`DEBUG` from the profile: at cargo's dev
defaults that is `CMAKE_BUILD_TYPE=Debug`, `-O0 -g`. `[profile.dev.package.lbug] opt-level = 2,
debug = false` flips it to Release, `-O3 -DNDEBUG`.

| Artifact | Before | After | Δ |
|---|---|---|---|
| `liblbug.a` | 2 169 748 672 B (2.02 GiB) | 83 940 608 B (80.1 MiB) | **−96.1 %** (25.8×) |
| `lbug` build directory | 4.3 GiB | 358 MiB | **−92 %** |
| cold `cargo build --features graph-lbug` | 3m36s | 8m36s | +5m00s |

Five minutes of `-O3` bought back 3.9 GiB, once per feature set. One `cargo clean -p lbug -p rtok`
before the measurement freed **24.5 GiB across 34 028 files** on a volume that was 98 % full.

**Dev, Rust debuginfo, default features.** Measured as an isolated A/B — three cold builds into
three empty `CARGO_TARGET_DIR`s, `--config profile.dev.debug=…`, nothing else varied:

| `debug` | `target/debug/rtok` | target dir | cold build |
|---|---|---|---|
| `true` (cargo default) | 62 500 520 B | 1 479 MiB | 1m50s |
| `"line-tables-only"` (chosen) | 60 378 600 B | 1 211 MiB | 2m00s |
| `false` | 54 364 592 B | 945 MiB | 1m13s |

`line-tables-only` gives up 268 MiB of the 534 MiB that `debug = false` would, and keeps what the
fail-open rule depends on. Checked, not assumed: a crate compiled with exactly that flag still
returns `Err` from `catch_unwind`, and `RUST_BACKTRACE=1` still prints `panicked at src/lib.rs:1:14`
with `at ./src/lib.rs:1:14`, `:2:14`, `:6:18`, `:5:42` on the frames. Only variable names and types
are gone. **No profile may set `panic = "abort"`** — `hooks::dispatch` fails open through five
`catch_unwind` sites, and an abort would exit non-zero.

The first "before" for the dev binary was misleading: `target/debug/rtok` held 45 135 464 B from an
older feature set, which made the new binary look 12 MB *larger*. Only the isolated A/B above is
the real comparison. Any size measurement in a shared `CARGO_TARGET_DIR` is a stale artifact until
proven otherwise.

**Packaging `liblbug` differently does not help.** A `.framework` is a directory around the same
Mach-O — same bytes, plus a plist. An `.xcframework` is strictly larger by construction: one slice
per platform/arch, and it has no representation for the Linux x86_64 target `dist` ships (T10.4).
A `.dylib` (`LBUG_SHARED=1`) is the only variant that removes bytes — static linking with
`+whole-archive` copies lbug into `rtok` *and* into each of ~12 test binaries — but it costs the
single static binary of D1, `lbug`'s `build.rs` emits no `-rpath` so those test binaries would not
find it at runtime, and `LBUG_SHARED` is a build-time env var while `.cargo/config.toml [env]` is
global, so "dylib in dev, static in release" is not expressible and toggling it forces a full C++
rebuild (`rerun-if-env-changed`). At 80 MiB rather than 2.02 GiB the premise is gone anyway.

### `rtok stats --save-baseline before-rtok` (2026-09-03)

Gate P1. Default `[stats] since = 30d` (not the 17-session slice above). File: `~/.rtok/measurements/before-rtok.json`. `--compare before-rtok` → all Δ0. Estimator: 4 chars/token. No rtok hooks in `settings.json` at save time. Proxy `usage` empty (`api` {}).

| Item | Value |
|------|-------|
| Sessions / lines | 580 / 181 303 (0 malformed) |
| Tool results, est. tokens | 13.03 M total; Bash 7.71 M (59 %), Read 3.07 M (24 %), WebSearch 0.70 M, MCP lean-ctx `ctx_read` 0.48 M |
| Bash by family | sed 1.71 M, cd 1.20 M, cat 0.93 M, grep 0.93 M, git 0.35 M |
| MCP groups | lean-ctx 0.86 M, engram 36 K |
| Cache | read 8 124 M, creation 235 M, uncached input 1.52 M → 97.2 % hit rate |
| Median final context | 65 882 tokens per session |
| Archive replay (estimate) | CTT 11.81 G → 8.42 G (−28.7 %); 1 803 candidates |

### A/B bench (T9.2, 2026-09-02)

Config A = `bench/configs/legacy.json` (81-hook + dual-proxy baseline described above). Config B = `bench/configs/rtok.json` (7 rtok hooks, `rtok mcp`, `ANTHROPIC_BASE_URL` :8790). Six tasks × 3 runs. Live `claude -p` is gated on `RTOK_BENCH_LIVE=1`; this commit ran without it, so usage/cost are zeros and pass rate is the tasks' `check = true`.

| config | mean input | mean cache | mean output | mean cost USD | pass |
|--------|------------|------------|-------------|---------------|------|
| a (legacy) | 0 | 0 | 0 | 0.0000 | 6/6 |
| b (rtok) | 0 | 0 | 0 | 0.0000 | 6/6 |
| **delta (b−a)** | 0 | 0 | 0 | 0.0000 | 0 |

Source: `bench/results/a.json`, `bench/results/b.json`. Re-run with `RTOK_BENCH_LIVE=1` to fill cost.

Your local meters (each measures a different slice, none the bill):

| Tool | Own meter | Note |
|------|-----------|------|
| rtk | 40.4 % of bash output over 5,725 cmds (1.1 M of 2.8 M) | `rtk read` only 6.4 %; grep 18.5 %; diff 90 % |
| headroom | 3.2 % today, 3.5 % 7 d, 11.3 % 30 d (14.35 M of 126.7 M) | proxy on :8788 chained to caveman :8787 |
| lean-ctx | "75 % ratio, 6.1 M difference" | +3.1 K tokens/turn fixed injection; 0 output tokens saved; "not a provider bill" (its words) |
| caveman | proxy in record mode; this session uncompressed | Pro/Max streaming sessions pass through |
| claude-mem | "87 % savings" | ratio of its own retrieval reads (21 K) vs. work it indexed (159 K) |
| token-optimizer | author's sessions: 14.44 M tokens / 30 d, "$313/mo measured" | 3.3 chars/token estimator; 27 Python hooks here |

## 3. Host surfaces (verified against code.claude.com/docs/en/hooks and env-vars, 2026-09-01)

- 32 hook events. PreToolUse may return `permissionDecision` (allow/deny/ask) and `updatedInput`; exit 2 blocks. **PostToolUse cannot modify or replace the tool result**; it only adds context. SessionStart/UserPromptSubmit inject via stdout or `additionalContext`. Command hooks support `async: true`. Timeouts are per event (600 s default, 30 s UserPromptSubmit, 10 s MessageDisplay).
- `ANTHROPIC_BASE_URL` routes traffic through a proxy; docs say it disables MCP tool search by default (check `rtok doctor`). `ENABLE_TOOL_SEARCH` (T388, checked 2026-10-06 against <https://code.claude.com/docs/en/env-vars> and <https://code.claude.com/docs/en/mcp#configure-tool-search>): unset defers all MCP tools, except it loads them upfront when `ANTHROPIC_BASE_URL` is a non-first-party host (also on Foundry-on-Azure and pre-4.5 Agent Platform models); `true` always defers (requests fail on proxies that drop `tool_reference` blocks); `auto` defers once definitions reach 10 % of context, `auto:N` sets N % (0-100); `false` loads all tools upfront. The value may be set in the shell or in the settings.json `env` field; `CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS` keeps it off and ignores an explicit value. So the base URL alone is a heuristic: `doctor` prints `enabled`, `disabled` or `unknown (heuristic: ANTHROPIC_BASE_URL set)` with the source. `BASH_MAX_OUTPUT_LENGTH` default 30,000 chars, max 150,000. `autoCompactWindow` is set to 300000 in your settings; the env var name for it is undocumented.
- API side: prompt caching 1.25× (5-min write), 2× (1-h write), 0.1× read (0.025× Fable/Mythos 5.1). Context editing strategies `clear_tool_uses_20250919`, `clear_thinking_20251015`, `compact_20260112` (trigger 100 K, keep 3). `count_tokens` is free and rate-limited. Memory tool `memory_20250818`. Claude Code issue #81967: tools-array mutation invalidates the cache (up to −274 K tokens observed).
- Other hosts: Cursor `hooks.json` (before/after shell), OpenCode plugin API `tool.execute.after` (the one host that can replace results), Codex (MCP; proxy needs Responses API), Gemini CLI (MCP).

### `modes` terse/yagni vs caveman/ponytail-style (2026-09-10)

Branch `feat/modes-cave-pony`. Native helpers in `src/modes/` + enriched `modes/terse.md` /
`modes/yagni.md` via `plugins::inject`. **Not** a wrap of JuliusBrussee/caveman or
DietrichGebert/ponytail (D6); prompt text stays data (D7). Re-run:
`cargo test --test mode_bench -- --nocapture`.

Estimator: prose 4.2 chars/token (same `Estimator` as the rest of the suite). Baselines are
honest and weak on purpose — not the vendors' marketing figures from §4.

#### Compress (caveman-style)

17 agent-reply fixtures with fluff, ``` fences, backtick error strings, and critical
negations. Weak **caveman-lite** strips only `Sure!` / `I'd be happy to help…`. Ours is
`compress_prose(…, CaveIntensity::Full)`.

| Metric | Weak caveman-lite | rtok Full | Notes |
|--------|-------------------|-----------|-------|
| Avg chars saved / fixture | 12.2 | **36.2** | |
| Char save % (corpus) | 11.7 % | **34.9 %** | |
| Est. tokens saved (sum) | 50 | **147** | |
| Fence bodies | n/a (lite may leave fluff outside) | **byte-identical** to input | Hard fail if mutated |
| Negation tokens | — | **kept** (`not`/`never`/`no`) | |

Why this shape: caveman's Go shrink path is out-of-process and has corrupted inline code
(#112). A deterministic in-process stripper that refuses to touch fences is the reversible,
CI-checkable analogue — and it still beats the weak lite baseline by ~3× on chars and
tokens on this fixture set.

#### Ladder (ponytail-style)

14 labelled contexts (speculative skip, reuse, stdlib, native-before-dep, security
`must_not_simplify`, …). Naive baseline = always [`LadderDecision::Minimum`] (unstructured
“be lazy” without a ladder).

| Metric | Naive always-Minimum | rtok `evaluate_ladder` |
|--------|----------------------|------------------------|
| Correct rung | 4 / 14 (**29 %**) | **14 / 14 (100 %)** |

Why better: a prompt-only YAGNI mode has no typed state; models default to “write the
smallest new code,” which is wrong when the right answer is skip, reuse, or native
platform. Security work forces Minimum even when `speculative` is set.

#### Mode markdown budgets (inject)

| Mode | Est. tokens | Chars | Cap |
|------|------------:|------:|-----|
| `terse.md` | 162 | 679 | ≤ 250 |
| `yagni.md` | 145 | 613 | ≤ 250 |
| `nudges.md` | 114 | 476 | ≤ 250 |

Aliases `cave`→`terse`, `pony`→`yagni` resolve to the same builtins at SessionStart.
`nudges` has no alias; it is opt-in via `[plugins.inject] modes` (default `[]`).

#### Coaching nudges A/B (T53.1, 2026-09-18)

I-18: short nudges (do not re-read, use `expand`, outline-first, search before Grep) may
cut waste, but they are re-read every turn. Data lives in `modes/nudges.md` (D7). Isolated
store, `[plugins.memory] recall_titles = 0`. Command:

```bash
printf '%s' '{"hook_event_name":"SessionStart","session_id":"t531","source":"startup"}' \
  | rtok --config <tmp>/rtok.toml hook SessionStart
```

| arm | `[plugins.inject] modes` | SessionStart `additionalContext` bytes | est. tokens (prose 4.2) |
|-----|--------------------------|---------------------------------------:|------------------------:|
| off (default) | `[]` | 0 | 0 |
| on | `["nudges"]` | 478 | 114 |

On-arm bytes are identical across two consecutive runs. The same config's
UserPromptSubmit `additionalContext` does not contain `# nudges`.

`rtok bench` both arms without `RTOK_BENCH_LIVE` (live `claude -p` **not** run):

| config | mean input | mean cache | mean output | mean cost USD | pass | live |
|--------|------------|------------|-------------|---------------|------|------|
| off | 0 | 0 | 0 | 0.0000 | 6/6 | false |
| on | 0 | 0 | 0 | 0.0000 | 6/6 | false |

Pass parity holds; cost is zeros.

**Live A/B (2026-09-18).** Creator approved API spend. Intended harness: `RTOK_BENCH_LIVE=1`
`rtok bench --runs 1` on the existing six-task suite, two arms (default `modes = []` vs
`modes = ["nudges"]`), same `claude -p` path and host default model, then
`rtok stats --price` per isolated store. Stopped before that pass:

```bash
claude --version
# 2.1.236 (Claude Code)
claude auth status
# {"loggedIn": false, "authMethod": "none", "apiProvider": "firstParty"}
claude -p "Reply with the single word ok and nothing else." --output-format json --max-turns 1
# is_error true; result: Failed to authenticate: OAuth session expired and could not be refreshed
# usage all zeros; total_cost_usd 0
```

`ANTHROPIC_API_KEY` was unset. `claude` CLI was present. A second probe with the
environment's gateway key against the already-set `ANTHROPIC_BASE_URL` returned HTTP 401
`Invalid API key`. No live token, cache, or USD rows. Cost per passed task cannot be
compared; the gate is **do not enable**. `nudges` stays **off** by default
(`config/default.toml` `[plugins.inject] modes = []`).

**Reading vs §4 vendor claims.** We still do **not** claim caveman's 65 % or ponytail's
−54 % LOC against a live bill. This gate shows the native path wins the re-runnable
fixture contest and stays inside the inject budget — the same honesty bar as the offline
T9.2 A/B zeros.

#### T134: does `updatedToolOutput` replace native tool output in the CLI? (2026-09-25)

Gate for I-91 (§17.2). Claude Code 2.1.267 (`/opt/homebrew/bin/claude`). Docs disagree:
`code.claude.com/docs/en/hooks` (CLI shell-command hook reference, fetched today) still
lists only `additionalContext`, `systemMessage`, `terminalSequence` for `PostToolUse`
`hookSpecificOutput` — no `updatedToolOutput`. `code.claude.com/docs/en/agent-sdk/hooks`
(Agent SDK — hosts built on the Python/TypeScript SDK) says: "For `PostToolUse` hooks, you
can set `additionalContext` to append information to the tool result. To replace the
tool's output before Claude sees it, set `updatedToolOutput`, which works for any tool in
both SDKs. The older `updatedMCPToolOutput` field replaces MCP tool output only and is
deprecated." `anthropics/claude-code#32105` asked for exactly this on built-in tools
("Extend `updatedMCPToolOutput` to work for all tools … or add a parallel
`updatedToolOutput` field") and is now closed with no resolution comment visible from a
page fetch of the issue.

Planned probe: a scratch `PostToolUse` command hook (matcher `Read|Bash|mcp__.*`,
`/private/tmp/.../scratchpad/t134/hook.sh`) returning
`{"hookSpecificOutput":{"hookEventName":"PostToolUse","updatedToolOutput":"PROBE-REPLACED-<tool>"}}`,
run under `claude -p … --settings <scratch>/settings.json --setting-sources ""
--mcp-config <scratch>/mcp.json --strict-mcp-config` against a Bash `echo`, a `Read` of a
scratch file, and the `rtok mcp` server's `tree` tool. Blocked before any transcript:

```
claude -p "say hi" --model haiku --output-format json
# is_error true; result: "Failed to authenticate: OAuth session expired and could not be
# refreshed" (usage all zeros, total_cost_usd 0)
```

Reproduced with zero custom flags (no hook, no `--settings`, no `--mcp-config`), so it is
not this probe's setup — a `claude -p` child process gets no live model turn from inside
the agent's sandboxed shell, the same failure already on record for the T53.1 live
A/B ("Coaching nudges A/B (T53.1, 2026-09-18)" above). A `--permission-mode bypassPermissions` variant was
separately refused by the auto-mode classifier ("Create Unsafe Agents") before the auth
call was even reached — nested `claude` invocations are not available from inside a CCD
session, by design or not.

**No transcript, so I-91 does not close today.** The only evidence is the doc split above:
the CLI's own `PostToolUse` JSON schema omits `updatedToolOutput` while the Agent SDK page
documents it as an Agent-SDK-level construct ("in both SDKs" reads as Python/TypeScript
Agent SDK, not the `claude` CLI's command-hook schema). That is consistent with the
standing rtok rule (the first bullet of §3) but does not prove it for the CLI surface. Re-run needs a
`claude -p` invocation outside this sandbox — a real terminal or CI runner with its own
OAuth session — to get an actual Bash/Read/MCP transcript.

## 4. Comparison matrix

Stars/language/license from the GitHub API on 2026-09-01. "Claimed" is the vendor's number; "Measured" is yours or an independent source.

| Tool | Layer | Mechanism | Surface | Lang · License · Stars | Local · LLM-free · Lossless | Claimed | Measured |
|------|-------|-----------|---------|------------------------|-----------------------------|---------|----------|
| rtk (rtk-ai) | command output | ~80 filters, TOML custom filters, `gain` (bytes/4) | PreToolUse rewrite | Rust · Apache-2.0 · 78.2 k | ✓ · ✓ · ✗ (drops lines) | 60–90 % | 40 % of bash bytes (yours); JetBrains bill +7.6 %/0 % |
| lean-ctx (yvgude) | file reads, search, shell | 78 MCP tools, 10 read modes, 95+ shell patterns, dedup re-reads | MCP + hooks (deny Grep/Glob) | Rust · Apache-2.0 · 3.7 k | ✓ · ✓ · ~ (expand) | 75 % (own) | +3.1 K/turn injection; 0 output saved |
| headroom (headroomlabs-ai) | API request | JSON crusher, code compressor, cache-aligned live zone, CCR retrieve | proxy + MCP + wrap | Python 82 %/Rust 13 % · Apache-2.0 · 68.3 k | ✓ · ✓ · ✓ (retrieve) | up to 95 % | 11.3 % 30 d, 3.2 % today (yours) |
| caveman (JuliusBrussee) | prose + request | terse mode, shrink-hook, proxy record/compress, TOON, MCP compress/retrieve | prompt + proxy + MCP | Go · custom · 102 k | ✓ · ✓ · ~ | 65 % output | 8.5 % agentic (JetBrains); 0 here (proxy inert); issue #112 corrupts inline code; rtok native Full beats weak-lite 34.9 % vs 11.7 % chars on 17 fixtures (2026-09-10, see modes subsection) |
| ponytail (DietrichGebert) | model output | YAGNI ladder prompt | prompt file | JS · MIT · 120 k | ✓ · ✓ · n/a | −54 % LOC, −22 % tokens (own bench, Haiku 4.5, n=4) | none independent; rtok typed ladder 14/14 vs naive Minimum 4/14 on 14 fixtures (2026-09-10, modes subsection) |
| token-optimizer (alexgreensh) | reads, bash, archive, compaction, coaching | delta reads, structure maps, bash compress (111 cmds), archive >4 KB, checkpoints, quality nudges | hooks (Python subprocess) | Python · PolyForm-NC · 2.1 k | ✓ · ✓ · ✓ (archive) | "$313/mo" | author's own meter only; 27 hooks on your machine |
| codebase-memory-mcp (DeusData) | code graph | tree-sitter 158 grammars + hybrid LSP (11 langs) → SQLite (zstd), 15 tools, Cypher-like queries | MCP | C · MIT · 42.1 k (v0.7.0, 2026-09-04) | ✓ · ✓ · n/a | 99.2 % on 5 queries; Linux kernel 3 min | exits on start here (0 tools, `doctor` 2026-09-02); 281 MB binary, 141 MB cache |
| codegraph (colbymchenry) | code graph | `.codegraph/codegraph.db` (SQLite+FTS5), one `codegraph_explore` tool | CLI + MCP | C/TS · MIT · 69.4 k (2026-09-04) | ✓ · ✓ · n/a | −88 % tool calls, −62 % tokens (7 repos) | 97 MB `.codegraph/` for one repo (cross-code); no binary here |
| code-review-graph (tirth8205) | code graph | tree-sitter → SQLite, impact radius, minimal context, communities | MCP (30 tools) | Python · MIT · 31.2 k (2.3.7, 2026-09-04) | ✓ · optional embeddings · n/a | 65× median (36–376×, 6 repos) | 30 tools ~2 295 desc tokens (`doctor`); F1 0.69 against its own edges (own README) |
| graphify (safishamsi) | code graph | 37 tree-sitter grammars + LLM extraction for docs → JSON, Leiden communities, HTML map | CLI + MCP + plugin | Python · Apache-2.0/MIT · 114 k (v8, 2026-09-04) | ✓ · ✗ (docs layer) · n/a | “code maps for free” | not installed; none |
| serena (oraios) | symbols | LSP-backed symbol tools | MCP | Python · MIT · 28.7 k (2026-09-04) | ✓ · ✓ · n/a | — | 22 tools ~1 494 desc tokens; times out at 30 s here; most precise |
| claude-mem (thedotmack) | memory | LLM-extracted observations, SQLite+Chroma, progressive disclosure | plugin + MCP + daemon | JS · Apache-2.0 · 92.9 k | ✓ · ✗ (uses Claude) · n/a | 87 % (own banner) | costs tokens to build memory |
| engram (Gentleman-Programming) | memory | agent-written notes, SQLite FTS5, HTTP+MCP | MCP | Go · MIT · 6.3 k | ✓ · ✓ · n/a | — | 18 tools' descriptions per session |
| mem0 / OpenMemory | memory | LLM extraction + vectors | MCP (Docker, Qdrant) | Python · Apache-2.0 · 64.5 k | ~ · ✗ · n/a | — | not local-first by default |
| OpenViking (volcengine) | context DB | L0/L1/L2 tiered loading, session compression | SDK + MCP | Python 78 %/Rust 14 % · AGPL-3.0 · 34.9 k | ✓ · ✗ · n/a | 34–91 % | none |
| TOON (toon-format) | data format | tabular JSON → TOON | library | TS · MIT · 25.3 k | ✓ · ✓ · ✓ | −42.6 % tokens; accuracy 72.2 vs 71.4 | vendor bench |
| LLMLingua-2 (microsoft) | prompt compression | small-model token pruning | library | Python · MIT · 6.6 k | ✓ · needs model · ✗ | 2–20× | quality risk on code |
| bifrost (maximhq) | gateway | semantic cache (redis, 0.9 threshold) | proxy | Go · Apache-2.0 · 7.7 k | ✓ · embeddings · ✗ | — | agent contexts never repeat; a hit is a wrong answer |
| Anthropic native | platform | prompt caching, context editing, memory tool, deferred tools, auto-compact | API/Claude Code | — | ✓ · — · ✓ | — | free; align with it, do not fight it |
| **Your stack today** | all layers | 10 tools, 81 hooks, 2 proxies (+bifrost in Docker) | hooks + MCP + proxy | mixed | ✓ · mostly · mixed | — | 3–40 % per slice; no end-to-end number |
| **rtok `modes` (terse/yagni)** | model output / prose | enriched prompt modes + native `compress_prose` / `evaluate_ladder` | inject + `src/modes` | Rust · (in-tree) | ✓ · ✓ · ✓ fences | none claimed on bill | fixture bench 2026-09-10: 34.9 % chars / 147 tok vs lite; ladder 14/14; budgets 162/145 |
| **Proposed `rtok`** | all layers | 10 plugins, 1 binary, 3 surfaces, measurement + bench | hooks + MCP + proxy | Rust | ✓ · ✓ · ✓ by default | none until measured | `rtok stats`, `rtok bench` |

## 5. Your stack vs. the proposed app

| Aspect | Today | Proposed |
|--------|-------|----------|
| Hooks | 81 across 16 events (token-optimizer 27 Python, orca 12, caveman-proxy 11, holdmylid 10, lean-ctx 9, cbm 7, tokenbar 2, rtk 1, caveman shrink 1, codegraph 1) | ≤ 8 token-related (`rtok hook <event>`), non-token hooks untouched |
| Per-tool-call overhead | up to ~30 subprocesses per event chain, several Python | one Rust process < 10 ms |
| Bash filtering | rtk + lean-ctx ctx_shell + token-optimizer bash_compress | `cmd` (delegates to rtk filters, archives raw, measures) |
| Reads | lean-ctx (78 tools, 3.1 K/turn) + token-optimizer read_cache + headroom audit | `read` (3 tools: `read`, `search`, `tree` as of 2026-10-08, per `src/plugins/read/README.md` Tools; no banner, dedup) |
| Proxies | headroom :8788 → caveman :8787 (inert on Max) → Anthropic; Docker: headroom → bifrost | `rtok proxy :8790` (passthrough → compress), chainable for A/B |
| Memory | claude-mem (LLM) + engram | one, agent-written, FTS5 |
| Code graph | code-review-graph + codebase-memory-mcp + serena (+codegraph stale) ≈ 85 MCP tools | `graph`: native tags index, 5 tools as of 2026-09-18 (`symbol`, `callers`, `impact`, `outline`, `explore` (T68.1), 127 description tokens; D6; was “adapter” before D6 was rewritten) |
| Injection per turn | lean-ctx 3.1 K + engram + claude-mem + ponytail + caveman + token-optimizer nudges | one budget (800 tokens), byte-stable |
| Measurement | 5 incompatible meters, none the bill | usage from proxy + transcripts; context-token-turns; A/B bench |
| Reversibility | partial (headroom retrieve, token-optimizer expand) | every rewrite has `expand <id>` |

## 6. Technique ranking (evidence-weighted, for this workload)

1. Clear or shrink old tool results in context (archive live zone; context editing) — compounds over turns.
2. Keep the cached prefix byte-stable (no tools-array or system-prompt churn; stable injections).
3. Read less: outline/signature modes on first read, dedup re-reads, deny giant native reads.
4. Fewer output tokens: YAGNI/terse modes (measure), fewer turns via better first reads.
5. Command output filtering — real but small in the bill (JetBrains); keep it lossless.
6. Tabular JSON → TOON where results are tables (−40 %).
7. Memory with progressive disclosure (titles → ids → bodies), never bodies at SessionStart.
8. Code graph queries instead of grep-and-read chains — plausible, not measured end to end (the T68.9 live run is not done, §2); native index per D6, never an adapter.
9. LLM-based compression — negative until proven; **v0.2+**, not v0.1 (`ideas.md` I-21).



### Plugin design surveys (P14, 2026-09-02)

Every alternative in `src/plugins/*/PLAN.md` with the survey date. Stars/versions for retired tools are from §4 (GitHub API 2026-09-01) unless noted.

| Alternative | Version / date | Cited by |
|-------------|----------------|----------|
| rtk gain | rtk-ai · 2026-09-01 | measure, cmd, guard |
| headroom savings / live zone / proxy / audit | 2026-09-01 | measure, read, archive, proxy |
| token-optimizer dashboard / bash_compress / structure map / archive / refetch | 2026-09-01 | measure, cmd, read, archive, guard |
| lean-ctx ctx_shell / read modes / banner / deny Grep | 2026-09-01 | cmd, read, inject, guard |
| caveman shrink / proxy / TOON | 2026-09-01 | archive, proxy, toon, inject/modes |
| engram | 2026-09-01 | inject, memory |
| claude-mem | 2026-09-01 | inject, memory |
| ponytail | 2026-09-01 | inject, `src/modes` ladder |
| rtok modes terse/yagni (native) | fixture bench 2026-09-10 | inject, `src/modes`, `tests/mode_bench.rs` |
| codebase-memory-mcp | v0.7.0 · 2026-09-04 | graph |
| codegraph | 2026-09-04 | graph |
| code-review-graph | 2.3.7 · 2026-09-04 | graph |
| graphify | v8 · 2026-09-04 | graph |
| serena | 2026-09-04 | graph |
| OpenViking | 2026-09-01 | memory |
| bifrost | 2026-09-01 | proxy |
| TOON (toon-format) | 2026-09-01 | toon |
| Langfuse generation usage | 3.x · 2026-09-02 | measure (outside retired stack) |
| cargo --message-format=json | rustc 1.90 · 2026-09-02 | cmd (outside) |
| aider repo map (tree-sitter + PageRank) | 0.82 · 2026-09-02 | read (outside) |
| Anthropic context editing / tool-result clearing | docs 2026-09-01 | archive (outside) |
| LiteLLM | 1.7x · 2026-09-02 | proxy (outside) |
| Lost in the Middle (Liu et al.) | arXiv 2307.03172 · 2023-07 | inject (outside) |
| LangGraph recursion limit | 0.2x · 2026-09-02 | guard (outside) |
| mem0 / OpenMemory | 2026-09-01 | memory (outside) |
| Universal Ctags | 6.x · 2026-09-02 | graph (outside) |
| minified JSON / CSV / JSONL | RFC 8259 / 4180 · 2026-09-02 | toon (outside) |

## 7. Fact-check ledger

Checked 27 claims + 19 repos. Refuted: JetBrains rtk post (rtk did not save; +7.6 % on low effort), TOON numbers (42.6 %, 72.2 vs 71.4), "headroom is Rust" (82 % Python), "OpenViking is Rust" (78 % Python), "claude-mem is TypeScript" (JS per API), "37 hook events" (32). Partial/unverifiable: `CLAUDE_CODE_AUTO_COMPACT_WINDOW` (undocumented; settings key exists), cache invalidation order and 20-block lookback (not in docs), `ENABLE_TOOL_SEARCH` value range and 10 % trigger (documented since; see §3, T388), offline Claude tokenizer (docs silent; `count_tokens` is the only official path). Confirmed: PostToolUse cannot modify results; PreToolUse `updatedInput`; caching multipliers incl. 0.025× Fable/Mythos; context-editing names; memory tool name; issue #81967; caveman #112; ponytail bench figures; codebase-memory-mcp and codegraph README figures; lean-ctx README figures. GitHub reports NOASSERTION for caveman and token-optimizer licenses; token-optimizer's local LICENSE file is PolyForm Noncommercial 1.0.0.

2026-10-08. mcpproxy-go (github.com/smart-mcp-proxy/mcpproxy-go) `README.md:43` claims ~99 % token reduction and 43 % accuracy improvement. `bench/README.md:54-57` measures 17 % (`retrieve_tools`, 1730 → 1431) and 43 % (`code_execution`, 1730 → 986) on a 45-tool corpus of name+description only, LLM-free. `specs/103-token-bench/findings/honest-savings-design.md:405-410` says the 99 % line cannot stand once the proxy's own tools are counted. rtok will not port it.

## 8. Open questions

- Does `ANTHROPIC_BASE_URL` really disable MCP tool search on your setup (deferred tools are visible in this session, so something enables it)? `rtok doctor` T1.4 answers it.
- Does headroom's live-zone compression keep the prefix byte-stable across turns in your traffic? T5.5 cache-health report answers it before rtok compress replaces it.
- Pricing for Fable 5.1 output tokens: answered. $50 / MTok output vs $10 input, so the cost split's p_out = 5 × p_in holds; cache read $0.25 (0.025×), 5-minute write $12.50 (https://platform.claude.com/docs/en/about-claude/pricing, checked 2026-10-06). `rtok stats --price` ships the row (T389).

## 9. Competitive gap review (2026-09-17)

Method: five Haiku web-scan agents (rtk/headroom/caveman; MCP read+graph servers; memory and repo-map tools; host-native features; new 2026 entrants) and one Sonnet agent that inventoried rtok's own surface from code and config. Synthesis by Fable 5.1. Scan cost: ~470 K subagent tokens. Everything below is from READMEs, release notes and docs as of 2026-09-17 unless marked *measured*; vendor numbers are quoted as claims. Two scan results were discarded as wrong targets (an `engram` by AcidicSoil, an `OpenViking` mirror by LoicHmh) — the §4 rows for those two tools stand un-refreshed. `token-optimizer-mcp` (ooples, v7.0.0, TypeScript, 74 tools) is a different project from the `token-optimizer` hook pile in §4; both are listed.

### 9.1 What moved since §4 (2026-09-01)

| Tool | Then | Now (2026-09-17) | Source |
|------|------|------------------|--------|
| rtk | ~80 filters | v0.47.0 / 0.50.0-rc: 100+ filters (ctest, mvnd, spring-boot, liquibase, ssh, AWS, containers), Gemini `BeforeTool`, OpenCode/OpenClaw/Hermes adapters, Windows native hook; maintainers now say "cuts bash output, not necessarily the bill by 90 %" | github.com/rtk-ai/rtk/releases |
| headroom | proxy + CCR | adds ASGI middleware, TS library, Codex WS fixes, hosted proxy; claims 21–57 % on own benchmarks, 20 % "for coding agents" | github.com/headroomlabs-ai/headroom/blob/main/CHANGELOG.md |
| caveman | skill + proxy | v2.7.0: `caveman learn` (ranks token sinks from history), hosted proxy, per-request accounting; engine relicensed BSL-1.1 | github.com/JuliusBrussee/caveman/releases |
| lean-ctx | 78 tools | v3.10.2: 30+ tools incl. `ctx_handoff`/`ctx_agent`, 10 read modes incl. `diff`/`density`, mode predictor learned from past sessions, ~13-token re-reads, still ~3.0 K fixed per-session overhead (own README) | github.com/yvgude/lean-ctx |
| token-optimizer-mcp | — | v7.0.0: default switched from `enforce` to `assist`; publishes a randomized 16-task holdout: cheaper on 11/16, median 0.926× cost, quality 0.994 vs control (own bench) | github.com/ooples/token-optimizer-mcp/releases/tag/v7.0.0 |
| codebase-memory-mcp | v0.7.0 | v0.10.0, LZ4 store, arXiv:2603.27277 (31 repos, 83 % answer quality, "99.2 % fewer tokens" on 5 queries) | github.com/DeusData/codebase-memory-mcp |
| serena | — | v1.6.0 (2026-07-16), 40+ LSP languages; no token measurement | github.com/oraios/serena |
| claude-mem | — | v13.25.1, 4 MCP tools, SQLite+FTS5+Chroma, cloud sync; one third-party "~10×" retrieval comparison (mindstudio.ai blog, not a bill delta) | github.com/thedotmack/claude-mem |
| mem0 | — | v3.1.8; states its own overhead: 6.7–7.0 K tokens + ~1 s per add | github.com/mem0ai/mem0 |
| New entrants | — | jCodeMunch-MCP (symbol retrieval, published 96.5 % vs grep-read bench, 2026-09-03); atlassian-labs/mcp-compressor (lossless wrapper that compresses *any* MCP server's results, levels low…max, 40–60 % claimed, no bench); Portkey/LiteLLM gateways ("tool description compression + allowlist", 18–28 % claimed); billion-context (prefix-cache-friendly reversible history compression proxy); tool-result-cache-rs / TVCACHE (content-hash tool-result caches) | URLs in the scan; all claims unverified |

### 9.2 Host-native features that make third-party work redundant (docs, 2026-09-17)

| Host | Native now | Effect on rtok |
|------|-----------|----------------|
| Claude Code | Tool Search defers MCP tools (~3 K tokens loaded per query instead of every schema); auto-memory `MEMORY.md` (v2.1.59+, on by default); `PreCompact` / `PostCompact` / `Setup` / `CwdChanged` / `FileChanged` / `PreModelSwitch` hooks; `promptCacheTtl`; images/PDFs auto-dropped near the limit | Description-token compression (Portkey-style) is not worth building — `doctor` already flags `mcp_tool_search_disabled`. Auto-memory overlaps `memory` recall on this host. rtok already registers `PreCompact`/`PostCompact` here (T2.5: checkpoint of prompts, paths, errors; modes re-injected on `source = compact`); shipped as T58.2 (2026-09-18): the checkpoint lists the live archive ids (`id <archive-id> <tool> <bytes>`), and Codex (`PreCompact`/`PostCompact`), Cursor (`preCompact`) and Copilot CLI (`preCompact`) register their compaction events; Cursor and Copilot have no post event, ZCode has none, pi/OpenCode follow T70.6. |
| Claude API | context editing (`clear_tool_uses`, `clear_thinking`), server-side compaction, memory tool (~2.5 K overhead), 1h cache TTL at 2× write, Fable/Mythos 5.1 cache read 0.025× | `archive` and context editing do the same job; T51.2 (emit native context editing) is the reconciliation. |
| Cursor | `afterMCPExecution` fires after the tool response and before it enters context; `preCompact`; "Dynamic Context" (v3.11, claims 46.9 %, no method published) | Resolved 2026-09-24 (T190, https://cursor.com/docs/agent/hooks, read that day): `afterMCPExecution` documents no output, so rtok treats it as audit-only (`{}`). Only `postToolUse` can replace an MCP result, through `updated_mcp_tool_output` (verified 2026-09-18, T70.4). |
| Codex CLI | `PreCompact`/`PostCompact`, `SubagentStart/Stop`, hooks may call MCP tools | Same compaction surface as Claude Code. |
| OpenCode | Two-phase compaction: non-destructive "marking" of old verbose tool outputs (trigger when >20 K freed, keeps newest 40 K), then LLM summary; `experimental.session.compacting` hook | Overlaps `archive` on this host; a pointer inside a marked result is harmless (fail open) but the saving is double-counted unless measured per host. |
| Gemini CLI / Copilot CLI | context-compression hook before summarization; Copilot auto-compacts at 80 % and has `/context` | Same as above; `/context` is what `rtok doctor` prints. |

### 9.3 Better / worse / missing, by category

Grounded in §2 (this workload: tool results 2.83 M est. tokens, Bash 35 %, Read 15 %; assistant output 8.6 M tokens of which **96 % is tool input**, i.e. the code and `old_string`s the model writes; on Fable/Mythos 5.1 output is **39 %** of the bill).

| Category | rtok better | rtok worse | Missing, and whether it is worth building |
|----------|-------------|------------|-------------------------------------------|
| Command output | lossless (`expand`), measured per family, one process ≤ 10 ms; a default rule (40 lines, head/tail, dedupe) caps every stem, so nothing passes through whole | 24 TOML rules + 13 formatters keep signal by meaning; remaining families are cut by position, so an error line can fall in the gap; rtk has 100+ per-command filters | Shipped 2026-09-18: per-family rules chosen by measured after-bytes (**T50.1**); table formatters (`docker ps` 3147→1190, `kubectl get` 4542→1731, `ps aux` 2341→870, each beating `Rule::default()` on the same fixture) (**T58.5**). |
| Reads | 4 modes at 2026-09-17 (6 by 2026-10-08: `full`, `lines`, `map`, `signatures`, `diff`, `stripped`), sha256 dedup, root guard, ~223 desc tokens for 12 tools (`rtok doctor`, 2026-09-18); no banner | lean-ctx: `diff` mode; token-optimizer: delta reads; lean-ctx re-read 13 tokens (rtok's "unchanged since" line is comparable) | Shipped as **T58.1** (2026-09-18): a changed file returns a unified diff against the archived previous read (plus `previous <id>` / `expand <id>`) instead of 9.5–17 K tokens again; measured 7.3 % of Read bytes (§2). |
| Model output (the code it writes) | typed `yagni` ladder 14/14 on fixtures; modes inside the 800-token budget | nothing targets the 96 % tool-input share | **Measured 2026-09-17 (T58.3):** `old_string` is 3.8 % of tool-input bytes and ≈ 1.3 % of output tokens, so an anchored `patch` tool (serena `replace_symbol_body`, lean-ctx `ctx_patch`) would move at most ~1 % of the output slice; not built (I-43 keeps the number). The output lever that remains is fewer and smaller writes — modes (T53.1) and the read side. |
| Injection / compaction | byte-stable 800-token budget; progressive-disclosure memory; on Claude Code a `PreCompact` checkpoint (prompts, paths, errors) and modes re-injected after the summary (T2.5) — rtk, headroom and caveman have nothing here | at 2026-09-17 the checkpoint existed on Claude Code only and carried no archive ids (both fixed by T58.2, 2026-09-18, see the next column) | Shipped as **T58.2** (2026-09-18): compaction events registered on Codex, Cursor and Copilot CLI (the hosts that have them) and the live archive ids added to the checkpoint. Gemini CLI was not a host on 2026-09-18 (it is since 2026-09-24); ZCode has no event. |
| Foreign MCP results | old ones shrink in the proxy live zone like any `tool_result` | fresh results of other servers pass whole (atlassian mcp-compressor wraps any server) | 2026-09-17 estimate: not worth it, MCP results were 15 K of 2.83 M (§2). The T59.4 re-measure the same day (§2, 885 sessions) put lean-ctx at ≈ 27 % of the tool table, so I-44 was promoted and shipped as T59.4 (2026-09-17). |
| Tool descriptions | 12 tools / ~223 tokens (`rtok doctor`, 2026-09-18); `doctor` prices every server | — | **Measured 2026-09-18 (T59.5):** on this host Tool Search is off (`ANTHROPIC_BASE_URL` set); MCP description tokens × API turns = **6.2 %** of session input (§2) → `proxy.tools_rewrite` ships, off by default. |
| Memory | agent-written, FTS5, no model calls, titles-first | claude-mem/mem0 have vectors (P29 landed hash-embed; no ONNX); Claude Code auto-memory is free on that host | Shipped as T59.7 (2026-09-18, from I-47): `doctor` names the host-native feature that duplicates a rtok surface, auto-memory included. |
| Code graph | 5 tools / 127 tokens (2026-09-18, T68.1 added `explore`), SQLite only, hook ≤ 10 ms | reference recall 0.351 vs LSP-grade (serena, codebase-memory-mcp hybrid LSP); 0.924 on 2026-10-04 after T387 re-landed the T52.5 type-position queries (§2, T8.8 table) | Shipped as T52.5 (re-landed as T387) / T30.2 (LSP optional). jCodeMunch's measured 96.5 % vs grep-read is the same claim class as `graph`; no new task. |
| Learning from history | `stats`, `report` rules (D24), `doctor --instructions` | caveman `learn`, lean-ctx mode predictor, context-budget plugin rank *sinks* and *recommend* | `report` already renders recommendations; shipped as T59.8 (2026-09-18, from I-48): a top-10 token-sink rule in `report`. |
| Sub-agents | — | lean-ctx `ctx_handoff`/`ctx_agent`; theme "sub-agent isolation" | Agent+Task results were 0.7 % of tool-result tokens on 30d (T59.6, §2); not a lever. I-46 was promoted as T59.6 and closed evidence-only on 2026-09-18 with that number; the `handoff` tool was not built for it. |
| Gateways / caches | 4 wires, usage capture, semantic cache off (P31: 0 hits at 0.99) | — | Nothing to add; bifrost/Portkey/LiteLLM are routing products. |
| Hosts | 11 hosts with a reversible installer at 2026-09-17 (24 on 2026-10-08, §26); per-host `support()` table | rtk/caveman list 30+ hosts (Windsurf/Cline/Aider/Qwen/OpenClaw/Hermes) | T48.8 (VS Code) is the only one with a measured user; the rest wait for a request. |

### 9.4 Ranking of the gaps by expected effect on this workload

Estimates, not measurements — each task's first step is the measurement that replaces the estimate.

1. **Anchored patch — measured and dropped (T58.3).** Output is 39 % of the Fable/Mythos bill and 96 % of output is tool input, but `old_string` is only 3.8 % of tool-input bytes (≈ 1.3 % of output tokens) over 925 sessions, so T58.4 was not built. `new_string` is 2.2× `old_string`: the model's output is the code it writes, not what it quotes back.
2. **Delta reads (T58.1) — shipped 2026-09-18.** Read is 15 % of tool-result tokens and the top-8 single results are all Reads; the dedup already handles unchanged re-reads, so the win is the changed-file re-read after an Edit. Counted from transcripts first: 7.3 % of Read bytes (§2), above the 3 % gate.
3. **Compaction checkpoint everywhere (T58.2) — shipped 2026-09-18.** Cheap (the T2.5 checkpoint and restore existed; the work was host registration and one more field); the effect is keeping the measured mode savings and `expand` reachability alive after compaction on Codex, Cursor and Copilot the way they already are on Claude Code.
4. **Filter families (T50.1, then T58.5) — both shipped 2026-09-18.** Real but small: JetBrains measured rtk at +7.6 % to 0 % on the bill, and rtok's default rule already caps every stem; the win is the error line that the positional cut drops. Data first (TOML rules), Rust only for table and grouped outputs.

Promoted: I-41 → T58.1, I-42 → T58.2, I-43 → T58.3 (measured; T58.4 dropped with the number). I-44–I-48 were not promoted on 2026-09-17 (each parked with its number in `ideas.md`), but all five were promoted the same day or the next and are closed: I-44 → T59.4 (foreign-MCP wrapper, 2026-09-17), I-45 → T59.5 (`tools[]` rewrite, 2026-09-18), I-46 → T59.6 (closed evidence-only, 0.7 %, 2026-09-18), I-47 → T59.7 (`doctor` overlap audit, 2026-09-18), I-48 → T59.8 (token-sink ranking in `report`, 2026-09-18).

## 10. Skill loading: where the tokens go (2026-09-17)

Question from the creator: how to spend fewer tokens and requests on loading skills (the
`SKILL.md` folders every host now reads). Sources: host docs (10.1, Haiku web survey), and
three dated measurements on this machine (10.2). Estimates are bytes/4 unless a row says
otherwise.

### 10.1 How each host loads a skill

| Host | Where | At session start | On invocation | Knobs that shrink the listing |
| --- | --- | --- | --- | --- |
| Claude Code | `~/.claude/skills/<n>/SKILL.md`, `.claude/skills/`, `<plugin>/skills/` (listed as `/plugin:skill`) | name + description of every listed skill in the system prompt (docs: "~100 tokens per skill") | the whole `SKILL.md` body; `references/`, `scripts/`, `assets/` only when the model reads them (script output enters context, script code does not) | `disable-model-invocation: true` (only `/name` by a human), project-scoped skills, plugin enable/disable |
| Cursor | `.cursor/skills/`, `.agents/skills/`, user equivalents | name + description | body on demand, resources lazily | `paths:` globs scope a skill to matching files; `disable-model-invocation` |
| OpenCode | `.opencode/skills/`, `~/.config/opencode/skills/`, Claude paths | Agent Skills standard (not documented in detail) | body on demand | `opencode.json` permission `allow` / `deny` / `ask` per skill pattern; `deny` hides the skill, custom agents can set `tools: skill: false`. No per-skill `disable-model-invocation` documented; unknown frontmatter fields are ignored (https://opencode.ai/docs/skills/, checked 2026-10-08) |
| Copilot CLI / VS Code | `.github/skills/`, `.agents/skills/`, `~/.copilot/skills/` | metadata for discovery | body when relevant or on `/name` | not documented as of 2026-10-08: no frontmatter field and no hide option (https://docs.github.com/en/copilot/concepts/agents/about-agent-skills) |
| Gemini CLI | `~/.gemini/skills/`, `.gemini/skills/`, `.agents/skills/`, extensions | metadata only | `activate_skill` tool loads the body | `/skills disable <n>` per session ("Prevents a specific skill from being used"); no frontmatter field; precedence built-in > extension > user > workspace (https://geminicli.com/docs/cli/skills/, checked 2026-10-08) |
| Codex / ChatGPT | `.agents/skills/`, plugins | not documented | not documented | `agents/openai.yaml` under `policy:` with `allow_implicit_invocation: false` (explicit `$skill` still works); no `SKILL.md` frontmatter field (https://learn.chatgpt.com/docs/build-skills, checked 2026-10-08) |

Docs: https://code.claude.com/docs/en/skills, https://cursor.com/docs/skills,
https://opencode.ai/docs/skills/, https://docs.github.com/en/copilot/concepts/agents/about-agent-skills,
https://geminicli.com/docs/cli/skills/, https://agentskills.io (spec: description ≤ 1024
chars, body ≤ 500 lines recommended). Every host does the same two-level load: a listing
that rides in every request, and a body that lands once per invocation and then stays in
the conversation for every later request of that session.

### 10.2 Measured on this machine

| What (date, command) | Result |
| --- | --- |
| Skills on disk (2026-09-17, `skillscan` over `~/.claude/{skills,plugins}`, `~/.codex/skills`, `~/.cursor/skills`, `~/.config/opencode`, `~/.copilot`, `~/.agents`) | 243 `SKILL.md`, 1,452,088 B (≈ 363 K tokens) of bodies; description median 200 chars; largest bodies 19–33 KB (`skill-creator`, `m5-onboard`, `skill-development`, `monitor-ci`, `imagegen`). Most of the 195 plugin-cache copies are marketplace clones, not installed. |
| What Claude Code actually lists (2026-09-17, `enabled.py` over `installed_plugins.json` + `~/.claude/skills`) | 25 user skills (2,888 description chars) + 41 skills from 5 enabled plugins (9,948 chars; claude-mem 18, claude-obsidian 15, ponytail 6, engram 1, slint 1) = 66 listed skills, 12,836 description chars ≈ 3.2 K tokens of descriptions, ≈ 4–5 K tokens with names and paths, in the system prompt of every request. Body bytes of the listed set: 214,605 (plugins) + user skills — loaded only on invocation. |
| Invocations (30 d to 2026-09-17, 890 transcripts, 81 sessions) | 26 `Skill` tool calls, 8 distinct skills (`slint` 8, `artifact-design` 7, `update-config` 4, `claude-api` 3, four × 1); 12 of 81 sessions (15 %) invoked any skill; 8 direct `Read`s of a `SKILL.md`; 115 slash-command messages (`<command-name>`), median 143 B — negligible. |
| Where the body lands (2026-09-17, `skillinj.py` over 173 transcripts in `~/.claude/projects`) | The `Skill` tool_result is 22 B (`Launching skill: <n>`); the body arrives as the **next user message** (`Base directory for this skill: …`): 17 bodies, median 8,863 B (≈ 2.2 K tokens), max 248,175 B (`update-config`, ≈ 62 K tokens in one message). `rtok stats` counts tool results, so it sees 2.5 KB where ≈ 150 KB entered. |
| What `rtok stats` now folds (2026-09-18, `rtok stats --json --since 30d`, T61.1 skills section over `~/.claude/projects`) | 24 injected bodies, 792,820 B (≈ 198 K tokens); `update-config` 391,824 B over 2 invocations, `claude-api` 248,816 B, `slint` 35,997 B over 8. The `resident` column — body bytes × the API requests that carried them — totals ≈ 96.8 MB over the 30 d window (the context-token-turns view of §10.3's "heavy tail"). |
| rtok hub skill (2026-09-18, `skills/rtok/SKILL.md` frontmatter; T71.3) | description **112 chars** (≤ 120); body **780 B** (≤ 2 KB); `disable-model-invocation` unset. `doctor` lists it from the same §10.1 user roots as any other skill once `rtok agents install <host>` has copied the hub. |

### 10.3 Where the cost is, ranked for this workload

1. **The listing, every request.** ≈ 4–5 K tokens of the cached prefix per request. At the
   measured 97.5 % cache-hit rate (§2, `rtok stats --since 30d`) it is mostly cache-read,
   but every change to the listed set — a plugin auto-update (`lastUpdated` 2026-09-15 on
   the installed plugins), a new user skill, an edited description — rewrites the whole
   prefix once per open session. Two levers: fewer listed skills, shorter descriptions.
2. **Bodies with a heavy tail.** Invocation is rare (26 in 30 d) but one body of 248 KB
   costs more than the listing does in 50 requests, and it stays in the session's context
   for every later request. A body over ~8 KB is almost always documentation pasted into
   `SKILL.md` instead of a `references/` file the model reads only when needed.
3. **Resources loaded through `Read`** are ordinary tool results: rtok's `read` plugin
   (dedup, modes) and the archive live zone already apply. When this was written
   (2026-09-17) skill bodies passed through no rtok surface: they are not tool results and not
   hook output. Since 2026-09-18 three surfaces reach them: the proxy archives old skill-body
   user messages outside the live zone (T61.2), Claude Code `PreToolUse(Skill)` can answer with
   a digest (T62.1) and OpenCode `tool.execute.after` shortens the `skill` tool output
   (T62.3); `stats` folds the bodies into a `skill` family (T61.1).
4. **Requests** are not the cost: a skill invocation is one tool call inside the turn, and
   the listing adds zero requests. The only request-shaped waste is a `Read` of a
   `SKILL.md` the host would have injected anyway (8 in 30 d).

### 10.4 Techniques, with the lever each pulls

| Technique | Lever | Evidence / limit |
| --- | --- | --- |
| Description ≤ 120 chars, one sentence: what it does and when to pick it | listing | median here is 200 chars; a 120-char cap on 66 skills is ≈ −1.3 K tokens per request, byte-stable once set |
| `disable-model-invocation: true` for skills only a human runs (setup, onboarding, release checklists) | listing | Claude Code and Cursor document it (Cursor re-checked 2026-10-08); Codex has `allow_implicit_invocation: false` in `agents/openai.yaml` instead; the skill keeps working as `/name`. Other hosts: not documented as of 2026-10-08 |
| Project-level skills for project-only knowledge; user-level only for cross-project ones | listing | Claude Code lists project skills only inside that project; Cursor `paths:` scopes further |
| Enable plugins per project, not globally | listing | 41 of the 66 listed skills here come from 5 plugins; a plugin unused in a repo still lists all its skills there |
| Body ≤ 2 K tokens: hub `SKILL.md` + `references/*.md` read on demand; scripts in `scripts/` (only their output enters context) | body | the 248 KB `update-config` body is the ceiling case; agentskills.io recommends ≤ 500 lines |
| One skill per task family, not per sub-step | listing + body | fewer lines in the listing; the body loads once instead of three times |
| Do not restate `CLAUDE.md` in a skill | body | `CLAUDE.md` is already in every request; a skill that repeats it pays twice |
| Pin plugin versions / update in one batch | cache | each listing change is a full prefix rewrite for every open session |
| Measure before trimming | all | on 2026-09-17 `rtok stats` could not see skill bodies (10.2); shipped as T61.1 (2026-09-18, I-49): they are a `skill` row now |

### 10.5 What rtok can add (ideas I-49–I-52; all four promoted and shipped 2026-09-18 as T61.1, T61.3, T61.2 and T71.3)

- **I-49 `stats` skill row.** Count the user message that follows a `Skill` tool_use (marker
  `Base directory for this skill:` or the `/plugin:skill` header) as a `skill` family: calls,
  bytes, mean, p95, per skill name. Without it the 150 KB measured above is invisible.
- **I-50 `doctor` skill audit.** For every skill the host lists: description chars, body
  bytes, invocations in the window; flag descriptions > 200 chars, bodies > 8 KB, skills
  never invoked in 30 d, and skills that duplicate a rtok surface (T59.7 already does the
  host-feature half). Output is advice, never an edit.
- **I-51 live-zone shrink for skill bodies.** The `archive` live zone already replaces old
  tool results with an `expand <id>` pointer; the same matcher on a skill-body user message
  older than N turns would drop the 248 KB case to a pointer for the rest of the session.
  Gate: measure how many requests carry a skill body (I-49 first).
- **I-52 rtok's own skill (the creator's request).** Design constraint from this section:
  description ≤ 120 chars, body ≤ 2 KB hub pointing at `docs/`, `disable-model-invocation`
  off (the model must pick it), installed and removed by `rtok agents install/remove`
  together with the host plugin, one per host that supports the format (10.1).

### 10.6 Open questions

- **T71.4 (2026-09-18):** `cargo test --test skill_listing` on
  `tests/fixtures/proxy/skills_listing_request.json` captured through `rtok proxy`
  (`call_io`): `<available_skills>` block **452 B** for **3** listed skills;
  **92 B** framing per skill beyond its description (name + `fullPath` + tags; not the
  docs' "~100 tokens per skill"). `doctor::SKILL_LISTING_FRAMING_BYTES` carries the
  constant; listing bytes per request = description bytes + `N × 92`.
- Whether hosts other than Claude Code and Cursor honour `disable-model-invocation` in the
  listing is not documented (10.1).
- **T399 (2026-10-08):** re-checked. OpenCode, Copilot, Gemini and the Agent Skills spec
  (https://agentskills.io/specification: frontmatter is name, description, license,
  compatibility, metadata, allowed-tools) have no per-skill disable field; Codex's equivalent
  is `allow_implicit_invocation: false` in `agents/openai.yaml`, not in the frontmatter. So the
  flag is only honoured by Claude Code and Cursor in the frontmatter form. The `doctor` skill
  advice the card names is not in origin/main `699b1279f`: `src/doctor.rs` flags only
  description length, body size and never-invoked, with no advice that cites the flag. No
  code change follows from this check; the creator decides whether the flag is worth adding.

### 10.7 Working around the blind spot (2026-09-17)

Two facts fix it. In the transcript the injected body is its own record: `type: "user"`,
`isMeta: true`, `turnCompanion: true`, `sourceToolUseID: <id of the Skill tool_use>`, text
`Base directory for this skill: <path>\n\n<SKILL.md body>` (`skillrec.py`, 2026-09-17,
two invocations checked). On the wire it is a plain user text block that follows the
`tool_result` `Launching skill: <name>` of that same `tool_use_id`, and it is re-sent whole
in every later request of the session. Three surfaces can act, in this order:

| Surface | What it can do | Limit |
| --- | --- | --- |
| `stats` (transcripts) | Count the body exactly: join the `isMeta` record to its `Skill` tool_use through `sourceToolUseID`; family `skill`, one row per skill name, bytes + est. tokens, and a "resident" column = bytes × later requests of the session (what the model actually paid for). | Claude Code only; other hosts' transcripts are not read (T49.2). |
| `proxy` / `archive` | Shrink the body outside the live zone the way old tool results are shrunk: key = the preceding `Launching skill` `tool_use_id` (byte-stable pointer), archive the body once, replace it with `[archived <id>: skill <name> · N lines · expand(<id>)]`. Lossless: `expand <id>`, or the model re-invokes the skill. The `keep_turns` boundary already decides "old". | Only when the proxy is in the chain (`ANTHROPIC_BASE_URL`); hooks never see the body (`UserPromptSubmit` carries the human prompt only, `PostToolUse(Skill)` fires before the injection). |
| `doctor` (advice) | Prevent at the source: list what the host lists, flag description > 200 chars, body > 8 KB, never invoked in 30 d, and say which lever applies (`references/`, `disable-model-invocation`, project scope). | Advice only — rtok never edits a user's skills. |

What does not work: a hook cannot intercept or rewrite the injection (it is not a tool
result, and PostToolUse can only add context); the MCP surface never sees it; a compaction
checkpoint (T2.5) does not carry skill bodies, so after auto-compaction the body is gone
and the model re-invokes — which is the cheap outcome, not a loss.

Order: measure first (T61.1), shrink behind the measurement (T61.2, gate: the `resident`
column shows skill bodies above 2 % of input tokens on a real window), advise in parallel
(T61.3). All three shipped on 2026-09-18. The 248 KB `update-config` body alone is ≈ 62 K tokens resident in every request of
that session; at the measured 97.5 % cache hit it is cache-read, at each cache miss it is
a full re-send.

### 10.8 What the host plugins can do (2026-09-17)

§10.7 said hooks never see the injection. They do not see it, but on two hosts a plugin
can act before it or on its carrier, and on one host the compaction checkpoint can carry
the fact that a skill was loaded. Checked against `src/agents/claude/mod.rs` (`ENTRIES`:
`PreToolUse` on `Bash`/`Read`, `PostToolUse *`, `PreCompact`, `PostCompact`, `SessionStart`)
and `plugins/opencode/rtok.ts` (`tool.execute.after` already rewrites bash output).

| Host | Interception point | What rtok can do there | Task |
| --- | --- | --- | --- |
| Claude Code | `PreToolUse` with matcher `Skill` (`tool_input.skill = <name>`), fires before the body is injected; the hook may answer `permissionDecision: deny` with a reason the model reads | for a body over a byte cap: archive it, answer deny with a digest (headings + first line per section) and the `expand <id>` trailer — the model gets the map, not the 248 KB, and pulls sections on demand. Off by default: a denied skill does not apply its frontmatter (`allowed-tools`, `model`, `context`), so skills carrying those keys always pass | T62.1 (shipped 2026-09-18) |
| Claude Code | `PreCompact` reads the transcript (T2.5 checkpoint already extracts prompts, paths, errors); the `isMeta` + `sourceToolUseID` records name the skills loaded so far | the restore line after compaction lists them with sizes so the model re-invokes only what the next step needs, instead of guessing which skill it had | T62.2 (shipped 2026-09-18) |
| OpenCode | `tool.execute.after` (`plugins/opencode/rtok.ts`) receives every tool's output, including the tool that loads a skill if OpenCode delivers skills as a tool call | shorten the body the way bash output is shortened, with an archive id so the full text is one `expand` away | T62.3 (step 1 verified the delivery path: OpenCode's native `skill` tool; shipped 2026-09-18) |
| Cursor, Codex, Copilot, Gemini | no hook fires on skill activation (Cursor hooks: shell, MCP, file read; Codex: none; Gemini: `activate_skill` is a tool, hooks not documented for it) | nothing on the plugin side; the proxy path (T61.2) is the only lever | — |

`PostToolUse(Skill)` stays useless for this: it can only add context, and the body is
already on its way. `UserPromptSubmit` carries the human prompt only.

## 11. rtk's four strategies and sqz, each against rtok (2026-09-18)

Sources: rtk README ("four strategies", "Does RTK break Claude's prompt cache?") and sqz README
(github.com/ojuschugh1/sqz, fetched 2026-09-18: Rust, ELv2, 625 stars, 265 commits; self-reported
"178,442 tokens saved across 3,003 compressions, 24.7 % avg reduction, up to 92 % with dedup").
Every claim on their side is vendor-reported; nothing here was re-measured. rtok side checked
in `src/plugins/cmd/{rules,formatters}.rs`, `src/plugins/guard/mod.rs`, `src/agents/`.

| Their feature | rtok today | Gap | Task |
| --- | --- | --- | --- |
| rtk smart filtering (noise, comments, boilerplate) | `keep`/`drop` patterns per rule, `BUILTIN_KEEP`, 13 formatters, raw archived first | Coverage, not mechanism: 24 rules + 13 formatters vs ~80 (rtk) / 45+ (sqz) | T50.1, T58.5 |
| rtk grouping (files by directory, errors by type) | none — `ls`/`find`/`tree` take 40 lines | generic grouping pass | T64.1 |
| rtk truncation | `max_lines`/`head`/`tail`, lossless (`expand <id>`) | rtok is ahead: rtk drops, rtok archives | — |
| rtk / sqz dedup of repeated log lines | `dedupe` folds adjacent identical lines to `(×N)` | non-adjacent, timestamp-normalised | T64.2 |
| rtk "does not break the prompt cache" paragraph | byte-stable inject, live-zone proxy rewrites, `report` cache section, 98.1 % hit rate on this machine | no page says it | T64.3 |
| sqz content-hash dedup (`§ref:HASH§`, 13 tokens) | `guard` dedups by input key only | same bytes from a different call paid twice | T65.1 (1.9 % of result bytes, 2026-09-18, `rtok stats --since 30d`, 924 sessions — above the 1 % gate) |
| sqz structural summaries (imports + signatures, ~70 %) | `read` modes via tree-sitter (`map`, `signatures`) | none | — |
| sqz JSON pipeline (nulls, arrays) | JSON compact then line cut (0.21 % of Bash bytes, 30 d, this machine) | — | T65.2 |
| sqz table compaction | none | padding collapse | T65.3 |
| sqz safe mode (traces, secrets pass whole) | single `panic`/`traceback` lines kept, frames cut; secrets never redacted | keep the block | T65.4 |
| sqz hosts: Windsurf, Cline, Gemini CLI, Kiro, Zed, Copilot CLI; browser and IDE extensions | 12 hosts in `src/agents/` on 2026-09-18 (no Cline, Kiro, Gemini; Cline and Gemini CLI are hosts since 2026-09-24, 24 hosts on 2026-10-08, §26); no extensions | hosts on request; extensions out of scope (one binary, D21) | — |
| sqz `gain` / `stats --breakdown` | `stats`, `report`, `dashboard`, one ledger | none | — |

Order by expected effect on this workload (§2: Bash 35 % of result tokens): T65.4 and T64.3
are cheap and close a correctness / documentation hole; T65.1 measured **1.9 %** of result
bytes as same-session SHA-256 repeats (`rtok stats --since 30d`, 924 sessions, 2026-09-18)
and proceeds; T65.2 still starts with a measured share; T64.1, T64.2, T65.3 are fixture-gated.

T65.2 gate (2026-09-18, this machine, 30 d): `measurements` where `plugin = 'cmd'` and
`before_bytes > 0`, body = archive file named by `ref_id` after the first `:`,
`json.loads` of the UTF-8 body. 4 / 200 rows, 5 776 / 2 792 960 B = **0.21 %**.
(`~/.rtok/rtok.db`, read-only; the same window’s whole `archive` table is 18.66 % JSON —
MCP/read blobs, not Bash.) The rewrite still ships: the Done line is the compact pass, not
a share floor.

## 12. recursive-llm (RLM), against rtok (2026-09-18)

Source: github.com/grishahq/recursive-llm (Python library, MIT, 604 stars, v0.4.0, last commit
2026-09-03); one Haiku agent read the README, `src/rlm/{core,repl,prompts,budget,stats}.py` and
`DOCUMENT_EVALUATION.md`. Self-reported numbers, not re-measured here: on 100 K-character
documents RLM cut tokens 61–78 % against direct completion (gpt-4-mini $0.0089 → $0.0048 with
3/3 correct vs 0/3; DeepSeek V4 Flash −61 %, 2/3 vs 0/3) at 5–10× the latency.

The idea: the document never enters the prompt. It sits as a `context` string in a sandboxed
Python REPL; the model writes code (`len(context)`, `re.search`, slicing) and gets only the
results back; `llm_query` / `rlm_query` run a child model on a slice, bounded by depth,
iterations and a `RunBudget` (calls hard; tokens and cost soft; wall clock). The system prompt
forbids answering before searching the context. Library only: no CLI, no MCP, no hooks.

| RLM feature | rtok today | Gap | Task |
| --- | --- | --- | --- |
| Context outside the prompt, a pointer with its size in the prompt | `cmd` trailer `[rtok <id> · N lines]`, `archive` live-zone pointer with head/tail and est. tokens, `read` cap | same shape | — |
| `re.search` over the context | `expand --grep` is a substring match that prints bare lines | a hit has no position, so nothing can follow but a full expand | T67.1 |
| Slice around a hit (`context[i-500:i+500]`) | `expand --lines a-b` | with T67.1 it takes two calls; every call is a turn that re-reads the prompt | T67.2 |
| Child model on a slice (`rlm_query`) | the host's Agent tool plus `expand <id>`; `handoff` (T59.6) closed at 0.7 % of tool-result tokens | nothing on rtok's side — rtok is not the agent loop; re-open T59.6 only if a later `rtok stats` Agent/Task share is ≥ 5 % | — |
| `RunBudget` hard/soft caps on calls, tokens, cost, time | none; hosts auto-compact; `stats --price`, `report` | not a saving lever for a tool outside the loop; parked as I-55 | — |
| "Search before you answer" system prompt | `inject` modes and T53.1 nudges; the T62.1 skill digest tells the model to `expand --grep <heading>` | none | — |
| REPL snapshot cap (1 MB) | `mcp.max_result_chars`, `read` cap with archive id | none | — |
| Per-depth usage tracker, trajectory JSONL | `calls` / `measurements` ledgers, `--json` (T60.1) | none | — |
| Benchmark: accuracy and cost vs direct | `rtok bench` cost per passed task | none | — |

What transfers is the loop, not the runtime: rtok already externalises every large payload
behind an id; T67.1 gives a grep hit a position and T67.2 folds the slice into the same call.

## 13. engram, feature by feature against `memory` (2026-09-18)

Source: github.com/Gentleman-Programming/engram `README.md` and `DOCS.md` read on 2026-09-18
(Go, MIT; the 18-tool / ~1 865-description-token row in §4 and `docs/comparison.md` is the
`rtok doctor` measurement of 2026-09-09). Creator request: what is missing in rtok, add what
is useful. engram's own docs carry no token-saving number; its value is recall, not bytes.

| engram feature | rtok `memory` today | Verdict | Where |
| --- | --- | --- | --- |
| `topic_key` upsert: same `project + scope + topic_key` updates the row, `revision_count++` | every `mem_save` inserted when this was written (2026-09-18); a re-saved decision left two rows with one title in the 5-title recall. Shipped as T66.1 (2026-09-18): `mem_save` updates the `(project, kind, title)` row in place and returns `{id, updated}` | adopt, zero-LLM, no schema: the title is the key, upsert on `(project, kind, title)` | T66.1 (shipped 2026-09-18) |
| Git Sync: gzipped JSONL chunks + manifest, `engram sync --import` | `memory import <file.jsonl>` exists (T6.3); nothing produced that file from `rtok.db` until T66.2 shipped `rtok memory export` (2026-09-18; T304, 2026-09-28, made it skip retired notes) | adopt the missing half: `memory export` in the shape `import` reads; no chunk manifest (a file in git is the manifest) | T66.2 (shipped 2026-09-18) |
| `mem_context` at session start: pinned + recent observations + sessions + prompts, 16 KiB default budget | SessionStart recall: 5 titles + ids ≤ 200 tokens; compaction checkpoint ≤ 400 tokens, same session only; SessionEnd writes `session:<id>` (T71.2); `startup_recall` restores the newest project note on `source=startup`, off by default | keep rtok's shape (D5 budget, titles not bodies); handoff stays off until a P7-style A/B. Measured 2026-09-18: `rtok stats --since 30d` → 0/939 sessions with a joinable checkpoint note (25 legacy `kind = checkpoint` rows with no session suffix, so not joinable to a session; they sit under project `rtok`, §14.1) | T71.2 |
| `pinned` observations first in context | recency only | parked; `kind = "pin"` would do it without a column | I-57 |
| project identity from the normalised `origin` remote, `.engram/config.json` override, child-repo scan | git-root basename | parked; one checkout per repo is the workflow here | I-58 |
| `mem_update(id)` | none when written; covered by T66.1 (re-save the same title), and a `mem_update` MCP tool exists since (`src/plugins/memory/mod.rs`) | covered by T66.1: re-save the same title | — |
| `normalized_hash` dedupe on save | `import` dedupes by body sha256; `mem_save` did not until T66.1 (2026-09-18) | covered by T66.1 (identical re-save is a no-op update) | — |
| `scope` project / personal / global | `project` column, `NULL` = no project | not needed: recall filters by project; a global note is a `project = NULL` row | — |
| `mem_judge` / `mem_compare` / `mem_review`: relations (`supersedes`, `conflicts_with`, …) judged by the model, `judgment_required` envelopes | none | rejected: every judgment is model output spent on bookkeeping, and the tool descriptions ride every turn (D15 target: fewer description tokens than engram) | — |
| `mem_session_summary` (mandatory before "done": goal, discoveries, next steps, files) | PreCompact checkpoint extracted mechanically from the transcript (prompts, paths, errors, skills) | rejected as a protocol; an agent may still `mem_save` a summary note by hand | — |
| `mem_capture_passive` (`## Key Learnings:` sections of the model's own output) | none | rejected: parses text the model already paid for; the agent-written note is the same bytes without a parser | — |
| `expires_at`, `review_after`, `duplicate_count`, `last_seen_at` lifecycle columns | none | not needed at note counts recall shows (5 titles); revisit with a measured stale-hit rate | — |
| HTTP API, TUI, cloud replication, Postgres backend | `rtok web` / `rtok tui` read the same store (D27); no cloud | out of scope (D8: one SQLite file) | — |
| optional embeddings beside FTS5 | `[plugins.memory.embed]`, off by default (P29) | parity | — |

Net: two tasks (T66.1, T66.2), three ideas (I-56–I-58), nothing that adds an MCP tool — the
description column stays at 3 memory tools.

## 14. graymatter, against rtok's `memory` (2026-09-18)

Source: github.com/angelnicolasc/graymatter (Go, MIT, one ~10 MB static binary; bbolt +
chromem-go in `.graymatter/gray.db`; MCP server, CLI and importable library; README fetched
2026-09-18). Every number below is graymatter's own (`go run ./benchmarks/token_count`,
keyword embedder, no LLM); nothing was re-measured here. rtok side checked in
`src/plugins/memory/mod.rs`, `src/store/mod.rs` (`notes`, `list_note_titles`, `search_notes`),
`src/store/embed.rs`, `migrations/0001.sql`, `src/agents/claude/mod.rs` (`ENTRIES`),
`src/web/model.rs` (`config_fields`).

Its claims: tokens per session against full-history injection ~80 → ~80 (1 session),
~630 → ~550 (10), ~1 880 → ~550 (30), ~6 960 → ~670 (100, "90 %"); a fact planted 96 sessions
ago retrieved 83 % of the time; superseded facts returned 0 %. The baseline is "re-inject the
whole history", which no coding host does, so the 90 % is not a bill delta; the two recall
numbers are the useful ones. rtok's own plant-and-recall numbers are the T69.3 table below — never graymatter's 83 %.

| Their feature | rtok today | Gap | Task |
| --- | --- | --- | --- |
| Hybrid recall: vector + keyword + recency, top-8, per-signal receipts | FTS5 BM25; optional hash-embed RRF (P29); SessionStart = newest 5 ids of the project; no recency, no receipts | ranking by age and use — T69.2 closed without a scorer on 2026-09-18 (no project exceeds `recall_titles`, §14.1) | T69.2 (closed) |
| 30-day decay half-life; never hard-delete; pinned facts exempt | none when written (2026-09-18); shipped as T69.1 the same day: retire, supersede, pin, never delete | lifecycle | T69.1 (shipped 2026-09-18), T69.2 (decay, closed without code) |
| `revise` / `forget` as tombstones; corrections recorded | insert-only when written; the in-place update by title shipped as T66.1 ("`mem_save` updates a note in place") and retire + supersede as T69.1 (both 2026-09-18) | retire + supersede | T69.1 (shipped) |
| Benchmark: tokens/session vs full injection, plant-and-recall, superseded = 0 | FTS5 and P29 hybrid 20/20 at N=1/10/30/100; superseded 0; SessionStart 100 B vs 371 866 B full injection at N=100 (`tests/memory_bench.rs`, 2026-09-18) | — | T69.3 |
| Claude Code hooks: SessionStart facts + conventions; UserPromptSubmit top-3 + `remember:`; PreCompact checkpoint; SessionEnd checkpoint + consolidation; errors to `hooks.log`, never break the session | SessionStart titles (T6.2); PreCompact checkpoint (T2.5); SessionEnd `session:<id>` note (T71.2, restore off by default); fail open ≤ 10 ms; nothing on UserPromptSubmit when written, `remember:` prompts save a note since T69.5 (2026-09-18; per-turn recall stays off, `prompt_recall = 0`, §27) | `remember:`; per-turn recall (A/B) | T69.5 (shipped); I-56 (engram `mem_context`) |
| `context-sync`: budgeted managed block in CLAUDE.md / AGENTS.md, hand-edit detection, backup | none when written (hook injection only; hosts without a SessionStart hook get no recall); `rtok memory sync` shipped as T69.6 (2026-09-18) | a sync command | T69.6 (shipped) |
| `status` / 4-tab `tui`: facts, KB, recall counts, health, weights | Memory page showed two config keys and there was no `memory status` when written; shipped as T69.4 (2026-09-18) | store rows on the page | T69.4 (shipped) |
| Knowledge graph: entities, co-mentions, Obsidian export, HTML force graph | `graph` is the code index | — | I-72 |
| Consolidation: summarise + decay + prune + extract (Ollama; OpenAI / Anthropic / keyword fallback) | no LLM (P28 is Later) | the mechanical half only | T69.1 / T69.2; I-73 |
| Embedding chain Ollama → OpenAI → Voyage → keyword | hash-embed local or `openai` (P29) | — | I-75 |
| `export --format obsidian` | JSONL import (T6.3); JSONL export is the memory card "`rtok memory export`" | markdown | I-74 |
| MCP wiring: Claude Code, Cursor, Codex, OpenCode, Antigravity, Windsurf, VS Code Copilot | 12 hosts in `src/agents/` on 2026-09-18; VS Code (T48.8) and Antigravity are hosts since; Windsurf is one on 2026-10-08 (24 hosts, §26) | — | — |
| Security: loopback + bearer on network surfaces; recalled facts fenced, never in the system prompt | `rtok mcp` is stdio; recall is `id title` lines in the hook's `additionalContext`, bodies only via `mem_get` | — | — |
| Go library in three lines | `rtok-plugin-sdk` (D25) | — | — |

### T69.3 memory recall bench (2026-09-18)

`cargo test --test memory_bench -- --nocapture`. Seeded in-memory store: N sessions × 6 filler notes of realistic length, 20 planted facts at known offsets, 5 revised later (T69.1). Query = eight content words from the live body. `search_limit` = 5. No network, no LLM.

`half_life_days = 30` is N/A: T69.2 closed without ranking code (0 live notes on that machine; no `uses` / `last_used` columns, no scorer). P29 hybrid (`embed.enabled`, hash-embed RRF) ran.

| N | FTS5 hit | hybrid hit | superseded returned | SessionStart recall bytes | full live-body injection bytes |
| --- | --- | --- | --- | --- | --- |
| 1 | 20/20 | 20/20 | 0 | 95 | 6 331 |
| 10 | 20/20 | 20/20 | 0 | 95 | 39 566 |
| 30 | 20/20 | 20/20 | 0 | 100 | 113 240 |
| 100 | 20/20 | 20/20 | 0 | 100 | 371 866 |

T69.2's default stays off: FTS5 already hits 20/20 at N=100 without extra recall bytes, and there is no scorer to turn on. Floors are the FTS5/hybrid columns in `tests/memory_bench.rs`; a drop fails the test.

Where rtok is ahead: one ledger — `Measurement` rows plus proxy `usage` — where graymatter's
numbers are its own bench; FTS5 in the same SQLite file as every other plugin (D8) and three
memory tools inside the measured 12-tool / ~223-token surface (`docs/comparison.md` §2,
`rtok doctor` 2026-09-18); the
`expand` path and the compaction checkpoint with modes re-injected (T2.5); titles → ids → bodies
where graymatter injects the top-K bodies.

Order by expected effect: T69.1 first (a wrong fact recalled is worse than a missing one),
T69.3 (landed 2026-09-18: FTS5/hybrid 20/20, Gate P6 now has a floor), T69.4 (cheap; feeds T69.2 step 1), then T69.2 / T69.5 /
T69.6 behind their gates.

### 14.1 Live notes vs `recall_titles` (T69.2, 2026-09-18)

Installed `rtok 0.1.1` (`dbcc7a162`) has no `memory status`. Counted with T69.4's
`Store::memory_note_aggs` query (`kind NOT LIKE 'checkpoint%'`) on this machine:

`sqlite3 ~/.rtok/rtok.db "SELECT COALESCE(project, '-'), SUM(CASE WHEN retired IS NULL THEN 1 ELSE 0 END) FROM notes WHERE kind NOT LIKE 'checkpoint%' GROUP BY project;"`

Result: **0 rows**. Live notes: **0**. Projects with more than `[plugins.memory] recall_titles` (5): **0**.
The same file holds 25 `checkpoint` rows under project `rtok` (title `compact`, none retired);
T69.4 excludes them from the live count. SessionStart still injects those titles
(`list_note_titles` does not filter kind). Ranking order among live facts never matters here,
so T69.2 ships no scorer and no `uses` / `last_used` columns. Since then `memory status` also leaves out `session:<id>` handoff notes and `memory export` leaves out retired notes (T304, 2026-09-28).

## 15. What a host plugin can do that rtok's own surfaces cannot (2026-09-18)

Creator request: go through §1–§13 and `ideas.md` for everything parked because rtok's three
surfaces (D2: hook, MCP, proxy) cannot reach it, and check whether a **host plugin** —
`plugins/<host>/`, linked by `rtok agents install <host>` (D21) — can.

Method: one Haiku web agent read the three plugin APIs rtok already links against
(https://pi.dev/docs/latest/extensions, https://opencode.ai/docs/plugins/,
https://cursor.com/docs/agent/hooks) and answered, per event, whether a return value may
replace a tool result, block a call, change the messages sent to the model, or run at
compaction. Vendor docs only — nothing re-measured here, and the scan disagrees with
`src/agents/cursor/mod.rs` on Cursor's event names (the installer writes
`beforeShellExecution` / `afterShellExecution`; the scan also reports `preToolUse` /
`postToolUse`). **Every task below therefore starts with a step that re-verifies the API
against the host's current docs and one real session, and closes with that finding if the
capability is not there.**

### 15.1 The three constraints that park work today

| Constraint | Where it is stated | What it blocks |
| --- | --- | --- |
| PostToolUse can only add context, never modify a tool result | §3, D2, `plan.md` working agreement | On Claude Code only `Bash` shrinks (PreToolUse rewrite → `rtok run`); `Read`, `Grep`, `Glob`, `WebFetch`, `Task` and every foreign MCP result enter context whole |
| The live zone needs the proxy | §9.3, `archive` plugin docs | A host with no base-URL setting (Cursor, Claude Desktop, pi, Windsurf, Zed, ZCode, Kimi, Copilot) never shrinks an old tool result — the lever §1 ranks first |
| A host without hook events reaches no hook plugin | `src/agents/<host>/README.md` module tables | `inject`, `guard` unreachable on pi, OpenCode, Codex; `guard` unreachable on every MCP-only host (as of 2026-09-18; `guard` has reached pi and OpenCode through their plugins since T70.5, the same day) |

### 15.2 What each plugin API offers against those constraints

Scan of 2026-09-18, unverified against a running host. "—" is "not documented".

| Capability | pi extension | OpenCode plugin | Cursor plugin |
| --- | --- | --- | --- |
| Replace a tool result | `tool_result` returns `content` for **every** tool | `tool.execute.after` mutates `output` (rtok uses it for bash; other tools — not documented) | MCP results only, per the scan (`updated_mcp_tool_output`); shell output not replaceable |
| Block a call with a reason | `tool_call` → `{block, reason}` | `tool.execute.before` (throw) | `beforeShellExecution` / `beforeMCPExecution` → `permission: deny` |
| Rewrite the messages sent to the model | `context` fires before **each** LLM call with the message array | — | — |
| Change the system prompt | `before_agent_start` | — | — |
| Inject context at session start | — | — | `sessionStart` → `additional_context`, `env` |
| Act at compaction | `session_before_compact` may supply the summary or cancel | `experimental.session.compacting` may replace the prompt | `preCompact` observational |
| Register a tool without MCP | `pi.registerTool` | — (MCP entry does it) | — (MCP entry does it) |

### 15.3 What that unblocks, and what it does not

| Parked item | Why it was parked | Host plugin that reaches it | Task |
| --- | --- | --- | --- |
| Shrink results of tools other than Bash on a host with no proxy | PostToolUse cannot modify results (§3) | pi `tool_result` (all tools) | T70.1 (shipped 2026-09-18) |
| `archive` live zone without a proxy | proxy-only (§9.3) | pi `context` rewrites the message array per call — the same job the proxy live zone does | T70.2 (shipped 2026-09-18) |
| pi reaches only `measure`, `cmd` (`src/agents/pi/README.md`) | "pi philosophy is no MCP" | `pi.registerTool` is not MCP: `read` / `search` / `graph` / `memory` can be pi tools | T70.3 (shipped 2026-09-18) |
| Foreign MCP results the **host** launched | T59.4 wraps only servers rtok itself spawns (`rtok mcp -- <argv>`); lean-ctx measured at ≈ 27 % of tool-result bytes over 30 d (§2) | Cursor's post-MCP output replacement | T70.4 (shipped 2026-09-18) |
| `guard` unreachable on pi and OpenCode | no hook events on either host | pi `tool_call` block, OpenCode `tool.execute.before` | T70.5 (shipped 2026-09-18) |
| Compaction outside Claude Code (T58.2 (a)) | T58.2 registers host **hook** events; pi and OpenCode have none | pi `session_before_compact`, OpenCode `experimental.session.compacting` — both stronger than a note: they own the summary | T70.6 (shipped 2026-09-18) |
| `inject` claimed reachable on Cursor | `reaches()` counts declared surfaces, and Cursor supports hooks — but the installer registers only the two shell events, which never carry a session start or a prompt | Cursor `sessionStart` / `beforeSubmitPrompt` | T70.7 (shipped 2026-09-18) |
| Skill bodies on pi | §10.8 lists Claude Code (T62.1) and OpenCode (T62.3) only | pi `context` (T70.2) drops an old body from the array like any other block; no separate task | — |
| Sub-agent handoff (I-46), strict-mode Read deny (I-82), `RunBudget` (I-55) | parked on a **measured share**, not on a missing surface | — | stay parked |

Reading (2026-09-18; T70.1–T70.7 all shipped that day, so pi, OpenCode and Cursor now reach
more than the lines below say): two of the three constraints are host-plugin-shaped, and pi
is the host where the gap was widest — it reached two plugins and its extension API is the
most capable of the three. The proxy stays the only path on Codex, Claude Desktop, Windsurf, Zed, ZCode
and Copilot, which have neither a plugin directory nor the events. Kimi has a plugin
store (`plugins/managed/`, T86) whose hooks and MCP server rtok's installer treats as
the singleton instead of its own tables.

### T50.1 default-rule families (2026-09-18)

Command: `rtok stats` on this machine (916 Claude Code sessions, `since` default). The `bash` table now has a `filter` column (`formatter` / `rule` / `default`). The `bash_default` table ranks stems where `cmd` `Measurement.kind = rule` still used `Rule::default()` (before the T50.1 rules landed), sorted by summed `after_bytes`.

Top 20 `bash_default` stems by filtered after-bytes:

| # | stem | `cmd` rule rows | after B |
| --- | --- | ---: | ---: |
| 1 | mise | 37 | 41,423 |
| 2 | gh | 4 | 13,809 |
| 3 | bash | 13 | 12,484 |
| 4 | cd | 4 | 6,334 |
| 5 | H=$(ls | 5 | 5,777 |
| 6 | awk | 4 | 4,954 |
| 7 | just | 2 | 4,018 |
| 8 | df | 3 | 3,451 |
| 9 | # | 3 | 3,094 |
| 10 | for | 3 | 2,881 |
| 11 | bv0thal3q.output; | 1 | 2,629 |
| 12 | head | 4 | 2,643 |
| 13 | lean-ctx | 2 | 1,552 |
| 14 | mkdir | 1 | 2,435 |
| 15 | sqlite3 | 20 | 8,556 |
| 16 | printf | 1 | 493 |
| 17 | if | 1 | 602 |
| 18 | diff | 1 | 150 |
| 19 | build.rs | 1 | 136 |
| 20 | rmcp-3.2.0 | 1 | 101 |

T50.1 added `[stem]` rules (and golden fixtures) for: `gh`, `pip`, `uv`, `python`, `python3`, `go`, `aws`, `mvn`, `gradle`, `dotnet`, `tsc`, `eslint`, `brew`, `apt`, `cmake`. T58.5 shipped table formatters (one row per object, `kind = formatter`) that beat `Rule::default()` on those fixtures: `docker ps` 3147→1190 vs rule 1311, `kubectl get` 4542→1731 vs rule 1770, `ps aux` 2341→870 vs rule 990 (`tests/cmd_golden/{docker_ps,kubectl_get,ps_aux}`).

### T386 re-measure (2026-10-06)

Command: `rtok stats` on this machine (`stats.since` = `30d`, header `sessions 288`, 2026-10-06), binary built from the T386 tree so `filter_kind` sees the new `df` / `sqlite3` formatters and the `[awk]` rule. `bash_default` ranks stored `cmd` measurements with `kind = rule` whose stem is still on `Rule::default()`, sorted by summed `after_bytes`. A row stored as `mise` or `just` stays there: the peel applies to new runs when the inner command is on the line, and it does not rewrite old families.

Top 20 `bash_default` stems by filtered after-bytes:

| # | stem | `cmd` rule rows | after B |
| --- | --- | ---: | ---: |
| 1 | cd | 15,458 | 17,969,939 |
| 2 | export | 665 | 1,183,761 |
| 3 | tail | 320 | 761,372 |
| 4 | scratchpad; | 480 | 571,428 |
| 5 | mise | 375 | 424,902 |
| 6 | for | 331 | 360,051 |
| 7 | until | 191 | 329,935 |
| 8 | f1147eb3-45b6-41d0-a395-06f85bd38b0b; | 1 | 329,525 |
| 9 | wc | 219 | 210,182 |
| 10 | echo | 295 | 206,765 |
| 11 | GIT_CONFIG_COUNT=1 | 133 | 146,500 |
| 12 | scratchpad | 96 | 99,057 |
| 13 | rtk | 89 | 98,371 |
| 14 | CARGO_INCREMENTAL=0 | 73 | 87,028 |
| 15 | head | 52 | 84,080 |
| 16 | WT_OWNER="Claude | 166 | 67,056 |
| 17 | R=$(ls | 41 | 55,482 |
| 18 | sleep | 159 | 53,069 |
| 19 | src; | 28 | 50,208 |
| 20 | ls; | 45 | 50,126 |

`awk`, `df` and `sqlite3` are absent from `bash_default` (`filter_kind` is `rule`, `formatter` and `formatter`). `just` is rank 44 (19 rows, 14,953 B). The same run's `bash` table (live stem, not stored rows) labels `mise` `default` (93 calls, 57,155 B), `just` `default` (282 calls, 120,142 B), `awk` `rule` (608 calls, 1,103,992 B), `df` `formatter` (170 calls, 102,390 B) and `sqlite3` `formatter` (36 calls, 20,179 B). This is not a saving against the 2026-09-18 snapshot: that one covered 916 sessions, this one is the current 30-day window. The goldens `tests/cmd_golden/{df,sqlite3,awk}` are shorter than `Rule::default()` (`df_sqlite_and_awk_beat_the_default_rule`). `df` and `sqlite3` are formatters; `awk` is the `[awk]` rule (`group = dir`, keep `awk:|fatal`).

### 15.4 MiMo Code host probe (T186, fetched 2026-09-24)

Confirmed against https://github.com/XiaomiMiMo/MiMo-Code, https://mimo.xiaomi.com/mimocode/start,
`/config-files`, `/config-overrides`, `/mcp-servers`, `/env-vars`, `/tools`: binary `mimo`
(install via `curl -fsSL https://mimo.xiaomi.com/install | bash`, `powershell -ep Bypass -c
"irm https://mimo.xiaomi.com/install.ps1 | iex"`, or `npm i -g @mimo-ai/cli`); global config
`~/.config/mimocode/mimocode.json` (`.jsonc` also accepted), moved by `MIMOCODE_HOME` /
`MIMOCODE_CONFIG`; project config `.mimocode/mimocode.json`, searched upward, parent-first
merge. MCP under `mcp.<name>`: local servers are `{type: "local", command: [..], enabled,
environment?, timeout?}` — the exact shape OpenCode kept from upstream, so `mimo`'s installer
reuses `opencode`'s `register_mcp`/`unregister_mcp` path (`src/agents/mod.rs::register_local_mcp`,
factored out in this task to keep `just dup` under budget); remote servers add
`{type: "remote", url, headers?, oauth?}`, unused here. No documented base-URL/proxy override
env var — `MIMOCODE_MODELS_URL` only relocates the model manifest fetch, not the API endpoint —
so `proxy` is `Support::No`. The only hook-shaped mention found is `tool.execute.before`/
`tool.execute.after` on the Custom Tools page, OpenCode's in-process plugin surface; no
`/mimocode/plugins` page exists (404) and no `@mimo-ai/plugin` package is documented, so v1
ships no plugin and `hooks`/`plugin` are both `Support::No` (creator instruction: do not guess
the plugin package). MiMo Desktop is confirmed to exist (early access, "powered by MiMo Code
as its core engine") but no separate config path is documented, so no Desktop variant ships.

**Update 2026-10-10 (T520): the plugin package and `mimocode.jsonc`.** The v1 gap above is closed;
everything below was checked on 2026-10-10 against primary sources.

- Plugin package: `@mimo-ai/plugin` 0.1.15 is on npm, published 2026-09-22
  (https://registry.npmjs.org/@mimo-ai%2fplugin), from
  https://github.com/XiaomiMiMo/MiMo-Code/tree/main/packages/plugin. Its `Hooks`
  (`packages/plugin/src/index.ts`) has `tool.execute.before` (output `{args, cancel?,
  cancelReason?}`), `tool.execute.after` (output `{title, output, metadata}`), `shell.env`,
  `experimental.session.compacting` (output `{context, prompt?}`) and
  `experimental.chat.system.transform` (output `{system}`), the same names and shapes
  `plugins/opencode/rtok.ts` uses.
- Loader: `packages/cli/src/config/plugin.ts` scans each config directory with
  `Glob.scan("{plugin,plugins}/*.{ts,js}")` (symlinks followed), as OpenCode and Kilo do; file
  hooks (`{hook,hooks}/*.{js,ts}`, `packages/cli/src/plugin/index.ts`) are a separate mechanism
  and unused. `readV1Plugin` (`packages/cli/src/plugin/shared.ts`) takes a default export object
  with a `server()` function, and a plugin loaded from a file path must export an `id`
  (`resolvePluginId`), so `export default { id: "rtok", server }` fits. `rtok.ts` imports only
  `node:child_process`. `ConfigPaths.directories` (`packages/cli/src/config/paths.ts`) lists the
  global config dir first, and `packages/cli/src/config/config.ts` runs `ConfigPlugin.load(dir)`
  for every directory it returns, outside the `.mimocode`-suffix check that only gates the
  config files, so `~/.config/mimocode/plugins/rtok.ts` is loaded. rtok's tests link the file
  there without running MiMo (no real agents).
- Config files: the global directory takes `config.json`, `mimocode.json` and `mimocode.jsonc`,
  "merged in that order (later overrides earlier)" (https://mimo.xiaomi.com/mimocode/config-overrides);
  `packages/cli/src/config/config.ts` (`loadGlobal`) reads them in that order and, when none of
  the three exists, creates a starter `mimocode.jsonc`. So `.jsonc` wins a conflict with `.json`,
  and a fresh MiMo has only a `.jsonc`. https://mimo.xiaomi.com/mimocode/config-files names only
  `mimocode.json` and says the format is JSON / JSONC. rtok edits `mimocode.jsonc` when
  `mimocode.json` is absent; when both exist it keeps writing `mimocode.json`, which MiMo merges
  under the `.jsonc` (a conflicting `mcp.rtok` in the `.jsonc` would still win).
- MiMo Desktop: still no documented config path. A third-party README
  (https://github.com/akaradje/mimo-desktop, **unverified**) says Desktop reads
  `~/.config/mimocode/mimocode.jsonc`; T521 waits for a read-only probe on the creator's machine.

## 16. Token savings beyond the shipped surface (2026-09-21)

Creator request: what else can save LLM tokens in an agent product like rtok (AirTalk), after inventorying what already ships. Sources: `plan.md` plugin catalogue, `ideas.md`, §§2/4/6/9–15 of this file, and the in-tree plugins under `src/plugins/`. Vendor % claims stay claims unless marked *measured*.

### 16.1 What rtok already does (short)

| Lever | Where | Mechanism |
| --- | --- | --- |
| Tool-output trim (lossless) | `cmd`, `read`, MCP wrap | Family formatters + TOML rules; archive raw → pointer + `expand` |
| Re-read / refetch dedup | `read`, `guard` | sha256 / identical read-or-command within N turns → deny or skip full body |
| Live-zone rewrite | `archive` + `proxy` | Old large `tool_result`s → head/tail + pointer; system/tools/last-2-turns untouched |
| Budgeted injection | `inject` | SessionStart / UserPromptSubmit under a token cap; modes as markdown data |
| Memory without stuffing | `memory` | FTS5 notes; SessionStart injects `id title` only; bodies via `mem_get` |
| Code navigation instead of dumps | `graph` | `symbol` / `callers` / `outline` / `impact` from tree-sitter-tags in SQLite |
| Measurement | `measure`, proxy usage | Context-token-turns + provider usage; `stats` / `report` / doctor |
| Optional tabular encode | `toon` | JSON tables → TOON (on by default since T127) |
| Optional extractive shrink | `compress` | P28 gate; off until semantic compress clears a bench |
| Host install surface | `agents install` | Hooks + MCP + proxy so the above actually see traffic |

Routing (D9), WASM plugins (P32), embeddings (P29), semantic cache (P31), and tiered context (P33) are **decisions or Later**, not the default v0.1 path.

### 16.2 Already tracked but not the default product yet

Status as of 2026-09-21, re-checked against `done.md` on 2026-10-08 (rows that moved are marked).

These are **not** greenfield — they live in `ideas.md` / `plan.md`. Listed so this scan does not reinvent them. Priority here is “still open for savings,” not “new invention.”

| Priority | Idea / task | Rough impact | Effort | Status | Why |
| --- | --- | --- | --- | --- | --- |
| P0 | **T59.5** tools[] description rewrite (I-45) | *measured* ~6.2 % of session **input** when Tool Search is off | M | shipped (off by default) | High repeat tax every turn; off-by-default rewrite is the right shape |
| P0 | **T61.2** live-zone skill bodies (I-51) | High when a large skill stays in every later request (§10.3) | M | shipped (off by default) | Same archive path as tool results; gated on skill stats (T61.1) |
| P1 | **T58.1** delta re-read (I-41) | Medium on Read-heavy sessions (Read ≈ 15 % of tool-result tokens §2) | M | shipped | MCP `read` returns unified diff when file changed since last read (§2: 7.3 % of reads) |
| P1 | **T58.2** compaction checkpoint + archive ids (I-42) | Medium on long sessions that compact | M | shipped 2026-09-18 | Survives host summarization; half is host-plugin work (T70.x, also shipped) |
| P1 | **T59.1** per-stem `skip_wrap` (I-39) | Medium for curl/ffmpeg-class Bash if currently unwrapped | S–M | shipped 2026-09-18 | Fail-open; needs hang Check |
| P2 | **P28 / I-21** LLMLingua-style / extractive `compress` on | High *if* bench beats lossless; quality risk on code | L | open | Default off; costs tokens to save tokens |
| P2 | **P31 / I-23** semantic response cache | High on repeated asks; dangerous false hits | L | open | Needs false-hit Check |
| P2 | **P33 / I-25** OpenViking-style tiered context | High on very long threads | L | open | License + model path |
| P2 | **T51.1** (I-09) compress nested JSON / `data:` inside live zone | Medium when blobs dominate | M | shipped 2026-09-18 | Complementary to tool_result archive |
| P3 | **I-55** session token/cost budget deny | Process control, not compression | S | open | Hosts already auto-compact |
| P3 | **I-71** HTML→text curl formatter | *measured* &lt; 1 % Bash bytes here — parked | S | open | Re-open only above gate |

### 16.3 Further options not yet a first-class rtok idea (or only as a Decision)

Written 2026-09-21; by 2026-10-08 every row below has an idea in `ideas.md`, a task, or a measured reason not to build (the `Idea / status` lines under the table). Prioritized for an agent product like AirTalk. Effort: S &lt; 1 week, M ~1–3 weeks, L multi-phase. Impact is expected **input** token or CTT reduction unless noted.

| # | Option | Impact | Effort | Notes / sources |
| --- | --- | --- | --- | --- |
| 1 | **Explicit prompt-cache breakpoints + sticky routing** | High $ (cache-read vs input); modest unique-token cut | M | Providers bill cache hits cheaply (rtok already prices cache in T49.1). Pin stable prefix (system + tools + modes) and keep the same backend pod/region so the KV/prompt cache hits. Anthropic prompt caching docs; OpenAI prompt caching. Not the same as I-23 semantic cache. |
| 2 | **Deferred / dynamic tool declarations** | High when many MCP tools | M | Ship short tool stubs; load full schemas on first use (host Tool Search / deferred tools — doctor already warns when `ANTHROPIC_BASE_URL` disables search). Related to I-45 but schema-level, not only shorter text. |
| 3 | **Model / tier routing by job** (D9) | High $; small raw-token change | M–L | Cheap model for format/classify/expand-prep; mid for edit; expensive only after confirm. Needs a router policy + measurement so “savings” are $. |
| 4 | **Thinking / reasoning strip on replay** | Medium–High on reasoning models (estimate; measured 0.0297 % of session input, T125, 2026-09-21, §2) | S–M | Do not re-send prior chain-of-thought blocks into the next turn when the host attaches them; keep final answers + tool I/O. Host- and provider-specific. |
| 5 | **Native context-editing APIs** (I-10 / T51.2) | Medium–High | M | Let the platform shrink history (Anthropic context editing / host compaction hooks) *and* keep rtok archive ids in the checkpoint (ties to T58.2). |
| 6 | **Structured tool I/O (JSON Schema / strict)** | Medium output + easier trim | M | Force tools to return compact tables/fields instead of prose; then `toon` / formatters win more often. T402 (2026-10-08): measured, not built — already-structured JSON is 0.15 % of tool-result input and a `toon`-eligible table saves ~0 there; the prose a tool schema could reach (MCP only) is ≤ 0.64 % of session input (§16.6). |
| 7 | **Sub-agent isolation + budgeted handoff** (I-46) | Medium when Task/Agent traffic grows | M | Child context starts small; parent gets a digest with archive ids — not a full transcript paste. |
| 8 | **Identifier / path dictionary in-session** | Low–Medium | L | Replace repeated long paths with short codes in tool results; expand on demand. Easy to break models; needs A/B. |
| 9 | **Multimodal token gate** | High $ when screenshots dominate (estimate; measured 0.91 %, T137, §2) | S–M | Prefer OCR/text or downscale; refuse or summarize images in the live zone. Separate from text CTT. T137 (2026-09-24): images are 0.91 % of session input (§2) — under the 5 % gate, not built. |
| 10 | **Speculative local draft → verify** | Mixed | L | Local small model proposes; cloud model verifies — can cut cloud **output** tokens, adds complexity and wrong-draft risk. |

Idea / status of each row (2026-10-08): (1) I-84, decision-shaped; the cache hit rate here is already 98.1 % (§2), so the head-room on this workload is small. (2) I-85, overlaps host Tool Search. (3) I-101 model routers, needs a bench with a quality gate; D9 stays a decision. (4) I-86, rejected 2026-09-21 (T125): the API already strips earlier turns' thinking blocks. (5) shipped as T51.2 (2026-09-17); the archive ids in the checkpoint shipped as T58.2 (2026-09-18). (6) I-109, promoted as T402 (todo). (7) I-46, promoted as T59.6 and closed evidence-only at 0.7 % on 2026-09-18; §17 re-measured it with sub-agent transcripts (T128). (8) I-110, promoted as T403; measured 0.44 % cache-safe saving, 2.05 % repeated bytes (§16.7), not built. (9) not built, under the gate (T137, 2026-09-24). (10) I-111, promoted as T404 (todo). Batch and flex tiers are I-102 (see §31 and `docs/batch-flex.md`).

### 16.4 Sources (non-obvious)

- Workload + technique ranking: this file §§2, 6, 9–15; comparison matrix §4.
- Parking lot (do not re-file duplicates): `ideas.md` Open / Later / Promoted.
- Plugin contracts: `plan.md` catalogue; each `src/plugins/*/PLAN.md`.
- Prompt caching (provider): Anthropic “Prompt caching”; OpenAI “Prompt caching” (billing ≠ semantic cache).
- Tool-description tax: Portkey / LiteLLM claims cited under I-45; rtok *measured* 6.2 % input (2026-09-18).
- External context + grep/slice: recursive-llm notes in §12 (I-53/I-54).
- Tiered memory / L0–L2: OpenViking row in §4 / I-25.
- Extractive / LLM compress: LLMLingua-2 family under I-21 / P28; in-tree `compress` is the off-by-default hook.
- HTML Readable path: tinyjuice / TokenJuice under I-71 (below rtok’s 1 % gate on the measured corpus).
- Host plugin ceilings: §15 (pi/Cursor/OpenCode events rtok cannot see via proxy alone).

### 16.5 Recommended next moves

1. **T59.5** and **T61.2** — highest *measured* or structurally recurring input taxes. Both shipped 2026-09-18 (off by default, as the §16.2 Status column says).
2. Add a plan card for **prompt-cache-stable prefixes + sticky proxy upstream** if `$` savings matter as much as raw tokens (pairs with existing `stats --price` cache rates).
3. Keep P28/P31/P33 in Later until a bench beats the lossless archive lane on *code* sessions.

### 16.6 Prose versus structured tool output (T402, 2026-10-08)

Question: how much tool output is free prose that a schema (fields or tables) could replace, and what would `toon` save on it. Ad hoc, read-only measurement on this machine's store (no shipped code); numbers are aggregates only.

**Corpus A, stored hook payloads.** `calls` rows named `PostToolUse` / `postToolUse` whose `call_io.request_json` is kept (33,702 rows, 2026-10-05 00:33 to 2026-10-08 06:33 UTC; recording started on 2026-10-05, and the store keeps no payload above about 64 KB: 569 calls / 139.6 MB of payload in the same window were not stored, so large results are under-counted). The result text is what the model sees: `Bash` stdout+stderr, `Read` file content, MCP text blocks, Cursor `tool_output` envelopes unwrapped. `Edit`/`Write`/`Monitor`/`Agent`/`ToolSearch` acknowledgements and Cursor `Read`/`Grep`/`Write` (size only, no body) are left out: 23,148 result bodies, 45,810,515 B ≈ 11.45 M est. tokens (bytes / 4) = 0.34 % of session input counted once. **Resident** weights each body by the API requests that follow it in its own (or its sub-agent's) transcript: 20,475 Claude Code bodies, 1.043 G token-turns = **31.4 %** of the 3.32 G session input (uncached + cache_create + cache_read of every request in that window, sub-agents included), an upper bound because compaction drops old results. 2,673 bodies (Cursor, or no transcript found) have no resident factor and appear only in the once-counted bytes.

**Classes** (per body, by heuristic): `file_content` (`Read`, `mcp__rtok__read`, `WebFetch`, `MCP:fetch`: the file's own text, no tool can reshape it); `json_tabular` (top-level array of at least 5 objects with the same 3+ scalar keys: exactly what `toon` encodes, tested with a port of `tabular_keys`); `json_wrapped_table` (the same array under an object key, which `toon` skips); `json_other`; `table_text` (60 % of lines start with `|`, or 80 % share one tab count); `records` (70 % of lines are `path:line:`); `free_text` (the rest: command output, logs, diffs, prose).

| Class | Share of result bytes | Resident, % of session input |
| --- | --- | --- |
| `free_text` | 61.4 % | 19.48 % |
| `file_content` | 31.2 % | 10.14 % |
| `records` | 4.8 % | 1.01 % |
| `table_text` | 2.2 % | 0.61 % |
| `json_tabular` + `json_wrapped_table` + `json_other` | 0.4 % | 0.15 % |

By tool: `Bash` is 17,410 bodies, 26.8 MB, 91.4 % `free_text`, 5.0 % `records`, 3.6 % `table_text`; resident 19.89 % of input (18.4 % of it `free_text`). 3,413 of 18,596 recorded `Bash` payloads already carry an `[rtok ... expand:]` marker, so these are post-`cmd` bytes. All MCP tools together: 1,925 bodies, 7.58 MB, 78 % `file_content` (`mcp__rtok__read` alone is 4.17 % of input); the non-file part is 1.36 % of input resident, of which `mcp__rtok__expand` (the archived original returned verbatim) is 0.72 %, the other `mcp__rtok__*` tools 0.21 % (`search` records 0.11 %, `worktree_list` pretty JSON 0.06 %) and third-party servers (Browser pane, Figma, host `ccd_*`) 0.43 %.

**`toon` on the structured part.** In corpus A four bodies were `toon`-eligible (395 B saved in total) and six were wrapped tables (15,232 B would be saved if unwrapped, 98 % of it one Cursor `Shell` body). **Corpus B, the archive** (`archive` table, 61,369 bodies, 340.1 MB, 2026-09-04 to 2026-10-08, tool name not stored; bodies rtok archived, mostly large ones): `free_text` 60.8 %, `file_content` (the `Read` wrapper) 35.3 %, `table_text` 2.6 %, `records` 1.2 %, `json_other` 0.14 %, `json_tabular` + `json_wrapped_table` 16 bodies / 28,813 B (0.008 %). On those 16, rtok's own encoder logic (port, ignoring the 72-byte pointer on a tiny sample) cuts 8,803 B (30.6 %), which is the only measured TOON ratio and is used below as an optimistic factor.

**What a schema could reach.** If every non-file byte of MCP output became a toon-style table at 30.6 %: 1.36 % x 0.306 = 0.42 % of input; without `expand` (whose payload is the original body) 0.64 % x 0.306 = 0.20 %. All non-file bytes of every tool (`Bash` included, 21.25 % of input) would give 6.5 % on the same factor, but `Bash`/`Shell` output is the stdout of external programs; no rtok schema applies there, only the `cmd` rules and formatters, which already shorten it (the `cmd` rule rows save 59 % of their input, `measurements` 2026-09-14 to 2026-10-08) and which are measured elsewhere.

**Decision (T402, 2026-10-08).** Already-structured tool output is 0.15 % of input and has no `toon` headroom; the prose a tool schema can change (MCP, shortest path) is at most 0.64 % of input before any saving, below the 1 % gate. No build task is filed; row #6 stays an idea. Reopen if an MCP server with large table-shaped prose output (a `search` or `list` of 50+ rows) gets heavy use.

Caveats: heuristic classes (no human labelling); `bytes / 4` token estimate; stored payloads under about 64 KB only (corpus A) and no tool names in the archive (corpus B); resident factors use the transcript's request timestamps and assume no compaction. Commands: read-only `sqlite3 "file:<home>/rtok.db?mode=ro"` plus a one-off Python script over `call_io.request_json` and the archive blobs; transcripts under `~/.claude/projects` for the denominator.

### 16.7 Path and identifier dictionary (T403, 2026-10-08)

`rtok stats --since 30d` on this machine (306 sessions, 633 651 transcript lines), `dictionary` row (`src/measure/dictionary.rs`). The proxy rows here hold no request bodies (0 `calls` with surface `proxy`, 0 `usage` rows in `rtok.db`), so the transcripts stand in: the tool inputs and tool results are the history every request re-sends. Per session, up to 6 evenly spaced API requests are rebuilt (everything up to that turn, restarted at a `compact_boundary`), 1 672 requests in all. A path is a `/`-containing run of at least 16 bytes; an identifier is snake_case or camelCase of at least 12 bytes (prose words and hex hashes are out). Shares divide by the provider-billed input of the same requests (uncached + cache writes + cache reads, 232 965 662 tokens, at the report's 4 bytes per token = 931.9 MB); message content alone is 176.3 MB, since the system prompt, tool schemas, prompts and thinking are not in the transcripts' tool blocks.

| Measure | Paths | Identifiers | Both |
| --- | --- | --- | --- |
| Bytes of tokens repeated at least twice anywhere in the request (ceiling) | 12 834 182, 1.38 % | 6 261 279, 0.67 % | **2.05 %** |
| Cache-safe saving, tool results only | 2 469 052, 0.26 % | 1 631 188, 0.18 % | **0.44 %** |

Cache-safe means the bytes of an earlier message never change when a later one arrives, or the prompt cache (98 % of input here, §2) breaks: a code is defined inline at a token's second use in a result and only the third and later uses shrink; codes go to the biggest tokens first; the model's own tool inputs and text are never rewritten (signed blocks, quoted back into edits), so they count toward the ceiling but not the saving. Source: this machine's `~/.claude/projects` transcripts read by the code above; no request content or path leaves the machine, only the totals above. Caveats: the sample is almost all rtok development, a path-heavy workload; 4 bytes per token is the report's estimate, and short codes tokenize worse than their byte length suggests, so the token saving is lower.

Under the 1 % gate on what a proxy could actually remove (0.44 %): the dictionary (§16.3 #8) stays unbuilt, with this number. The 2.05 % ceiling is above the gate, but it is not reachable: the first use of every token stays, and the model's own turns cannot change. Risk if built anyway: a model that copies a `~7` into a tool input breaks the call unless something expands it back first.

### 16.8 How much input is old archived tool output: P28 Phase 1 (T385.8, 2026-10-08)

Question (`docs/research/optimization.md` §3.3, `docs/llm-soft-compression.md` Phase 1): what share of the input an agent sends is tool output that an LLM compressor could rewrite, and what must such a rewrite keep. No compressor exists; this is a measurement and a test fixture.

**"Old enough" is the shipped archive rule**, not a new one: a tool result is eligible when at least `plugins.archive.keep_turns` = 4 user messages follow it in the request (a turn is one `user` message counted from the end, the newest is turn 0; a tool-result message is a `user` message) and its body is at least `plugins.archive.min_tokens` = 1500 tokens (`src/config/mod.rs` `Archive`, `src/plugins/archive/mod.rs` `outside_live_zone`, `src/proxy/wire.rs` `turn_setup`, all at `d9dac1b5e`). The host-side rule it is compared with is Anthropic's `clear_tool_uses_20250919`: it fires at 100,000 input tokens, keeps the last 3 tool uses and has no `clear_at_least` by default (https://platform.claude.com/docs/en/build-with-claude/context-editing, fetched 2026-10-08); it is off in rtok (`proxy.context_management`, T51.2) and was not measured separately.

**Method** (read-only, aggregates only). Corpus: every Claude Code transcript under `~/.claude/projects` modified in the last 60 days, sub-agent files included as separate conversations, counting only API requests stamped 2026-09-08 to 2026-10-08 (2,148 files, 144,452 requests). Denominator: the real usage of each request, `input + cache_creation + cache_read` tokens, once per `message.id` (21.16 G tokens). Numerator: for each request, every tool result still in that conversation, weighted by one request each (the "resident" weight of §16.6), at `bytes / 4` tokens, counted when it is at least `keep` user messages behind the request's newest user message and at least `min` tokens. Adjacent `user` lines are one message; a `compact_boundary` empties the conversation, so compaction is modelled (§16.6 was not). `rtok stats --since 30d --json` (rtok 0.15.1, `fe170a431`) gave the cross-check; the rtok database was opened with `sqlite3 -readonly` for the archive rows. The pass was a one-off Python script, not committed.

| Measure (2026-09-08 to 2026-10-08) | Value |
| --- | --- |
| Real input, all requests | 21.16 G tokens (cache hit 98.4 %, `rtok stats`) |
| Tool results resident, any age or size | 17.29 % of input (163,019 results, 257.2 MB) |
| **Old enough and large enough (keep 4, min 1500)** | **6.54 % of input** (1.38 G est. tokens) |
| Same, sensitivity: keep 8 / keep 16 | 6.13 % / 5.34 % |
| Same, sensitivity: min 300 / min 3000 (keep 4) | 13.76 % / 3.23 % |
| Same, last three days only (2026-10-05 on) | 6.79 % |
| Results that ever become eligible | 8,625 of 163,019 (5.3 %), 99.5 MB of 257.2 MB (38.7 %) |
| Old (keep 4) but under the floor: 300-1,499 tokens / under 300 tokens | 7.22 % / 2.44 % of input |
| Eligible and already carrying an rtok `[rtok … expand:]` marker | 0 (all marked old results, any size: 0.20 %) |

Eligible share by tool (sums to 6.54 %): `Bash` 2.12 %, `Read` of code 1.40 %, rtok `read` 1.23 %, `Read` of prose 0.86 %, other MCP 0.59 %, `Read` of other types 0.19 %, config files 0.05 %, search 0.03 %, rest 0.07 %. By content shape (a line-regex heuristic: 40 % of lines look like code, or a `Read` of a code extension): code 3.57 %, prose and logs 2.92 %, config and data 0.05 %. So about half of what a compressor would touch is source code or diffs, the content the research flags as weakest for token-dropping compressors.

**Archive store** (`archive` table, rtok database, read-only, 2026-09-04 to 2026-10-08): 61,853 bodies, 342.4 MB. Bodies of at least 6,000 bytes (the 1500-token floor): 9,627 rows (15.6 %) holding 248.4 MB (72.5 % of archived bytes); bodies of 24 KB or more: 1,452 rows, 163.3 MB (47.7 %). `archive_decisions` has 0 rows and `proxy.mode = "passthrough"`, so the proxy rewrite has never run on this machine and every figure above is the unrewritten prompt. `rtok stats` replay of the shipped pointer over the same 30 days: tool-result context-token-turns 22.0 G to 15.4 G (-30 %), 2,520 results (that metric ignores compaction, so only its ratio is comparable); 901 `expand` calls followed a shown pointer, 3.4 MB.

**What it means for P28.** The eligible set is 6.54 % of input tokens. A compressor at the 2x-5x range the LLMLingua-2 paper reports (arXiv 2403.12968, as cited in `optimization.md` §3.2, not re-fetched) would remove 3.3 % to 5.2 % of input tokens at most, ceiling 6.5 %. These are tokens, not dollars: 98.4 % of input is served from the prompt cache, and a rewrite of an old block busts the cache from that block on unless the replacement is stable (`docs/prompt-cache.md`). The compressor's own input is the 99.5 MB of ever-eligible bytes, about 24.9 M tokens, 0.12 % of the volume, paid once per result if cached per archive id. The lossless pointer already targets the same set (head 8 and tail 4 lines), so an LLM view competes with a pointer of roughly a dozen lines on quality per token, not on raw size. A cheaper lever sits next to it: `min_tokens` = 1500 leaves 7.22 % of input in old results of 300-1,499 tokens untouched, and min 300 doubles the eligible share to 13.76 %; that is a bench question for the lossless path (idea I-116), not for a model.

**Must-keep fixture.** `tests/fixtures/p28_must_keep.toml` (hand-written, fictional `acme` project, no real data): 7 tool results (build errors, test failure, diff, grep records, JSON response, service log, design note) with 54 verbatim must-keep spans of six kinds: paths with `file:line`, identifiers, error lines, numbers, commands, ids. At least two spans per body sit outside the first 8 and last 4 lines the pointer shows. `tests/p28_must_keep.rs` validates the fixture and scores the shipped pointer: the pointer keeps 14 of the 54 (26 %), exactly those on the first 8 or last 4 lines; `expand` returns all of them.

Caveats: `bytes / 4` underestimates code (about 3 chars per token), so the true token shares are somewhat higher; the tool-name classes come from the transcript's own `tool_use` records; transcripts of the last 60 days only, Claude Code only (Codex, Cursor and others are not in the denominator); mtime-selected files may hold older, uncounted requests; eligibility assumes the 4-message boundary of the proxy even though no proxy ran; micro-compaction by the host, if any, is not modelled. Commands: read-only `sqlite3 -readonly "file:<home>/rtok.db?mode=ro"` for the `archive`, `archive_decisions` and config values; `rtok stats --since 30d --json`.

## 17. Sharing context between an agent and its sub-agents (2026-09-21)

Creator request: a freshly spawned sub-agent gets none of the parent's context, reads the same files again and pays for them again. Find what rtok can apply; sub-agents stay cheap (Haiku). Method: two Haiku agents (repo inventory, web scan), one Haiku docs check, an ad-hoc scan of this machine's transcripts; synthesis in the main session. Vendor numbers stay claims.

### 17.1 What T59.6 measured, and what it missed

T59.6 closed `handoff` at 0.7 % because it measured the `Agent` tool's input and result **in the parent transcript**. The cost of a sub-agent is not there: it is in `<session>/subagents/agent-<id>.jsonl` (plus `agent-<id>.meta.json`: `agentType`, `model`, `toolUseId`, `spawnDepth`), which no rtok code attributed to a parent when this section was written (2026-09-21; `src/` had no `agent_id`, `agent_type` or sidechain handling). T128 (2026-09-22) attributes sub-agent transcripts to their parent session in `rtok stats`, and T129 (2026-09-22) makes the hook payload carry `agent_id`.

`rtok stats --since 30d --json` → `subagents`, run 2026-09-22 on this machine's `~/.claude/projects` (the T128 row replaces the 2026-09-21 ad-hoc scan and supersedes its numbers):

| Quantity | Value |
| --- | --- |
| Sessions with sub-agents / sub-agents | 51 / 531 |
| Tool-result bytes: sub-agents vs their parents | 32,112,993 vs 37,054,167 (46 % of the tree) |
| Sub-agent file-read result bytes | 10,262,213 (32 % of sub-agent tool-result bytes) |
| … of a path the parent also read | 2,554,602 (25 % of sub-agent read bytes) |
| … of a path an earlier sibling read | 1,460,241 (14 %) |
| Re-read total | 4,014,843 B = 39 % of sub-agent read bytes, 13 % of sub-agent tool-result bytes, 5.8 % of the tree's |
| Sub-agent usage (tokens) | input 78,320 · cache read 1,083,085,186 · cache write 40,376,428 · output 655,280 |

Largest `agentType × model` splits (the full list is `--json` `by_type`): `general-purpose | haiku` 351 agents / 1,028,835 B re-read; `general-purpose | sonnet` 63 / 1,599,793 B; `general-purpose | -` 46 / 676,366 B.

Caveats: path-level match (no range or sha), parent reads counted over the whole session (before or after the spawn), bytes are JSON-encoded `Read` result sizes. Parent-first: a path both the parent and an earlier sibling read counts as a parent re-read, so the two re-read columns are disjoint and sum to the total. Sub-agent transcripts are attributed to their parent session and are no longer counted as sessions of their own (they were before T128). Every re-read byte is also re-sent on each later sub-agent turn (the cache-read column), so the byte share understates the token share.

### 17.2 What the host gives us (Claude Code)

| Fact | Source | Status |
| --- | --- | --- |
| Hooks fired inside a sub-agent carry `agent_id` and `agent_type` | https://code.claude.com/docs/en/hooks | documented |
| They carry the **parent's** `session_id` | sub-agent transcript lines: `sessionId` = parent, `agentId`, `isSidechain: true` | observed 2026-09-21; not documented |
| `PreToolUse` may return `updatedInput`; `Agent` `tool_input` has `prompt`, `subagent_type` | https://code.claude.com/docs/en/hooks | documented (`model`, `description` in `tool_input`: unverified) |
| `SubagentStart` exists, matcher = agent type; `additionalContext` reaches the sub-agent | same page / Agent SDK hooks page | stated generically — verify on a live hook before relying on it (T130) |
| `SubagentStop` input: `agent_id`, `agent_type`, `agent_transcript_path`, `last_assistant_message` | same page | documented |
| `fork` sub-agent inherits conversation, model and prompt cache | https://code.claude.com/docs/en/sub-agents , https://code.claude.com/docs/en/prompt-caching | documented — runs on the parent's model, so it is not a cheap-Haiku path |
| Agent frontmatter: `model`, `tools`, `skills`, `memory`, `hooks`, `mcpServers`, `initialPrompt` | https://code.claude.com/docs/en/sub-agents | documented |
| Haiku 4.5: cache read 0.1× input, minimum cacheable prefix 4,096 tokens, TTL 5 min / 1 h | https://platform.claude.com/docs/en/build-with-claude/prompt-caching | documented |
| `PostToolUse` `updatedToolOutput` replaces any tool's output | Agent SDK hooks page | **unverified for CLI command hooks**; contradicts a standing rtok rule → I-91 (T134, 2026-09-25: doc split confirmed — field absent from the CLI hooks page; live probe blocked by sandbox auth, see §3) |

### 17.3 What follows for rtok

1. **A context window is `(session_id, agent_id)`, not `session_id`.** Every "already seen" state in rtok is keyed by session: `guard::pre_tool` (`src/plugins/guard/mod.rs`), the read cache (`src/plugins/read/cache.rs`), `plugin::identical_result`. A sub-agent that reads a file its parent read inside the guard window is denied with `duplicate; rtok expand <id>` for a body it never saw: one extra round trip, the full body anyway, and a `guard` Measurement row that claims a saving. Observed: 22 such denial strings in 11 sub-agent transcripts over 30 days (how many were cross-context is unmeasured). T122 fixes the MCP `read` side with a size threshold because MCP cannot see the caller; the hook side **can** see `agent_id` → T129. **Decision (2026-09-24, T127):** MCP cannot see the caller still holds — `src/mcp.rs` builds every `Ctx::new`, never `with_agent` — so `identical_result`'s MCP `read` call keeps its session-only scoping rather than dropping the pointer outright; known gap: a body one sub-agent read through MCP `read` can still be handed to a different sub-agent (or the main window) sharing that session as a pointer it never saw the content behind. The hook surface closed the equivalent gap the same day: `cmd::run`'s dedup is now keyed on `(session, agent_id)` via the `PreToolUse(Bash)` rewrite's `--agent` flag.
2. **Content cannot be shared for free; pointers can.** A sub-agent needs the bytes in its own window. What rtok can remove is the *search and whole-file* cost: the parent's ledger already knows which paths matter, their archive ids, outlines and line ranges. A budgeted, byte-stable brief appended to the `Agent` prompt at spawn ("these files, these ranges, `read` with `range`, `expand <id>`") replaces the sub-agent's Glob/Grep/whole-file Read turns with ranged reads → T130. This is the Anthropic "pass references, not content" pattern (https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents) and lean-ctx `ctx_handoff` (§9), built on the existing `handoff` digest — one call path (D21).
3. **It ships only with a number.** The brief costs tokens in every spawn. T128 makes the re-read share a `rtok stats` row; T131 splits it by brief on/off; default-on only if the net is positive.
4. **A cheap scout by construction.** A shipped agent definition (`model: haiku`, tools limited to rtok MCP `read`/`search`/`outline`/`explore`/`expand`, answer = `path:line` citations) makes the cheap path the default one → T132; measured by T128's per-`agentType` split.
5. Not pursued: `fork` (parent model, parent-sized context — the opposite of cheap); shrinking the `Agent` result (0.7 %, T59.6 stands). Parked as ideas: sibling cache-prefix sharing (I-89), sibling findings board (I-90), `updatedToolOutput` (I-91).

## 18. Git worktrees: accumulation, ownership, build artifacts (2026-09-21)

Creator question: worktrees pile up, nobody knows whose they are, names are random, and they fill the disk. Not a token saving — no `Measurement` row and no public number follows from this section.

### 18.1 Measured on this repository (2026-09-21, `git worktree list --porcelain`, `du -sk`, `df -h /`)

- The data volume had 220 MiB free of 926 GiB; a sub-agent's shell failed with `ENOSPC`. A full disk also fakes test failures (integration tests exit 1 with empty output).
- 28 worktrees in 5 locations: 12 under `/private/tmp` (11 in Claude Code session scratchpads, which macOS purges), 7 as siblings `apps/rtok-*`, 5 in `.claude/worktrees/`, 3 in `_worktrees/` and `_ci-no-main-push/`, plus the main checkout.
- A worktree is 19–29 MB of source. Six held a cargo `target/` of 1.4–7.2 GB, ≈ 27 GB together; `crates/rtok-webui/target` added 5.2 GB. **All of the weight is tagged build cache** (`CACHEDIR.TAG`).
- The largest consumer was invisible to git: `.claude/worktrees/graph-perf`, 18.1 GB of `target/`, untouched since 2026-09-12. Its `.git` file pointed at `/Users/…/GitHub/rtok/.git/worktrees/graph-perf` — the repository had moved and the absolute link broke, so neither `git worktree list` nor `prune` sees it. It also held 23 edited source files that exist in no commit. Deleting only `target/` freed 17 GiB and lost nothing.
- Names carry no information: `rtok-wt-t126` holds branch `t138-inquire-prompts`, `rtok-t103` holds `t106-otel-export-tests`, agent worktrees are `agent-<hex>`; the three locked ones had no lock reason.
- `git branch --merged` cannot answer "is it merged": PRs are squash-merged, so merged branches (T138, T81) still show 1–2 commits ahead of `origin/main`.

### 18.2 What git gives and does not give (https://git-scm.com/docs/git-worktree)

Admin data is `$GIT_DIR/worktrees/<id>/` (`gitdir`, `HEAD`, `index`, `locked`); the worktree holds a `.git` *file*. No owner, description, TTL or size exists. Ignored files are never shared or cleaned. Free-text metadata fits in `git worktree lock --reason` (shown by `list --porcelain`; non-ASCII is C-quoted there, so keep it ASCII) or `git config --worktree` (needs `extensions.worktreeConfig`). `rm -rf` leaves the admin entry until `gc.worktreePruneExpire` (3 months) — and forever when the worktree was locked: checked 2026-09-22 with git 2.54, a locked worktree whose directory was deleted is not even reported `prunable`. Meanwhile its branch counts as checked out. `git worktree remove <path>` on the missing directory (after `unlock`) drops that single record → T153. `worktree.useRelativePaths` (git ≥ 2.48) would have kept `graph-perf` linked, but sets `extensions.relativeWorktrees`, which older git and possibly libgit2/gix-based tools refuse → T157.

#### Relative worktree links probe (2026-09-24, T157)

Scratch repositories only. `git config worktree.useRelativePaths true` followed by `git worktree add ../wt` writes `gitdir: ../repo/.git/worktrees/wt` in `wt/.git` and a relative back-link. It also sets `extensions.relativeWorktrees = true` and raises `core.repositoryformatversion` to 1.

| Reader | Version | Opens the worktree | How checked |
| --- | --- | --- | --- |
| git CLI | 2.54.0 | yes | `git status`, `git worktree list` |
| cargo (VCS dirty check) | 1.97.1 | yes | `cargo package --list` inside the worktree reports the one uncommitted file |
| gh | 2.101.0 | yes | `gh repo view --json name` inside the worktree resolves `pyrlyn/rtok` |
| lazygit, delta, editors (VS Code, Zed, Cursor) | — | not tested | interactive; lazygit shells out to the git CLI, delta never opens a repository |

Move test: the parent directory holding `repo/` and `wt/` was moved with `mv a b`. With relative links, `git -C b/wt status` and `git -C b/repo worktree list` work unchanged, and `git worktree repair` is not needed. The absolute-link control breaks on the same move: `fatal: not a git repository: (null)`, and the worktree is listed as `prunable`. Renaming only one side (the repository or the worktree) breaks the relative link too, as expected.

Conclusion: every non-interactive reader on this machine opens a relative-link worktree. The editors are untested. The setting stays opt-in until they are checked: the T157 Check asks for "every reader passes" before the setting is added to the `worktrees` skill and AGENTS.md.

### 18.3 Hosts (vendor docs, fetched 2026-09-21, not re-verified by running each host)

| Host | Location | Naming | Automatic cleanup |
| --- | --- | --- | --- |
| Claude Code (https://code.claude.com/docs/en/worktrees) | `<repo>/.claude/worktrees/<name>` | random words or `agent-<hex>`; branch `worktree-<name>` | only when unchanged; sub-agent worktrees by a `cleanupPeriodDays` sweep |
| Cursor (https://cursor.com/docs/configuration/worktrees) | `~/.cursor/worktrees/` | undocumented | max 25, 6-hourly sweep, oldest evicted |
| Codex (https://learn.chatgpt.com/docs/environments/git-worktrees) | `$CODEX_HOME/worktrees` | `thread-N`, detached HEAD | keeps the 15 most recent |
| Conductor (https://www.conductor.build/docs/concepts/git-worktrees) | `~/conductor/workspaces/<repo>/` | workspace name | archive script only |

No host accounts for build output. Claude Code exposes `WorktreeCreate`/`WorktreeRemove` hooks that replace the default create/remove (https://code.claude.com/docs/en/hooks) → T156. The same four complaints are filed upstream: anthropics/claude-code#46098 (names), #24207 (unbounded growth), #56639 (archive deletes uncommitted work).

#### `WorktreeCreate`/`WorktreeRemove` contract (T156, docs checked 2026-09-27)

This table records what Claude Code's docs promise. The "probe" column is for the creator's live run; the probe kit is a scratch file and is not committed (T280: agents never run a real host). Sources, all checked 2026-09-27: https://code.claude.com/docs/en/hooks (`#worktreecreate`, `#worktreeremove`, `#common-input-fields`, `#common-fields`, `#matcher-patterns`), /worktrees, /desktop, and https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md (top entry 2.1.283).

| Question | Documented | Source | Probe |
| --- | --- | --- | --- |
| Which entry points fire `WorktreeCreate` | `claude --worktree`, a sub-agent with `isolation: "worktree"`, a background session isolated in its own worktree | hooks `#worktreecreate` | pending: CLI, sub-agent |
| Desktop app | Its **worktree** option makes Git worktrees in `<project-root>/.claude/worktrees/` or in the folder set by the "Worktree location" setting. Neither page says the desktop app fires the hooks — **unverified** | desktop `#work-in-parallel-with-sessions` | pending |
| Replaces default creation | Yes, "entirely"; `.worktreeinclude` is not processed | hooks `#worktreecreate`, worktrees | pending |
| Create input | The common fields plus `name`, a slug the user gave or one that was generated (`bold-oak-a3f2`). The docs' example shows `session_id`, `transcript_path`, `cwd`, `hook_event_name` and `name`. `agent_id`/`agent_type` are sent "only when the hook fires inside a subagent call"; the docs do not say whether a sub-agent's create counts as that | hooks `#worktreecreate-input`, `#common-input-fields` | pending: fields per entry point, the generated name for a sub-agent |
| Create output | Command hook: the path is the last non-empty line of stdout (ANSI stripped), so the hook cannot return JSON. HTTP hook: `hookSpecificOutput.worktreePath`. Any non-zero exit or a missing path fails the creation; there is no fail-open | hooks `#worktreecreate-output`, `#exit-code-2-behavior-per-event` | pending |
| Path rules | A relative path is resolved against the hook's cwd. An absolute path with `.`/`..`, or one that goes through a symlink below the repo root, is refused (since 2.1.216). If the path is not a directory the session can enter, the session exits with code 1 | hooks `#worktreecreate-output`, worktrees | — |
| Which entry points fire `WorktreeRemove` | Exiting a `--worktree` session and choosing remove; a sub-agent with `isolation: "worktree"` finishing; deleting a background session whose worktree the hook made | hooks `#worktreeremove` | pending |
| Remove input / output | The common fields plus `worktree_path` (the path create returned). Only the exit code counts; JSON is discarded. A non-zero exit while the directory still exists fails the removal and keeps the directory | hooks `#worktreeremove-input` | pending |
| Default removal of a hook-made git worktree | The docs say "for git-based worktrees, Claude Code handles cleanup automatically with `git worktree remove`". They do not say whether it still does this when both hooks are set | hooks `#worktreeremove` | pending |
| Lock and sweep | Claude Code holds a `git worktree lock` while an agent runs. The cleanup sweep keeps every worktree without Claude Code's marker, including hook-made ones | worktrees | pending: is a hook-made worktree locked |
| Timeout, matcher | 600 s default timeout for command hooks. No matcher: the hook fires every time | hooks `#common-fields`, `#matcher-patterns` | — |
| History | Events added in 2.1.50; plugin hooks fixed in 2.1.69; HTTP form in 2.1.84; path screening in 2.1.216; `/batch` on hook-made worktrees in 2.1.281 | CHANGELOG | — |

What this means for T159: the hook does not just observe creation, it is the only thing that creates the worktree. When `rtok worktree add` fails, the hook itself must still print a usable path. It also has to do the work of `.worktreeinclude`.

Re-checked for T159, 2026-10-02, against the raw pages https://code.claude.com/docs/en/hooks.md (sections `WorktreeCreate`, `WorktreeRemove`) and https://code.claude.com/docs/en/worktrees.md (`Customize worktree creation`, `Clean up worktrees`). The table above still holds: create input is the common fields plus `name` (the docs' example has `session_id`, `transcript_path`, `cwd`, `hook_event_name`, `name`), remove input is the common fields plus `worktree_path`, and neither event has a matcher. What T159 builds on, all from those two pages:

| Fact | Source |
| --- | --- |
| A command hook returns the path as the last non-empty stdout line; any failure or a missing path fails the creation, so a hook cannot fail open by printing nothing | hooks `#worktreecreate-output` |
| `WorktreeRemove` exit 0 counts as removed whatever is on disk; non-zero fails the removal only while the directory at `worktree_path` still exists, and then there is no `git worktree remove --force` fallback. With no `WorktreeRemove` hook the host falls back to `git worktree remove --force` on the path the create hook returned | hooks `#worktreeremove` |
| The host never deletes the branch of a hook-created worktree; the remove hook has to | hooks `#worktreeremove` |
| Default creation, which the launcher's plain-git fallback reproduces: `<repo>/.claude/worktrees/<name>` on branch `worktree-<name>`, from the default branch, or from the local `HEAD` when `origin/HEAD` is not available | worktrees `#start-claude-in-a-worktree`, `#choose-the-base-branch` |
| A relative path is resolved against the hook's cwd; an absolute path with `.` or `..` segments, or one through a symlink below the repository root, is refused (since 2.1.216); other output belongs on stderr | hooks `#worktreecreate-output` |
| Default hook `timeout` is 600 s for command hooks | hooks `#common-fields` |

Still **unverified** (needs the creator's live run, T156 part 1): whether a sub-agent's create and remove payloads carry `agent_id`, and which `session_id` a sub-agent's `WorktreeRemove` carries. T159 resolves the owner from `session_id` and, when present, `agent_id`, and removes under any agent of the payload's session, so either answer works; the docs name `agent_id` only for hooks that fire inside a sub-agent call.

### 18.4 Libraries and tools

- `git2` 0.21: add, list, lock, prune — no `move`, `repair`, or dirty-checked `remove`; adds libgit2. `gix` 0.87: worktrees read-only (create/move/remove/repair open in its `crate-status.md`). worktrunk and `git-worktree-runner` shell out to `git`. rtok already shells out to `git` (`git_changed_files`, `src/plugins/graph/mod.rs`) and has no shared git helper; `git_root` exists twice (`src/config/layers.rs`, `src/doctor.rs`). **Decision: `git worktree list --porcelain -z` through one helper, no new dependency.**
- worktrunk (https://github.com/max-sixty/worktrunk): path templates, merge-and-remove, `--copy-ignored` reflink seeding of `target/`. It does not record owners, find orphans, or clean idle caches — the three things measured in §18.1.
- Squash-aware "merged" needs no GitHub call: `git merge-tree --write-tree <base> <branch>` equals `<base>^{tree}` when merging the branch would change nothing.
- A shared `CARGO_TARGET_DIR` is rejected: ~5 parallel agents would serialize on the build lock. `sccache` does not cache incremental builds. Reflink seeding (`reflink-copy`) only lowers the cost at creation; cleaning idle caches removes it → measure before adopting (T156).
- First data point for T156 (2026-09-22, APFS, `cp -c -R <other-worktree>/target <new-worktree>/target`, disk delta from `df -k`, not `du`): an 8.1 GB `target/` cloned in 8.8 s for 17 MiB of physical disk; the first `cargo nextest run --lib --test worktree` in the seeded worktree (T150) rebuilt only the four workspace crates, 24 s, with no dependency recompiled. A cold build was not run for comparison — the disk had under 8 GiB free, which is why the clone was tried at all.
- Second data point for T156 (2026-09-25, part 2 of the card): two fresh worktrees of `origin/main`, `target/` removed in both; the seeded one got `cp -c -R ../rtok-t246.5b/target target && rm -rf target/tmp` (a 14.8 GB `target/` left by two `just check` runs). Then every recipe of `just check` ran in order, timed one by one, continuing past a failure. Disk: used blocks of `df -k /System/Volumes/Data` — `/` is the sealed system snapshot and never moves (a first run measured `/` and read 0 for both). Mac15,9, 16 CPUs, load average 16–19 throughout from other agents' builds, so seconds are rough. Scratch script, not committed.

  | | cold | seeded |
  | --- | ---: | ---: |
  | clone (`cp -c -R`) | — | 15 s, +34 MiB |
  | `fmt-check` | 1 s | 1 s |
  | `lint` (two clippy runs) | 29 s | 22 s |
  | `test`: build (`Finished test` profile) | 43 s | 27 s |
  | `test`: run (nextest `Summary`, 1,782 tests) | 81 s | 81 s |
  | `test`: `dunnage run target` (T236), recipe time minus build and run | ≈ 6 s | ≈ 215 s |
  | `build-min` | 24 s | 23 s |
  | `dup`, `js`, `python` | 1 s | 1 s |
  | **`just check` total** | **185 s** | **371 s** (+15 s clone) |
  | `Compiling`/`Checking` lines | 818 | 12 (the six workspace crates) |
  | data-volume used, delta over the run | +6.28 GiB | +3.35 GiB |
  | `target/` logical size at the end | 6.4 GiB | 10.9 GiB |

  Seeding worked as the first data point said — no dependency rebuilt — but saved only ≈ 23 s of compile here: on 16 cores the dependency graph builds fast, and the workspace crates rebuild at a new path either way. The loss came from T236's `dunnage` pass after `test`: in the seeded tree it compressed 13,539 files (8.1 GB planned, 5.36 GB applied) and deduplicated 15,503 (433 MB), against 76 files in the cold tree. Rewriting a cloned file un-shares it from the source, which is the likely source of the seeded tree's +3.35 GiB against +34 MiB right after the clone. As `just check` stands, seeding is slower (371 s against 185 s) and saves under half the disk. Seeding without that pass was not measured, so no number is claimed for it. No `reflink-copy` from these numbers; the conflict is parked as I-99 in `ideas.md`.
- Third data point for T156 (2026-09-27): this run was meant to measure seeding without the `dunnage` pass. It stopped early because the disk ran low. Setup: two worktrees of `origin/main` (`1ccba28c`) made with `wt.sh new`. The seeded one got `cp -c -R ../rtok-t283/target target && rm -rf target/tmp`: 37 GB logical, of which `debug/` was 35 GB in 164k files. Then the `just check` recipes ran one at a time, **without `dunnage`**, and the test build was split out as `cargo nextest run --workspace --no-run`. Disk was measured as the used blocks of `df -k /System/Volumes/Data`, as in the second data point. Machine: Mac15,9, 16 CPUs, with three other agents building at the same time (load average 28 → 55). The disk deltas therefore include their writes and are upper bounds, not the seeded tree's own cost. Scratch script, not committed.

  | Step (seeded) | Wall | Data-volume used, delta | `Compiling`/`Checking` |
  | --- | ---: | ---: | --- |
  | clone | 44 s | +1.33 GiB | — |
  | `just fmt-check` | 2 s | 0 | — |
  | `just lint` | 47 s | +1.69 GiB | workspace crates only |
  | `cargo nextest run --workspace --no-run` | 105 s (cargo: "Finished in 54.56 s") | +4.38 GiB | workspace crates only, no dependency |

  The run stopped at that point with 4.4 GiB free, below the 5 GiB floor. The test run, `build-min`, `dup`, `js`, `python` and `dunnage` did not run. The cold worktree was skipped: it needs ≥ 30 GiB free and only 13 GiB were free at the start. `wt.sh clean` on the seeded tree removed its `target/` (41 GB logical) and raised free space by only 1.8 GiB while the other builds kept writing. What the run confirms: with a clone, no dependency rebuilds, and the workspace crates rebuild at the new path. That rebuild alone still wrote several GiB here, which fits the second data point's +3.35 GiB. The seeded-without-`dunnage` total and a cold comparison are still unmeasured: both need a quiet machine with ≥ 30 GiB free. The installed `dunnage` 0.1.0 has its own `seed` and `worktree` subcommands (`dunnage --help`); they are a lead for I-99 and were not measured.

### 18.5 What follows for rtok

1. rtok's hooks already fire in every session on every host and carry `session_id` and `cwd`; that is an ownership record no worktree manager has, at no cost to the agent → T154.
2. The always-safe operation is "delete tagged caches, keep the worktree"; `git worktree remove` cannot express it (it refuses the whole worktree when anything is uncommitted) → T152.
3. Inventory first, then deletion: T150 → T151 → T152/T153. Conventions ship as a skill so they cost one description line, not `AGENTS.md` budget → T155.
4. Every measured problem starts at creation (location, name, reason-less lock), and creation is the only moment the owner is known for certain. rtok creates the worktree itself → T158. A skill is advice; on Claude Code the `WorktreeCreate`/`WorktreeRemove` hooks are the one place the rules cannot be skipped, at the price of a decision on what fail-open and the 10 ms budget mean for a hook that must spawn git → T159.

## 19. Hook wall-clock time as Claude Code sees it (2026-09-23)

T178. Machine: the creator's Mac (Apple silicon, macOS, `/bin/sh` → bash), shared with other agents' cargo builds, so every run states its load average. Release build of `68760c6` (`target/release/rtok`, 26,831,904 bytes). All runs use an isolated `RTOK_HOME` and two payloads recorded in the store (a 968-byte `PreToolUse` Bash call and a 2,692-byte `PostToolUse` Bash call, `cwd` rewritten to the worktree). Not a token saving: no `Measurement` row follows.

### 19.1 What Claude Code records

Claude Code writes every hook run into the session transcript as an `attachment` (`hook_success`, `hook_cancelled`, `hook_non_blocking_error`) with `durationMs` and `command`. Command: `find ~/.claude/projects -name '*.jsonl' -mtime -3 | xargs cat | jq 'select(.type=="attachment") | .attachment | select(.durationMs!=null)'`, grouped by `command`.

| `command` | runs | p10 | p50 | p95 |
| --- | ---: | ---: | ---: | ---: |
| `"${CLAUDE_PLUGIN_ROOT}/scripts/hook.sh" PreToolUse` (plugin) | 5,997 | 18 ms | 23 ms | 71 ms |
| `"${CLAUDE_PLUGIN_ROOT}/scripts/hook.sh" PostToolUse` (plugin) | 7,976 | 18 ms | 20 ms | 52 ms |
| `rtok hook PreToolUse` (settings-file install) | 3,246 | 14 ms | 17 ms | 78 ms |
| `rtok hook PostToolUse` (settings-file install) | 4,566 | 13 ms | 16 ms | 80 ms |
| another vendor's `/bin/sh` hook (reads stdin, prints `{}`, may POST to a local port) | 11,699 | 13 ms | 19 ms | 53 ms |

### 19.2 Reproducing it: node `spawn(cmd, {shell: true})`, payload on stdin, clock stops on `close`

A 30-line node harness spawns each command the way Claude Code does, round-robin so load drift hits every command equally. 300 rounds, load average 12.6 → 21.6:

| command | p50 | p95 | delta |
| --- | ---: | ---: | --- |
| `true` | 5.19 ms | 7.27 ms | node + `/bin/sh` floor |
| `rtok --version` | 10.81 ms | 13.59 ms | +5.6 ms: loading the 27 MB binary, clap |
| `rtok hook PreToolUse` | 13.61 ms | 19.63 ms | +2.8 ms: the hook itself |
| `"${CLAUDE_PLUGIN_ROOT}/scripts/hook.sh" PreToolUse` (shipped) | 20.01 ms | 24.88 ms | +6.4 ms: the launcher |

The harness lands on Claude Code's own p50 (20–23 ms), so the gap between the 0.3 ms in-process `calls.ms` and what Claude Code waits for is: shell floor 5 ms, binary start 5.6 ms, launcher 6.4 ms, hook work 2.8 ms. `hyperfine -N -w 10 -r 200` (no shell): `/bin/echo` 2.0 ms, `rtok --version` 5.3 ms, `rtok hook PreToolUse` 8.4 ms, `rtok hook PostToolUse` 8.6 ms.

### 19.3 Root causes, largest first

1. **The launcher, 6.4 ms.** Claude Code runs `/bin/sh -c`, which execs `hook.sh`, a second `/bin/sh` (bash in sh mode, ~5 ms to start on macOS), which forks once more for `$(command -v rtok)` before it execs `rtok`.
2. **Binary start, 5.6 ms over the shell floor** (3.3 ms over `/bin/echo` in hyperfine). Two static initialisers only (`__mod_init_func` is 16 bytes); dyld maps 25 MB of `__TEXT`, rebases 860 KB of `__DATA_CONST` and loads Security, CoreFoundation and CoreServices. `DYLD_PRINT_STATISTICS` prints nothing on this macOS, and samply is not installed, so this was not split further.
3. **In-process, 2.9 ms.** Temporary `Instant` marks (not committed), p50 of 200 runs, with a second connection held open as a live MCP server holds one: clap parse 0.23 ms, `Config::load_lenient` 1.07 ms (figment: defaults, user TOML, legacy fold, env), `Store::open` 0.84 ms (connection, pragmas, settled-migration check, host row), registry 0.01 ms, `calls` row and plugins 0.6 ms, `set_call_ms` and `call_io` 0.08 ms, drop 0.08 ms. With no other connection open, drop costs 1.1–2.1 ms more: the last connection checkpoints the WAL on close. Migrations do not run on a settled store (`migrate` returns after one `COUNT`).

### 19.4 The 5 s cancellations and UserPromptSubmit

`~/.claude/settings.json` has no `hooks` key; the only enabled plugin is `rtok@rtok`, whose `hooks.json` owns every `UserPromptSubmit` hook, with `timeout: 5` from `setup.hook_timeout_s`. The ten cancelled rtok hooks (5 `PreToolUse`, 5 `UserPromptSubmit`, 2026-09-14 to 2026-09-21) all ran the older settings-file command `rtok hook <event>`. The store does hold `UserPromptSubmit` rows (905, mean 0.52 ms in-process). For four of the ten, the matching `calls` row carries a timestamp 0–1 s before Claude Code logged the cancellation, 5 s after it started the hook, and recorded 0.3–0.6 ms in-process; the other six left no row in that window. So the stall came before `record_call`, which is `Config` load or `Store::open`. `Store::open` waits `busy_timeout = 1000` ms on a locked database and retries a locked open up to ten times (~11 s at worst), so this points at the SQLite write lock. Not reproduced here; re-counted after the fixes in §19.8, where the lock explanation does not hold.

### 19.5 Change and result

`plugins/claude/hooks/hooks.json` now runs `command -v rtok >/dev/null 2>&1 && exec rtok hook <event>; exec "${CLAUDE_PLUGIN_ROOT}/scripts/hook.sh" <event>`. `command -v` is a shell builtin, so with `rtok` on PATH Claude Code's own shell execs it directly; without it, `hook.sh` keeps the ketch-store lookup and the fail-open hint. Same harness, 500 rounds, load average 40 → 30:

| event | before (`hook.sh`) p50 / p95 | after p50 / p95 | `true` floor p50 |
| --- | --- | --- | ---: |
| PreToolUse | 20.89 / 62.26 ms | 14.63 / 41.65 ms | 5.38 ms |
| PostToolUse | 18.99 / 33.33 ms | 13.30 / 22.24 ms | 4.86 ms |

About −6 ms, or 30 %, on every hook call. The T178 Check (p50 under 10 ms as Claude Code sees it) is **not met**: the shell floor plus `rtok --version` alone is 10.8 ms. What is left cannot come from trimming the hook path. Parse, config and store open together are 2.1 ms. Reaching 10 ms needs a process that starts in ~1–2 ms: a small hook client with no TLS or framework dependencies, talking to a resident process over a socket, and falling open when the process is absent.

### 19.6 A locked store (2026-09-23)

Where the waits came from: the hook opens the store fine with another writer holding the lock (WAL readers never wait), and its first write, `record_call`, waited out `busy_timeout = 1000` ms. The error was dropped, so the plugins ran on and each of their writes could wait another second. A migration run held on a fresh or upgraded store waits up to 30 s. Fix: `Store::open_with` takes a `LockWait` (per-statement `busy_timeout`, connect attempts, migration wait). `Store::open` keeps 1 s × 10 / 30 s; `rtok hook` passes 5 ms × 1 / 5 ms. When `record_call` comes back "database is locked", the hook returns `{}`: input unchanged, no row written, one `rtok: hook <event> skipped: store locked` line on stderr.

`tests/hook_fail_open.rs` `a_locked_store_fails_the_hook_open_in_ms`: another thread holds `BEGIN IMMEDIATE` on the store, and `rtok hook` (debug build) runs with a Bash payload that the unlocked control run rewrites. Wall time of the whole process, start to exit:

| event | before | after (3 runs) |
| --- | --- | --- |
| PreToolUse | 1.06 s, command still rewritten | 19–31 ms, `{}` |
| PostToolUse | 2.13 s | 21–23 ms, `{}` |
| UserPromptSubmit | 1.07 s | 21–22 ms, `{}` |
| SessionStart | 1.07 s | 20–30 ms, `{}` |

This does not reproduce a full 5 s cancellation. With the lock held, one event wrote at most two statements that waited, but UserPromptSubmit injection and a migration's 30 s wait can add more. After the fix, none of these waits exceeds 5 ms on the hook path.

### 19.7 Resident + `rtok-hook` as Claude Code sees it (2026-09-26)

T178 Check re-run. Machine: the creator's Mac (Apple silicon, macOS), shared with other agents' cargo builds. Release build of `6e608af2` (`target/release/rtok` 28,109,008 bytes; `target/release/rtok-hook` 464,320 bytes). Isolated `RTOK_HOME`, resident `rtok hook --serve` already up. `plugins/claude/hooks/hooks.json` now prefers `rtok-hook` → `rtok hook` → `hook.sh`. Not a token saving: no `Measurement` row follows. No live Claude Code session; the §19.1 transcript table is unchanged history.

Harness (same shape as §19.2): a small node script (`node` 26.8.2) `spawn`s each command with `{shell: true}`, writes a recorded Bash PreToolUse / PostToolUse payload on stdin, and stops the clock on the child `close` event. 300 rounds, round-robin so load drift hits every command equally. Load average 46.23 → 39.18. Timed command (PreToolUse; PostToolUse swaps the event name):

```
command -v rtok-hook >/dev/null 2>&1 && exec rtok-hook PreToolUse; command -v rtok >/dev/null 2>&1 && exec rtok hook PreToolUse; exec "${CLAUDE_PLUGIN_ROOT}/scripts/hook.sh" PreToolUse
```

| command | p50 | p95 |
| --- | ---: | ---: |
| `true` | 5.63 ms | 13.76 ms |
| `hooks.json` PreToolUse (`rtok-hook`, resident up) | 12.28 ms | 25.18 ms |
| `hooks.json` PostToolUse (`rtok-hook`, resident up) | 12.54 ms | 29.22 ms |

About −2.4 ms vs §19.5's `rtok hook` PreToolUse p50 (14.63 → 12.28 ms) on a quieter load then; the node + `/bin/sh` floor is still 5.6 ms, and the tiny client plus IPC leave ~6.7 ms above it. The T178 Check (p50 under 10 ms as Claude Code sees it) is **still not met**. Blocker: even with the resident answering and `rtok-hook` first on PATH, harness p50 stays ~12 ms on this machine under the stated load.

Closed the same day (T178, 2026-09-26): the creator raised the Check to hook p50 as Claude Code sees it under **20 ms** on the §19.2 harness, which the numbers above meet (12.28 and 12.54 ms). Getting to 10 ms stays on the roadmap (Later), not in `plan.md`.

### 19.8 Cancellations after the fixes (T397, 2026-10-08)

§19.4 counted ten cancelled rtok hooks before T178 and §19.6 validated the lock-wait fix only against a synthetic lock. Re-counted on the creator's real transcripts and logs, read-only (streamed `jq` over `~/.claude/projects/**/*.jsonl`, `grep -c` over `~/.rtok/rtok.log`; nothing written, no `rtok` command opened the store). Aggregates only; timestamps are UTC. Window: 2026-09-27 to 2026-10-08, the days after T178 closed (#413, 2026-09-26); no rtok hook attachment survives in the transcripts for 2026-09-22 to 2026-09-26. Machine: Apple M3 Max, load average 80–200 while measuring.

| | runs | cancelled | rate |
| --- | ---: | ---: | ---: |
| Before T178: `rtok hook <event>` from `settings.json`, 2026-09-14 to 2026-09-21 (surviving transcripts only) | 13,091 | 10 | 0.076 % |
| After T178: plugin `hooks.json` command (`rtok-hook`, else `rtok hook`, else `hook.sh`), 2026-09-27 to 2026-10-08 | 165,669 | 114 | 0.069 % |

The rate did not move; the count rose with use. Of the 114: 107 `PreToolUse`, 4 `UserPromptSubmit`, 3 `SubagentStart`. Duration at cancellation: 75 at 5.x s (the `timeout: 5`), 10 at 6 s, 15 at 7–9 s, 9 at 10–29 s, 5 at 30–70 s. They cluster: 13 sessions, one with 54 and one with 20; 107 fall on three days (2026-10-02: 29, 2026-10-07: 27, 2026-10-08: 51); at most 24 in one hour and 4 in one minute.

Lock check, `~/.rtok/rtok.log` since 2026-09-26: 7 `hook … skipped: store locked` events (2026-09-30: 1, 2026-10-02: 2, 2026-10-06: 1, 2026-10-07: 3), so the T178 fix does fire on the real store and returns `{}` instead of waiting. Seven skips cannot account for 114 cancellations. Of 6,545 slow-hook warnings (over `max_ms = 10`), the slowest in-process run was 3,260 ms and none reached 5 s, so the 5 s cancellations are not hooks that ran to the end and logged; they are hooks that never got that far. These data do not say why (a starved process start under load, a host-side abort, and a stall before `record_call` all fit), and a transcript attachment carries no stack. Result: the lock-wait fix is confirmed to fire live; it is **not** shown to remove the cancellations, whose cause stays open.

## 20. WebSearch, WebFetch and browser page text: size, reach, what would cut it (2026-09-23)

T180. Corpus: `~/.claude/projects/**/*.jsonl` modified in the last 7 days — 347 files, 33,599 tool results, deduplicated by `tool_use_id` (resumed sessions copy history, which inflated the 2026-09-22 audit's figures). Bytes are the result text the model received, after any rtok shrinking. Claude Code 2.1.267. Scan scripts stayed in scratch.

### 20.1 Size

| Tool | Calls | Bytes | Share of all tool-result bytes | p50 | p90 | max |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| all tools | 33,599 | 31,948,245 | 100 % | | | |
| `WebSearch` | 552 | 1,544,421 | 4.83 % | 2,803 | 3,310 | 4,827 |
| `WebFetch` | 513 | 1,039,970 | 3.26 % | 1,266 | 2,835 | 41,232 |
| `Claude_Browser` `get_page_text` | 16 | 150,143 | 0.47 % | 4,561 | 25,477 | 40,447 |
| `Claude_Browser` `read_page` | 13 | 95,083 | 0.30 % | 4,911 | 20,292 | 20,301 |
| Bash calling `curl`/`wget`/`gh api` | 160 | 231,431 | 0.72 % | 575 | 3,920 | 17,258 |

Web results together: 3.06 MB, 9.6 % of tool-result bytes — third after Bash (41.8 %) and file reads (`Read` + MCP reads, 34.7 %). `claude-in-chrome` did not appear in the window.

### 20.2 Anatomy (400-result samples per tool)

- **`WebSearch`**: a fixed header, a `Links: [...]` JSON array (title + url, usually 10 entries), prose written by the search sub-call, and a `REMINDER` line about citing sources. The `Links` array is **48.4 %** of the bytes. Only **12 %** of listed links (mean per result) are cited by url or host in the prose; the rest are often off-topic (a search for cargo cleanup tools lists Wikipedia pages on a spacecraft and a vacuum cleaner). 615 of 3,665 link mentions repeat a url already listed in an earlier result.
- **`WebFetch`**: already reduced by the host — a small model answers the agent's prompt over the page, so p50 is 1.3 KB. The long tail is pages returned nearly verbatim (Markdown docs such as `code.claude.com`): 12 of 400 results above 8 KB hold 220 KB.
- **Browser page text**: page text or an accessibility tree with `[ref_N]` handles the agent needs for clicks; 415 duplicate-line bytes in the largest page. Below the 1 % gate on its own.

### 20.3 Offline estimate of candidate reductions

| Reduction | Tool | Saving on sample | Lossless? |
| --- | --- | ---: | --- |
| `Links` JSON → one `- title <url>` line per link | `WebSearch` | 5.3 % | yes (format only) |
| same, plus keep only links cited in the prose; the rest behind `expand <id>` | `WebSearch` | 42.8 % | via `expand` |
| head/tail above 8 KB, rest behind `expand <id>` | `WebFetch` | 15.9 % (12 of 400 hit) | via `expand` |
| head/tail above 4 KB | `WebFetch` | 25.8 % (34 of 400 hit) | via `expand` |

Scaled to the week: WebSearch 42.8 % × 1.54 MB ≈ 661 KB, WebFetch (8 KB cap) 15.9 % × 1.04 MB ≈ 165 KB — ≈ 826 KB, **2.6 % of all tool-result bytes**, before counting that each result is re-sent (cached) on every later turn of its session. These are estimates on a scratch script, not `Measurement` rows; a built filter must record its own.

### 20.4 Surfaces that can reach these results

| Surface | Reaches | Today | Notes |
| --- | --- | --- | --- |
| `rtok proxy` `proxy_filter` on the Anthropic wire | every tool result in the request; tool name from the preceding `tool_use` | works for proxy users; **not the creator** — their `ANTHROPIC_BASE_URL` is `https://api.anthropic.com` | byte-stable rewrite on first sight keeps the cache prefix (the `archive` invariants); fail open on any parse error |
| PostToolUse `updatedToolOutput` | native `WebSearch`/`WebFetch` in every Claude Code session | unknown — T134 is the probe | if honoured, the widest reach with no proxy |
| PostToolUse `updatedMCPToolOutput` | MCP tools only (browser page text) | documented for MCP tools | browser text is 0.77 % — below the gate |
| rtok MCP `fetch` replacing `WebFetch` (I-92) | pages the agent is steered to fetch through rtok | not built | loses the host's per-prompt summary, so a readable-text page (p50 several KB) would usually be **larger** than WebFetch's answer (p50 1.3 KB); wins only on the verbatim tail |
| rtok replacing `WebSearch` | — | no | `WebSearch` is a server-side search; rtok has no search backend and should not add one |

### 20.5 Recommendation

One pure formatter for web results — `WebSearch`: compact the `Links` array and keep only cited links, the full array archived behind `expand <id>`; `WebFetch`: head/tail above a byte cap, archived — wired first behind whichever hook surface T134 opens for native tools, and into the proxy as a second consumer for proxy users. Do T134 before building anything: without `updatedToolOutput` the creator's own sessions see no saving. I-92 as written would grow context on the common case. Browser page text and Bash network calls stay below the gate. Proposed as I-97 in `ideas.md` for creator approval.

## 21. General HTTP(S) interception as a surface: measured, not built (2026-09-23)

T165. Question: would a local MITM proxy (`HTTPS_PROXY` + a CA the user trusts) reach agent tokens that `rtok proxy`, the hooks and MCP cannot? Corpus and method as §20: `~/.claude/projects`, last 7 days, 33,599 tool results deduplicated by `tool_use_id`, 31.9 MB of result text; Claude Code 2.1.267.

### 21.1 How much agent context arrives over HTTP outside the model API

| Result source | Share of tool-result bytes | How the bytes travel | Already reachable by |
| --- | ---: | --- | --- |
| `WebSearch` | 4.83 % | server-side search inside a model API call — not outside the API | `rtok proxy` (tool result in the next request); PostToolUse if T134 |
| `WebFetch` | 3.26 % | the host fetches the page itself, then a small model answers the agent's prompt over it; some Markdown pages come back verbatim | `rtok proxy`; PostToolUse if T134 |
| Bash `curl` / `wget` / `gh api` | 0.72 % | the command's own HTTP | Bash PreToolUse rewrite (`cmd`, `[curl]` rule) |
| Browser page text (`Claude_Browser`) | 0.77 % | a separate browser renders the page; text returns as an MCP result | `updatedMCPToolOutput`; `rtok proxy` |
| **Reachable only by interception** | **≈ 0 %** | | |

Non-API HTTP carries 4.75 % of the bytes (WebFetch, Bash, browser), but every one of those results enters the context as a tool result that an existing surface already sees, and in its final form. An interceptor would see the raw page instead, and for WebFetch only before the host's summarizing call — so what it could shrink is that side call's input, not the agent's context. Below the card's 1 % gate: **do not build.** Creator approved the stop on 2026-09-23.

Re-open when a host appears that fetches content client-side and places it in context through no hook, MCP or `*_BASE_URL` surface, and that share reaches ≥ 1 % of tool-result bytes on a 7-day scan.

### 21.2 Survey (read 2026-09-23), kept for a re-open

| Option | Version / date | Fit for rtok | CA install and removal | Pinning, HTTP/2, streaming |
| --- | --- | --- | --- | --- |
| mitmproxy | 12.2.3 (PyPI) | a second runtime (Python) beside the single rtok binary | own CA in `~/.mitmproxy`; the user trusts it per OS (Keychain, `update-ca-certificates`, `certutil`) | HTTP/1, 2, 3 and WebSockets; `ignore_hosts` passes hosts through untouched; TLS-failure hooks allow excluding a host after a pinning failure |
| `hudsucker` (Rust) | 0.25.0, crates.io 2026-07-15 | in-process library on hyper + rustls, `rcgen` authority; fits the binary | rtok would generate the CA and script trust and removal itself | HTTP/2 feature, WebSocket interception; bypass for pinned hosts is rtok's job |
| `http-mitm-proxy` (Rust) | 0.18.0, crates.io 2026-01-24 | lower-level library, smaller user base | same as `hudsucker` | SSE and WebSocket passed raw, no parsers |
| Proxyman / Charles | desktop apps | not embeddable; GUI-first, commercial | `proxyman-cli install-root-cert … --trust` (macOS Keychain) | per-host SSL proxying toggles |
| No MITM: `*_BASE_URL` proxy + hooks + MCP | shipped | the current design | none | nothing to pin; covers every row of §21.1 |

Claude Code trusts its bundled Mozilla set plus the OS store by default (`CLAUDE_CODE_CERT_STORE=bundled,system`) and honours `HTTPS_PROXY`/`NO_PROXY` (code.claude.com network-config page, read 2026-09-23), so an interceptor would work for it without extra flags; clients that ship their own root set would reject the CA and must pass through untouched. Latency was not measured, since nothing is built.

### 21.3 Privacy rule recorded for any future interception work

Default-deny: no host is decrypted unless it is on an explicit allow-list of hosts that carry agent-visible text; everything else is a plain CONNECT tunnel. The CA key lives in `RTOK_HOME` with owner-only permissions, is never exported, and one command removes both the key and the trust entry. Nothing is stored beyond what `archive` already keeps under its retention. Creator choice, 2026-09-23.

## 22. Host junk map (T182) (2026-09-24)

Nothing here is deleted by rtok until the creator reviews the map; the first 17 hosts of the table below were re-verified against official docs or source repos via WebSearch/WebFetch on 2026-09-24; `commandcode`, `cline`, `mimo`, `devin`, `antigravity`, `roo` and `qwen` were added on 2026-10-08 (T391), so the table now covers all 24 hosts of `HOSTS` in `src/agents/mod.rs`, and a cell reads "not documented" rather than a guess whenever no host-specific official source names a path — a wrong row here can destroy a user's real data. Evidence: "documented" = the host's own docs site; "source" = the host's own repo (file cited); both give the full URL, never a site name.
Never junk, on any host: settings/config files, credentials and auth tokens (never read for expiry, never deleted), installed extensions/plugins, and a whole config/state directory named as if it were all junk (`~/.gemini/`, `~/.kimi-code/`, `~/Library/Application Support/Zed`, `~/.config/Code/` are never junk as a whole).
Session or conversation history and snapshots (e.g. pi's `~/.pi/agent/sessions`, aider's `.aider.chat.history.md`/`.aider.input.history`, Gemini's checkpoints) are never junk by default: rtok clears a session only when the user names `--kind sessions`, only on a host whose sessions cell in §22.1 documents the whole session unit and the index the host keeps beside it, and never the host's memory, index or store files; per-project snapshot stores are never cleared (D36, §22.1).
rtok clears only paths in this table, directories carrying a valid `CACHEDIR.TAG`, and paths the user names in `[agents.junk] extra`; platform cache roots and Electron subfolders without a row here are listed read-only with their size and never cleared (D36, §22.2).

| Host | Temp | Logs | Cache | Evidence | Checked |
| --- | --- | --- | --- | --- | --- |
| claude | `~/.claude/shell-snapshots/` (cleared on clean exit; sweep clears the rest) | `~/.claude/debug/`; legacy `~/.claude/logs/`, `todos/`, `statsig/` are no longer written | `~/.claude/paste-cache/` | documented: https://code.claude.com/docs/en/claude-directory | 2026-09-24 |
| cursor | not documented | not documented | not documented | not documented — closed-source app; docs confirm a `/logs` command exists but name no fixed path (https://cursor.com/changelog/04-14-26) | 2026-09-24 |
| codex | not documented | `$CODEX_HOME/log` (default `~/.codex/log/`) | not documented | documented: https://learn.chatgpt.com/docs/config-file/config-reference; source: https://github.com/openai/codex/blob/main/codex-rs/core/src/config/mod.rs#L4045 | 2026-09-24 |
| opencode | not documented | `~/.local/share/opencode/log/` (macOS/Linux), `%USERPROFILE%\.local\share\opencode\log` (Windows), auto-rotated | `~/.cache/opencode/` (macOS/Linux), `%USERPROFILE%\.cache\opencode` (Windows) | documented: https://opencode.ai/docs/troubleshooting/ | 2026-09-24 |
| kilo | not documented | not documented | not documented | not documented — CLI exposes a `kilo debug paths` runtime lookup instead of a fixed default (https://kilo.ai/docs/code-with-ai/platforms/cli-reference) | 2026-09-24 |
| pi | not documented | `~/.pi/agent/pi-debug.log` (written by `/debug`) | not documented | source: https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/usage.md | 2026-09-24 |
| omp | not documented | not documented | `~/.omp/cache/github-cache.db` only — the sibling `~/.omp/cache/auth-broker-snapshot.enc` is an encrypted credential snapshot; never delete the `cache/` folder itself | source: https://github.com/can1357/oh-my-pi/blob/main/docs/environment-variables.md | 2026-09-24 |
| zcode | `~/.zcode/cli/exec` (past terminal output; ZCode recreates it) | not documented | not documented | documented: https://zcode.z.ai/en/docs/qa | 2026-09-24 |
| kimi | not documented | `~/.kimi-code/logs/` | `~/.kimi-code/bin/` (downloaded `rg`/`fd`, redownloaded on use); `~/.kimi-code/updates/latest.json` | source: https://github.com/MoonshotAI/kimi-code/blob/main/docs/en/configuration/data-locations.md | 2026-09-24 |
| grok | not documented | `~/.grok/logs/` (`unified.jsonl`, MCP server logs; override `GROK_LOG_FILE`) | not documented | source: https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager/docs/user-guide/05-configuration.md | 2026-09-24 |
| vscode | not documented | not documented | `<user data>/CachedData/<commit>` for every commit but the current one (macOS `~/Library/Application Support/Code`, Linux `~/.config/Code`); never the `CachedData` folder itself or the newest commit folder (§22.2) | documented (mechanism only, no path given): https://docs.github.com/en/copilot/troubleshooting-github-copilot/viewing-logs-for-github-copilot-in-your-environment; user data folder: https://code.visualstudio.com/docs/configure/settings; source: https://github.com/microsoft/vscode/blob/8002c7893fcc/src/mainImpl.ts (`path.join(userDataPath, 'CachedData', commit)`) | 2026-10-08 |
| copilot | not documented | `~/.copilot/logs/` (recreated every session) | macOS `~/Library/Caches/copilot`, Linux `$XDG_CACHE_HOME/copilot` (or `~/.cache/copilot`), Windows `%LOCALAPPDATA%/copilot`, override `COPILOT_CACHE_HOME` | documented: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference | 2026-09-24 |
| aider | not documented | not documented | not documented | not documented — only documented files are `.aider.conf.yml` and the chat/input history, both excluded (https://aider.chat/docs/faq.html, https://aider.chat/docs/config/options.html) | 2026-09-24 |
| windsurf | not documented | not documented | not documented | not documented — Devin Desktop's Windsurf-migration FAQ names generic Electron `logs/`/`crashpad/`/cache categories but no Windsurf-specific path (https://docs.devin.ai/desktop/devin-desktop-faq) | 2026-09-24 |
| zed | not documented | macOS `~/Library/Logs/Zed/Zed.log`; Linux `~/.local/share/zed/logs/Zed.log` (or `$XDG_DATA_HOME/zed/logs/Zed.log`) | macOS `~/Library/Caches/Zed`; Linux `$XDG_CACHE_HOME/zed` — caution, issue #17835 shows Zed has also written `~/.cache/zed` on macOS by bug; verify the installed version first | documented: https://zed.dev/docs/troubleshooting, https://zed.dev/docs/macos; source (bug): https://github.com/zed-industries/zed/issues/17835 | 2026-09-24 |
| gemini | not documented — `~/.gemini/tmp/<hash>/` exists but holds checkpoints/shell history, which is session history, not junk | not documented | not documented | documented: https://geminicli.com/docs/cli/settings/, https://geminicli.com/docs/resources/troubleshooting/ | 2026-09-24 |
| codewhale | not documented | not documented — `audit.log` is tied to session reconciliation, not a pure rotating log | `~/.codewhale/update-check.json` (single file; caches the update-check result, reused for `check_interval_hours`) | source: https://github.com/Hmbown/Codewhale/blob/main/docs/CONFIGURATION.md | 2026-09-24 |
| commandcode | not documented | not documented | not documented | documented: https://commandcode.ai/docs/settings names only config, settings, auth, providers, keybindings, MCP, cron and per-project state files under `~/.commandcode`, and no log, cache or temp path | 2026-10-08 |
| cline | not documented | `~/.cline/data/logs/` (CLI and hub logs; the CLI log is `cline.log`, `CLINE_LOG_PATH` moves it, `CLINE_DATA_DIR` replaces `~/.cline/data/`, `CLINE_DIR` replaces `~/.cline/`) | not documented — a cache folder appears only in a user's issue report (**unverified**) | documented: https://docs.cline.bot/getting-started/config | 2026-10-08 |
| mimo | not documented | `~/.local/share/mimocode/log/` | `~/.cache/mimocode` (provider packages) | documented: https://mimo.xiaomi.com/mimocode/troubleshooting; config home `~/.config/mimocode` per https://mimo.xiaomi.com/mimocode/config-files | 2026-10-08 |
| antigravity | not documented | not documented | not documented — no Google page names a path; the `~/.gemini/antigravity*` and `~/.config/Antigravity` locations in circulation come from third-party guides and a forum thread only, **unverified** (https://discuss.ai.google.dev/t/bug-antigravity-keeps-big-cache-directories-in-config-antigravity/129240) | 2026-10-08 |
| devin | not documented | `~/.local/share/devin/cli/logs/devin_<timestamp>_<pid>.log` (macOS/Linux; Windows `%APPDATA%\devin\cli\logs`) | not documented — the page names no cache folder; user config is `~/.config/devin/config.json` (read from a search snippet of the config-file page, **unverified**) | documented: https://docs.devin.ai/cli/troubleshooting, https://docs.devin.ai/cli/reference/configuration/config-file | 2026-10-08 |
| roo | not documented | not documented | not documented — the per-task folders under the extension's `globalStorage` hold task history (§22.1 rule) | not documented — a 2026-10-08 search of docs.roocode.com found no log or cache path; the storage root is only in source (https://github.com/RooCodeInc/Roo-Code/blob/main/src/utils/storage.ts) | 2026-10-08 |
| qwen | not documented — `~/.qwen/tmp/<project_hash>/shell_history` is shell history, not junk | not documented — the settings page says session debug logs sit "under the runtime `debug/` directory" without a path | not documented | documented: https://qwenlm.github.io/qwen-code-docs/en/users/configuration/settings/ (`QWEN_HOME`, default `~/.qwen`) | 2026-10-08 |
| rtok (own, T330.3.1) | not documented | `rtok.log` siblings and archive payloads (T182) | `<project>/.rtok-lsp-xdg/{cache,pub-cache}` for every registered project (cargo workspaces `target/rtok-lsp-xdg/…`, Dart `.dart_tool/rtok-lsp-xdg/…`), cleared only when idle (T152); `$XDG_CACHE_HOME/rtok` (macOS `~/Library/Caches/rtok`, Windows `%LOCALAPPDATA%\rtok\cache`): rtok writes nothing there yet, the name is rtok's own namespace under the platform cache root | source: https://github.com/pyrlyn/rtok/blob/3ab01d79/src/plugins/graph/lsp.rs#L146-L173 (the confined state dirs); cache roots per https://specifications.freedesktop.org/basedir-spec/latest/ and Apple's file-system guide (§22.2) | 2026-10-06 |

### 22.1 T330 junk kinds vs "never junk": sessions, tokens, snapshots (T338) (2026-10-03)

The conflict: T330 deletes `sessions` (`review`, default threshold 3 days), `stale-tokens` (explicit only) and `snapshots` (`review`); the §22 rule above says credentials, auth tokens and session or conversation history are never junk on any host. T330.1 (PR #651, merged 2026-10-03) shipped only the read-only `list` over rtok's own T182 junk, so that slice takes no side; the choice is D36 (§22.2). Checked 2026-10-03; primary sources only, secondary ones marked **unverified**.

**What the hosts themselves say about these files.**

| Host | Sessions (where, who prunes) | Credentials | Snapshots / checkpoints | Source |
| --- | --- | --- | --- | --- |
| claude | `~/.claude/projects/<project>/<session>.jsonl` plus `<session>/subagents/` and `<session>/tool-results/`; Claude Code's own retention sweep deletes them after `cleanupPeriodDays` (default 30, minimum 1, `0` is a validation error); `claude project purge [--dry-run] [-i]` deletes one project's state. The same `projects/` tree also holds **auto memory**, and deleting it loses "Resume, continue, and rewind for past sessions, and auto memory for every project". | macOS Keychain; `~/.claude/.credentials.json` (mode `0600`) on Linux/Windows and as the macOS fallback; logins refresh automatically; `/logout` "removes and revokes" the credential | `~/.claude/file-history/<session>/`, used by `/rewind`, deleted by the same sweep "about 30 days after the session last saved one" | https://code.claude.com/docs/en/claude-directory, https://code.claude.com/docs/en/authentication, https://code.claude.com/docs/en/checkpointing (read 2026-10-03) |
| codex | `$CODEX_HOME/sessions/` and `archived_sessions/` (constants `SESSIONS_SUBDIR`, `ARCHIVED_SESSIONS_SUBDIR`); a reference index records fork edges between rollout files, and a SQLite state DB (`sqlite_home`) holds resumable state; no session retention key in the config reference (only `history.max_bytes` for `history.jsonl`) | `$CODEX_HOME/auth.json` holds `OPENAI_API_KEY`, `tokens` (id, access, refresh) and `last_refresh` in one file; `cli_auth_credentials_store` = `file \| keyring \| auto \| ephemeral` | not documented | https://github.com/openai/codex/blob/dff5270b298b/codex-rs/rollout/src/lib.rs#L86-L87, …/codex-rs/rollout/src/rollout_reference_index.rs, …/codex-rs/login/src/auth/storage.rs#L47-L63, https://learn.chatgpt.com/docs/config-file/config-reference (main `dff5270b298b`, release `rust-v0.160.0`, read 2026-10-03) |
| gemini | `~/.gemini/tmp/<project_hash>/chats/`; Gemini CLI's own `general.sessionRetention` (`enabled` default true, `maxAge` default 30 days, `maxCount`, `minRetention` default 1 day) | OAuth credentials move from `~/.gemini/oauth_creds.json` into keychain-backed storage on first read (`migrateFromFileStorage`) | shadow git repo `~/.gemini/history/<project_hash>` plus `~/.gemini/tmp/<project_hash>/checkpoints`, used by `/restore`; the checkpointing page documents no cleanup | https://geminicli.com/docs/cli/session-management/, https://geminicli.com/docs/cli/checkpointing/, https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/config/storage.ts, …/packages/core/src/code_assist/oauth-credential-storage.ts (release `v0.62.0`, read 2026-10-03) |
| copilot | `~/.copilot/session-state/` "Session history and workspace data", not auto-cleaned, "With caution": "You will no longer be able to resume past sessions"; `session-store.db` holds cross-session data | `config.json` holds authentication together with installed plugins and internal state; `mcp-oauth-config/` holds MCP OAuth tokens | not documented | https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference (read 2026-10-03) |
| kimi | `~/.kimi-code/sessions/<workDirKey>/<sessionId>/` indexed by `session_index.jsonl`; the docs' cleanup recipe deletes both together and calls it removing "all conversation history" | `~/.kimi-code/credentials/<name>.json`; the documented way to clear provider auth is `/logout` | not documented | https://github.com/MoonshotAI/kimi-code/blob/main/docs/en/configuration/data-locations.md (read 2026-10-03) |
| opencode | `~/.local/share/opencode/project/<slug>/storage/` ("session and message data") | `~/.local/share/opencode/auth.json` ("API keys, OAuth tokens") | not documented | https://opencode.ai/docs/troubleshooting/ (read 2026-10-03) |
| cline | not documented | not documented | a shadow git repo per workspace; the docs give no path and no cleanup | https://docs.cline.bot/core-workflows/checkpoints (read 2026-10-03) |

Every other host in `HOSTS`: not documented for these three columns (§22 table above and §26).

**Findings that decide the question.**

1. Sessions are user data on every host that documents them: they back resume, rewind and history search, and two hosts already age them out themselves (Claude Code and Gemini CLI, both 30 days by default). T330's 3-day default is ten times more aggressive than either host's own default.
2. Sessions are not self-contained files. Claude Code keeps auto memory inside the same `projects/` tree; Codex indexes fork edges between rollouts (a forked session can rest on an older one) and keeps a state DB; Kimi and Copilot keep an index or store next to the session files. A time-only `rm` behind the host's back can delete the base of a newer fork or leave the host's index pointing at missing files. Where a host ships its own delete (`claude project purge`, Gemini's retention), that path keeps the host consistent; rtok's own walk does not.
3. Token files are not caches. Access-token expiry is normal: Claude Code and Codex refresh it (`last_refresh`, a stored refresh token), so "stored expiry in the past" does not mean the file is stale. Codex's `auth.json` mixes an API key with tokens; Copilot's `config.json` mixes auth with plugins and state; Gemini migrates the legacy file into the keychain on first read. Deleting the file does not revoke anything server-side (Claude Code's `/logout` does). The space freed is a few KB.
4. Snapshots are rewind data. Claude Code ties them to the session and already ages them out; Gemini's shadow-git history has no documented cleanup and can grow, but it is the only way to `/restore`.

**Approach A: keep T330, relax §22.** Sessions are `review` (in `--include review`, 3-day threshold), snapshots `review`, token files deletable by name. Useful: the largest folders on a heavy user's disk are often transcripts. Risk: highest. A broad `--include review` run deletes resumable sessions, Claude Code auto memory if the walk is not exact, Gemini restore points, and signs the user out; a wrong row in §22 now costs history instead of a cache. Maintenance: rtok must track every host's session layout and index files, which change between releases (Claude Code's table above changed in v2.1.248 and v2.1.274).

**Approach B: keep §22, drop the three kinds.** `sessions`, `snapshots` and token files become `never`: listed with size only, never deleted, and the row names the host's own control where one exists (`cleanupPeriodDays` / `claude project purge`, `general.sessionRetention`). Risk: none added. Usefulness: `list` still shows where the space is and how to free it; `clear` frees less. Maintenance: low; rtok only reads sizes.

**Approach C: keep §22 as the default, allow sessions by explicit name only.** Credentials stay absolute (token files dropped, as in B). Sessions and snapshots are class `never` for every default and `--include review` run, and are deleted only with `--kind sessions` (snapshots go with their session where the host ties them, as Claude Code's `file-history/<session>/`), only on hosts whose §22 row documents the whole session unit and its index, never the host's memory, index or store files themselves; per-project shadow-git history (Gemini) stays `never`. `list` shows the host's own retention setting next to rtok's threshold. Risk: low; deleting history needs a deliberate flag and a documented layout. Usefulness: keeps the creator's `stale_session_days` feature for hosts that do not prune themselves (Codex, Copilot, Kimi, opencode). Maintenance: one new §22 column (Sessions), verified per host like the others.

**Recommendation: C.** It keeps the §22 safety rule as the default, removes the one kind with no upside (token files: KB freed, sign-out and keychain-migration risk), and keeps session cleanup as an explicit, documented-only action instead of dropping it. If the creator prefers the smallest surface, B is the fallback; A is not recommended. The 3-day default then applies only to an explicit `--kind sessions` run.

**Decision (creator, 2026-10-03):** C, recorded as D36; `stale_session_days` default raised from 3 to 30 to match Claude Code and Gemini.

**Per-host verdict for `--kind sessions` (T330.5.2, 2026-10-08).** A host takes part only when its sessions cell above names the whole session unit and no index sits beside it that rtok would have to edit. Sources: the table above (read 2026-10-03) and, for Gemini, https://geminicli.com/docs/cli/session-management/ (re-read 2026-10-08).

| Host | Verdict | Why |
| --- | --- | --- |
| claude | yes | One session is `projects/<project>/<session-id>.jsonl`, the sibling folder `<session-id>/` (subagents, tool results) and `file-history/<session-id>/`; the docs name no index beside them. `projects/<project>/memory/` never matches (a session is a UUID-named transcript) and stays. |
| codex | no | A rollout index records fork edges between sessions and a SQLite state DB holds resumable state: removing one rollout can leave either pointing at a missing file. |
| gemini | no | The page names `~/.gemini/tmp/<project_hash>/chats/` but not the files of one session (it says deleting a session also removes its plans, task trackers, tool outputs and activity logs without a path for each). Gemini prunes by `general.sessionRetention` itself. |
| copilot | no | `session-store.db` holds cross-session data beside `session-state/`. |
| kimi | no | `session_index.jsonl` indexes every session; removing one needs the index edited. |
| opencode | no | Session and message data sit in one `project/<slug>/storage/` per project, not per session. |
| cline and every other host | no | Not documented (§22 and §26). |

`list` shows the host's own retention next to rtok's threshold for Claude Code (`cleanupPeriodDays`, default 30) and Gemini (`general.sessionRetention.maxAge`, default 30d), read from the host's `settings.json`.

### 22.2 T330 cache detection: §22 paths only vs heuristics (T339) (2026-10-03)

The conflict: T330 says paths come only from §22 and "not documented" cells are not scanned, yet detects agent caches from platform cache roots, Electron/Chromium subfolders, `CACHEDIR.TAG` and all of `~/Library/Caches`, and its Check clears Cursor caches although every Cursor cell in §22 is "not documented". Checked 2026-10-03.

**What each heuristic rests on.**

| Heuristic | Primary evidence | What it does not prove |
| --- | --- | --- |
| `CACHEDIR.TAG` with the signature `Signature: 8a477f597d28d172789f06886806bc55` | The spec: the tag means the entire directory "consists of cached information that can be re-generated if necessary". The owning program writes it, so it is that program's own declaration. https://bford.info/cachedir/ (read 2026-10-03) | The spec grants backup tools leave to skip the directory; it does not say other programs may delete it (T342 covers the T152 rules for deleting tagged dirs). |
| macOS `~/Library/Caches/<app>` | Apple: put "support files that your app can re-create easily" there. https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/FileSystemProgrammingGuide/FileSystemOverview/FileSystemOverview.html (read 2026-10-03) | Apple's guarantee is a convention for the app's author, not proof that a given third-party app follows it (Zed has written caches to the wrong root by bug, §22 table). Which entry belongs to which agent needs the host's bundle id or name, which §22 does not record for most hosts. |
| `$XDG_CACHE_HOME/<app>`, `%LOCALAPPDATA%\<app>\Cache` | Same convention per platform; §22 already cites hosts that document theirs (copilot, opencode, zed). | Same: a convention, not a host statement. |
| Electron/Chromium subfolders (`Cache`, `Code Cache`, `GPUCache`, `DawnCache`, `Service Worker/CacheStorage`) | Electron: `sessionData` holds "localStorage, cookies, disk cache, downloaded dictionaries, network state, DevTools files and compiled GPU shaders" and "By default this points to `userData`". https://www.electronjs.org/docs/latest/api/app (latest, read 2026-10-03) | The same folder mixes caches with localStorage and cookies, so only individual subfolders can be cache; Electron's API docs do not name the subfolders. `Service Worker/CacheStorage` is the web Cache API, i.e. stored app data, not a disk cache (**unverified** as a Chromium statement). |
| VS Code `CachedData/<commit>` | VS Code source: code cache at `<userData>/CachedData/<commit>`; VS Code deletes folders other than the current commit's after ~3 months (stable) or 1 week (insiders). https://github.com/microsoft/vscode/blob/8002c7893fcc/src/mainImpl.ts#L667, …/src/vs/code/electron-utility/sharedProcess/contrib/codeCacheCleaner.ts#L27-L35 (main `8002c7893fcc`, read 2026-10-03) | Source-backed for VS Code only. That Cursor inherits this layout and cleaner is **unverified** (closed source; a Cursor forum report says its `~/.config/Cursor/CachedData` grows without cleanup, https://forum.cursor.com/t/old-cache-directories-in-config-cursor-cacheddata-accumulate-indefinitely-without-automatic-cleanup/141573, **unverified**). |
| Cursor | No Cursor docs page names a data, cache or log path (§22 row; re-checked 2026-10-03 with a search limited to cursor.com and docs.cursor.com). | The paths in circulation (`~/Library/Application Support/Cursor/Cache`, `CachedData`, `User/workspaceStorage`) come from forum posts only, **unverified**; `User/workspaceStorage` holds chat history, so a wrong guess there destroys sessions. |

**Approach A: §22 only.** Scan and clear only cells §22 documents; everything else is invisible. Risk: lowest. Usefulness: low on today's map: most hosts' Cache cells are "not documented", Cursor has nothing, so `list` hides the space the user is asking about. Maintenance: every new path needs a cited §22 row first.

**Approach B: heuristics as written.** Scan and clear platform roots, Electron folders and tagged dirs for every host. Useful: finds most cache bytes, Cursor included. Risk: the one §22 warns about; Cursor's guessed paths sit next to chat history, `~/Library/Caches` attribution is a guess, and a running Electron app may hold the files. Maintenance: low to write, high to keep correct across host releases.

**Approach C: evidence decides what is cleared; heuristics only inform `list`.** `clear` deletes only paths with evidence: (1) a §22 cell (docs or source); (2) a directory carrying a valid `CACHEDIR.TAG`, the owner's own declaration (deletion rules per T342); (3) a path the user names in `[agents.junk] extra`. Platform cache roots and Electron subfolders are scanned read-only: `list` shows them under the agent with size and the note "not documented: not cleared (add to `[agents.junk] extra` to clear)", and they never count toward "Freed by `clear`". Cursor is `list`-only until a primary source names its paths. A heuristic path that later gets a cited §22 row moves to (1) with no code change beyond the map. Risk: low; nothing is deleted on a guess. Usefulness: the user sees every large cache folder, and freeing the undocumented ones takes one explicit config line. Maintenance: same as A for deletion, plus a small read-only scanner.

**Recommendation: C.** `list` is read-only (T330.1 shipped it that way in PR #651), so showing heuristic finds costs no data; only deletion needs evidence. It keeps §22's rule for everything `clear` removes and makes the user, not rtok, the source for any undocumented path. Candidate §22 rows for T330.3, each to be added only with its citation: VS Code `CachedData/<commit>` other than the current commit (VS Code source above). Added as a §22 row on 2026-10-08 (T391). VS Code's cleaner receives the running commit from the app (`codeCacheCleaner.ts` only takes the `basename` of the current cache path), which rtok cannot read, so `clear` treats the most recently modified commit folder as current and keeps it, offers the others under T152's idle rule, and never touches the `CachedData` folder itself; the cleaner's own age limit (~3 months stable) is not applied.

**Decision (creator, 2026-10-03):** C, recorded as D36.

## 23. Subagent-start context injection per host (T262.2) (2026-09-24)

Question: which hosts let a hook add context to a sub-agent before it runs, the way Claude Code's `SubagentStart` returns `hookSpecificOutput.additionalContext` (the T130 spawn brief)? Checked from each host's hook docs or source; the two yes rows re-read first-hand.

| Host | Verdict | Event, output | Source |
| --- | --- | --- | --- |
| Claude Code | yes | `SubagentStart`, `additionalContext` | wired in T130.2 |
| VS Code Copilot Chat | yes | runs `plugins/claude` hooks as-is | `src/agents/vscode/mod.rs` |
| Codex | yes | `SubagentStart`; plain stdout or hook-specific context becomes developer context for the subagent | https://learn.chatgpt.com/docs/hooks |
| Copilot CLI | yes | `subagentStart` (matcher on agent name), `additionalContext` prepended to the subagent's prompt; the built-in general-purpose agent emits no event | https://docs.github.com/en/copilot/reference/hooks-reference |
| Kimi | event-only | `SubagentStart` fires; the result of `runner.trigger` is discarded | MoonshotAI/kimi-code `packages/agent-core-v2/src/features/externalHooks/session/sessionExternalHooksService.ts` |
| Cursor | event-only | `subagentStart` output has only `permission` / `user_message` | https://cursor.com/docs/hooks |
| Grok | event-only (weak) | `SubagentStart` / `SubagentStop` fire; re-checked 2026-10-08: no output schema and no returned fields documented for them (only `PreToolUse` documents an output, `decision` and `reason`; passive events' stdout is ignored); no `invoke_subagent` event | https://docs.x.ai/build/features/hooks (checked 2026-10-08) |
| CodeWhale | event-only | `subagent_spawn` is an observer event; result discarded | `src/agents/codewhale/README.md` |
| Gemini CLI | no | no subagent event (`BeforeAgent` / `AfterAgent` are the parent turn) | https://geminicli.com/docs/hooks/reference/ |
| ZCode | no | no subagent event | https://zcode.z.ai/en/docs/hooks |
| OpenCode, Kilo | no | plugin events have no subagent spawn | https://opencode.ai/docs/plugins |
| Pi, omp | no | no hookable spawn; subagents are an extension of their own | badlogic/pi-mono `docs/extensions.md` |
| Windsurf | no | no subagent event among the documented hooks | https://docs.devin.ai/desktop/cascade/hooks |
| Cline | no | `new_task` hands off in the same conversation, no child agent | https://docs.cline.bot/customization/hooks |
| Antigravity | no (weak) | re-checked 2026-10-08: five events only (`PreToolUse`, `PostToolUse`, `PreInvocation`, `PostInvocation`, `Stop`), no subagent event; `invoke_subagent` is a tool name, so a `PreToolUse` matcher could hit it, but that output is only `decision` and `reason` (deny), so no context reaches the subagent | https://antigravity.google/docs/hooks/ (checked 2026-10-08) |
| MiMo | no | no hook system | mimo docs |

Follow-ups: T262.3 (Codex) and T262.4 (Copilot CLI). Grok and Antigravity rest on missing docs, so a docs change there is worth a recheck. Rechecked 2026-10-08 (T399): still no output schema for Grok's subagent events and no subagent hook on Antigravity; verdicts unchanged, no spawn-brief task filed. Found on the way: Copilot CLI `subagentStop` accepts `modifiedResponse`, which replaces the subagent's answer to the parent (idea I-98).

## 24. Cloud MCP mode for the Grok API (2026-09-26)

The Grok API has no hooks and reaches MCP only over Streamable HTTP or SSE, from xAI's servers. Findings, sources and the proposed stdio/http split live in [`docs/research/grok-cloud-mcp.md`](docs/research/grok-cloud-mcp.md).

## 25. Same-name MCP servers: plugin vs config entry, per host (T275) (2026-09-27)

What each agent does when rtok's MCP server is declared both in its own config file and by an rtok plugin it loads. Checked 2026-09-27 against the agent's docs and, where installed here, `<cli> --version`.

| Host | Version checked | Behaviour | Source |
| --- | --- | --- | --- |
| Claude Code | 2.1.267 | Plugin servers are namespaced (`plugin:rtok:rtok`, tools `mcp__plugin_rtok_rtok__*`), so they never merge with a config `rtok`: two servers. Same-name entries across local/project/user scopes resolve by precedence. | https://code.claude.com/docs/en/mcp |
| Claude Desktop, Code tab | Claude Code 2.1.267 | Two servers (`mcp__rtok__*` from `claude_desktop_config.json` plus the plugin's), observed by the creator on 2026-09-26 (T271). Whether it merges a desktop `rtok` with a `~/.claude.json` `rtok` is **unverified**; the T275 Check confirms it on the creator's machine. | plan.md T271 |
| Gemini CLI | not installed | settings.json wins over an extension's same-name server: one server. | https://geminicli.com/docs/extensions/reference |
| Codex CLI | 0.155.1 | Plugin servers live under `plugins.<plugin>.mcp_servers.<server>`, a separate table: likely two servers (**unverified**, no literal quote). | https://learn.chatgpt.com/docs/extend/mcp?surface=cli |
| Cursor | cursor-agent 2026.09.23 | Docs silent on duplicates: **unknown**. | https://cursor.com/docs/context/mcp |
| Copilot CLI | 1.0.83 | Docs silent: **unknown**. | https://docs.github.com/en/copilot/how-tos/provide-context/use-mcp/extend-copilot-chat-with-mcp |
| VS Code / Insiders | not checked | Docs silent: **unknown**. | https://code.visualstudio.com/docs/copilot/customization/mcp-servers |
| ZCode, Kimi (0.39.1), Grok Build (1.0.34) | as listed | No public docs found: **unknown**. | — |
| omp, pi, Antigravity | — | No conflict: omp's and pi's plugins serve no MCP; Antigravity's plugin is its only MCP path. | repo `src/agents/<host>/mod.rs` |

Decision (D33): Claude and every host marked unknown or two servers drop MCP from the rtok plugin and keep the config entry `rtok`; Gemini keeps both, since settings.json wins.

## 26. Worktrees and session ids per host (T281–T290) (2026-09-27)

Creator request 2026-09-27: rtok manages worktrees for every agent the same way, gives every agent a unique id, and lets the user and agents message each other by that id. This section records what each host offers for that. Not a token saving: no `Measurement` row follows from it.


Checked 2026-09-27 unless noted. Primary sources only; secondary sources marked **unverified**.
Hosts = `src/agents/*` (21 on 2026-09-27: aider, antigravity, claude, cline, codewhale, codex, copilot, cursor, devin, gemini, grok, kilo, kimi, mimo, omp, opencode, pi, vscode, windsurf, zcode, zed; `commandcode` landed 2026-09-30, `roo` and `qwen` 2026-10-05, so 22 on 2026-10-04 and 24 on 2026-10-08).

| Host | Native worktrees | Interception mechanism | Session id exposure | Start/end hooks | Source |
|---|---|---|---|---|---|
| Claude Code | yes — `.claude/worktrees/<name>`, `--worktree`, subagent `isolation:"worktree"` | `WorktreeCreate`/`WorktreeRemove` hooks *replace* default create/remove; nonzero exit aborts create, or fails remove if dir still exists; a command hook returns the path as its last stdout line (§18.3, T156) | `session_id` in every hook's JSON stdin (common field); subagents carry parent's `session_id` + own `agentId` | `SessionStart`/`SessionEnd`, matchers `startup/resume/clear/compact/fork` and `clear/resume/logout/prompt_input_exit/other` | https://code.claude.com/docs/en/hooks, /worktrees |
| Cursor | yes, `~/.cursor/worktrees/`, cap 25, 6h sweep | none for create/remove itself — only `.cursor/worktrees.json` keys `setup-worktree[-unix\|-windows]` run a script *after* creation; no block/replace | `sessionStart` payload has `session_id` (= `conversation_id`); all hooks carry `conversation_id`, `generation_id` | `sessionStart`/`sessionEnd` documented | https://cursor.com/docs/hooks, /configuration/worktrees |
| Codex (OpenAI) | yes, `$CODEX_HOME/worktrees`, `thread-N`, detached HEAD, keeps 15 newest | none documented | `session_id` common field; turn-scoped adds `turn_id` | `SessionStart`(`source`)/`SessionEnd`(`reason`) | https://learn.chatgpt.com/docs/hooks, /environments/git-worktrees |
| Copilot CLI/desktop (`~/.copilot`, rtok's target) | not documented for this surface (the VS Code "Agents window" worktree feature is a separate product surface, not `~/.copilot`) | none found; app "does not document hooks" per rtok's own install-code comment | `sessionId` in `sessionStart`/`sessionEnd` payload (camelCase) | `sessionStart`(`source`)/`sessionEnd`(`reason`) documented for the CLI | https://docs.github.com/en/copilot/reference/hooks-reference |
| VS Code (Copilot Chat plugin surface) | `git.worktreeIncludeFiles` setting exists for the separate "Agents window" feature; exact worktree path **unverified** | none — VS Code runs the linked Claude-shaped plugin's hooks verbatim, and that set (`SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, PreCompact, SubagentStart, SubagentStop, Stop`) has no Worktree hooks | same as Claude Code plugin payload (`session_id`) | `SessionStart` yes; no separate `SessionEnd` in the linked set | src/agents/vscode/mod.rs (T117), https://code.visualstudio.com/docs/copilot/agents/copilot-cli |
| Grok Build (xAI) | yes — `--parallel` spawns ≤8 sub-agents each in its own worktree; exact dir/naming **unverified** | none documented for create/remove | `sessionId` in payload; env vars `GROK_HOOK_EVENT`, `GROK_HOOK_NAME`, `GROK_SESSION_ID`, `GROK_WORKSPACE_ROOT` | `SessionStart`/`SessionEnd` documented | https://docs.x.ai/build/features/hooks, x.ai/news/grok-build-cli |
| Gemini CLI | not found/no evidence of native worktrees | none | `session_id` in stdin JSON; also `GEMINI_SESSION_ID` env var | `SessionStart` (startup/resume/`/clear`) / `SessionEnd` (exit/clear) | https://geminicli.com/docs/hooks/reference |
| Kimi Code CLI (Moonshot) | not documented | none — no worktree hooks in the 20-event list | `session_id` in every hook's stdin, plus `session_title`, `client_type`, `cwd` | `SessionStart`/`SessionEnd`/`SessionHeartbeat` documented | https://moonshotai.github.io/kimi-code/en/customization/hooks.html |
| MiMo Code (Xiaomi) | yes — `auto_worktree` config toggle (default off), "orchestrator mode" gives per-child-session worktree isolation | none documented; "auto-worktree notice" fires on main-worktree mutations (observational, not a gate) | session id inheritance for same-session subagents mentioned; exact field name **unverified** (no docs page fetched, only GitHub release notes) | not confirmed — no hooks doc fetched | https://github.com/XiaomiMiMo/MiMo-Code releases (v0.1.14, 2026-09-23) — secondary-quality (changelog, not a hooks reference) |
| omp (Oh My Pi) | yes, per-task `isolated:true` → git worktree / fuse-overlay / ProjFS; whole-session isolation is an open feature request (**unverified**, issue #452) | none — isolation is a task-definition config key, not an interceptable create/remove event | `sessionId` field appears in `tool_approval_requested`/`resolved`/`session_stop` payloads | `session_start`/`session_shutdown` events exist (no fields documented) | https://omp.sh/docs/hooks |
| OpenCode | no built-in worktree manager; plugin ctx exposes `worktree` (the git worktree path opencode itself is running in); third-party `opencode-worktree` plugin builds create/cleanup on top using `session.idle` | none native; a plugin can only react to generic session events, not intercept git's own create/remove | not documented in the plugin-context page fetched (**unverified**) | `session.created`/`session.idle`/`session.deleted` etc.; no dedicated "SessionStart/End" pair | https://opencode.ai/docs/plugins |
| Kilo Code (built on the OpenCode server, `~/.config/kilo/`) | yes — Agent Manager creates managed worktrees under `.kilo/worktrees/`, branch prefix/auto-name or explicit name | `.kilo/setup-script`(`.ps1`) runs post-create only, receives `WORKTREE_PATH`/`REPO_PATH`; no pre-remove hook — "closing a managed worktree" deletes the directory directly | not documented (**unverified**) | not confirmed for Kilo's own session hooks (its plugin format is OpenCode's `tool.execute.before/after`, `shell.env`, no `SessionStart`) | https://kilo.ai/docs/automate/agent-manager |
| Pi (earendil-works) | core has none; community extensions (`pi-worktree`, `pi-git-worktrees`) add it via "project-level post-create/pre-remove hooks" — **unverified**, not in the primary `extensions.md` | none in core; extensions hook generic `session_start`/`agent_end`/`tool_call` etc., not worktree-specific | not documented in `extensions.md` (**unverified**) | `session_start`/`session_shutdown` (resource lifecycle, not literal "SessionStart/End" event names) | https://github.com/earendil-works/pi …/docs/extensions.md |
| ZCode (Z.ai) | not found/no evidence | none found (hooks page has no worktree event) | `"session_id"` in every hook's stdin JSON | `SessionStart`/`Stop` (no explicit `SessionEnd` distinct from `Stop`) | https://zcode.z.ai/en/docs/hooks |
| CodeWhale (Hmbown, DeepSeek-focused) | not documented | none — 15 events, none worktree-related | `DEEPSEEK_SESSION_ID` in every event except `shell_env`, format `sess_xxxxxxxx` | `session_start`/`session_end` documented | https://github.com/Hmbown/Codewhale/blob/main/docs/HOOKS.md |
| Devin Desktop (Cascade) | yes, shares Windsurf's pool: `~/.windsurf/worktrees/<repo_name>`, random name, cap ~20 workspace-wide LRU eviction; deleting a conversation removes its worktree | `post_setup_worktree` fires **after** creation only (post-create, not a gate/replace); receives `$ROOT_WORKSPACE_PATH`; no pre-create or pre/post-remove hook | `trajectory_id` (conversation) + `execution_id` (single turn) — no field literally named "session id" | none — no `SessionStart`/`SessionEnd` among Cascade's 12 events | https://docs.devin.ai/desktop/cascade/worktrees, /hooks |
| Windsurf (Cascade) | same as Devin above — Cognition owns both; `PostSetupWorktreeEvent`(https://docs.windsurf.com wording) = `post_setup_worktree` | same `post_setup_worktree`, post-create only | same `trajectory_id`/`execution_id` | none | https://docs.windsurf.com/windsurf/cascade/worktrees; https://docs.devin.ai/desktop/cascade/hooks (same underlying doc set) |
| Antigravity (Google) | yes — per-conversation "New Worktree Mode" in Projects, isolates each agent/subagent; exact directory/naming **unverified** | none — only 5 hooks (`PreToolUse/PostToolUse/PreInvocation/PostInvocation/Stop`), none worktree-related | `conversationId` (UUID) in every payload; no field named "session id" | none — no `SessionStart`/`SessionEnd`; `Stop` only ends the execution loop, not the session | https://antigravity.google/docs/hooks/, /docs/projects/ |
| Cline | not found/no evidence (Cline uses a shadow-git "checkpoints" repo instead of worktrees) | n/a | not documented (checked 2026-10-08): https://docs.cline.bot/customization/hooks names no parent process or session id and points to SDK Plugins, and https://docs.cline.bot/sdk/plugins does not cover hook processes (**unverified**) | not confirmed | https://docs.cline.bot/customization/hooks, https://docs.cline.bot/sdk/plugins, https://docs.cline.bot/core-workflows/checkpoints |
| aider | no — no worktree feature in official docs; only secondary blog posts describe manual pairing with worktrees | n/a | no hook/event system found in aider's docs at all | no | https://aider.chat/docs/git.html, /docs/faq.html |
| Zed | no auto-managed worktree pool the way other hosts have one; a "thread" can be pointed at a plain git worktree the user creates themselves, chosen per-thread — exact mechanics **unverified** | none — Zed has **no shell hook events at all** (confirmed in rtok's own `src/agents/zed/mod.rs`: "external agents can reach them over ACP") | ACP gives a stable session id via `session/new`/`session/load`, but Zed itself issues no hook payload with one | none — no hook system to have start/end hooks in | https://zed.dev/docs/ai/parallel-agents, https://zed.dev/acp; src/agents/zed/mod.rs |

## Notes worth flagging

1. **Devin and Windsurf are the same agent.** Cognition acquired Windsurf; "Devin Desktop's Cascade" and "Windsurf's Cascade" share the worktree pool path (`~/.windsurf/worktrees/<repo>`), the same 12 hook events, and the same `post_setup_worktree` hook. Treat them as one mechanism with two doc sites (https://docs.devin.ai and https://docs.windsurf.com), not two independent implementations.
2. **Only Claude Code documents a true create/remove interception hook** (`WorktreeCreate`/`WorktreeRemove`, replace + nonzero-exit-aborts semantics). Every other host that has both worktrees and hooks (Cursor, Codex, Grok Build, MiMo, Kilo, omp) only offers a **post-create setup hook/script** (Cursor's `setup-worktree*`, Devin/Windsurf's `post_setup_worktree`, Kilo's `.kilo/setup-script`) — none can block or redirect creation, and none has a pre-remove hook, so rtok cannot rely on those to enforce ownership/naming/cleanup rules the way it can on Claude Code.
3. **`session_id` (or an equivalent) is a near-universal hook field** — Claude Code, Cursor, Codex, Copilot CLI, Grok Build, Gemini CLI, Kimi, ZCode, CodeWhale, omp all pass one. The two ACP-only surfaces (Zed, and Kimi's own `kimi acp` mode) instead rely on the Agent Client Protocol's `session/new`/`session/load` identity, which is a different mechanism from a hook payload field. Devin/Windsurf uses `trajectory_id`+`execution_id` instead of a literal "session id". Antigravity uses `conversationId`. Aider, Zed (non-ACP) and Pi's core have nothing.
4. **SessionStart/SessionEnd pairs exist on:** Claude Code, Cursor, Codex, Copilot CLI, Grok Build, Gemini CLI, Kimi, ZCode (Stop, no distinct End), CodeWhale. **Missing on:** VS Code's linked plugin set (Start only), Devin/Windsurf, Antigravity, OpenCode/Kilo (event-based, no Start/End pair), Pi (resource-lifecycle naming, not literal session boundary), Zed, Cline, https://aider.
5. Weakest coverage, flagged rather than guessed: Cline (hooks docs name no parent process or session id, checked 2026-10-08), Pi's worktree extensions (community, not core docs), Kilo's own session-hook existence, MiMo's exact session-id field name and worktree directory — all marked **unverified** above rather than asserted.

Consequences for the plan (D34): only Claude Code can redirect creation and removal (T159). Cursor, Kilo and Devin/Windsurf run a script after they create a worktree, which is where `rtok worktree adopt` hooks in (T289). Every other host gets rtok's worktrees through the skill and the MCP tools only (T285, T286). The host session id is not unique across hosts and is missing on several, so rtok issues its own agent id (T282).

### MCP process ↔ host session link (T281)

`rtok mcp` is started by the host with only `["mcp"]` and its cwd (`src/mcp.rs:592`); `Server::new` (`src/mcp.rs:272`) never reads `initialize.params.clientInfo` or `_meta` beyond `capabilities.roots` (T263) and `protocolVersion`, and no plugin passes a `--host`/session flag yet (T283 adds that). This subsection is what the vendor docs and, where open source, the vendor's own spawn code say about linking that process back to a session, ahead of the live probe (T280: real agents are for manual debugging only, never tests). Checked 2026-09-27 unless noted; commits pinned where a repo is open source. A blog, forum thread or other people's issue is a lead only, marked **unverified**; unknown is an acceptable answer.

| Host | Version / doc date | Session id env var MCP process inherits | Hook payload session field | `initialize` clientInfo / `_meta` | One MCP process per session? | Sub-agents share parent's MCP process? | Proposed link rule | Probe |
|---|---|---|---|---|---|---|---|---|
| Claude Code CLI | 2.1.267 (research.md §25); code.claude.com/docs/en/mcp, /hooks, /sub-agents, 2026-09-27 | **None exists.** Seven open feature requests (github.com/anthropics/claude-code issues #25642, #47018, #44607, #13733, #20132, #32500, #96959, checked 2026-09-27) ask Anthropic to add a `CLAUDE_SESSION_ID`-style var; none is implemented. Whether an unlisted var otherwise passes through (full vs filtered inherit) is undocumented on code.claude.com/docs/en/mcp — **unknown**. | `session_id` in every hook's JSON stdin (code.claude.com/docs/en/hooks); a sub-agent's hook carries the **parent's** `session_id` plus its own `agentId` (research.md §17.2, observed, not documented) | Undocumented on code.claude.com/docs/en/mcp — **unknown**. (The MCP spec's 2026-07-28 revision, blog.modelcontextprotocol.io/posts/2026-07-28/, moved session context out of `initialize` into a per-request `_meta` scheme; whether Claude Code has adopted that revision is **unverified**.) | Inferred yes — "Stdio servers are local processes; Claude Code doesn't reconnect them automatically" (code.claude.com/docs/en/mcp) implies session-scoped, but no explicit "one process per session" statement — **unverified/inferred** | Depends on how the sub-agent references the server: code.claude.com/docs/en/sub-agents (2026-09-27) says an inline `mcpServers` block in a subagent's own frontmatter "connect[s] when the subagent starts... and disconnect[s] when it finishes" (its own process); a plain string reference to a configured server "share[s] the parent session's connection" (same process). rtok is referenced by name, so subagents **share** the parent's `rtok mcp` process. | No session-id env var exists and full-inherit is unconfirmed, so (a) is out. Use (b): ppid-chain from the MCP process to a common ancestor shared with the session's hook processes, cross-checked against the hooks' `session_id`. (c) cwd+host+start-time only as a fallback, ambiguous if two sessions share a cwd. | pending |
| Claude Code desktop | Observed by the creator 2026-09-26 (T271, plan.md); no primary Anthropic doc found this session for `claude_desktop_config.json`'s `env` field — only secondary blogs (dev.to/danishashko, prompt-architects.com), checked 2026-09-27, **unverified** | **Unknown** — no evidence found either way | **Unverified** whether the Desktop "Code" tab's hook `session_id` is the same value space as the CLI's, since it runs the same Claude Code engine under a separate MCP config file (research.md §25) | **Unknown**, no source found | **Unverified** — secondary sources describe the desktop app owning the MCP process lifecycle for as long as the app runs, suggesting one long-lived process across conversations rather than per-conversation; no primary doc confirms | **Unknown/not applicable** — no documented sub-agent-spawning feature distinct from the CLI found for Desktop specifically | None of (a)/(b)/(c) is confirmed reliable. Best available is (c) host+start-time identifying the *app instance*, not a specific conversation — an open gap, not a solved case. | pending |
| Cursor | cursor-agent 2026.09.23 (research.md §25); cursor.com/docs/context/mcp, /docs/mcp, /docs/hooks, /docs/cloud-agent, 2026-09-27 | `env`/`envFile` config fields exist for stdio servers (cursor.com/docs/context/mcp, /docs/mcp) but no documented auto-injected session-id var; the only session-shaped var found, `CURSOR_SESSION_TOKEN`, is Background-Agent-API auth, not a conversation id — **unknown/not found** | `session_id` (= `conversation_id`) plus `generation_id` in every hook (research.md §26, cursor.com/docs/hooks) | **Unknown**, not in fetched docs | **Unverified** — not explicitly documented; secondary forum reports (forum.cursor.com, **unverified**) only note processes restart with the editor | No for Cloud/Background Agents: each runs in "isolated VMs" with "its own Git worktree" and its own MCP server config (cursor.com/docs/cloud-agent, 2026-09-27) — a reasonable inference of a separate MCP process, though not phrased that explicitly | No confirmed session-id env var and process-per-conversation is unconfirmed, so use (b): ppid-chain correlated with the hook-carried `session_id`/`generation_id`. (c) cwd+host+start-time as fallback, ambiguous across multiple Cursor windows on one repo. | pending |
| Codex CLI | openai/codex @ `71b38795` (main, 2026-09-27, github.com/openai/codex); learn.chatgpt.com/docs (hooks, MCP), /environments/git-worktrees | **None — confirmed absent, not just undocumented.** `create_env_for_mcp_server` builds the stdio child's env from an explicit allowlist, `DEFAULT_ENV_VARS` (Unix: `HOME, LOGNAME, PATH, SHELL, USER, __CF_USER_TEXT_ENCODING, LANG, LC_ALL, TERM, TMPDIR, TZ`), plus per-server configured `env_vars` — no `std::env::vars()` passthrough. Verified directly against github.com/openai/codex/blob/71b38795/codex-rs/rmcp-client/src/utils.rs (`DEFAULT_ENV_VARS` and `create_env_for_mcp_server`), 2026-09-27. No session-id var is in that list and none is added programmatically. | `session_id` common field; a turn adds `turn_id` (research.md §26) | clientInfo `{name: "codex-mcp-client", version: env!("CARGO_PKG_VERSION")}` (same file, `utils.rs`, verified). No `_meta` field found in the initialize params built in `rmcp_client.rs` — **unverified** whether one exists elsewhere in the crate. | Each `codex` CLI invocation (one process per conversation) owns its own MCP client set; the exact manager type post-refactor is **unverified** (the old `mcp_connection_manager.rs` path no longer resolves; the logic now lives in `codex-rs/rmcp-client`) | No native sub-agent/orchestrator code found in `codex-rs` (`subagent`/`sub_agent` — zero hits, 2026-09-27). Third-party wrappers shelling out `codex exec` spawn a wholly separate process with its own MCP connections (issue #20135, secondary, **unverified**) — not a first-party sub-agent feature. | The env allowlist rules out (a) entirely. Use (b): ppid-chain — rtok's `rtok mcp` child is a direct child of the `codex` process. (c) cwd+host+start-time as fallback. | pending |
| Copilot CLI | 1.0.83 (research.md §25); docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-mcp-servers, /reference/hooks-reference, 2026-09-27; `github/copilot-cli` repo confirmed closed source (its tree holds only README/install.sh/.github, `gh api repos/github/copilot-cli`, 2026-09-27) | **Unknown** — docs state "The `PATH` variable is automatically inherited from your environment. All other environment variables must be configured here" (docs.github.com, above), i.e. env is filtered/explicit rather than fully inherited; no session-id var is documented in any case. | `sessionId` (camelCase) in `sessionStart`/`sessionEnd` payload (research.md §26) | **Unknown** — closed source, not documented | **Unknown**, not documented | Custom/task-tool sub-agents do **not** receive MCP tool connections at all per github/copilot-cli#2630 (secondary issue-tracker lead, **unverified** whether that reflects a deliberate separate-process design or a wiring bug) | (a) ruled out by the filtered-env docs; no source to confirm a ppid relationship in a closed-source binary beyond what a live probe's `ps` can observe. Best available without the probe is (c) cwd+host+start-time. | pending |
| Grok CLI (Grok Build, xAI) | xai-org/grok-build @ `f0e3be11` (public, Apache-2.0, confirmed via `gh api repos/xai-org/grok-build`, 2026-09-27); docs.x.ai/build/features/hooks | **`GROK_SESSION_ID` — confirmed injected directly into the MCP child's env**, not just hooks: `apply_stdio_env(cmd, env, session_id)` calls `cmd.env("GROK_SESSION_ID", session_id)` on the `Command` built for every stdio MCP spawn, and no `env_clear()` call exists anywhere in the file, so the child also inherits the full parent environment by Rust's `Command` default. Verified directly against github.com/xai-org/grok-build/blob/f0e3be11/crates/codegen/xai-grok-mcp/src/servers.rs (`apply_stdio_env`, `start_mcp_server`), 2026-09-27. | `sessionId` in payload; env vars `GROK_HOOK_EVENT`, `GROK_HOOK_NAME`, `GROK_SESSION_ID`, `GROK_WORKSPACE_ROOT` (docs.x.ai/build/features/hooks; research.md §26) | clientInfo `{name: "grok-shell-{server_name}", version: xai_grok_version::VERSION}`, protocol pinned to `rmcp::model::ProtocolVersion::V_2025_11_25` (`servers.rs`, verified). A `_meta["x.ai/mcp/servers"]` field exists but is used for SDK-declared in-process servers, not a session carrier in `initialize` — **unverified** for that specific use. | Spawn context is session-scoped (`McpSpawnCtx::for_session(session_id, ...)`), consistent with one process set per session — **inferred**, no explicit "one process per session" comment/doc found. | **Unverified/inferred** — no code or doc directly confirms whether each `--parallel` worktree sub-agent gets its own MCP spawn; the session-scoped `McpSpawnCtx` suggests each (separate `session_id`) would. | (a): `GROK_SESSION_ID`, first-party code-confirmed on the MCP child itself. The strongest, most direct link rule found across every host researched. | pending |
| Gemini CLI | google-gemini/gemini-cli @ `2fe7c2d3` (public, github.com/google-gemini/gemini-cli); geminicli.com/docs/hooks/reference, /docs/core/subagents, 2026-09-27 | **None — confirmed absent by code, not just undocumented.** `GEMINI_SESSION_ID` is built only into the local `env` object of each hook's own `spawn()` call in `hookRunner.ts` (`{...sanitizeEnvironment(process.env, ...), GEMINI_SESSION_ID: input.session_id, ...}`); it never mutates the gemini-cli process's own `process.env`. The separate MCP spawn path in `mcp-client.ts` builds its child env from `{...process.env, ...extensionEnv}` run through `sanitizeEnvironment` (a redact-known-secrets pass, not an allowlist) plus the server's own configured `env` map — so an MCP child sees `GEMINI_SESSION_ID` only if a user explicitly put it in that server's config. Verified directly against github.com/google-gemini/gemini-cli/blob/2fe7c2d3/packages/core/src/hooks/hookRunner.ts and .../packages/core/src/tools/mcp-client.ts, 2026-09-27. | `session_id` in stdin JSON (geminicli.com/docs/hooks/reference; research.md §26) | clientInfo `{name: "gemini-cli-mcp-client", version: clientVersion}` (`mcp-client.ts`, verified). No `_meta` session field found in the SDK-built `initialize` call. | Yes by construction — gemini-cli is a one-shot process per invocation, not a persistent daemon, so every MCP server it spawns lives for that process's lifetime. | **Documented**: a subagent's frontmatter `tools` field, if it omits its own `mcpServers`, "inherits all tools from the parent session" (geminicli.com/docs/core/subagents, github.com/google-gemini/gemini-cli/blob/main/docs/core/subagents.md, verified) — i.e. it shares the parent's already-connected MCP processes. A subagent that declares its own inline `mcpServers` gets isolated servers of its own. | No session-id env var reaches the MCP child, ruling out (a). Use (b): ppid-chain — the MCP child is a direct child of the `gemini` process — correlated with the hook-carried `session_id`. | pending |
| Kimi CLI | MoonshotAI/kimi-code @ `be7d5f5f` (the current open-source TS monorepo, not the archived Python `kimi-cli`, github.com/MoonshotAI/kimi-code); moonshotai.github.io/kimi-code/en/customization/hooks.html, /mcp.html, 2026-09-27 | **None exists to inherit**, but not because of filtering: `mergeStdioEnv(configEnv, parentEnv = process.env)` copies the **full** `process.env` (no denylist), then overlays the server's configured `env`. Verified against github.com/MoonshotAI/kimi-code/blob/be7d5f5f/packages/agent-core-v2/src/mcpCore/client-stdio.ts, 2026-09-27. There is simply no session-id var anywhere in the CLI to be inherited: hooks receive `session_id` only in the stdin JSON payload (moonshotai.github.io/kimi-code/en/customization/hooks.html), and the one `KIMI_SESSION_ID`-shaped string in the source is a skill-command template placeholder (`registry.ts`) unrelated to process env or MCP spawn. | `session_id` in every hook's stdin, plus `session_title`, `client_type`, `cwd` (moonshotai.github.io, research.md §26) | clientInfo `{name: KIMI_MCP_CLIENT_NAME ("kimi-code"), version: KIMI_MCP_CLIENT_VERSION}` (`client-shared.ts`, verified; the version constant's resolved value wasn't traced further — **unverified** exact string). No `_meta` found. | One `Client` connection per configured server per process (`connection-manager.ts` keeps a map) — consistent with one process per session, no explicit doc statement either way | **Unknown** — not documented in the fetched hooks/MCP pages, no code path found either way | Full env inheritance carries nothing session-specific, and no session-id var exists at all, ruling out (a). Use (b): ppid-chain to the `kimi` process. (c) cwd+host+start-time as fallback. | pending |
| ZCode | zai-org/ZCode @ `29628c9a` (open-sourced 2026-09-21, github.com/zai-org/ZCode); zcode.z.ai/en/docs/mcp-services, /docs/hooks, 2026-09-27 | **Confirmed unavailable to MCP children, by two independent paths.** Env: `buildMcpStdioEnv` runs `sanitizeZCodeRuntimeEnv(filterStringEnv(process.env))` — a denylist removing ZCode's own broker/telemetry keys and proxy-var patterns — then spreads the server's configured `env` in afterward; verified against github.com/zai-org/ZCode/blob/29628c9a/apps/zcode-cli/packages/adapters/src/mcp/network.ts and packages/shared/src/runtimeEnv.ts. `ZCODE_SESSION_ID` is never in `process.env` to begin with — it exists only inside the per-hook input object and the custom-command template-expansion context. Confirming the gap the other way: a **plugin** MCP config that references `${ZCODE_SESSION_ID}` in its own `env` throws `PluginVariableError("Plugin variable requires a runtime session context")` at resolve time — github.com/zai-org/ZCode/blob/29628c9a/apps/zcode-cli/packages/adapters/src/plugins/mcp.ts, verified. | `"session_id"` in every hook's stdin JSON (zcode.z.ai/en/docs/hooks; research.md §26) | clientInfo `{name: this.clientName ?? "zcode", version: this.clientVersion ?? "0.0.0"}` (`apps/zcode-cli/packages/adapters/src/mcp/index.ts`, verified). A separate `_meta` identity header exists only for the official stdio plugin's own auth, per message — not a session tracker. | One process per server per client-manager instance (keyed map); docs say plugin MCP servers are "managed by the corresponding plugin," suggesting plugin-scoped processes — **unverified** for the general (non-plugin) case | **Unknown** — no doc or code found either way | No session-id var reaches an MCP child by any path (env inherit or template), ruling out (a). Use (b): ppid-chain to the `zcode` process. (c) cwd+host+start-time as fallback. | pending |
| CodeWhale | Hmbown/CodeWhale @ `94130d91` (public, Rust, github.com/Hmbown/CodeWhale); github.com/Hmbown/Codewhale/blob/main/docs/HOOKS.md, docs/MCP.md, 2026-09-27 | **None — confirmed absent, though the child gets everything else.** The stdio spawn is `Command::new(&config.command).args(&config.args).envs(&config.env)` with no `env_clear()` anywhere in the file, so the child inherits the **full** host environment plus the server's configured overlay. Verified against github.com/Hmbown/CodeWhale/blob/94130d91/crates/mcp/src/stdio_client.rs, 2026-09-27. `DEEPSEEK_SESSION_ID` is set only inside the hook-subprocess's own env map in `crates/tui/src/hooks/executor.rs` (`env.insert("DEEPSEEK_SESSION_ID", ...)`, spawned via a separate `.envs(&env)` call) and is never written to the codewhale-tui process's own environment (no `std::env::set_var` found) — so it lives in a map that never reaches `stdio_client.rs`'s full-inherit spawn. | `DEEPSEEK_SESSION_ID` in every event except `shell_env` (docs/HOOKS.md; research.md §26) — a hook-payload/env fact, not an MCP one | clientInfo `{"name": "codewhale-mcp-server", "version": env!("CARGO_PKG_VERSION")}` (`stdio_client.rs`, verified). No `_meta`. | **Documented, yes**: "Each MCP client session spawns its own server process" (docs/MCP.md, github.com/Hmbown/CodeWhale/blob/main/docs/MCP.md) | **Unknown/undocumented** — no doc or code found either way | Full inherit carries no session id (confirmed above), ruling out (a). Use (b): ppid-chain — the MCP child's parent is the codewhale-tui process, the same ancestor any hook child shares. (c) cwd+host+start-time as fallback. | pending |

Cross-host pattern: of the ten hosts probed, only **Grok Build** confirms a session-id env var reaching the MCP child by first-party code (`GROK_SESSION_ID`, injected directly at MCP spawn, independent of any hooks path). Every other host that has an equivalent var (Claude Code: none found then — superseded by `CLAUDE_CODE_SESSION_ID`, see "Claude desktop sessions and `CLAUDE_CODE_SESSION_ID` (T473)" below; Gemini's `GEMINI_SESSION_ID`; ZCode's `ZCODE_SESSION_ID`; CodeWhale's `DEEPSEEK_SESSION_ID`) scopes it to the hook subprocess's own env map and never mutates the host process's own environment, so an MCP child spawned from the same host process never inherits it. Codex and Copilot CLI filter the child's env to an explicit allowlist regardless. Kimi has no session-id var of any kind. The one **documented** default-sharing behaviour for sub-agents is Claude Code (string-referenced servers share the parent's connection) and Gemini CLI (a subagent without its own `mcpServers` inherits the parent's); every other host's sub-agent/MCP-process relationship is unknown or unverified. Absent a working env-var rule, T283's default per host should be (b) — correlate the MCP process's ppid chain with the hook processes' `session_id` via a common host-process ancestor — falling back to (c) cwd+host+start-time, explicitly marked ambiguous when a cwd hosts more than one live session; Grok Build alone can use (a) directly.

**What T283.1 ships, and how sure it is (2026-10-03).** `src/agents/link.rs` implements the rule order above as **doc-derived**: it follows the vendor docs and spawn code in this table, not a live run. The T281 probe column stays `pending`, and when the creator's logs arrive they only confirm the rule per host or change it for a host where they disagree. Shipped: (a) `GROK_SESSION_ID` for `grok` (the one host with a confirmed env var) and (c) the host's live agents in the MCP process's cwd, where one match links and two or more are ambiguous and bind nothing. (b), the nearest common host ancestor pid, shipped later as T283.3 (2026-10-03, see the next section): the hook wire request carries the client's pid and the resident walks that pid's ancestors, because the resident hook process is not the host's child. A host with no hook support (`Agent::support(_, "hooks")` is `No`) registers its own agent row from the MCP process. A host that the `hosts` table does not know yet registers under `other`, so its cwd candidates include every such host.

### Hook client ↔ host ancestry (T283.3)

Question: is the process that runs an rtok hook a descendant of the host process the host's `rtok mcp` child also descends from, so the nearest common ancestor pid links the two? Checked 2026-10-03. Primary sources only; commits pinned.

| Host | Source (version / commit) | How the hook command is started | Verdict |
|---|---|---|---|
| Codex CLI | openai/codex @ `12a30d4e`, `codex-rs/hooks/src/engine/command_runner.rs` `build_command` | `Command::new(shell)` + `-lc <command>` (or the configured shell args), `ProcessMode::NewSession` on Unix: a shell that is the codex process's child; the hook is the shell's child, or the shell itself if it execs | **Verified**: host → shell → hook |
| Gemini CLI | google-gemini/gemini-cli @ `fb972b2f`, `packages/core/src/hooks/hookRunner.ts` | `spawn(shellConfig.executable, [...argsPrefix, command], {shell: false, cwd})` | **Verified**: host → shell → hook |
| CodeWhale | Hmbown/CodeWhale @ `60ea7c22`, `crates/tui/src/hooks/executor.rs` `build_shell_command` | `Command::new("sh").arg("-c").arg(command)` with `process_group(0)` (`cmd` on Windows) | **Verified**: host → sh → hook |
| Kimi CLI | MoonshotAI/kimi-code @ `21406fb4`, `packages/agent-core-v2/src/features/externalHooks/internal/runHook.ts` and `src/os/backends/node-local/hostProcessService.ts` | `hostProcess.spawn(command, [], {shell: true})`; the node-local backend wraps `node:child_process` `spawn` | **Verified** for the local backend only; other backends not read |
| Cline | cline/cline @ `476b165b`, `apps/vscode/src/core/hooks/HookProcess.ts` | `child_process.spawn`; "Unix executes hook files through the shell for shebang support" (source comment) | **Verified**: extension host → shell → hook link (source). Docs checked 2026-10-08: https://docs.cline.bot/customization/hooks names no parent process or session id. Link rule: Cline has hooks and no session env var, so rule 2 (ancestor pid, `src/agents/link.rs`) applies, with the cwd rule as fallback |
| Claude Code | code.claude.com/docs/en/hooks.md, 2026-10-03 | docs: a command hook "run[s] a shell command"; with `args` the command "is spawned directly ... with no shell involved" | **Unverified** for the parent: closed source, the docs do not name the parent process; probe pending |
| ZCode | zai-org/ZCode @ `29628c9a` | spawn site not located by code search | **Unverified**; probe pending |
| Cursor, Copilot CLI, Devin, Command Code | closed source; docs describe command hooks only. Re-checked 2026-10-08: Devin's hook docs (https://docs.devin.ai/desktop/cascade/hooks) run commands through `bash -c` or `powershell -Command` and name no parent; Command Code's docs (https://commandcode.ai/docs/hooks) say "Each hook fires with its own process" and list `PreToolUse`, `PostToolUse`, `Stop`, `SessionStart`, no `SubagentStart`, no ancestry field | not documented | **Unverified**; probe pending |

Consequence for the rule: in every verified case a shell sits between the host and the hook command, so the hook is the host's grandchild unless the shell execs it. The rule therefore records the hook client's first three ancestors, not its parent, and matches any of them against the `rtok mcp` process's own first three. The cap keeps a shared terminal, `tmux` server or `launchd` out of the match.

Reading a parent pid chain without a process spawn (the hook client's budget is 10 ms, so no `ps`):

| Platform | Call | Source (checked 2026-10-03) |
|---|---|---|
| Linux | read `/proc/<pid>/stat`, field 4 (`ppid`), parsed after the last `)` because the `comm` field may hold spaces and parentheses | man7.org/linux/man-pages/man5/proc.5.html |
| macOS | `proc_pidinfo(pid, PROC_PIDTBSDINFO, ...)` returns `proc_bsdinfo.pbi_ppid`; `libc` 0.2.189 declares `proc_pidinfo`, `PROC_PIDTBSDINFO` and `proc_bsdinfo` (`src/unix/bsd/apple/mod.rs`) | docs.rs/libc/0.2.189 and the crate source; `libc` is already in `Cargo.lock` and in the parent `rust.md` |
| Windows | `CreateToolhelp32Snapshot` + `PROCESSENTRY32.th32ParentProcessID` exists, but a Windows parent pid is never updated after the parent exits (the `rtok-sys::parent_pid` comment), so a chain says nothing reliable: not implemented, the rule is skipped | learn.microsoft.com/windows/win32/api/tlhelp32/ns-tlhelp32-processentry32 |

No crate in `rust.md` reads another process's parent pid: `rustix::process::getppid` is the caller's own, `sysinfo` is the heavy CPU/RAM/disks crate, and `libproc` is not listed. `rtok-sys` already holds the unsafe process shims (`parent_pid`, `process_alive`), so the macOS call goes there.

### Claude desktop sessions and `CLAUDE_CODE_SESSION_ID` (T473)

Question: why do Claude Code desktop-app (Code tab) sessions so often lack `RTOK_AGENT_ID`, and is there a session id that reaches the agent's shell and the MCP child without `SessionStart`? Checked 2026-10-08 against Claude Code 2.1.293 (desktop app, `CLAUDE_CODE_ENTRYPOINT=claude-desktop`).

- **Documented: `CLAUDE_CODE_SESSION_ID`.** Claude Code sets it to the current session id in Bash and PowerShell tool subprocesses, hook commands and stdio MCP server subprocesses. For Bash and hooks it matches the hook input's `session_id` and follows `/clear`; an MCP server subprocess keeps the id it was spawned with, and on `--continue` or `--resume` without an id it may get the startup id instead. Source: https://code.claude.com/docs/en/env-vars (row `CLAUDE_CODE_SESSION_ID`, checked 2026-10-08).
- **Documented: `CLAUDE_ENV_FILE`** is given to `SessionStart`, `Setup`, `CwdChanged` and `FileChanged` hooks only, and its contents run before each Bash command. Sources: https://code.claude.com/docs/en/hooks ("Persist environment variables", checked 2026-10-08) and the `CLAUDE_ENV_FILE` row of https://code.claude.com/docs/en/env-vars (checked 2026-10-08). Neither page says anything about the desktop app or SDK mode for this variable.
- **Measured (the creator's machine, 2026-10-08):** in this session's own Bash tool `CLAUDE_CODE_SESSION_ID` equals the `host_session_id` of its rtok agent row, `CLAUDE_ENV_FILE` is unset and `~/.claude/session-env/<session>/` is an empty directory. The store has PreToolUse/PostToolUse calls for the session but no `SessionStart` (and no `UserPromptSubmit` for the first prompt). Across 28 recent desktop sessions, the env file (`sessionstart-hook-1.sh`, rtok's plugin hook) exists exactly in the 17 where the store has a `SessionStart` call; in 16 of 19 sessions checked, the first `SessionStart` came hours after the session's first hook (a resume or compact), not at startup. So the env-file mechanism works; the plugin's startup `SessionStart` is what does not reach rtok in the desktop app, while the user-settings `SessionStart` hook at index 0 does run (it creates the empty directory). Why the plugin hook is skipped at startup is not documented; third-party reports of plugin `SessionStart` hooks missing the first run exist (anthropics/claude-code issues 10997, 16763, via claudeissues.com mirrors) — **unverified**.
- **What T473 ships:** `CLAUDE_CODE_SESSION_ID` joins `GROK_SESSION_ID` in `src/agents/link.rs` `SESSION_ENV`. From a shell (`rtok agents whoami/status/send/inbox`, `rtok worktree …`) it names the main agent row of that host session when `RTOK_AGENT_ID` is unset. In `rtok mcp` it links only a row the hooks already wrote and never registers one, because the MCP child's copy can be stale (`/clear`, `--continue`); with no row yet, the ancestor and cwd rules run as before. Measured 2026-10-09 (`ps eww`): each session's own `rtok mcp` child of `claude` carries that session's id, while the app-level servers the desktop app starts from `claude_desktop_config.json` (T455's `Link::Outside` case) carry none, so the env rule never links one server to many sessions.

### Post-create hook file shapes (T289.3, 2026-10-10)

Question: which file does each host read after it creates a worktree, and what exactly is in it, so `rtok agents install <host> --project` can add one entry and leave the rest of the file alone. All three are vendor docs, fetched 2026-10-10.

- **Cursor.** https://cursor.com/docs/configuration/worktrees. The file is `.cursor/worktrees.json`, read "in the worktree path" first, then "in the root path of your project" (the page says nothing about a user-level copy, so project only). Keys, all optional: `setup-worktree-unix` (macOS/Linux, "takes precedence over `setup-worktree` on Unix"), `setup-worktree-windows` (same on Windows) and `setup-worktree` (any OS). A value is either an array of shell command strings, "executed sequentially in the worktree", or a string path to a script relative to `.cursor/worktrees.json`. Example from the page: `{"setup-worktree": ["npm ci", "cp $ROOT_WORKTREE_PATH/.env .env"]}`. `ROOT_WORKTREE_PATH` points at the main checkout. Not documented: what happens when a command fails, and which shell runs the commands. Consequence for the writer: our command goes into every key whose value is an array (so an OS-specific key cannot shadow it); a key whose value is a script path is never edited, because that would change a file that is not ours.
- **Devin Desktop and Windsurf.** https://docs.devin.ai/desktop/cascade/hooks (https://docs.windsurf.com/windsurf/cascade/hooks answers 307 to it) and https://docs.devin.ai/desktop/cascade/worktrees. Config files: user `~/.codeium/windsurf/hooks.json` (JetBrains: `~/.codeium/hooks.json`), workspace `.devin/hooks.json` in the workspace root, and a system file; "the legacy `.windsurf/hooks.json` is used only when `.devin/hooks.json` is absent or defines no hooks". The worktrees page still says `.windsurf/hooks.json`, so the hooks page is the newer. Shape: `{"hooks": {"post_setup_worktree": [{"command": "bash $ROOT_WORKSPACE_PATH/hooks/setup_worktree.sh", "show_output": true}]}}`. The hook runs inside the new worktree after it is created and configured, gets `$ROOT_WORKSPACE_PATH` (the original workspace) and a JSON payload on stdin (`tool_info.worktree_path`, `tool_info.root_workspace_path`); commands run through `bash -c` or `powershell -Command`. Exit code 2 can block only pre-hooks, any other code lets the action proceed. Cap: 20 worktrees per workspace, oldest last-accessed removed first. Consequence: creating `.devin/hooks.json` next to an existing `.windsurf/hooks.json` would switch the project to the new file and silently disable the legacy hooks, so the writer picks the file the host reads today (`.devin/hooks.json` when it defines hooks, else a `.windsurf/hooks.json` that does, else `.devin/hooks.json`).
- **Kilo.** https://kilo.ai/docs/automate/agent-manager and https://kilo.ai/docs/automate/agent-manager-workflows. The setup script is `.kilo/setup-script` or `.kilo/setup-script.sh` (macOS/Linux, in that order) and `.kilo/setup-script.ps1`, `.cmd`, `.bat` (Windows, in that order), "in your project". It runs "whenever a new worktree is created", after the root-level `.env` files are copied and before the agent starts, with `WORKTREE_PATH` (the new worktree) and `REPO_PATH` (the repository root) in the environment; `sh` runs a POSIX script, no executable bit needed; five-minute timeout; on failure the worktree is kept for inspection (so a failing setup line can hold the session up, and our line must fail open). Managed worktrees live in `.kilo/worktrees/`. The file is a plain script, so our entry is a marked block, and an existing `.kilo/setup-script.sh` is edited instead of creating `.kilo/setup-script`, which would shadow it. Windows variants are not written (no unix shell to run the block).
- **Not confirmed** (so not claimed): whether any of the three evicts or breaks on a git-locked worktree (T281's live probe); what Cursor does with a failing command.

## 27. Memory defaults and one install point (T291–T294) (2026-09-27)

Creator 2026-09-27: the write hook and the session-end hook must fire, smart memory must be on by default, and every agent must get that from one module (or one `create`), not from a hand-written manifest per host. Decision D35. Cloud sync of notes is a later task in a private repo; that repo was not found in the local tree or via `gh repo list` / `gh search` on this date (the only cloud-sync hit was Fern in `listepo/budget-app`, a different product). D8 stands: this repo keeps one SQLite file and does not grow a sync protocol. T294 only widens the JSONL row so a later replicator can carry a tombstone.

### 27.1 Why memory looks off

Runtime is already one path. `inject_event` dispatches `SessionStart`, `UserPromptSubmit` and `SubagentStart`; `hooks::dispatch` writes the session note on `SessionEnd`. Install is not one path: `claude::ENTRIES` plus a separate list in Cursor, Copilot, Gemini and ZCode, and a hand-written `plugins/*/hooks/hooks.json` for each.

Cursor's plugin manifest (`plugins/cursor/hooks/hooks.json`, tree read 2026-09-27) registers `sessionStart`, `preCompact`, shell and MCP. It does not register `beforeSubmitPrompt`, `sessionEnd` or `subagentStart`. `cursor_event` maps `beforeSubmitPrompt` → `UserPromptSubmit` and leaves `sessionEnd` and `subagentStart` unmapped (`src/hooks/types.rs`). Cursor documents all three (https://cursor.com/docs/hooks, fetched 2026-09-27). The same page says cloud agents defer `sessionStart` and have no IDE-lifetime `sessionEnd`; those two stay unwired for cloud agents, recorded on the module, with no second memory implementation.

Defaults that leave the smart paths silent, both in `config/default.toml` and `Memory` (`src/config/mod.rs`): `prompt_recall = 0`, `startup_recall = false`, `handoff = false`, `spawn_brief = false`. Embeddings stay off. Recall itself is on (`recall_tokens = 200`, five titles, no bodies).

On this machine, `rtok memory status` (2026-09-27, rtok 0.10.0): 125 live notes, 235 recalls, `mem_get` 0, `mem_search` 3. Titles are injected and the bodies are never fetched.

### 27.2 What the five products actually do

Fetched 2026-09-27. Vendor diagrams are their own illustrations. Letta, Zep and Graphiti were absent from `research.md` before this section. Mem0 and claude-mem were already in §4 and §9 (scan 2026-09-17); this section adds only the mechanism that bears on D35.

| Product | What reduces tokens | What spends tokens | Store / sync shape | Source |
| --- | --- | --- | --- | --- |
| Mem0 | The app calls `search` and chooses which memories enter the prompt, instead of replaying the transcript. | Default `add` runs an LLM extraction, then embeds. SQL holds facts; a vector store and an entity store sit beside it. `infer=False` stores the raw text. | Platform manages the three stores. OSS leaves the backing stores to config. | https://docs.mem0.ai/core-concepts/how-it-works |
| Letta MemFS | Files under `system/` are in the system prompt every turn. Every other path is a name in the tree; the body is read with ordinary file tools when needed. No vector index by default. | The `system/` tree is always-on. Dreaming and memory-doctor subagents edit memory in git worktrees. Semantic search is an optional mod (QMD). | One git repo per agent. Cloud agents push to a hosted repo. Local agents commit on the machine. | https://docs.letta.com/concepts/memfs/ |
| Zep | `scope="auto"` returns one context block packed to `max_characters` (default 2500, cap 50000). Search `limit` defaults to 10. | The block is built to be pasted into the prompt. The graph behind it is Graphiti. | Managed context graph. | https://help.getzep.com/searching-the-graph |
| Graphiti | Episodes stay the raw ingested stream; derived facts carry temporal validity windows and point back at episodes. Retrieval is hybrid (semantic, keyword, graph). | Ingest uses an LLM. Backing store is Neo4j, FalkorDB, Neptune or Kuzu. `SEMAPHORE_LIMIT` defaults to 10 concurrent ingest operations; that is a concurrency cap, not a retrieval cap. The README on `main` does not state a current release version (it mentions v0.17.0 for the database-driver API). A search-limit constant of 10 at tag `v0.30.2` was reported from `graphiti_core/search/search_config.py` and is **unverified** (not in that README body). | Open-source framework under Zep. A context graph is a second store. | https://github.com/getzep/graphiti (README on `main`) |
| claude-mem | SessionStart shows an index: id, time, type, title, approximate token cost, and the name of the MCP tool that fetches the body (`search`, then `timeline`, then `get_observations`). | Observations are extracted and compressed. The index is always injected. The page's 35,000 / 800 / 920 token figures are illustrations, not a measured bill. | SQLite plus the tool-use table. Cloud sync was already noted in §9.1 (v13.25.1, 2026-09-17); this page does not restate a version. | https://docs.claude-mem.ai/progressive-disclosure |

What survives D4, D8 and D13: an index of titles plus a fetch (`mem_get`), a character/token cap on anything that is injected, and verbatim episodes (rtok's note body). What does not: LLM extraction on the hook, a vector or graph store beside SQLite, lossy observation compression, and a git repo as the source of truth.

### 27.3 What follows

T291 is the one module. T292 turns the four flags on inside the existing budgets (200 / 400 / 300 / 800). T293 puts a token count and the `mem_get` name on each title line, and records a measurement when a body is fetched. T294 adds tombstone fields to the JSONL row. T131 still measures the spawn-brief net; D35 says that net does not choose the default.

## 28. Projects, task claiming and cross-agent session handoff (2026-09-30)

Creator request 2026-09-30: research how rtok could give agents *projects* that stay in sync across different agents, make tasks easy to claim and track, reuse what the agents already ship (as a plugin where useful), and let one session start on one agent and finish on another. Survey the alternatives and take the union of their features without duplication.

Method: rtok code read at `d25d5ed0`; host docs and third-party repos read 2026-09-30 (repo facts from `gh api repos/<r>` and `/releases/latest` on that date). Secondary claims are marked **unverified**.

### 28.1 What rtok already has

| Piece | Where | Reuse for |
| --- | --- | --- |
| Agent id per host session (UUIDv4, parent for sub-agents), `agents` table with activity and `status_text` | D34, `src/store/schema.rs` (`agents`), T281–T284 | claim owner, liveness (lease) |
| Worktree ownership by git lock reason `<owner> \| <task-id> \| <date> \| agent <uuid>`; `worktree_claims` table; `worktree claim/adopt` | D34, T285–T289 | one task = one worktree = one owner |
| Agent-to-agent messages, framed as information (`agents send/inbox`, MCP `agent_send/agent_inbox`) | T287–T288 | messages keyed by task id |
| `handoff` MCP tool: budgeted digest of paths, symbol ranges and `expand` ids | `src/plugins/memory/handoff.rs` (T59.6) | handoff body |
| SessionEnd note `session:<id>`; SessionStart `startup` recall injects the newest one for the project on any host with a SessionStart hook | `src/plugins/checkpoint.rs`, `inject/mod.rs` (T71.2) | cross-host handoff on one machine already works, implicitly |
| Transcript parsers: Claude Code (full), Codex (token counts only) | `src/measure/jsonl.rs`, `src/measure/codex.rs` | reading the source session |
| `memory export/import` (JSONL) | T66.2 | the only cross-machine path today |
| Claim protocol for this repo's own tasks: `plan.md` table, Status + Agent columns, `todo.md` mirror, `done.md` | `plan.md`, workspace rulebook | the task format itself |

Constraint: D8 keeps one SQLite file per machine and no sync protocol; §27.2 rejects "a git repo as the source of truth" for memory. Tasks are different: the creator's `plan.md` already lives in git and already is the source of truth.

### 28.2 What the agents ship (do not duplicate)

| Need | Native answer | Source |
| --- | --- | --- |
| Shared task list across sessions | Only Claude Code: `CLAUDE_CODE_TASK_LIST_ID=<name>` → `~/.claude/tasks/<name>/`; agent teams claim with file locks, one team per session, experimental | https://code.claude.com/docs/en/interactive-mode#task-list, https://code.claude.com/docs/en/agent-teams |
| In-session todo | Codex `update_plan` (off by default since 0.152.0, **unverified**: seen quoted in https://github.com/openai/codex/issues/42365), Gemini `write_todos` (session-scoped), OpenCode `todowrite`, Copilot CLI todos, Roo `update_todo_list` | https://raw.githubusercontent.com/google-gemini/gemini-cli/main/docs/tools/todos.md, https://opencode.ai/docs/tools/, https://roocodeinc.github.io/Roo-Code/features/task-todo-list |
| Cross-agent board | Cline Kanban (research preview): worktree per card, dependency chains, runs Cline, Claude Code, Codex, OpenCode | https://docs.cline.bot/usage/kanban.md |
| Same-vendor session move | Claude `--teleport` / `--cloud`; Codex `cloud` + `apply`; Copilot `/delegate`; Cursor `&` to a Cloud Agent (return path **unverified**) | https://code.claude.com/docs/en/cli-reference, https://learn.chatgpt.com/docs/developer-commands?surface=cli, https://docs.github.com/en/copilot/reference/cli-command-reference, https://cursor.com/docs/cli/using |
| Resume / fork in place | Claude `-c/-r/--fork-session`; `codex resume/fork`; Gemini `--resume`; OpenCode `-c/--session/--fork`, `export`/`import`; Copilot `--resume` | same pages; https://geminicli.com/docs/cli/session-management/, https://opencode.ai/docs/cli/ |
| Thread handoff | Amp `/handoff` (2025-10-23), removed 2026-05-06 in favour of compaction and thread references | https://ampcode.com/news/handoff, https://ampcode.com/news/neo |
| Cross-vendor session move | **None native.** Claude documents its transcript as internal and version-dependent. `claude import codex` imports config, not sessions. Zed imports threads from its external agents | https://code.claude.com/docs/en/sessions, https://zed.dev/docs/ai/external-agents |
| Standards | ACP `session/load/resume/list` (per agent, no translation), plan entries; MCP `tasks` = async request state, not a todo list; A2A tasks unused by coding hosts; AGENTS.md = instructions only | https://agentclientprotocol.com/protocol/session-setup, https://modelcontextprotocol.io/specification/2025-11-25/basic/utilities/tasks, https://a2a-protocol.org/latest/specification/, https://agents.md/ |

Hooks that carry session state exist on the hosts rtok already wires: SessionStart/SessionEnd/PreCompact on Claude Code, Codex, Cursor, Gemini (`PreCompress`), Copilot; Claude adds `TaskCreated`/`TaskCompleted` (https://learn.chatgpt.com/docs/hooks, https://cursor.com/docs/agent/hooks, https://raw.githubusercontent.com/google-gemini/gemini-cli/main/docs/hooks/index.md).

### 28.3 Third-party alternatives

| Tool | Storage / sync | Claim | Deps + ready | Maintained (2026-09-30) | Source |
| --- | --- | --- | --- | --- | --- |
| beads (`bd`) | Dolt SQL, push/pull on a git ref, cell merge; hash ids; JSONL export only | atomic `--claim` | 4 link types, only `blocks` gates `bd ready` | MIT, Go, v1.3.0 2026-09-15 | https://github.com/gastownhall/beads |
| Gas Town (`gt`) | on beads + worktree "hooks" | `sling`, federated `wl claim` | convoys | MIT, v1.2.1 2026-06-06 | https://github.com/gastownhall/gastown |
| Backlog.md | one Markdown file per task, YAML frontmatter; reads other active branches | assignee + status (not atomic) | deps, ordinal; AC, DoD, plan, notes | MIT, v1.53.0 2026-09-24 | https://github.com/MrLesk/Backlog.md |
| Task Master | `.taskmaster/tasks.json` | none | deps, `next`, validate/fix deps; tags | MIT + Commons Clause; last release 2026-03-31 (slowing) | https://github.com/eyaltoledano/claude-task-master |
| MCP Agent Mail | git Markdown + SQLite FTS5, HTTP server | TTL file leases (exclusive/shared, advisory), build slots, pre-commit guard | uses beads ids as threads | MIT + rider, v0.3.2 2026-04-16 | https://github.com/Dicklesworthstone/mcp_agent_mail |
| Spec Kit / Kiro | `tasks.md` in repo, `[P]` parallel marker / dependency waves | none | phases, waves | Spec Kit MIT v1.0.13 2026-09-29 | https://github.com/github/spec-kit, https://kiro.dev/docs/specs |
| CCPM | Markdown + GitHub Issues as truth | issue per worktree | `depends_on`, `parallel`, `conflicts_with` | MIT, no releases, last push 2026-03-18 | https://github.com/automazeio/ccpm |
| git-bug | git objects, operation DAG, conflict-free | assignee | none | GPL-3.0, v0.11.0 2026-09-22 | https://github.com/git-bug/git-bug |
| Vibe Kanban | SQLite board | board assign | none | README says sunsetting | https://github.com/BloopAI/vibe-kanban |
| casr | canonical session IR → native target file, atomic write, read-back verify, rollback | – | – | MIT + rider, Rust, v0.4.1 2026-09-07, ~120 stars | https://github.com/Dicklesworthstone/cross_agent_session_resumer |
| continues | reads 16 tools' native stores, cross-tool → context document (3/10/20/50-message tiers, file changes, command log, git state) | – | – | MIT, no releases, last push 2026-05-07 | https://github.com/yigitkonur/cli-continues |
| session-porter | Claude ↔ Codex, raw/full/compact/resume-only, registers in Codex's SQLite | – | – | MIT, ~46 stars | https://github.com/liwala/session-porter |
| cass | search over 30+ agents' sessions, SQLite + Tantivy, SSH/rsync sources | – | – | MIT + rider, v0.9.0 2026-09-25 | https://github.com/Dicklesworthstone/coding_agent_session_search |
| SpecStory | Markdown history of ~14 agents, optional cloud; cross-agent resume claimed in README (**unverified**) | – | – | Apache-2.0, v2.15.1 2026-09-24 | https://github.com/specstoryai/getspecstory |

Linear MCP and GitHub MCP give an assignee field (non-atomic) on a SaaS tracker (https://linear.app/docs/mcp, https://github.com/github/github-mcp-server). Task Master's PRD parsing and complexity analysis, Ruflo's swarm and vector memory, and Shrimp (last push 2025-08-21) are out of scope: LLM work or unmaintained.

### 28.4 Feature union, deduplicated, and what rtok does with each

| # | Feature | Origin | rtok verdict |
| --- | --- | --- | --- |
| F1 | Atomic claim: owner + `in progress` in one step, refused if owned | beads `--claim` | **take** — local SQLite transaction + `plan.md` row edit |
| F2 | Claim = lease: owner's agent id must be alive (hook activity); stale claims flagged, not auto-freed | Agent Mail TTL, Gas Town stall detector | **take** — D34 `agents.activity` is the heartbeat |
| F3 | Ready queue: `todo`, unowned, no open blocker, sorted by priority | beads `bd ready`, Task Master `next` | **take** |
| F4 | Typed deps, only "blocked by" gates readiness; validate cycles and dangling ids | beads, Task Master `validate-dependencies` | **take** (one `Depends:` line in the card; the table stays as the rulebook defines it) |
| F5 | Collision-free id allocation | beads hash ids | **adapt** — keep `T<n>`; allocate `max(origin/main, local claims)+1` under the DB lock (the collision is a known pain: memory note on concurrent plan ids) |
| F6 | Progress: readiness %, status transitions, close = move row + card to `done.md`, sync `todo.md` | rulebook, Backlog.md DoD | **take** — rtok writes all three files so they never drift |
| F7 | Session-start priming: inject the agent's claimed task (id, title, card plan) within the `inject` budget | beads `bd prime` | **take** — a token saving: the agent stops re-reading `plan.md` |
| F8 | Messages threaded by task id | Agent Mail, beads messages | **reuse** T287 with a `task` field |
| F9 | Parallel-safe / conflict markers | Spec Kit `[P]`, CCPM `conflicts_with` | **later** — worktree-per-task already isolates |
| F10 | File leases + pre-commit guard | Agent Mail | **skip** — worktree per task covers it; revisit if agents share a checkout |
| F11 | Mirror to GitHub Issues / Linear | CCPM, git-bug bridges | **later**, optional exporter; never the source of truth |
| F12 | Handoff document in tiers (minimal / standard / full): goal, open task, last N prompts, files and symbols touched, commands and errors, git state (branch, diff stat, unpushed commits), `expand` ids | continues, session-porter modes, rtok `handoff` | **take** — extend `handoff` + `checkpoint::extract` |
| F13 | Explicit, one-shot pickup on the target agent (consumed on first SessionStart in the same worktree / task) | Amp handoff, rtok T71.2 | **take** — replaces "newest note of the project" guessing |
| F14 | Native-file conversion so the target's own `resume` works | casr, session-porter | **skip in-tree** — formats are internal and change per release (Claude docs); optionally shell out to `casr` when installed |
| F15 | Parsers for more hosts' transcripts | continues, cass | **take incrementally**: Codex full, Gemini, OpenCode (`opencode export` JSON) first |
| F16 | Cross-machine: handoff and claim travel with git | beads refs, CCPM | **adapt** — claim commit on `plan.md`; handoff as `.rtok/handoff/<task>.md` on the *task branch* (removed before merge), so no sync protocol (D8 holds) |
| F17 | Redaction before export | OpenCode `--sanitize` | **take** — reuse `guard` secret patterns if present, else a minimal token/key regex |
| F18 | Query an old session instead of replaying it | Gas Town `seance`, Amp thread refs | **already** — `mem_search` + `expand` |
| F19 | Mirror host todo into the task | Claude `CLAUDE_CODE_TASK_LIST_ID`, `TaskCompleted` hook | **later** — host todos stay sub-steps of one rtok task; optionally set the list id to `<project>-<task>` so a Claude resume sees the same sub-steps |
| F20 | Board UI | Backlog.md web, Vibe Kanban, Cline Kanban | **later** — `rtok dashboard` page (the plugin trait has `dashboard_page`) |

### 28.5 Build or adopt

The rulebook says take the best ready tool first. beads is the strongest (atomic claim, ready queue, maintained), but its source of truth is Dolt, not `plan.md`; adopting it means two trackers or dropping the workspace-wide `plan.md` convention, plus a Go binary and a Dolt store beside D8's SQLite. Backlog.md is Markdown but one file per task with its own schema, and its claim is not atomic. Neither reads `plan.md`. So the fit is a thin rtok layer over the existing files, not a new tracker. Session handoff: no maintained, permissively licensed Rust library exists (casr carries a licence rider and ~120 stars), and rtok already owns 80 % of the parts (`handoff`, checkpoint extract, SessionStart inject, archive).

Creator decision needed before planning:
1. Source of truth for tasks: **(a) `plan.md` via rtok (recommended)**, (b) beads with `plan.md` generated from it, (c) Backlog.md.
2. Scope: rtok's charter is cutting tokens; F7 and F12–F13 save tokens, F1–F6 are coordination. Ship them in the same binary as a `project` plugin, or as a separate crate under `packages/`.
3. Cross-machine handoff file on the task branch (F16): allowed in the repo tree, or local-only via `memory export/import`.

### 28.6 Proposed shape (if 1a)

One plugin `project` (D21: plugin + MCP as one unit), CLI and MCP with one call path each:

- `rtok task list [--ready]`, `task claim <id>`, `task progress <id> <n>%`, `task done <id>`, `task new <title> [--priority --complexity --depends]`, `task release <id>`. MCP: `task_ready`, `task_claim`, `task_progress`, `task_done`. Claim also runs `worktree claim` (D34) so task, worktree and agent share one owner record.
- Store: a `task_claims` table (task id, repo root, agent uuid, since); `plan.md`/`todo.md`/`done.md` stay the truth and are edited with anchor checks; host config rules from AGENTS.md unchanged.
- `rtok session handoff [--to <host>] [--tier standard]` writes the brief (F12), stores it as note `handoff:<task>` + archive, prints the target command (`codex "…"`, `claude "…"`, `gemini -i "…"`); the target's SessionStart in the same worktree injects it once (F13).
- Measurement rows: bytes injected by priming and handoff vs. the reads they replace; no saving claim without them.

Ideas filed: I-103 – I-107.

## 29. What the native `Read` deny costs (T355, 2026-10-02)

Source: a `sqlite3 .backup` copy of `~/.rtok/rtok.db` taken 2026-10-02 (rtok v0.10.0), hook rows from 2026-09-21 (first T127 deny text) to 2026-10-02. The deny is `src/plugins/read/hook.rs` `pre_tool`: native `Read` of a file over `plugins.read.native_max_bytes` (32,768) that was not just edited, unless `limit` ≤ 5.

### 29.1 Method

1. `h`: every `PreToolUse`/`PostToolUse` hook row with `tool_name`, the path/command argument, `tool_input.limit`, and the `tool_response` length (`PostToolUse` stdin carries it).
2. `d`: `PreToolUse(Read)` rows whose response holds the T127 reason.
3. `g`: per deny, the first `PostToolUse` in the same session within 300 s that reads the same file — rtok MCP `read` (`mcp__*rtok__read`, path equal or a suffix), native `Read`, or `Bash` `cat`/`sed -n`/`head`/`tail`/`nl`/`awk` naming the path — plus whether a `ToolSearch` came before the first rtok MCP call.
4. Tokens are bytes / 4. Native ranged cost: 10.7 tokens per line, measured over the 1,082 allowed native `Read` calls with 5 < `limit` ≤ 300 in the same window. Native full-read cost: the first 2,000 lines of each denied file still on disk (+7 bytes per line for the `N\t` prefix), capped at 25,000 tokens (the host's `Read` limit).

```sql
-- sqlite3 rtok-copy.db < t355.sql   (temp tables only)
CREATE TEMP TABLE h AS
  SELECT c.id, c.session_id s, c.ts, c.name ev,
         json_extract(io.request_json, '$.tool_name') tn,
         coalesce(json_extract(io.request_json, '$.tool_input.file_path'),
                  json_extract(io.request_json, '$.tool_input.path'),
                  json_extract(io.request_json, '$.tool_input.command'), '') arg,
         json_extract(io.request_json, '$.tool_input.limit') lim,
         length(json_extract(io.request_json, '$.tool_response')) out,
         length(json_extract(io.request_json, '$.tool_input')) inl,
         length(json_extract(io.response_json, '$.hookSpecificOutput.permissionDecisionReason')) rl,
         io.response_json LIKE '%use rtok read; before Edit%' deny
  FROM calls c JOIN call_io io ON io.call_id = c.id
  WHERE c.kind = 'hook' AND c.ts >= unixepoch('2026-09-21')
    AND c.name IN ('PreToolUse', 'PostToolUse');
CREATE INDEX temp.h_s ON h (s, ev, id);
CREATE TEMP TABLE d AS
  SELECT id, s, ts, arg p, inl + rl dlen FROM h WHERE ev = 'PreToolUse' AND tn = 'Read' AND deny;
CREATE TEMP TABLE f AS
  SELECT d.id, d.p,
    (SELECT y.id FROM h y WHERE y.s = d.s AND y.ev = 'PostToolUse' AND y.id > d.id AND y.ts - d.ts < 300
       AND ((y.tn LIKE 'mcp__%rtok__read' AND (y.arg = d.p OR d.p LIKE '%/' || y.arg))
         OR (y.tn = 'Read' AND y.arg = d.p)
         OR (y.tn = 'Bash' AND instr(y.arg, d.p) > 0
             AND (y.arg GLOB '*[ |;&(]cat *' OR y.arg GLOB 'cat *' OR y.arg GLOB 'sed -n*' OR y.arg GLOB 'head *'
                  OR y.arg GLOB 'tail *' OR y.arg GLOB 'nl *' OR y.arg GLOB 'awk *')))
     ORDER BY y.id LIMIT 1) rid,
    (SELECT count(*) FROM h y WHERE y.s = d.s AND y.ev = 'PostToolUse' AND y.id > d.id AND y.ts - d.ts < 300
       AND y.tn = 'ToolSearch' AND y.id < coalesce((SELECT min(z.id) FROM h z WHERE z.s = d.s
           AND z.ev = 'PostToolUse' AND z.id > d.id AND z.tn LIKE 'mcp__%rtok__%'), 0)) ts_n
  FROM d;
CREATE TEMP TABLE g AS
  SELECT f.*, CASE WHEN y.id IS NULL THEN 'no read of the file within 5 min'
                   WHEN y.tn LIKE 'mcp__%' THEN 'rtok read'
                   WHEN y.tn = 'Read' AND y.lim IS NOT NULL THEN 'native Read with limit'
                   WHEN y.tn = 'Read' THEN 'native Read, full'
                   ELSE 'Bash cat/sed/head/tail' END how, coalesce(y.out, 0) out
  FROM f LEFT JOIN h y ON y.id = f.rid;
SELECT CASE WHEN x.lim IS NULL THEN 'unranged' ELSE 'ranged' END, g.how, count(*),
       round(avg(g.out) / 4.0), sum(g.ts_n > 0)
FROM g JOIN h x ON x.id = g.id GROUP BY 1, 2 ORDER BY 1, 3 DESC;
```

### 29.2 Numbers

984 denies in 77 sessions. 687 (70 %) of the denied calls were already ranged (`limit` 6…2,000, mean 122 lines; 459 ≤ 100, 166 in 101–300, 62 > 300); 297 had no `limit`.

| Denied call | Follow-up (first read of the same file ≤ 5 min) | Count | Follow-up tokens (mean) |
| --- | --- | --- | --- |
| ranged | rtok `read` | 258 | 1,929 |
| ranged | no read | 277 | — |
| ranged | native `Read` limit ≤ 5 (edit gate) | 145 | 54 |
| ranged | `Bash` cat/sed/head/tail | 7 | 387 |
| unranged | no read | 239 | — |
| unranged | rtok `read` | 48 | 4,538 |
| unranged | native `Read` with limit | 8 | 2,364 |
| unranged | native full / `Bash` | 2 | 867 / 605 |

- The deny itself: ~50 tokens (the call and the reason) and one more model turn. 541 denies (55 %) were followed by a `ToolSearch` before the first rtok MCP call — a second extra turn to load the deferred rtok tool schemas.
- Ranged → rtok `read`: the native range asked for ≈ 112 lines × 10.7 ≈ 1,195 tokens; rtok returned 1,929. Net ≈ −780 tokens per deny, plus one or two turns.
- Unranged → rtok `read`: native estimate median 24,005 tokens (capped mean 23,053; 76 of the 306 rtok-follow-up files still on disk, 34 of them at the 25,000 cap) against 4,538. Net ≈ +18,000 tokens per deny; 48 denies.
- 516 denies (52 %) were not followed by any read of that file within 5 minutes: the agent went elsewhere (the most common next finished tools after a deny are `Bash`, 310, and `ToolSearch`, 246). Whether that is a saving or lost work cannot be told from the store.

### 29.3 Recommendation

Narrow the deny. A ranged native `Read` already costs less than the rtok `read` that replaces it, so the deny loses about 780 tokens and one or two turns on 70 % of its hits. Only an unranged read of a large file pays (about 18,000 tokens per deny). Proposed change (a separate task, not part of T355): let a native `Read` with `limit` ≤ 300 lines pass like the edit gate does, keep the deny for unranged reads and larger ranges, and write a `Measurement` row per deny so the saving is claimed from data rather than from this estimate.

### 29.4 ToolSearch after a deny, pre-release baseline for T384 (2026-10-04)

Measured on 2026-10-04 with rtok 0.15.1 (the build that still says `use rtok read` for every host; T383 and T384 are unreleased), so this is the baseline: the share after the change needs a release and fresh data.

- **Hook rows, the §29.1 query.** The same SQL on a `sqlite3 .backup` copy of `~/.rtok/rtok.db` taken 2026-10-04, counting a deny as followed when a `ToolSearch` finished in the same session within 300 s before the first rtok MCP call. Request bodies of hook rows survive in this database only from 2026-10-01, so the window is 2026-10-01 to 2026-10-04: **89 of 182 denies (48.9 %)**; 2026-10-02 to 2026-10-04 alone: 68 of 136 (50.0 %). §29.2 had 541 of 984 (55 %) over 2026-09-21 to 2026-10-02 from an earlier copy that still held the older bodies.
- **Transcripts, `rtok stats` (new `then ToolSearch` count, T384).** Denies in `~/.claude/projects/**/*.jsonl` followed by a `ToolSearch` within the next 3 tool calls and before any other `Read` or rtok MCP call. `rtok stats --since 30d`: **49 of 166 (29.5 %)**; `--since 10d` (from 2026-09-24): 47 of 157 (29.9 %); `--since 3d`: 13 of 37 (35.1 %). This is the stricter definition (3 calls, a native `Read` in between ends the search), so it reads lower than the SQL; both are kept so the next measurement can be compared like for like.
- What T384 changes: the Claude Code deny now names `mcp__rtok__read` and the `ToolSearch select:mcp__rtok__read` call that loads it. Tool names and deferral come from the vendors' pages read 2026-10-04: Claude Code names MCP tools `mcp__<server>__<tool>` and defers their schemas by default (https://code.claude.com/docs/en/mcp); Gemini CLI names them `mcp_{serverName}_{toolName}` and discovers them at startup (https://geminicli.com/docs/tools/mcp-server/); Codex names them `mcp__<server>__<tool>` in hooks (https://learn.chatgpt.com/docs/hooks) but has no native `Read` tool to deny. Other hosts keep the generic text.
- A setting that keeps the tool loaded exists and is documented: `"alwaysLoad": true` on a server entry loads every tool of that server up front, for all server types, and an MCP server can mark single tools with `"anthropic/alwaysLoad": true` in the tool's `_meta` (https://code.claude.com/docs/en/mcp, "Exempt a server from deferral", read 2026-10-04). `rtok doctor` names the server-wide form with its token price; the per-tool form needs a `_meta` field on the SDK's `ToolDef` and is recorded as an idea.

### 29.5 Symbol-shaped Grep replay for T369 (2026-10-06)

Measured on 2026-10-06 over `~/.claude/projects/**/*.jsonl` (3.1 GB, 80 project folders) with a read-only copy of the indexes in `~/.rtok/rtok.db` (five indexed roots). Only counts are recorded here.

- **Native `Grep` calls: 0.** No `tool_use` named `Grep` exists in the transcripts (the user's hosts deny or never offer it), so the card's replay over `Grep` has no sample. The numbers below are a proxy: shell `rg` / `grep` commands that start a Bash call, 19,499 of them.
- **Narrow classifier** (the first version: a bare identifier or `fn|def|class|struct|type|func|interface <ident>`, no `-g`/`-t`/`-i`/multiline, no path or `.`). First run, 2026-10-06 morning: 172 calls; 119 in a root that has an index; **25 of 119 (21 %)** have one to five definitions in it (92 have none, 2 have more than five); 25 of 172 (15 %) counting the 53 unindexed-root calls as unanswered. Re-run the same afternoon (transcripts and indexes had grown): 26 of 131 (20 %), 26 of 172 (15 %).
- **Widened classifier** (T369 close-out, 2026-10-06; also `\bfoo\b`, `\bfoo\(`, `foo\(`, `\bfoo`, and a `path` that is a directory inside the project, keeping the definitions under it; two or more path arguments, a file, a missing directory or a path outside the project excluded; the directory test ran on today's file system): 1,001 eligible calls, 432 in an indexed root, **128 of 432 (30 %)** answered (298 have no definition, 6 have more than five); 128 of 1,001 (13 %) counting the 569 unindexed-root calls as unanswered.
- Reading: the 70 % bar is not met under either classifier, so the flag ships opt-in. Most identifier greps look for a string, a field or a constant rather than a definition (zero definitions in about two thirds of the indexed-root calls), and a hook can only answer where `rtok graph index` or the MCP graph tools already built an index. The creator delegated the decision to the coordinator on 2026-10-06. The second check (follow-up Grep/Read on the same name within 3 calls is at most 25 % after a dated window with the flag on) is T369.1.
- Re-run: extract `Bash` commands whose first word is `rg` or `grep`, apply the classifier, look the name up in `symbols` (`is_def = 1`, `path` under the directory argument when there is one) under the longest indexed root that contains the call's `cwd`.

## 30. Where each agent keeps its token counts (T358.3, T358.4, 2026-10-03)

What `rtok agents usage --source logs` reads per host, from each host's own source or documentation. A repository is cited by commit, a documentation page by the day it was read. Anything backed only by a secondary source is marked **unverified**; the ccusage guide (https://ccusage.com/guide/, fetched 2026-10-02) was used as a lead for locations only.

| Host | Reads | Verdict |
| --- | --- | --- |
| OpenCode | `message.data` JSON in `opencode.db` | supported |
| Kilo | `message.data` JSON in `kilo.db` | supported, not run on real data (the local `kilo.db` holds no messages) |
| Copilot CLI | `modelMetrics` of `session.shutdown` in `session-state/*/events.jsonl` | supported, per session |
| Gemini CLI | `tokens` of `gemini` messages in `tmp/*/chats/session-*.jsonl` (and legacy `.json`) | supported, not run on real data (no chats on this machine) |
| Droid | none | `unsupported` |
| pi | `usage` of entries in `sessions/*/*.jsonl` | supported, matches an independent sum on real files |
| Kimi Code | `usage.record` lines of `sessions/*/*/agents/*/wire.jsonl` | supported, matches an independent sum on real files |
| Grok | none | `unsupported` (T358.4) |
| ZCode | none | `unsupported` (T358.4) |
| Antigravity | none | `unsupported` (T358.4) |

### 30.1 OpenCode and Kilo

- Sessions live in a SQLite file under the data directory, `${XDG_DATA_HOME}/opencode` (https://github.com/anomalyco/opencode, commit `c42ae0d56b6f86f8df39d451d6d2cfe6414b3928`, `packages/core/src/global.ts`: `path.join(xdgData, "opencode")`). Kilo uses `kilo` the same way (https://github.com/Kilo-Org/kilocode, commit `76bcfd40be616a72f4697b3041565f322245b462`, `packages/core/src/global.ts`) and names its file `kilo.db`, or `kilo-<channel>.db` for a non-stable channel (`packages/opencode/src/storage/db.ts`).
- The `message` table has `id`, `session_id`, `time_created` (milliseconds) and a JSON `data` column (`packages/core/src/session/sql.ts`). An assistant message's `data` carries `role`, `modelID`, `providerID`, `time.created`, and `tokens` = `{input, output, reasoning, cache: {read, write}}` (`packages/core/src/v1/session.ts`, `Assistant`).
- `input` already excludes the cache legs and `output` excludes the reasoning tokens: `input = inputTokens - cacheRead - cacheWrite`, `output = outputTokens - reasoningTokens` (`packages/opencode/src/session/session.ts`, `getUsage`, and the same arithmetic in Kilo's `packages/core/src/v1/session.ts`). The reader adds reasoning back into output, as it is billed as output.
- This corrects the 2026-09-17 note in `measure::codex` that `opencode.db` carries no token counts. Checked on this machine on 2026-10-03: the sums over `role = 'assistant'` messages (input, cache read, cache write, output + reasoning) match what `rtok agents usage --host opencode` prints for all four legs.

### 30.2 Copilot CLI

- The config directory is `~/.copilot`, relocated by `COPILOT_HOME`; `session-state/<session id>/events.jsonl` is the session event log (https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference, read 2026-10-03).
- The SDK's event schema (https://github.com/github/copilot-sdk, commit `5b2d7cdd7da1e5d082ae316a533cbb7584a495c1`, `nodejs/src/generated/session-events.ts`) marks `assistant.usage` (per API call) as ephemeral, "not persisted to the session event log on disk". What is persisted is `session.shutdown.data.modelMetrics[<model>].usage`: `inputTokens`, `outputTokens`, `cacheReadTokens`, `cacheWriteTokens`, `reasoningTokens?`, totals over the whole run. A session that never shut down cleanly has no totals and is not counted.
- **Unverified** (no primary text says it): that `inputTokens` includes the cache legs and `outputTokens` includes reasoning. The one real log on this machine has cache reads below `inputTokens` (18.6M against 17.7M), so the reader treats input as inclusive, and its printed totals are exactly that file's `usage` block. The same event's `tokenDetails` block disagrees with `usage` (uncached input 2.10M plus 17.7M cache reads, output 18.9K against 9.8K): the SDK does not say why, and the reader follows `usage`, the block with documented per-field meaning. Also unverified: whether a resumed session writes a second shutdown with run-only or cumulative totals (the reader sums them).

### 30.3 Gemini CLI

- https://github.com/google-gemini/gemini-cli, commit `fb972b2f87fe7d5b06d37eac711490162d98de2c`. `packages/core/src/config/storage.ts`: the project temp directory is `<home>/.gemini/tmp/<project identifier>`, chats are in its `chats/` directory, and `GEMINI_CLI_HOME` replaces the home directory (`packages/core/src/utils/paths.ts`). Sub-agent chats sit in `chats/<parent session id>/`.
- `packages/core/src/services/chatRecordingService.ts` and `chatRecordingTypes.ts`: a chat is an append-only JSONL log of messages, `$patch` records, `$rewindTo` records and `$set` metadata; a `gemini` message has `id`, `timestamp`, `model` and `tokens` = `{input, output, cached, thoughts?, tool?, total}` where `input` is `promptTokenCount` (which includes `cached`), `output` `candidatesTokenCount`, `thoughts` `thoughtsTokenCount` and `tool` `toolUsePromptTokenCount`. The last record with a given message id wins. An older release wrote one `session-*.json` document with a `messages` array; the CLI migrates it only when the session is opened again.
- Not run on real files: this machine has no Gemini chats.

### 30.4 Droid (Factory)

- Sessions are `~/.factory/sessions/<project folder>/<uuid>.jsonl` plus `<uuid>.settings.json`; the settings file holds "which model, how long it ran, token counts, autonomy mode" (https://github.com/Factory-AI/factory-plugins, commit `d362dc1823301bee56315bd73f22b5614392365c`, `plugins/core/skills/session-navigation/SKILL.md`). No Factory page read on 2026-10-03 (docs.factory.ai CLI settings and session API reference, the plugin above) names the keys, and `DROID_SESSIONS_DIR` is ccusage's variable, not one a Factory page documents. Reading it would be a guess, so the host is `unsupported` and listed in `skipped` when the directory has content. No Droid files exist on this machine.

### 30.5 pi (T358.4)

- https://github.com/earendil-works/pi (the former badlogic/pi-mono), commit `69f0be6f0a6ca1bf66a2a9cf59c821b632e6bca1`, `packages/coding-agent/docs/sessions.md`: sessions are stored under `~/.pi/agent/sessions/`, grouped by working directory; `--session-dir`, `PI_CODING_AGENT_SESSION_DIR` or the `sessionDir` setting moves them. `docs/environment-variables.md` names `PI_CODING_AGENT_DIR` as the agent folder.
- `docs/session-format.md`: a session is JSONL, one entry per line, each with `id` and an ISO `timestamp`. An assistant `message` carries `provider`, `model` and `usage`; `compaction`, `branch_summary` and `usage` entries carry `usage` themselves, and the doc says they all "contribute to session token and cost totals". `docs/message-types.md`: `usage` is `{input, output, cacheRead, cacheWrite, totalTokens, cost}` and `reasoning`, when present, is already inside `output`, so it is not added again. A `usage` entry with an unknown `kind` is normal usage.
- Not documented: whether a forked session copies the parent's entries with their ids. The reader counts an entry once by (`id`, `timestamp`) across all files, which is the same result either way. **unverified**
- Checked on this machine on 2026-10-03: a read-only run of `rtok agents usage --source logs --host pi` against the real `~/.pi/agent/sessions` (10 files, 953 usage entries) and an independent `jq` sum of the same entries give identical totals for all four legs (input 8,063,051; cache read 129,106,459; cache write 0; output 744,047). No file content was copied.

### 30.6 Kimi Code (T358.4)

- https://github.com/MoonshotAI/kimi-code, commit `21406fb4c805cc8c715e6d1f16ad3fb5f25f4fe3`, `docs/en/configuration/data-locations.md`: the data root is `~/.kimi-code`, or `$KIMI_CODE_HOME` when set (`docs/en/configuration/env-vars.md`); sessions are under `sessions/<workDirKey>/<sessionId>/`.
- `packages/agent-core-v2/src/agent/usage/usageOps.ts` and `src/session/usage/usageAgentModel.ts`: each LLM request is recorded as a `usage.record` wire line with `model`, `usage` (`inputOther`, `inputCacheRead`, `inputCacheCreation`, `output`) and `usageScope`, which is `turn` for a request a turn issued and `session` otherwise. Both are single requests that the agent's own totals add up, so the reader sums all of them. The wire files live at `agents/<agent>/wire.jsonl` (`docs/en/guides/sessions.md`); a sub-agent has its own, and counts toward the session above it. `time` is in milliseconds (the wire fixture in the repository's tests).
- The archived MoonshotAI/kimi-cli keeps `~/.kimi` in a different layout; it is not read.
- Checked on this machine on 2026-10-03: a read-only run against the real `~/.kimi-code/sessions` (306 wire files, 360 `usage.record` lines) and an independent `jq` sum give identical totals (input 3,398,492; cache read 33,477,393; cache write 0; output 250,065). No file content was copied.

### 30.7 Grok (T358.4)

- xAI's own guide, shipped in `~/.grok/docs/user-guide/17-sessions.md` of grok 1.0.34 (read 2026-10-03; `~/.grok/version.json`): sessions are `~/.grok/sessions/<encoded-cwd>/<session-id>/` (`$GROK_HOME` moves the base), and `signals.json` holds "token usage, tool/turn counters". It names no keys. The same page documents `grok usage <session-id>` ("Use this instead of reading session files") that prints per-session and per-turn tokens and cost as JSON.
- Verdict: `unsupported`. The documented route is another program, which rtok does not run (D6), and the file's keys are not documented. Reading `signals.json` from observed keys would be a guess. If the creator accepts observed keys as a source, it is a small reader.

### 30.8 ZCode (T358.4)

- https://zcode.z.ai/en/docs/usage-stats (read 2026-10-03): "App Usage" reads "local ZCode session records". No path, file name or token field is given anywhere in the documentation read; `~/.zcode` exists on this machine but what it holds is not documented.
- Verdict: `unsupported`; named in `skipped` when the directory has content.

### 30.9 Antigravity (T358.4)

- Google's Antigravity documentation (https://antigravity.google/docs, read 2026-10-03) says nothing about where Antigravity keeps conversations or token counts. `~/.gemini/antigravity` exists on this machine and holds only an MCP config.
- Verdict: `unsupported`; named in `skipped` when the directory has content.

## 31. Optimization gaps: proxy lanes, LLM compression, other levers (2026-10-04)

What rtok still lacks for cost and token optimization: a separate lane in `rtok proxy` for everything that is not a live agent turn (Batch, bulk/background sync calls, files, embeddings, rtok's own model calls), the LLM lane of P28, prompt-cache engineering and routing, and a gap table of every remaining token lever against `main`. It consolidates `docs/batch-flex.md`, `docs/model-routing.md`, `docs/llm-soft-compression.md`, §16 of this file and the closed `docs/batch-flex-pass` / `docs/token-saving-next` branches into one ordered plan: [`docs/research/optimization.md`](docs/research/optimization.md).

## 32. New host installers (T413)

Facts a host installer writes against. A field that is not in a primary source stays out of the installer.

### 32.1 Roo Code (T413.1, checked 2026-10-05)

- MCP files and stdio shape: https://docs.roocode.com/features/mcp/using-mcp-in-roo — global `mcp_settings.json` (opened from the Roo MCP settings view) and project `.roo/mcp.json`. A name in both files uses the project entry. A stdio server is `mcpServers.<name>` with `command` (required) and `args` (optional); `type` is required only for `streamable-http` and legacy SSE, not for stdio.
- Extension id: https://github.com/RooCodeInc/Roo-Code/blob/main/src/package.json (`publisher` `RooVeterinaryInc`, `name` `roo-cline`, version 3.53.0 on that fetch). Settings file name and directory: https://github.com/RooCodeInc/Roo-Code/blob/main/src/shared/globalFileNames.ts (`mcpSettings` = `mcp_settings.json`) and https://github.com/RooCodeInc/Roo-Code/blob/main/src/utils/storage.ts (`getSettingsDirectoryPath` joins the VS Code `globalStorage` path with `settings`). `roo-cline.customStoragePath` replaces that base when set; the installer does not read the VS Code setting, `[setup.roo] mcp_path` is the override.
- The on-disk globalStorage folder is the lowercased extension id (`rooveterinaryinc.roo-cline`), the same rule `src/agents/cline` already uses for `saoudrizwan.claude-dev`. VS Code's `ExtensionContext.globalStorageUri` is the extension's global storage directory: https://code.visualstudio.com/api/references/vscode-api#ExtensionContext.globalStorageUri (checked 2026-10-05).
- CLI: https://docs.roocode.com/update-notes/v3.50.0 (2026-02-19) documents a `roo` CLI and its stdin protocol. That page does not name an MCP file. No CLI variant until a primary source names the path.
- Hooks and a linkable plugin directory: not in the MCP page or the package manifest's contributes. Both stay `Support::No`. `roo-cline.debugProxy.serverUrl` in `package.json` is a debug proxy, not a model base URL, so proxy stays out too.

### 32.2 Qwen Code (T413.2, checked 2026-10-05)

Qwen Code is a Gemini CLI fork (`qwen`, https://github.com/QwenLM/qwen-code/blob/main/README.md). The installer uses the facts below. Gemini's `BeforeTool` names and millisecond timeouts are not among them.

- User settings file: `~/.qwen/settings.json`. Project settings: `.qwen/settings.json`. `QWEN_HOME` replaces the global configuration directory (default `~/.qwen`); an empty string is unset. https://qwenlm.github.io/qwen-code-docs/en/users/configuration/settings/ (checked 2026-10-05).
- MCP: `mcpServers.<name>` with stdio fields `command` (string) and `args` (array of strings). At least one of `command`, `url`, or `httpUrl` is required. https://qwenlm.github.io/qwen-code-docs/en/users/configuration/settings/ (`mcpServers`) and https://qwenlm.github.io/qwen-code-docs/en/users/features/mcp/ (stdio transport table and the `.qwen/settings.json` example; `qwen mcp add` writes the user scope). Optional `env`, `cwd`, `timeout`, `trust` stay out: rtok's entry is the minimal `{command, args}` those pages show as sufficient.
- Hooks live in settings under `hooks.<Event>[]`, each entry `{matcher?, hooks: [{type: "command", command, timeout}]}`. `timeout` is seconds (default 60). A command-hook value of 1000 or more is still read as legacy milliseconds. https://qwenlm.github.io/qwen-code-docs/en/users/features/hooks/ (page "Last updated on October 1, 2026"; checked 2026-10-05). The same rule is in https://github.com/QwenLM/qwen-code/blob/main/packages/core/src/hooks/hook-timeout.ts (`LEGACY_MILLISECOND_TIMEOUT_THRESHOLD = 1000`).
- Event names are Claude's (`PreToolUse`, `PostToolUse`, `UserPromptSubmit`, `SessionStart`, `SessionEnd`, `PreCompact`, `PostCompact`, `SubagentStart`, and others). An unknown name is skipped (`Invalid hook event name`). https://qwenlm.github.io/qwen-code-docs/en/users/features/hooks/
- Tool matchers: runtime ids, or an exact alias. The hooks page names Claude Code's `Bash`, `Read`, and `Write`. `*` / `.*` / `""` match every value of that event. An omitted matcher matches all (`if (!entry.matcher) return true` in https://github.com/QwenLM/qwen-code/blob/main/packages/core/src/hooks/hookPlanner.ts). `Skill` maps to the `skill` tool in https://github.com/QwenLM/qwen-code/blob/main/packages/core/src/permissions/rule-parser.ts (`TOOL_NAME_ALIASES`, checked 2026-10-05). The hooks page's "including Bash, Read and Write" does not list `Skill`; the alias table does, and the planner comment says tool matchers include every permission-rule name for that one tool.
- Hook stdin is Claude-shaped: `session_id`, `cwd`, `hook_event_name`, `tool_name`, `tool_input`. PreToolUse output is `hookSpecificOutput.permissionDecision` (`allow` / `deny` / `ask`). SubagentStart accepts `hookSpecificOutput.additionalContext`. https://qwenlm.github.io/qwen-code-docs/en/users/features/hooks/ No `--host` adapter is required.
- Extensions: `qwen extensions link <path>` symlinks into `<home>/.qwen/extensions/<name>/` where `<name>` comes from `qwen-extension.json`. `qwen extensions uninstall <name>` removes it. If an extension and `settings.json` both set `mcpServers` under the same name, settings win. https://qwenlm.github.io/qwen-code-docs/en/developers/extensions/extension/ (checked 2026-10-05). That page does not list a `hooks` field. The loader reads `hooks/hooks.json` (or `{hooks: ...}` inside it) when the manifest's `hooks` is absent: https://github.com/QwenLM/qwen-code/blob/main/packages/core/src/extension/extensionManager.ts (checked 2026-10-05).
- `model.baseUrl` is "Not intended to be set by hand — use the `/model` picker or a `modelProviders` entry". Proxy stays out. https://qwenlm.github.io/qwen-code-docs/en/users/configuration/settings/
- Desktop: the README links a desktop build (https://github.com/QwenLM/qwen-code/releases/tag/desktop-latest) and does not name a second settings path. No desktop variant.
- Events with no rtok plugin (`Stop`, `Notification`, `PostToolUseFailure`, and the rest of the hooks page) stay uninstalled.

## 33. Which tests a change reaches (T420, 2026-10-06)

Question: what picks the tests for `just check` so that a docs-only change runs no cargo tests and an edit to one source file runs its unit tests plus the integration tests that reach it. All sources checked 2026-10-06.

| Option | Finding | Source |
| --- | --- | --- |
| cargo-nextest filtersets | Predicates select by test name, crate (`package`, `deps`, `rdeps`), binary and kind. None selects by changed file or git diff. `rdeps` works per crate and rtok is one big crate, so it cannot narrow anything. | https://nexte.st/docs/filtersets/reference/ |
| cargo-difftests 0.6.1 | Coverage based: reruns tests whose executed files changed. Needs nightly Rust and `cargo-binutils`. Last crates.io release 2024-02-24, so unmaintained for over two years; the project rule forbids a dead dependency. | https://crates.io/api/v1/crates/cargo-difftests, https://github.com/dnbln/cargo-difftests |
| cargo-test-changed 0.1.1 | Maps git changes to crates and their dependents. Crate granularity again; last release 2025-04-04. | https://crates.io/api/v1/crates/cargo-test-changed |
| rtok's own graph (`rtok graph affected`, MCP `impact` with a path) | Walks callers from every definition in the changed files and keeps the test files. Measured with the release `rtok 0.15.1` on this repo (499 files, 84 486 rows): a one-line edit to `src/plugins/checkpoint.rs` selected 99 of the 100 `tests/*.rs` targets (878 hit lines) and the first run took 9 min 55 s wall clock (242 s user, 307 s system); the debug build was slower still. Cause: the walk is keyed on bare symbol names, and that file defines `write`, `kind`, `walk`, `extract`, `save`, `render`, so every caller of any function of that name, anywhere, is reached. A docs-only diff answers in 0.14 s. | measured here: `rtok graph affected --since HEAD` in the T420 worktree, 2026-10-06 |
| Name match (`tools/test-changed.sh`, T72.1) | A test file is selected when its name or body mentions a path segment of a changed file (`-w`, case-insensitive); the unit-test binary is filtered by the same segments. The same one-line edit selected 26 of 100 targets and 184 of 1 799 tests, with the test phase taking 18 s. It can miss a test that exercises a module without naming it. | measured here: `just check` with `CHECK_BASE=HEAD`, 2026-10-06 |

Decision: the selection stays the in-repo name match. The only maintained candidates select per crate or need nightly coverage, and rtok's graph selects everything at the cost of minutes. `just check` falls back to `just full-check` when the merge base cannot be computed or a shared input changed (`Cargo.toml`, `build.rs`, `justfile`, `crates/`, `config/`, test fixtures); `just full-check` is the unchanged whole gate and CI never calls `check`. Reopen the graph only if `affected` stops following bare names (qualified symbol keys), which is a graph-plugin change outside T420.

## 34. SessionStart repo map by file-level PageRank (T370, 2026-10-06)

Sources: this repository at branch `t370-pagerank-map` (base `pyrlyn/main` `c7e42944`); numbers from the tests named below, run on a macOS arm64 laptop under a load average of 30 to 50 from other sessions. Empryo (BSL 1.1) was read for the idea only (PageRank over a file graph, damping 0.85, 20 iterations); no code was taken.

- Backtest (`cargo test --lib backtest -- --ignored --nocapture`, `src/plugins/graph/rank.rs`). The tree is indexed once (3.1 s, 499 files, 39 267 edges). For each of the last 200 commits that touched at least one indexed file (129 commits), recall is the share of the commit's indexed files that a `map_tokens = 1000` map lists. The index is today's, not each commit's parent's.

  | Map order | Files listed in 1000 tokens | Recall, all 129 | Newer 64 | Older 65 |
  | --- | --- | --- | --- | --- |
  | `refs` (references per name, T52.3) | 71 | 26.7 % | 23.8 % | 29.6 % |
  | `pagerank`, 2 definitions per file | 109 | 44.1 % | 41.6 % | 46.6 % |
  | `pagerank` personalized by the three previous commits' files | 109 | 47.1 % | 43.8 % | 50.3 % |

  `pagerank` minus `refs` is 17.4 pp over all commits, 17.8 pp on the newer half and 17.0 pp on the older half; the card asks for 15 pp. Definitions per file were set by this backtest: three per file gave 41.7 % (15.0 pp, no margin), one gave 49.8 %. Two keeps a name next to each file while clearing the bar on both halves. The personalized row stands in for "files edited in the last day"; the hook seeds from the indexed mtime.
- Cost on the hook path: a stored graph of 500 files and 39 920 edges is 701 395 bytes of JSON. Decoding it takes 1.4 ms and 20 personalized iterations take 0.33 ms (release, `timing` probe, removed after the run). Building the graph at index time takes 12 ms. `latency_hook_session_start_with_a_pagerank_map_p95_under_10ms` spawns the real hook; a second pass of three variants with the same graph, release: with the map p50 12.9 ms / p95 16.6 ms, with the config but no stored graph p50 10.8 ms / p95 15.7 ms, with neither p50 10.9 ms / p95 13.6 ms. The map adds about 2 ms at p50. On this loaded host the existing SessionStart latency tests (`..._with_project_registration`, `..._with_populated_notes`) also miss 10 ms (p95 13.9 ms and 25.9 ms), so the 10 ms gate needs a quiet machine to be judged.
- The card's check of 30 ms p95 measured 30.9 ms in the first run (load about 50, so just over) and 16.6 ms in the second; the stricter 10 ms budget of T428 cannot be judged on this host. A quiet-machine run of `cargo test --release --test latency session_start` is the open item.

## 35. Task adapters: prior art, GitHub and GitLab APIs (T441.1, checked 2026-10-07)

Every fact below was read on 2026-10-07 from the vendor's docs, the project's own repository or its release API. **Unverified** marks what only a secondary source or nothing at all supports.

### 35.1 Prior art (re-check of the T441 table)

| Tool | Release | Storage | Ids under parallel agents | Sync | Source |
| --- | --- | --- | --- | --- | --- |
| Backlog.md | v1.53.0, 2026-09-24 | One Markdown file per task in `backlog/`, config `backlog.config.yml` | Prefix configurable (`TASK-1`), subtasks `TASK-5.3` (`TASK-5.01` only with `zeroPaddedIds`). The next id is generated inside `FileSystem.withCreateLock()` from active and completed tasks plus other worktrees' uncommitted files ("Same-repository worktrees share the task ID namespace even before their task files are committed") | None (git remote checks only) | [releases](https://api.github.com/repos/MrLesk/Backlog.md/releases/latest), [`src/core/backlog.ts`](https://github.com/MrLesk/Backlog.md/blob/main/src/core/backlog.ts), [`src/utils/prefix-config.ts`](https://github.com/MrLesk/Backlog.md/blob/main/src/utils/prefix-config.ts), [MCP README](https://github.com/MrLesk/Backlog.md/blob/main/src/mcp/README.md) |
| Taskmaster AI (`task-master-ai`) | 0.43.1, 2026-03-31 | `.taskmaster/tasks/tasks.json` | `withFileLock`: exclusive `.lock` file, 5 retries at 100 ms, a stale lock taken over after 10 s | None | [releases](https://api.github.com/repos/eyaltoledano/claude-task-master/releases/latest), [`scripts/modules/utils.js`](https://github.com/eyaltoledano/claude-task-master/blob/main/scripts/modules/utils.js), [`tool-registry.js`](https://github.com/eyaltoledano/claude-task-master/blob/main/mcp-server/src/tools/tool-registry.js) |
| beads | v1.3.1, 2026-09-30 | Dolt, embedded single-writer under `.beads/embeddeddolt/` (server mode for concurrent writers); `issues.jsonl` is interchange only | Hash ids (`bd-a1b2`), 4–8 characters growing with the database, 30 attempts on collision; children `bd-a3f8e9.1`, 3 levels | `bd github`, `bd gitlab`, `bd linear`, `bd jira`, `bd ado`, `bd notion` with `pull`/`push`/`status`/`sync` | [releases](https://api.github.com/repos/steveyegge/beads/releases/latest), [hash ids](https://github.com/steveyegge/beads/blob/main/docs/core-concepts/hash-ids.md), [adaptive ids](https://github.com/steveyegge/beads/blob/main/docs/core-concepts/adaptive-ids.md), [CLI reference](https://github.com/steveyegge/beads/blob/main/docs/CLI_REFERENCE.md) |
| beads_rust (`br`) | v0.7.4, 2026-10-01 | SQLite WAL, `BEGIN IMMEDIATE` writes, `.beads/issues.jsonl` | Prefix `br-`, children `<parent>.1`; child collisions renumbered on merge. README only | Not found | [repo](https://github.com/Dicklesworthstone/beads_rust) |
| GitHub MCP server | v2.0.1, 2026-10-07 | GitHub | GitHub's issue number | `issues` toolset: `issue_read`, `issue_write`, `list_issues`, `search_issues`, `sub_issue_write`, …; `projects`: `projects_get`, `projects_list`, `projects_write` | [releases](https://api.github.com/repos/github/github-mcp-server/releases/latest), [README](https://github.com/github/github-mcp-server) |
| GitLab MCP server | GA in GitLab 19.5, all tiers | GitLab | GitLab's iid | `https://<host>/api/v4/mcp`, OAuth 2.0 dynamic client registration; `save_work_item`, `get_work_item`, `list_work_items`, `link_work_items` | [MCP server](https://docs.gitlab.com/user/model_context_protocol/mcp_server/), [tools](https://docs.gitlab.com/user/model_context_protocol/mcp_server_tools/) |
| Linear MCP | hosted | Linear | `ENG-123` | `https://mcp.linear.app/mcp`, Streamable HTTP, OAuth 2.1 or API key. Tool names are not on the page (`save_issue` **unverified**) | [docs](https://linear.app/docs/mcp) |
| Claude Code tasks | docs as of 2026-10-07 | Host-internal | Host-internal | `TaskCreate`/`TaskGet`/`TaskUpdate`/`TaskList`, legacy `TodoWrite` (`CLAUDE_CODE_ENABLE_TASKS=0`); statuses `pending`, `in_progress`, `completed`, plus `deleted`. `CLAUDE_CODE_TASK_LIST_ID` is not on the env-vars page (**unverified**) | [todo tracking](https://code.claude.com/docs/en/agent-sdk/todo-tracking), [env vars](https://code.claude.com/docs/en/env-vars) |
| Codex CLI `update_plan` | rust-v0.161.0, 2026-10-07 | Protocol types only, no storage in them | — | `StepStatus` = `pending`, `in_progress`, `completed` | [`plan_tool.rs`](https://github.com/openai/codex/blob/main/codex-rs/protocol/src/plan_tool.rs) |

Correction to the T441 card: Backlog.md and Taskmaster both lock id allocation across processes in one repository, and beads avoids collisions with hash ids; beads also syncs GitHub and GitLab. What none of them has is one counter per project shared by every repository checkout, worktree and agent host on the machine, served through the MCP server and CLI the agents already have.

### 35.2 GitHub

- Sub-issues: up to 100 per parent and 8 levels of nesting ([adding sub-issues](https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/adding-sub-issues)). REST `POST /repos/{o}/{r}/issues/{n}/sub_issues` takes `sub_issue_id`, the issue's database id, not its number; also list, remove, reprioritize and `GET …/parent` ([sub-issues API](https://docs.github.com/en/rest/issues/sub-issues)). GraphQL `addSubIssue`, `removeSubIssue`, `reprioritizeSubIssue` ([schema](https://docs.github.com/public/fpt/schema.docs.graphql)). Fine-grained permission: repository Issues, write ([permissions](https://docs.github.com/en/rest/authentication/permissions-required-for-fine-grained-personal-access-tokens)). One parent per issue is implied by `replace_parent`, not stated (**unverified**).
- Rate limits: 5,000 REST requests/hour per user; secondary limits of 100 concurrent requests, 900 points/minute (writes cost 5) and "no more than 80 content-generating requests per minute and no more than 500 content-generating requests per hour" ([REST rate limits](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api)); GraphQL 5,000 points/hour ([GraphQL limits](https://docs.github.com/en/graphql/overview/rate-limits-and-query-limits-for-the-graphql-api)). Over a limit: 403/429, honour `retry-after`, else `x-ratelimit-reset`; wait at least 1 s between writes ([best practices](https://docs.github.com/en/rest/guides/best-practices-for-using-the-rest-api)).
- Search: 30 requests/minute, 1,000 results per query ([search API](https://docs.github.com/en/rest/search/search)); index delay undocumented. `GET /repos/{o}/{r}/issues?labels=…&state=all` filters by label without search; its read-after-write consistency is undocumented (**unverified**).
- Projects v2: GraphQL `addProjectV2ItemById`, `updateProjectV2ItemFieldValue` with `singleSelectOptionId`; classic scopes `read:project` / `project` ([projects API guide](https://docs.github.com/en/issues/planning-and-tracking-with-projects/automating-your-project/using-the-api-to-manage-projects)). A REST API for Projects exists since 2025-09-11: `/orgs/{org}/projectsV2/{n}/items` and `/fields`, same under `/users/{user}/` ([changelog](https://github.blog/changelog/2025-09-11-a-rest-api-for-github-projects-sub-issues-improvements-and-more/), [items](https://docs.github.com/en/rest/projects/items)). Fine-grained tokens cannot reach user-owned projects ([token docs](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens)).
- Closing: `state_reason` = `completed`, `not_planned`, `duplicate`, `reopened` ([issues API](https://docs.github.com/en/rest/issues/issues)).
- Token: `GH_TOKEN` wins over `GITHUB_TOKEN`, both over stored credentials ([gh environment](https://cli.github.com/manual/gh_help_environment)); `gh auth token` prints the stored one ([gh auth token](https://cli.github.com/manual/gh_auth_token)).

### 35.3 GitLab

- Hierarchy: epic → child epic (7 levels, Ultimate), epic → issue, issue → task, on every tier; at most 5,000 direct children ([child items](https://docs.gitlab.com/user/work_items/child_items/)). Issue → issue is not listed. Tasks exist on every tier ([tasks](https://docs.gitlab.com/user/tasks/)).
- REST v4 creates `issue_type=task` but has no parent field; parent/child needs the GraphQL work item API (`workItemCreate`/`workItemUpdate` hierarchy widget; exact arguments **unverified**). No REST work items API ([issues API](https://docs.gitlab.com/api/issues/), [tracking issue](https://gitlab.com/gitlab-org/gitlab/-/issues/368055)). Issue links are `relates_to`/`blocks`/`is_blocked_by` only, every tier ([issue links](https://docs.gitlab.com/api/issue_links/)).
- Status: native Status ("To do", "In progress", "Done", "Won't do", "Duplicate") is Premium and Ultimate, generally available in 18.4 ([status](https://docs.gitlab.com/user/work_items/status/)); set through GraphQL only (**unverified** argument shape). Scoped labels (`status::…`, mutually exclusive) are Premium and Ultimate too; on Free a `status::` label is ordinary and the old one must be removed by hand ([labels](https://docs.gitlab.com/user/project/labels/)). REST edit: `state_event=close|reopen`, `add_labels`, `remove_labels`; list by `labels`, `state=all` ([issues API](https://docs.gitlab.com/api/issues/)).
- Rate limits on GitLab.com: 2,000 authenticated API requests/minute per user, issue creation 200 requests/minute ([GitLab.com limits](https://docs.gitlab.com/user/gitlab_com/rate_limits/)); headers `RateLimit-*`, `Retry-After`. Self-managed issue-creation limit is off by default ([content creation](https://docs.gitlab.com/rate_limits/content_creation/)).
- Token: scope `api` to write, `read_api` to read ([scopes](https://docs.gitlab.com/security/tokens/access_token_scopes/)); `glab` reads `GITLAB_TOKEN` (also `GITLAB_ACCESS_TOKEN`, `OAUTH_TOKEN`) and `GITLAB_HOST` ([glab auth login](https://docs.gitlab.com/cli/auth/login/)).

### 35.4 What this settles for T441

1. Allocator: a Diesel table in rtok's store (`~/.rtok/rtok.db`). The store already runs WAL with `busy_timeout` and `exclusive_transaction` (`src/store/mod.rs`), so read-increment-write is one transaction across processes and crash-safe, with no second file or lock to keep; a JSON file + `flock` would duplicate that.
2. Subtask depth 2 (`R2.1`) fits both providers: a GitHub sub-issue (100 per parent) and a GitLab task under an issue (every tier).
3. GitLab: REST v4 for issues, GraphQL only for the parent link and native Status. Free tier: a `status::in-progress` label the adapter swaps itself, a `relates_to` link when the parent link is unavailable, and close on done.
4. Won't-do is native on both (`not_planned`; "Won't do"), so a `Closed` status apart from `Done` maps cleanly.
5. Bulk create and `sync` pace writes: GitHub's 500 content-creating requests/hour is the tightest limit; one write per second, honour `retry-after`.
6. Collision check on a remote: list issues by the `rtok:R13` label with `state=all` (not the search API: 30/min and an undocumented index delay).
7. Tokens: `GH_TOKEN` → `GITHUB_TOKEN` → `gh auth token`; `GITLAB_TOKEN`. User-owned GitHub projects need a classic token with `project`.

### 35.5 Linear GraphQL (T441.13.6, checked 2026-10-11)

Sources: the developer docs under <https://linear.app/developers> and Linear's own `schema.graphql` in <https://github.com/linear/linear> (`packages/sdk/src/schema.graphql`, SDK 97.1.0, master on 2026-10-11).

- Endpoint and auth: one endpoint, `https://api.linear.app/graphql`; a personal API key goes in `Authorization: <key>` with no `Bearer` prefix ([getting started](https://linear.app/developers/graphql)). Errors are a GraphQL `errors` array (`message`, `path`, `extensions`); a response may be HTTP 200 with partial data plus errors, so the array is checked before success is assumed (same page).
- Create: `issueCreate(input: IssueCreateInput!)` returns `IssuePayload` (`success`, `issue`). The docs example sets `title`, `description` and `teamId`; the schema adds `stateId`, `labelIds`, `parentId` and `priority`, with `teamId` the only required field ([getting started](https://linear.app/developers/graphql); [schema](https://github.com/linear/linear/blob/master/packages/sdk/src/schema.graphql), `IssueCreateInput`).
- Update: `issueUpdate(id: String!, input: IssueUpdateInput!)`; `id` is a UUID or a short identifier such as `BLA-123` ([getting started](https://linear.app/developers/graphql)). `IssueUpdateInput` takes `title`, `description`, `stateId`, `labelIds` (replaces all), `addedLabelIds`, `removedLabelIds`, `parentId` (schema).
- Workflow states: `WorkflowState.type` is one of `triage`, `backlog`, `unstarted`, `started`, `completed`, `canceled`, `duplicate`; `position` orders states of one type; each team owns its states (schema, `WorkflowState`). A state is therefore chosen by type, not by name: `completed` for done, `canceled` for closed.
- Teams: the docs identify a team by id and show a `teams` query ([getting started](https://linear.app/developers/graphql)); the schema's `TeamFilter` has `key: StringComparator`, so `teams(filter: {key: {eq: "ENG"}})` finds it from the short key a person knows.
- Finding an earlier export: `issues(filter: …)` takes `labels: {name: {eq: …}}` ([filtering](https://linear.app/developers/filtering)), `team` and `title` comparators (`startsWith`, `contains`; schema `IssueFilter`, `StringComparator`). The export tags every issue with the label `rtok:<id>` (plus `rtok`) and a title prefix `R1. ` as the GitHub adapter does, and finds them by the `rtok` label. A label must exist before `labelIds` can name it: `issueLabelCreate(input: {name, teamId})` creates one, `issueLabels(filter: {name, team})` lists them; a workspace label has no team (`NullableTeamFilter.null`) (schema).
- Pagination: `first` (default 50), `after`, `pageInfo.hasNextPage`, `pageInfo.endCursor` ([pagination](https://linear.app/developers/pagination)); the docs state no maximum page size (**unverified** above 50).
- Rate limits: API key 2,500 requests/hour and 3,000,000 complexity points/hour; a single query is capped at 10,000 points, each object costs 1, each property 0.1, and a connection multiplies its children by `first` (default 50). A limited request returns HTTP **400** with `extensions.code` = `RATELIMITED` (not 429), plus `X-RateLimit-Requests-*` headers whose reset is UTC epoch milliseconds ([rate limiting](https://linear.app/developers/rate-limiting)). Page sizes are kept at 50 so a listing stays far below the cap.
- Not documented (**unverified**): whether Linear normalises description markdown on write; the export compares the text trimmed, and a difference shows up as an `update` on every run rather than a lost write.

## 36. Cross-session read duplication (T385.13, measured 2026-10-08)

Question (optimization.md §5, "Cross-session read dedup"): how much of the input is the same file content read again in a second session on the same day. Build gate: at least 1 % of input. Ad-hoc read-only SQL and a scratch script over a copy of the owner's real store (`rtok.db`, 335k `calls`, 2026-09-14 to 2026-10-08); no code shipped, no `Measurement` row, no saving claimed.

**Definitions.**

- **Read** = a successful file read, from three `calls` sources. (1) Claude Code native `Read`: `kind='hook'`, `name='PostToolUse'`, `tool_name='Read'`, the text in `tool_response.file.content`. (2) rtok MCP `read`: `kind='mcp_call'`, `name='read'`, `ok=1`, the text in `response_json` (so a different `mode` or range is a different content). Errors, images and `file_unchanged` pointers are dropped. The hook copy of an MCP read (`mcp__rtok__read`) is not counted twice. (3) Cursor `Read`, only as a sensitivity row: its hook stores `content_length` and the path, never the text.
- **Same content** = equal SHA-256 of the returned text, any path (a file read from two worktrees counts; the key is the content, not the path). Cursor: path plus length, which can miss an edit of the same size and cannot see a copy at another path.
- **Within a day** = the same UTC calendar day (`strftime` of `calls.ts`). A whole-window row with no day bound is the upper bound for any 24 h window.
- **Cross-session duplicate** = for each (day, content), the first read in every session except the earliest session. Repeats inside one session (and between a parent and its sub-agents, which share `session_id`) are the existing in-session dedup and are not counted. Bytes = the duplicate's result bytes, tokens = bytes / 4 (the estimator used in §2).
- **Input** = uncached + cache_create + cache_read tokens of the same window from `RTOK_HOME=<copy> rtok stats --since 79h --json`, main transcripts plus sub-agent transcripts (`subagents.usage_*`), as in §2. The copy keeps `stats` from writing to the real store.
- **Resident weighting**: a read stays in the context for the rest of the session, so its share of input is its tokens times the tool's context-token-turns per token (`tools.Read.ctt / est_tokens` = 735; `mcp__rtok__read` = 1,260 in the same window), divided by input. Generous: sub-agent windows are shorter than the main one.

**Window A (the headline): 2026-10-05 to 2026-10-08, native Read plus MCP read.** Inline hook bodies exist only from 2026-10-05; before that `call_io` keeps hashes of the whole payload, which cannot identify a file.

| Measure | Value |
| --- | --- |
| Sessions with a read / reads / bytes read | 38 / 3,049 (1,932 native, 1,117 MCP) / 13,613,229 B |
| Reads whose content another session read the same UTC day | 232 reads in 25 (day, content) groups |
| Cross-session duplicate reads | **38** (1.2 % of reads) |
| Duplicate bytes / tokens | **56,561 B ≈ 14,140 tokens** (0.42 % of the bytes read) |
| Input in the window (main + sub-agents) | 4,139,214,544 tokens |
| Share of input, duplicate counted once | 0.00034 % |
| Share of input, resident weighted | **0.25 %** (x735) to **0.43 %** (x1,260) |
| No day bound (any gap inside the 3.3 days) | 59 reads, 78,598 B ≈ 19,649 tokens, resident 0.35 % to 0.60 % |
| Same, keyed on path plus content instead of content | 2 reads, 2,698 B (the content matches found above sit at different paths, mostly different worktrees) |

Where the duplicates sit (basenames only): `mod.rs`, `retry.rs`, `gc.rs`, `terminal.rs`, `sse.rs` (source files), then `AGENTS.md`, `Cargo.toml`, `release-plz.toml`. No file dominates; the largest basename is 21 % of the duplicate bytes.

**Cross-checks.**

| Source | Window | Result |
| --- | --- | --- |
| MCP `read` only, response hash | 2026-09-14 to 2026-10-08, 128 sessions, 5,615 reads, 26,192,465 B | 93 duplicate reads, 290,927 B ≈ 72,731 tokens (1.1 % of bytes); input 22,257,526,251 tokens (`--since 24d`, 279 sessions); share 0.00033 % once, 0.28 % resident (x864) |
| `read_cache` (session, path, content hash) joined to `archive.bytes`, rtok-handled reads | 2026-09-04 to 2026-10-08, 146 sessions, 7,221 entries, 42,378,105 B | 56 duplicate entries, 354,322 B ≈ 88,580 tokens (0.84 % of the bytes); since 2026-10-05: 7 entries, 15,380 B (0.21 %) |
| Cursor `Read`, path plus `content_length` | 2026-10-05 to 2026-10-07, 34 sessions, 4,477 reads | 76 duplicate reads, 488,485 B (0.20 % of the 242,932,224 B its hook reports). Cursor sessions are not in the `stats` input, so no share is given; the duplicate fraction of bytes is the same order as Claude Code's |

**Result.** Under every definition tried the cross-session duplicate is **0.2 % to 1.1 % of the bytes read** and **0.25 % to 0.60 % of input** after weighting for how long a read stays resident (0.0005 % if each duplicate is counted once). That is below the 1 % gate, so no build task is filed and `optimization.md` §5 keeps "Cross-session read dedup" as not built.

**Why it is small.** Sessions on this machine work in separate worktrees, so the same file sits at different paths and a path-keyed cache would catch almost nothing (2 reads); only a content-keyed cache would match, and the content-keyed pool is still 0.4 % of read bytes. Within a session the existing guard and read cache already collapse repeats, and `rtok read` returns line-range slices, so two sessions rarely ask for the same slice of the same file on the same day.

**Limits.** (1) Native Read bodies exist for four days only; the 25-day figure is MCP `read`, which is a lower bound for all reads. (2) The denominator is every Claude Code transcript in the window, sub-agents included (63 % of the input), and the reads counted include sub-agent reads, so the whole-input figure is the matching one. Against main-session input alone (1,518,798,579 tokens) the resident-weighted share would read 0.68 % to 1.17 %, but that pairs sub-agent reads with a denominator that leaves their input out. (3) Reads by a different path to the same content in the same session are not cross-session and are not counted. (4) Commands: `sqlite3 -readonly` on a file copy of `rtok.db` for the schema checks; a Python 3 script (`sqlite3`, `hashlib`) grouped reads by (UTC day, SHA-256); `RTOK_HOME=<copy> rtok stats --since 79h --json` and `--since 24d --json` for input and the CTT ratios. Re-run these after a change to the read plugin or a move to shared worktrees; a result above 1 % would justify a content-keyed, cross-session pointer.

## 37. Deferred tool schemas and replayed thinking, re-measured (T385.11, measured 2026-10-11)

Both ideas were last judged before the thinking page and the tool search page of the Anthropic docs changed (checked 2026-10-11). Method for both: every Claude Code transcript under `~/.claude/projects/**/*.jsonl` with mtime in the last 31 days, lines stamped 2026-09-11 or later, unique `message.id`, context of a request taken from the `parentUuid` chain (so a compaction ends the chain), input = `input_tokens` + `cache_creation_input_tokens` + `cache_read_input_tokens`.

**Vendor facts.** (1) Claude Opus 4.5 and later Opus, Sonnet 4.6 and later Sonnet, Haiku 5.5, Fable and Mythos "keep all prior turns" of thinking, and "retained thinking blocks count as input"; earlier models and Haiku through 4.5 keep the last turn only (https://platform.claude.com/docs/en/build-with-claude/thinking, section "Thinking block preservation by model", fetched 2026-10-11). The old premise of I-86, "the API already strips earlier turns", holds only for those older models. (2) Outside a tool-use turn a client may omit prior turns' thinking; inside one it must pass them back unmodified (same page, "Preserving thinking blocks"). (3) On accounts created from 2026-08-31 a block stays valid only while everything before it is unchanged; a client-side edit of an earlier message makes the later blocks invalid, and the API errors or drops them. "Server-side context management never invalidates thinking blocks" (same page; https://platform.claude.com/docs/en/build-with-claude/context-editing, fetched 2026-10-11). (4) The server-side way to drop old thinking is the beta strategy `clear_thinking_20251015` with `keep` `{type: "thinking_turns", value: N}`; "when thinking blocks are cleared, the cache is invalidated at the point where clearing occurs" (context-editing page). (5) `display: "omitted"` is the default on Opus 4.7 and later and on the 5.x models: the `thinking` field is empty and the `signature` "carries the encrypted full thinking" (thinking page, "Controlling thinking display"). (6) Tool search: `defer_loading: true` per tool plus `tool_search_tool_regex_20251119` or `tool_search_tool_bm25_20251119`; deferred tools are excluded from the prefix, so caching is kept; all tools are still sent every request (https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool, fetched 2026-10-11). OpenAI Responses has the same feature for `gpt-5.4` and later: `{"type": "tool_search"}` plus `defer_loading: true` (https://developers.openai.com/api/docs/guides/tools-tool-search, fetched 2026-10-11).

**Why T125's 0.0297 % was wrong.** `measure::jsonl` counts the `thinking` text. On the current models that text is empty (every block in the last 3 days of transcripts has `thinking` = "" and a 900 to 1,600 character `signature`), so the numerator was about zero; it also counted each block once instead of once per request that replays it (the resident weighting T385.13 uses).

**I-86, thinking replay.**
- Size of an omitted block: two independent fits agree. Output tokens of messages with a thinking block against signature length give a median of 4.6 signature characters per thinking token (10,667 messages); a least-squares fit of the growth in input between consecutive tool-loop requests on signature length gives 3.99 characters per token (130,850 request pairs). Table values use 4.3, with 4.0 to 4.6 as the range.
- Whether the host replays: on keep-all models the first request after a new user prompt reads 100 % of the previous request's input from cache in the median (91.8 % of 1,643 such requests read at least 90 %), so Claude Code sends earlier turns' thinking unchanged.
- Window 2026-09-11 to 2026-10-11, 153,539 requests on keep-all models (97.7 % of requests), 22.92 G input tokens: replayed thinking is **11.5 %** of input tokens (10.8 % to 12.4 %). Two thirds of it belongs to the current turn (long tool loops) and must be sent. The part from earlier turns, which the docs allow to omit, is **3.4 %** (3.2 % to 3.7 %).
- Dollars (cache read 0.1, 5-minute write 1.25, output 5, relative to input 1; 97.8 % of input is cache reads): dropping earlier turns' thinking on every request saves 0.1 per replayed token per request but re-creates the span after the first dropped block each time a new turn starts. Simulated on the same transcripts (3,700 warm turn starts, 681 cold requests): net **1.3 %** of input cost (1.1 % to 1.6 %), 1.2 % of input plus output cost; gross 3.0 %, of which 1.7 % is paid back as cache re-creation. Dropping only at cold requests (TTL expired, no break cost): 1.35 % (1.26 % to 1.45 %).
- A proxy can only do this client-side (edit `messages`), which on current accounts invalidates the later blocks of the same turn: Claude Code logged 338 `thinking_drop` attachments (`reason: prefix_mismatch`) in the window for that reason. The safe route is the server-side `clear_thinking_20251015` edit (beta). Quality with the older thinking removed is unmeasured; the docs call keeping it "recommended" and say preserving has "no negative effect".
- Verdict: **I-86 dropped** by the creator (2026-10-11). The token share (3.4 %) clears I-86's own 3 % gate, but the dollar saving (1.1 % to 1.6 %) sits on the 1 % gate used since T385.13, the quality risk is unmeasured, and a client-side edit is unsafe. If it is ever revisited, the only safe route is the server-side beta edit `clear_thinking_20251015`.

**I-85, deferred tool schemas.**
- Claude Code already defers: 1,491 of 1,495 sessions (99.7 %) carry a `deferred_tools_delta` attachment and in every one of them MCP tool names are among the deferred ones; the session's first request has a median 64,497 input tokens (main sessions, n = 228; sub-agents 53,182, n = 1,267) against a mean of 147,225 per request. 404 `ToolSearch` calls against 4,631 MCP tool calls. `tools_rewrite`'s 6.2 % (§2, 2026-09-18) was measured with Tool Search off on a host with a custom `ANTHROPIC_BASE_URL`; with that variable pointing at `https://api.anthropic.com` the host defers by itself.
- Hosts that do not defer (Codex, Cursor, Gemini wires) have no request bodies here: `usage` has 0 rows and `calls` has 0 proxy rows in `rtok.db` (2026-10-11), so there is nothing to measure and no saving is claimed. Both Anthropic and OpenAI now do the deferral server side (facts above), so a proxy stub scheme would be a worse copy of a vendor feature.
- Verdict: **I-85 dropped**, no measured saving left on the traffic we can see.

**Limits.** Signature length is a proxy for billed tokens (two fits, above). The replay assumption is an upper bound for hosts other than Claude Code. The cost model uses list-price ratios, not the invoice. Cursor and Codex sessions are not in the transcripts. Commands: Python 3 scripts reading the transcripts (parent-chain walk, `-I` mode), `sqlite3` on a file copy of `rtok.db` for the empty `usage` table; no change to rtok code.

## 38. Local draft, cloud verify (T404, checked 2026-10-11)

Question (§16.3 #10): can a local model draft output that the cloud model only verifies, so the cloud emits fewer output tokens? Desk research only: no code, no bench, nothing measured here. Primary sources are the vendor pages named in each row, fetched 2026-10-11; "inference" marks a step that follows from those sources but that no page states.

### 38.1 Can a host or proxy hand the cloud model a draft?

| Fact | Source (checked 2026-10-11) |
| --- | --- |
| Anthropic Messages API: a last message with role `assistant` is a **prefill** and the answer continues from it, but "on models that don't support prefill" such a request returns 400. | https://platform.claude.com/docs/en/api/messages, `messages` parameter |
| Prefill is **not supported on Claude 4.6 and later models** (and Mythos Preview). A prefilled last assistant message returns 400 `invalid_request_error`: "This model does not support assistant message prefill. The conversation must end with a user message." Suggested replacements: structured outputs, system-prompt instructions, `output_config.format`. | https://platform.claude.com/docs/en/api/errors, "Prefill not supported" |
| The current models on the pricing page (Fable 5.1, Mythos 5.1, Opus 5.5, Sonnet 5.5, Haiku 5.5) are all later than 4.6, so **prefill is unavailable on every current Claude model**. Inference from the two rows above plus the model list in the migration index (https://platform.claude.com/docs/en/about-claude/models/migration-guide). | same |
| OpenAI Predicted Outputs: `prediction` parameter on Chat Completions, only `gpt-4o`, `gpt-4o-mini`, `gpt-4.1`, `gpt-4.1-mini`, `gpt-4.1-nano`; unsupported with `tools`, `max_completion_tokens`, `n` > 1, `logprobs`. Predicted tokens absent from the final answer "are still charged at completion token rates", so a bad draft raises the bill. | https://developers.openai.com/api/docs/guides/predicted-outputs |
| Gemini text-generation guide: no prefill, speculative-decoding or predicted-output feature documented (absence on that page only, not a statement that none exists). | https://ai.google.dev/gemini-api/docs/text-generation |
| Claude Code, Codex and other hosts as injection points: not researched from their own docs here. Inference: a proxy only rewrites a request the host built, so on a current Claude model it cannot append an assistant prefill (400 above), and on OpenAI it cannot attach `prediction` to a request that carries `tools`, which every coding-agent turn does. | inference |

Consequence: the "pre-filled draft" mechanism that makes draft-and-verify cheap is gone on Anthropic and unusable with tools on OpenAI. What remains is to put the draft in the **user** turn and ask the model to confirm or correct it.

### 38.2 Is there a draft worth sending? Where the output tokens go

§2 and §16.3 #1 already measured this workload: 96 % of assistant output is tool input (the code and edit arguments the model writes), and `new_string` is 2.2x `old_string`. A local draft would have to be that new code. Prefill-style text continuation cannot carry a `tool_use` block in any case (inference: prefill is text-only in the API reference above), so only a user-turn draft is possible, and the cloud model then has to re-emit the accepted draft as its own tool call: **an accepted draft still costs the full output tokens of the tool call**. Confirming "yes" saves nothing when the useful output is the edit itself. The saving exists only for prose answers the model could end with "accepted as written", and §2 puts the cost in tool input, not prose.

### 38.3 Local draft speed (Apple Silicon)

Generation speed is memory-bandwidth bound. Source: llama.cpp's own benchmark thread, LLaMA 7B (6.74 B parameters), tg128, batch 1, build 8e672ef (https://github.com/ggml-org/llama.cpp/discussions/4167, checked 2026-10-11):

| Chip | Q4_0 | Q8_0 | F16 |
| --- | --- | --- | --- |
| M3 Max 40 GPU cores (400 GB/s) | 66.3 t/s | 42.8 | 25.1 |
| M4 Max 40 GPU cores (546 GB/s) | 83.1 t/s | 54.1 | 31.6 |
| M3 Ultra 80 cores (800 GB/s) | 92.1 t/s | 63.9 | 39.8 |

So a 7 B model at Q4 drafts about 65 to 90 tokens/s on a Max-class chip; that is fast enough that speed is not the blocker. The numbers are a 2023-era base model on a plain generation test: they say nothing about draft **quality** for code against current Claude models. No coding pass rate for any local model was found or measured. MLX also ships speculative decoding (`--draft-model`, `--num-draft-tokens`, default 3; same tokenizer required: https://github.com/ml-explore/mlx-lm/blob/main/mlx_lm/generate.py), but that is a local-only trick where the *big* model is also local; it does not apply to a cloud verifier, which exposes no draft-token interface in the pages above.

### 38.4 What verification costs

Prices per MTok (https://platform.claude.com/docs/en/about-claude/pricing, checked 2026-10-11; OpenAI: https://developers.openai.com/api/docs/pricing, same date): Fable 5.1 input $10, output $50, cache read $0.25; OpenAI gpt-5 input $1.25, output $10. Output is 5x input on Fable 5.1 (§2 p_out = 5 p_in) and 8x on gpt-5.

Idealised model (inference, not measured). A task has D output tokens when written by the cloud model alone: cost = D x p_out. With a draft in the user turn: the draft is fresh input (no cache hit, it was never seen), so D x p_in; the model answers with the verdict V (small); on reject it writes the D tokens itself, probability 1 - a. Cost = D p_in + V p_out + (1 - a) D p_out. Break-even at p_out = 5 p_in is a > 0.2 + V/D with zero thinking tokens and an accept that costs only V. If the accepted draft must be re-emitted as a tool call (37.2), the accept path also costs D p_out and the saving is **zero or negative** (the draft input is pure extra cost). Adding the reasoning tokens a thinking model spends checking a draft, and the extra turns of a rejected draft, raises the required `a` further; the sources give no figure for either, so none is claimed.

Also: wrong-draft anchoring (the verifier accepts plausible code it would not have written) is a quality risk; no primary source measures it for these models, so it stays unquantified.

### 38.5 Verdict

**Close T404 with this finding; do not build.** (1) The one API mechanism built for this, assistant prefill, is removed on every current Claude model, and OpenAI's equivalent excludes tool calls, so a proxy cannot inject a draft where it matters. (2) The output that dominates cost is tool-call input, which prefill could not carry and which the cloud model must re-emit even after accepting a draft. (3) Local drafting speed is adequate (about 65 to 90 t/s for 7 B at Q4 on Max-class chips), but speed was never the constraint.

Reopen only if a bench could show **cost per passed task falling with the pass rate held**: that needs a host path that lets a verifier accept a draft without re-emitting it (for example a new prefill-like or edit-by-reference API, or a draft delivered as a tool result the model can apply by id), plus a local model whose measured accept rate `a` on this workload clears the break-even in 37.4 with thinking tokens included. Neither exists in the sources checked today. Reliability: every row above is a vendor page or the repository's own file, fetched 2026-10-11; the llama.cpp thread is community-run but primary for its own numbers. No blog or forum fact is used. The break-even is arithmetic on published prices, not a measurement.

