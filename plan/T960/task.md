---
status: in progress
priority: P1
complexity: 3
readiness: 100%
agent: OpenAI / GPT-6
---

# T960. Audit HTTP archive access and log rotation failures

The creator requested bug and security fixes, focused refactoring, and feature suggestions.
Review the current main branch against the existing audit before changing code. Older
findings A1–A9 already have fixes; do not repeat them or take another agent's task.

Scope: A11 (HTTP expansion must not expose other sessions' archives), and A13
(rotation lock contention or failure must not silently suppress log lines). Reuse the
existing archive capability boundary and file-lock implementation; preserve stdio expansion,
content-addressed deduplication, output accounting, and Windows rename behavior. Keep new
feature proposals separate from implementation.

Execution plan:
1. Add regression tests for foreign archive prefixes/full IDs, current-server expansion,
   duplicate content first archived by another session, and rotation-lock failures.
2. Restrict HTTP archive resolution at the runtime capability boundary; share the existing
   expansion rendering and accounting. Move rotation coordination out of the shared temp
   namespace and preserve appends when rotation cannot acquire its lock.
3. Run focused tests before and after, inspect the diff, run `just check`, and record exact
   results and any environment blockers. Review existing helpers before adding code.

Check: unauthorized HTTP archive IDs reveal no payload or candidate IDs; newly archived
HTTP content remains expandable, including duplicate bytes; stdio behavior is unchanged;
rotation failures preserve the appended line; `just check`.
