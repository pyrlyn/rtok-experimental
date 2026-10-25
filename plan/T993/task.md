---
status: in progress
priority: P2
complexity: 3
readiness: 0%
agent: 92b1bcc9-6c0f-4ea7-ad76-d74268fcc6ac
---

# T993. Select the file workspace before probing outline LSP capability

The Dart LSP integration gate fails when Dart is installed but rust-analyzer is unavailable: outline capability probing uses the Cargo cwd before selecting the file's Dart workspace. Done means capability detection uses the same workspace as the outline session, and a deterministic regression proves the Dart workspace is not blocked by unrelated cwd capabilities without invoking real language servers.

Execution plan: reuse the existing outline workspace selection before capability probing; add a regression with cached capabilities for separate Cargo and Dart projects; preserve the shared session for nested Cargo manifests; run focused graph outline/LSP tests and the full gate. The standard library and existing capability API cover the change; no dependency is needed.

Check: `just check`
