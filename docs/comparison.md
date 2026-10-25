# rtok compared with the alternatives

Every tool named here is real, useful, and was measured before rtok was written. The raw
survey — 19 repositories, GitHub API metadata, 27 vendor claims fact-checked, 17 local
session transcripts — is
[`research.md`](https://github.com/pyrlyn/rtok/blob/main/research.md) §4–§7. This page is the reading of
that survey: what each category does well, what rtok does differently, and which of rtok's
claims are backed by a row in a database rather than by a README.

Survey date 2026-09-01, refreshed 2026-09-04/09/10 (modes bench). Star counts and versions move; the
mechanisms do not.

## 1. The short version

Ten tools that each shrink one slice of the context, stacked on one machine, produce a
stack that nobody measures end to end. The author's own machine, before rtok:
**88 hook entries across 16 events, nine MCP servers, ~8 600 tool-description tokens paid on
every single turn, and two chained proxies.** Every one of those tools reported a saving.
The bill did not move, because no meter covered the whole path.

rtok is the opposite bet: **one binary, one ledger, one number.** Ten methods live as
plugins behind one trait, they share one SQLite file, and a saving that is not a
`measurements` row does not exist — including rtok's own, which is why the A/B table in the
README is currently all zeros instead of a marketing figure.

## 1a. rtok vs ponytail and other prompt-level tools

[ponytail](https://ponytail.dev) is a prompt-rules pack: it tells the agent to write less code.
That is a real lever, and it works on the output side. rtok mostly cuts the input side — what
the model reads each turn — and also ships the same idea as an optional mode, so choosing rtok
does not mean giving up the minimal-code rules.

**Why rtok**

- Input-side reduction through hooks, the MCP server and the API proxy, as plugins behind one
  trait (see the plugin table in the [README](../README.md)); a prompt pack touches none of these.
- The ponytail idea is built in: `modes/yagni.md` is a minimal-code ladder, injected by the
  inject hook (alias `pony`, `src/plugins/inject/mod.rs`) inside the shared 800-token inject
  budget, so it is byte-stable and counted with every other injection.
- `rtok stats` reports what was saved and, with per-model prices in `[stats.prices]`, what it
  costs in USD (`src/cli/stats.rs`); `rtok dashboard` shows the same store in a local UI.
- `rtok doctor --instructions` audits your instruction files, and `rtok doctor` also audits what the
  installed skills cost to list on every turn (`src/cli/doctor.rs`, `src/doctor.rs`).
- `rtok task` keeps the plan as tasks the agent can claim instead of loose TODO prose
  ([commands](commands.md)).
- `rtok graph review` turns a git diff into a deterministic reading list, with no model call
  ([review](review.md)).

- `rtok review` hands a reviewer model the diff plus signature digests
  ([review-context](review-context.md)).

**Benchmarks.** ponytail is [not measurable in this suite](../bench/competitors/ponytail/README.md):
it is prompt-only and needs a live model. A side-by-side reading is in
[rtok-vs-ponytail](comparison/rtok-vs-ponytail.md); the `mode_bench` fixture results for the
yagni mode are in section 2, "Prompt-level", and the survey is in [research.md](../research.md).
There are no live end-to-end numbers comparing rtok and ponytail.
ponytail 5 reports −53 % lines of code, −45 % output tokens and −26 % cost from its own
bench ([ponytail.dev](https://ponytail.dev)); these numbers are unverified here.

## 2. What each category does well

### Command-output filters — rtk

~80 hand-written filters that shrink `git status`, `cargo test`, `npm` and friends, wired
into `PreToolUse`. The idea is correct and rtok copies it: Bash output was 35 % of the
author's tool-result tokens (1.00 M of 2.83 M).

Two problems. It drops lines with no way to get them back, and the only independent
measurement of it — JetBrains — found it cost **+7.6 % to 0 %** on agentic work against a
claimed 60–90 %. rtok's `cmd` plugin keeps the filtering (per-family TOML rules plus
formatters for `cargo`, `git`, test runners, `tree` and the container tables) and adds
the two things that were missing: the raw output goes to `~/.rtok/archive/<id>` before
anything is cut, and the before/after byte counts go to `measurements`.

### Command-output filters — sqz

The same idea in Rust (github.com/ojuschugh1/sqz; ELv2; 625 stars on 2026-09-18), with 45+
formatters, a content-hash dedup that returns a 13-token `§ref:HASH§` for bytes already seen
in the session, JSON null-stripping, column-padding collapse and a safe mode that passes
stack traces through whole. Lossless like rtok (`sqz expand`). Its numbers are self-reported
(README: 24.7 % average over 3,003 compressions). Against rtok: the content-hash dedup, JSON
and table passes and the trace guard are gaps (`research.md` §11 → T65.1–T65.4); rtok keys
its dedup by call input (`guard`), measures every cut into one ledger, and runs the same
plugin set over the proxy and MCP surfaces, which sqz does not have.

### Read, search and shell MCP servers — lean-ctx, token-optimizer

Bounded reads, outline modes, deduplicated re-reads, compact directory trees. Also correct,
also copied: `read` ships 3 MCP tools with 4 read modes and a `read_cache` table.

The cost these tools do not count is their own presence. lean-ctx injects ~3.1 K tokens per
turn of banner and instruction text; token-optimizer installs 27 Python hooks, and a full
event chain on this machine fires up to ~30 subprocesses. Both are paid on every turn,
cached or not, whether or not the agent reads a file.
rtok's whole injection budget is **800 tokens per turn**, byte-stable so it never busts the
prompt cache.

### Compressing proxies — headroom, caveman, bifrost

The proxy is the only surface that can rewrite what is already in the conversation, and
headroom's cache-aligned live zone is the sharpest idea in the field: shrink the tail,
leave the cached prefix byte-identical. rtok's `archive` plugin implements the same
invariant, and `proxy` captures provider `usage` as ground truth.

Measured against their claims: headroom claims up to 95 %, the author's own meter showed
11.3 % over 30 days; caveman claims 65 %, JetBrains measured 8.5 % on agentic work, and its
issue #112 corrupts inline code. bifrost's semantic cache is a category error for coding
agents — agent contexts never repeat, so a cache hit is a wrong answer.

### Code graphs — serena, codegraph, code-review-graph, codebase-memory-mcp, graphify

Ask "who calls this" instead of grepping and reading five files. The best of them are very
good: serena's LSP backend is the most precise symbol resolution available, and
code-review-graph's impact analysis is genuinely clever.

What they cost is the tool list. Measured with `rtok doctor` on 2026-09-09 — every MCP
server on the author's machine, not only the code graphs:

| MCP server | tools | description tokens, every turn |
|---|---:|---:|
| code-review-graph | 30 | ~2 295 |
| engram | 18 | ~1 865 |
| mobile | 32 | ~1 507 |
| serena | 22 | ~1 494 |
| lean-ctx | 12 | ~697 |
| caveman | 5 | ~485 |
| headroom | 3 | ~161 |
| **rtok** (read + memory + graph + expand) | **12** | **~223** |

The rtok row is re-measured with `rtok doctor` on 2026-09-18, after `explore` joined
the graph surface; the other rows are the 2026-09-09 probe.

That column is not a one-off: it is re-sent on every request of every session. Two of those
servers together cost more per turn than rtok's entire injection budget. `graph` answers
`symbol` / `callers` / `impact` / `outline` / `explore` / `graph_diff` / `graph_export` in 165 description tokens
(`cargo nextest run -p rtok graph_surface`, 2026-10-10).

The trade is real, though, and §5 states it: serena resolves references that rtok's
tree-sitter tags index misses.

A with/without graph bench now exists (`rtok bench --suite graph`, `bench/graph.toml`,
T68.9). Dry-run on 2026-09-18: `rtok bench --suite graph --runs 1 --dry-run` (12 questions
× 3 repos × mcp/native, no API spend). Live numbers are not in yet; vendor with/without
claims wait for §5 beside rtok's own row.

### Memory — claude-mem, engram, mem0

Notes that survive compaction. claude-mem extracts them with an LLM, which costs tokens to
build the thing that saves tokens; mem0 wants Docker and a vector database. rtok's `memory`
is agent-written notes in SQLite FTS5 with progressive disclosure — titles, then ids, then
bodies, never bodies at `SessionStart` — for 3 tools and no model calls. graymatter is the closest design — one Go binary, MCP plus
Claude Code hooks, no LLM required — and adds tombstones, decay and pinning. rtok's own plant-and-recall bench is `tests/memory_bench.rs` (2026-09-18, `research.md` §14): FTS5 and P29 hybrid 20/20 at 1/10/30/100 sessions, superseded returned 0, SessionStart 100 bytes vs 371 866 bytes of full live-body injection at N=100. Never graymatter's 83 %.

engram was re-read feature by feature on 2026-09-18 (`research.md` §13). Two of its ideas
fit rtok's zero-LLM lane and were adopted: topic keys — one row per evolving topic, which
rtok does as an upsert on project + kind + title with no new column — and a portable
export that its own `import` reads. Its relation judging (`mem_judge`, `mem_compare`,
`mem_review`), the mandatory end-of-session summary, and passive capture of the model's
own `## Key Learnings` were not: each spends model output tokens on bookkeeping, and the
18-tool description column above is the price of carrying them on every turn.

### Prompt-level — ponytail, terse modes

The cheapest lever in the field, because output tokens are 20 % of the bill (research.md
§9.3, 2026-09-17) on standard models and **39 % on Fable/Mythos 5.1**, same source, where
cache reads cost 0.025×. ponytail's own bench
claims −54 % LOC and −22 % tokens; no independent check exists. caveman claims 65 % output
shrink; JetBrains measured **8.5 %** on agentic work, and issue #112 has corrupted inline
code.

rtok ships the same *ideas* as optional prompt modes
(`rtok agents install claude --mode terse,yagni`, aliases `cave`/`pony`) plus native helpers in
`src/modes/` — not a wrap of either tool (D6). Modes are markdown data under `modes/`
injected once per session inside the shared **800-token** budget (D7); measured mode sizes
2026-09-10: `terse` **162** est. tokens, `yagni` **145** (cap 250 each).

Structured A/B against honest weak baselines (`cargo test --test mode_bench -- --nocapture`,
2026-09-10; full write-up in [`research.md`](../research.md) modes subsection):

| Slice | Baseline | rtok | Why better |
|-------|----------|------|------------|
| Prose compress (17 fixtures) | weak caveman-lite: **11.7 %** chars, **50** tok saved | Full: **34.9 %** chars, **147** tok saved | Fence bodies byte-identical; negations (`not`/`never`/`no`) kept — caveman #112 class of bug is a hard fail here |
| YAGNI ladder (14 fixtures) | naive always-Minimum: **4/14 (29 %)** | typed ladder: **14/14 (100 %)** | Prompt-only “be lazy” collapses to Minimum; the ladder can Skip / Reuse / Stdlib / Native first |
| Injection cost | caveman MCP ~485 desc tokens + separate prompt; ponytail another file | modes share inject budget; aliases resolve to the same builtins | One binary, byte-stable injection, no Go/JS on the path |

These are **fixture benches**, not a live bill delta — same honesty rule as the README A/B
zeros. They show the native path beats the weak baselines we can re-run in CI; they do not
replace an end-to-end `rtok bench` with a provider.

### Formats — TOON, LLMLingua-2

TOON's −42.6 % on tabular JSON is a vendor bench that survived fact-checking, so rtok has a
`toon` plugin — **on by default**, and it rewrites a block only when TOON is smaller, because most agent payloads are not tables. LLMLingua-2
prunes tokens with a small model; on code that is a quality risk with no measured upside,
so it is v0.2+ at the earliest.

### The platform itself — prompt caching, context editing, auto-compact

Free, first-party, and the thing every tool above is really competing with. The measured
hit rate and the per-surface story: [prompt-cache.md](prompt-cache.md). The
correct move is to align with it, not fight it: rtok's byte-stable injection and
cache-preserving proxy rewrites exist for exactly this reason.

## 3. Where rtok is different

| | The field | rtok |
|---|---|---|
| Processes per tool call | up to ~30 subprocesses per event chain, several Python | one Rust process, p95 **8.25 ms** (research.md §2, Gate P17 serialized run 2026-09-09) |
| Hook entries | 88 across 16 events (10 tools) | **8** across 7 events, one binary |
| MCP description tokens/turn | ~8 600 across 10 servers | **~143** across 11 tools |
| Injection per turn | lean-ctx 3.1 K + engram + claude-mem + nudges | one **800-token** budget, byte-stable |
| Reversibility | partial (headroom retrieve, token-optimizer expand) | every rewrite has `rtok expand <id>` |
| Measurement | 5 incompatible meters, none of them the bill | one ledger: `measurements` + provider `usage` |
| Failure mode | varies; some block the tool call | fail open: exit 0, unmodified input, ≤ 10 ms |
| Runtime | Python, Node, Go, Docker, daemons | one static binary, 18.9 MiB, no daemon (research.md, Gate P8d (5), 2026-09-09) |
| Third-party code on the hot path | adapters and wrappers | none (decision D6) |
| Observability | per-tool dashboards | OTLP to Jaeger / Grafana / SigNoz / Maple |

## 4. The advantages, with their evidence

1. **The tool list is context too.** 11 tools for ~143 description tokens, against 30 tools
   for ~2 295 in one competing server. Source: `rtok doctor`, 2026-09-09.
2. **Nothing is lost.** `cmd`, `read` and `archive` write the raw payload to
   `~/.rtok/archive/<id>` *before* shortening it; `rtok expand <id>` returns it, with
   `--lines` and `--grep`. rtk drops lines outright.
3. **A saving is a row or it is nothing.** `Ctx::record(&Measurement)` is the only way for a
   plugin to claim one, and `rtok stats --plugin <id>` prints them. The proxy writes
   provider-reported `usage`, which is the actual bill rather than a chars/4 estimate.
4. **It admits what it has not proven.** The committed A/B bench ran offline and reports
   zeros; the README says so. The graph with/without suite (`rtok bench --suite graph
   --dry-run`, 2026-09-18) prints a schedule and no dollars. Every vendor number in §2 that
   was independently checked came in 5–10× below its claim.
5. **It cannot break your agent.** A hook exits 0 with unmodified input on any error or
   panic, inside 10 ms. A half-installed or crashing rtok is a no-op, not an outage.
6. **It does not fight the cache.** Injections are budgeted and byte-stable across turns;
   the proxy never rewrites system instructions, tool definitions, or the newest tool
   results. At the 97.5 % hit rate measured in [`docs/prompt-cache.md`](prompt-cache.md)
   (2026-09-18), busting the prefix costs more than any filter saves.
7. **Three surfaces, because one is not enough.** `PostToolUse` cannot modify a tool result
   — verified against the hook docs — so shrinking what is already in context needs the
   proxy, and replacing a tool needs MCP. Single-surface tools have a ceiling that is a
   property of the host, not of their code.
8. **One config, with provenance.** Every flag is a config key; `rtok config show --sources`
   names the layer each value came from. No tool in the survey can answer that question.
9. **Reversible install.** `rtok agents install claude --dry-run` prints the exact edits, the real run
   backs up the settings file before writing it, and `rtok agents uninstall claude` takes the hooks,
   the MCP registration and the proxy variable back out (foreign entries stay).
10. **Your ledger, in your observability stack.** `rtok otel flush` projects calls, logs and
    metrics as OTLP/HTTP JSON — verified against Jaeger 2.11 and Grafana `otel-lgtm`, and
    against an independent validator that re-implements the spec. Cost when an endpoint is
    configured: +0.81 ms p95 on the hook path (research.md, Gate P16, 2026-09-04).

## 5. Where rtok is behind

Stated plainly, because §4 is only worth reading if this section exists.

- **No live end-to-end cost win has been demonstrated.** The A/B harness and the graph
  with/without suite (`rtok bench --suite graph`) run; neither has been run against a live
  API bill (T68.9 live clause is open). Until one is, rtok's own claim is "measurable", not
  "cheaper", and vendor 88 % / 62 % figures stay next to an empty rtok row.
- **`graph` finds every definition and misses most references.** Definition recall and
  precision are 1.000 over a 30-symbol hand-labelled set; reference recall is **0.351**,
  because the tree-sitter tags query does not capture type positions or macro bodies.
  serena's LSP backend is more precise here. An LSP backend for `graph` is v0.2.
- **No LLM compression, no embeddings, no semantic search.** claude-mem and mem0 do those
  today. In rtok they are v0.2+ and gated on beating the lossless path in a bench.
- **`memory` recall is newest-five titles, with no recency ranking.** Lifecycle (retire /
  supersede / pin) landed in T69.1. The plant-and-recall bench (T69.3, `research.md` §14)
  is 20/20 FTS5 and P29 hybrid at N=100 against this generator; `half_life_days` ranking
  did not ship (T69.2), so that row is N/A. Never cite graymatter's 83 % as rtok's.
- **Smaller filter library than rtk and sqz.** 28 rule families in `rules/default.toml` plus
  ten per-family formatters, against rtk's ~80 filters and sqz's 45+ (`research.md` §11;
  the T65.1 content-hash dedup, T65.2 JSON compact and T65.3 column collapse are in —
  the library itself is still the smallest of the three).
- **No Batch observe, Flex prepare, or model router yet.** Provider Batch paths already pass through the proxy fallback; Flex `service_tier` injection and D9 routing are planned (`docs/batch-flex.md`). bifrost, Portkey and LiteLLM ship routers today.
- **Younger, and a single maintainer.** Several tools in §2 have five-figure star counts and
  years of edge cases baked in. rtok is at v0.0.1.
- **`toon` is opt-in** because it lost its gate on this machine.
  LadybugDB and Grafeo were measured then **removed** (P39, 2026-09-12): Ladybug won depth-4
  impact by 77× but missed the hook ≤10 ms bar; Grafeo abandoned after warm `impact(2)` ~22 000×
  slower than SQLite. The symbol index is SQLite only.


### Batch, Flex, and routing gateways

rtok is a token-reduction middleware with a local proxy; bifrost, Portkey, and LiteLLM are
primarily **routing / gateway** products. Batch (async provider APIs), Flex (`service_tier`),
and model routing are distinct levers — rtok documents them in [`batch-flex.md`](batch-flex.md)
and does **not** auto-convert sync agent turns into Batch jobs.

| Capability | rtok | bifrost | Portkey | LiteLLM |
|------------|------|---------|---------|---------|
| Provider **Batch** pass-through | Fallback forward today; observe/usage **planned** | Gateway routing; Batch support varies by provider plugin | Virtual keys + provider Batch where configured | Provider pass-through / mapped Batch endpoints |
| **Flex** / `service_tier` | Client-set forwarded; `prepare` inject **planned** | Depends on provider config | Can set tier via config/hooks | Can set `service_tier` in request params |
| **Model routing** (cheap→expensive) | D9 **planned**; off by default | First-class router | First-class router / fallbacks | First-class model alias + router |
| Prompt-cache **sticky** upstream | Documented intent (I-84); flag **planned** | Load-balancer / deployment choice | Deployment / gateway affinity | Deployment / load-balance settings |
| Sync→Batch auto-convert | **Never** (by design) | Not the coding-agent default | Possible via workflows; not rtok's model | Possible via custom callbacks |
| Semantic response cache | Opt-in P31, default off (agent false-hit risk) | Shipping feature (false-hits on agents) | Available | Available |

Where rtok stays different: measurement into one SQLite ledger, cache-byte-stable compress,
and hooks/MCP that never see the LLM HTTP path. Where it is behind: bifrost/Portkey/LiteLLM
already ship production routers; rtok's Batch observe, Flex prepare, and D9 routing are
still backlog items in `ideas.md` (I-100/I-101).

## 6. Check any of it yourself

```bash
rtok doctor                       # your hooks, MCP servers and their description-token cost
rtok plugins                      # what is enabled, and on which surface
rtok stats --since 7d             # transcript estimates plus provider usage
rtok stats --save-baseline before # then change something, then --compare before
rtok bench --dry-run              # the A/B schedule that has to beat the baseline
rtok expand <id>                  # the original of anything rtok shortened
```

`rtok doctor` reads the settings of whatever is already installed, so it will price your
current stack before you install anything of rtok's.

## 7. When not to use rtok

- You want the most precise symbol resolution available and do not mind 22 tools of
  description on every turn — use serena.
- You want a big, mature filter library for shell output and can live with lossy cuts — use
  rtk.
- You need LLM-extracted memory or vector search today — use claude-mem or mem0.
- You are on a plan where tokens are not the constraint. Then none of this pays, including
  rtok.
