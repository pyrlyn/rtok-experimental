# Audit coverage — PR #986

Evidence baseline: `ea999781d7a48733e3a5c92e711cfad291c13536`, incorporating main through `085b49e9450de9326e7738dc441718e1bd667363`. Earlier findings retain their own evidence revisions in `plan.md` and `done.md`. Date: 2026-10-12.

This is a source-audit ledger, not a test-coverage report. The continuation closes the remaining module-family pass, including shared host helpers, CLI wrappers, SDK contracts, TUI rendering, SPA pages/graph rendering and packaging scripts. Large subsystems retain the bounded call-path depth recorded below; this does not mean every function, branch, supported platform or external integration has been verified. No percentage of lines reviewed or tests covered is inferred from a file inventory.

## Coverage by subsystem

“Production bodies” means source inspection excluding terminal test modules. “Targeted” means the named contracts/call paths were inspected; other paths in that subsystem must not be inferred to be covered. Prior audit IDs refer to the bounded records in `done.md`.

| Source family | Reviewed scope and depth | Finding/evidence records |
|---|---|---|
| Entry points, `src/bin`, `src/cli`, completions | Production CLI wrappers and argument forwarding; hook launcher; shell completion generation. Traced configuration selection, task/worktree/report dispatch and error propagation. | T530, T540, T585; T630 |
| Root helpers | Production bodies of agent view, lane, shell-agent, skill-path, stdio, TLS, process capture, info, man, memory status, progress, logfile, render and benchmark helpers. Targeted byte/range/diff, path identity, logging and task-ID invariants. | T585, T588, T602, T612; T638 |
| `src/hooks`, `crates/rtok-hook` | Targeted dispatch, resident protocol, framing/input bounds, fail-open paths, identity and measurement flow; hook transport crate production source. | T530, T610, T612 |
| `src/agents/<host>` | Production adapter bodies for all 25 hosts listed below, plus Claude legacy migration. Registration, config ownership, installed-state probes and install/remove call paths. | T625, T626, T636 |
| Shared `src/agents` | Production helper review: command construction, hook events, JSONC editing, cache/install/version selection, post-create hooks, restart, skills, MCP registration, outdated detection, proxy environment, usage, links and task lists. Junk planning/recheck/removal is a targeted path review, not every filesystem failure. | T568, T575; T627–T629 |
| `crates/rtok-agent-sdk` | Targeted backup ownership/pruning, JSON edit/write, atomic writes, symlink handling, module registration and plugin-link ownership. | T568; T569, T570, T625 |
| `src/config`, `crates/rtok-config` | Targeted layered extraction, validation and editing; production wrappers, typed provenance and selected-file reload. | T540, T585, T593; T594, T595, T630 |
| `src/doctor` and Doctor model/UI | Targeted installation probes, checklist/fix planning, selection refresh, immediate recheck and junk protected paths; TUI fix production body. | T575; T576, T577 |
| `src/tasks`, `src/worktree` | Targeted task disk/create/status/remote pagination, locking, runner claims, ID validation, worktree claims/removal and ownership. CLI and host fallback callers inspected. | T540, T600, T612, T614; T635 |
| `src/project`, `src/modes` | Project identity and local-origin normalization; Cargo/npm/pnpm/Go/Python reference resolution and graph following; mode loading and injection budgets. | T561, T585, T605 |
| `src/proxy`, `src/batch`, proxy plugin | Targeted routing, request shaping, cache/wire, Batch/Flex lifecycle, SSE event persistence and usage capture. Provider/network behavior is not a complete integration matrix. | T530, T582; T583, T584 |
| `src/mcp`, `crates/rtok-mcp` | Config read/edit/remove, operation/spec/status bodies and dispatcher/tool boundaries; agent/message/worktree tool bodies. | T530, T568, T603; T604, T627 |
| `src/plugin`, `crates/rtok-plugin-sdk`, WASM | Plugin registry/loader and wire/host/task-prefix contracts; SDK production bodies and demo guest; pending WASM dispatch is documented scope, not a new regression. | T568 |
| Archive, command, read and guard plugins | Targeted archive/blob/skill decisions, command filtering/quoting, bounded output, read search/tree/walk, guard keys/invalidation and skill exemptions. | T530, T554, T568, T588; T555–T559, T590 |
| Compression, TOON, JSON-tree plugins | Targeted folding/reference identity, decoder oracle, prose suffixes/fences and compression ladder combinations. | T554, T603, T607; T560, T604, T608, T609 |
| Inject, checkpoint and docs plugins | Targeted budgets/byte stability, mode reads, checkpoint privacy/lifecycle, rustdoc fetch/index and serialization boundaries. | T547, T605; T606 |
| Memory plugin | Targeted pack/sync/files, recall/fusion, observation synthesis/privacy and storage callers. | T561; T562–T567, T617, T618 |
| Graph plugin | Targeted walking/index/watch/rank, references/scope/fan-out, export/redaction, co-change, health scoring and LSP framing/document synchronization. This is not every graph query or grammar. | T540, T561, T568; T571, T620, T621 |
| `src/measure`, measure plugin | Targeted transcript accounting/cache discovery and host parsers; shared usage providers, image and dictionary helpers inspected. | T578; T579 |
| `src/otel`, Store OTel, check tooling | Targeted export cursor, cumulative metric/encoding flow, trace extraction and validation scripts. | T591; T592, T624 |
| `src/store`, `crates/rtok-store` | Targeted archive/call/measurement persistence, notes/observations, sanitation, registry/parent identity, messages, claims, pending worktrees, retention and graph events. Real migrations/public APIs used for selected probes; not every query or migration transition. | T530, T598, T612, T614; T599, T613, T615, T617, T618 |
| `src/model`, `src/report` | Model page construction and task refresh; report ledgers/advice/AI serialization; HTML and PDF production bodies and layout arithmetic. | T578, T585; T580, T581, T587, T630, T632, T633 |
| `src/tui`, `src/ui` | Production state/event routing, health/live calls, views/theme, panels/projects/compare/export and Doctor/junk helpers; selected rendering and input probes. | T572; T619, T637, T639 |
| `src/web` | Targeted request protocol, task/config/Doctor actions, snapshot generation, shared calls/live polling/replay and SPA asset serving. | T572; T573, T574 |
| `web/src` | Source review of API/reply correlation, shell/navigation/palette, pages and reusable tables/charts; graph overview/drill/compare, 2D/3D scene/layout/camera/selection and live feed. Generated API declarations serve as contracts, not separately audited generated logic. | T575; T576, T623 |
| `src/demon`, `crates/rtok-sys`, logging | Targeted service lifecycle/PID identity, process boundaries and log following; sys/log crate production source. Platform-specific execution remains limited. | T588, T596; T589, T597 |
| Host extension packages in `plugins/` | Production scripts/TypeScript and JSON manifests, including Pi/OMP mixed results, OpenCode filters, Claude worktree fallback and host hook commands. Host applications were not launched. | T622, T635 |
| Build, release, package and repository tooling | `build.rs`, Cargo/profile/dist configuration, workflow/build-setup source, release/publish/version/share/check scripts, npm/PyPI wrappers, web generation/Vite configuration, brand/asset generation. Review of generated release YAML is structural, not an executed release. | T624, T631, T634 |

Host adapter set: `aider`, `antigravity`, `claude`, `cline`, `codebuddy`, `codewhale`, `codex`, `commandcode`, `copilot`, `cursor`, `devin`, `gemini`, `grok`, `kilo`, `kimi`, `mimo`, `omp`, `opencode`, `pi`, `qwen`, `roo`, `vscode`, `windsurf`, `zcode`, `zed`.

## Executed evidence for the final continuation

Each finding in `plan.md` contains the input, observed failure, relevant control, source link and ordered correction plan. These records are durable; temporary probe directories are not required to understand the findings.

| Findings | Execution | Deliberate boundary |
|---|---|---|
| T617–T618 | Real Store migrations/public APIs plus production recall/privacy helpers | In-memory synthetic data; no full hook or host |
| T619 | Actual Panel with Ratatui TestBackend | No interactive terminal |
| T620–T621 | Production LSP methods with a synthetic subprocess peer | No real language server |
| T622–T623 | Focused Vitest probes of Pi code and extracted web API implementation | Mocked transport/clocks; no browser |
| T624 | Actual trace-ID extraction pipeline with synthetic response | No Docker/Jaeger service |
| T625–T626 | Production host helpers and real SDK writer where applicable | Temporary files; no live host |
| T627–T628, T636 | Verbatim JSONC/cache/migration functions | Caller writes/host activation source-traced |
| T629 | Actual restart orchestration with recording process seams | No real applications quit/reopened |
| T630 | Real config crate and exact configured predicate | Page/browser refresh source-traced |
| T631 | Actual build script in a minimal Cargo package and real linked Git worktree | No full rtok release build |
| T632–T633 | Source constants and executable layout/page-number arithmetic | No rendered PDF visual inspection |
| T634 | Actual publisher script with fake git/cargo/twine | No upload, authentication or package validation |
| T635 | Actual shell field extractor with JSON path fixtures | No real worktree creation/removal |
| T637 | Actual key-routing prefix and Config handler | Unrelated methods stubbed; Calls path source-traced |
| T638 | Actual bounded-run helper with synthetic shell descendants | macOS only; no real agent benchmark |
| T639 | Actual Calls table with Ratatui 0.30.2 TestBackend | Fixture data/theme; full navigation source-traced |

## Exclusions and follow-up boundaries

- The tracked-path inventories are navigation aids. A file appearing in a list, a source citation, a grep result or a bundle does not establish that its full body was reviewed. No inventory count is presented as an audit completion percentage.
- Tests, fixtures, stories, snapshots, generated schemas/bindings, lockfiles, vendored dependencies and binary/media assets are outside blanket production-source coverage. Relevant tests and generated contracts were inspected for specific findings. T560 explicitly concerns a test oracle.
- No full Cargo suite, browser/Playwright session, real host/agent integration, release upload, Docker/Jaeger integration, Windows/Linux execution matrix or exhaustive concurrent filesystem/database failure matrix was run in this continuation.
- Existing T10.9 already states the foreign proxy-URL preservation contract. OpenCode's current unconditional `strip` remains a revalidation of that contract, not a new counted finding. The release build-setup template also omits the brand install present in the checked-in release workflow; no regenerated release failure is claimed without executing that path.
- Findings are fixes to implement, not fixes delivered by this PR. No saving/performance claim follows from this audit. Future code changes require their own regression checks; this ledger is pinned to the evidence revision above.
