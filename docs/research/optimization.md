# Optimization gaps: proxy lanes, LLM compression, other levers

Research and plan note (2026-10-04). Status: proposal, nothing built in this change. rtok
paths were checked against `origin/main` (`bada4224`, 2026-10-04); provider facts against the
provider docs on the same date. Vendor percentages stay vendor claims unless marked
*measured*.

This note gathers the optimization research that already exists (on `main` and on closed
branches, see §7), says what rtok still lacks, and turns it into one ordered plan. It covers:

1. **Proxy** — what is missing so that every request through `rtok proxy` that is not a live
   agent turn (Batch, bulk/background sync calls, files, embeddings, metadata) is recognised
   and handled on its own lane.
2. **LLM compression** — the model-based lane of P28 on top of the shipped lossless and
   extractive lanes.
3. **Two more levers** — prompt-cache engineering and model routing.
4. **What else is missing for maximum token reduction** — every remaining lever rtok does not
   pull yet, against the current code.

Promote items into `plan.md` cards (each with one `Check:` line) before coding. Note: the
`T250`–`T259` ids used for the Batch/Flex cards in `roadmap.md` collide with unrelated cards
already in `done.md` (`T251`–`T255`); the promoted cards need fresh ids.

## 1. Summary

| # | Area | Today (main, 2026-10-04) | Main gap | First step |
| --- | --- | --- | --- | --- |
| 1 | Proxy lanes | Exact sync wires only; everything else is a byte-identical fallback hop | No request classifier, no per-lane policy, no Batch/Flex config | Lane classifier + `calls.kind` tag, no behaviour change |
| 2 | LLM compression | Lossless archive + extractive `compress`, both only in `proxy.mode = "compress"` | No compressor client, no eligibility set, no Gate P28 bench | Phase 1 measurement from `docs/llm-soft-compression.md` |
| 3a | Prompt cache | Byte-stable prefix, live-zone rewrites outside `keep_turns` | No breakpoint injection, no sticky upstream, no per-lane hit rate | Per-lane cache-hit ledger |
| 3b | Model routing | Planned only (`docs/model-routing.md`, D9) | No policy, no measurement | Route rtok's own and bulk-lane calls first |
| 4 | Max token reduction | Most levers ship **off** by default | Defaults, thinking replay, deferred tool schemas, … (§5) | Bench the defaults, then flip what wins |

## 2. Proxy: a separate lane for everything that is not a live agent turn

### 2.1 What the proxy does today

From `src/proxy/mod.rs`, `src/proxy/wire.rs`, `config/default.toml` and `docs/batch-flex.md`:

- `Wire::matches` is exact for the sync chat shapes: Anthropic `/v1/messages`, OpenAI
  `/v1/chat/completions` and `/v1/responses`, Gemini `:generateContent`. Only these get
  `record` → `compress` (in `compress` mode) → `prepare` → usage parsing.
- Every other path (Anthropic `/v1/messages/batches…`, OpenAI `/v1/batches`, `/v1/files`,
  `/v1/embeddings`, `/v1/models`, `count_tokens`, …) hits the axum fallback: forwarded byte for
  byte, a `calls` row when bookkeeping is on, no usage row, no policy.
- `proxy.mode` defaults to `passthrough`, so the archive live-zone rewrite, `toon` and
  `compress` (all `proxy_filter`s) do not run unless the user switches to `compress`.
- `[proxy.batch]`, `[proxy.flex]` and `[proxy.routing]` are documented stubs only; uncommented
  they fail `rtok config validate`.
- The semantic cache (P31, off) already hashes a caller identity from request headers
  (`semantic_cache::caller_identity`); nothing else in the proxy distinguishes callers.
- There is no concurrency limit, queue or rate-limit isolation between callers.

So a script that sends thousands of bulk requests through the same `rtok proxy` port as the
agent is treated exactly like the agent: same compress pipeline, same upstream, same tier,
same ledger bucket, and it competes for the same provider rate limit.

### 2.2 What is missing

| Gap | Why it matters | Notes |
| --- | --- | --- |
| **Lane classifier** | Nothing can be handled separately until it is recognised | Lanes: `agent` (sync wire with tools and a session), `bulk` (sync wire, opted in or no agent markers), `batch` (provider Batch create/poll/list/cancel/results), `files`, `embeddings`, `meta` (`/v1/models`, `count_tokens`), `internal` (rtok's own model calls, e.g. the P28 compressor). Classify by path first, then an explicit header (e.g. `x-rtok-lane: bulk`, stripped before forwarding) or a path prefix (`/lane/bulk/v1/…`), then heuristics only after a measurement. |
| **Ledger tag per lane** | `rtok stats` / `report` cannot show Batch, bulk or Flex spend apart from agent spend | `calls.kind` / `call_io` metadata (roadmap S2); no schema change if `kind` is enough. |
| **Per-lane policy table** | Agent-only rewrites must never touch Batch JSONL; bulk traffic needs different rules | One table decides, per lane: `compress`/archive, `toon`, `tools_rewrite`, `context_management`, semantic cache, Flex, routing, upstream, timeout. Batch/files: pass-through always. |
| **Batch observe + results → usage** | Provider Batch is the cheapest tier for offline work; the ledger is blind to it | Roadmap S2/S3: tag create/poll, parse result lines into `usage` when `parse_results = true`, fail open. OpenAI and Anthropic both price Batch at a 50 % discount (OpenAI pricing page and the aicost.tools vendor table, 2026-10-04). |
| **Flex on the bulk lane** | Same price as Batch on a sync call | OpenAI docs (2026-10-04): `service_tier = "flex"` is billed at Batch rates and prompt-cache discounts still apply; a capacity miss returns `429 Resource Unavailable` and is not billed. Inject only on `bulk`/`internal`, never silently on `agent`; retry policy `none` / `backoff` / `default` (retry with `service_tier = "auto"`). Anthropic has no Flex twin. |
| **Per-lane upstream** | Bulk or internal calls may go to a cheaper provider or region; Batch must go to the provider that owns the job | `upstream`, `openai_upstream`, `gemini_upstream` are global today. |
| **Isolation (queue / in-flight cap)** | A bulk run must not starve or rate-limit the interactive agent | Per-lane `max_in_flight` and a small queue; the agent lane is never queued behind bulk. |
| **Opt-in Batch CLI** | Humans and scripts that already have JSONL | `rtok batch submit/status/fetch` from `docs/batch-flex.md`; talks to the same proxy hop; no sync→Batch conversion. |
| **Per-lane prices** | `stats --price` needs Batch/Flex rows | Dated USD/MTok rows under `[stats.prices]`. |
| **Reporting** | Prove savings from the ledger, not the provider console | `report` / `stats` breakdown by lane and tier (roadmap S5). |

Boundaries stay as `docs/batch-flex.md` sets them: never convert a live agent turn into a
Batch job; never rewrite Batch JSONL; hooks and MCP never see LLM HTTP, so all of this lives
under `src/proxy/`.

### 2.3 Plan (proxy)

| Stage | Work | Check |
| --- | --- | --- |
| L0 | Config structs for `[proxy.batch]`, `[proxy.flex]`, `[proxy.routing]` and a new `[proxy.lanes]` table; defaults keep today's bytes | Config golden tests; proxy bytes identical with defaults |
| L1 | Lane classifier (path → header → prefix) and `calls.kind` tag; header stripped before forwarding | Integration: each lane path gets its tag; `/v1/messages` still matches the Anthropic wire, `/v1/messages/batches` does not |
| L2 | Per-lane policy: compress/archive/semantic cache/tools_rewrite only on `agent` by default | Bulk/batch bodies byte-identical in `compress` mode |
| L3 | Batch observe + opt-in `parse_results` (roadmap S2/S3) | Fixture result streams → expected `usage` rows; malformed line fails open |
| L4 | Flex on `bulk`/`internal` with the 429 policy (roadmap S4) | Mock upstream: omit/force/respect matrix; 429 → configured behaviour |
| L5 | Per-lane upstream and in-flight cap | Agent request latency unchanged while a bulk burst runs (test with a slow mock upstream) |
| L6 | `rtok batch` CLI, prices, `report` lane breakdown (roadmap S5) | `stats --price` shows Batch/Flex rows from fixtures |

## 3. LLM compression

### 3.1 Today

The design is already written: `docs/llm-soft-compression.md` (phases 0–5) and
`src/plugins/compress/PLAN.md` (T28.0). Shipped: the lossless archive with `expand`, the live
zone (`plugins.archive`, `keep_turns = 4`, `min_tokens = 1500`), and the deterministic
extractive `compress` (T28.2 / T127). Both run only in `proxy.mode = "compress"`. Not built:
any model call. Gate P28 (`roadmap.md`): `rtok bench` cost per passed task must not rise
against the lossless path, and `expand` must still recover a non-regenerable original.

### 3.2 State of the art (checked 2026-10-04)

| Method | Claim (source) | Fit for rtok |
| --- | --- | --- |
| LLMLingua-2: token keep/drop classifier distilled from GPT-4 | Its paper reports 2x–5x compression and 1.6x–2.9x end-to-end speed-up (arXiv 2403.12968, ACL Findings 2024) | Extractive, small model, no generation; weak on identifiers and code. Idea only (D6 rejects wrapping the Microsoft package). |
| ACON: compression guidelines optimised in natural language from agent failures, then distilled into a small compressor | Its authors report 26–54 % lower peak tokens with equal or better task success (arXiv 2510.00615, ICML 2026) | Closest to rtok's case (long-horizon agents, API models, no weight changes). The guideline loop maps onto `rtok bench` failures. |
| Host compaction / provider context editing | Anthropic `clear_tool_uses` is already behind `proxy.context_management` (T51.2, off) | Complementary: drops whole results; rtok keeps the archive id. |
| KV-cache compression, quantisation, attention sinks | Serving-side | Out of scope: rtok never sees tensors (`docs/llm-soft-compression.md` §2 C). |

### 3.3 What is missing

| Gap | Notes |
| --- | --- |
| **Phase 1 numbers** | Share of archived bytes still shown live, code vs prose mix, must-keep spans (paths, line numbers, error codes, rustc spans). Without them Gate P28 is guesswork. |
| **Compressor client on the `internal` lane** | One small client that calls a configured cheap model through the proxy's own lane (§2), so its spend is tagged, can use Flex (OpenAI) and can be routed (§4.2). |
| **Summarise once, reuse every turn** | Store the summary next to the archive row, keyed by archive id + compressor version. The replacement text is then byte-stable across turns, so the prompt cache survives (the same rule `docs/prompt-cache.md` sets for pointers). |
| **Off the hot path** | Compress asynchronously when a result enters the archive; use the summary from the next turn on. A request never waits on the compressor (fail open to the extractive view). |
| **Eligibility** | Only archived results outside `keep_turns`, above a size floor, by family (logs and prose first; diffs and source last). |
| **Guideline loop (ACON-style)** | Keep the compressor prompt as data; when a bench task fails with compression and passes without, record which dropped fact mattered and update the guideline. |
| **Code-aware guard** | Must-keep spans copied verbatim into every summary; reject a summary that drops one. |
| **Gate P28 bench** | Cost per passed task including compressor tokens, pass rate, `expand` recovery. |

### 3.4 Plan (LLM compression)

Phases 1–5 of `docs/llm-soft-compression.md` stand. Additions from this pass: the compressor
uses the `internal` lane (needs §2 L1–L2), summaries are cached per archive id for cache
stability, compression is asynchronous, and the guideline loop is part of Phase 2. Priority
stays below Batch/Flex and routing: soft compression spends tokens to save tokens.

## 4. Two more levers

### 4.1 Prompt-cache engineering

Measured hit rate on this workload is already 98.1 % (`research.md` §2, Cache row), so the
head-room on Claude Code is small (I-84). What is still missing:

- **Per-lane and per-host cache ledger.** Hit rate by lane and host (Codex, Gemini, OpenAI
  wires) instead of one number; the other hosts have not been measured.
- **Breakpoint injection where the host omits it.** On Anthropic wires, add `cache_control` on
  the stable prefix (system + tools) when the client sent none; off by default, measured.
- **Sticky upstream** (`[proxy.routing] sticky`, I-84) once a per-lane upstream exists (§2 L5).
- **Cache-aware rewrites.** Every proxy rewrite (live zone, P28 summaries, `tools_rewrite`)
  must produce byte-identical output on the next turn; add a replay test that sends the same
  conversation twice and asserts identical prefixes.

### 4.2 Model routing (D9)

`docs/model-routing.md` keeps routing of agent turns behind a bench with a quality gate
(I-101). Start where quality risk is lowest: rtok's own `internal` calls (compressor,
summaries) and the `bulk` lane. Agent-turn routing stays Later.

## 5. What else is missing for maximum token reduction

Every remaining lever, against `main` on 2026-10-04. "Off" means shipped but disabled by
default.

| Lever | rtok today | Gap / next step |
| --- | --- | --- |
| Proxy rewrites by default | `plugins.toon` and `plugins.compress` are enabled (T127), but `proxy.mode = "passthrough"` is the default, and the archive live zone, `toon` and `compress` act only in `compress` mode | Bench `compress` mode as the default proxy mode; flip it if it clears the bench (PR #562 was the first attempt at these defaults) |
| `tools_rewrite` (T59.5) | Off | *Measured* 6.2 % of session input on 2026-09-18 when Tool Search is off (`research.md` §2); turn on by default when `doctor` sees Tool Search disabled |
| Context editing (T51.2) | Off | Measure with archive ids kept; candidate default for Anthropic wires |
| Live-zone skill bodies (T61.2) | Off (`plugins.archive.skills`) | Gate on skill stats (T61.1), then default |
| Nested JSON / `data:` blobs (T51.1) | Off (`plugins.archive.live_blobs`) | Same: bench, then default |
| Deferred tool schemas | Not built | Short tool stubs, full schema on first use, for hosts without Tool Search (`research.md` §16.3 #2). *Measured* 2026-10-11 (T385.11, `research.md` §37): 99.7 % of Claude Code sessions already defer, both vendors do it server side; not built |
| Thinking / reasoning replay | Not built | Drop prior reasoning blocks the host re-sends where the provider allows it (`research.md` §16.3 #4); provider rules differ, needs per-wire tests. *Measured* 2026-10-11 (T385.11, `research.md` §37): replayed thinking is 11.5 % of input tokens, 3.4 % from earlier turns; dropping those nets 1.1 % to 1.6 % of input cost after cache re-creation; the earlier 0.03 % (T125) was a measurement error. Dropped; only safe route if revisited is the server-side `clear_thinking_20251015` edit |
| Lane-aware Flex / Batch | Not built | §2 (dollars, not tokens) |
| LLM soft compression (P28) | Not built | §3 |
| Semantic response cache (P31) | Off | Zero hits at threshold 0.99 (`research.md` §9.3); keep off for agents, consider for `bulk` |
| Tiered context L0/L1/L2 (P33) | Off (`plugins.archive.tiers`) | Native implementation T33.2; license note stands |
| Structured tool output | Not built | Ask tools for compact fields so formatters and `toon` win more often (`research.md` §16.3 #6) |
| Path / identifier dictionary | Not built | Low gain, risky (`research.md` §16.3 #8); A/B first |
| Session budget (I-55) | Not built | Process control rather than compression; cheap |
| Image gate | Parked | Images were 0.91 % of input on 2026-09-24 (T137), under the 5 % gate |
| Sub-agent handoff (I-46) | Parked | Agent+Task results 0.7 % of tool-result tokens (T59.6) |
| Foreign MCP results (I-44) | Parked | 15 K of 2.83 M tool-result tokens (`research.md` §9.3) |
| Cross-session read dedup | Not built | `read`/`guard` dedup is per session. *Measured* 0.25 % to 0.60 % of input (resident weighted; 0.42 % of the bytes read) on 2026-10-08, under the 1 % gate (`research.md` §36); re-measure if worktrees become shared |
| Output tokens | Modes (terse/yagni) only | Output is the code the model writes (`research.md` §9.3); the remaining lever is fewer writes, i.e. better first reads |

## 6. Consolidated plan

Order by expected dollars per effort; each line becomes a `plan.md` card with one `Check:`.

1. **Lane classifier + ledger tag** (§2 L0–L1). Check: lane tags per path; bytes unchanged.
2. **Per-lane policy** (§2 L2). Check: bulk/batch bodies byte-identical in `compress` mode.
3. **Defaults bench**: `compress` mode, `tools_rewrite`, context editing, `skills`,
   `live_blobs` (§5). Check: `rtok bench` cost per passed task per setting, dated in
   `research.md`.
4. **Batch observe + results usage** (§2 L3). Check: fixture results → `usage` rows.
5. **Flex on bulk/internal** (§2 L4). Check: prepare matrix and 429 policy tests.
6. **Per-lane cache ledger + replay byte-stability test** (§4.1). Check: per-lane hit rate in
   `stats`; replay test green.
7. **Isolation and per-lane upstream** (§2 L5). Check: agent latency under a bulk burst.
8. **P28 Phase 1 measurement** (§3). Check: dated `research.md` rows and the must-keep fixture.
9. **P28 Phase 2 compressor** on the `internal` lane, async, cached per archive id (§3.3).
   Check: Gate P28.
10. **Routing for internal/bulk calls** (§4.2). Check: dollars per lane before/after.
11. **Deferred tool schemas, thinking replay** (§5). Measured 2026-10-11 (T385.11, `research.md` §37): both dropped; nothing to build.
12. **`rtok batch` CLI and report breakdown** (§2 L6). Check: trycmd + report fixture.

## 7. Sources found

Existing research, consolidated here:

| Where | What | State |
| --- | --- | --- |
| `docs/batch-flex.md`, `docs/model-routing.md`, `docs/llm-soft-compression.md`, `docs/prompt-cache.md` (§ "Sticky routing vs Batch / Flex") | Batch/Flex pass-through semantics, routing (D9), P28 phases | On `main` |
| `roadmap.md` "Batch / Flex API pass — implementation plan" (S0–S6) and its task cards | Batch/Flex stages | On `main`; ids collide with `done.md` (see top) |
| `research.md` §16 (2026-09-21), §6, §9 | Savings beyond the shipped surface, ranking, gap review | On `main` |
| `ideas.md` I-21 (→ P28), I-23 (→ P31), I-84, I-101 | Compression, semantic cache, prompt cache, routers | On `main` |
| Branch `docs/batch-flex-pass`, PR #303 (closed draft), commits `bcb03778`…`97902765` (2026-09-24/25) | Origin of the Batch/Flex/routing/P28 docs, `next.md` (added `0284865a`, removed `02ec5f21`) | Content merged to `main` except `next.md` |
| Branch `docs/token-saving-next`, PRs #266 and #550 (closed drafts), commit `54e1c6d5` (2026-09-24) | `next.md` token-saving backlog | Not on `main`; verbatim in the appendix |
| Branch `t128-proxy-compress-default`, PR #562 (closed draft), commit `df2a13ba` (2026-09-21) | `toon` and `compress` on by default, rewrite fixes | Not merged; relevant to §5 row 1 |

Searched with nothing new: other `pyrlyn/rtok` and `pyrlyn/cox` branches, issues, local stashes
in `~/GitHub/listepo/apps/rtok`, and local Markdown notes under `~/GitHub/listepo`.

External sources (checked 2026-10-04): OpenAI "Flex processing" guide and pricing page;
Azure OpenAI "Flex processing" page; LLMLingua-2 (arXiv 2403.12968); ACON (arXiv 2510.00615,
ICML 2026).

## Appendix A. `next.md` (branch `docs/token-saving-next`, commit `54e1c6d5`), verbatim

```markdown
# Next: token saving

Plan / diffs only for token-economy work. Drawn from `research.md`,
`docs/comparison.md`, `docs/prompt-cache.md`, `ideas.md`, and `plan.md` /
`done.md`. Promote cards into `plan.md` before implementing.

## 1. Token saving

### Already in rtok

- **Terse / YAGNI modes** — byte-stable inject under the shared ~800-token budget (`docs/comparison.md`, modes).
- **Lean MCP tool descriptions** — ~143 desc tokens/turn vs multi-k competitor stacks (`README.md`, `docs/comparison.md`).
- **Archive + expand** — raw tool output archived before shorten; reversible via `rtok expand` (archive/cmd/read plugins).
- **Proxy compress / live zone** — cache-preserving rewrites shrink old tool results (`proxy`; `docs/comparison.md`).
- **Prompt-cache preserving** — documented hit rates and bust ledger (`docs/prompt-cache.md`); pricing via T49.1 `stats --price`.
- **Guard early-exit** — block duplicate native work without an LLM call (`guard` plugin).
- **Semantic response cache** — opt-in P31 (`plugins.proxy.semantic_cache`, `docs/config.md`; false-hit risk I-23).
- **Output / prose compress** — extractive + caveman-style benches (`plugins.compress`, `docs/comparison.md`).

### To add

- **Batch / Flex API path** — ~50% provider discount (and cache stacking where supported) for async or Flex-tier traffic; rtok today only rides sync agent calls.
- **Model routing (small → large)** — cheap model for classify/route/simple edits, large only on hard turns; not in plan today.
- **LLM soft compression (P28)** — Later / default-off LLMLingua-class shrink of archived context; ship only if a bench beats v0.1 lossless (`plan.md` P28, I-21).
```
