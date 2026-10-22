# T962. Audit project boundaries and fix confirmed defects

Creator request: scan the project for bugs and vulnerabilities, fix confirmed defects,
refactor where useful, and propose features without implementing them.

Confirmed scope: contain runtime SPA assets across symlinks, bound WASM guest
linear memory/table allocation (existing audit A12), and apply the existing HTTP
secret policy to the shared text reader used by the graph fallback. A10 and A11/A13 belong to T961
and T960 respectively; those branches are not modified or merged here.

The creator approved minimum security updates on 2026-10-22: rustls-webpki 0.103.16,
webpki-root-certs 1.0.10, and webpki-roots 1.0.10. Re-run the advisory scan after
updating those resolved versions; do not update unrelated dependencies.

Follow-up maintenance scope: replace the sole `rustls-pemfile` call with the existing
`rustls::pki_types` PEM reader, then remove the unmaintained wrapper dependency. Preserve
certificate ordering, non-certificate filtering, and fail-closed bundle handling; no version bump.

The creator separately approved the web development dependency fixes on 2026-10-22:
Storybook 10.6.3, Vitest 5.0.5, Vite 8.3.5, and source-map-js 1.2.2. Resolve only these
patches and their required dependency graph updates, then rerun web audit/build/tests.
Before browser integration tests, set the existing `RTOK_HOST_SANDBOX` in the E2E child
environment: a temporary HOME/PATH alone does not isolate absolute desktop app paths.
Web validation requires the tracked brand dependencies as well as web dependencies.
An incomplete first setup left a stale Vitest filesystem module cache; clearing only this
worktree's `.vitest-cache` restored the icon-contract tests without a source change.

Execution plan:
1. Inventory the workspace, prior audits and active fixes; inspect network, filesystem,
   configuration, process, archive, plugin and frontend trust boundaries.
2. Reproduce new findings with isolated fixtures before changing production code.
3. Reuse existing path/security helpers for narrow fixes; preserve embedded assets,
   stdio behavior and normal files. Limit version changes to creator-approved security updates.
4. Run focused regressions and project gates; record exact coverage, blockers, outstanding
   findings and feature proposals in this task's audit report.

Check: new regression tests fail before each fix and pass afterwards; `just check` and
the full project gate are attempted and their results recorded without claiming unrun checks.
