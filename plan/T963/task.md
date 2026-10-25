---
status: in progress
priority: P1
complexity: 2
readiness: 100%
agent: OpenAI / GPT-6
---

# T963. Combine T960 and T961 for review

The creator requested one branch and a PR containing both completed security fixes.

Execution plan:
1. Start from fresh origin/main, cherry-pick the five T960/T961 commits and resolve
   conflicts while retaining all three audit fixes and regression tests.
2. Reconcile task reports and audit status, inspect the combined diff and run just check.
3. Commit the integration report, push the combined branch and create a PR against main.

Check: A10, A11 and A13 changes and tests are retained, just check passes, clean branch
is pushed and a reviewable PR exists. Do not merge the PR or modify the source worktrees.
