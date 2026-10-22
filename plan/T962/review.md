# T962. Project audit

Reviewed 2026-10-22, starting at `b7183a941` (`origin/main`). The user's checkout
`feat/new-ideas` at `41f823081` was left untouched, including its untracked files.
It is behind main; older findings were compared against main before proposing fixes.

## Coverage and limits

Workspace inventory: 11 Cargo packages, 147 Cargo targets; 432 files under `src`,
138 under `crates`, 62 under `plugins`, 302 under `web`, 25 under `tools`, 12 under
`fuzz`, and 595 under `tests` (file inventory, not lines manually reviewed).
Inspection focused on HTTP MCP, dashboard asset/auth handlers, proxy request/cache
boundaries, filesystem containment, graph text reads, configuration trust, WASM
allocation, logging and existing audits. Static scans covered process/unsafe sites
and frontend HTML sinks. Commands and results are in [validation.md](validation.md).

This is a repository-wide scan with focused source review, not an assertion that every
line is correct. Windows runtime behavior, fuzz campaigns, production traffic and
real agent applications were not exercised. No benchmark or token-saving claim is made.

## Confirmed code defects

| Finding | Trigger and impact | Fix and proof |
| --- | --- | --- |
| SPA directory escape | With `RTOK_WEB_DIST`, a symlink inside the served directory could expose a readable file outside it through the unauthenticated asset route. Requires a symlink in the configured asset tree; embedded assets are unaffected. | `src/web/spa.rs`: retain lexical traversal checks, then reuse `rtok_config::root::contain`. Regression covers file links, directory links, compressed variants and valid internal links. |
| WASM allocation exhaustion (A12) | Fuel alone did not bound initial memory/table allocation or growth by a loaded guest. | `src/plugins/wasm.rs`: install wasmi's existing `StoreLimits` before instantiation/start; 64 MiB linear memory, 65,536 table elements, one memory/table/instance per guest. Regression includes valid guests, oversized initial allocations and denied growth. |
| Text graph secret disclosure | An operator enabling graph tools over HTTP with the text backend could receive definitions from `.env` or key files. The graph's walker bypassed `search`'s secret filter. | `src/plugins/read/search.rs`: enforce the existing secret policy in shared `text_files`, before reading. Keep stdio behavior and template files unchanged. Unit coverage plus an HTTP test check a public definition remains available and a credential definition does not. |

The SPA fix resolves existing symlinks; it is not descriptor-relative protection against
a local writer racing directory replacement between validation and open. WASM limits
bound guest linear memory and tables, not all host-side allocations or total memory across
an arbitrary number of plugins. Guests needing more resources now receive allocation
failure; `memory.grow`/`table.grow` keep their normal failure return value.

## Reuse and refactoring

Reused `rtok_config::root::contain`, which already centralizes symlink-aware containment
and uses `fs-kit` path handling. Reused `roots::is_secret` and the existing shared text
reader instead of maintaining a second graph credential list. Extracted a policy-taking
reader helper for deterministic tests without changing process-wide flags. Reused wasmi
2.0.0's limiter rather than adding a sandbox library. No new runtime dependency or public
interface was introduced. The limiter API is documented by [wasmi](https://docs.rs/wasmi/2.0.0/wasmi/struct.StoreLimitsBuilder.html)
(checked 2026-10-22).

Removed the unmaintained `rustls-pemfile` wrapper. `src/tls.rs` now uses
`rustls::pki_types::CertificateDer::pem_reader_iter` from the already-linked Rustls stack.
Both readers filter non-certificate PEM sections and report malformed certificate data.
The existing TLS tests plus a mixed-bundle regression passed with the old parser before
the replacement; post-change results are recorded in validation. File-open and parse errors
retain the bundle path, and certificate-free bundles still fail closed.

Before running browser integration tests, the E2E child environment was made to set
`RTOK_HOST_SANDBOX=1`. Its temporary HOME and restricted PATH alone did not redirect
absolute desktop application paths; `src/agents/mod.rs::expand_app` requires this existing
switch to confine them to the fixture home. This changes test isolation, not product behavior.

## Dependency audit: approved security updates

`cargo-deny` 0.20.2 found these TLS advisories in the resolved dependency tree.
The creator approved these three minimum patched versions on 2026-10-22. The direct
`webpki-roots` requirement now excludes older releases; resolved transitive versions are
updated precisely in the lockfile. Validation records the post-update advisory result.

| Package | Resolved | Minimum fixed | Primary source, checked 2026-10-22 |
| --- | --- | --- | --- |
| rustls-webpki | 0.103.15 | 0.103.16 | [RUSTSEC-2026-0382](https://rustsec.org/advisories/RUSTSEC-2026-0382.html) |
| webpki-root-certs | 1.0.6 | 1.0.10 | [RUSTSEC-2026-0383](https://rustsec.org/advisories/RUSTSEC-2026-0383.html) |
| webpki-roots | 1.0.9 | 1.0.10 | [RUSTSEC-2026-0381](https://rustsec.org/advisories/RUSTSEC-2026-0381.html) |

These advisories concern trust in distrusted emSign intermediate certificates. A green
code test suite does not resolve the dependency findings.

The initial scan also reported unmaintained `bincode 1.3.3`
([RUSTSEC-2025-0141](https://rustsec.org/advisories/RUSTSEC-2025-0141.html), via printpdf/azul-layout)
and `rustls-pemfile 2.2.0`
([RUSTSEC-2025-0134](https://rustsec.org/advisories/RUSTSEC-2025-0134.html)), without a safe upgrade
listed. The latter is removed by the refactoring above. `bincode` remains a maintenance finding;
replacing the PDF dependency chain requires a separate compatibility review. This maintenance
advisory is distinct from the three patched TLS vulnerabilities.

## Web dependency audit

The initial full `web` npm audit reported eight affected package entries (four moderate,
two high, two critical), all in development tooling. `npm audit --omit=dev` reported zero;
the separate `brand` lockfile also reported zero. Development servers are still meaningful
attack surfaces: see [Storybook origin validation](https://github.com/advisories/GHSA-f2ph-cmg5-wxc8),
[Vitest file restrictions](https://github.com/advisories/GHSA-p458-7748-mcp3), and
[source-map-js resource exhaustion](https://github.com/advisories/GHSA-68fv-2mgg-jv7q)
(checked 2026-10-22). Audit commands and log locations are recorded in validation.

The creator separately approved aligned Storybook packages 10.6.3, aligned Vitest
packages 5.0.5, Vite 8.3.5, and source-map-js 1.2.2 through its existing dependency range.
Both full and production-only npm audits now report zero vulnerabilities. A normal `npm ci`
validated the updated lockfile. Required transitive updates include Vitest's internal packages
and Vite's Rolldown/PostCSS dependencies; unrelated Storybook icons and magic-string updates
were reverted. Six nested Tailwind WASI entries are npm metadata for an unchanged bundled
package. Web unit tests (503), Storybook tests (260) and browser E2E tests (45) passed, as did
type checking and both production builds. Detailed results are recorded in validation.

## Existing findings and other branches

Prior audit A1–A9 already have fixes on main. T960 owns HTTP archive capabilities and
log rotation (A11/A13); T961 owns project configuration trust (A10). Their worktrees and
changes were not modified or merged. They are not treated as fixed in this branch merely
because another branch contains a patch. The existing audit in `plan/README.md` also tracks
legacy file permissions (A14), proxy buffering (A15), and OTLP content export defaults (A16).

## Feature proposals (not implemented)

1. **Security exposure report in `rtok doctor`.** Consolidate the existing exposure idea:
   show network binds, auth requirements, project trust, archive scope, file permissions
   and content-export settings with machine-readable status. Prioritize this because the
   existing audit spans separate configuration and transport boundaries.
2. **HTTP tool capability report.** Show which enabled tools read files, archives or graph
   data, and which restrictions each path enforces. Back it with one fixture matrix across
   `read`, `search`, text graph and `expand`; the graph bypass shows why per-tool tests alone
   miss shared policy gaps.
3. **Bounded cache/retention controls with occupancy reporting.** Continue existing idea
   I-124: expose bytes/entries and enforce a measured budget in addition to TTL. Establish
   representative workloads first; do not invent a token-saving estimate or silently change
   retention defaults.
