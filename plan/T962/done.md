# T962. Project audit and confirmed fixes

Completed 2026-10-22 on local branch `codex/t962-security-audit`, based on `b7183a941`.
No PR was opened and no branch was pushed. The original checkout was left untouched.

Fixed runtime SPA symlink escapes, unbounded WASM guest memory/table allocation, and
HTTP text-graph access to credential files. Reused existing containment and secret-policy
helpers, and installed wasmi's existing allocation limiter before guest instantiation.
Regression fixtures demonstrated all three defects before the fixes.

Applied the creator-approved TLS and web development dependency patches. Removed the
unmaintained rustls-pemfile wrapper in favor of Rustls's existing PEM reader, preserving
bundle parsing behavior. Browser test fixtures now also set the existing host sandbox flag.
No new runtime dependency or proposed feature was added.

Checks: `just check` passed the full Rust gate (4,192 tests passed, 8 skipped). After web
dependency updates, 503 unit tests, 260 Storybook tests, 45 browser E2E tests, and 22 fresh
embedded-SPA/HTTP integration tests passed. Type checks, production and Storybook builds,
and the changed E2E helper's formatting/lint checks passed. Full and production-only npm
audits report zero vulnerabilities.

`cargo deny check advisories` still exits 1 for unmaintained `bincode 1.3.3` in the PDF
dependency chain; the three TLS vulnerabilities and rustls-pemfile finding are gone. Other
previously tracked findings and parallel task ownership remain explicit in the audit report.
This closes the audit and fixes described by T962, not every outstanding security task.

See [review.md](review.md) for coverage, limitations, residual findings and three feature
proposals, and [validation.md](validation.md) for commands and exact results. No benchmarks
or token-saving claims were made.
