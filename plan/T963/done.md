# T963. Combined security fixes and PR

Combined T960 and T961 on `codex/t963-combine-security-fixes`, based on fresh main.
Retained all code and regression tests for A10, A11 and A13. Resolved the only conflict,
the audit-table A10/A11 status rows, with both findings marked fixed alongside A13.
Reconciled report commit references and T960's historical A10 deferral. Source worktrees
were left unchanged.

Published [PR #1249](https://github.com/pyrlyn/rtok/pull/1249) against `pyrlyn/rtok:main`.
GitHub reported the branch mergeable. The PR is open for review; it has not been merged.

Check: combined normalized `just check` selected full-check and exited 0: 4,196 Rust tests
and 43 Python tests passed, 8 skipped; two passing tests were marked leaky. Clippy,
formatting, duplication checks and minimal production build passed. Swarfr was unavailable.
See [combined validation](validation.md) for commands, limitations and run ID.

Status: done 2026-10-22
