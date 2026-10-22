---
status: in progress
priority: P1
complexity: 4
readiness: 100%
agent: OpenAI / GPT-6
---

# T965. Close remaining security audit findings

Creator request: remove the unmaintained bincode dependency and fix the remaining
security findings from the audit. Continue the completed T962 changes in this branch.
A10, A11 and A13 are owned by T963; do not modify or merge those foreign branches.

Execution plan:
1. Trace bincode's dependency path; remove unused PDF features if native report
   rendering does not need them, preserving PDF output and avoiding version bumps.
2. Close A16 with explicit opt-in telemetry content and tests for endpoint fallback.
3. Close A15 with typed body limits and a global concurrency bound that covers
   buffering and response streaming even when lane scheduling is disabled.
4. Close A14 with a permission audit and explicit, bounded, idempotent doctor repair;
   preserve data, refuse unsafe paths, and exercise partial failure in scratch fixtures.
5. Synchronize configuration/schema and translated documentation, run focused
   regressions and project gates, and record evidence and residual limitations.

Check: PDF regressions, telemetry opt-in, proxy resource bounds and permission repair tests pass; run just check and the dependency advisory scan and record their exact outcomes.
