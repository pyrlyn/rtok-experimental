# T962 validation

## Reproduced before fixes

- `mise exec -- cargo nextest run -p rtok --lib -E 'test(web::spa::tests::directory_assets_refuse_symlink_escapes_but_keep_internal_links)'`
  - 1 failed, 2,738 filtered. Panic: `src/web/spa.rs:373:13: served leak.js`.
- `mise exec -- cargo nextest run -p rtok --lib --all-features -E 'test(guest_allocations_are_bounded_before_start)'`
  - 1 passed, 4 failed, 2,742 skipped. Cases 2–5: `assertion left == right failed`; oversized initial memory and table were accepted, and both `memory.grow` and `table.grow` returned success instead of `-1`.
- `mise exec -- cargo nextest run -p rtok --lib --all-features -E 'test(text_file_consumers_share_the_http_secret_policy)'`
  - 1 failed, 2,747 filtered. At `src/plugins/read/search.rs:218`, expected `[".env.example", "public.txt"]`, got `[".env", ".env.example", "public.txt", "server.key"]`.

## After fixes

- `mise exec -- cargo nextest run -p rtok --lib -E 'test(web::spa::tests)'`
  - 6 passed, 2,734 filtered, including the symlink boundary regression.
- `mise exec -- cargo fmt --all`
  - Passed (no output).

## Baseline gate

- `mise exec -- just full-check` was attempted before the SPA/WASM/read fixes. It stopped during the `rtok` library-test compile because the first version of the SPA test used an undeclared direct dependency:
  `error[E0433]: cannot find module or crate tempfile in this scope` at `src/web/spa.rs:359`; `recipe lint` exited 101. The test was changed to use `crate::testutil::tmp_dir`. The final full gate later passed after this fix; see Final gate. This baseline attempt predates the TLS parser regression test.
- Full-check log: `/tmp/rtok-t962-full-check-before.log`.

## Dependency advisory scan

- `mise exec cargo-deny@0.20.2 -- cargo deny check advisories` exited 1 against the current advisory database. It reported five entries:
  - `RUSTSEC-2026-0382`: `rustls-webpki 0.103.15` via `rustls`, `reqwest`, `rustls-platform-verifier`, and `tokio-rustls`; fixed in `>=0.103.16`.
  - `RUSTSEC-2026-0383`: `webpki-root-certs 1.0.6` via `rustls-platform-verifier`; fixed in `>=1.0.10`.
  - `RUSTSEC-2026-0381`: `webpki-roots 1.0.9`; fixed in `>=1.0.10`.
  - `RUSTSEC-2025-0141`: `bincode 1.3.3` through `printpdf`/`azul-layout`; unmaintained, advisory says no safe upgrade is available.
  - `RUSTSEC-2025-0134`: `rustls-pemfile 2.2.0`; unmaintained, advisory says no safe upgrade is available.
- The three approved RustSec fixes were applied to `Cargo.toml`, `Cargo.lock`, and `fuzz/Cargo.lock`. A later local refactor replaced rustls-pemfile with rustls-pki-types PEM parsing; the lockfiles were manually updated only by removing its package entry and the rtok reference (no resolver churn). Final `mise exec cargo-deny@0.20.2 -- cargo deny check advisories` exits 1 because `RUSTSEC-2025-0141` bincode 1.3.3 is unmaintained with no safe upgrade. The rustls-pemfile advisory `RUSTSEC-2025-0134` is gone. Raw output ends `advisories FAILED`; exit: 1. Log: `/tmp/rtok-t962-cargo-deny-post-parser.log` (`/tmp/rtok-t962-cargo-deny-post-parser.exit`).

## Web dependency audit

After the approved pins and lockfile updates, `npm audit --package-lock-only --ignore-scripts --prefix web` exited 0 with `found 0 vulnerabilities`; production-only `npm audit --omit=dev --package-lock-only --ignore-scripts --prefix web` also exited 0 with `found 0 vulnerabilities`. `npm ci --prefix web` passed after the final lock review: `added 282 packages, and audited 283 packages` and `found 0 vulnerabilities`. `brand` audit found 0 vulnerabilities. Initial pre-update audit details remain in `/tmp/rtok-t962-npm-web-audit.log` and `/tmp/rtok-t962-npm-web-prod-audit.log`; final audit logs are `/tmp/rtok-t962-npm-web-audit-final.log` and `/tmp/rtok-t962-npm-web-prod-audit-final.log`.

## Focused web integration

`mise exec -- cargo nextest run -p rtok --all-features --test web --test mcp_http` — 22 passed, 0 skipped (including `http_text_graph_does_not_reveal_credential_definitions`). Log: `/tmp/rtok-t962-web-integration.log`.

## Final gate

This full gate ran before the later TLS PEM parser regression test/refactor. `env -u RUST_LOG mise exec -- just full-check` exited 0. Raw log summary: `Summary [ 278.829s] 4191 tests run: 4191 passed, 8 skipped`. The full gate also passed fmt, workspace clippy, wasm-demo guest clippy, min-feature build, duplication, JS lint/format, and Python tests. Log: `/tmp/rtok-t962-full-check-final.log`.

A duplicate `env -u RUST_LOG mise exec -- just check` was intentionally interrupted before the TLS parser refactor because the running suite contained only 4,191 tests and did not include the new test. Raw tail: `2860/4191 tests run: 2844 passed (1 leaky), 16 failed, 8 skipped`; `1331/4191 tests were not run due to interrupt`. The wrapper returned 130; the 16 cancellations are not test assertion failures. Log: `/tmp/rtok-t962-check-final.log`.

TLS regression baseline before the parser refactor: `mise exec -- cargo nextest run -p rtok --lib -E 'test(tls::tests)'` — `4 tests run: 4 passed, 2737 skipped`. After the refactor and lock removal, the same command again passed `4 tests run: 4 passed, 2737 skipped`. Logs: `/tmp/rtok-t962-tls-baseline.log`, `/tmp/rtok-t962-tls-after.log`.

Final `env -u RUST_LOG mise exec -- just check` after the TLS refactor exited 0. Raw nextest summary: `Summary [270.799s] 4192 tests run: 4192 passed, 8 skipped`. Shared Cargo changes made selective-check delegate to full-check. Log: `/tmp/rtok-t962-check-post-parser.log`.

## Focused fixes

- `mise exec -- cargo fmt --all`
  - Passed after all four regression test targets were in the worktree.
- `mise exec -- cargo nextest run -p rtok --lib --all-features -E 'test(web::spa::tests) | test(plugins::wasm::tests) | test(plugins::read::search::tests) | test(plugins::graph::text::tests)'`
  - 45 passed, 2,703 skipped. This includes the SPA symlink test; all five WASM allocation cases; the read secret-policy test and existing search tests; graph text tests.
  - Log: `/tmp/rtok-t962-focused.log`.

## Dependency inventory

Initial `mise exec -- cargo metadata --format-version 1` before removal of rustls-pemfile reported 11 workspace packages and 692 resolved packages including dependencies.

## Web checks after approved dependency updates

- `npm run typecheck --prefix web` exited 0.
- `npm run build --prefix web` exited 0, including Vite production build and precompression (18 files); Vite emitted its existing large-chunk warning.
- `STORYBOOK_DISABLE_TELEMETRY=1 npm run build-storybook --prefix web` exited 0.
- `STORYBOOK_DISABLE_TELEMETRY=1 npm run test:stories --prefix web` exited 0: `Test Files 39 passed (39); Tests 260 passed (260)`.
- `npm run test:e2e --prefix web` exited 0: `45 passed (30.8s)`. The local binary was rebuilt after the production web build. Browser fixtures set `RTOK_HOST_SANDBOX=1`, use a fresh temporary HOME/RTOK_HOME and `/usr/bin:/bin` PATH, and pass no real agent app paths.
- The first post-install unit runs used the wrong cache cleanup path (`web/node_modules/.vite`); Vitest's persistent filesystem module cache is `web/node_modules/.vitest-cache`. After verifying that cache was a real directory inside this worktree, `mise exec -- npx vitest --clearCache` removed it. On the unchanged production source, focused tests then passed: `Test Files 2 passed (2); Tests 5 passed (5)` (`/tmp/rtok-t962-icon-tests-after-vitest-cache-clear.log`). The quiet full unit run passed: `Test Files 55 passed (55); Tests 503 passed (503)` (`/tmp/rtok-t962-web-tests-after-vitest-cache-clear.log`).
- Investigation correction: an initial isolated dependency fixture omitted the nested installed brand icon package, so its early failure was invalid; a separate-glob experiment in a valid fixture passed but was unnecessary. The cache clear on the final worktree fixed the tests with no Icon source change.

After clearing the correct Vitest cache, the complete web unit suite passed: `55` files and `503` tests. The fresh embedded SPA also passed the focused Rust HTTP integration suite: `Summary [23.398s] 22 tests run: 22 passed, 0 skipped` for `env -u RUST_LOG mise exec -- cargo nextest run -p rtok --all-features --test web --test mcp_http`. The updated E2E helper passed `mise exec -- oxfmt --check web/e2e/rtok.ts` and `mise exec -- oxlint --deny-warnings web/e2e/rtok.ts`.
