---
status: in progress
priority: P1
complexity: 3
readiness: 100%
agent: OpenAI / GPT-6
---

# T961. Restrict untrusted project configuration (A10)

The creator explicitly requested the A10 fix, regression tests and checks.
Project TOML and project dotenv must not change network destinations, credentials,
commands or security-sensitive filesystem settings. User config, home dotenv, process
environment and explicit CLI configuration remain trusted sources.

Execution plan:
1. Reproduce project TOML/dotenv overriding trusted sensitive values.
2. Filter both project providers before merging, including legacy aliases; retain safe
   project settings, warnings and ignored-source diagnostics without exposing values.
3. Prove precedence, home dotenv, explicit config and worktree behavior; update docs,
   run focused tests and just check, inspect the diff, commit and write done.md.

No new dependencies or interactive trust workflow.

Check: malicious project settings
cannot redirect data, widen read access, execute extractors or redirect storage.
