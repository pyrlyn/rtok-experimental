# T965. Close remaining security audit findings

Completed 2026-10-22 on local branch `codex/t965-security-followup`, continuing T962.
Implementation commit: `d3213aad7646cc929bb07ccdaf63a9e160a92a89`.
No branch was pushed or PR opened. The original checkout was left untouched.

Removed unmaintained bincode by disabling printpdf's unused default HTML feature.
Native PDF reporting is preserved; both lockfiles lose only the 15 unused packages,
with no additions or retained version changes. The dependency advisory check now passes.

Closed A14 with a Unix permission audit and explicit, descriptor-relative repair via
`rtok doctor --fix --only permissions --yes`. Preview, partial refusal and repeated
repair preserve file contents; unsafe links and external configured paths are refused.
The doctor JSON/web model exposes the same findings. No real user data was repaired.

Closed A15 with positive configurable limits: 8 MiB per request/retained response prefix
and eight global in-flight requests, including plain mode and disabled lane scheduling.
The concurrency permit covers streaming and detached buffer processing. Large responses
still stream whole; truncated recordings do not enter the semantic cache.

Closed A16 by making telemetry content opt-in, including generic OTLP endpoint fallback.
Existing explicit opt-in is preserved. Configuration, schemas, CLI snapshots and English,
Russian and Ukrainian documentation are synchronized.

Checks: 4,203 Rust tests passed, eight skipped; 504 web tests and 43 Python tests passed.
PDF regressions passed before and after the dependency change. Clippy, formatting,
duplication checks, web type/build checks and the minimal Rust build completed. All
recorded stages of the final `just check` completed without errors; its shell exit code
was not retained, and the final build was independently verified with exit 0. See
[validation.md](validation.md) for exact commands, evidence and the capture limitation.

The fuzz lockfile was already stale against its manifest; its locked metadata check is
still not validated. A10/A11/A13 are implemented separately in T963's
[PR #1249](https://github.com/pyrlyn/rtok/pull/1249), not merged into this branch.
See [review.md](review.md) for compatibility, permission-scan limits and remaining scope.
No unrelated feature, benchmark or token-saving claim was added.
