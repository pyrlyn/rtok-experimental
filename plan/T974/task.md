---
status: in progress
priority: P2
complexity: 2
readiness: 100%
agent: Claude Code / claude-opus-5-5
---

# T974. `zsh_tab_completes_with_a_stale_compinit_dump_and_no_zshrc_edit` fails

The 2026-10-22 log review reported this test (`tests/completions_shells.rs`) as failing. Is it
a real regression in `src/completions/install.rs`, a host dependency (a zsh version, a user rc
file leaking in), or flaky?

Done when: the cause is found and fixed, or the test is isolated from the host. The test passes
locally and in CI and is not ignored.

Check: `cargo nextest run --test completions_shells` three times in a row; `just check`.
