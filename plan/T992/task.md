---
status: in progress
priority: P2
complexity: 3
readiness: 0%
agent: 92b1bcc9-6c0f-4ea7-ad76-d74268fcc6ac
---

# T992. Refresh the report snapshot for tracked project skills

The `tests/trycmd/report-md.toml` snapshot expects one project skill (`dry-refactoring`), but the current checkout contains four tracked project skills: `core`, `dogfood`, `agent-browser` and `dry-refactoring`. The full gate therefore fails because the report output includes all four. Refresh the stale snapshot rows to match the current tracked skill inventory.

Execution plan: inspect the current rendered rows in `report-md.toml` and confirm all four current skill descriptions and body sizes; update only the stale expected skill rows in that snapshot; rerun the focused CLI snapshot test, full CLI trycmd suite, and `just check`. Do not change skill content or production listing behavior.

Check: `just check`
