---
status: in progress
priority: P2
complexity: 3
readiness: 0%
agent: 92b1bcc9-6c0f-4ea7-ad76-d74268fcc6ac
---

# T994. Isolate pipe-output tests from ambient debug logging

Two pipe-output integration tests inherit ambient `RUST_LOG=warn`, so rtok's logger adds an error prefix and breaks their byte-exact stderr assertions. Done means tests pin their expected default logging behavior by removing logging environment variables in child processes; production logging remains unchanged.

Execution plan: inspect test child-process helpers and logger environment handling; remove `RUST_LOG` and `RUST_LOG_STYLE` in the affected child commands using the existing standard-library API; prove the tests pass under ambient debug logging and forced log styling; run focused tests and the full gate.

Check: `just check`
